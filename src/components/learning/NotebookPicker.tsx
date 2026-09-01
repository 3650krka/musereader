import { invoke } from '@tauri-apps/api/core';
import { Check, NotebookText, Search } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import type { VocabNotebook } from '@/types';

/** 单条待收词条（词 + 携带的语境信息）。 */
export interface NotebookPickEntry {
  word: string;
  definition: string;
  context: string;
  bookId: string;
  chapter: string;
}

interface NotebookPickerProps {
  /** 单词模式（阅读器点词）：向后兼容字段。 */
  word?: string;
  definition?: string;
  context?: string;
  bookId?: string;
  chapter?: string;
  /** 批量模式（学习页多选收词）：优先于单词字段。 */
  entries?: NotebookPickEntry[];
  onDone: () => void;
  onCancel: () => void;
}

/** 生词本选择弹窗：把词/译文/例句加入任意自建生词本；
    列表支持搜索（问题 F.7：所有列表型弹窗均应支持搜索）。
    entries 提供时为批量收词（学习页多选），否则走单词模式（阅读器点词）。 */
export function NotebookPicker(props: NotebookPickerProps) {
  const { entries, onDone, onCancel } = props;
  const single: NotebookPickEntry | null = props.word
    ? { word: props.word, definition: props.definition ?? '', context: props.context ?? '', bookId: props.bookId ?? '', chapter: props.chapter ?? '' }
    : null;
  const batch = entries && entries.length > 0 ? entries : single ? [single] : [];
  const headline = batch.length === 1 ? batch[0].word : `${batch.length} 词`;
  const t = useT();
  const [notebooks, setNotebooks] = useState<VocabNotebook[]>([]);
  const [query, setQuery] = useState('');
  const [addedTo, setAddedTo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<VocabNotebook[]>('list_vocab_notebooks')
      .then(setNotebooks)
      .catch(() => setNotebooks([]));
  }, []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? notebooks.filter((nb) => nb.name.toLowerCase().includes(q)) : notebooks;
  }, [notebooks, query]);

  const addTo = async (notebookId: string) => {
    if (busy || batch.length === 0) return;
    setBusy(true);
    try {
      await invoke('add_vocab_notebook_entries', { notebookId, entries: batch });
      setAddedTo(notebookId);
      window.setTimeout(onDone, 450);
    } catch (error) {
      console.error('add to notebook failed:', error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[125]" role="dialog" aria-label={t.learning.pickNotebook}>
      <button type="button" className="absolute inset-0 bg-black/20" onClick={onCancel} aria-label={t.notes.cancelEdit} />
      <div className="nb-picker">
        <div className="nb-picker-head">
          <NotebookText size={16} />
          <span className="font-semibold text-sm">{t.learning.pickNotebook}</span>
          <span className="nb-picker-word truncate">{headline}</span>
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
          {filtered.length === 0 && (
            <p className="card-desc" style={{ padding: '8px 4px' }}>{t.learning.notebooksEmpty}</p>
          )}
          {filtered.map((nb) => (
            <button
              key={nb.id}
              type="button"
              className={`nb-picker-item ${addedTo === nb.id ? 'done' : ''}`}
              onClick={() => void addTo(nb.id)}
              disabled={busy}
            >
              {addedTo === nb.id ? <Check size={14} /> : <NotebookText size={14} />}
              <span className="truncate">{nb.name}</span>
              <span className="nb-picker-count">{nb.entries.length}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
