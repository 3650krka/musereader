import { useCallback, useEffect, useState } from 'react';
import type {
  ReaderFlow,
  ReaderFontFamily,
  ReaderHeaderFooter,
  ReaderLang,
  ReaderPageTurnStyle,
  ReaderPageTurnZones,
  ReaderPageWidth,
  ReaderTextAlign,
  ReaderTypeExtra,
} from './epubReaderTypes';
import type { WordWiseGloss, WordWiseStyle } from './wordWiseInject';
import type { ChineseVariant } from './chineseVariant';
import type { ReaderZoneAction } from './readerZones';
import { ZONE_PRESETS } from './readerZones';

/** 词汇难度档(与后端 parse_user_level 标签一致)。 */
export type ReaderWordLevel =
  | '中考' | '高考' | '四级' | '六级' | '考研' | '雅思' | '托福' | '专四' | '专八' | 'GRE';

/** 阅读背景主题。
    custom=用户自定义背景色+文字色(customThemeBg/customThemeFg)。 */
export type ReaderThemeName =
  | 'paper' | 'light' | 'sepia' | 'green' | 'parchment' | 'dark' | 'custom';


const THEME_NAMES: readonly ReaderThemeName[] = [
  'paper', 'light', 'sepia', 'green', 'parchment', 'dark', 'custom',
];

/** 主题循环的下一个主题名（顶栏快捷循环按钮反馈用）。 */
export function nextReaderThemeName(current: ReaderThemeName): ReaderThemeName {
  return THEME_NAMES[(THEME_NAMES.indexOf(current) + 1) % THEME_NAMES.length];
}

/** 阅读器偏好(排版 + 模式 + 主题 + 翻页动画 + 页眉页脚),localStorage 每设备持久化。 */
export interface ReaderPrefs {
  /** 语言模式:双语对照 / 仅中文 / 仅英文。 */
  lang: ReaderLang;
  /** 翻页模式:横向单页 / 横向双页展开 / 垂直滚动。 */
  flow: ReaderFlow;
  /** 行宽档位(单页模式的内容最大宽度)。 */
  pageWidth: ReaderPageWidth;
  /** 主动回忆:译文默认模糊,悬停/点按显示。 */
  translationBlur: boolean;
  fontSizeEm: number;
  lineHeight: number;
  paragraphSpacingEm: number;
  /** 细粒度排版(字距/词距/字重/缩进/标题缩放/对齐)。 */
  typeExtra: ReaderTypeExtra;
  /** 正文字体。 */
  fontFamily: ReaderFontFamily;
  /** 自定义字体文件绝对路径(fontFamily='custom' 时生效)。 */
  customFontPath: string;
  /** 自定义字体展示名(设置面板显示)。 */
  customFontLabel: string;
  /** 英文字体（问题：中英文分开设置——双语书英文栏专用）。 */
  enFontFamily: ReaderFontFamily;
  /** 英文自定义字体文件路径(enFontFamily='custom' 时生效)。 */
  customEnFontPath: string;
  customEnFontLabel: string;
  /** 主题名(映射到阅读器注入色板)。 */
  themeName: ReaderThemeName;
  /** 跟随系统深浅色（anx autoAdjustReadingTheme 对齐）：浅色用 themeName，深色自动切 dark。 */
  autoTheme: boolean;
  /** 自定义主题配色(themeName='custom' 时生效)。 */
  customThemeBg: string;
  customThemeFg: string;
  /** 主题背景图路径（空=无；asset 协议转 URL 后作阅读舞台背景）。 */
  themeBgImagePath: string;
  /** 背景图不透明度 0.1–1。 */
  themeBgImageOpacity: number;
  /** 翻页动画。 */
  pageTurnStyle: ReaderPageTurnStyle;
  /** 书本样式(spread 中缝阴影 + 纸页边缘层次)。 */
  bookLook: boolean;
  /** 触控翻页分区布局。 */
  pageTurnZones: ReaderPageTurnZones;
  /** 自定义 3×3 分区配置（pageTurnZones='custom' 时生效，行优先 9 格）。 */
  pageTurnCustomZones: ReaderZoneAction[];
  /** 页边距。 */
  sideMarginPercent: number;
  /** 上下页边距 px。 */
  topMarginPx: number;
  bottomMarginPx: number;
  /** 自定义 CSS。 */
  customCss: string;
  customCssEnabled: boolean;
  /** 页眉三槽位。 */
  headerSlots: ReaderHeaderFooter;
  /** 页脚三槽位。 */
  footerSlots: ReaderHeaderFooter;
  /** 页眉页脚字号 px。 */
  hfFontSizePx: number;
  /** 生词透析开关(行内 WordWise 标注)。 */
  wordWise: boolean;
  /** 用户词汇难度:仅标注高于该档的词(learning 词除外,始终标注)。 */
  userLevel: ReaderWordLevel;
  /** 生词词形样式:虚线下划线 / 高亮底色 / 仅上标。 */
  wordWiseStyle: WordWiseStyle;
  /** 上标内容:中文释义 / 音标。 */
  wordWiseGloss: WordWiseGloss;
  /** 上标字号(em 相对正文字号),0.5–0.85。 */
  wordWiseSize: number;
  /** WordWise 行距加大系数:1=关闭,1.05–1.35 常用,仅行距不动字间距。 */
  wordWiseLineBoost: number;
  /** 相邻上标最小间隙(px):同行上标间隙低于该值时自动平移避让,绝不重叠。 */
  wordWiseGapPx: number;
  /** 标注主色(上标文字+划线/高亮装饰)。 */
  wordWiseColor: string;
  /** 设置面板是否展开(不进 localStorage)。 */
  settingsOpen: boolean;
  /** 主题轮切信号(由顶栏太阳按钮触发)。 */
  themeCycle?: boolean;
  /** 音量键翻页(Tauri 桌面端适用)。 */
  volumeKeyFlip: boolean;
  /** 自动翻页开关。 */
  autoFlip: boolean;
  /** 自动翻页间隔(秒),10–100。 */
  autoFlipInterval: number;
  /** 屏幕常亮(Tauri 电源管理)。 */
  keepScreenOn: boolean;
  /** 隐藏状态栏(Tauri 全屏/沉浸模式)。 */
  hideStatusBar: boolean;
  /** 亮度百分比(0–100,桌面端通过 CSS filter 模拟)。 */
  brightnessPercent: number;
  /** 段间距 px 下限(与段距 em 取较大者生效,不叠加;0=仅用段距)。 */
  paragraphGapPx: number;
  /** 段落小红点菜单开关(长按/悬停唤出段落动作)。 */
  showParaDot: boolean;
  /** 使用书籍内置样式。 */
  useBookStyles: boolean;
  /** 栏数。 */
  columnCount: 0 | 1 | 2;
  /** 自动分栏阈值 px。 */
  columnThresholdPx: number;
  /** 键盘快捷键翻页总开关。 */
  keyboardShortcutTurnPage: boolean;
  /** 简繁转换。 */
  chineseVariant: ChineseVariant;
  /** 划选自动翻译(浮窗预填翻译动作)。 */
  autoTranslateSelection: boolean;
  /** 单击查词：单击英文词命中本地分级词库即弹词典卡。 */
  clickDictionary: boolean;
  /** 划选自动标记生词。 */
  autoMarkSelection: boolean;
}

const WORD_LEVELS: readonly ReaderWordLevel[] = [
  '中考', '高考', '四级', '六级', '考研', '雅思', '托福', '专四', '专八', 'GRE',
];

/** v3:页眉页脚升级为三槽位 + 翻页分区 + 自定义主题/CSS;旧 v2 键迁移读取。 */
const STORAGE_KEY = 'musereader.reader.prefs.v3';
/* 旧键回落链：musetranslate 时代 v3 → 更早 v2/v1（升级用户偏好不丢）。 */
const LEGACY_V3_STORAGE_KEY = 'musetranslate.reader.prefs.v3';
const LEGACY_V2_STORAGE_KEY = 'musetranslate.reader.prefs.v2';
const LEGACY_STORAGE_KEY = 'musetranslate.reader.prefs.v1';

const DEFAULT_TYPE_EXTRA: ReaderTypeExtra = {
  letterSpacingPx: 0,
  wordSpacingPx: 0,
  fontWeight: 400,
  indentEm: 0,
  headingScale: 1.0,
  textAlignment: 'auto',
};

const DEFAULT_PREFS: ReaderPrefs = {
  lang: 'both',
  flow: 'paginated',
  pageWidth: 'standard',
  showParaDot: true,
  translationBlur: false,
  fontSizeEm: 1.15,
  lineHeight: 1.8,
  paragraphSpacingEm: 1.2,
  typeExtra: DEFAULT_TYPE_EXTRA,
  fontFamily: 'auto',
  customFontPath: '',
  customFontLabel: '',
  enFontFamily: 'auto',
  customEnFontPath: '',
  customEnFontLabel: '',
  themeName: 'paper',
  autoTheme: false,
  customThemeBg: '#f6f2ea',
  customThemeFg: '#23211d',
  themeBgImagePath: '',
  themeBgImageOpacity: 0.45,
  pageTurnStyle: 'slide',
  bookLook: true,
  pageTurnZones: 'standard',
  pageTurnCustomZones: [...ZONE_PRESETS.standard],
  sideMarginPercent: 6,
  topMarginPx: 0,
  bottomMarginPx: 0,
  customCss: '',
  customCssEnabled: false,
  headerSlots: { left: 'none', center: 'chapter', right: 'none' },
  footerSlots: { left: 'none', center: 'chapterProgress', right: 'time' },
  hfFontSizePx: 12,
  wordWise: true,
  userLevel: '四级',
  wordWiseStyle: 'underline',
  wordWiseGloss: 'zh',
  wordWiseSize: 0.62,
  wordWiseLineBoost: 1.12,
  wordWiseGapPx: 14,
  wordWiseColor: '#b45309',
  settingsOpen: false,
  volumeKeyFlip: false,
  autoFlip: false,
  autoFlipInterval: 30,
  keepScreenOn: true,
  hideStatusBar: false,
  brightnessPercent: 100,
  paragraphGapPx: 0,
  useBookStyles: false,
  columnCount: 1,
  columnThresholdPx: 900,
  keyboardShortcutTurnPage: true,
  chineseVariant: 'none',
  autoTranslateSelection: false,
  clickDictionary: true,
  autoMarkSelection: false,
};

/** 段距密度 → 段间距基线（em）：紧凑/标准/宽松。 */
function densitySpacing(density: GlobalReadingDefaults['paragraphDensity']): number {
  if (density === 'compact') return 0.3;
  if (density === 'relaxed') return 1.2;
  return 0.7;
}

/** 全局阅读默认（设置页「阅读」面板，Y3 接线）：首次加载（本地无偏好）时
    作为基线，用户一旦在阅读器内改过即以本地 prefs 为准。 */
export interface GlobalReadingDefaults {
  bilingualDefault: boolean;
  wordwiseDefault: boolean;
  paragraphDensity: 'compact' | 'standard' | 'relaxed';
}

/**
 * 共享“用户词汇难度档”真源：设置→词库的下拉与阅读器/WordWise/背诵的提词档
 * 是同一个值（存于 reader prefs 存储的 userLevel 字段）。读-改-写只动 userLevel，
 * 不影响其余偏好；与 loadReaderPrefs 交叉——下次加载阅读器即生效（整本种子重扫）。
 */
export function loadUserWordLevel(): ReaderWordLevel {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY) ?? window.localStorage.getItem(LEGACY_V3_STORAGE_KEY);
    if (!raw) return DEFAULT_PREFS.userLevel;
    const parsed = JSON.parse(raw) as Partial<ReaderPrefs>;
    return parsed.userLevel && WORD_LEVELS.includes(parsed.userLevel)
      ? parsed.userLevel
      : DEFAULT_PREFS.userLevel;
  } catch {
    return DEFAULT_PREFS.userLevel;
  }
}

export function saveUserWordLevel(level: ReaderWordLevel): void {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    const base: Record<string, unknown> = raw ? (JSON.parse(raw) as Record<string, unknown>) : {};
    base.userLevel = level;
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(base));
  } catch {
    // 存不下仅影响下次启动恢复；当前会话值由调用方 state 保持。
  }
}

export function loadReaderPrefs(globalDefaults?: GlobalReadingDefaults): ReaderPrefs {
  try {
    const currentRaw = window.localStorage.getItem(STORAGE_KEY);
    const legacyRaw = currentRaw
      ? null
      : window.localStorage.getItem(LEGACY_V3_STORAGE_KEY)
        ?? window.localStorage.getItem(LEGACY_V2_STORAGE_KEY)
        ?? window.localStorage.getItem(LEGACY_STORAGE_KEY);
    const raw = currentRaw ?? legacyRaw;
    if (!raw) {
      return {
        ...DEFAULT_PREFS,
        lang: globalDefaults?.bilingualDefault ? 'both' : 'en',
        wordWise: globalDefaults?.wordwiseDefault ?? DEFAULT_PREFS.wordWise,
        paragraphSpacingEm: densitySpacing(globalDefaults?.paragraphDensity ?? 'standard'),
      };
    }
    const parsed = JSON.parse(raw) as Partial<ReaderPrefs> & { typeExtra?: Partial<ReaderTypeExtra> };
    const prefs: ReaderPrefs = {
      lang: parsed.lang === 'zh' || parsed.lang === 'en' ? parsed.lang : 'both',
      flow: legacyRaw
        ? 'paginated'
        : parsed.flow === 'scrolled' || parsed.flow === 'spread'
          ? parsed.flow
          : 'paginated',
      showParaDot: typeof parsed.showParaDot === 'boolean' ? parsed.showParaDot : true,
      pageWidth: ['narrow', 'standard', 'wide'].includes(parsed.pageWidth ?? '')
        ? (parsed.pageWidth as ReaderPrefs['pageWidth'])
        : 'standard',
      translationBlur: parsed.translationBlur === true,
      fontSizeEm: clamp(parsed.fontSizeEm, 0.5, 3.0, DEFAULT_PREFS.fontSizeEm),
      lineHeight: clamp(parsed.lineHeight, 1.0, 3.0, DEFAULT_PREFS.lineHeight),
      paragraphSpacingEm: clamp(parsed.paragraphSpacingEm, 0, 5, DEFAULT_PREFS.paragraphSpacingEm),
      typeExtra: loadTypeExtra(parsed.typeExtra),
      fontFamily: ['serif', 'sans', 'custom'].includes(parsed.fontFamily ?? '')
        ? (parsed.fontFamily as ReaderFontFamily)
        : 'auto',
      customFontPath: typeof parsed.customFontPath === 'string' ? parsed.customFontPath.slice(0, 1024) : '',
      customFontLabel: typeof parsed.customFontLabel === 'string' ? parsed.customFontLabel.slice(0, 128) : '',
      enFontFamily: ['auto', 'serif', 'sans', 'custom'].includes(parsed.enFontFamily ?? '')
        ? (parsed.enFontFamily as ReaderFontFamily)
        : 'auto',
      customEnFontPath: typeof parsed.customEnFontPath === 'string' ? parsed.customEnFontPath.slice(0, 512) : '',
      customEnFontLabel: typeof parsed.customEnFontLabel === 'string' ? parsed.customEnFontLabel.slice(0, 128) : '',
      themeName: THEME_NAMES.includes(parsed.themeName as ReaderThemeName)
        ? (parsed.themeName as ReaderThemeName)
        : 'paper',
      autoTheme: parsed.autoTheme === true,
      customThemeBg: /^#[0-9a-fA-F]{6}$/.test(parsed.customThemeBg ?? '')
        ? (parsed.customThemeBg as string)
        : DEFAULT_PREFS.customThemeBg,
      customThemeFg: /^#[0-9a-fA-F]{6}$/.test(parsed.customThemeFg ?? '')
        ? (parsed.customThemeFg as string)
        : DEFAULT_PREFS.customThemeFg,
      themeBgImagePath: typeof parsed.themeBgImagePath === 'string' ? parsed.themeBgImagePath.slice(0, 1024) : '',
      themeBgImageOpacity: clamp(parsed.themeBgImageOpacity, 0.1, 1, DEFAULT_PREFS.themeBgImageOpacity),
      pageTurnZones: ['standard', 'swapped', 'centerTurn', 'topMenu', 'leftHand', 'custom'].includes(parsed.pageTurnZones ?? '')
        ? (parsed.pageTurnZones as ReaderPageTurnZones)
        : 'standard',
      pageTurnCustomZones: (() => {
        const valid: ReaderZoneAction[] = ['none', 'prev', 'next', 'menu'];
        const raw = (parsed as { pageTurnCustomZones?: unknown }).pageTurnCustomZones;
        if (Array.isArray(raw) && raw.length === 9 && raw.every((v) => valid.includes(v as ReaderZoneAction))) {
          return raw as ReaderZoneAction[];
        }
        return [...ZONE_PRESETS.standard];
      })(),
      sideMarginPercent: clamp(parsed.sideMarginPercent, 0, 20, DEFAULT_PREFS.sideMarginPercent),
      topMarginPx: clamp(parsed.topMarginPx, 0, 200, DEFAULT_PREFS.topMarginPx),
      bottomMarginPx: clamp(parsed.bottomMarginPx, 0, 200, DEFAULT_PREFS.bottomMarginPx),
      customCss: typeof parsed.customCss === 'string' ? parsed.customCss.slice(0, 8192) : '',
      customCssEnabled: parsed.customCssEnabled === true,
      headerSlots: loadLegacyHeader(parsed),
      footerSlots: loadLegacyFooter(parsed),
      hfFontSizePx: clamp(parsed.hfFontSizePx, 8, 24, DEFAULT_PREFS.hfFontSizePx),
      pageTurnStyle: ['slide', 'fade', 'none', 'flip', 'book'].includes(parsed.pageTurnStyle ?? '')
        ? (parsed.pageTurnStyle as ReaderPageTurnStyle)
        : 'slide',
      bookLook: parsed.bookLook !== false,
      wordWise: parsed.wordWise !== false,
      userLevel: WORD_LEVELS.includes(parsed.userLevel as ReaderWordLevel)
        ? (parsed.userLevel as ReaderWordLevel)
        : DEFAULT_PREFS.userLevel,
      wordWiseStyle: ['underline', 'highlight', 'plain'].includes(parsed.wordWiseStyle ?? '')
        ? (parsed.wordWiseStyle as WordWiseStyle)
        : DEFAULT_PREFS.wordWiseStyle,
      wordWiseGloss: ['zh', 'en', 'phonetic'].includes(parsed.wordWiseGloss ?? '')
        ? (parsed.wordWiseGloss as WordWiseGloss)
        : DEFAULT_PREFS.wordWiseGloss,
      wordWiseSize: clamp(parsed.wordWiseSize, 0.5, 0.85, DEFAULT_PREFS.wordWiseSize),
      wordWiseLineBoost: clamp(parsed.wordWiseLineBoost, 1, 1.6, DEFAULT_PREFS.wordWiseLineBoost),
      wordWiseGapPx: clamp(parsed.wordWiseGapPx, 0, 48, DEFAULT_PREFS.wordWiseGapPx),
      wordWiseColor: /^#[0-9a-fA-F]{6}$/.test(parsed.wordWiseColor ?? '')
        ? String(parsed.wordWiseColor)
        : DEFAULT_PREFS.wordWiseColor,
      settingsOpen: false,
      volumeKeyFlip: parsed.volumeKeyFlip === true,
      autoFlip: parsed.autoFlip === true,
      autoFlipInterval: clamp(parsed.autoFlipInterval, 10, 100, DEFAULT_PREFS.autoFlipInterval),
      keepScreenOn: parsed.keepScreenOn !== false,
      hideStatusBar: parsed.hideStatusBar === true,
      brightnessPercent: clamp(parsed.brightnessPercent, 0, 100, DEFAULT_PREFS.brightnessPercent),
      paragraphGapPx: clamp(parsed.paragraphGapPx, 0, 60, DEFAULT_PREFS.paragraphGapPx),
      useBookStyles: parsed.useBookStyles === true,
      columnCount: [0, 1, 2].includes(parsed.columnCount as number)
        ? (parsed.columnCount as 0 | 1 | 2)
        : 1,
      columnThresholdPx: clamp(parsed.columnThresholdPx, 400, 1200, DEFAULT_PREFS.columnThresholdPx),
      keyboardShortcutTurnPage: parsed.keyboardShortcutTurnPage !== false,
      chineseVariant: ['none', 's2t', 't2s'].includes(parsed.chineseVariant ?? '')
        ? (parsed.chineseVariant as ChineseVariant)
        : 'none',
      autoTranslateSelection: parsed.autoTranslateSelection === true,
      clickDictionary: parsed.clickDictionary !== false,
      autoMarkSelection: parsed.autoMarkSelection === true,
    };
    if (legacyRaw) {
      const { settingsOpen: _omit, ...persist } = prefs;
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(persist));
    }
    return prefs;
  } catch {
    return DEFAULT_PREFS;
  }
}

const SLOT_VALUES = ['none', 'chapter', 'chapterProgress', 'bookProgress', 'time'] as const;

function loadSlots(raw: unknown, fallback: ReaderHeaderFooter): ReaderHeaderFooter {
  const obj = (raw ?? {}) as Partial<ReaderHeaderFooter>;
  const pick = (v: unknown, fb: ReaderHeaderFooter['left']) =>
    SLOT_VALUES.includes(v as typeof SLOT_VALUES[number]) ? (v as ReaderHeaderFooter['left']) : fb;
  return {
    left: pick(obj.left, fallback.left),
    center: pick(obj.center, fallback.center),
    right: pick(obj.right, fallback.right),
  };
}

/** v2 旧字段 pageHeader 迁移到三槽位(chapter→center=chapter)。 */
function loadLegacyHeader(parsed: Partial<ReaderPrefs> & { pageHeader?: string }): ReaderHeaderFooter {
  const slots = loadSlots((parsed as { headerSlots?: unknown }).headerSlots, DEFAULT_PREFS.headerSlots);
  if ((parsed as { headerSlots?: unknown }).headerSlots) return slots;
  return parsed.pageHeader === 'none'
    ? { left: 'none', center: 'none', right: 'none' }
    : { left: 'none', center: 'chapter', right: 'none' };
}

function loadLegacyFooter(parsed: Partial<ReaderPrefs> & { pageFooter?: string }): ReaderHeaderFooter {
  const slots = loadSlots((parsed as { footerSlots?: unknown }).footerSlots, DEFAULT_PREFS.footerSlots);
  if ((parsed as { footerSlots?: unknown }).footerSlots) return slots;
  if (parsed.pageFooter === 'none') return { left: 'none', center: 'none', right: 'none' };
  if (parsed.pageFooter === 'time') return { left: 'none', center: 'none', right: 'time' };
  if (parsed.pageFooter === 'both') return { left: 'none', center: 'chapterProgress', right: 'time' };
  return { left: 'none', center: 'chapterProgress', right: 'none' };
}

function loadTypeExtra(raw?: Partial<ReaderTypeExtra>): ReaderTypeExtra {
  return {
    letterSpacingPx: clamp(raw?.letterSpacingPx, -3, 7, DEFAULT_TYPE_EXTRA.letterSpacingPx),
    wordSpacingPx: clamp(raw?.wordSpacingPx, 0, 7, DEFAULT_TYPE_EXTRA.wordSpacingPx),
    fontWeight: (() => {
      const w = Math.round(Number(raw?.fontWeight) / 100) * 100;
      return w >= 100 && w <= 900 ? w : DEFAULT_TYPE_EXTRA.fontWeight;
    })(),
    indentEm: clamp(raw?.indentEm, -0.5, 8, DEFAULT_TYPE_EXTRA.indentEm),
    headingScale: clamp(raw?.headingScale, 0.5, 2.0, DEFAULT_TYPE_EXTRA.headingScale),
    textAlignment: ['left', 'center', 'right', 'justify'].includes(raw?.textAlignment ?? '')
      ? (raw?.textAlignment as ReaderTextAlign)
      : 'auto',
  };
}

export function useReaderPrefs(globalDefaults?: GlobalReadingDefaults) {
  const [prefs, setPrefs] = useState<ReaderPrefs>(loadReaderPrefs);

  const update = useCallback((patch: Partial<ReaderPrefs>) => {
    setPrefs((current) => {
      let next = { ...current, ...patch };
      if (patch.themeCycle) {
        next.themeName = THEME_NAMES[(THEME_NAMES.indexOf(current.themeName) + 1) % THEME_NAMES.length];
        next.themeCycle = false;
        /* 手动循环选主题 = 用户接管：解除跟随系统的暗色覆盖，
           否则 autoTheme 恒定把 readerTheme 钉在 dark，循环看起来"无效"。 */
        if (current.autoTheme) next.autoTheme = false;
      }
      try {
        const { settingsOpen: _omit, themeCycle: _omit2, ...persist } = next;
        window.localStorage.setItem(STORAGE_KEY, JSON.stringify(persist));
      } catch {
        // 隐私模式存储失败不影响阅读
      }
      return next;
    });
  }, []);

  // 跨标签页同步
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_KEY && event.newValue) {
        try {
          setPrefs({ ...loadReaderPrefs(globalDefaults), settingsOpen: false });
        } catch {
          /* ignore */
        }
      }
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  return { prefs, update };
}

function clamp(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.min(max, Math.max(min, value))
    : fallback;
}

const LANG_PER_BOOK_KEY = 'musereader.reader.langPerBook.v1';
const LEGACY_LANG_PER_BOOK_KEY = 'musetranslate.reader.langPerBook.v1';

/** 读取该书上次使用的语言模式。 */
export function loadLangForBook(bookId: string): ReaderLang | null {
  try {
    const raw =
      window.localStorage.getItem(LANG_PER_BOOK_KEY) ??
      window.localStorage.getItem(LEGACY_LANG_PER_BOOK_KEY);
    if (!raw) return null;
    const map = JSON.parse(raw) as Record<string, ReaderLang>;
    const lang = map[bookId];
    return lang === 'zh' || lang === 'en' || lang === 'both' ? lang : null;
  } catch {
    return null;
  }
}

/** 记忆该书语言模式(上限 64 条,超出裁剪最旧)。 */
export function saveLangForBook(bookId: string, lang: ReaderLang): void {
  try {
    const raw = window.localStorage.getItem(LANG_PER_BOOK_KEY);
    const map: Record<string, ReaderLang> = raw ? JSON.parse(raw) : {};
    map[bookId] = lang;
    const keys = Object.keys(map);
    if (keys.length > 64) delete map[keys[0]];
    window.localStorage.setItem(LANG_PER_BOOK_KEY, JSON.stringify(map));
  } catch {
    // 隐私模式存储失败不影响阅读
  }
}
