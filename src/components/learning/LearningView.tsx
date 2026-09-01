import { invoke } from '@tauri-apps/api/core';
import { BarChart3, BookOpen, Layers, NotebookText, PenLine, Play, Plus, Search, Trash2, X } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import { useAppDialog } from '@/components/common/AppDialog';
import type { Book, ReaderState, VocabNotebook } from '@/types';
import { isWebPreview, WEB_PREVIEW_READER_STATES } from '@/hooks/webPreviewFixtures';
import type { VocabBucket } from '@/components/insights/insightsShared';
import { LevelBars } from './LevelBars';
import { NotebookDeck } from './NotebookDeck';
import { NotebookPicker } from './NotebookPicker';
import { GlobalVocabQueue } from './GlobalVocabQueue';
import { StudyOverlay } from './StudyOverlay';

/**
 * 学习视图 v3（Cherry Studio 式三区布局，与术语表同设计语言）：
 *  - 左栏：书籍列表 + 自建生词本树（新建/删除/选中即切换数据源）
 *  - 右区上：统计条 + 难度分布图（分级词库真实档位，可点选筛选）+ 生词本书籍分布
 *  - 右区下：词表（搜索/多选/移入生词本/删除）+ Anki 式背诵（书籍卡组或生词本卡组）
 * 移除旧版四卡片 hub 与上下矛盾的卡片/选项布局。
 */

type SourceKind =
  | { kind: 'all' }
  | { kind: 'book'; bookId: string; title: string }
  | { kind: 'notebook'; notebookId: string; name: string };

/** 词行统一视图：书籍 vocabCards 与生词本 entries 归一。 */
interface VocabRow {
  word: string;
  chapter: string;
  bookId: string;
  dueAt?: string | null;
  repetitions: number;
  context: string;
  definition: string;
  note?: string;
}

/** 分级词库档位顺序（与后端 WordLevel 枚举一致）。 */
const LEVEL_ORDER = ['中考', '高考', '四级', '六级', '考研', '雅思', '托福', '专四', '专八', 'GRE'];

function bookRows(state: ReaderState): VocabRow[] {
  return Object.values(state.vocabCards).map((card) => ({
    word: card.word,
    chapter: card.chapter,
    bookId: state.bookId,
    dueAt: card.dueAt,
    repetitions: card.repetitions,
    context: card.context,
    definition: card.customDefinition ?? '',
    note: card.note ?? '',
  }));
}

function notebookRows(notebook: VocabNotebook): VocabRow[] {
  return notebook.entries.map((entry) => ({
    word: entry.word,
    chapter: entry.chapter,
    bookId: entry.bookId,
    dueAt: entry.dueAt,
    repetitions: entry.repetitions,
    context: entry.context,
    definition: entry.definition,
    note: entry.note ?? '',
  }));
}

function isDue(dueAt?: string | null): boolean {
  return !dueAt || new Date(dueAt).getTime() <= Date.now();
}

export function LearningView({ books, initialBookId, onInitialBookConsumed }: {
  books: Book[];
  /** 旧四卡片布局遗留 prop，保留签名兼容但已无消费（重构后仅列表+背诵）。 */
  defaultView?: 'cards' | 'feed';
  initialBookId?: string | null;
  onInitialBookConsumed?: () => void;
}) {
  const t = useT();
  const { appConfirm } = useAppDialog();
  const [readerStates, setReaderStates] = useState<ReaderState[]>([]);
  const [notebooks, setNotebooks] = useState<VocabNotebook[]>([]);
  const [source, setSource] = useState<SourceKind>({ kind: 'all' });
  const [mode, setMode] = useState<'list' | 'study'>('list');
  const [query, setQuery] = useState('');
  const [levelFilter, setLevelFilter] = useState<string | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [picking, setPicking] = useState(false);
  const [newNotebookName, setNewNotebookName] = useState('');
  const [wordInfo, setWordInfo] = useState<Map<string, { level: string; phonetic: string; definition: string }>>(new Map());
  /* 词详情浮层（列表模式点词看详情 + 四档评分 + 笔记编辑）。 */
  const [detailRow, setDetailRow] = useState<VocabRow | null>(null);
  const [detailNote, setDetailNote] = useState('');
  const [detailBusy, setDetailBusy] = useState(false);

  /* ── 数据加载 ── */
  const reloadStates = useCallback(() => {
    if (isWebPreview) return;
    invoke<ReaderState[]>('list_reader_states')
      .then(setReaderStates)
      .catch((error) => console.error('Failed to load reader states:', error));
  }, []);

  const reloadNotebooks = useCallback(() => {
    if (isWebPreview) return;
    invoke<VocabNotebook[]>('list_vocab_notebooks')
      .then(setNotebooks)
      .catch(() => setNotebooks([]));
  }, []);

  useEffect(() => {
    if (isWebPreview) {
      setReaderStates(WEB_PREVIEW_READER_STATES);
      return;
    }
    reloadStates();
    reloadNotebooks();
  }, [reloadStates, reloadNotebooks]);

  /* 书库跳入：选中该书 */
  useEffect(() => {
    if (initialBookId) {
      const book = books.find((b) => b.id === initialBookId);
      if (book) setSource({ kind: 'book', bookId: book.id, title: book.title });
      onInitialBookConsumed?.();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialBookId]);

  /* 当前源词行 */
  const rows = useMemo<VocabRow[]>(() => {
    if (source.kind === 'all') return readerStates.flatMap(bookRows);
    if (source.kind === 'book') {
      const state = readerStates.find((s) => s.bookId === source.bookId);
      return state ? bookRows(state) : [];
    }
    const notebook = notebooks.find((nb) => nb.id === source.notebookId);
    return notebook ? notebookRows(notebook) : [];
  }, [readerStates, notebooks, source]);

  /* 词表信息增强（音标/难度/释义兜底，不做难度过滤） */
  useEffect(() => {
    if (isWebPreview || rows.length === 0) {
      setWordInfo(new Map());
      return;
    }
    let mounted = true;
    const words = [...new Set(rows.map((row) => row.word))].slice(0, 500);
    invoke<{ word: string; level: string; phonetic: string; definition: string }[]>('lookup_words_info_batch', { words })
      .then((info) => {
        if (mounted) setWordInfo(new Map(info.map((item) => [item.word, item])));
      })
      .catch(() => { if (mounted) setWordInfo(new Map()); });
    return () => { mounted = false; };
  }, [rows]);

  /* 难度分布（分级词库档位 + 未收录兜底桶） */
  const levelBuckets = useMemo<VocabBucket[]>(() => {
    const counts = new Map<string, number>();
    let unknown = 0;
    for (const row of rows) {
      const level = wordInfo.get(row.word)?.level;
      if (level) counts.set(level, (counts.get(level) ?? 0) + 1);
      else unknown += 1;
    }
    const buckets = LEVEL_ORDER
      .filter((label) => (counts.get(label) ?? 0) > 0)
      .map((label) => ({ label, count: counts.get(label)! }));
    if (unknown > 0) buckets.push({ label: t.learning.uncategorized, count: unknown });
    return buckets;
  }, [rows, wordInfo, t]);

  /* 统计条：书籍源取 wordMarks 状态；生词本源取调度状态 */
  const stats = useMemo(() => {
    const total = rows.length;
    const due = rows.filter((row) => isDue(row.dueAt)).length;
    if (source.kind === 'notebook') {
      return { total, due, learning: rows.filter((r) => r.repetitions === 0).length, mastered: rows.filter((r) => r.repetitions > 0).length };
    }
    const states = source.kind === 'book' ? readerStates.filter((s) => s.bookId === source.bookId) : readerStates;
    const learning = states.reduce((sum, s) => sum + Object.values(s.wordMarks ?? {}).filter((m) => m.status === 'learning').length, 0);
    const mastered = states.reduce((sum, s) => sum + Object.values(s.wordMarks ?? {}).filter((m) => m.status === 'mastered').length, 0);
    return { total, due, learning, mastered };
  }, [rows, readerStates, source]);

  /* 生词本源：书籍分布 */
  const bookDist = useMemo(() => {
    if (source.kind !== 'notebook') return [];
    const counts = new Map<string, number>();
    for (const row of rows) counts.set(row.bookId, (counts.get(row.bookId) ?? 0) + 1);
    return [...counts.entries()]
      .map(([bookId, count]) => ({ title: books.find((b) => b.id === bookId)?.title ?? t.learning.manualEntry, count }))
      .sort((a, b) => b.count - a.count);
  }, [source, rows, books, t]);

  /* 列表过滤：搜索 + 难度档 */
  const visibleRows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rows.filter((row) => {
      if (q && !row.word.toLowerCase().includes(q)) return false;
      if (!levelFilter) return true;
      const level = wordInfo.get(row.word)?.level;
      return levelFilter === t.learning.uncategorized ? !level : level === levelFilter;
    });
  }, [rows, query, levelFilter, wordInfo, t]);

  const bookTitle = useCallback((bookId: string) => books.find((b) => b.id === bookId)?.title ?? '', [books]);

  /* 词详情：打开（携带当前笔记）/ 四档评分（按源路由书籍卡或生词本条目）/ 笔记保存。 */
  const openDetail = (row: VocabRow) => {
    setDetailRow(row);
    setDetailNote(row.note ?? '');
  };

  /* 列表行直接评分（不点开详情也能按背诵模式四档评分）。
     行级轻量版：成功后刷新本地行状态（rep/due 由后端持有）。 */
  const rateRow = async (row: VocabRow, quality: 0 | 3 | 4 | 5) => {
    try {
      if (source.kind === 'notebook' && activeNotebook) {
        const next = await invoke<VocabNotebook[]>('review_vocab_notebook_entry', {
          notebookId: activeNotebook.id, word: row.word, quality,
        });
        setNotebooks(next);
      } else if (row.bookId) {
        await invoke('review_vocab_card', { bookId: row.bookId, word: row.word, quality });
        reloadStates();
      }
    } catch (error) {
      console.error('rate row failed:', error);
    }
  };

  const rateDetail = async (quality: 0 | 3 | 4 | 5) => {
    if (!detailRow || detailBusy) return;
    setDetailBusy(true);
    try {
      if (source.kind === 'notebook' && activeNotebook) {
        const next = await invoke<VocabNotebook[]>('review_vocab_notebook_entry', {
          notebookId: activeNotebook.id, word: detailRow.word, quality,
        });
        setNotebooks(next);
      } else if (detailRow.bookId) {
        await invoke('review_vocab_card', { bookId: detailRow.bookId, word: detailRow.word, quality });
        reloadStates();
      }
      setDetailRow((cur) => (cur ? { ...cur, repetitions: cur.repetitions + (quality >= 3 ? 1 : 0) } : cur));
    } finally {
      setDetailBusy(false);
    }
  };

  const saveDetailNote = async () => {
    if (!detailRow || detailBusy) return;
    setDetailBusy(true);
    try {
      if (source.kind === 'notebook' && activeNotebook) {
        const next = await invoke<VocabNotebook[]>('save_vocab_notebook_entry_note', {
          notebookId: activeNotebook.id, word: detailRow.word, note: detailNote,
        });
        setNotebooks(next);
      } else if (detailRow.bookId) {
        await invoke('save_vocab_card_note', { bookId: detailRow.bookId, word: detailRow.word, note: detailNote });
        reloadStates();
      }
      setDetailRow((cur) => (cur ? { ...cur, note: detailNote } : cur));
    } finally {
      setDetailBusy(false);
    }
  };

  const sourceTitle = source.kind === 'all' ? t.learning.allBooks : source.kind === 'book' ? source.title : source.name;
  const activeNotebook = source.kind === 'notebook' ? notebooks.find((nb) => nb.id === source.notebookId) ?? null : null;

  /* ── 操作 ── */
  const pickSource = (next: SourceKind) => {
    setSource(next);
    setMode('list');
    setQuery('');
    setLevelFilter(null);
    setChecked(new Set());
  };

  /* 复合键：全部书籍源同词跨书出现，单一 word 作 key/选中键会重复与误选。 */
  const rowKeyOf = (row: VocabRow) => `${row.bookId}:${row.word}`;

  const toggleChecked = (rowKey: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(rowKey)) next.delete(rowKey);
      else next.add(rowKey);
      return next;
    });
  };

  const createNotebook = async () => {
    const name = newNotebookName.trim();
    if (!name) return;
    try {
      const next = await invoke<VocabNotebook[]>('create_vocab_notebook', { name });
      setNotebooks(next);
      setNewNotebookName('');
    } catch (error) {
      console.error('create notebook failed:', error);
    }
  };

  const deleteNotebook = async (notebookId: string) => {
    const notebook = notebooks.find((n) => n.id === notebookId);
    if (!(await appConfirm({ title: t.learning.deleteNotebookConfirm, message: notebook?.name, danger: true }))) return;
    try {
      const next = await invoke<VocabNotebook[]>('delete_vocab_notebook', { notebookId });
      setNotebooks(next);
      if (source.kind === 'notebook' && source.notebookId === notebookId) pickSource({ kind: 'all' });
    } catch (error) {
      console.error('delete notebook failed:', error);
    }
  };

  /* 删除所选：生词本源删词条；书籍源删生词卡（批量破坏性操作，需确认） */
  const removeChecked = async () => {
    const selected = rows.filter((row) => checked.has(rowKeyOf(row)));
    if (selected.length === 0) return;
    if (!(await appConfirm({ title: t.learning.removeWordsConfirm.replace('{n}', String(selected.length)), danger: true }))) return;
    try {
      if (source.kind === 'notebook' && activeNotebook) {
        for (const row of selected) {
          await invoke('remove_vocab_notebook_entry', { notebookId: activeNotebook.id, word: row.word });
        }
        reloadNotebooks();
      } else {
        for (const row of selected) {
          if (row.bookId) await invoke('remove_vocab_card', { bookId: row.bookId, word: row.word });
        }
        reloadStates();
      }
      setChecked(new Set());
    } catch (error) {
      console.error('remove checked words failed:', error);
    }
  };

  const checkedEntries = useMemo(() => rows.filter((row) => checked.has(rowKeyOf(row))), [rows, checked]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="page-stage">
      <header className="page-head">
        <h1 className="page-title">{t.learning.title}</h1>
        <div className="actions" />
      </header>

      <div className="glossary-v2">
        {/* ── 左栏：书籍 + 生词本 ── */}
        <div className="gloss-left">
          <div className="gloss-zone-tabs">
            <button type="button" className={`chip ${source.kind !== 'notebook' ? 'on' : ''}`} onClick={() => pickSource({ kind: 'all' })}>
              <Layers size={13} />
              {t.learning.allBooks}
            </button>
          </div>
          <div className="gloss-list">
            {books.map((book) => {
              const state = readerStates.find((s) => s.bookId === book.id);
              const count = state ? Object.keys(state.vocabCards).length : 0;
              return (
                <div
                  key={book.id}
                  className={`gloss-item ${source.kind === 'book' && source.bookId === book.id ? 'on' : ''}`}
                  onClick={() => pickSource({ kind: 'book', bookId: book.id, title: book.title })}
                >
                  <BookOpen size={13} style={{ color: 'var(--ink-3)', flex: 'none' }} />
                  <span className="gloss-item-name truncate" title={book.title}>{book.title}</span>
                  <span className="gloss-count">{count}</span>
                </div>
              );
            })}
          </div>

          <div className="gloss-zone-tabs" style={{ marginTop: 8 }}>
            <span className="chip" style={{ cursor: 'default' }}>
              <NotebookText size={13} />
              {t.learning.notebooks}
            </span>
          </div>
          <div className="gloss-list">
            {notebooks.map((nb) => (
              <div
                key={nb.id}
                className={`gloss-item ${source.kind === 'notebook' && source.notebookId === nb.id ? 'on' : ''}`}
                onClick={() => pickSource({ kind: 'notebook', notebookId: nb.id, name: nb.name })}
              >
                <NotebookText size={13} style={{ color: 'var(--ink-3)', flex: 'none' }} />
                <span className="gloss-item-name truncate" title={nb.name}>{nb.name}</span>
                <span className="gloss-count">{nb.entries.length}</span>
              </div>
            ))}
            {notebooks.length === 0 && <p className="gloss-empty">{t.learning.notebooksEmpty}</p>}
          </div>
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              value={newNotebookName}
              placeholder={t.learning.notebookNamePlaceholder}
              onChange={(e) => setNewNotebookName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') void createNotebook(); }}
              style={{ flex: 1, minWidth: 0 }}
            />
            <button type="button" className="iconbtn" title={t.learning.createNotebook} onClick={() => void createNotebook()} disabled={!newNotebookName.trim()}>
              <Plus size={13} />
            </button>
          </div>
        </div>

        {/* ── 右区 ── */}
        <div className="gloss-right">
          <div className="gloss-right-head">
            <div className="gloss-right-title">
              {sourceTitle}
              <span className="ptitle-count">· {stats.total} {t.learning.words}</span>
            </div>
            <div className="gloss-right-actions">
              {activeNotebook && (
                <button type="button" className="iconbtn danger" title={t.learning.deleteNotebook} onClick={() => void deleteNotebook(activeNotebook.id)}>
                  <Trash2 size={13} />
                </button>
              )}
              <button type="button" className={`chip sm ${mode === 'list' ? 'on' : ''}`} onClick={() => setMode('list')}>
                <BarChart3 size={12} />
                {t.learning.listMode}
              </button>
              <button type="button" className={`chip sm ${mode === 'study' ? 'on' : ''}`} onClick={() => setMode('study')} disabled={rows.length === 0}>
                <Play size={12} />
                {t.learning.studyMode}
                {stats.due > 0 && <b>{stats.due}</b>}
              </button>
            </div>
          </div>

          {/* 统计条 */}
          <div className="stat-row2" style={{ gap: 10, margin: 0 }}>
            <div className="stat2"><div className="k">{t.learning.totalWords}</div><div className="v">{stats.total}</div></div>
            <div className="stat2"><div className="k">{t.learning.remaining}</div><div className="v">{stats.due}</div></div>
            <div className="stat2"><div className="k">{t.learning.inLearning}</div><div className="v">{stats.learning}</div></div>
            <div className="stat2"><div className="k">{t.learning.mastered}</div><div className="v">{stats.mastered}</div></div>
          </div>

          {/* 难度分布 + 生词本书籍分布 */}
          {mode === 'list' && (levelBuckets.length > 0 || bookDist.length > 1) && (
            <div className="learn-dist-row">
              <LevelBars buckets={levelBuckets} active={levelFilter} onPick={setLevelFilter} />
              {bookDist.length > 1 && (
                <div className="lvl-bars" role="img" aria-label="notebook book distribution">
                  {bookDist.slice(0, 5).map((item, i) => {
                    const pct = Math.round((item.count / stats.total) * 100);
                    const shades = ['var(--ink)', 'var(--ink-2)', 'var(--ink-3)', 'var(--ink-4)', 'var(--ink-5)'];
                    return (
                      <div key={item.title} className="lvl-bar-row" title={`${item.title} · ${item.count} (${pct}%)`}>
                        <span className="lvl-bar-k">{item.title}</span>
                        <span className="lvl-bar-track"><i style={{ width: `${Math.max(4, pct)}%`, background: shades[i] }} /></span>
                        <span className="lvl-bar-v">{item.count}</span>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {mode === 'list' && (
            <>
              {/* 工具行：搜索 + 批量操作 */}
              <div className="gloss-tools">
                <div className="nb-picker-search" style={{ flex: 1 }}>
                  <Search size={13} />
                  <input
                    value={query}
                    placeholder={t.notes.searchPlaceholder}
                    onChange={(e) => setQuery(e.target.value)}
                  />
                  {levelFilter && (
                    <button type="button" className="chip sm on" onClick={() => setLevelFilter(null)}>
                      {levelFilter}
                      <X size={11} />
                    </button>
                  )}
                </div>
                {checked.size > 0 && (
                  <>
                    <span className="gloss-count">{t.learning.selected} {checked.size}</span>
                    <button type="button" className="btn ghost sm" onClick={() => setPicking(true)}>
                      <NotebookText size={13} />
                      {t.learning.addToNotebook}
                    </button>
                    <button type="button" className="btn ghost sm danger-text" onClick={() => void removeChecked()}>
                      <Trash2 size={13} />
                      {t.notes.deleteNote}
                    </button>
                    <button type="button" className="iconbtn" title={t.learning.clearSelection} onClick={() => setChecked(new Set())}>
                      <X size={13} />
                    </button>
                  </>
                )}
              </div>

              {/* 词表 */}
              <div className="gloss-entries">
                {visibleRows.length === 0 && <p className="gloss-empty">{t.learning.emptyCards}</p>}
                {visibleRows.slice(0, 300).map((row) => (
                  <label key={rowKeyOf(row)} className="gloss-entry" style={{ cursor: 'pointer' }} onClick={() => openDetail(row)}>
                    <input
                      type="checkbox"
                      checked={checked.has(rowKeyOf(row))}
                      onClick={(e) => e.stopPropagation()}
                      onChange={() => toggleChecked(rowKeyOf(row))}
                    />
                    <span className="gloss-src" title={row.word}>{row.word}</span>
                    <span className="chip sm" style={{ fontSize: 10.5, flex: 'none' }}>
                      {wordInfo.get(row.word)?.level || t.learning.uncategorized}
                    </span>
                    <span style={{ fontSize: 12, color: 'var(--ink-3)', flex: 1 }} className="truncate">
                      {source.kind === 'all' ? bookTitle(row.bookId) || row.chapter : row.chapter}
                    </span>
                    {row.note && (
                      <span style={{ fontSize: 12, color: 'var(--ink-3)', flex: 'none' }} title={row.note}>
                        <PenLine size={12} strokeWidth={2} />
                      </span>
                    )}
                    <span className="chip sm" style={{ fontSize: 10.5, flex: 'none' }}>
                      {isDue(row.dueAt) ? t.learning.remaining : t.learning.scheduled}
                    </span>
                    {/* 行内四档评分（悬停浮现，不占常驻空间；点击不冒泡到行点击） */}
                    <span className="gloss-row-rate" onClick={(e) => e.stopPropagation()}>
                      {([0, 3, 4, 5] as const).map((q, i) => (
                        <button
                          key={q}
                          type="button"
                          data-tone={q === 0 ? 'bad' : q === 3 ? 'hard' : q === 4 ? 'good' : 'easy'}
                          title={[t.learning.forgot, t.learning.hard, t.learning.good, t.learning.easy][i]}
                          aria-label={[t.learning.forgot, t.learning.hard, t.learning.good, t.learning.easy][i]}
                          onClick={() => void rateRow(row, q)}
                        >
                          {i + 1}
                        </button>
                      ))}
                    </span>
                  </label>
                ))}
              </div>
            </>
          )}
        </div>
      </div>

      {/* 全屏背诵子界面（Anki 式独立层级，非右栏内嵌） */}
      {mode === 'study' && (
        <StudyOverlay
          title={activeNotebook ? activeNotebook.name : sourceTitle}
          onExit={() => setMode('list')}
        >
          {activeNotebook ? (
            <NotebookDeck key={activeNotebook.id} notebook={activeNotebook} levelInfo={wordInfo} />
          ) : (
            <GlobalVocabQueue
              key={source.kind === 'book' ? source.bookId : 'all'}
              books={books}
              initialBookFilter={source.kind === 'book' ? source.bookId : null}
              onInitialBookFilterConsumed={() => { /* 源切换由 key 重挂载承担 */ }}
            />
          )}
        </StudyOverlay>
      )}

      {/* 词详情浮层：点词看详情 + 四档评分 + 笔记编辑 */}
      {detailRow && (
        <div className="fixed inset-0 z-[140]" role="dialog" aria-label={detailRow.word}>
          <button type="button" aria-label="close" className="absolute inset-0 bg-black/25" onClick={() => setDetailRow(null)} />
          <div className="app-dialog-card" style={{ width: 'min(460px, 92vw)', gap: 12 }}>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
              <h3 className="app-dialog-title" style={{ fontSize: 20, fontFamily: 'var(--font-display)' }}>{detailRow.word}</h3>
              <span className="chip sm" style={{ fontSize: 10.5 }}>{wordInfo.get(detailRow.word)?.level || t.learning.uncategorized}</span>
              {wordInfo.get(detailRow.word)?.phonetic && (
                <span style={{ fontSize: 12, color: 'var(--ink-3)' }}>{wordInfo.get(detailRow.word)?.phonetic}</span>
              )}
            </div>
            {(detailRow.definition || wordInfo.get(detailRow.word)?.definition) && (
              <p className="app-dialog-message" style={{ whiteSpace: 'pre-line' }}>
                {detailRow.definition || wordInfo.get(detailRow.word)?.definition}
              </p>
            )}
            {detailRow.context && (
              <p style={{ fontSize: 12.5, fontStyle: 'italic', lineHeight: 1.7, color: 'var(--ink-3)' }}>
                {detailRow.context}
              </p>
            )}
            {/* 四档评分（按源路由） */}
            <div className="deck-actions" style={{ width: '100%' }}>
              {([
                [0, t.learning.forgot, 'forgot'],
                [3, t.learning.hard, 'hard'],
                [4, t.learning.good, 'known'],
                [5, t.learning.easy, 'easy'],
              ] as const).map(([quality, label, tone]) => (
                <button key={quality} type="button" data-tone={tone} disabled={detailBusy} onClick={() => void rateDetail(quality)}>
                  <span>{label}</span>
                </button>
              ))}
            </div>
            {/* 笔记编辑 */}
            <textarea
              className="app-dialog-input"
              style={{ resize: 'vertical', minHeight: 64, lineHeight: 1.6 }}
              placeholder={t.learning.notePlaceholder}
              value={detailNote}
              onChange={(e) => setDetailNote(e.target.value)}
            />
            <div className="app-dialog-actions">
              <button type="button" className="btn ghost sm" onClick={() => setDetailRow(null)}>'关闭'</button>
              <button type="button" className="btn sm" disabled={detailBusy || (detailRow.note ?? '') === detailNote} onClick={() => void saveDetailNote()}>
                {t.learning.saveNote ?? '保存笔记'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 批量收词弹窗 */}
      {picking && checkedEntries.length > 0 && (
        <NotebookPicker
          entries={checkedEntries.map((row) => ({
            word: row.word,
            definition: row.definition || wordInfo.get(row.word)?.definition || '',
            context: row.context,
            bookId: row.bookId,
            chapter: row.chapter,
          }))}
          onDone={() => { setPicking(false); setChecked(new Set()); reloadNotebooks(); }}
          onCancel={() => setPicking(false)}
        />
      )}
    </div>
  );
}
