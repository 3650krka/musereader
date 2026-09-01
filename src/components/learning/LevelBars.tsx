import type { VocabBucket } from '@/components/insights/insightsShared';

/** 学习页难度分布横向条形图：分级词库真实档位（中考→GRE），
    lieflat 语法（可数刻度 + 明度阶），点击档位可筛选词表。 */
export function LevelBars({ buckets, active, onPick }: {
  buckets: VocabBucket[];
  active: string | null;
  onPick: (label: string | null) => void;
}) {
  const total = buckets.reduce((sum, b) => sum + b.count, 0);
  if (total === 0) return null;
  const shades = ['var(--ink)', 'var(--ink-2)', 'var(--ink-3)', 'var(--ink-4)', 'var(--ink-5)'];
  return (
    <div className="lvl-bars" role="img" aria-label="vocab level distribution">
      {buckets.map((b, i) => {
        const pct = Math.round((b.count / total) * 100);
        const on = active === b.label;
        return (
          <button
            key={b.label}
            type="button"
            className={`lvl-bar-row ${on ? 'on' : ''}`}
            onClick={() => onPick(on ? null : b.label)}
            title={`${b.label} · ${b.count} (${pct}%)`}
          >
            <span className="lvl-bar-k">{b.label}</span>
            <span className="lvl-bar-track">
              <i style={{ width: `${Math.max(4, pct)}%`, background: shades[Math.min(i, shades.length - 1)] }} />
            </span>
            <span className="lvl-bar-v">{b.count}</span>
          </button>
        );
      })}
    </div>
  );
}
