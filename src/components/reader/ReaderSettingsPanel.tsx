import { motion } from 'motion/react';
import { SlidersHorizontal, X } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { ReaderFontFamily } from './epubReaderTypes';
import { UI } from './readerShared';
import type { ReaderPrefs } from './useReaderPrefs';
import { Slider } from '@/components/common/Slider';
import { useT } from '@/i18n';

interface ReaderSettingsPanelProps {
  prefs: ReaderPrefs;
  onChange: (patch: Partial<ReaderPrefs>) => void;
  onClose: () => void;
  /** 打开不常用设置的居中弹窗。 */
  onOpenAdvanced: () => void;
}

const THEMES: { id: ReaderPrefs['themeName']; bg: string; fg: string }[] = [
  { id: 'paper', bg: '#f6f2ea', fg: '#23211d' },
  { id: 'light', bg: '#ffffff', fg: '#1c1c1a' },
  { id: 'sepia', bg: '#e9e6e0', fg: '#2a2825' },
  { id: 'green', bg: '#dfeadf', fg: '#25301f' },
  { id: 'parchment', bg: '#f3e7c9', fg: '#3d2f1e' },
  { id: 'dark', bg: '#17181a', fg: '#d8d5cf' },
];

const LANGS: { id: ReaderPrefs['lang'] }[] = [
  { id: 'both' },
  { id: 'zh' },
  { id: 'en' },
];

const FLOWS: { id: ReaderPrefs['flow']; label: string }[] = [
  { id: 'paginated', label: UI.paginated },
  { id: 'spread', label: UI.flowSpread },
  { id: 'scrolled', label: UI.scrolled },
];

const PAGE_WIDTHS: { id: ReaderPrefs['pageWidth']; label: string }[] = [
  { id: 'narrow', label: UI.typoNarrow },
  { id: 'standard', label: UI.typoStandard },
  { id: 'wide', label: UI.typoWide },
];

/* 快捷字体档位：自定义字体文件管理在更多设置弹窗，这里只给三档常用切换；
    当前为 custom 时补一枚入口按钮跳转到弹窗（不做无效切换）。 */
const FONT_QUICK: { id: ReaderFontFamily; label: string }[] = [
  { id: 'auto', label: UI.typoFontAuto },
  { id: 'serif', label: UI.typoFontSerif },
  { id: 'sans', label: UI.typoFontSans },
];

/** 快捷阅读设置面板：阅读模式（翻页/行宽）+ 排版 + 外观；不常用项进更多设置弹窗。
    点外自动收起（含 iframe 正文点击，经 mt:iframe-pointerdown 桥）+ Esc。 */
export function ReaderSettingsPanel({ prefs, onChange, onClose, onOpenAdvanced }: ReaderSettingsPanelProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onPointerDown = (event: Event) => {
      if (ref.current?.contains(event.target as Node)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('mt:iframe-pointerdown', onPointerDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('mt:iframe-pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [onClose]);

  return (
    <motion.div
      ref={ref}
      initial={{ opacity: 0, y: -8, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -6, scale: 0.98 }}
      transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
      className="settings-panel"
      role="dialog"
      aria-label={UI.typoPanel}
    >
      <div className="settings-panel-head">
        <span>{UI.typoPanel}</span>
        <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={onClose} aria-label={UI.close}>
          <X size={14} />
        </button>
      </div>

      <div className="settings-panel-group">{UI.groupMode}</div>
      <div className="settings-panel-row">
        <span className="label">{UI.spread}</span>
        <div className="seg">
          {FLOWS.map((f) => (
            <button key={f.id} type="button" data-active={prefs.flow === f.id} onClick={() => onChange({ flow: f.id })}>
              {f.label}
            </button>
          ))}
        </div>
      </div>
      <div className="settings-panel-row">
        <span className="label">{UI.typoColumn}</span>
        <div className="seg">
          {PAGE_WIDTHS.map((w) => (
            <button
              key={w.id}
              type="button"
              data-active={prefs.pageWidth === w.id}
              onClick={() => onChange({ pageWidth: w.id })}
            >
              {w.label}
            </button>
          ))}
        </div>
      </div>

      <div className="settings-panel-group">{UI.groupType}</div>
      <div className="settings-panel-row">
        <span className="label">{UI.typoFontSize}</span>
        <Slider
          min={0.5} max={3.0} step={0.1}
          value={prefs.fontSizeEm}
          onChange={(v) => onChange({ fontSizeEm: v })}
          ariaLabel={UI.typoFontSize}
          formatValue={(v) => v.toFixed(2)}
        />
        <em>{prefs.fontSizeEm.toFixed(2)}</em>
      </div>
      <div className="settings-panel-row">
        <span className="label">{UI.typoLineHeight}</span>
        <Slider
          min={1.0} max={3.0} step={0.1}
          value={prefs.lineHeight}
          onChange={(v) => onChange({ lineHeight: v })}
          ariaLabel={UI.typoLineHeight}
          formatValue={(v) => v.toFixed(2)}
        />
        <em>{prefs.lineHeight.toFixed(2)}</em>
      </div>
      <div className="settings-panel-row">
        <span className="label">{UI.typoParaSpacing}</span>
        <Slider
          min={0} max={5} step={0.5}
          value={prefs.paragraphSpacingEm}
          onChange={(v) => onChange({ paragraphSpacingEm: v })}
          ariaLabel={UI.typoParaSpacing}
          formatValue={(v) => v.toFixed(1)}
        />
        <em>{prefs.paragraphSpacingEm.toFixed(1)}</em>
      </div>
      <div className="settings-panel-row">
        <span className="label">{UI.typoFont}</span>
        <div className="seg">
          {FONT_QUICK.map((f) => (
            <button key={f.id} type="button" data-active={prefs.fontFamily === f.id} onClick={() => onChange({ fontFamily: f.id })}>
              {f.label}
            </button>
          ))}
          {prefs.fontFamily === 'custom' && (
            <button type="button" data-active title={UI.fontCustom} onClick={onOpenAdvanced}>
              {UI.fontCustomShort}
            </button>
          )}
        </div>
      </div>
      <div className="settings-panel-row">
        <span className="label">{UI.typoFontEn}</span>
        <div className="seg">
          {FONT_QUICK.map((f) => (
            <button key={f.id} type="button" data-active={prefs.enFontFamily === f.id} onClick={() => onChange({ enFontFamily: f.id })}>
              {f.label}
            </button>
          ))}
          {prefs.enFontFamily === 'custom' && (
            <button type="button" data-active title={UI.fontCustom} onClick={onOpenAdvanced}>
              {UI.fontCustomShort}
            </button>
          )}
        </div>
      </div>

      <div className="settings-panel-group">{UI.groupLook}</div>
      <div className="settings-panel-row">
        <span className="label">{UI.theme}</span>
        <div className="seg">
          {THEMES.map((th) => (
            <button
              key={th.id}
              type="button"
              data-active={prefs.themeName === th.id}
              onClick={() => onChange({ themeName: th.id })}
              style={{ background: th.bg, color: th.fg }}
              title={t.reader.themeLabels[th.id]}
            >
              A
            </button>
          ))}
          {/* 自定义主题：选中后可在下方直接编辑背景/文字色 */}
          <button
            type="button"
            data-active={prefs.themeName === 'custom'}
            onClick={() => onChange({ themeName: 'custom' })}
            style={{
              background: prefs.customThemeBg,
              color: prefs.customThemeFg,
              border: '1px dashed rgba(128,128,128,.5)',
            }}
            title={UI.themeCustom}
          >
            +
          </button>
        </div>
      </div>
      {prefs.themeName === 'custom' && (
        <div className="settings-panel-row">
          <span className="label">{UI.themeCustom}</span>
          <div className="seg theme-custom-colors">
            <label className="color-chip" title={UI.themeBg}>
              {UI.themeBg}
              <input
                type="color"
                value={prefs.customThemeBg}
                onChange={(e) => onChange({ customThemeBg: e.target.value })}
              />
            </label>
            <label className="color-chip" title={UI.themeFg}>
              {UI.themeFg}
              <input
                type="color"
                value={prefs.customThemeFg}
                onChange={(e) => onChange({ customThemeFg: e.target.value })}
              />
            </label>
          </div>
        </div>
      )}
      <div className="settings-panel-row">
        <span className="label">{UI.bilingual}</span>
        <div className="seg">
          {LANGS.map((l) => (
            <button key={l.id} type="button" data-active={prefs.lang === l.id} onClick={() => onChange({ lang: l.id })}>
              {l.id === 'both' ? t.reader.langBoth : l.id === 'zh' ? t.reader.langZh : t.reader.langEn}
            </button>
          ))}
        </div>
      </div>

      {/* 不常用项（学习辅助/生词细节）收进居中弹窗，快捷面板保持轻量 */}
      <button type="button" className="settings-more" onClick={onOpenAdvanced}>
        {UI.advanced}
        <SlidersHorizontal size={13} />
      </button>
    </motion.div>
  );
}
