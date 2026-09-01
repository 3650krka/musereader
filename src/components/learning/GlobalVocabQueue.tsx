import { GraduationCap, RotateCcw } from 'lucide-react';
import { useEffect } from 'react';
import { useT } from '@/i18n';
import { StudyCardBack } from './StudyCardBack';
import type { Book } from '@/types';
import { useGlobalVocabQueue } from './useVocabDeck';
import { DeckRatingButtons } from './VocabStudyDeck';

interface GlobalVocabQueueProps {
  books: Book[];
  /** 打开时聚焦的书（学习页左侧选书 / 书库「词汇透析」跳入）。 */
  initialBookFilter?: string | null;
  onInitialBookFilterConsumed?: () => void;
}

/** 全局背诵台（Anki 式全屏）：到期队列 + 四档自评 + 间隔预览；
    空格翻面、1-4 评分。书籍范围在进入前的页面（学习页左栏/书库）选定，
    背诵台内不再提供切换（功能层级归属：选择不属于背诵动作）。 */
export function GlobalVocabQueue({ books, initialBookFilter, onInitialBookFilterConsumed }: GlobalVocabQueueProps) {
  const t = useT();
  const bookFilter = initialBookFilter ?? '';
  const queue = useGlobalVocabQueue(bookFilter);

  /* 外部聚焦一次性消费（防重复跳转）。 */
  useEffect(() => {
    if (initialBookFilter) onInitialBookFilterConsumed?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialBookFilter]);

  const bookTitleOf = (bookId: string): string =>
    books.find((book) => book.id === bookId)?.title ?? bookId;

  const card = queue.currentCard;
  const done = !queue.loading && !card && queue.reviewedCount > 0;

  const rate = (quality: 0 | 3 | 4 | 5) => {
    if (queue.revealed) void queue.review(quality);
  };

  /* 键盘（Anki 对齐）：空格/Enter 翻面；1-4 = 忘记/困难/记住/简单；输入框聚焦时让位。 */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, [contenteditable]')) return;
      if (event.code === 'Space' || event.key === 'Enter') {
        if (card) {
          event.preventDefault();
          queue.setRevealed(!queue.revealed);
        }
        return;
      }
      if (!queue.revealed) return;
      if (event.key === '1') { event.preventDefault(); rate(0); }
      if (event.key === '2') { event.preventDefault(); rate(3); }
      if (event.key === '3') { event.preventDefault(); rate(4); }
      if (event.key === '4') { event.preventDefault(); rate(5); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [card, queue.revealed]);

  return (
    <div className="deck-full">
      {/* 舞台：卡片纵向居中占满（flex:none + auto margin），内容超出时整卡滚动 */}
      <div className="deck-full-stage">
        {queue.loading && <p className="empty-hint">…</p>}

        {!queue.loading && !card && !done && (
          <div className="empty-hint">
            <GraduationCap size={26} strokeWidth={1.5} style={{ margin: '0 auto 10px', color: 'var(--ink-4)' }} />
            {t.learning.emptyCards}
          </div>
        )}

        {done && (
          <div className="empty-hint">
            <GraduationCap size={30} strokeWidth={1.5} style={{ margin: '0 auto 12px', color: 'var(--signal)' }} />
            <b style={{ fontSize: 17 }}>{t.learning.allDone}</b>
            <p style={{ marginTop: 6 }}>{t.learning.reviewed} {queue.reviewedCount}</p>
            <div style={{ marginTop: 16, display: 'flex', gap: 10, justifyContent: 'center' }}>
              <button type="button" className="btn sm" onClick={() => void queue.refresh()}>
                <RotateCcw size={13} />
                {t.learning.restart}
              </button>
            </div>
          </div>
        )}

        {card && (
          <div className="deck-card deck-card-full">
            <div className="deck-card-top">
              <span className="chip" style={{ fontSize: 11, padding: '3px 10px' }}>{card.level || t.learning.dueNew}</span>
              <span style={{ color: 'var(--ink-3)', fontSize: 11.5 }}>
                {bookTitleOf(card.bookId)}{card.chapter ? ` · ${card.chapter}` : ''}
              </span>
            </div>
            <button type="button" className="deck-face" onClick={() => queue.setRevealed(!queue.revealed)}>
              <h3>{card.word}</h3>
              {card.phonetic && <p className="phon">{card.phonetic}</p>}
              {queue.revealed ? (
                <StudyCardBack face={card} />
              ) : (
                <p style={{ marginTop: 24, fontSize: 12.5, color: 'var(--ink-3)' }}>{t.learning.flip}</p>
              )}
            </button>
          </div>
        )}
      </div>

      {/* 底部操作区（Anki 式）：未翻面=显示答案；翻面后=四档 */}
      <div className="deck-full-foot">
        {card && !queue.revealed && (
          <button type="button" className="deck-reveal-btn" onClick={() => queue.setRevealed(true)}>
            {t.learning.flip}
            <em>Space</em>
          </button>
        )}
        {card && queue.revealed && (
          <DeckRatingButtons
            card={card}
            onReview={(quality) => void queue.review(quality)}
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
          <span>{t.learning.remaining} {queue.remaining}</span>
          <span>{t.learning.reviewed} {queue.reviewedCount}</span>
        </div>
      </div>
    </div>
  );
}
