import { invoke } from '@tauri-apps/api/core';
import { motion } from 'motion/react';
import { ArrowRightLeft, ArrowDownAZ, ArrowUpZA, BookOpen, Download, FolderPlus, Lock, NotebookText, Search, Trash2, Upload } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useT } from '@/i18n';
import { useAppDialog } from '@/components/common/AppDialog';
import type { Book, GlossaryDeckView, GlossaryEntry, GlossaryOverrideInput, UserGlossaryEntry } from '@/types';

/** 术语库 v3：
    - 6.1/6.6 单一可编辑列表直编：不重复展示，取消独立编辑界面；
    - 6.4 单词删除；6.5 通用词汇可编辑；
    - 6.7 批量转移到任意词库（弹窗带搜索）；
    - 6.3 图标按钮替代文字。 */
export function GlossaryView({ books }: { books: Book[] }) {
  const t = useT();
  const { appConfirm } = useAppDialog();
  const [decks, setDecks] = useState<GlossaryDeckView[]>([]);
  const [decksLoaded, setDecksLoaded] = useState(false);
  const [zone, setZone] = useState<'general' | 'book'>('general');
  const [selectedDeckId, setSelectedDeckId] = useState<string>('');
  const [bookId, setBookId] = useState<string>('');
  const [bookEntries, setBookEntries] = useState<GlossaryEntry[]>([]);
  const [bookLoaded, setBookLoaded] = useState(false);
  const [query, setQuery] = useState('');
  const [selectedWords, setSelectedWords] = useState<string[]>([]);
  const [transferOpen, setTransferOpen] = useState(false);
  /** 列排序：原文/译法 + 升降序。 */
  const [sortKey, setSortKey] = useState<'source' | 'target'>('source');
  const [sortAsc, setSortAsc] = useState(true);
  const importFileRef = useRef<HTMLInputElement | null>(null);

  const activeBook = books.find((b) => b.id === bookId) ?? books[0];
  const selectedDeck = decks.find((d) => d.id === selectedDeckId) ?? decks[0] ?? null;

  const reloadDecks = useCallback(() => {
    invoke<GlossaryDeckView[]>('list_glossary_decks')
      .then(setDecks)
      .catch(() => setDecks([]))
      .finally(() => setDecksLoaded(true));
  }, []);

  useEffect(() => { reloadDecks(); }, [reloadDecks]);

  useEffect(() => {
    if (!activeBook) return;
    let cancelled = false;
    setBookLoaded(false);
    invoke<GlossaryEntry[]>('list_glossary_entries', { taskId: activeBook.id })
      .then((rows) => { if (!cancelled) setBookEntries(rows); })
      .catch(() => { if (!cancelled) setBookEntries([]); })
      .finally(() => { if (!cancelled) setBookLoaded(true); });
    return () => { cancelled = true; };
  }, [activeBook?.id]);

  useEffect(() => { setSelectedWords([]); setQuery(''); }, [zone, selectedDeck?.id, activeBook?.id]);

  /* 排序应用：先过滤后排序。 */
  const applySort = useCallback(<T extends { source: string }>(rows: T[], valueOf: (row: T) => string): T[] => {
    const dir = sortAsc ? 1 : -1;
    return [...rows].sort((a, b) => dir * valueOf(a).localeCompare(valueOf(b), undefined, { sensitivity: 'base' }));
  }, [sortAsc]);

  const filteredGeneral = useMemo(() => {
    const q = query.trim().toLowerCase();
    const entries = selectedDeck?.entries ?? [];
    const rows = q ? entries.filter((e) => e.source.toLowerCase().includes(q) || e.target.toLowerCase().includes(q)) : entries;
    return applySort(rows, sortKey === 'source' ? (e) => e.source : (e) => e.target);
  }, [selectedDeck?.entries, query, applySort, sortKey]);

  const filteredBook = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = q ? bookEntries.filter((e) => e.source.toLowerCase().includes(q) || e.translation.toLowerCase().includes(q)) : bookEntries;
    return applySort(rows, sortKey === 'source' ? (e) => e.source : (e) => e.translation);
  }, [bookEntries, query, applySort, sortKey]);

  const currentCount = zone === 'general' ? filteredGeneral.length : filteredBook.length;
  const currentKeys = zone === 'general' ? filteredGeneral.map((e) => e.source) : filteredBook.map((e) => e.source);
  const allSelected = currentKeys.length > 0 && currentKeys.every((k) => selectedWords.includes(k));

  const toggleAll = () => setSelectedWords(allSelected ? [] : currentKeys);
  const toggleWord = (source: string) => {
    setSelectedWords((words) => words.includes(source) ? words.filter((w) => w !== source) : [...words, source]);
  };

  /* 通用词表直编：整表替换保存（去重归一由后端负责） */
  const saveGeneralEntries = async (entries: UserGlossaryEntry[]) => {
    if (!selectedDeck) return;
    try {
      const next = await invoke<GlossaryDeckView[]>('save_glossary_deck_entries', {
        deckId: selectedDeck.id,
        entries,
      });
      setDecks(next);
    } catch (error) {
      console.error('save deck entries failed:', error);
    }
  };

  const saveTimeoutRef = useRef<number | null>(null);
  const patchGeneralEntry = (source: string, patch: Partial<UserGlossaryEntry>) => {
    if (!selectedDeck) return;
    const next = selectedDeck.entries.map((e) => (e.source === source ? { ...e, ...patch } : e));
    setDecks((all) => all.map((d) => (d.id === selectedDeck.id ? { ...d, entries: next, entryCount: next.length } : d)));
    if (saveTimeoutRef.current) window.clearTimeout(saveTimeoutRef.current);
    saveTimeoutRef.current = window.setTimeout(() => {
      void saveGeneralEntries(next);
    }, 450);
  };

  /* 单词删除：整表替换去掉该行（取消 debounce 防旧表复活） */
  const removeGeneralEntry = (source: string) => {
    if (!selectedDeck) return;
    if (saveTimeoutRef.current) window.clearTimeout(saveTimeoutRef.current);
    const next = selectedDeck.entries.filter((e) => e.source !== source);
    setDecks((all) => all.map((d) => (d.id === selectedDeck.id ? { ...d, entries: next, entryCount: next.length } : d)));
    void saveGeneralEntries(next);
    setSelectedWords((words) => words.filter((w) => w !== source));
  };

  /* 专属词表修订保存（旁路 overrides；renameFrom 承载英文原文改名） */
  const saveBookOverrides = async (overrides: GlossaryOverrideInput[]) => {
    if (!activeBook) return;
    try {
      await invoke('save_artifact_glossary_overrides', { taskId: activeBook.id, overrides });
      setBookEntries((rows) => rows.map((row) => {
        const rename = overrides.find((o) => o.renameFrom && o.renameFrom === row.source);
        if (rename) return { ...row, source: rename.source, translation: rename.translation };
        const hit = overrides.find((o) => o.source === row.source && !o.renameFrom);
        return hit ? { ...row, translation: hit.translation } : row;
      }));
    } catch (error) {
      console.error('save overrides failed:', error);
    }
  };

  const exportTsv = () => {
    const rows = zone === 'general'
      ? filteredGeneral.map((e) => `${e.source}\t${e.target}`)
      : filteredBook.map((e) => `${e.source}\t${e.translation}`);
    const blob = new Blob([rows.join('\n')], { type: 'text/tab-separated-values' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${zone === 'general' ? selectedDeck?.name ?? 'glossary' : activeBook?.title ?? 'glossary'}-terms.tsv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  /* 导入：TSV/CSV/JSON，按原文去重合并
     （与后端归一语义一致：首见译法优先）。仅通用区开放——专属区为管线产物。 */
  const parseImportText = (text: string): { source: string; target: string }[] => {
    const trimmed = text.trim();
    if (!trimmed) return [];
    if (trimmed.startsWith('[') || trimmed.startsWith('{')) {
      try {
        const parsed = JSON.parse(trimmed);
        const arr = Array.isArray(parsed) ? parsed : [parsed];
        return arr
          .map((row: Record<string, unknown>) => ({
            source: String(row.source ?? row.src ?? row.word ?? '').trim(),
            target: String(row.target ?? row.translation ?? row.zh ?? '').trim(),
          }))
          .filter((row) => row.source && row.target);
      } catch {
        return [];
      }
    }
    const sep = trimmed.includes('\t') ? '\t' : ',';
    // CSV 引号感知解析：`"Smith, John",史密斯` 的引号内逗号不拆分（RFC4180 简化版）。
    const parseCsvLine = (line: string): string[] => {
      const fields: string[] = [];
      let field = '';
      let inQuotes = false;
      for (let i = 0; i < line.length; i++) {
        const ch = line[i];
        if (inQuotes) {
          if (ch === '"' && line[i + 1] === '"') {
            field += '"';
            i += 1;
          } else if (ch === '"') {
            inQuotes = false;
          } else {
            field += ch;
          }
        } else if (ch === '"') {
          inQuotes = true;
        } else if (ch === sep) {
          fields.push(field);
          field = '';
        } else {
          field += ch;
        }
      }
      fields.push(field);
      return fields.map((f) => f.trim());
    };
    return trimmed
      .split(/\r?\n/)
      .map((line) => {
        const [source = '', target = ''] = parseCsvLine(line);
        return { source, target };
      })
      .filter((row) => row.source && row.target);
  };

  const importGlossaryFile = async (file: File) => {
    if (!selectedDeck) return;
    try {
      const text = await file.text();
      const imported = parseImportText(text);
      if (imported.length === 0) return;
      // 竞态防护：取消未决的 debounce 保存（防旧闭包表覆写刚导入的新表）
      if (saveTimeoutRef.current) window.clearTimeout(saveTimeoutRef.current);
      const existing = selectedDeck.entries;
      const seen = new Set(existing.map((e) => e.source.toLowerCase()));
      const additions: UserGlossaryEntry[] = imported
        .filter((row) => !seen.has(row.source.toLowerCase()))
        .map((row) => ({ source: row.source, target: row.target, enforcement: 'strict' }));
      if (additions.length === 0) return;
      await saveGeneralEntries([...existing, ...additions]);
    } catch (error) {
      console.error('import glossary file failed:', error);
    }
  };

  const toggleSort = (key: 'source' | 'target') => {
    if (sortKey === key) setSortAsc((v) => !v);
    else {
      setSortKey(key);
      setSortAsc(true);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }}
      className="page-stage"
    >
      <header className="page-head">
        <h1 className="page-title">{t.terms.title}</h1>
        <div className="actions">
          <div className="searchbox sm">
            <Search size={13} />
            <input value={query} placeholder={t.terms.searchTerms} onChange={(e) => setQuery(e.target.value)} />
          </div>
          <button type="button" className="btn ghost sm" onClick={exportTsv} disabled={currentCount === 0} title="导出 TSV">
            <Download size={14} />
          </button>
          {zone === 'general' && selectedDeck && (
            <>
              <button
                type="button"
                className="btn ghost sm"
                onClick={() => importFileRef.current?.click()}
                title="导入 TSV/CSV/JSON（按原文去重合并）"
              >
                <Upload size={14} />
              </button>
              <input
                ref={importFileRef}
                type="file"
                accept=".tsv,.csv,.json,.txt"
                style={{ display: 'none' }}
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) void importGlossaryFile(file);
                  e.target.value = '';
                }}
              />
            </>
          )}
        </div>
      </header>

      <div className="glossary-v2">
        {/* 左侧：分区 + 词表/书目列表 */}
        <div className="gloss-left">
          <div className="gloss-zone-tabs">
            <button type="button" className={`chip ${zone === 'general' ? 'on' : ''}`} onClick={() => setZone('general')}>
              <NotebookText size={13} />
              {t.terms.generalZone} · {decks.length}
            </button>
            <button type="button" className={`chip ${zone === 'book' ? 'on' : ''}`} onClick={() => setZone('book')}>
              <BookOpen size={13} />
              {t.terms.bookZone} · {books.length}
            </button>
          </div>

          <div className="gloss-list">
            {zone === 'general' ? (
              !decksLoaded ? <p className="gloss-empty">…</p> : decks.length === 0 ? (
                <p className="gloss-empty">{t.terms.emptyUser}</p>
              ) : decks.map((deck) => (
                <div
                  key={deck.id}
                  className={`gloss-item ${selectedDeck?.id === deck.id ? 'on' : ''}`}
                  onClick={() => setSelectedDeckId(deck.id)}
                >
                  <label className="toggle" title={t.terms.deckToggle}>
                    <input
                      type="checkbox"
                      checked={deck.enabled}
                      onClick={(e) => e.stopPropagation()}
                      onChange={(e) => void toggleDeck(deck.id, e.target.checked, reloadDecks)}
                    />
                    <span className="toggle-track" />
                  </label>
                  <span className="gloss-item-name truncate">{deck.name}</span>
                  <span className="gloss-count">{deck.entryCount}</span>
                  <button
                    type="button"
                    className="gloss-item-del"
                    title={t.terms.remove}
                    onClick={(e) => {
                      e.stopPropagation();
                      void (async () => {
                        if (!(await appConfirm({ title: t.terms.deleteDeckConfirm, message: deck.name, danger: true }))) return;
                        await deleteDeck(deck.id, reloadDecks);
                      })();
                    }}
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              ))
            ) : books.length === 0 ? (
              <p className="gloss-empty">{t.terms.empty}</p>
            ) : books.map((book) => (
              <div
                key={book.id}
                className={`gloss-item ${activeBook?.id === book.id ? 'on' : ''}`}
                onClick={() => setBookId(book.id)}
              >
                <Lock size={13} style={{ color: 'var(--ink-3)', flex: 'none' }} />
                <span className="gloss-item-name truncate">{book.title}</span>
              </div>
            ))}
          </div>

          {zone === 'general' && <NewDeckRow onCreated={reloadDecks} />}
        </div>

        {/* 右侧：可编辑词条列表（单一视图，直编直删） */}
        <div className="gloss-right">
          <div className="gloss-right-head">
            <div className="gloss-right-title">
              {zone === 'general' ? selectedDeck?.name ?? '—' : activeBook?.title ?? '—'}
              <span className="ptitle-count">· {currentCount}</span>
            </div>
            <div className="gloss-right-actions">
              {/* 列排序：点按同列切换升降序，切列重置升序 */}
              <button
                type="button"
                className={`chip sm ${sortKey === 'source' ? 'on' : ''}`}
                onClick={() => toggleSort('source')}
                title="按原文排序"
              >
                {sortKey === 'source' && sortAsc ? <ArrowUpZA size={12} /> : <ArrowDownAZ size={12} />}
                {t.terms.sourceLabel ?? '原文'}
              </button>
              <button
                type="button"
                className={`chip sm ${sortKey === 'target' ? 'on' : ''}`}
                onClick={() => toggleSort('target')}
                title="按译法排序"
              >
                {sortKey === 'target' && sortAsc ? <ArrowUpZA size={12} /> : <ArrowDownAZ size={12} />}
                {t.terms.targetLabel ?? '译法'}
              </button>
              <label className="gloss-checkall">
                <input type="checkbox" checked={allSelected} onChange={toggleAll} />
                {t.terms.selectAll}
              </label>
              {selectedWords.length > 0 && (
                <button type="button" className="btn sm" onClick={() => setTransferOpen(true)}>
                  <ArrowRightLeft size={13} />
                  {selectedWords.length}
                </button>
              )}
            </div>
          </div>

          {((zone === 'general' && !decksLoaded) || (zone === 'book' && !bookLoaded)) && (
            <p className="gloss-empty">…</p>
          )}
          {currentCount === 0 && (
            <p className="gloss-empty">{zone === 'general' ? t.terms.emptyUser : t.terms.empty}</p>
          )}

          {currentCount > 0 && zone === 'general' && selectedDeck && (
            <div className="gloss-entries">
              {filteredGeneral.map((entry) => (
                <div key={entry.source} className="gloss-entry">
                  <input
                    type="checkbox"
                    checked={selectedWords.includes(entry.source)}
                    onChange={() => toggleWord(entry.source)}
                  />
                  {/* 英文原文可编辑：改 source 即整行替换。 */}
                  <input
                    className="gloss-edit-input gloss-edit-src"
                    value={entry.source}
                    onChange={(e) => {
                      const nextSource = e.target.value;
                      if (!nextSource) return;
                      setDecks((all) => all.map((d) => d.id === selectedDeck.id ? {
                        ...d,
                        entries: d.entries.map((row) => row.source === entry.source ? { ...row, source: nextSource } : row),
                      } : d));
                      if (saveTimeoutRef.current) window.clearTimeout(saveTimeoutRef.current);
                      saveTimeoutRef.current = window.setTimeout(() => {
                        const updated = selectedDeck.entries.map((row) => row.source === entry.source ? { ...row, source: nextSource } : row);
                        void saveGeneralEntries(updated);
                      }, 450);
                    }}
                    aria-label="source"
                  />
                  <input
                    className="gloss-edit-input"
                    value={entry.target}
                    onChange={(e) => patchGeneralEntry(entry.source, { target: e.target.value })}
                    aria-label={t.terms.targetLabel}
                  />
                  <select
                    className="gloss-edit-select"
                    value={entry.enforcement || 'strict'}
                    onChange={(e) => patchGeneralEntry(entry.source, { enforcement: e.target.value })}
                    aria-label={t.terms.enforceStrict}
                  >
                    <option value="strict">{t.terms.enforceStrict}</option>
                    <option value="contextual">{t.terms.enforceContextual}</option>
                  </select>
                  <button
                    type="button"
                    className="gloss-row-del"
                    title={t.terms.remove}
                    onClick={() => removeGeneralEntry(entry.source)}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              ))}
            </div>
          )}

          {currentCount > 0 && zone === 'book' && activeBook && (
            <div className="gloss-entries">
              {filteredBook.map((entry) => (
                <div key={entry.source} className="gloss-entry">
                  <input
                    type="checkbox"
                    checked={selectedWords.includes(entry.source)}
                    onChange={() => toggleWord(entry.source)}
                  />
                  {/* 英文原文可编辑：改名经 renameFrom 旁路提交，不动 artifact 本体。 */}
                  <input
                    className="gloss-edit-input gloss-edit-src"
                    defaultValue={entry.source}
                    key={`src-${entry.source}`}
                    onBlur={(e) => {
                      const next = e.target.value.trim();
                      if (!next || next === entry.source) return;
                      void saveBookOverrides([{ source: next, translation: entry.translation, renameFrom: entry.source }]);
                    }}
                    aria-label={t.terms.sourceLabel ?? 'source'}
                  />
                  <input
                    className="gloss-edit-input"
                    value={entry.translation}
                    onChange={(e) => {
                      const next = e.target.value;
                      setBookEntries((rows) => rows.map((row) => row.source === entry.source ? { ...row, translation: next } : row));
                    }}
                    onBlur={(e) => void saveBookOverrides([{ source: entry.source, translation: e.target.value }])}
                    aria-label={t.terms.targetLabel}
                  />
                  <span className="chip sm" style={{ fontSize: 10.5, flex: 'none' }}>
                    {entry.origin === 'user' ? t.terms.originUser : entry.origin === 'static' ? t.terms.originStatic : t.terms.originDynamic}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 批量转移弹窗（选目标词库，带搜索） */}
      {transferOpen && (
        <GlossaryTransferModal
          zone={zone}
          decks={decks}
          books={books}
          selectedDeckId={selectedDeck?.id ?? ''}
          activeBookId={activeBook?.id ?? ''}
          words={selectedWords}
          getEntries={(source) =>
            zone === 'general'
              ? selectedDeck?.entries.find((e) => e.source === source)?.target ?? ''
              : bookEntries.find((e) => e.source === source)?.translation ?? ''
          }
          onDone={() => { setTransferOpen(false); setSelectedWords([]); reloadDecks(); }}
          onCancel={() => setTransferOpen(false)}
        />
      )}
    </motion.div>
  );
}

/* ── 组件：新建词表行（图标按钮） ── */
function NewDeckRow({ onCreated }: { onCreated: () => void }) {
  const t = useT();
  const [name, setName] = useState('');
  const create = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    try {
      await invoke('create_glossary_deck', { name: trimmed });
      setName('');
      onCreated();
    } catch (error) {
      console.error('create deck failed:', error);
    }
  };
  return (
    <div className="gloss-newdeck">
      <input
        value={name}
        placeholder={t.terms.deckNamePlaceholder}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') void create(); }}
      />
      <button type="button" className="iconbtn" onClick={() => void create()} disabled={!name.trim()} title={t.terms.addDeck}>
        <FolderPlus size={15} />
      </button>
    </div>
  );
}

/* ── 组件：批量转移弹窗（目标列表带搜索） ── */
function GlossaryTransferModal({
  zone,
  decks,
  books,
  selectedDeckId,
  activeBookId,
  words,
  getEntries,
  onDone,
  onCancel,
}: {
  zone: 'general' | 'book';
  decks: GlossaryDeckView[];
  books: Book[];
  selectedDeckId: string;
  activeBookId: string;
  words: string[];
  getEntries: (source: string) => string;
  onDone: () => void;
  onCancel: () => void;
}) {
  const t = useT();
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);

  /* 可选目标：通用区→其他词表；专属区→全部词表 + 当前书修订层 */
  const targets = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list: { kind: 'deck' | 'book'; id: string; label: string }[] = [];
    if (zone === 'general') {
      for (const deck of decks) {
        if (deck.id !== selectedDeckId) list.push({ kind: 'deck', id: deck.id, label: `${t.terms.generalZone} · ${deck.name}` });
      }
    } else {
      for (const deck of decks) list.push({ kind: 'deck', id: deck.id, label: `${t.terms.generalZone} · ${deck.name}` });
      list.push({ kind: 'book', id: activeBookId, label: `${t.terms.bookZone} · ${books.find((b) => b.id === activeBookId)?.title ?? ''}` });
    }
    return q ? list.filter((item) => item.label.toLowerCase().includes(q)) : list;
  }, [zone, decks, selectedDeckId, activeBookId, books, query, t]);

  const transferTo = async (target: { kind: 'deck' | 'book'; id: string }) => {
    if (busy) return;
    setBusy(true);
    try {
      const additions = words.map((word) => ({ source: word, translation: getEntries(word) })).filter((a) => a.translation.trim());
      if (target.kind === 'deck') {
        const deck = decks.find((d) => d.id === target.id);
        if (!deck) return;
        const merged: UserGlossaryEntry[] = [
          ...deck.entries,
          ...additions.map((a) => ({ source: a.source, target: a.translation, enforcement: 'strict' })),
        ];
        await invoke('save_glossary_deck_entries', { deckId: target.id, entries: merged });
        /* 通用区之间转移：从来源词表移除 */
        if (zone === 'general' && selectedDeckId) {
          const src = decks.find((d) => d.id === selectedDeckId);
          if (src) {
            const remaining = src.entries.filter((e) => !words.includes(e.source));
            await invoke('save_glossary_deck_entries', { deckId: selectedDeckId, entries: remaining });
          }
        }
      } else if (target.kind === 'book') {
        /* 转入专属词表：作为 user 新增写入修订层 */
        await invoke('save_artifact_glossary_overrides', { taskId: target.id, overrides: additions });
      }
      onDone();
    } catch (error) {
      console.error('transfer failed:', error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[125]" role="dialog" aria-label={t.terms.transferToGeneral}>
      <button type="button" className="absolute inset-0 bg-black/20" onClick={onCancel} aria-label={t.notes.cancelEdit} />
      <div className="nb-picker">
        <div className="nb-picker-head">
          <ArrowRightLeft size={16} />
          <span className="font-semibold text-sm">{t.terms.transferToGeneral}</span>
          <span className="nb-picker-word truncate">{words.length} {t.learning.words}</span>
        </div>
        <div className="nb-picker-search">
          <Search size={13} />
          <input
            value={query}
            placeholder={t.notes.searchPlaceholder}
            onChange={(e) => setQuery(e.target.value)}
            autoFocus
          />
        </div>
        <div className="nb-picker-list">
          {targets.length === 0 && <p className="card-desc" style={{ padding: '8px 4px' }}>{t.terms.emptyUser}</p>}
          {targets.map((target) => (
            <button
              key={`${target.kind}-${target.id}`}
              type="button"
              className="nb-picker-item"
              onClick={() => void transferTo(target)}
              disabled={busy}
            >
              {target.kind === 'deck' ? <NotebookText size={14} /> : <BookOpen size={14} />}
              <span className="truncate">{target.label}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

async function toggleDeck(deckId: string, enabled: boolean, reload: () => void) {
  try {
    await invoke('toggle_glossary_deck', { deckId, enabled });
    reload();
  } catch (error) {
    console.error('toggle deck failed:', error);
  }
}

async function deleteDeck(deckId: string, reload: () => void) {
  try {
    await invoke('delete_glossary_deck', { deckId });
    reload();
  } catch (error) {
    console.error('delete deck failed:', error);
  }
}
