import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { Download, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useT } from '@/i18n';
import { loadUserWordLevel, saveUserWordLevel, type ReaderWordLevel } from '@/components/reader/useReaderPrefs';
import { useToast } from '@/components/common/Toast';
import { useAppDialog } from '@/components/common/AppDialog';

interface WordlistPackInfo {
  id: string;
  name: string;
  format: string;
  createdAt: string;
  wordCount: number;
}

interface WordlistStatus {
  activeWords: number;
  empty: boolean;
  packs: WordlistPackInfo[];
}

/* 与阅读器/WordWise/背诵共享同一词汇难度真源（reader prefs 的 userLevel），
   不再持有局部状态——离开再回不重置；与导入行未标档时的默认档同一语义。 */
const LEVELS: ReaderWordLevel[] = ['中考', '高考', '四级', '六级', '考研', '雅思', '托福', '专四', '专八', 'GRE'];

/** 词库面板：导入/管理用户词包（词库数据不随应用分发）。 */
export function WordlistPanel() {
  const t = useT();
  const toast = useToast();
  const { appConfirm } = useAppDialog();
  const [status, setStatus] = useState<WordlistStatus | null>(null);
  const [defaultLevel, setDefaultLevel] = useState<ReaderWordLevel>(() => loadUserWordLevel());
  const changeLevel = (next: ReaderWordLevel) => {
    setDefaultLevel(next);
    saveUserWordLevel(next);
  };
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<WordlistStatus>('wordlist_status'));
    } catch (error) {
      console.error('wordlist_status failed:', error);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleImport = useCallback(async () => {
    setBusy(true);
    try {
      const path = await openFileDialog({
        multiple: false,
        title: t.set.wordlistImport,
        filters: [
          { name: 'Word list', extensions: ['json', 'csv', 'tsv', 'txt'] },
          { name: 'All files', extensions: ['*'] },
        ],
      });
      if (typeof path !== 'string' || !path) return;
      const summary = await invoke<{ imported: number; skipped: number }>('import_wordlist', {
        path,
        defaultLevel,
        name: null,
      });
      toast.success(t.set.wordlistImported.replace('{n}', String(summary.imported)).replace('{s}', String(summary.skipped)));
      await refresh();
    } catch (error) {
      toast.error(`${t.set.wordlistImportFailed}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBusy(false);
    }
  }, [defaultLevel, refresh, t.set.wordlistImport, t.set.wordlistImported, t.set.wordlistImportFailed, toast]);

  const handleDelete = useCallback(async (pack: WordlistPackInfo) => {
    if (!(await appConfirm({ title: t.set.wordlistDeleteConfirm, danger: true }))) return;
    try {
      setStatus(await invoke<WordlistStatus>('delete_wordlist', { id: pack.id }));
    } catch (error) {
      toast.error(`${t.set.wordlistImportFailed}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }, [appConfirm, t.set.wordlistDeleteConfirm, t.set.wordlistImportFailed, toast]);

  return (
    <div className="card">
      <div className="desc" style={{ marginBottom: 14 }}>{t.set.wordlistDesc}</div>
      {status?.empty && (
        <div className="chip" style={{ display: 'inline-block', marginBottom: 14, padding: '6px 12px' }}>
          {t.set.wordlistEmpty}
        </div>
      )}
      {status && !status.empty && (
        <div className="chip on" style={{ display: 'inline-block', marginBottom: 14, padding: '6px 12px' }}>
          {t.set.wordlistActive.replace('{n}', String(status.activeWords))}
        </div>
      )}
      <div className="row2" style={{ alignItems: 'center', marginBottom: 12 }}>
        <label className="grow" style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ flex: 'none' }}>{t.set.wordlistDefaultLevel}</span>
          <select value={defaultLevel} onChange={(e) => changeLevel(e.target.value as ReaderWordLevel)}>
            {LEVELS.map((level) => <option key={level} value={level}>{level}</option>)}
          </select>
        </label>
        <button type="button" className="btn" onClick={() => void handleImport()} disabled={busy}>
          <Download size={14} />
          {busy ? t.set.wordlistImporting : t.set.wordlistImport}
        </button>
      </div>
      {status && status.packs.length > 0 && (
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
          <thead>
            <tr style={{ textAlign: 'left', color: 'var(--ink-3)' }}>
              <th style={{ padding: '6px 8px' }}>{t.set.pname}</th>
              <th style={{ padding: '6px 8px' }}>{t.set.wordlistColFormat}</th>
              <th style={{ padding: '6px 8px' }}>{t.set.wordlistColWords}</th>
              <th style={{ padding: '6px 8px' }}>{t.set.wordlistColCreated}</th>
              <th style={{ padding: '6px 8px' }} />
            </tr>
          </thead>
          <tbody>
            {status.packs.map((pack) => (
              <tr key={pack.id} style={{ borderTop: '1px solid var(--paper-3)' }}>
                <td style={{ padding: '6px 8px' }}>{pack.name}</td>
                <td style={{ padding: '6px 8px' }}>{pack.format}</td>
                <td style={{ padding: '6px 8px' }}>{pack.wordCount.toLocaleString()}</td>
                <td style={{ padding: '6px 8px' }}>{pack.createdAt.slice(0, 10)}</td>
                <td style={{ padding: '6px 8px', textAlign: 'right' }}>
                  <button
                    type="button"
                    className="iconbtn"
                    onClick={() => void handleDelete(pack)}
                    aria-label={t.set.wordlistDelete}
                    title={t.set.wordlistDelete}
                  >
                    <Trash2 size={14} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
