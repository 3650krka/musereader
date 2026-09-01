import { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import './slider.css';

interface SliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
  ariaLabel?: string;
  /** 悬浮提示的数值格式；缺省为去除多余小数位。 */
  formatValue?: (value: number) => string;
  className?: string;
  style?: CSSProperties;
}

/** 夕珊瑚滑块：渐变填充+辉光轨道、白盘深描边拇指、拖动放大、悬浮数值提示。
    交互由透明原生 input 承担（方向键步进可用）；全站无焦点描边，聚焦以拇指光环示意。 */
export function Slider({ value, min, max, step, onChange, ariaLabel, formatValue, className, style }: SliderProps) {
  const [drag, setDrag] = useState(false);
  const pct = max > min ? Math.min(100, Math.max(0, ((value - min) / (max - min)) * 100)) : 0;

  useEffect(() => {
    if (!drag) return;
    const release = () => setDrag(false);
    window.addEventListener('pointerup', release);
    window.addEventListener('pointercancel', release);
    return () => {
      window.removeEventListener('pointerup', release);
      window.removeEventListener('pointercancel', release);
    };
  }, [drag]);

  const tip = formatValue ? formatValue(value) : String(Math.round(value * 100) / 100);

  return (
    <div
      className={className ? `cslider ${className}` : 'cslider'}
      data-active={drag ? '1' : '0'}
      style={{ ...style, '--pct': `${pct}%` } as CSSProperties}
    >
      <input
        type="range"
        min={min}
        max={max}
        step={step ?? 1}
        value={value}
        aria-label={ariaLabel}
        onChange={(e) => onChange(Number(e.target.value))}
        onPointerDown={() => setDrag(true)}
      />
      <div className="cslider-track" />
      <div className="cslider-glow" />
      <div className="cslider-fill" />
      <div className="cslider-thumb" />
      <div className="cslider-tip">{tip}</div>
    </div>
  );
}
