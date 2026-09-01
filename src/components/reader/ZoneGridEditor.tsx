import { ArrowLeft, ArrowRight, Ban, Menu } from 'lucide-react';
import type { ReaderZoneAction } from './readerZones';
import { ZONE_PRESETS } from './readerZones';
import type { ReaderPageTurnZones } from './epubReaderTypes';

/** 分区动作的视觉语义。 */
const ACTION_STYLE: Record<ReaderZoneAction, { bg: string; fg: string }> = {
  prev: { bg: 'rgba(90,140,220,0.28)', fg: 'var(--accent, #8a6d3b)' },
  next: { bg: 'rgba(220,120,110,0.26)', fg: 'var(--signal, #b45309)' },
  menu: { bg: 'rgba(110,180,130,0.26)', fg: 'var(--ok, #3F7D54)' },
  none: { bg: 'var(--paper-3, #ececea)', fg: 'var(--ink-4, #a3a39e)' },
};

function ZoneIcon({ action, size }: { action: ReaderZoneAction; size: number }) {
  const color = ACTION_STYLE[action].fg;
  if (action === 'prev') return <ArrowLeft size={size} color={color} />;
  if (action === 'next') return <ArrowRight size={size} color={color} />;
  if (action === 'menu') return <Menu size={size} color={color} />;
  return <Ban size={size} color={color} />;
}

/** 3×3 迷你预览图（预设选择条与图例共用）。 */
export function ZoneDiagram(props: {
  zones: ReaderZoneAction[];
  size?: number;
  selected?: boolean;
  onClick?: () => void;
  label?: string;
}) {
  const { zones, size = 52, selected = false, onClick, label } = props;
  return (
    <button
      type="button"
      className="zone-diagram"
      onClick={onClick}
      aria-pressed={selected}
      aria-label={label}
      style={{
        display: 'inline-flex', flexDirection: 'column', alignItems: 'center', gap: 4,
        background: 'none', border: 0, padding: 0, cursor: onClick ? 'pointer' : 'default',
      }}
    >
      <span
        style={{
          display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 1.5,
          width: size, height: size, padding: 3, borderRadius: 8,
          border: `1.5px solid ${selected ? 'var(--accent, #8a6d3b)' : 'var(--paper-3, #ddd)'}`,
          background: 'var(--card, #fff)',
          boxShadow: selected ? '0 0 0 2px color-mix(in srgb, var(--accent) 25%, transparent)' : 'none',
          transition: 'border-color .15s, box-shadow .15s',
        }}
      >
        {zones.map((action, i) => (
          <span
            key={i}
            style={{
              display: 'grid', placeItems: 'center', borderRadius: 3,
              background: ACTION_STYLE[action].bg,
            }}
          >
            {action !== 'none' && <ZoneIcon action={action} size={9} />}
          </span>
        ))}
      </span>
      {label && <span style={{ fontSize: 11, color: selected ? 'var(--accent)' : 'var(--ink-3)' }}>{label}</span>}
    </button>
  );
}

/** 自定义 3×3 交互编辑器：点击格子循环切换动作（prev→next→menu→none）。
    比 、一次点击即切换。 */
export function ZoneGridEditor(props: {
  zones: ReaderZoneAction[];
  onChange: (zones: ReaderZoneAction[]) => void;
  labels: Record<ReaderZoneAction, string>;
}) {
  const { zones, onChange, labels } = props;
  const ORDER: ReaderZoneAction[] = ['prev', 'next', 'menu', 'none'];
  const cycle = (index: number) => {
    const next = [...zones];
    next[index] = ORDER[(ORDER.indexOf(next[index]) + 1) % ORDER.length];
    onChange(next);
  };
  return (
    <div className="zone-editor" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div
        role="grid"
        aria-label="zone editor"
        style={{
          display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 4,
          width: '100%', maxWidth: 260, margin: '0 auto', aspectRatio: '1',
        }}
      >
        {zones.map((action, i) => (
          <button
            key={i}
            type="button"
            role="gridcell"
            onClick={() => cycle(i)}
            title={labels[action]}
            aria-label={`zone ${i + 1}: ${labels[action]}`}
            style={{
              border: 0, borderRadius: 8, cursor: 'pointer',
              display: 'grid', placeItems: 'center',
              background: ACTION_STYLE[action].bg,
              transition: 'background .15s, transform .1s',
            }}
            onMouseDown={(e) => (e.currentTarget.style.transform = 'scale(0.94)')}
            onMouseUp={(e) => (e.currentTarget.style.transform = '')}
          >
            <ZoneIcon action={action} size={20} />
          </button>
        ))}
      </div>
      <div style={{ display: 'flex', justifyContent: 'center', gap: 12, fontSize: 11.5, color: 'var(--ink-3)' }}>
        {(Object.keys(ACTION_STYLE) as ReaderZoneAction[]).map((a) => (
          <span key={a} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <ZoneIcon action={a} size={11} />
            {labels[a]}
          </span>
        ))}
      </div>
    </div>
  );
}

/** 预设选择条（含 custom 入口）。 */
export function ZonePresetPicker(props: {
  value: ReaderPageTurnZones;
  customZones: ReaderZoneAction[];
  labels: Record<Exclude<ReaderPageTurnZones, 'custom'>, string> & { custom: string };
  onChange: (v: ReaderPageTurnZones) => void;
}) {
  const { value, customZones, labels, onChange } = props;
  const presets = Object.keys(ZONE_PRESETS) as Exclude<ReaderPageTurnZones, 'custom'>[];
  return (
    <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', justifyContent: 'center' }}>
      {presets.map((id) => (
        <ZoneDiagram
          key={id}
          zones={ZONE_PRESETS[id]}
          selected={value === id}
          label={labels[id]}
          onClick={() => onChange(id)}
        />
      ))}
      <ZoneDiagram
        zones={customZones}
        selected={value === 'custom'}
        label={labels.custom}
        onClick={() => onChange('custom')}
      />
    </div>
  );
}
