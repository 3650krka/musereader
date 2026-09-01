import { useMemo } from 'react';
import type { ReaderActivity } from '@/types';
import type { ReadingPoint, VocabBucket } from './insightsShared';

/** lieflat 语法：胶囊柱图 + 账本横线家具，明度即数据（最深 = 最高值）。 */
export function MinutesBars({ data }: { data: ReadingPoint[] }) {
  const max = Math.max(...data.map((d) => d.minutes), 1);
  const W = 560, H = 240, BASE = 200, LEFT = 44, RIGHT = 544;
  const n = Math.max(1, data.length);
  const slot = (RIGHT - LEFT) / n;
  const barW = Math.min(34, Math.max(10, slot * 0.55));

  return (
    <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="minutes per day">
      {/* 账本横线：基线 + 4 条虚线，与原型同坐标 */}
      <g stroke="var(--paper-3)" strokeWidth="1">
        <line x1={LEFT} y1={BASE} x2={RIGHT} y2={BASE} />
        {[160, 120, 80, 40].map((y) => (
          <line key={y} x1={LEFT} y1={y} x2={RIGHT} y2={y} strokeDasharray="2 4" />
        ))}
      </g>
      <g fontSize="10" fill="var(--ink-3)" textAnchor="end" fontWeight="600">
        <text x={LEFT - 6} y={BASE + 4}>0</text>
        <text x={LEFT - 6} y={124}>{Math.round(max / 2)}</text>
        <text x={LEFT - 6} y={44}>{max}</text>
      </g>
      <g fill="var(--ink)">
        {data.map((d, i) => {
          const h = Math.max(4, (d.minutes / max) * 160);
          const x = LEFT + slot * i + (slot - barW) / 2;
          const y = BASE - h;
          const isLast = i === data.length - 1;
          return (
            <rect
              key={d.date}
              x={x} y={y} width={barW} height={h} rx={barW / 2}
              fill={isLast ? 'var(--signal)' : 'var(--ink)'}
            >
              <title>{d.date} · {d.minutes} min</title>
            </rect>
          );
        })}
      </g>
      <g fontSize="11" fill="var(--ink-3)" textAnchor="middle" fontWeight="600">
        {data.map((d, i) => (
          <text key={d.date} x={LEFT + slot * i + slot / 2} y={BASE + 20}>{d.date}</text>
        ))}
      </g>
      {/* 柱顶数值：800 字重，末柱信号色 */}
      <g fontSize="10.5" fill="var(--ink)" textAnchor="middle" fontWeight="800">
        {data.map((d, i) => {
          const h = Math.max(4, (d.minutes / max) * 160);
          const isLast = i === data.length - 1;
          return (
            <text
              key={`${d.date}-v`}
              x={LEFT + slot * i + slot / 2}
              y={BASE - h - 8}
              fill={isLast ? 'var(--signal)' : 'var(--ink)'}
            >
              {d.minutes}
            </text>
          );
        })}
      </g>
    </svg>
  );
}

/** 30 天视图：平滑面积折线（catmull-rom → bezier），峰值点信号色，周刻度。 */
export function MinutesTrend({ data }: { data: ReadingPoint[] }) {
  const W = 560, H = 240, BASE = 200, LEFT = 44, RIGHT = 544;
  const max = Math.max(...data.map((d) => d.minutes), 1);
  const span = Math.max(1, data.length - 1);
  const px = (i: number) => LEFT + ((RIGHT - LEFT) / span) * i;
  const py = (v: number) => BASE - (v / max) * 160;

  const pts = data.map((d, i) => [px(i), py(d.minutes)] as const);
  const line = pts
    .map((p, i) => {
      if (i === 0) return `M ${p[0]} ${p[1]}`;
      const [x0, y0] = pts[i - 1];
      const cx = (x0 + p[0]) / 2;
      return `C ${cx} ${y0}, ${cx} ${p[1]}, ${p[0]} ${p[1]}`;
    })
    .join(' ');
  const area = `${line} L ${px(data.length - 1)} ${BASE} L ${LEFT} ${BASE} Z`;
  const peakIndex = data.reduce((best, d, i) => (d.minutes > data[best].minutes ? i : best), 0);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="minutes trend">
      <g stroke="var(--paper-3)" strokeWidth="1">
        <line x1={LEFT} y1={BASE} x2={RIGHT} y2={BASE} />
        {[160, 120, 80, 40].map((y) => (
          <line key={y} x1={LEFT} y1={y} x2={RIGHT} y2={y} strokeDasharray="2 4" />
        ))}
      </g>
      <g fontSize="10" fill="var(--ink-3)" textAnchor="end" fontWeight="600">
        <text x={LEFT - 6} y={BASE + 4}>0</text>
        <text x={LEFT - 6} y={124}>{Math.round(max / 2)}</text>
        <text x={LEFT - 6} y={44}>{max}</text>
      </g>
      <path d={area} fill="var(--ink)" opacity={0.07} />
      <path d={line} fill="none" stroke="var(--ink)" strokeWidth="2.2" strokeLinecap="round" />
      {data.map((d, i) =>
        i === peakIndex ? (
          <circle key={d.date} cx={px(i)} cy={py(d.minutes)} r={4.5} fill="var(--signal)" stroke="var(--card)" strokeWidth="2" />
        ) : d.minutes > 0 ? (
          <circle key={d.date} cx={px(i)} cy={py(d.minutes)} r={3} fill="var(--ink)" opacity={0.85}>
            <title>{d.date} · {d.minutes} min</title>
          </circle>
        ) : null,
      )}
      <g fontSize="10.5" fill="var(--ink)" textAnchor="middle" fontWeight="800">
        <text x={px(peakIndex)} y={py(data[peakIndex].minutes) - 10} fill="var(--signal)">{data[peakIndex].minutes}</text>
      </g>
      <g fontSize="10" fill="var(--ink-3)" textAnchor="middle" fontWeight="600">
        {data.map((d, i) => (i % 7 === 0 || i === data.length - 1 ? (
          <text key={d.date} x={px(i)} y={BASE + 20}>{d.date}</text>
        ) : null))}
      </g>
    </svg>
  );
}

/** 生词难度环形图：档位圆弧 + 中心总数 + 底部图例。 */
export function VocabDonut({ buckets, total }: { buckets: VocabBucket[]; total: number }) {
  const R = 88, SW = 22, CIRC = 553;
  const colors = ['var(--ink)', 'var(--ink-2)', 'var(--ink-4)', 'var(--ink-5)'];
  let acc = 0;
  const arcs = buckets.map((b, i) => {
    const frac = total > 0 ? b.count / total : 0;
    const len = Math.max(0, frac * CIRC - 6);
    const off = -acc * CIRC;
    acc += frac;
    return { ...b, len, off, color: colors[i % colors.length] };
  });

  return (
    <svg viewBox="0 0 320 240" role="img" aria-label="vocab level distribution">
      <g transform="translate(160,112)">
        <circle r={R} fill="none" stroke="var(--paper-2)" strokeWidth={SW} />
        {arcs.map((a) => (
          <circle
            key={a.label}
            r={R} fill="none" stroke={a.color} strokeWidth={SW}
            strokeDasharray={`${a.len} ${CIRC}`} strokeDashoffset={a.off}
            strokeLinecap="round" transform="rotate(-90)"
          >
            <title>{a.label} · {a.count}</title>
          </circle>
        ))}
        <text y="-6" textAnchor="middle" fontSize="34" fontWeight="800" fill="var(--ink)">{total}</text>
        <text y="16" textAnchor="middle" fontSize="10.5" letterSpacing="2" fill="var(--ink-4)">WORDS</text>
      </g>
      <g fontSize="11" fontWeight="600" fill="var(--ink-2)">
        {arcs.map((a, i) => {
          const x = 20 + i * 104;
          return (
            <g key={a.label}>
              <text x={x} y={222} fill={a.color}>■</text>
              <text x={x + 15} y={222}>{`${a.label} · ${a.count}`}</text>
            </g>
          );
        })}
      </g>
    </svg>
  );
}
export function YearHeat({ activities }: { activities: ReaderActivity[] }) {
  const weeks = useMemo(() => {
    const buckets = new Array<number>(52).fill(0);
    const now = Date.now();
    for (const a of activities) {
      const t = new Date(a.createdAt).getTime();
      if (Number.isNaN(t)) continue;
      const weeksAgo = Math.floor((now - t) / (7 * 24 * 3600 * 1000));
      if (weeksAgo >= 0 && weeksAgo < 52) buckets[51 - weeksAgo] += a.minutes;
    }
    const max = Math.max(...buckets, 1);
    return buckets.map((v) => (v === 0 ? 0 : Math.min(4, Math.ceil((v / max) * 4))));
  }, [activities]);

  return (
    <div className="heat2">
      {weeks.map((level, i) => (
        <i key={i} data-l={level} />
      ))}
    </div>
  );
}

/** 活动构成：阅读/复习/翻译 横向堆叠条，语义配色（强调色/成功色/信号色）。 */
export function ActivitySplit({ items }: { items: { label: string; minutes: number; color: string }[] }) {
  const total = Math.max(items.reduce((s, i) => s + i.minutes, 0), 1);
  return (
    <div className="act-split">
      <div className="act-split-bar" role="img" aria-label="activity composition">
        {items.filter((i) => i.minutes > 0).map((i) => (
          <span
            key={i.label}
            style={{ width: `${(i.minutes / total) * 100}%`, background: i.color }}
            title={`${i.label} · ${i.minutes} min`}
          />
        ))}
      </div>
      <div className="act-split-legend">
        {items.map((i) => (
          <span key={i.label} className="act-split-item">
            <i style={{ background: i.color }} />
            {i.label}
            <b>{i.minutes}</b>
            <small>min · {Math.round((i.minutes / total) * 100)}%</small>
          </span>
        ))}
      </div>
    </div>
  );
}

/** 洞察页图表（lieflat-charts 设计原则对齐：明度即数据 + 可数刻度 + 彩色色系）。 */
const INSIGHT_PALETTE = ['#5B8C7E', '#7FA5C4', '#C4A06A', '#B47878', '#8A9BB0', '#A5B48A'];

/** 每书进度横向进度条（Glance 型：粗柱 + 排序 + 彩色）。 */
export function BookProgressBars({ items }: { items: { title: string; progress: number }[] }) {
  const sorted = [...items].sort((a, b) => b.progress - a.progress).slice(0, 8);
  return (
    <div className="prog-bars" role="img" aria-label="book progress">
      {sorted.map((item, i) => (
        <div key={item.title} className="prog-bar-row">
          <span className="prog-bar-label truncate" title={item.title}>{item.title}</span>
          <span className="prog-bar-track">
            <i style={{ width: `${Math.max(3, item.progress)}%`, background: INSIGHT_PALETTE[i % INSIGHT_PALETTE.length] }} />
          </span>
          <span className="prog-bar-val">{item.progress}%</span>
        </div>
      ))}
    </div>
  );
}

/** 记忆质量分布（复习 quality 0–5）：胶囊柱 + 明度阶。 */
export function QualityBars({ distribution }: { distribution: { quality: number; count: number }[] }) {
  const max = Math.max(...distribution.map((d) => d.count), 1);
  return (
    <div className="q-bars" role="img" aria-label="review quality distribution">
      {distribution.map((d) => (
        <div key={d.quality} className="q-bar-col">
          <span className="q-bar-num">{d.count}</span>
          <span className="q-bar" style={{ height: `${Math.max(8, (d.count / max) * 64)}px`, background: INSIGHT_PALETTE[d.quality % INSIGHT_PALETTE.length] }} />
          <span className="q-bar-k">Q{d.quality}</span>
        </div>
      ))}
    </div>
  );
}
