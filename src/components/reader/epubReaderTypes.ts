import type { RefObject } from 'react';

export type ReaderLang = 'zh' | 'en' | 'both';
export type ReaderFlow = 'paginated' | 'spread' | 'scrolled';
/** 行宽档位：单页模式的内容最大宽度（双页展开栏宽由视口决定，忽略此项）。 */
export type ReaderPageWidth = 'narrow' | 'standard' | 'wide';
/** 翻页动画：滑动（原生平滑滚动）/ 淡入淡出 / 无 / 翻书（3D 翻面）。 */
export type ReaderPageTurnStyle = 'slide' | 'fade' | 'none' | 'flip' | 'book';
/** 正文对齐：auto 保持原书样式（实际渲染为两端对齐）。 */
export type ReaderTextAlign = 'auto' | 'left' | 'center' | 'right' | 'justify';
/** 正文字体偏好：auto 保持原书字体；custom 为用户导入字体（customFontPath）。 */
export type ReaderFontFamily = 'auto' | 'serif' | 'sans' | 'custom';
/** 页眉页脚槽位内容。 */
export type ReaderInfoSlot = 'none' | 'chapter' | 'chapterProgress' | 'bookProgress' | 'time';
/** 页眉/页脚三槽位配置。 */
export interface ReaderHeaderFooter {
  left: ReaderInfoSlot;
  center: ReaderInfoSlot;
  right: ReaderInfoSlot;
}
/** 触控翻页分区布局：
    standard=左右翻页中央菜单；swapped=左右交换（右翻左习惯）；
    centerTurn=中央左右半翻页、两侧唤出菜单；topMenu=顶部菜单其余翻页；
    leftHand=单手连读（大部分下一页）；custom=用户 3×3 逐格自定义。 */
export type ReaderPageTurnZones = 'standard' | 'swapped' | 'centerTurn' | 'topMenu' | 'leftHand' | 'custom';

/** 细粒度排版项：由阅读器注入 CSS 应用到正文文档。 */
export interface ReaderTypeExtra {
  /** 字符间距 px（-3–7，负值收紧）。 */
  letterSpacingPx: number;
  /** 词间距 px（0–7）。 */
  wordSpacingPx: number;
  /** 字重（100–900，步进 100）。 */
  fontWeight: number;
  /** 段首缩进 em（-0.5–8；负值=跟随书籍样式，0=强制无缩进，>0=指定缩进）。 */
  indentEm: number;
  /** 标题相对缩放（0.5–2.0，zoom 应用于 h1–h6）。 */
  headingScale: number;
  /** 正文对齐。 */
  textAlignment: ReaderTextAlign;
}

export interface ReaderLocation {
  href: string;
  offset: number;
}

export interface ReaderTheme {
  bg: string;
  fg: string;
  muted: string;
  enFg: string;
}

export interface ReaderPageInfo {
  page: number;
  total: number;
}

export interface ReaderFrameTocItem {
  title: string;
  href: string;
  index: number;
  /** 归并层级：0=书籍目录条目；>0=前一目录条目下的连续子文件（缩进展示）。 */
  depth: number;
}

export interface ReaderFrameApi {
  loading: boolean;
  error: string | null;
  chapterTitle: string;
  sectionIndex: number;
  sectionKey: string;
  sectionSrcDoc: string;
  pageInfo: ReaderPageInfo;
  progress: number;
  location: ReaderLocation | null;
  toc: ReaderFrameTocItem[];
  docSnapshot: Document | null;
  iframeRef: RefObject<HTMLIFrameElement | null>;
  containerRef: RefObject<HTMLDivElement | null>;
  onIframeLoad: () => void;
  getDoc: () => Document | null;
  nextPage: () => void;
  prevPage: () => void;
  goToProgress: (ratio: number) => void;
  goTo: (href: string) => void;
  /** EPUB 渲染层专有：带页内偏移的锚点跳转（阅读位置精确恢复用）。 */
  goToAnchor?: (href: string, offset?: number) => void;
}
