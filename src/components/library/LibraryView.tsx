import { invoke } from '@tauri-apps/api/core';
import { open as openFileDialog, save as saveFileDialog } from '@tauri-apps/plugin-dialog';
import { motion } from 'motion/react';
import { createPortal } from 'react-dom';
import { FolderInput, FolderPlus, Globe, History, Pencil, Plus, Star, Trash2, Upload, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useT } from '@/i18n';
import type { AppSettings, Book, SkillLibraryEntry, TranslationTask } from '@/types';
import { formatActionError } from './libraryViewShared';
import { LibraryTaskDetailSheet, LibraryTaskHistorySheet } from './LibraryTaskSheets';
import { PreReadModal } from './PreReadModal';
import { useLibraryWorkbench } from './useLibraryWorkbench';
import { sortLibraryBooks, type LibrarySortKey } from './libraryWorkbenchShared';
import { displayBookExcerpt, displayBookTitle } from './bookDisplay';
import { useToast } from '@/components/common/Toast';
import { useAppDialog } from '@/components/common/AppDialog';
import { BookContextMenu } from './BookContextMenu';
import { TranslateConfigModal } from './TranslateConfigModal';

interface LibraryViewProps {
  books: Book[];
  tasks: TranslationTask[];
  onOpenBook: (book: Book) => void;
  onTaskCreated: (task: TranslationTask) => void;
  onTaskUpdated: (task: TranslationTask) => void;
  onDeleteTask: (taskId: string) => void;
  surfaceStyle: AppSettings['librarySurfaceStyle'];
  /** Skill 库（翻译前配置弹窗注入勾选）。 */
  skillEntries: SkillLibraryEntry[];
  onToggleFavorite: (bookId: string, favorite: boolean) => void;
  onSetTags: (bookId: string, tags: string[]) => void;
  /** 文件夹分组：folder 空串=移出分组。 */
  onSetFolder: (bookId: string, folder: string) => void;
  /** 右键「进行校对」。 */
  onOpenReview: (book: Book) => void;
  /** 右键「词汇透析」→ 学习视图该书生词库。 */
  onOpenVocab?: (book: Book) => void;
}

const COVER_HUES = ['#B7C4B6', '#C9C2B4', '#A8B8C4', '#C4B4C0', '#B4C4C0', '#C0B8A8', '#B8BCC8', '#C8BCA8'];

function hueFor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 997;
  return COVER_HUES[h % COVER_HUES.length];
}

/** 书库 v2：顶部栏（搜索/剪藏/导入/新建）→ hero（封面完整平铺）→ 竖封网格。 */
export function LibraryView({
  books,
  tasks,
  onOpenBook,
  onTaskCreated,
  onTaskUpdated,
  surfaceStyle,
  onToggleFavorite,
  onSetTags,
  onSetFolder,
  onOpenReview,
  onOpenVocab,
  onDeleteTask,
  skillEntries,
}: LibraryViewProps) {
  const t = useT();
  const toast = useToast();
  const { appPrompt, appConfirm } = useAppDialog();
  /* 基础大类：全部/已翻译/待翻译/收藏；已读/未读为另一维度切换。 */
  const [filter, setFilter] = useState<'all' | 'translated' | 'untranslated' | 'favorite' | 'read' | 'unread'>('all');
  /* 右键菜单 */
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; book: Book } | null>(null);
  /* 翻译配置弹窗：区别于直接导入，先配置再开翻 */
  const [configTarget, setConfigTarget] = useState<Book | null>(null);
  /* 新建翻译选书模式：右上角按钮打开配置弹窗（上传/书库选书） */
  const [newTaskOpen, setNewTaskOpen] = useState(false);
  /* 数据迁移：导入外部已译成书（本应用导出的双语/译文 EPUB 或任意原语书）。 */
  const [importingTranslated, setImportingTranslated] = useState(false);
  const handleImportTranslated = async () => {
    if (importingTranslated) return;
    setImportingTranslated(true);
    try {
      const path = await openFileDialog({
        multiple: false,
        title: t.library.importTranslatedBook,
        filters: [{ name: 'EPUB / 迁移包 / 工件目录（选 manifest.json）', extensions: ['epub', 'zip', 'json', 'md'] }],
      });
      if (typeof path !== 'string' || !path) return;
      const task = await invoke<TranslationTask>('import_translated_book', { path });
      // 后端已持久化并发 task-progress 事件；此处同步合并保证离线事件丢失时也能入库。
      onTaskUpdated(task);
      toast.success(t.library.importTranslatedDone);
    } catch (error) {
      toast.error(formatActionError(t.library.importTranslatedFailed, error));
    } finally {
      setImportingTranslated(false);
    }
  };
  /* 文件夹分组：打开的分组视图 + 拖拽目标高亮 */
  const [openFolder, setOpenFolder] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const dragBookIdRef = useRef<string | null>(null);
  const dragFolderRef = useRef<string | null>(null);
  /* 显式新建分组：空分组记录持久化，供拖拽入组 */
  const [extraGroups, setExtraGroups] = useState<string[]>(() => {
    try { return JSON.parse(window.localStorage.getItem('lib-extra-groups') ?? '[]') as string[]; } catch { return []; }
  });
  const persistExtraGroups = (groups: string[]) => {
    setExtraGroups(groups);
    window.localStorage.setItem('lib-extra-groups', JSON.stringify(groups));
  };
  const createGroup = async () => {
    const name = await appPrompt({ title: t.library.createFolderPrompt });
    const folder = name?.trim();
    if (!folder) return;
    if (extraGroups.includes(folder) || folders.map.has(folder)) return;
    persistExtraGroups([...extraGroups, folder]);
  };

  /* 单书迁移包导出：任务记录+工件+阅读状态打包 zip，可在另一台机器/重装后导入。 */
  const exportingBundleRef = useRef(false);
  const handleExportBundle = async (book: Book) => {
    if (exportingBundleRef.current) return;
    exportingBundleRef.current = true;
    try {
      const safeTitle = book.title.replace(/[\\/:*?"<>|]/g, '_').slice(0, 60) || 'book';
      const target = await saveFileDialog({
        title: t.library.exportBundle,
        defaultPath: `${safeTitle}.musebundle.zip`,
        filters: [{ name: 'MuseReader bundle', extensions: ['zip'] }],
      });
      if (typeof target !== 'string' || !target) return;
      await invoke('export_book_bundle', { taskId: book.id, outputPath: target });
      toast.success(t.library.exportBundleDone);
    } catch (error) {
      toast.error(formatActionError(t.library.exportBundleFailed, error));
    } finally {
      exportingBundleRef.current = false;
    }
  };

  /* 分组管理（X1：空分组此前无法删除/重命名）。右键分组卡弹出。 */
  const [folderMenu, setFolderMenu] = useState<{ x: number; y: number; name: string } | null>(null);
  useEffect(() => {
    if (!folderMenu) return;
    const onDown = (e: PointerEvent) => {
      if (!(e.target as HTMLElement)?.closest?.('.book-ctx-menu')) setFolderMenu(null);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setFolderMenu(null); };
    document.addEventListener('pointerdown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [folderMenu]);
  const renameGroup = async (name: string) => {
    const next = (await appPrompt({ title: t.library.renameGroup, initial: name }))?.trim();
    if (!next || next === name) return;
    if (extraGroups.includes(next) || folders.map.has(next)) return;
    if (extraGroups.includes(name)) persistExtraGroups(extraGroups.map((g) => (g === name ? next : g)));
    for (const book of books.filter((b) => (b.folder ?? '').trim() === name)) onSetFolder(book.id, next);
    if (openFolder === name) setOpenFolder(next);
  };
  const deleteGroup = async (name: string) => {
    if (!(await appConfirm({ title: t.library.deleteGroupConfirm, danger: true }))) return;
    for (const book of books.filter((b) => (b.folder ?? '').trim() === name)) onSetFolder(book.id, '');
    persistExtraGroups(extraGroups.filter((g) => g !== name));
    if (openFolder === name) setOpenFolder(null);
  };
  const [sort, setSort] = useState<LibrarySortKey>('recent');
  const clipRef = useRef<HTMLInputElement>(null);
  const [clipUrl, setClipUrl] = useState('');
  const [clipOpen, setClipOpen] = useState(false);

  const wb = useLibraryWorkbench({ books, tasks, onTaskCreated, onTaskUpdated, surfaceStyle });
  const {
    currentBook, catalogBooks, searchQuery, setSearchQuery,
    fileInputRef, handleFileChange,
    selectedFile, setSelectedFile, startTranslation, uploading, errorMessage, importing,
    config, setConfig, glossaryDecks, selectedDeckIds, toggleSelectedDeck,
    selectedBook, setSelectedBook, selectedTask, setSelectedTask,
    historyOpen, setHistoryOpen, captureWebArticle,
    cancelTask, resumeTask, retryTask, openArtifact, revealArtifact,
    selectedTaskNotices, clearTaskNotices,
  } = wb;

  const filtered = sortLibraryBooks(
    catalogBooks.filter((b) => {
      switch (filter) {
        case 'favorite': return Boolean(b.favorite);
        case 'translated': return b.progress >= 100;
        case 'read': return b.progress > 0;
        case 'unread': return b.progress === 0;
        /* untranslated：书架只展示书目；待启动/失败任务卡见下方任务区 */
        case 'untranslated': return b.progress < 100;
        default: return true;
      }
    }),
    sort,
  );
  /* 翻译中/失败任务：已翻译类里显示百分比圆环；待翻译类展示待启动任务 */
  const activeTasks = tasks.filter((task) => task.status === 'processing' || task.status === 'pending');
  /* 待翻译分类：展示失败任务（可重试），其余为导入未翻译的提示 */
  const failedTasks = filter === 'untranslated' ? tasks.filter((task) => task.status === 'failed') : [];
  /* 文件夹分组：folder → books；未分组直接铺排 */
  const folders = useMemo(() => {
    const map = new Map<string, Book[]>();
    const ungrouped: Book[] = [];
    for (const book of filtered) {
      const folder = (book.folder ?? '').trim();
      if (!folder) { ungrouped.push(book); continue; }
      const list = map.get(folder) ?? [];
      list.push(book);
      map.set(folder, list);
    }
    return { map, ungrouped };
  }, [filtered]);

  const folderInView = openFolder ? folders.map.get(openFolder) : null;
  const inFolderBooks = folderInView ?? [];
  const visibleUngrouped = openFolder ? [] : folders.ungrouped;

  /* 拖拽：书 → 分组卡片（加入）；书 → 书（新建分组）；分组卡片 → 分组卡片（合并） */
  const handleDragStartBook = (bookId: string, folder: string | null) => {
    dragBookIdRef.current = bookId;
    dragFolderRef.current = folder;
  };
  const handleDropOnFolder = (folder: string) => {
    const bookId = dragBookIdRef.current;
    const fromFolder = dragFolderRef.current;
    dragBookIdRef.current = null;
    dragFolderRef.current = null;
    setDropTarget(null);
    /* 文件夹 A 拖到 B：合并整组（bookId 为空、fromFolder 有值）。
       用未过滤的 books 全集取来源分组（folders.map 只含筛选可见书，会拆散源组）。 */
    if (!bookId) {
      if (fromFolder && fromFolder !== folder) {
        const source = books.filter((b) => (b.folder ?? '').trim() === fromFolder);
        for (const book of source) onSetFolder(book.id, folder);
        setOpenFolder(folder);
      }
      return;
    }
    /* 书拖到分组卡：只移动这一本（含跨组移动）。 */
    onSetFolder(bookId, folder);
  };
  const handleDropOnBook = async (targetBook: Book) => {
    const bookId = dragBookIdRef.current;
    dragBookIdRef.current = null;
    dragFolderRef.current = null;
    setDropTarget(null);
    if (!bookId || bookId === targetBook.id) return;
    const name = await appPrompt({ title: t.library.createFolderPrompt });
    if (!name?.trim()) return;
    const folder = name.trim();
    onSetFolder(bookId, folder);
    onSetFolder(targetBook.id, folder);
  };

  /* 翻译中百分比圆环：按书 id 找进行中任务的进度 */
  const progressRingFor = (bookId: string): number | null => {
    const task = activeTasks.find((tk) => tk.id === bookId);
    return task ? task.progress : null;
  };

  const hero = currentBook ?? filtered[0];
  const heroExcerpt = hero ? displayBookExcerpt(hero) : '';
  const heroTitle = hero ? displayBookTitle(hero, t.library.untitledBook) : '';

  const submitClip = async () => {
    if (!clipUrl.trim()) return;
    await captureWebArticle(clipUrl.trim());
    setClipUrl('');
    setClipOpen(false);
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }}
      className="page-stage"
    >
      <input
        ref={fileInputRef}
        type="file"
        accept="application/pdf,.pdf,application/epub+zip,.epub,.docx,.md,.markdown,.txt"
        hidden
        onChange={handleFileChange}
      />

      {/* 顶部栏：搜索 + 剪藏 + 导入 + 新建翻译 */}
      <div className="lib-topline">
        <label className="lib-search">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
            <circle cx="11" cy="11" r="6.5" /><path d="M16 16l4.5 4.5" />
          </svg>
          <input value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} placeholder={t.library.search} />
        </label>
        <button type="button" className="iconbtn" onClick={() => setClipOpen((v) => !v)} aria-label={t.library.clip}>
          <Globe size={20} strokeWidth={1.7} />
        </button>
        <button type="button" className="iconbtn" onClick={() => fileInputRef.current?.click()} aria-label={t.library.importFile}>
          <Upload size={18} strokeWidth={1.8} />
        </button>
        <button
          type="button"
          className="iconbtn"
          onClick={() => void handleImportTranslated()}
          disabled={importingTranslated}
          aria-label={t.library.importTranslatedBook}
          title={t.library.importTranslatedBook}
        >
          <FolderInput size={18} strokeWidth={1.7} />
        </button>
        <button
          type="button"
          className="iconbtn"
          onClick={() => setHistoryOpen(true)}
          title={t.library.taskHistory}
          aria-label={t.library.taskHistory}
        >
          <History size={18} strokeWidth={1.8} />
        </button>
        <button type="button" className="btn sm" onClick={() => setNewTaskOpen(true)}>
          <Plus size={14} />
          {t.library.newTask}
        </button>
      </div>

      {clipOpen && (
        <div className="lib-search" style={{ marginBottom: 18 }}>
          <input
            ref={clipRef}
            value={clipUrl}
            onChange={(e) => setClipUrl(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && void submitClip()}
            placeholder="https://…"
            autoFocus
          />
          <button type="button" className="btn sm" onClick={() => void submitClip()}>{t.library.clip}</button>
        </div>
      )}

      <header className="page-head">
        <h1 className="page-title">{t.library.title}</h1>
        <div className="actions">
          {/* 主筛选（单选）：全部/已译/待译/收藏/已读/未读——六选一并集，
              消除旧版「状态组 + 阅读维度组」两个"全部"并存的歧义。 */}
          {([
            ['all', t.library.all],
            ['translated', t.library.filterTranslated],
            ['untranslated', t.library.filterUntranslated],
            ['favorite', t.library.favorite],
            ['read', t.library.readRead],
            ['unread', t.library.readUnread],
          ] as const).map(([key, label]) => (
            <button key={key} type="button" className={`chip ${filter === key ? 'on' : ''}`} onClick={() => setFilter(key)}>
              {label}
            </button>
          ))}
          <select
            className="chip"
            style={{ border: 0 }}
            value={sort}
            onChange={(e) => setSort(e.target.value as LibrarySortKey)}
            aria-label={t.library.sort}
          >
            <option value="recent">{t.library.sortRecent}</option>
            <option value="title">{t.library.sortTitle}</option>
            <option value="progress">{t.library.sortProgress}</option>
          </select>
        </div>
      </header>

      {/* hero：封面完整平铺 */}
      {hero ? (
        <button type="button" className="hero-book" onClick={() => onOpenBook(hero)}>
          <div className="hcover">
            {hero.cover ? <img src={hero.cover} alt={heroTitle} /> : (
              <div className="vph" style={{ background: hueFor(hero.id), width: '100%', height: '100%', borderRadius: 8 }}>
                <div style={{ fontFamily: 'var(--font-display)', fontWeight: 600, padding: 16, textAlign: 'center' }}>{heroTitle}</div>
              </div>
            )}
          </div>
          <div className="hmeta">
            <div className="tags">
              <span className="chip"><span className="dot" />&nbsp;{t.library.latest}</span>
              <span className="chip">{hero.format || hero.category} · {t.library.bilingual}</span>
              <span className="chip">{hero.tocCount ?? hero.toc?.length ?? 0}{t.library.chapters}</span>
            </div>
            <h3>{heroTitle}</h3>
            <p className="author">{hero.author} · {t.library.translated}</p>
            {heroExcerpt && <p className="excerpt">{heroExcerpt}</p>}
            <div className="foot">
              {hero.progress > 0 && (
                <>
                  <span className="pct">{hero.progress}%</span>
                  <div className="bar"><i style={{ width: `${hero.progress}%` }} /></div>
                </>
              )}
              <span className="btn sm">{hero.progress > 0 ? t.library.continue : t.library.start}</span>
            </div>
          </div>
        </button>
      ) : (
        <div className="empty-hint">
          <p style={{ fontSize: 16, fontWeight: 600 }}>{t.library.empty1}</p>
          <p style={{ fontSize: 12.5, marginTop: 6 }}>{t.library.empty2}</p>
        </div>
      )}

      {/* 待翻译分类：失败任务重试条 */}
      {failedTasks.length > 0 && (
        <div className="failed-task-strip">
          {failedTasks.map((task) => (
            <div key={task.id} className="failed-task-card">
              <div className="min-w-0 flex-1">
                <div className="truncate" style={{ fontSize: 13, fontWeight: 600 }}>{task.filename}</div>
                <div className="text-muted" style={{ fontSize: 11.5 }}>{task.message}</div>
              </div>
              <button type="button" className="btn ghost sm" onClick={() => void retryTask(task)}>
                {t.tasks.retry}
              </button>
            </div>
          ))}
        </div>
      )}

      {/* 竖封网格 + 文件夹分组：分组卡片 → 点入分组视图；拖拽成组/入组/合并 */}
      {filtered.length > 0 && (
        <>
          <div className="shelf-label">
            <span>{openFolder ?? t.library.shelfAll}</span>
            {openFolder && (
              <span
                className={`chip ${dropTarget === '__ungroup__' ? 'on' : ''}`}
                onDragOver={(e) => { e.preventDefault(); setDropTarget('__ungroup__'); }}
                onDragLeave={() => setDropTarget((d) => (d === '__ungroup__' ? null : d))}
                onDrop={(e) => {
                  e.preventDefault();
                  const bookId = dragBookIdRef.current;
                  dragBookIdRef.current = null;
                  dragFolderRef.current = null;
                  setDropTarget(null);
                  if (bookId) onSetFolder(bookId, '');
                }}
                onClick={() => setOpenFolder(null)}
                title={t.library.ungroupDrop}
              >
                ← {t.library.backToAll}
              </span>
            )}
            {!openFolder && (
              <button type="button" className="chip" onClick={() => void createGroup()} title={t.library.folderHint}>
                <FolderPlus size={13} />
                {t.library.newGroup}
              </button>
            )}
          </div>
          {!openFolder && folders.map.size > 0 && (
            <p className="folder-hint">{t.library.folderHint}</p>
          )}
          <div className="vshelf">
            {!openFolder && extraGroups.filter((name) => !folders.map.has(name)).map((name) => (
              <button
                key={`folder-${name}`}
                type="button"
                className={`vfolder vfolder-empty ${dropTarget === name ? 'drop-on' : ''}`}
                onDragOver={(e) => { e.preventDefault(); setDropTarget(name); }}
                onDragLeave={() => setDropTarget((d) => (d === name ? null : d))}
                onDragEnd={() => setDropTarget(null)}
                onDrop={(e) => { e.preventDefault(); handleDropOnFolder(name); }}
                onClick={() => setOpenFolder(name)}
                onContextMenu={(e) => { e.preventDefault(); setFolderMenu({ x: e.clientX, y: e.clientY, name }); }}
              >
                <div className="vfolder-stack vfolder-stack-empty" aria-hidden="true">
                  <FolderPlus size={26} style={{ color: 'var(--ink-4)' }} />
                </div>
                <div className="vmeta">
                  <h3>{name}</h3>
                  <div className="author">0 {t.library.folderBooks}</div>
                </div>
              </button>
            ))}
            {!openFolder && Array.from(folders.map.entries()).map(([name, list]) => (
              <button
                key={`folder-${name}`}
                type="button"
                className={`vfolder ${dropTarget === name ? 'drop-on' : ''}`}
                draggable
                onDragStart={(e) => { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', `folder:${name}`); dragFolderRef.current = name; dragBookIdRef.current = null; }}
                onDragEnd={() => setDropTarget(null)}
                onDragOver={(e) => { e.preventDefault(); setDropTarget(name); }}
                onDragLeave={() => setDropTarget((d) => (d === name ? null : d))}
                onDrop={(e) => { e.preventDefault(); handleDropOnFolder(name); }}
                onClick={() => setOpenFolder(name)}
                onContextMenu={(e) => { e.preventDefault(); setFolderMenu({ x: e.clientX, y: e.clientY, name }); }}
              >
                <div className="vfolder-stack" aria-hidden="true">
                  {list.slice(0, 3).map((b, i) => (
                    b.cover ? (
                      <img key={b.id} src={b.cover} alt="" className="vfolder-cover" style={{ zIndex: 3 - i }} />
                    ) : (
                      <div key={b.id} className="vfolder-ph" style={{ background: hueFor(b.id), zIndex: 3 - i }} />
                    )
                  ))}
                </div>
                <div className="vmeta">
                  <h3>{name}</h3>
                  <div className="author">{list.length} {t.library.folderBooks}</div>
                </div>
              </button>
            ))}
            {(openFolder ? inFolderBooks : visibleUngrouped).map((b) => (
              <button
                key={b.id}
                type="button"
                className={`vbook ${dropTarget === b.id ? 'drop-on' : ''}`}
                draggable
                onDragStart={(e) => { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', `book:${b.id}`); handleDragStartBook(b.id, b.folder ?? null); }}
                onDragEnd={() => setDropTarget(null)}
                onDragOver={(e) => { e.preventDefault(); setDropTarget(b.id); }}
                onDragLeave={() => setDropTarget((d) => (d === b.id ? null : d))}
                onDrop={(e) => { e.preventDefault(); void handleDropOnBook(b); }}
                onClick={() => setSelectedBook(b)}
                onContextMenu={(e) => { e.preventDefault(); setCtxMenu({ x: e.clientX, y: e.clientY, book: b }); }}
              >
                <div className="vwrap">
                  {b.cover ? <img src={b.cover} alt={b.title} /> : (
                    <div className="vph" style={{ background: hueFor(b.id) }}>
                      <div>
                        <div style={{ fontFamily: 'var(--font-display)', fontWeight: 600, fontSize: 15, lineHeight: 1.3 }}>{displayBookTitle(b, t.library.untitledBook)}</div>
                        <div style={{ fontSize: 11, color: 'var(--ink-2)', marginTop: 8 }}>{b.author}</div>
                      </div>
                    </div>
                  )}
                  {b.progress >= 100 && <span className="badge">{t.library.finished}</span>}
                  {progressRingFor(b.id) !== null && progressRingFor(b.id) !== undefined && (
                    <span className="progress-ring" title={t.tasks.running}>
                      <svg viewBox="0 0 36 36" aria-hidden="true">
                        <circle cx="18" cy="18" r="15" fill="none" stroke="rgba(28,28,26,.15)" strokeWidth="4" />
                        <circle
                          cx="18" cy="18" r="15" fill="none" stroke="var(--signal, #b45309)" strokeWidth="4"
                          strokeDasharray={`${(progressRingFor(b.id) ?? 0) * 94.2 / 100} 94.2`}
                          strokeLinecap="round" transform="rotate(-90 18 18)"
                        />
                      </svg>
                      <b>{progressRingFor(b.id) ?? 0}%</b>
                    </span>
                  )}
                  {/* span 而非 button：外层 vbook 已是 button，HTML 不允许 button 嵌套 */}
                  <span
                    role="button"
                    tabIndex={0}
                    className={`favbtn ${b.favorite ? 'on' : ''}`}
                    aria-label={t.library.favorite}
                    aria-pressed={Boolean(b.favorite)}
                    onClick={(e) => { e.stopPropagation(); onToggleFavorite(b.id, !b.favorite);
                      if (!b.favorite) toast.success(t.library.ctxFavorite);
                      else toast.info(t.library.ctxUnfavorite); }}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault(); e.stopPropagation();
                        onToggleFavorite(b.id, !b.favorite);
                        if (!b.favorite) toast.success(t.library.ctxFavorite);
                        else toast.info(t.library.ctxUnfavorite);
                      }
                    }}
                  >
                    <Star size={13} strokeWidth={1.9} fill={b.favorite ? 'currentColor' : 'none'} />
                  </span>
                </div>
                <div className="vmeta">
                  <h3>{displayBookTitle(b, t.library.untitledBook)}</h3>
                  <div className="author">{b.author}</div>
                  {b.progress > 0 && b.progress < 100 && (
                    <>
                      <div className="bar"><i style={{ width: `${b.progress}%` }} /></div>
                      <div className="pct">{b.progress}%</div>
                    </>
                  )}
                </div>
              </button>
            ))}
          </div>
        </>
      )}

      {/* 分组右键菜单（X1：重命名/删除）——portal 到 body：
          page-stage（motion transform）会让 position:fixed 失效，菜单随滚动大偏移 */}
      {folderMenu && createPortal(
        <div
          className="book-ctx-menu"
          style={{
            left: Math.min(folderMenu.x, window.innerWidth - 202),
            top: Math.min(folderMenu.y, window.innerHeight - 120),
          }}
          role="menu"
        >
          <button
            type="button"
            onClick={() => { void renameGroup(folderMenu.name); setFolderMenu(null); }}
          >
            <Pencil size={14} className="ctx-muted" />
            {t.library.renameGroup}
          </button>
          <button type="button" className="ctx-danger" onClick={() => { void deleteGroup(folderMenu.name); setFolderMenu(null); }}>
            <Trash2 size={14} />
            {t.library.deleteGroup}
          </button>
        </div>,
        document.body,
      )}

            {/* 悬空防护：菜单打开期间书可能被任务事件移除——书不在列表时收起 */}
      {ctxMenu && books.some((b) => b.id === ctxMenu.book.id) && (
        <BookContextMenu
          state={ctxMenu}
          onClose={() => setCtxMenu(null)}
          onOpen={onOpenBook}
          onToggleFavorite={onToggleFavorite}
          onTranslate={(book) => setConfigTarget(book)}
          onVocab={(book) => (onOpenVocab ?? onOpenBook)(book)}
          onReview={onOpenReview}
          onDetail={(book) => setSelectedBook(book)}
          onExport={(book) => void handleExportBundle(book)}
          onDelete={(taskId) => onDeleteTask(taskId)}
        />
      )}

      {configTarget && (
        <TranslateConfigModal
          book={configTarget}
          glossaryDecks={glossaryDecks}
          selectedDeckIds={selectedDeckIds}
          toggleDeck={toggleSelectedDeck}
          skills={skillEntries}
          onClose={() => setConfigTarget(null)}
          onStart={async (cfg) => {
            setConfigTarget(null);
            await startTranslation({
              articleType: cfg.articleType,
              concurrent: cfg.concurrent,
              maxWorkers: cfg.maxWorkers,
              deckIds: cfg.deckIds,
              sourcePath: configTarget.originalPath ?? configTarget.primaryArtifactPath,
            });
          }}
        />
      )}

      {/* 新建翻译：先配置（上传/书库选书 + 全部可配置项）再开翻 */}
      {newTaskOpen && (
        <TranslateConfigModal
          book={hero ?? books[0]}
          glossaryDecks={glossaryDecks}
          selectedDeckIds={selectedDeckIds}
          toggleDeck={toggleSelectedDeck}
          skills={skillEntries}
          pick={{
            books,
            onPicked: (sourcePath, _title, cfg) => {
              setNewTaskOpen(false);
              /* 上传的新文件/书库书籍：用弹窗当次配置直接开翻。 */
              void startTranslation({
                articleType: cfg.articleType,
                concurrent: cfg.concurrent,
                maxWorkers: cfg.maxWorkers,
                deckIds: cfg.deckIds,
                sourcePath,
              });
            },
          }}
          onClose={() => setNewTaskOpen(false)}
          onStart={async () => {
            setNewTaskOpen(false);
          }}
        />
      )}

      <PreReadModal
        book={selectedBook}
        onClose={() => setSelectedBook(null)}
        onStartReading={onOpenBook}
        onToggleFavorite={onToggleFavorite}
        onSetTags={onSetTags}
      />

      {/* 待启动条：选完文件 / 剪藏完成 → 一键开翻 */}
      {selectedFile && (
        <div className="card pending-row">
          <div className="pending-ico"><Plus size={16} strokeWidth={2} /></div>
          <div className="min-w-0 flex-1">
            <div className="truncate pending-name">{selectedFile.name}</div>
            <div className="pending-sub">{t.tasks.pending}</div>
          </div>
          <select
            className="chip"
            value={config.articleType}
            onChange={(e) => setConfig({ ...config, articleType: e.target.value })}
            style={{ border: 0 }}
          >
            <option value="fiction">fiction</option>
            <option value="academic">academic</option>
            <option value="nonfiction">nonfiction</option>
            <option value="tech_doc">tech_doc</option>
            <option value="textbook">textbook</option>
          </select>
          {glossaryDecks.length > 0 && (
            <div className="deck-pick" title={t.tasks.glossaryPickTitle}>
              {glossaryDecks.map((deck) => (
                <button
                  key={deck.id}
                  type="button"
                  className={`chip sm ${deck.enabled && selectedDeckIds.includes(deck.id) ? 'on' : ''}`}
                  disabled={!deck.enabled}
                  onClick={() => toggleSelectedDeck(deck.id)}
                >
                  {deck.name}
                </button>
              ))}
            </div>
          )}
          <button type="button" className="btn sm" disabled={uploading} onClick={() => void startTranslation()}>
            {uploading ? '…' : t.tasks.new}
          </button>
          <button type="button" className="iconbtn danger" onClick={() => setSelectedFile(null)} aria-label={t.tasks.cancel}>
            <X size={15} />
          </button>
        </div>
      )}
      {(errorMessage || importing) && (
        <p className={`pending-msg ${errorMessage ? 'is-error' : ''}`}>{importing ? '…' : errorMessage}</p>
      )}

      {selectedTask && (
        <LibraryTaskDetailSheet
          task={selectedTask}
          book={books.find((book) => book.id === selectedTask.id)}
          notices={selectedTaskNotices}
          onClose={() => setSelectedTask(null)}
          onCancel={(task) => void cancelTask(task)}
          onResume={(task) => void resumeTask(task)}
          onRetry={(task) => void retryTask(task)}
          onOpenArtifact={(path) => void openArtifact(path)}
          onRevealArtifact={(path) => void revealArtifact(path)}
          onClearNotices={(taskId) => void clearTaskNotices(taskId)}
        />
      )}

      {historyOpen && (
        <LibraryTaskHistorySheet
          tasks={tasks}
          books={books}
          selectedTaskId={selectedTask?.id}
          onClose={() => setHistoryOpen(false)}
          onSelectTask={(task) => { setHistoryOpen(false); setSelectedTask(task); }}
        />
      )}
    </motion.div>
  );
}
