import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Book, BookProfile, TranslationTask } from '@/types';
import { useDocumentVisibility } from './useDocumentVisibility';
import {
  isWebPreview,
  WEB_PREVIEW_BOOKS,
  WEB_PREVIEW_TASKS,
} from './webPreviewFixtures';
import {
  applyBookProfiles,
  loadCompletedBooks,
  loadMissingBooks,
  mergeTaskUpdates,
  upsertBookProfile,
  upsertTaskEntry,
} from './taskRuntimeShared';

// 事件契约（与后端 commands/tasks/events.rs 对应）：
// - task-progress: payload 为 JSON 序列化的 TranslationTask（创建/进度/完成/失败/取消/暂停）
// - task-deleted: payload 为 { taskId }
const TASK_PROGRESS_EVENT = 'task-progress';
const TASK_DELETED_EVENT = 'task-deleted';
// 低频兜底拉取：事件通道异常时仍能自愈（30s 一次，页面不可见时暂停）。
const FALLBACK_POLL_INTERVAL_MS = 30_000;

function parseTaskPayload(payload: unknown): TranslationTask | null {
  if (typeof payload !== 'string') return null;
  try {
    return JSON.parse(payload) as TranslationTask;
  } catch (error) {
    console.error('Failed to parse task event payload:', error);
    return null;
  }
}

export function useTaskRuntime() {
  const [baseBooks, setBooks] = useState<Book[]>([]);
  const [tasks, setTasks] = useState<TranslationTask[]>([]);
  const [profiles, setProfiles] = useState<BookProfile[]>([]);
  const booksRef = useRef<Book[]>([]);
  const completedNotifiedRef = useRef<Set<string>>(new Set());
  const isDocumentVisible = useDocumentVisibility();

  // 书库展示层：任务重建的书籍 + 收藏/标签档案 merge（档案独立持久化于 book_profiles.json）。
  const books = useMemo(() => applyBookProfiles(baseBooks, profiles), [baseBooks, profiles]);

  useEffect(() => {
    booksRef.current = books;
  }, [books]);

  useEffect(() => {
    const loadTasks = async () => {
      if (isWebPreview) {
        setTasks(WEB_PREVIEW_TASKS);
        setBooks(WEB_PREVIEW_BOOKS);
        return;
      }
      try {
        const taskList = await invoke<TranslationTask[]>('list_tasks');
        setTasks(taskList);
        // 档案拉取独立降级：预览环境无该命令/返回异常值时书库照常渲染（无收藏/标签）
        try {
          const loadedProfiles = await invoke<BookProfile[]>('list_book_profiles');
          setProfiles(Array.isArray(loadedProfiles) ? loadedProfiles : []);
        } catch (profileError) {
          console.error('Failed to load book profiles:', profileError);
        }
        setBooks(await loadCompletedBooks(taskList));
      } catch (error) {
        console.error('Failed to load tasks:', error);
      }
    };

    void loadTasks();
  }, []);

  // 收藏切换：先写后端（持久化成功才回前端），失败不乐观更新。
  const toggleBookFavorite = useCallback(async (bookId: string, favorite: boolean) => {
    try {
      const profile = await invoke<BookProfile>('set_book_favorite', { bookId, favorite });
      if (profile && typeof profile.bookId === 'string') {
        setProfiles((prev) => upsertBookProfile(prev, profile));
      }
    } catch (error) {
      console.error('Failed to set book favorite:', error);
    }
  }, []);

  const setBookTags = useCallback(async (bookId: string, tags: string[]) => {
    try {
      const profile = await invoke<BookProfile>('set_book_tags', { bookId, tags });
      if (profile && typeof profile.bookId === 'string') {
        setProfiles((prev) => upsertBookProfile(prev, profile));
      }
    } catch (error) {
      console.error('Failed to set book tags:', error);
    }
  }, []);

  /* 文件夹分组：空串=移出分组；同名 folder 即同组。 */
  const setBookFolder = useCallback(async (bookId: string, folder: string) => {
    try {
      const profile = await invoke<BookProfile>('set_book_folder', { bookId, folder });
      if (profile && typeof profile.bookId === 'string') {
        setProfiles((prev) => upsertBookProfile(prev, profile));
      }
    } catch (error) {
      console.error('Failed to set book folder:', error);
    }
  }, []);

  // 新完成的书籍：加载为 Book 并触发完成通知。事件与兜底轮询共用。
  const ingestTaskUpdates = useCallback(async (updates: TranslationTask[]) => {
    const newBooks = await loadMissingBooks(updates, booksRef.current);
    for (const book of newBooks) {
      if (!completedNotifiedRef.current.has(book.id)) {
        completedNotifiedRef.current.add(book.id);
        if ('Notification' in window && Notification.permission === 'granted') {
          new Notification('翻译完成', {
            body: `《${book.title}》翻译完成，点击查看`,
            icon: book.cover,
          });
        }
      }
    }
    if (newBooks.length === 0) return;

    setBooks((prev) => {
      const knownIds = new Set(prev.map((book) => book.id));
      return [...newBooks.filter((book) => !knownIds.has(book.id)), ...prev];
    });
  }, []);

  // 事件订阅：后端每次任务状态持久化后推送完整任务体，前端即时更新。
  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];

    const subscribe = async () => {
      try {
        unlisteners.push(
          await listen<string>(TASK_PROGRESS_EVENT, (event) => {
            const task = parseTaskPayload(event.payload);
            if (!task || disposed) return;
            setTasks((prev) => upsertTaskEntry(prev, task));
            void ingestTaskUpdates([task]);
          }),
          await listen<string>(TASK_DELETED_EVENT, (event) => {
            const payload = parseTaskPayload(event.payload) as unknown as
              | { taskId?: string }
              | null;
            const taskId = payload?.taskId;
            if (!taskId || disposed) return;
            setTasks((prev) => prev.filter((task) => task.id !== taskId));
            setBooks((prev) => prev.filter((book) => book.id !== taskId));
          }),
        );
      } catch (error) {
        console.error('Failed to subscribe task events:', error);
      }
    };

    if ('Notification' in window && Notification.permission === 'default') {
      void Notification.requestPermission();
    }

    void subscribe();
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [ingestTaskUpdates]);

  const activeTasks = useMemo(
    () => tasks.filter((task) => task.status === 'pending' || task.status === 'processing'),
    [tasks],
  );
  const activeTaskSignature = useMemo(
    () => activeTasks.map((task) => task.id).sort().join('|'),
    [activeTasks],
  );

  // 低频兜底：事件丢失（如前端后启动、窗口重建）时通过 list_tasks 对齐状态。
  useEffect(() => {
    if (!activeTaskSignature || !isDocumentVisible) return;

    let disposed = false;
    const pollTasks = async () => {
      try {
        const updates = await Promise.all(
          activeTasks.map((task) =>
            invoke<TranslationTask>('get_task_status', {
              taskId: task.id,
            }),
          ),
        );
        if (disposed) return;

        setTasks((prev) => mergeTaskUpdates(prev, updates));
        await ingestTaskUpdates(updates);
      } catch (error) {
        console.error('Failed to poll task status:', error);
      }
    };

    const interval = window.setInterval(() => {
      void pollTasks();
    }, FALLBACK_POLL_INTERVAL_MS);

    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [activeTaskSignature, isDocumentVisible, ingestTaskUpdates]);

  const upsertTask = useCallback((task: TranslationTask) => {
    setTasks((prev) => upsertTaskEntry(prev, task));
  }, []);

  const removeTask = useCallback((taskId: string) => {
    setTasks((prev) => prev.filter((task) => task.id !== taskId));
    setBooks((prev) => prev.filter((book) => book.id !== taskId));
  }, []);

  return {
    books,
    tasks,
    upsertTask,
    removeTask,
    toggleBookFavorite,
    setBookTags,
    setBookFolder,
  };
}
