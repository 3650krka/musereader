import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  applyReaderFlow,
  collectReaderAnchors,
  isPagedFlow,
  readerElementPage,
  readerPageCount,
  readerPageExtent,
  readerPageWidthPx,
  readerScrollOffset,
  animateReaderToPage,
  scrollReaderToPage,
  type ReaderViewport,
} from './epubPagination';
import type {
  ReaderFlow,
  ReaderFontFamily,
  ReaderLang,
  ReaderLocation,
  ReaderPageInfo,
  ReaderPageTurnStyle,
  ReaderPageWidth,
  ReaderTheme,
  ReaderTypeExtra,
} from './epubReaderTypes';
import { useEpubPackage } from './useEpubPackage';
import { useEpubPresentation } from './useEpubPresentation';
import { useEpubSectionDocument } from './useEpubSectionDocument';

export type { ReaderFlow, ReaderLang, ReaderLocation, ReaderTheme } from './epubReaderTypes';

type PendingNavigation =
  | { kind: 'start' }
  | { kind: 'end' }
  | { kind: 'fraction'; fraction: number }
  | { kind: 'location'; href: string; offset: number };

function splitReaderHref(href: string): { path: string; anchor: string } {
  const hashIndex = href.indexOf('#');
  if (hashIndex < 0) return { path: href, anchor: '' };
  return {
    path: href.slice(0, hashIndex),
    anchor: safeDecode(href.slice(hashIndex + 1)),
  };
}

function normalizeEpubPath(path: string): string {
  return safeDecode(path).replace(/\\/g, '/').replace(/^\.\//, '');
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch (error) {
    console.warn('EPUB href decoding failed:', error);
    return value;
  }
}

function sameLocation(left: ReaderLocation | null, right: ReaderLocation): boolean {
  return left?.href === right.href && left.offset === right.offset;
}

export function useEpubReader(
  bookId: string,
  enabled: boolean,
  extras?: {
    /** 章节渲染后的行内标注钩子（WordWise 生词标注）：在 applyFlow 分页前执行并等待。 */
    annotate?: (doc: Document) => Promise<void>;
    /** 分页/排版应用后的测量钩子（WordWise 上标碰撞自愈）：applyFlow 后下一帧执行。
        返回 true 表示修补改变了流布局（块内边距），调用方应重分页一次再终测。 */
    postFlow?: (doc: Document) => boolean;
  },
) {
  const {
    pkg,
    loading: packageLoading,
    error: packageError,
    sections,
    toc,
    sectionIndex,
    setSectionIndex,
    currentSection,
    chapterTitle,
  } = useEpubPackage(bookId, enabled);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const [pageInfo, setPageInfo] = useState<ReaderPageInfo>({ page: 0, total: 1 });
  const [location, setLocation] = useState<ReaderLocation | null>(null);
  const [docSnapshot, setDocSnapshot] = useState<Document | null>(null);

  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const readyRef = useRef(false);
  const locationRef = useRef<ReaderLocation | null>(null);
  const pendingNavigationRef = useRef<PendingNavigation | null>(null);
  const targetPageRef = useRef<number | null>(null);
  const relocationTimerRef = useRef<number | null>(null);
  const documentCleanupRef = useRef<(() => void) | null>(null);
  /* 标注/自愈钩子经 ref 消费：它们携带词表闭包，每次生词变化都会换身份。
     若把它们放进 applyFlow/reflowPreservingLocation 的依赖数组，会引发
     「回调换身份 → 重排 → 自动播种 → 生词本状态再变 → 回调再换身份」的
     连环重排；走 ref 后回调身份变化不再触碰这两个函数，生词即时重标
     改由 ReaderView 侦听词表内容变化后显式调用（reannotate），
     且重标本身因幂等播种不会再造成状态变化——链条在此收敛。 */
  const annotateRef = useRef(extras?.annotate);
  annotateRef.current = extras?.annotate;
  const postFlowRef = useRef(extras?.postFlow);
  postFlowRef.current = extras?.postFlow;
  const [lang, setLangState] = useState<ReaderLang>('both');
  const [flow, setFlowState] = useState<ReaderFlow>('paginated');
  const [pageWidth, setPageWidthState] = useState<ReaderPageWidth>('standard');
  const [translationBlur, setTranslationBlurState] = useState(false);
  const [fontSize, setFontSize] = useState(1);
  const [lineHeight, setLineHeight] = useState(1.9);
  const [paraSpacing, setParaSpacing] = useState(1.6);
  const [theme, setThemeState] = useState<ReaderTheme | null>(null);
  const [typeExtra, setTypeExtraState] = useState<ReaderTypeExtra | null>(null);
  const [fontFamily, setFontFamilyState] = useState<ReaderFontFamily>('auto');
  const [customFontPath, setCustomFontPathState] = useState('');
  /* 中英文字体分开（问题：双语书英文栏与中文栏可各选字体）。 */
  const [enFontFamily, setEnFontFamilyState] = useState<ReaderFontFamily>('auto');
  const [customEnFontPath, setCustomEnFontPathState] = useState('');
  const [pageTurnStyle, setPageTurnStyleState] = useState<ReaderPageTurnStyle>('slide');
  const [sideMarginPercent, setSideMarginPercentState] = useState(6);
  const [customCss, setCustomCssState] = useState('');
  const [brightnessPercent, setBrightnessPercentState] = useState(100);
  const [paragraphGapPx, setParagraphGapPxState] = useState(0);
  const [useBookStyles, setUseBookStylesState] = useState(false);
  const [columnCount, setColumnCountState] = useState<number>(1);
  const [columnThresholdPx, setColumnThresholdPxState] = useState(900);
  const [topMarginPx, setTopMarginPxState] = useState(0);
  const [bottomMarginPx, setBottomMarginPxState] = useState(0);

  const sectionDocument = useEpubSectionDocument(currentSection, enabled && Boolean(pkg));

  useEffect(() => {
    readyRef.current = false;
    setProgress(0);
    setPageInfo({ page: 0, total: 1 });
    setLocation(null);
    locationRef.current = null;
    pendingNavigationRef.current = null;
    targetPageRef.current = null;
    setRenderError(null);
  }, [bookId, enabled]);

  const getDoc = useCallback((): Document | null => {
    try {
      return iframeRef.current?.contentDocument ?? null;
    } catch (error) {
      console.error('EPUB document access failed:', error);
      return null;
    }
  }, []);

  const getViewport = useCallback((): ReaderViewport => ({
    width: iframeRef.current?.clientWidth ?? 0,
    height: iframeRef.current?.clientHeight ?? 0,
  }), []);

  const { applyDualLayout, applyStyles } = useEpubPresentation({
    getDoc,
    hasBilingualMarkup: Boolean(pkg?.hasBilingualMarkup),
    lang,
    fontSize,
    lineHeight,
    paraSpacing,
    theme,
    translationBlur,
    typeExtra: typeExtra ?? undefined,
    fontFamily,
    enFontFamily,
    customEnFontPath,
    customCss,
    brightnessPercent,
    paragraphGapPx,
    useBookStyles,
    columnCount,
    columnThresholdPx,
    topMarginPx,
    bottomMarginPx,
    customFontPath,
  });

  const applyFlow = useCallback(() => {
    const doc = getDoc();
    const viewport = getViewport();
    if (!doc?.body || viewport.width <= 0 || viewport.height <= 0) return;
    // 双页展开在过窄视口无意义（epubPagination 内部也会再兜底回落单页）
    const columns = flow === 'spread' ? 2 : 1;
    const paginate = () => applyReaderFlow(doc, flow, viewport, {
      columns,
      maxContentWidth: readerPageWidthPx(pageWidth),
      sideMarginPercent,
    });
    paginate();
    /* WordWise 碰撞自愈：分页列宽/断行定型后下一帧重测（幂等重入）。
       修补（块内边距/词距外边距）会改变流布局 → 重分页吸收再终测；
       收敛检测（修补量与上轮比对）保证迭代自然终止，上限 3 轮防病态链。
       注意：iframe 沙箱无 allow-scripts，其窗口帧回调永不执行
       （文档脚本被禁用）——必须走宿主 realm 帧调度，
       DOM 测量本身仍对 iframe 文档调用（跨 realm 访问不受限）。 */
    const settle = (round: number) => {
      window.requestAnimationFrame(() => {
        if (getDoc() !== doc) return;
        const needsReflow = postFlowRef.current?.(doc) ?? false;
        if (needsReflow && round < 3) {
          paginate();
          settle(round + 1);
        }
      });
    };
    settle(0);
  }, [flow, getDoc, getViewport, pageWidth, sideMarginPercent]);

  const getPageCount = useCallback(() => {
    const doc = getDoc();
    if (!doc) return 1;
    return readerPageCount(doc, flow, getViewport());
  }, [flow, getDoc, getViewport]);

  const getCurrentPage = useCallback(() => {
    const win = iframeRef.current?.contentWindow;
    const extent = readerPageExtent(flow, getViewport());
    if (!win || extent <= 0) return 0;
    return Math.max(0, Math.floor((readerScrollOffset(win, flow) + 1) / extent));
  }, [flow, getViewport]);

  const collectAnchors = useCallback(() => {
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    const extent = readerPageExtent(flow, getViewport());
    if (!doc?.body || !win) return [];
    return collectReaderAnchors(doc.body, win, flow, extent);
  }, [flow, getDoc, getViewport]);

  const readLocation = useCallback((): ReaderLocation | null => {
    if (!currentSection) return null;
    const currentPage = getCurrentPage();
    const anchors = collectAnchors();
    let currentAnchor: (typeof anchors)[number] | undefined;
    for (const anchor of anchors) {
      if (anchor.page > currentPage) break;
      currentAnchor = anchor;
    }
    if (!currentAnchor) return { href: currentSection.href, offset: currentPage };
    return {
      href: `${currentSection.href}#${encodeURIComponent(currentAnchor.key)}`,
      offset: Math.max(0, currentPage - currentAnchor.page),
    };
  }, [collectAnchors, currentSection, getCurrentPage]);

  /* 全书进度按章节正文字符数加权：封面/扉页/广告页权重≈0，
     避免「每章等权」导致小章节虚占进度、大章节被压缩。 */
  const sectionWeightSums = useMemo(() => {
    const sums: number[] = [0];
    for (const section of sections) {
      sums.push(sums[sums.length - 1] + Math.max(0, section.textLength || 0));
    }
    return sums;
  }, [sections]);
  const totalSectionWeight = sectionWeightSums[sections.length] ?? 0;

  const publishPage = useCallback((page: number, total: number) => {
    setPageInfo((current) =>
      current.page === page && current.total === total ? current : { page, total },
    );

    const sectionFraction =
      total <= 1 ? (sectionIndex === sections.length - 1 ? 1 : 0) : page / (total - 1);
    let fullProgress: number;
    if (sections.length === 0) {
      fullProgress = 0;
    } else if (totalSectionWeight > 0) {
      const sectionWeight = Math.max(0, sections[sectionIndex]?.textLength ?? 0);
      fullProgress = Math.min(
        1,
        (sectionWeightSums[sectionIndex] + sectionWeight * sectionFraction) / totalSectionWeight,
      );
    } else {
      fullProgress = Math.min(1, (sectionIndex + sectionFraction) / sections.length);
    }
    setProgress((current) => Math.abs(current - fullProgress) < 0.0001 ? current : fullProgress);
  }, [sectionIndex, sections, sectionWeightSums, totalSectionWeight]);

  const syncRelocation = useCallback(() => {
    const total = getPageCount();
    const actualPage = Math.min(total - 1, getCurrentPage());
    const targetPage = targetPageRef.current;
    const visiblePage = targetPage === null ? actualPage : Math.min(total - 1, targetPage);
    const targetReached =
      targetPage === null || (targetPage < total && actualPage === targetPage);
    if (targetReached) targetPageRef.current = null;
    publishPage(visiblePage, total);

    if (!targetReached) return;

    const nextLocation = readLocation();
    if (nextLocation && !sameLocation(locationRef.current, nextLocation)) {
      locationRef.current = nextLocation;
      setLocation(nextLocation);
    }
  }, [getCurrentPage, getPageCount, publishPage, readLocation]);

  // 注意：定时器必须用宿主 realm 的 window.setTimeout——iframe 带 sandbox（无 allow-scripts）
  // 时其文档 scripting 被禁用，win.setTimeout 的回调永远不会执行（实测封面之后的
  // 位置同步全部丢失即此原因）。
  const scheduleRelocationSync = useCallback((delay: number) => {
    if (!iframeRef.current?.contentWindow) return;
    if (relocationTimerRef.current !== null) {
      window.clearTimeout(relocationTimerRef.current);
    }
    relocationTimerRef.current = window.setTimeout(() => {
      relocationTimerRef.current = null;
      syncRelocation();
    }, delay);
  }, [syncRelocation]);

  const handleScroll = useCallback(() => {
    scheduleRelocationSync(100);
  }, [scheduleRelocationSync]);

  /* 翻页滚动行为：slide 用原生平滑滚动；fade/flip 由视图层播 CSS 动画，
     滚动本身瞬移（动画中点换页）；none 直接瞬移。 */
  const goToPage = useCallback((page: number, smooth = true) => {
    const win = iframeRef.current?.contentWindow;
    const viewport = getViewport();
    const extent = readerPageExtent(flow, viewport);
    if (!win || extent <= 0) return;
    const total = getPageCount();
    const clamped = Math.max(0, Math.min(total - 1, Math.trunc(page)));
    targetPageRef.current = clamped;
    publishPage(clamped, total);
    if (smooth && pageTurnStyle === 'slide') {
      animateReaderToPage(win, flow, extent, clamped, () => scheduleRelocationSync(60));
    } else {
      scrollReaderToPage(win, flow, extent, clamped, 'auto');
      scheduleRelocationSync(50);
    }
  }, [flow, getPageCount, getViewport, pageTurnStyle, publishPage, scheduleRelocationSync]);

  const goToAnchorInCurrentSection = useCallback((href: string, offset = 0) => {
    const { anchor } = splitReaderHref(href);
    if (!anchor) {
      goToPage(offset, false);
      return;
    }

    const match = collectAnchors().find((point) => point.key === anchor);
    if (match) {
      goToPage(match.page + offset, false);
      return;
    }

    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    const extent = readerPageExtent(flow, getViewport());
    const element = doc?.getElementById(anchor);
    if (element && win) {
      goToPage(readerElementPage(element, win, flow, extent) + offset, false);
      return;
    }
    goToPage(offset, false);
  }, [collectAnchors, flow, getDoc, getViewport, goToPage]);

  const beginSectionNavigation = useCallback((index: number, pending: PendingNavigation) => {
    if (index < 0 || index >= sections.length || index === sectionIndex) return;
    pendingNavigationRef.current = pending;
    targetPageRef.current = null;
    locationRef.current = null;
    setLocation(null);
    setPageInfo({ page: 0, total: 1 });
    readyRef.current = false;
    setDocSnapshot(null);
    setSectionIndex(index);
  }, [sectionIndex, sections.length]);

  const goToAnchor = useCallback((href: string | null, offset = 0) => {
    if (!href || sections.length === 0) return;
    const parsed = splitReaderHref(href);
    const currentPath = currentSection?.href ?? '';
    const requestedPath = normalizeEpubPath(parsed.path || currentPath);
    const targetIndex = sections.findIndex(
      (section) => normalizeEpubPath(splitReaderHref(section.href).path) === requestedPath,
    );
    if (targetIndex < 0) return;
    if (targetIndex !== sectionIndex) {
      beginSectionNavigation(targetIndex, { kind: 'location', href, offset });
      return;
    }
    goToAnchorInCurrentSection(href, offset);
  }, [beginSectionNavigation, currentSection?.href, goToAnchorInCurrentSection, sectionIndex, sections]);

  const goTo = useCallback((href: string) => goToAnchor(href, 0), [goToAnchor]);

  const nextPage = useCallback(() => {
    const total = getPageCount();
    const page = targetPageRef.current ?? getCurrentPage();
    if (page < total - 1) {
      goToPage(page + 1);
      return;
    }
    beginSectionNavigation(sectionIndex + 1, { kind: 'start' });
  }, [beginSectionNavigation, getCurrentPage, getPageCount, goToPage, sectionIndex]);

  const prevPage = useCallback(() => {
    const page = targetPageRef.current ?? getCurrentPage();
    if (page > 0) {
      goToPage(page - 1);
      return;
    }
    beginSectionNavigation(sectionIndex - 1, { kind: 'end' });
  }, [beginSectionNavigation, getCurrentPage, goToPage, sectionIndex]);

  const goToProgress = useCallback((ratio: number) => {
    if (sections.length === 0) return;
    const clamped = Math.min(1, Math.max(0, ratio));
    let targetIndex: number;
    let fraction: number;
    if (totalSectionWeight > 0) {
      // 加权进度的逆映射：按累计权重定位章节与章内比例。
      const target = clamped * totalSectionWeight;
      targetIndex = sections.length - 1;
      for (let index = 0; index < sections.length; index += 1) {
        if (target <= sectionWeightSums[index + 1]) {
          targetIndex = index;
          break;
        }
      }
      const weight = sections[targetIndex]?.textLength ?? 0;
      fraction = weight > 0
        ? Math.min(1, Math.max(0, (target - sectionWeightSums[targetIndex]) / weight))
        : 0;
    } else {
      const scaled = clamped * sections.length;
      targetIndex = clamped === 1
        ? sections.length - 1
        : Math.min(sections.length - 1, Math.floor(scaled));
      fraction = clamped === 1 ? 1 : scaled - targetIndex;
    }
    if (targetIndex !== sectionIndex) {
      beginSectionNavigation(targetIndex, { kind: 'fraction', fraction });
      return;
    }
    goToPage(Math.round(fraction * Math.max(0, getPageCount() - 1)), false);
  }, [beginSectionNavigation, getPageCount, goToPage, sectionIndex, sections, sectionWeightSums, totalSectionWeight]);

  const applyPendingNavigation = useCallback(() => {
    const pending = pendingNavigationRef.current;
    pendingNavigationRef.current = null;
    if (!pending) {
      syncRelocation();
      return;
    }
    if (pending.kind === 'start') goToPage(0, false);
    if (pending.kind === 'end') goToPage(getPageCount() - 1, false);
    if (pending.kind === 'fraction') {
      goToPage(Math.round(pending.fraction * Math.max(0, getPageCount() - 1)), false);
    }
    if (pending.kind === 'location') {
      goToAnchorInCurrentSection(pending.href, pending.offset);
    }
  }, [getPageCount, goToAnchorInCurrentSection, goToPage, syncRelocation]);

  const reflowPreservingLocation = useCallback(() => {
    if (!readyRef.current) return;
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    if (!doc || !win) return;
    const pendingTargetPage = targetPageRef.current;
    const savedLocation = locationRef.current;
    applyStyles();
    applyDualLayout();
    // 行内标注（WordWise）会改变文本度量，必须先完成再分页
    void Promise.resolve(annotateRef.current?.(doc)).then(() => {
      if (getDoc() !== doc) return;
      applyFlow();
      /* 宿主 realm 帧调度：iframe 沙箱无 allow-scripts，其窗口帧回调不可依赖。 */
      window.requestAnimationFrame(() => {
        if (getDoc() !== doc) return;
        if (pendingTargetPage !== null) goToPage(pendingTargetPage, false);
        else if (savedLocation) goToAnchorInCurrentSection(savedLocation.href, savedLocation.offset);
        else syncRelocation();
      });
    });
  }, [applyDualLayout, applyFlow, applyStyles, getDoc, goToAnchorInCurrentSection, syncRelocation]);

  /* 重排后的保位（reannotate/relayout 共用）：分页/双页按「页号」恢复——
     页是 extent 的整数倍，比例恢复必然落在页中间（"某页卡在屏幕中间、
     莫名跳转"的直接根源）；单页与双页在此算法一致（extent 已抽象栏宽，
     双页栏宽=半视口，页号语义相同）。滚动模式无页格：保像素偏移。
     宿主 realm 帧调度：iframe 沙箱无 allow-scripts，其窗口帧回调不可依赖。 */
  const restoreAfterFlow = useCallback((doc: Document, win: Window, savedPage: number, savedOffset: number) => {
    window.requestAnimationFrame(() => {
      if (getDoc() !== doc) return;
      if (isPagedFlow(flow)) {
        goToPage(savedPage, false);
      } else {
        const extent = readerPageExtent(flow, getViewport());
        const afterLength = Math.max(doc.documentElement.scrollHeight, doc.body?.scrollHeight ?? 0);
        win.scrollTo(0, Math.min(savedOffset, Math.max(0, afterLength - extent)));
      }
      scheduleRelocationSync(50);
    });
  }, [flow, getDoc, getViewport, goToPage, scheduleRelocationSync]);

  /* 生词即时重标入口：依赖数组里不含标注回调（经 ref 消费），故生词本
     变化不会自动重排——由 ReaderView 侦听词表内容变化后显式触发
     （词表局部变化走局部补丁路径，见 ReaderView；此入口覆盖偏好/档位等
     需要全量重标的情形）。 */
  const reannotate = useCallback(() => {
    if (!readyRef.current) return;
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    if (!doc || !win) return;
    const savedPage = getCurrentPage();
    const savedOffset = readerScrollOffset(win, flow);
    void Promise.resolve(annotateRef.current?.(doc)).then(() => {
      if (getDoc() !== doc) return;
      applyFlow();
      restoreAfterFlow(doc, win, savedPage, savedOffset);
    });
  }, [applyFlow, flow, getDoc, getCurrentPage, restoreAfterFlow]);

  /* 仅重排版（不重标）：生词局部补丁报告修补改变了流布局时调用，
     由分页吸收修补——保位语义与 reannotate 完全一致，用户无跳变感。 */
  const relayout = useCallback(() => {
    if (!readyRef.current) return;
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    if (!doc || !win) return;
    const savedPage = getCurrentPage();
    const savedOffset = readerScrollOffset(win, flow);
    applyFlow();
    restoreAfterFlow(doc, win, savedPage, savedOffset);
  }, [applyFlow, flow, getDoc, getCurrentPage, restoreAfterFlow]);

  const detachDocumentListeners = useCallback(() => {
    documentCleanupRef.current?.();
    documentCleanupRef.current = null;
  }, []);

  const onIframeLoad = useCallback(() => {
    detachDocumentListeners();
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    if (!doc?.body || !win) {
      setRenderError('EPUB 章节渲染失败');
      return;
    }

    setRenderError(null);
    readyRef.current = true;
    applyStyles();
    applyDualLayout();

    const onScroll = () => handleScroll();
    let reflowFrame = 0;
    const scheduleReflow = () => {
      window.cancelAnimationFrame(reflowFrame);
      reflowFrame = window.requestAnimationFrame(reflowPreservingLocation);
    };
    const pendingImages = Array.from(doc.images).filter((image) => !image.complete);
    win.addEventListener('scroll', onScroll, { passive: true });
    pendingImages.forEach((image) => {
      image.addEventListener('load', scheduleReflow);
      image.addEventListener('error', scheduleReflow);
    });
    documentCleanupRef.current = () => {
      win.removeEventListener('scroll', onScroll);
      window.cancelAnimationFrame(reflowFrame);
      if (relocationTimerRef.current !== null) {
        window.clearTimeout(relocationTimerRef.current);
        relocationTimerRef.current = null;
      }
      pendingImages.forEach((image) => {
        image.removeEventListener('load', scheduleReflow);
        image.removeEventListener('error', scheduleReflow);
      });
    };

    // 行内标注（WordWise）会改变文本度量：先完成标注，再分页与发布快照
    void Promise.resolve(annotateRef.current?.(doc)).then(() => {
      if (getDoc() !== doc) return;
      applyFlow();
      setDocSnapshot(doc);

      window.requestAnimationFrame(() => {
        if (getDoc() !== doc) return;
        applyPendingNavigation();
      });

      void doc.fonts?.ready
        .then(() => {
          if (getDoc() === doc) reflowPreservingLocation();
        })
        .catch((error: unknown) => {
          console.warn('EPUB font layout stabilization failed:', error);
        });
      const decodes = Array.from(doc.images, (image) => image.decode?.() ?? Promise.resolve());
      if (decodes.length > 0) {
        void Promise.allSettled(decodes).then(() => {
          if (getDoc() === doc) reflowPreservingLocation();
        });
      }
    });
  }, [
    applyDualLayout,
    applyFlow,
    applyPendingNavigation,
    applyStyles,
    detachDocumentListeners,
    getDoc,
    handleScroll,
    reflowPreservingLocation,
  ]);

  useEffect(() => {
    readyRef.current = false;
    setDocSnapshot(null);
    setRenderError(null);
    detachDocumentListeners();
    return detachDocumentListeners;
  }, [detachDocumentListeners, sectionDocument.document?.key]);

  useEffect(() => {
    reflowPreservingLocation();
  }, [reflowPreservingLocation]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    /* 窗口拖拽缩放会高频连发 resize；全量重标注代价高，
       用尾随 rAF 把一帧内的多次回调合并为一次重排。 */
    let raf = 0;
    const observer = new ResizeObserver(() => {
      if (raf) return;
      raf = window.requestAnimationFrame(() => {
        raf = 0;
        reflowPreservingLocation();
      });
    });
    observer.observe(container);
    return () => {
      if (raf) window.cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, [reflowPreservingLocation]);

  useEffect(() => detachDocumentListeners, [detachDocumentListeners]);

  const setLang = useCallback((value: ReaderLang) => setLangState(value), []);
  const setFlow = useCallback((value: ReaderFlow) => setFlowState(value), []);
  const setPageWidth = useCallback((value: ReaderPageWidth) => setPageWidthState(value), []);
  const setTranslationBlur = useCallback((value: boolean) => setTranslationBlurState(value), []);
  const setTheme = useCallback((value: ReaderTheme | null) => setThemeState(value), []);
  const setTypeExtra = useCallback((value: ReaderTypeExtra) => setTypeExtraState(value), []);
  const setFontFamily = useCallback((value: ReaderFontFamily) => setFontFamilyState(value), []);
  const setCustomFontPath = useCallback((value: string) => setCustomFontPathState(value), []);
  const setEnFontFamily = useCallback((value: ReaderFontFamily) => setEnFontFamilyState(value), []);
  const setCustomEnFontPath = useCallback((value: string) => setCustomEnFontPathState(value), []);
  const setPageTurnStyle = useCallback((value: ReaderPageTurnStyle) => setPageTurnStyleState(value), []);
  const setSideMarginPercent = useCallback((value: number) => setSideMarginPercentState(value), []);
  const setCustomCss = useCallback((value: string) => setCustomCssState(value), []);
  const setBrightnessPercent = useCallback((value: number) => setBrightnessPercentState(value), []);
  const setParagraphGapPx = useCallback((value: number) => setParagraphGapPxState(value), []);
  const setUseBookStyles = useCallback((value: boolean) => setUseBookStylesState(value), []);
  const setColumnCount = useCallback((value: number) => setColumnCountState(value), []);
  const setColumnThresholdPx = useCallback((value: number) => setColumnThresholdPxState(value), []);
  const setTopMarginPx = useCallback((value: number) => setTopMarginPxState(value), []);
  const setBottomMarginPx = useCallback((value: number) => setBottomMarginPxState(value), []);
  const loading =
    packageLoading || Boolean(currentSection && !sectionDocument.document && !sectionDocument.error);
  const error = packageError ?? sectionDocument.error ?? renderError;

  return {
    pkg,
    loading,
    error,
    ready: Boolean(docSnapshot),
    sections,
    toc,
    sectionIndex,
    chapterTitle,
    sectionKey: sectionDocument.document?.key ?? '',
    sectionUrl: sectionDocument.document?.assetUrl ?? '',
    sectionSrcDoc: sectionDocument.document?.srcDoc ?? '',
    pageInfo,
    progress,
    nextPage,
    prevPage,
    goToPage,
    goToProgress,
    location,
    goTo,
    goToAnchor,
    lang,
    setLang,
    flow,
    setFlow,
    pageWidth,
    setPageWidth,
    translationBlur,
    setTranslationBlur,
    fontSize,
    setFontSize,
    lineHeight,
    setLineHeight,
    paraSpacing,
    setParaSpacing,
    theme,
    setTheme,
    typeExtra,
    setTypeExtra,
    fontFamily,
    setFontFamily,
    customFontPath,
    setCustomFontPath,
    enFontFamily,
    setEnFontFamily,
    customEnFontPath,
    setCustomEnFontPath,
    pageTurnStyle,
    setPageTurnStyle,
    sideMarginPercent,
    setSideMarginPercent,
    customCss,
    setCustomCss,
    brightnessPercent,
    setBrightnessPercent,
    paragraphGapPx,
    setParagraphGapPx,
    useBookStyles,
    setUseBookStyles,
    columnCount,
    setColumnCount,
    columnThresholdPx,
    setColumnThresholdPx,
    topMarginPx,
    setTopMarginPx,
    bottomMarginPx,
    setBottomMarginPx,
    reannotate,
    relayout,
    iframeRef,
    containerRef,
    onIframeLoad,
    getDoc,
    docSnapshot,
  };
}

export type EpubReaderApi = ReturnType<typeof useEpubReader>;
