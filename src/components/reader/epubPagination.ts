import type { ReaderFlow, ReaderPageWidth } from './epubReaderTypes';

export interface ReaderViewport {
  width: number;
  height: number;
}

export interface ReaderAnchorPoint {
  key: string;
  page: number;
}

const FLOW_STYLE_ID = 'mt-reader-flow';
const TRAILING_SPACER_ATTRIBUTE = 'data-mt-reader-trailing-space';

export interface ReaderFlowOptions {
  /** 每视口栏数：1=单页，2=双页展开（物理书隐喻）。窄视口自动回落 1。 */
  columns?: 1 | 2;
  /** 单栏内容最大宽度（行宽档位映射），默认 1120。 */
  maxContentWidth?: number;
  /** 页边距；缺省用内建启发式。 */
  sideMarginPercent?: number;
}

export function applyReaderFlow(
  doc: Document,
  flow: ReaderFlow,
  viewport: ReaderViewport,
  options: ReaderFlowOptions = {},
): void {
  const style = ensureFlowStyle(doc);
  removeTrailingSpacer(doc);
  if (flow === 'scrolled') {
    style.textContent = buildScrolledCss(viewport.width);
    return;
  }

  const layout = buildPaginatedLayout(viewport, options);
  style.textContent = layout.css;
  const contentEnd = Math.max(doc.documentElement.scrollWidth, doc.body?.scrollWidth ?? 0);
  appendTrailingSpacer(doc, contentEnd, layout.trailingSpace);
}

/** 横向分页（单页/双页）与垂直滚动的判定边界。 */
export function isPagedFlow(flow: ReaderFlow): boolean {
  return flow !== 'scrolled';
}

/** 行宽档位 → 单页模式内容最大宽度（px）。默认档 1120 取代旧版 760 过窄上限。 */
export function readerPageWidthPx(pageWidth: ReaderPageWidth): number {
  return pageWidth === 'narrow' ? 840 : pageWidth === 'wide' ? 1400 : 1120;
}

export function readerPageExtent(flow: ReaderFlow, viewport: ReaderViewport): number {
  return isPagedFlow(flow) ? viewport.width : viewport.height;
}

export function readerPageCount(
  doc: Document,
  flow: ReaderFlow,
  viewport: ReaderViewport,
): number {
  const extent = readerPageExtent(flow, viewport);
  if (extent <= 0) return 1;
  const body = doc.body;
  const root = doc.documentElement;
  const contentExtent = isPagedFlow(flow)
    ? Math.max(root.scrollWidth, body?.scrollWidth ?? 0)
    : Math.max(root.scrollHeight, body?.scrollHeight ?? 0);
  return Math.max(1, Math.ceil(contentExtent / extent));
}

export function readerScrollOffset(win: Window, flow: ReaderFlow): number {
  return isPagedFlow(flow) ? win.scrollX : win.scrollY;
}

export function scrollReaderToPage(
  win: Window,
  flow: ReaderFlow,
  extent: number,
  page: number,
  behavior: ScrollBehavior,
): void {
  const offset = Math.max(0, page) * Math.max(0, extent);
  win.scrollTo({
    left: isPagedFlow(flow) ? offset : 0,
    top: flow === 'scrolled' ? offset : 0,
    behavior,
  });
}

/** rAF 自绘翻页滚动（anx paginator 模式）：
    duration = clamp(200, 300, 250 × 距离/页宽)ms，easeOutSine 缓动。
    取代浏览器 smooth（时长/缓动不可控、不同内核手感不一）。 */
export function animateReaderToPage(
  win: Window,
  flow: ReaderFlow,
  extent: number,
  page: number,
  onSettled?: () => void,
): void {
  const target = Math.max(0, page) * Math.max(0, extent);
  const startX = win.scrollX;
  const startY = win.scrollY;
  const paged = isPagedFlow(flow);
  const current = paged ? startX : startY;
  const distance = Math.abs(target - current);
  if (distance <= 1 || extent <= 0) {
    scrollReaderToPage(win, flow, extent, page, 'auto');
    onSettled?.();
    return;
  }
  const duration = Math.max(200, Math.min(300, (250 * distance) / extent));
  /* rAF 与时间戳必须取宿主 realm：iframe 带 sandbox（无 allow-scripts）时其
     事件循环里的回调永不执行，win.requestAnimationFrame 注册即石沉大海
     （slide 翻页完全失效即此原因）；win.scrollTo 本身仍可跨 realm 调用。 */
  const startedAt = window.performance.now();
  const raf = (now: number) => {
    const ratio = Math.min(1, (now - startedAt) / duration);
    const eased = Math.sin((ratio * Math.PI) / 2); // easeOutSine
    const value = current + (target - current) * eased;
    if (paged) win.scrollTo(value, startY);
    else win.scrollTo(startX, value);
    if (ratio < 1) window.requestAnimationFrame(raf);
    else onSettled?.();
  };
  window.requestAnimationFrame(raf);
}

export function collectReaderAnchors(
  body: HTMLElement,
  win: Window,
  flow: ReaderFlow,
  extent: number,
): ReaderAnchorPoint[] {
  if (extent <= 0) return [];
  const points: ReaderAnchorPoint[] = [];
  const seen = new Set<string>();

  const push = (element: Element, key: string | null) => {
    const normalizedKey = key?.trim();
    if (!normalizedKey || seen.has(normalizedKey)) return;
    seen.add(normalizedKey);
    const rect = element.getBoundingClientRect();
    const documentOffset =
      isPagedFlow(flow) ? rect.left + win.scrollX : rect.top + win.scrollY;
    points.push({
      key: normalizedKey,
      page: Math.max(0, Math.floor((documentOffset + 1) / extent)),
    });
  };

  body.querySelectorAll('[data-mt-idx]').forEach((element) => {
    push(element, element.getAttribute('data-mt-idx'));
  });
  body.querySelectorAll('[id]').forEach((element) => push(element, element.id));
  points.sort((left, right) => left.page - right.page);
  return points;
}

export function readerElementPage(
  element: Element,
  win: Window,
  flow: ReaderFlow,
  extent: number,
): number {
  if (extent <= 0) return 0;
  const rect = element.getBoundingClientRect();
  const documentOffset =
    isPagedFlow(flow) ? rect.left + win.scrollX : rect.top + win.scrollY;
  return Math.max(0, Math.floor((documentOffset + 1) / extent));
}

function ensureFlowStyle(doc: Document): HTMLStyleElement {
  const existing = doc.getElementById(FLOW_STYLE_ID);
  if (existing instanceof HTMLStyleElement) return existing;
  const style = doc.createElement('style');
  style.id = FLOW_STYLE_ID;
  doc.head.appendChild(style);
  return style;
}

function buildPaginatedLayout(
  viewport: ReaderViewport,
  options: ReaderFlowOptions = {},
): {
  css: string;
  trailingSpace: number;
} {
  const width = Math.max(1, Math.trunc(viewport.width));
  const height = Math.max(1, Math.trunc(viewport.height));
  // 双页展开：满足 2*(colW+G) == W 且 sidePad == G/2 时栏与页边界精确对齐；
  // 视口过窄（<1000px）时双栏各栏过瘦，回落单页。
  const wantsSpread = options.columns === 2 && width >= 1000;
  const maxContentWidth = options.maxContentWidth ?? 1120;
  let columnGap: number;
  let contentWidth: number;
  let inlinePadding: number;
  if (wantsSpread) {
    columnGap = Math.max(48, Math.round(width * 0.045));
    /* 半页宽保留小数精度（不做整数截断）：截断会让每个跨页窄 1px，
       漂移随页数累积导致栏目与页面边界错位（下一页内容侵入上一页、
       上标落入中缝）；小数栏宽在 CSS px 空间内逐页精确对齐。 */
    contentWidth = Math.max(1, width / 2 - columnGap);
    inlinePadding = columnGap / 2;
  } else {
    // 单页：页边距 = max(最小保护值, 用户设置百分比)）
    const minimumInlinePadding = width < 520 ? 20 : 32;
    const desiredInline = Math.max(
      minimumInlinePadding,
      Math.round(width * ((options.sideMarginPercent ?? 6) / 100)),
    );
    contentWidth = Math.max(1, Math.min(width - desiredInline * 2, maxContentWidth));
    columnGap = Math.max(desiredInline * 2, width - contentWidth);
    inlinePadding = columnGap / 2;
  }
  const blockPadding = Math.max(18, Math.min(46, Math.round(height * 0.055)));
  const contentHeight = Math.max(1, height - blockPadding * 2);

  return {
    trailingSpace: inlinePadding,
    css: `
html{
  width:${width}px!important;
  height:${height}px!important;
  margin:0!important;
  padding:0!important;
  overflow:hidden!important;
  scroll-behavior:auto!important;
}
body{
  display:block!important;
  box-sizing:border-box!important;
  width:${width}px!important;
  height:${height}px!important;
  min-height:0!important;
  max-width:none!important;
  max-height:none!important;
  margin:0!important;
  padding:${blockPadding}px ${inlinePadding}px!important;
  overflow:visible!important;
  column-width:${contentWidth}px!important;
  column-gap:${columnGap}px!important;
  column-fill:auto!important;
  overflow-wrap:break-word!important;
}
body > *{ max-width:100%; }
body img,body svg,body video{
  max-height:${contentHeight}px!important;
  object-fit:contain!important;
}
`.trim(),
  };
}

function buildScrolledCss(viewportWidth: number): string {
  const inlinePadding = Math.max(20, Math.min(64, Math.round(viewportWidth * 0.06)));
  return `
html{
  width:100%!important;
  min-height:100%!important;
  margin:0!important;
  padding:0!important;
  overflow-x:hidden!important;
  overflow-y:auto!important;
}
body{
  display:block!important;
  box-sizing:border-box!important;
  width:min(100%, 56rem)!important;
  height:auto!important;
  min-height:100%!important;
  max-height:none!important;
  margin:0 auto!important;
  padding:2.5rem ${inlinePadding}px 5rem!important;
  overflow:visible!important;
  column-width:auto!important;
  column-gap:normal!important;
  column-fill:balance!important;
  overflow-wrap:break-word!important;
}
body > *{ max-width:100%; }
`.trim();
}

function appendTrailingSpacer(doc: Document, left: number, width: number): void {
  if (width <= 0) return;
  const spacer = doc.createElement('span');
  spacer.setAttribute(TRAILING_SPACER_ATTRIBUTE, '');
  spacer.setAttribute('aria-hidden', 'true');
  spacer.style.cssText = [
    'position:absolute',
    `left:${Math.max(0, left)}px`,
    'top:0',
    `width:${width}px`,
    'height:1px',
    'pointer-events:none',
    'visibility:hidden',
  ].join(';');
  doc.documentElement.appendChild(spacer);
}

function removeTrailingSpacer(doc: Document): void {
  doc.documentElement
    .querySelectorAll(`[${TRAILING_SPACER_ATTRIBUTE}]`)
    .forEach((element) => element.remove());
}
