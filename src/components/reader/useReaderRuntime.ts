import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Book, ReaderState, ReaderTab, WordMarkStatus } from '@/types';
import {
  ACTIVITY_INTERVAL_MS,
  ACTIVITY_MINUTES,
  DEFAULT_CHAPTER,
  type ReaderSelection,
} from './readerShared';

function clampProgressPercent(value: number): number {
  return Math.round(Math.min(100, Math.max(0, value)));
}

export function useReaderRuntime(book: Book, options?: { getDoc?: () => Document | null }) {
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<ReaderTab>('notes');
  const [selection, setSelection] = useState<ReaderSelection | null>(null);
  const [readerState, setReaderState] = useState<ReaderState | null>(null);
  const [noteDraft, setNoteDraft] = useState('');
  const lastActivityAtRef = useRef(0);
  const activityBusyRef = useRef(false);

  useEffect(() => {
    let mounted = true;

    const loadReaderState = async () => {
      try {
        const state = await invoke<ReaderState>('get_reader_state', { bookId: book.id });
        if (mounted) {
          setReaderState(state);
        }
      } catch (error) {
        console.error('Failed to load reader state:', error);
      }
    };

    void loadReaderState();
    return () => {
      mounted = false;
    };
  }, [book.id]);

  const effectiveProgress = readerState?.progress ?? book.progress;
  const effectiveChapter = readerState?.chapter ?? DEFAULT_CHAPTER;
  const effectiveHref = readerState?.href ?? null;
  const effectiveOffset = readerState?.offset ?? 0;

  /* 上次已落盘位置ref 镜像：卸载时比较，有变化立即补写——
     否则翻页后 450ms 内退出阅读器，最后一步进度丢失。 */
  const persistedPositionRef = useRef<{ progress: number; chapter: string; href: string | null; offset: number } | null>(null);

  // 前后端统一使用 0–100 整数百分比；章节/进度/位置变化 450ms 防抖写入。
  useEffect(() => {
    const persist = () => {
      const last = persistedPositionRef.current;
      const progress = clampProgressPercent(effectiveProgress);
      const href = effectiveHref ?? null;
      const offset = effectiveOffset ?? 0;
      if (last && last.progress === progress && last.chapter === effectiveChapter && last.href === href && last.offset === offset) {
        return;
      }
      persistedPositionRef.current = { progress, chapter: effectiveChapter, href, offset };
      void invoke<ReaderState>('save_reader_progress', {
        bookId: book.id,
        progress,
        chapter: effectiveChapter,
        href,
        offset,
      }).catch((error) => console.error('Failed to save reader progress:', error));
    };

    const timer = window.setTimeout(persist, 450);

    return () => {
      window.clearTimeout(timer);
      persist(); // 卸载补写：防抖窗口内的最后一步进度
    };
  }, [book.id, effectiveChapter, effectiveProgress, effectiveHref, effectiveOffset]);

  useEffect(() => {
    lastActivityAtRef.current = 0;
  }, [book.id]);

  useEffect(() => {
    const tick = () => {
      if (document.hidden || selection) return;
      const now = Date.now();
      if (now - lastActivityAtRef.current < ACTIVITY_INTERVAL_MS) return;
      void recordActivity(ACTIVITY_MINUTES);
    };

    const interval = window.setInterval(tick, 30_000);
    return () => window.clearInterval(interval);
  }, [selection, effectiveChapter, effectiveProgress, book.id]);

  const recordActivity = async (minutes: number) => {
    if (activityBusyRef.current) return;
    activityBusyRef.current = true;
    try {
      const next = await invoke<ReaderState>('log_reader_activity', {
        bookId: book.id,
        minutes,
        progress: clampProgressPercent(effectiveProgress),
        chapter: effectiveChapter,
      });
      setReaderState(next);
      lastActivityAtRef.current = Date.now();
    } catch (error) {
      console.error('Failed to log reader activity:', error);
    } finally {
      activityBusyRef.current = false;
    }
  };

  const openNotes = () => {
    setSidebarOpen(true);
    setActiveTab('notes');
  };

  /* 阅读器内嵌校对模式：开关 = 侧栏 review tab 的显隐 */
  const [reviewMode, setReviewModeState] = useState(false);
  const toggleReviewMode = () => {
    const next = !reviewMode;
    setReviewModeState(next);
    setSidebarOpen(next);
    setActiveTab(next ? 'review' : 'notes');
  };

  const openToc = () => {
    setSidebarOpen(true);
    setActiveTab('toc');
  };

  const openSearch = () => {
    setSidebarOpen(true);
    setActiveTab('search');
  };

  // 词汇黑白名单：status 为 null 时撤回标记。
  const markWord = useCallback(
    async (word: string, status: WordMarkStatus | null, context: string) => {
      try {
        const next =
          status === null
            ? await invoke<ReaderState>('unmark_word_status', { bookId: book.id, word })
            : await invoke<ReaderState>('mark_word_status', {
                bookId: book.id,
                word,
                status,
                context,
              });
        setReaderState(next);
      } catch (error) {
        console.error('Failed to mark word:', error);
      }
    },
    [book.id],
  );

  // 词汇透析成卡：语境句 + 当前章节 + AI 语境义（若有）一并记录。
  const addVocabCard = useCallback(
    async (word: string, context: string, customDefinition?: string, contextZh?: string) => {
      try {
        const next = await invoke<ReaderState>('add_vocab_card', {
          bookId: book.id,
          word,
          context,
          chapter: effectiveChapter,
          customDefinition: customDefinition ?? null,
          contextZh: contextZh ?? null,
        });
        setReaderState(next);
      } catch (error) {
        console.error('Failed to add vocab card:', error);
      }
    },
    [book.id, effectiveChapter],
  );

  // AI 语境释义保存：多义词点击「AI 释义」生成后写回（自动落入 learning 黑名单）。
  const saveWordDefinition = useCallback(
    async (word: string, definition: string, context: string) => {
      try {
        const next = await invoke<ReaderState>('save_word_definition', {
          bookId: book.id,
          word,
          definition,
          context,
        });
        setReaderState(next);
      } catch (error) {
        console.error('Failed to save word definition:', error);
      }
    },
    [book.id],
  );

  // 书签/笔记：progress 落库为 0–100 整数；href（EPUB 章节锚点）用于跳回原文。
  const saveBookmark = async (quote: string, href?: string | null, style = 'bookmark') => {
    try {
      const next = await invoke<ReaderState>('add_bookmark', {
        bookId: book.id,
        chapter: effectiveChapter,
        quote,
        progress: clampProgressPercent(effectiveProgress),
        href: href ?? null,
        style,
      });
      setReaderState(next);
      setSelection(null);
    } catch (error) {
      console.error('Failed to add bookmark:', error);
    }
  };

  /* 删除书签/划线与笔记（只增不删处补删除入口）。 */
  const removeBookmark = async (bookmarkId: string) => {
    try {
      const next = await invoke<ReaderState>('remove_bookmark', { bookId: book.id, bookmarkId });
      setReaderState(next);
    } catch (error) {
      console.error('Failed to remove bookmark:', error);
    }
  };

  /** 删除书签同步摘除正文可视划线（mark.mt-mark 文本=quote）。 */
  const removeBookmarkMark = useCallback((quote: string) => {
    const doc = options?.getDoc?.();
    if (!doc?.body || !quote.trim()) return;
    const needle = quote.trim();
    doc.body.querySelectorAll('mark.mt-mark').forEach((mark) => {
      if ((mark.textContent ?? '').trim() === needle) {
        const parent = mark.parentNode;
        if (!parent) return;
        while (mark.firstChild) parent.insertBefore(mark.firstChild, mark);
        mark.remove();
      }
    });
    }, []);

  const removeNote = async (noteId: string) => {
    try {
      const next = await invoke<ReaderState>('remove_note', { bookId: book.id, noteId });
      setReaderState(next);
    } catch (error) {
      console.error('Failed to remove note:', error);
    }
  };

  const saveNote = async (quote: string, note: string, href?: string | null) => {
    if (!note.trim()) return;
    try {
      const next = await invoke<ReaderState>('add_note', {
        bookId: book.id,
        chapter: effectiveChapter,
        quote,
        note,
        progress: clampProgressPercent(effectiveProgress),
        href: href ?? null,
      });
      setReaderState(next);
      setNoteDraft('');
      setSelection(null);
      setSidebarOpen(true);
      setActiveTab('notes');
    } catch (error) {
      console.error('Failed to add note:', error);
    }
  };

  /** EPUB 渲染层回写阅读位置（章节名 + 0–100 整数进度 + 精确锚点）。 */
  const setReadingPosition = useCallback(
    (chapter: string, progressPercent: number, href?: string | null, offset?: number) => {
      setReaderState((current) => {
        const clamped = clampProgressPercent(progressPercent);
        if (!current) return current;
        const nextHref = href ?? current.href ?? null;
        const nextOffset = offset ?? current.offset ?? 0;
        if (
          current.chapter === chapter &&
          current.progress === clamped &&
          (current.href ?? null) === nextHref &&
          (current.offset ?? 0) === nextOffset
        ) {
          return current;
        }
        return { ...current, chapter, progress: clamped, href: nextHref, offset: nextOffset };
      });
    },
    [],
  );

  return {
    activeTab,
    reviewMode,
    toggleReviewMode,
    effectiveChapter,
    effectiveHref,
    effectiveOffset,
    effectiveProgress,
    noteDraft,
    openNotes,
    openSearch,
    openToc,
    markWord,
    addVocabCard,
    saveWordDefinition,
    readerState,
    setReaderState,
    removeBookmark,
    removeBookmarkMark,
    removeNote,
    saveBookmark,
    saveNote,
    setReadingPosition,
    selection,
    setActiveTab,
    setNoteDraft,
    setSelection,
    setSidebarOpen,
    sidebarOpen,
  };
}
