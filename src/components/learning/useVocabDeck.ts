import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { GlobalDueCard, ReaderState, VocabCardFace } from '@/types';

/** 四档自评（Anki 式）：again=0 / hard=3 / good=4 / easy=5，映射 SM-2 quality。 */
export type ReviewQuality = 0 | 3 | 4 | 5;

interface UseVocabDeckOptions {
  bookId: string;
  /** 分章背诵过滤；undefined = 全书 */
  chapter?: string;
}

// 词汇透析背诵卡组：到期卡片拉取 + SM-2 评分写回 + Anki 导出。
// 卡片牌面由后端 list_due_vocab_cards 用全量词表填充（存储层只存调度状态）。
export function useVocabDeck({ bookId, chapter }: UseVocabDeckOptions) {
  const [dueCards, setDueCards] = useState<VocabCardFace[]>([]);
  const [cursor, setCursor] = useState(0);
  const [revealed, setRevealed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [reviewedCount, setReviewedCount] = useState(0);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const cards = await invoke<VocabCardFace[]>('list_due_vocab_cards', {
        bookId,
        chapter: chapter ?? null,
        includeFuture: false,
      });
      setDueCards(cards);
      setCursor(0);
      setRevealed(false);
      setReviewedCount(0);
    } catch (error) {
      console.error('Failed to load vocab cards:', error);
    } finally {
      setLoading(false);
    }
  }, [bookId, chapter]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const currentCard = dueCards[cursor] ?? null;
  const remaining = dueCards.length - cursor;

  const review = useCallback(
    async (quality: ReviewQuality) => {
      const card = dueCards[cursor];
      if (!card) return;
      try {
        await invoke<ReaderState>('review_vocab_card', {
          bookId,
          word: card.word,
          quality,
        });
      } catch (error) {
        console.error('Failed to review vocab card:', error);
        return;
      }
      setReviewedCount((count) => count + 1);
      // 忘记(0)：卡片稍后重现（10 分钟后到期）——本轮排到队尾
      if (quality < 3) {
        setDueCards((cards) => {
          const next = [...cards];
          const [done] = next.splice(cursor, 1);
          next.push(done);
          return next;
        });
        setRevealed(false);
        return;
      }
      setRevealed(false);
      setCursor((value) => value + 1);
    },
    [bookId, cursor, dueCards],
  );

  const exportAnki = useCallback(
    async (outputPath: string): Promise<boolean> => {
      try {
        await invoke('export_anki_cards', {
          bookId,
          chapter: chapter ?? null,
          outputPath,
        });
        return true;
      } catch (error) {
        console.error('Failed to export Anki cards:', error);
        return false;
      }
    },
    [bookId, chapter],
  );

  const summary = useMemo(
    () => ({ total: dueCards.length, remaining, reviewed: reviewedCount }),
    [dueCards.length, remaining, reviewedCount],
  );

  return {
    currentCard,
    revealed,
    setRevealed,
    loading,
    summary,
    review,
    refresh,
    exportAnki,
  };
}

/** SM-2 下次间隔预览（与后端 sm2_review 同构重放，仅计算不落库）：
    四档按钮下方展示「下次 ~」。返回天数；<1 天时由 UI 换算小时/分钟。 */
export function previewNextIntervalDays(card: VocabCardFace, quality: ReviewQuality): number {
  let repetitions = card.repetitions;
  let intervalDays = card.intervalDays;
  const easeFactor = card.easeFactor || 2.5;
  if (quality < 3) {
    repetitions = 0;
    intervalDays = 0.007; // ~10 分钟后再见
  } else {
    repetitions += 1;
    intervalDays =
      repetitions === 1 ? 1.0 : repetitions === 2 ? 6.0 : Math.max(1.0, intervalDays * easeFactor);
  }
  return intervalDays;
}

/** 全局跨书到期队列（学习中心全局背诵台数据源）。
    bookFilter：按书过滤队列（''=全部书籍）。 */
export function useGlobalVocabQueue(bookFilter: string = '') {
  const [allCards, setAllCards] = useState<GlobalDueCard[]>([]);
  const [cursor, setCursor] = useState(0);
  const [revealed, setRevealed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [reviewedCount, setReviewedCount] = useState(0);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const all = await invoke<GlobalDueCard[]>('list_due_vocab_cards_all');
      setAllCards(all);
      setCursor(0);
      setRevealed(false);
      setReviewedCount(0);
    } catch (error) {
      console.error('Failed to load global vocab queue:', error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  /* 按书过滤：空=全部书籍；过滤变化时重置游标与计数
     （防过滤后列表短于游标时误显「全部完成」）。 */
  const cards = useMemo(
    () => (bookFilter ? allCards.filter((c) => c.bookId === bookFilter) : allCards),
    [allCards, bookFilter],
  );
  useEffect(() => {
    setCursor(0);
    setReviewedCount(0);
  }, [bookFilter]);
  const currentCard = cards[cursor] ?? null;
  const remaining = cards.length - cursor;

  const review = useCallback(
    async (quality: ReviewQuality) => {
      const card = cards[cursor];
      if (!card) return;
      try {
        await invoke<ReaderState>('review_vocab_card', {
          bookId: card.bookId,
          word: card.word,
          quality,
        });
      } catch (error) {
        console.error('Failed to review vocab card:', error);
        return;
      }
      setReviewedCount((count) => count + 1);
      if (quality < 3) {
        // 忘记(0)：卡片移到全局卡片列表末尾，稍后再次重现复习
        setAllCards((prev) => {
          const next = [...prev];
          const targetIndex = next.findIndex((c) => c.bookId === card.bookId && c.word === card.word);
          if (targetIndex !== -1) {
            const [done] = next.splice(targetIndex, 1);
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
    [cards, cursor],
  );

  return {
    currentCard,
    revealed,
    setRevealed,
    loading,
    remaining,
    reviewedCount,
    review,
    refresh,
  };
}
