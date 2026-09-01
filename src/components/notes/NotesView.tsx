import { invoke } from '@tauri-apps/api/core';
import { motion } from 'motion/react';
import { ArrowDownAZ, ArrowUpAZ, CalendarDays, ListOrdered, Check, Download, NotebookPen, PencilLine, Search, Trash2, X } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import { useLang } from '@/i18n/context';
import { useAppDialog } from '@/components/common/AppDialog';
import { isWebPreview, WEB_PREVIEW_READER_STATES } from '@/hooks/webPreviewFixtures';
import type { Book, ReaderState } from '@/types';
import { buildNotesSummary, type NotesItem } from './notesShared';

interface NotesViewProps {
  books: Book[];
  defaultView: 'cards' | 'feed';
  /** 跳转到书籍（笔记→书并支持跳回）。 */
  onOpenBook?: (book: Book, anchorHref?: string | null) => void;
}

/** 笔记页 v2：搜索 + 按书分组 + 编辑/删除/导出/跳书。
    布局为笔记本式：左侧书脊分组栏，右侧笔记行（横线纸感）。 */
export function NotesView({ books, onOpenBook }: NotesViewProps) {
  const t = useT();
  const { lang } = useLang();
  const { appConfirm } = useAppDialog();
  const [query, setQuery] = useState('');
  const [activeBookId, setActiveBookId] = useState<string>('');
  const [readerStates, setReaderStates] = useState<ReaderState[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState('');
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    if (isWebPreview) {
      setReaderStates(WEB_PREVIEW_READER_STATES);
      return;
    }
    let mounted = true;
    const load = async () => {
      try {
        const states = await invoke<ReaderState[]>('list_reader_states');
        if (mounted) setReaderStates(states);
      } catch (error) {
        console.error('Failed to load reader states:', error);
      }
    };
    void load();
    return () => { mounted = false; };
  }, [refreshKey]);

  const summary = useMemo(() => buildNotesSummary(books, readerStates, lang, 'all'), [books, readerStates, lang]);

  /* 按书分组 */
  const byBook = useMemo(() => {
    const map = new Map<string, NotesItem[]>();
    for (const item of summary.items) {
      const list = map.get(item.bookId) ?? [];
      list.push(item);
      map.set(item.bookId, list);
    }
    return map;
  }, [summary.items]);

  /* 侧栏悬空回退：选中书的全部条目被删后分组消失——回退到「全部」。 */
  useEffect(() => {
    if (activeBookId && !byBook.has(activeBookId)) setActiveBookId('');
  }, [byBook, activeBookId]);

  const bookGroups = useMemo(() => {
    const titleOf = (bookId: string) =>
      books.find((b) => b.id === bookId)?.title ?? (lang === 'zh' ? '未知书籍' : 'Unknown book');
    return Array.from(byBook.entries()).map(([bookId, items]) => ({ bookId, title: titleOf(bookId), items }));
  }, [byBook, books, lang]);

  /* 排序：日期 / 章节顺序 × 升降序。章节序用 progress 近似（章内再按日期）。 */
  const [sortKey, setSortKey] = useState<'date' | 'chapter'>('date');
  const [sortAsc, setSortAsc] = useState(false);
  const visibleItems = useMemo(() => {
    let items = activeBookId ? byBook.get(activeBookId) ?? [] : summary.items;
    const q = query.trim().toLowerCase();
    if (q) {
      items = items.filter((i) =>
        i.note.toLowerCase().includes(q)
        || i.quote.toLowerCase().includes(q)
        || i.source.toLowerCase().includes(q)
        || i.chapter.toLowerCase().includes(q));
    }
    const dir = sortAsc ? 1 : -1;
    return [...items].sort((a, b) => {
      if (sortKey === 'chapter') {
        const byChapter = (a.progress - b.progress) * dir;
        if (byChapter !== 0) return byChapter;
      }
      return (new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime()) * dir;
    });
  }, [byBook, summary.items, activeBookId, query, sortKey, sortAsc]);

  const exportMarkdown = () => {
    const lines: string[] = [`# ${t.notes.title}`, ''];
    for (const group of bookGroups) {
      lines.push(`## ${group.title}`, '');
      for (const item of group.items) {
        lines.push(`- **${item.kind === 'bmk' ? t.notes.bookmarks : t.notes.anno}** · ${item.chapter} · ${item.createdAt.slice(0, 10)}`);
        if (item.quote) lines.push(`  > ${item.quote}`);
        lines.push(`  ${item.note}`, '');
      }
    }
    const blob = new Blob([lines.join('\n')], { type: 'text/markdown' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'notes-export.md';
    a.click();
    URL.revokeObjectURL(url);
  };

  const startEdit = (item: NotesItem) => {
    setEditingId(item.id);
    setEditDraft(item.note);
  };

  const saveEdit = async (item: NotesItem) => {
    try {
      await invoke('update_note', { bookId: item.bookId, noteId: item.id, note: editDraft });
      setEditingId(null);
      setRefreshKey((k) => k + 1);
    } catch (error) {
      console.error('update note failed:', error);
    }
  };

  const deleteItem = async (item: NotesItem) => {
    if (!(await appConfirm({ title: t.notes.deleteConfirm, message: item.note.slice(0, 60), danger: true }))) return;
    try {
      if (item.kind === 'anno') {
        await invoke('remove_note', { bookId: item.bookId, noteId: item.id });
      } else {
        await invoke('remove_bookmark', { bookId: item.bookId, bookmarkId: item.id });
      }
      setRefreshKey((k) => k + 1);
    } catch (error) {
      console.error('delete note failed:', error);
    }
  };

  const jumpToBook = (item: NotesItem) => {
    const book = books.find((b) => b.id === item.bookId);
    if (book && onOpenBook) onOpenBook(book, item.href ?? null);
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1] }}
      className="page-stage"
    >
      <header className="page-head">
        <h1 className="page-title">{t.notes.title}</h1>
        <div className="actions">
          <div className="searchbox sm">
            <Search size={13} />
            <input value={query} placeholder={t.notes.searchPlaceholder} onChange={(e) => setQuery(e.target.value)} />
          </div>
          {/* 排序：日期/章节 × 升降序，图标化 */}
          <button
            type="button"
            className={`chip sm ${sortKey === 'date' ? 'on' : ''}`}
            onClick={() => setSortKey('date')}
            title={lang === 'zh' ? '按日期排序' : 'Sort by date'}
            aria-label="sort by date"
          >
            <CalendarDays size={12} />
          </button>
          <button
            type="button"
            className={`chip sm ${sortKey === 'chapter' ? 'on' : ''}`}
            onClick={() => setSortKey('chapter')}
            title={lang === 'zh' ? '按章节顺序排序' : 'Sort by chapter order'}
            aria-label="sort by chapter"
          >
            <ListOrdered size={12} />
          </button>
          <button
            type="button"
            className="chip sm"
            onClick={() => setSortAsc((v) => !v)}
            title={sortAsc ? (lang === 'zh' ? '切换为降序' : 'Switch to descending') : (lang === 'zh' ? '切换为升序' : 'Switch to ascending')}
            aria-label={sortAsc ? 'ascending' : 'descending'}
          >
            {sortAsc ? <ArrowUpAZ size={12} /> : <ArrowDownAZ size={12} />}
          </button>
          <button type="button" className="btn ghost sm" onClick={exportMarkdown} disabled={summary.items.length === 0}>
            <Download size={14} />
            {t.notes.export}
          </button>
        </div>
      </header>

      <div className="notes-v2">
        {/* 左侧书脊分组栏 */}
        <aside className="notes-v2-rail">
          <button
            type="button"
            className={`notes-rail-item ${activeBookId === '' ? 'on' : ''}`}
            onClick={() => setActiveBookId('')}
          >
            <span className="truncate">{t.notes.allBooks}</span>
            <span className="notes-rail-count">{summary.items.length}</span>
          </button>
          {bookGroups.map((group) => (
            <button
              key={group.bookId}
              type="button"
              className={`notes-rail-item ${activeBookId === group.bookId ? 'on' : ''}`}
              onClick={() => setActiveBookId(group.bookId)}
            >
              <span className="truncate">{group.title}</span>
              <span className="notes-rail-count">{group.items.length}</span>
            </button>
          ))}
        </aside>

        {/* 右侧笔记行（笔记本横线纸感） */}
        <div className="notes-v2-main">
          {visibleItems.length === 0 && (
            <div className="empty-hint">
              <NotebookPen size={26} strokeWidth={1.5} style={{ margin: '0 auto 10px', color: 'var(--ink-4)' }} />
              {t.notes.empty}
            </div>
          )}
          {visibleItems.map((item) => (
            <article key={item.id} className="notes-v2-row">
              <div className="notes-v2-row-head">
                <span className={`notes-kind ${item.kind}`}>
                  {item.kind === 'bmk' ? t.notes.bookmarks : t.notes.anno}
                </span>
                <span className="notes-v2-src">{item.source}</span>
                <span className="notes-v2-meta">{item.chapter} · {item.createdAt.slice(0, 10)}</span>
                <span className="notes-v2-actions">
                  {editingId !== item.id && (
                    <button type="button" className="notes-act" title={t.notes.editNote} onClick={() => startEdit(item)}>
                      <PencilLine size={13} />
                    </button>
                  )}
                  <button type="button" className="notes-act" title={t.notes.jumpToBook} onClick={() => jumpToBook(item)}>
                    <NotebookPen size={13} />
                  </button>
                  <button type="button" className="notes-act danger" title={t.notes.deleteNote} onClick={() => void deleteItem(item)}>
                    <Trash2 size={13} />
                  </button>
                </span>
              </div>
              {item.quote && <p className="notes-v2-quote">“{item.quote}”</p>}
              {editingId === item.id ? (
                <div className="notes-v2-edit">
                  <textarea
                    value={editDraft}
                    rows={3}
                    autoFocus
                    onChange={(e) => setEditDraft(e.target.value)}
                    onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void saveEdit(item); }}
                  />
                  <div className="notes-v2-edit-actions">
                    <button type="button" className="chip" onClick={() => setEditingId(null)}><X size={12} />{t.notes.cancelEdit}</button>
                    <button type="button" className="chip on" onClick={() => void saveEdit(item)}><Check size={12} />{t.notes.saveEdit}</button>
                  </div>
                </div>
              ) : (
                <p className="notes-v2-note">{item.note}</p>
              )}
            </article>
          ))}
        </div>
      </div>
    </motion.div>
  );
}
