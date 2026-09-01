import { invoke } from '@tauri-apps/api/core';
import { GraduationCap, RotateCcw } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import { StudyCardBack } from './StudyCardBack';
import type { VocabCardFace, VocabNotebook } from '@/types';
import { DeckRatingButtons } from './VocabStudyDeck';
import type { ReviewQuality } from './useVocabDeck';

/** 生词本条目 → 背诵牌面（词表信息由调用方注入；调度字段取自 entry）。 */
function entryToFace(
  entry: VocabNotebook['entries'][number],
  info: { phonetic: string; definition: string; level: string } | undefined,
): VocabCardFace {
  return {
    word: entry.word,
    phonetic: info?.phonetic ?? '',
    definition: entry.definition || info?.definition || '',
    level: info?.level ?? '',
    context: entry.context,
    contextZh: '',
    chapter: entry.chapter,
    repetitions: entry.repetitions,
    intervalDays: entry.intervalDays,
    easeFactor: entry.easeFactor,
    dueAt: entry.dueAt,
  };
}

/** 生词本背诵台（Anki 式全屏）：SM-2 评分经 review_vocab_notebook_entry 写回，
    与书籍卡组同参数语义；空格翻面、1-4 评分。 */
export function NotebookDeck({ notebook, levelInfo }: {
  notebook: VocabNotebook;
  /** 词表增强（音标/难度/词表释义兜底），word → 信息。 */
  levelInfo: Map<string, { phonetic: string; definition: string; level: string }>;
}) {
  const t = useT();
  const now = Date.now();
  /* 到期优先，其余按创建时间；忘记(0)的卡稍后重现（移队尾）。 */
  const [queue, setQueue] = useState(() => {
    const due = notebook.entries.filter((e) => !e.dueAt || new Date(e.dueAt).getTime() <= now);
    const later = notebook.entries.filter((e) => e.dueAt && new Date(e.dueAt).getTime() > now);
    return [...due, ...later];
  });
  const [cursor, setCursor] = useState(0);
  const [revealed, setRevealed] = useState(false);
  const [reviewed, setReviewed] = useState(0);

  const card = queue[cursor] ?? null;
  const remaining = queue.length - cursor;
  const face = useMemo(
    () => (card ? entryToFace(card, levelInfo.get(card.word)) : null),
    [card, levelInfo],
  );
  const progress = queue.length > 0 ? Math.round((reviewed / queue.length) * 100) : 0;

  const review = useCallback(
    async (quality: ReviewQuality) => {
      if (!card) return;
      try {
        await invoke('review_vocab_notebook_entry', {
          notebookId: notebook.id,
          word: card.word,
          quality,
        });
      } catch (error) {
        console.error('Failed to review notebook entry:', error);
        return;
      }
      setReviewed((count) => count + 1);
      if (quality < 3) {
        setQueue((prev) => {
          const next = [...prev];
          const idx = next.findIndex((e) => e.word === card.word);
          if (idx !== -1) {
            const [done] = next.splice(idx, 1);
            next.push(done);
          }
          return next;
        });
        setRevealed(false);
        return;
      }
      setRevealed(false);
      setCursor((value) => value + 1);
    },
    [card, notebook.id],
  );

  const restart = useCallback(() => {
    setCursor(0);
    setRevealed(false);
    setReviewed(0);
  }, []);

  /* 键盘（Anki 对齐）：空格/Enter 翻面；1-4 评分。 */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, [contenteditable]')) return;
      if (event.code === 'Space' || event.key === 'Enter') {
        if (card) {
          event.preventDefault();
          setRevealed((v) => !v);
        }
        return;
      }
      if (!revealed) return;
      if (event.key === '1') { event.preventDefault(); void review(0); }
      if (event.key === '2') { event.preventDefault(); void review(3); }
      if (event.key === '3') { event.preventDefault(); void review(4); }
      if (event.key === '4') { event.preventDefault(); void review(5); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [card, revealed, review]);

  const done = !card && reviewed > 0;

  return (
    <div className="deck-full">
      <div className="deck-full-head">
        <span style={{ fontSize: 12.5, color: 'var(--ink-3)', fontWeight: 600 }}>{notebook.name}</span>
        <div className="study-progress" role="progressbar" aria-valuenow={progress} aria-valuemin={0} aria-valuemax={100}>
          <i style={{ width: `${progress}%` }} />
        </div>
      </div>

      <div className="deck-full-stage">
        {queue.length === 0 && (
          <div className="empty-hint">{t.learning.notebooksEmpty}</div>
        )}
        {queue.length > 0 && !card && !done && (
          <div className="empty-hint">{t.learning.emptyCards}</div>
        )}
        {done && (
          <div className="empty-hint">
            <GraduationCap size={30} strokeWidth={1.5} style={{ margin: '0 auto 12px', color: 'var(--signal)' }} />
            <b style={{ fontSize: 17 }}>{t.learning.allDone}</b>
            <p style={{ marginTop: 6 }}>{t.learning.reviewed} {reviewed}</p>
            <div style={{ marginTop: 16, display: 'flex', gap: 10, justifyContent: 'center' }}>
              <button type="button" className="btn sm" onClick={restart}>
                <RotateCcw size={13} />
                {t.learning.restart}
              </button>
            </div>
          </div>
        )}
        {face && (
          <div className="deck-card deck-card-full">
            <div className="deck-card-top">
              <span className="chip" style={{ fontSize: 11, padding: '3px 10px' }}>{face.level || t.learning.dueNew}</span>
              <span style={{ color: 'var(--ink-3)', fontSize: 11.5 }}>{face.chapter}</span>
            </div>
            <button type="button" className="deck-face" onClick={() => setRevealed(!revealed)}>
              <h3>{face.word}</h3>
              {face.phonetic && <p className="phon">{face.phonetic}</p>}
              {revealed ? (
                <StudyCardBack face={face} />
              ) : (
                <p style={{ marginTop: 24, fontSize: 12.5, color: 'var(--ink-3)' }}>{t.learning.flip}</p>
              )}
            </button>
          </div>
        )}
      </div>

      <div className="deck-full-foot">
        {face && !revealed && (
          <button type="button" className="deck-reveal-btn" onClick={() => setRevealed(true)}>
            {t.learning.flip}
            <em>Space</em>
          </button>
        )}
        {face && revealed && (
          <DeckRatingButtons
            card={face}
            onReview={(quality) => void review(quality)}
            labels={{
              forgot: t.learning.forgot,
              hard: t.learning.hard,
              good: t.learning.good,
              easy: t.learning.easy,
              day: t.learning.intervalDay,
              hour: t.learning.intervalHour,
              min: t.learning.intervalMin,
            }}
          />
        )}
        <div className="deck-counts">
          <span>{t.learning.remaining} {Math.max(0, remaining)}</span>
          <span>{t.learning.reviewed} {reviewed}</span>
        </div>
      </div>
    </div>
  );
}
