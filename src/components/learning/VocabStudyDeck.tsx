import type { VocabCardFace } from '@/types';
import { previewNextIntervalDays, type ReviewQuality } from './useVocabDeck';

/** 间隔预览文案：≥1 天按天（保留一位小数），否则小时/分钟。 */
export function formatInterval(days: number, labels: { day: string; hour: string; min: string }): string {
  if (days >= 1) return `${Number.isInteger(days) ? days : days.toFixed(1)}${labels.day}`;
  const hours = days * 24;
  if (hours >= 1) return `${Math.round(hours)}${labels.hour}`;
  return `${Math.max(1, Math.round(days * 24 * 60))}${labels.min}`;
}

/** 四档自评按钮（忘记/困难/记住/简单）+ 下次间隔预览（Anki 式）。 */
export function DeckRatingButtons(props: {
  card: VocabCardFace;
  onReview: (quality: ReviewQuality) => void;
  labels: { forgot: string; hard: string; good: string; easy: string; day: string; hour: string; min: string };
}) {
  const { card, onReview, labels } = props;
  const tiers: { quality: ReviewQuality; tone: string; label: string; key: string }[] = [
    { quality: 0, tone: 'forgot', label: labels.forgot, key: '1' },
    { quality: 3, tone: 'hard', label: labels.hard, key: '2' },
    { quality: 4, tone: 'known', label: labels.good, key: '3' },
    { quality: 5, tone: 'easy', label: labels.easy, key: '4' },
  ];
  return (
    <div className="deck-actions">
      {tiers.map((tier) => (
        <button key={tier.quality} type="button" data-tone={tier.tone} onClick={() => onReview(tier.quality)}>
          <span><i className="deck-kbd">{tier.key}</i>{tier.label}</span>
          <em>{formatInterval(previewNextIntervalDays(card, tier.quality), labels)}</em>
        </button>
      ))}
    </div>
  );
}
