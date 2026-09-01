import { invoke } from '@tauri-apps/api/core';
import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { Book, GlossaryDeckView, RuntimeNotice, TranslationTask } from '@/types';
import { useDocumentVisibility } from '@/hooks/useDocumentVisibility';
import { formatActionError, UI } from './libraryViewShared';
import { filterBooksByShelf, getShelfOptions, type LibraryShelfKey } from './libraryCatalog';
import {
  buildLibraryShelfViewModel,
  buildPendingTaskDraft,
  filterBooksByQuery,
  isSupportedImportFilename,
  resolveWorkbenchMainClass,
  sameNoticeList,
  type SelectedImportFile,
  type WorkbenchConfig,
} from './libraryWorkbenchShared';

interface UseLibraryWorkbenchArgs {
  books: Book[];
  tasks: TranslationTask[];
  onTaskCreated: (task: TranslationTask) => void;
  onTaskUpdated: (task: TranslationTask) => void;
  surfaceStyle: 'cards' | 'canvas';
}

export function useLibraryWorkbench({
  books,
  tasks,
  onTaskCreated,
  onTaskUpdated,
  surfaceStyle,
}: UseLibraryWorkbenchArgs) {
  const [selectedFile, setSelectedFile] = useState<SelectedImportFile | null>(null);
  const [selectedBook, setSelectedBook] = useState<Book | null>(null);
  const [selectedTask, setSelectedTask] = useState<TranslationTask | null>(null);
  const [runtimeNotices, setRuntimeNotices] = useState<RuntimeNotice[]>([]);
  const [selectedTaskNotices, setSelectedTaskNotices] = useState<RuntimeNotice[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [errorMessage, setErrorMessage] = useState('');
  const [importing, setImporting] = useState(false);
  const [activeShelf, setActiveShelf] = useState<LibraryShelfKey>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [config, setConfig] = useState<WorkbenchConfig>({
    articleType: 'academic',
    concurrent: true,
    maxWorkers: 10,
  });
  // 翻译启动时的术语词表选择：默认勾选所有已启用词表
  const [glossaryDecks, setGlossaryDecks] = useState<GlossaryDeckView[]>([]);
  const [selectedDeckIds, setSelectedDeckIds] = useState<string[]>([]);
  useEffect(() => {
    invoke<GlossaryDeckView[]>('list_glossary_decks')
      .then((decks) => {
        setGlossaryDecks(decks);
        setSelectedDeckIds(decks.filter((d) => d.enabled).map((d) => d.id));
      })
      .catch(() => setGlossaryDecks([]));
  }, []);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const isDocumentVisible = useDocumentVisibility();

  useEffect(() => {
    if (!selectedTask) return;
    const latestTask = tasks.find((task) => task.id === selectedTask.id);
    setSelectedTask(latestTask ?? null);
  }, [selectedTask, tasks]);

  useEffect(() => {
    if (!selectedTask || !isDocumentVisible) {
      setSelectedTaskNotices([]);
      return;
    }

    let disposed = false;
    const pollNotices = async () => {
      try {
        const notices = await invoke<RuntimeNotice[]>('get_runtime_notices', {
          limit: 12,
          scope: null,
          taskId: selectedTask.id,
        });
        if (!disposed) {
          setSelectedTaskNotices((previous) =>
            sameNoticeList(previous, notices) ? previous : notices,
          );
        }
      } catch (error) {
        if (!disposed) {
          console.error('Failed to load task notices:', error);
        }
      }
    };

    void pollNotices();
    const interval = window.setInterval(() => {
      void pollNotices();
    }, 1500);

    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [selectedTask, isDocumentVisible]);

  useEffect(() => {
    if (!isDocumentVisible) return;

    let disposed = false;
    const pollNotices = async () => {
      try {
        const notices = await invoke<RuntimeNotice[]>('get_runtime_notices', {
          limit: 4,
          scope: null,
          taskId: null,
        });
        if (!disposed) {
          setRuntimeNotices((previous) =>
            sameNoticeList(previous, notices) ? previous : notices,
          );
        }
      } catch (error) {
        if (!disposed) {
          console.error('Failed to load runtime notices:', error);
        }
      }
    };

    void pollNotices();
    const interval = window.setInterval(() => {
      void pollNotices();
    }, 3000);

    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [isDocumentVisible]);

  const handleFileSelect = () => fileInputRef.current?.click();

  // 网页剪藏：URL → 后端抓取正文 → MD 入导入目录 → 直接挂为待翻译文件。
  const captureWebArticle = async (url: string) => {
    setImporting(true);
    setErrorMessage('');
    try {
      const result = await invoke<{ title: string; outputPath: string }>(
        'capture_web_article',
        { url },
      );
      setSelectedFile({ name: `${result.title}.md`, path: result.outputPath });
    } catch (error) {
      console.error('Web capture error:', error);
      setErrorMessage(formatActionError(UI.captureFailed, error));
    } finally {
      setImporting(false);
    }
  };

  const handleFileChange = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;

    if (!isSupportedImportFilename(file.name)) {
      setErrorMessage(UI.selectDocument);
      return;
    }

    setImporting(true);
    setErrorMessage('');
    try {
      const buffer = await file.arrayBuffer();
      const importedPath = await invoke<string>('import_pdf', {
        filename: file.name,
        data: Array.from(new Uint8Array(buffer)),
      });
      setSelectedFile({ name: file.name, path: importedPath });
    } catch (error) {
      console.error('File import error:', error);
      setErrorMessage(formatActionError(UI.importFailed, error));
    } finally {
      setImporting(false);
    }
  };

  const refreshTask = async (taskId: string) => {
    const task = await invoke<TranslationTask>('get_task_status', { taskId });
    onTaskUpdated(task);
    setSelectedTask((current) => (current?.id === task.id ? task : current));
    return task;
  };

  const startTranslation = async (opts?: {
    articleType?: string;
    concurrent?: boolean;
    maxWorkers?: number;
    deckIds?: string[];
    sourcePath?: string;
  }) => {
    // 重翻已有书：sourcePath 指定原文件；新建：用待启动条选中文件
    const sourcePath = opts?.sourcePath ?? selectedFile?.path;
    if (!sourcePath) {
      setErrorMessage(UI.selectDocumentFirst);
      return;
    }

    setUploading(true);
    setErrorMessage('');
    try {
      const taskId = await invoke<string>('start_translation', {
        pdfPath: sourcePath,
        articleType: opts?.articleType ?? config.articleType,
        concurrent: opts?.concurrent ?? config.concurrent,
        maxWorkers: opts?.maxWorkers ?? config.maxWorkers,
        // 用户显式选择本次翻译启用的词表（后端仅应用其中处于启用状态的）
        glossaryDeckIds: opts?.deckIds ?? selectedDeckIds,
      });

      const createdAt = new Date().toISOString();
      /* 草稿来源：重译优先用本次 sourcePath；待启动文件不被重译占用或清除。 */
      const retranslated = Boolean(opts?.sourcePath);
      const currentSelected = retranslated
        ? { name: (opts?.sourcePath ?? 'Document').split(/[\\/]/).pop() ?? 'Document', path: opts!.sourcePath! }
        : selectedFile;
      if (currentSelected) {
        const effectiveConfig = {
          articleType: opts?.articleType ?? config.articleType,
          concurrent: opts?.concurrent ?? config.concurrent,
          maxWorkers: opts?.maxWorkers ?? config.maxWorkers,
        };
        onTaskCreated(buildPendingTaskDraft(taskId, currentSelected, effectiveConfig, createdAt, UI.taskCreated));
      }
      /* 仅消费待启动文件时清除（重译不动队列）。 */
      if (!retranslated) setSelectedFile(null);
    } catch (error) {
      console.error('Translation start error:', error);
      setErrorMessage(formatActionError(UI.startFailed, error));
    } finally {
      setUploading(false);
    }
  };

  const cancelTask = async (task: TranslationTask) => {
    setErrorMessage('');
    try {
      await invoke('cancel_translation_task', { taskId: task.id });
      await refreshTask(task.id);
    } catch (error) {
      console.error('Cancel task error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const retryTask = async (task: TranslationTask) => {
    setErrorMessage('');
    try {
      await invoke('retry_translation_task', { taskId: task.id });
      await refreshTask(task.id);
    } catch (error) {
      console.error('Retry task error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const resumeTask = async (task: TranslationTask) => {
    setErrorMessage('');
    try {
      await invoke('resume_translation_task', { taskId: task.id });
      await refreshTask(task.id);
    } catch (error) {
      console.error('Resume task error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const openArtifact = async (path?: string) => {
    if (!path) return;
    try {
      await openPath(path);
    } catch (error) {
      console.error('Open artifact error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const revealArtifact = async (path?: string) => {
    if (!path) return;
    try {
      await revealItemInDir(path);
    } catch (error) {
      console.error('Reveal artifact error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const clearTaskNotices = async (taskId: string) => {
    try {
      await invoke<number>('clear_runtime_notices', {
        scope: null,
        taskId,
      });
      setSelectedTaskNotices((prev) => prev.filter((notice) => notice.taskId !== taskId));
    } catch (error) {
      console.error('Clear task notices error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const clearAllNotices = async () => {
    try {
      await invoke<number>('clear_runtime_notices', {
        scope: null,
        taskId: null,
      });
      setRuntimeNotices([]);
      setSelectedTaskNotices([]);
    } catch (error) {
      console.error('Clear all notices error:', error);
      setErrorMessage(formatActionError(UI.actionFailed, error));
    }
  };

  const shelfOptions = useMemo(() => getShelfOptions(), []);
  const filteredBooks = useMemo(() => {
    const shelfBooks = filterBooksByShelf(books, activeShelf);
    return filterBooksByQuery(shelfBooks, searchQuery);
  }, [activeShelf, books, searchQuery]);
  const allShelfBooks = useMemo(() => filterBooksByShelf(books, activeShelf), [activeShelf, books]);
  const { currentBook, continueShelf, catalogBooks } = useMemo(
    () => buildLibraryShelfViewModel(filteredBooks, books),
    [filteredBooks, books],
  );
  const mainClass = useMemo(() => resolveWorkbenchMainClass(surfaceStyle), [surfaceStyle]);

  return {
    activeShelf,
    allShelfBooks,
    cancelTask,
    catalogBooks,
    config,
    continueShelf,
    currentBook,
    errorMessage,
    fileInputRef,
    filteredBooks,
    handleFileChange,
    handleFileSelect,
    historyOpen,
    importing,
    mainClass,
    openArtifact,
    refreshTask,
    revealArtifact,
    resumeTask,
    retryTask,
    runtimeNotices,
    searchQuery,
    selectedBook,
    selectedFile,
    selectedTask,
    selectedTaskNotices,
    shelfOptions,
    glossaryDecks,
    selectedDeckIds,
    toggleSelectedDeck: (deckId: string) => {
      setSelectedDeckIds((ids) =>
        ids.includes(deckId) ? ids.filter((id) => id !== deckId) : [...ids, deckId],
      );
    },
    setActiveShelf,
    setConfig,
    setHistoryOpen,
    setSearchQuery,
    setSelectedBook,
    setSelectedFile,
    setSelectedTask,
    clearAllNotices,
    clearTaskNotices,
    captureWebArticle,
    startTranslation,
    uploading,
  };
}
