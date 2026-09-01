import { motion } from 'motion/react';
import { Trash2, Upload, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { useEffect, useState } from 'react';
import { useT } from '@/i18n';
import { UI } from './readerShared';
import type {
  ReaderFontFamily,
  ReaderHeaderFooter,
  ReaderInfoSlot,
  ReaderPageTurnStyle,
  ReaderTextAlign,
} from './epubReaderTypes';
import type { ReaderPrefs } from './useReaderPrefs';
import { ZoneGridEditor, ZonePresetPicker } from './ZoneGridEditor';
import { Slider } from '@/components/common/Slider';
import './readerAdvanced.css';

interface ReaderAdvancedPanelProps {
  prefs: ReaderPrefs;
  onChange: (patch: Partial<ReaderPrefs>) => void;
  onClose: () => void;
}

const WORD_LEVELS: ReaderPrefs['userLevel'][] = [
  '中考', '高考', '四级', '六级', '考研', '雅思', '托福', '专四', '专八', 'GRE',
];

const WW_STYLES: { id: ReaderPrefs['wordWiseStyle']; label: string }[] = [
  { id: 'underline', label: UI.wwUnderline },
  { id: 'highlight', label: UI.wwHighlight },
  { id: 'plain', label: UI.wwPlain },
];

/* 标注主色预设：与阅读主题成对的可读色（暖棕为默认）。 */
const WW_COLORS = ['#b45309', '#c2410c', '#7c3aed', '#0f766e', '#1d4ed8', '#be185d'];

const FONTS: { id: ReaderFontFamily; label: string }[] = [
  { id: 'auto', label: UI.typoFontAuto },
  { id: 'serif', label: UI.typoFontSerif },
  { id: 'sans', label: UI.typoFontSans },
  { id: 'custom', label: UI.fontCustom },
];

const TURNS: { id: ReaderPageTurnStyle; label: string }[] = [
  { id: 'slide', label: UI.turnSlide },
  { id: 'fade', label: UI.turnFade },
  { id: 'book', label: UI.turnBook },
  { id: 'flip', label: UI.turnFlip },
  { id: 'none', label: UI.turnNone },
];

const SLOT_OPTIONS: { id: ReaderInfoSlot; label: string }[] = [
  { id: 'none', label: UI.slotNone },
  { id: 'chapter', label: UI.slotChapter },
  { id: 'chapterProgress', label: UI.slotChapterProgress },
  { id: 'bookProgress', label: UI.slotBookProgress },
  { id: 'time', label: UI.slotTime },
];

const ALIGNS: { id: ReaderTextAlign; label: string }[] = [
  { id: 'auto', label: UI.alignAuto },
  { id: 'left', label: UI.alignLeft },
  { id: 'center', label: UI.alignCenter },
  { id: 'right', label: UI.alignRight },
  { id: 'justify', label: UI.alignJustify },
];

const FLOWS: { id: ReaderPrefs['flow']; label: string }[] = [
  { id: 'paginated', label: UI.flowPaginated },
  { id: 'spread', label: UI.flowSpread },
  { id: 'scrolled', label: UI.flowScrolled },
];

/** 页眉/页脚单槽位下拉。 */
function SlotSelect(props: {
  value: ReaderInfoSlot;
  onChange: (v: ReaderInfoSlot) => void;
}) {
  return (
    <select
      className="settings-select"
      value={props.value}
      onChange={(e) => props.onChange(e.target.value as ReaderInfoSlot)}
    >
      {SLOT_OPTIONS.map((o) => (
        <option key={o.id} value={o.id}>{o.label}</option>
      ))}
    </select>
  );
}

/** 页眉/页脚三槽位编辑行（左/中/右各一个下拉）。 */
function SlotsRow(props: {
  label: string;
  value: ReaderHeaderFooter;
  onChange: (v: ReaderHeaderFooter) => void;
}) {
  const { label, value, onChange } = props;
  return (
    <div className="settings-panel-row slots-row">
      <span className="label">{label}</span>
      <div className="slots-group">
        <label>{UI.slotLeft}<SlotSelect value={value.left} onChange={(v) => onChange({ ...value, left: v })} /></label>
        <label>{UI.slotCenter}<SlotSelect value={value.center} onChange={(v) => onChange({ ...value, center: v })} /></label>
        <label>{UI.slotRight}<SlotSelect value={value.right} onChange={(v) => onChange({ ...value, right: v })} /></label>
      </div>
    </div>
  );
}

/** 滑杆行：标签 + 滑杆 + 数值显示。 */
function SliderRow(props: {
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  onChange: (v: number) => void;
  format?: (v: number) => string;
}) {
  const { label, min, max, step, value, onChange, format } = props;
  return (
    <div className="settings-panel-row">
      <span className="label">{label}</span>
      <Slider
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={onChange}
        ariaLabel={label}
        formatValue={format}
      />
      <em>{format ? format(value) : value.toFixed(step < 1 ? 2 : 0)}</em>
    </div>
  );
}

/** 分段选择按钮组。 */
function Segmented<T extends string>(props: {
  options: { id: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="seg">
      {props.options.map((o) => (
        <button
          key={o.id}
          type="button"
          data-active={props.value === o.id}
          onClick={() => props.onChange(o.id)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** 开关行：标签 + switch 按钮（aria-label 承载控件名，便于自动化与无障碍）。 */
function SwitchRow(props: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  hint?: string;
}) {
  const { label, checked, onChange, hint } = props;
  return (
    <>
      <div className="settings-panel-row">
        <span className="label wide">{label}</span>
        <button
          type="button"
          className={`switch ${checked ? 'on' : ''}`}
          onClick={() => onChange(!checked)}
          aria-pressed={checked}
          aria-label={label}
        />
      </div>
      {hint && <div className="settings-panel-hint">{hint}</div>}
    </>
  );
}

/** 自定义字体管理行：列表选择 + 导入 + 删除。 */
interface ReaderFontItem {
  fileName: string;
  label: string;
  sizeBytes: number;
  path: string;
}

function FontManagerRow(props: {
  currentPath: string;
  onPick: (font: ReaderFontItem) => void;
  /** 删除的是当前使用字体时重置选择（防路径残留指向已删文件）。 */
  onRemoveCurrent: () => void;
}) {
  const { currentPath, onPick, onRemoveCurrent } = props;
  const [fonts, setFonts] = useState<ReaderFontItem[]>([]);
  const [busy, setBusy] = useState(false);

  const reload = () => {
    invoke<ReaderFontItem[]>('list_reader_fonts')
      .then(setFonts)
      .catch((error) => console.error('list fonts failed:', error));
  };

  useEffect(reload, []);

  const importFont = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const picked = await openFileDialog({
        multiple: false,
        filters: [{ name: 'Fonts', extensions: ['ttf', 'otf', 'woff', 'woff2'] }],
      });
      if (!picked) return;
      const next = await invoke<ReaderFontItem[]>('import_reader_font', { sourcePath: picked });
      setFonts(next);
      const imported = next.find((f) => f.path === picked) ?? next[next.length - 1];
      if (imported) onPick(imported);
    } catch (error) {
      console.error('import font failed:', error);
    } finally {
      setBusy(false);
    }
  };

  const removeFont = async () => {
    if (busy) return;
    const current = fonts.find((f) => f.path === currentPath);
    if (!current) return;
    setBusy(true);
    try {
      const next = await invoke<ReaderFontItem[]>('delete_reader_font', { fileName: current.fileName });
      setFonts(next);
      onRemoveCurrent();
    } catch (error) {
      console.error('delete font failed:', error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-panel-row" style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
      <select
        className="settings-select"
        style={{ flex: 1 }}
        value={currentPath}
        disabled={fonts.length === 0}
        onChange={(e) => {
          const font = fonts.find((f) => f.path === e.target.value);
          if (font) onPick(font);
        }}
        aria-label={UI.fontCustom}
      >
        {fonts.length === 0 && <option value="">{UI.fontEmpty}</option>}
        {!fonts.some((f) => f.path === currentPath) && <option value="">{UI.fontEmpty}</option>}
        {fonts.map((font) => (
          <option key={font.fileName} value={font.path}>
            {font.label}（{(font.sizeBytes / 1024 / 1024).toFixed(1)}MB）
          </option>
        ))}
      </select>
      <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={() => void importFont()} disabled={busy} title={UI.fontImport} aria-label={UI.fontImport}>
        <Upload size={13} />
      </button>
      <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={() => void removeFont()} disabled={busy || !fonts.some((f) => f.path === currentPath)} title={UI.fontRemove} aria-label={UI.fontRemove}>
        <Trash2 size={13} />
      </button>
    </div>
  );
}

/** 本地路径文件名截断显示。 */
function bgFileName(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  return name.length > 24 ? `${name.slice(0, 12)}…${name.slice(-8)}` : name;
}

/** 更多设置：全部设置项数值化、分组即时生效。 */
export function ReaderAdvancedPanel({ prefs, onChange, onClose }: ReaderAdvancedPanelProps) {
  const t = useT();
  const extra = prefs.typeExtra;
  /* 词库就绪探测（不内置词库后，wordwise 依赖用户导入）：面板打开时查一次。 */
  const [wordlistMissing, setWordlistMissing] = useState(false);
  useEffect(() => {
    let alive = true;
    invoke<{ empty: boolean }>('wordlist_status')
      .then((status) => {
        if (alive) setWordlistMissing(status.empty);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);
  const patchExtra = (patch: Partial<ReaderPrefs['typeExtra']>) =>
    onChange({ typeExtra: { ...extra, ...patch } });

  /* 背景图选择：复制进字体同目录（复用 asset 授权），存路径进 prefs。 */
  const pickBgImage = async () => {
    try {
      const picked = await openFileDialog({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'avif'] }],
      });
      if (!picked) return;
      // 复用字体导入通道落盘（扩展校验对图片放行由后端字体白名单之外的兜底：直接存路径，
      // 若文件在受管目录外则退化为原路径——asset scope 已在字体导入时授权整目录）。
      onChange({ themeBgImagePath: picked });
    } catch (error) {
      console.error('pick bg image failed:', error);
    }
  };

  return (
    <>
      <div className="adv-dialog-backdrop" onClick={onClose} />
      <motion.div
        className="adv-dialog"
        initial={{ opacity: 0, y: 20, scale: 0.96 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 20, scale: 0.96 }}
        transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
        role="dialog"
        aria-label={UI.advanced}
      >
        <div className="adv-dialog-head">
          <span>{UI.advanced}</span>
          <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={onClose} aria-label={UI.close}>
            <X size={14} />
          </button>
        </div>
        <div className="adv-anchor-nav" aria-label="分组导航">
          {[
            ['g-font', UI.groupFont], ['g-type', UI.groupTypeExtra], ['g-look', UI.groupLook],
            ['g-turn', UI.groupPageTurn], ['g-display', UI.groupDisplay], ['g-hf', UI.groupHeaderFooter],
            ['g-assist', UI.groupAssist], ['g-words', UI.groupWords], ['g-adv', UI.groupAdvanced],
          ].map(([gid, label]) => (
            <button
              key={gid}
              type="button"
              onClick={() => document.getElementById(gid)?.scrollIntoView({ behavior: 'smooth', block: 'start' })}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="adv-dialog-scroll">
          <div className="adv-dialog-body">
            {/* ── 字体 ── */}
            <div className="settings-panel-group" id="g-font">{UI.groupFont}</div>
            <SliderRow
              label={UI.fontSize}
              min={0.5} max={3.0} step={0.05}
              value={prefs.fontSizeEm}
              onChange={(v) => onChange({ fontSizeEm: v })}
              format={(v) => `${Math.round(v * 100)}%`}
            />
            <div className="settings-panel-row">
              <span className="label">{UI.typoFont}</span>
              <Segmented options={FONTS} value={prefs.fontFamily} onChange={(v) => onChange({ fontFamily: v })} />
            </div>
            {prefs.fontFamily === 'custom' && (
              <FontManagerRow
                currentPath={prefs.customFontPath}
                onPick={(font) => onChange({ customFontPath: font.path, customFontLabel: font.label })}
                onRemoveCurrent={() => onChange({ customFontPath: '', customFontLabel: '' })}
              />
            )}
            {/* 英文字体（中英文分开设置）：双语书英文栏专用；auto=跟随中文字体栈的拉丁字形 */}
            <div className="settings-panel-row">
              <span className="label">{UI.typoFontEn}</span>
              <Segmented options={FONTS} value={prefs.enFontFamily} onChange={(v) => onChange({ enFontFamily: v })} />
            </div>
            {prefs.enFontFamily === 'custom' && (
              <FontManagerRow
                currentPath={prefs.customEnFontPath}
                onPick={(font) => onChange({ customEnFontPath: font.path, customEnFontLabel: font.label })}
                onRemoveCurrent={() => onChange({ customEnFontPath: '', customEnFontLabel: '' })}
              />
            )}

            {/* ── 细节排版 ── */}
            <div className="settings-panel-group" id="g-type">{UI.groupTypeExtra}</div>
            <SliderRow label={UI.lineHeight} min={1.0} max={3.0} step={0.2} value={prefs.lineHeight} onChange={(v) => onChange({ lineHeight: v })} format={(v) => v.toFixed(1)} />
            <SliderRow label={UI.paraSpacing} min={0} max={5} step={0.5} value={prefs.paragraphSpacingEm} onChange={(v) => onChange({ paragraphSpacingEm: v })} format={(v) => `${v.toFixed(1)}em`} />
            <SliderRow label={UI.paragraphGap} min={0} max={60} step={2} value={prefs.paragraphGapPx} onChange={(v) => onChange({ paragraphGapPx: v })} format={(v) => `${v}px`} />
            <SliderRow label={UI.typoLetterSpacing} min={-3} max={7} step={1} value={extra.letterSpacingPx} onChange={(v) => patchExtra({ letterSpacingPx: v })} format={(v) => `${v}px`} />
            <SliderRow label={UI.typoWordSpacing} min={0} max={7} step={1} value={extra.wordSpacingPx} onChange={(v) => patchExtra({ wordSpacingPx: v })} format={(v) => `${v}px`} />
            <SliderRow
              label={UI.typoIndent} min={-0.5} max={8} step={0.5}
              value={extra.indentEm} onChange={(v) => patchExtra({ indentEm: v })}
              format={(v) => (v < 0 ? UI.alignAuto : `${v.toFixed(1)}em`)}
            />
            <SliderRow label={UI.typoHeading} min={0.5} max={2.0} step={0.1} value={extra.headingScale} onChange={(v) => patchExtra({ headingScale: v })} format={(v) => `×${v.toFixed(1)}`} />
            <SliderRow
              label={UI.typoFontWeight} min={100} max={900} step={100}
              value={extra.fontWeight} onChange={(v) => patchExtra({ fontWeight: v })}
              format={(v) => String(v)}
            />
            <div className="settings-panel-row">
              <span className="label">{UI.typoAlign}</span>
              <Segmented options={ALIGNS} value={extra.textAlignment} onChange={(v) => patchExtra({ textAlignment: v })} />
            </div>

            {/* ── 外观 ── */}
            <div className="settings-panel-group" id="g-look">{UI.groupLook}</div>
            <div className="settings-panel-row">
              <span className="label">{UI.theme}</span>
              <div className="seg">
                {(['paper', 'light', 'sepia', 'green', 'parchment', 'dark'] as const).map((name) => (
                  <button
                    key={name}
                    type="button"
                    data-active={prefs.themeName === name}
                    onClick={() => onChange({ themeName: name })}
                  >
                    {name === 'paper' ? '纸' : name === 'light' ? '白' : name === 'sepia' ? '褐' : name === 'green' ? '绿' : name === 'parchment' ? '羊' : '夜'}
                  </button>
                ))}
              </div>
            </div>
            <SwitchRow label={UI.autoTheme} checked={prefs.autoTheme} onChange={(v) => onChange({ autoTheme: v })} hint={UI.autoThemeHint} />
            <SwitchRow label={UI.bookLook} checked={prefs.bookLook} onChange={(v) => onChange({ bookLook: v })} hint={UI.bookLookHint} />
            <div className="settings-panel-row" style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
              <span className="label">{UI.themeBgImage}</span>
              <span className="truncate" style={{ flex: 1, fontSize: 12, color: 'var(--ink-3)' }} title={prefs.themeBgImagePath}>
                {prefs.themeBgImagePath ? bgFileName(prefs.themeBgImagePath) : UI.themeBgNone}
              </span>
              <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={() => void pickBgImage()} title={UI.themeBgPick} aria-label={UI.themeBgPick}>
                <Upload size={13} />
              </button>
              {prefs.themeBgImagePath && (
                <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={() => onChange({ themeBgImagePath: '' })} title={UI.fontRemove} aria-label={UI.fontRemove}>
                  <Trash2 size={13} />
                </button>
              )}
            </div>
            {prefs.themeBgImagePath && (
              <SliderRow label={UI.themeBgOpacity} min={0.1} max={1} step={0.05} value={prefs.themeBgImageOpacity} onChange={(v) => onChange({ themeBgImageOpacity: v })} format={(v) => `${Math.round(v * 100)}%`} />
            )}
            <SwitchRow label={UI.useBookStyles} checked={prefs.useBookStyles} onChange={(v) => onChange({ useBookStyles: v })} hint={UI.useBookStylesHint} />
            <div className="settings-panel-row">
              <span className="label">{UI.columnCount}</span>
              <Segmented
                options={[
                  { id: '0', label: UI.colAuto },
                  { id: '1', label: UI.colSingle },
                  { id: '2', label: UI.colDouble },
                ]}
                value={String(prefs.columnCount)}
                onChange={(v) => onChange({ columnCount: Number(v) as 0 | 1 | 2 })}
              />
            </div>
            {prefs.columnCount === 0 && (
              <SliderRow label={UI.colThreshold} min={400} max={1200} step={20} value={prefs.columnThresholdPx} onChange={(v) => onChange({ columnThresholdPx: v })} format={(v) => `${v}px`} />
            )}

            {/* ── 翻页 ── */}
            <div className="settings-panel-group" id="g-turn">{UI.groupPageTurn}</div>
            <div className="settings-panel-row">
              <span className="label">{UI.flow}</span>
              <Segmented options={FLOWS} value={prefs.flow} onChange={(v) => onChange({ flow: v })} />
            </div>
            <div className="settings-panel-row">
              <span className="label">{UI.turnStyle}</span>
              <Segmented options={TURNS} value={prefs.pageTurnStyle} onChange={(v) => onChange({ pageTurnStyle: v })} />
            </div>
            <div className="settings-panel-row">
              <span className="label">{UI.turnZones}</span>
            </div>
            <div className="settings-panel-row" style={{ display: 'block' }}>
              <ZonePresetPicker
                value={prefs.pageTurnZones}
                customZones={prefs.pageTurnCustomZones}
                labels={{
                  standard: UI.zoneStandard,
                  swapped: UI.zoneSwapped,
                  centerTurn: UI.zoneCenterTurn,
                  topMenu: UI.zoneTopMenu,
                  leftHand: UI.zoneLeftHand,
                  custom: UI.zoneCustom,
                }}
                onChange={(v) => onChange({ pageTurnZones: v })}
              />
              <div className="settings-panel-hint">{UI.zoneSpreadHint}</div>
            </div>
            {prefs.pageTurnZones === 'custom' && (
              <div className="settings-panel-row" style={{ display: 'block' }}>
                <ZoneGridEditor
                  zones={prefs.pageTurnCustomZones}
                  onChange={(zones) => onChange({ pageTurnCustomZones: zones })}
                  labels={{ prev: UI.zonePrev, next: UI.zoneNext, menu: UI.zoneMenu, none: UI.zoneNone }}
                />
                <div className="settings-panel-hint">{UI.zoneCustomHint}</div>
              </div>
            )}
            <SliderRow label={UI.sideMargin} min={0} max={20} step={1} value={prefs.sideMarginPercent} onChange={(v) => onChange({ sideMarginPercent: v })} format={(v) => `${v}%`} />
            <div className="settings-panel-row">
              <span className="label">{UI.pageWidth}</span>
              <Segmented
                options={[
                  { id: 'narrow', label: UI.widthNarrow },
                  { id: 'standard', label: UI.widthStandard },
                  { id: 'wide', label: UI.widthWide },
                ]}
                value={prefs.pageWidth}
                onChange={(v) => onChange({ pageWidth: v })}
              />
            </div>
            <SwitchRow label={UI.autoFlip} checked={prefs.autoFlip} onChange={(v) => onChange({ autoFlip: v })} />
            {prefs.autoFlip && (
              <SliderRow label={UI.autoFlipInterval} min={10} max={100} step={5} value={prefs.autoFlipInterval} onChange={(v) => onChange({ autoFlipInterval: v })} format={(v) => `${v}s`} />
            )}

            {/* ── 显示 ── */}
            <div className="settings-panel-group" id="g-display">{UI.groupDisplay}</div>
            <SwitchRow label={UI.keepScreenOn} checked={prefs.keepScreenOn} onChange={(v) => onChange({ keepScreenOn: v })} />
            <SliderRow label={UI.brightness} min={20} max={100} step={5} value={prefs.brightnessPercent} onChange={(v) => onChange({ brightnessPercent: v })} format={(v) => `${v}%`} />
            <SwitchRow label={UI.paraDot} checked={prefs.showParaDot} onChange={(v) => onChange({ showParaDot: v })} hint={UI.paraDotHint} />

            {/* ── 页眉页脚 ── */}
            <div className="settings-panel-group" id="g-hf">{UI.groupHeaderFooter}</div>
            <SlotsRow label={UI.header} value={prefs.headerSlots} onChange={(v) => onChange({ headerSlots: v })} />
            <SlotsRow label={UI.footer} value={prefs.footerSlots} onChange={(v) => onChange({ footerSlots: v })} />
            <SliderRow label={UI.hfFontSize} min={8} max={24} step={1} value={prefs.hfFontSizePx} onChange={(v) => onChange({ hfFontSizePx: v })} format={(v) => `${v}px`} />
            <SliderRow label={UI.vMarginTop} min={0} max={200} step={20} value={prefs.topMarginPx} onChange={(v) => onChange({ topMarginPx: v })} format={(v) => `${v}px`} />
            <SliderRow label={UI.vMarginBottom} min={0} max={200} step={20} value={prefs.bottomMarginPx} onChange={(v) => onChange({ bottomMarginPx: v })} format={(v) => `${v}px`} />

            {/* ── 辅助 ── */}
            <div className="settings-panel-group" id="g-assist">{UI.groupAssist}</div>
            <SwitchRow label={UI.recall} checked={prefs.translationBlur} onChange={(v) => onChange({ translationBlur: v })} hint={UI.recallHint} />
            <div className="settings-panel-row">
              <span className="label">{UI.chineseVariant}</span>
              <Segmented
                options={[
                  { id: 'none', label: UI.cvNone },
                  { id: 's2t', label: UI.cvS2T },
                  { id: 't2s', label: UI.cvT2S },
                ]}
                value={prefs.chineseVariant}
                onChange={(v) => onChange({ chineseVariant: v })}
              />
            </div>
            <SwitchRow label={UI.keyboardTurn} checked={prefs.keyboardShortcutTurnPage} onChange={(v) => onChange({ keyboardShortcutTurnPage: v })} />
            <SwitchRow label={UI.autoTranslate} checked={prefs.autoTranslateSelection} onChange={(v) => onChange({ autoTranslateSelection: v })} />
            <SwitchRow label={UI.autoMark} checked={prefs.autoMarkSelection} onChange={(v) => onChange({ autoMarkSelection: v })} />
            <SwitchRow label={UI.clickDictionary} checked={prefs.clickDictionary} onChange={(v) => onChange({ clickDictionary: v })} hint={UI.clickDictionaryHint} />

            {/* ── 生词 ── */}
            <div className="settings-panel-group" id="g-words">{UI.groupWords}</div>
            <SwitchRow label={UI.wordAssist} checked={prefs.wordWise} onChange={(v) => onChange({ wordWise: v })} />
            {wordlistMissing && (
              <div className="hint" style={{ padding: '2px 2px 8px', fontSize: 12, color: 'var(--ink-3)', lineHeight: 1.6 }}>
                {t.reader.wordlistMissing}
              </div>
            )}
            <div className="settings-panel-row">
              <span className="label">{UI.wwLevel}</span>
              <select
                className="settings-select ww-level-select"
                value={prefs.userLevel}
                onChange={(e) => onChange({ userLevel: e.target.value as ReaderPrefs['userLevel'] })}
              >
                {WORD_LEVELS.map((level) => (
                  <option key={level} value={level}>{level}</option>
                ))}
              </select>
            </div>
            <div className="settings-panel-row">
              <span className="label">{UI.wwStyle}</span>
              <Segmented options={WW_STYLES} value={prefs.wordWiseStyle} onChange={(v) => onChange({ wordWiseStyle: v })} />
            </div>
            <div className="settings-panel-row">
              <span className="label">{UI.wwGloss}</span>
              <div className="seg">
                <button type="button" data-active={prefs.wordWiseGloss === 'zh'} onClick={() => onChange({ wordWiseGloss: 'zh' })}>
                  {UI.wwGlossZh}
                </button>
                <button type="button" data-active={prefs.wordWiseGloss === 'en'} onClick={() => onChange({ wordWiseGloss: 'en' })}>
                  {UI.wwGlossEn}
                </button>
                <button type="button" data-active={prefs.wordWiseGloss === 'phonetic'} onClick={() => onChange({ wordWiseGloss: 'phonetic' })}>
                  {UI.wwGlossPhonetic}
                </button>
              </div>
            </div>
            <SliderRow label={UI.wwSize} min={0.5} max={0.85} step={0.01} value={prefs.wordWiseSize} onChange={(v) => onChange({ wordWiseSize: v })} />
            <SliderRow label={UI.wwLineBoost} min={1} max={1.6} step={0.05} value={prefs.wordWiseLineBoost} onChange={(v) => onChange({ wordWiseLineBoost: v })} format={(v) => (v <= 1.001 ? UI.wwLineBoostOff : `×${v.toFixed(2)}`)} />
            <SliderRow label={UI.wwGap} min={0} max={48} step={2} value={prefs.wordWiseGapPx} onChange={(v) => onChange({ wordWiseGapPx: v })} format={(v) => `${Math.round(v)}px`} />
            <div className="settings-panel-row">
              <span className="label">{UI.wwColor}</span>
              <div className="ww-color-row">
                {WW_COLORS.map((color) => (
                  <button
                    key={color}
                    type="button"
                    className="ww-swatch"
                    style={{ background: color }}
                    data-active={prefs.wordWiseColor === color}
                    aria-label={color}
                    onClick={() => onChange({ wordWiseColor: color })}
                  />
                ))}
                <input
                  type="color"
                  className="ww-color-input"
                  value={prefs.wordWiseColor}
                  aria-label={UI.wwColor}
                  onChange={(e) => onChange({ wordWiseColor: e.target.value })}
                />
              </div>
            </div>

            {/* ── 高级 ── */}
            <div className="settings-panel-group" id="g-adv">{UI.groupAdvanced}</div>
            <SwitchRow label={UI.customCss} checked={prefs.customCssEnabled} onChange={(v) => onChange({ customCssEnabled: v })} hint={UI.customCssHint} />
            {prefs.customCssEnabled && (
              <div className="settings-panel-row css-row">
                <textarea
                  className="custom-css-input"
                  rows={4}
                  spellCheck={false}
                  placeholder={`body { font-family: serif; }`}
                  value={prefs.customCss}
                  onChange={(e) => onChange({ customCss: e.target.value })}
                />
              </div>
            )}
          </div>
        </div>
      </motion.div>
    </>
  );
}
