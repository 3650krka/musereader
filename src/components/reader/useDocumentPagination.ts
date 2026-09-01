import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from 'react';
import type { Book } from '@/types';
import {
  readerElementPage,
  readerPageCount,
  readerPageExtent,
  readerScrollOffset,
  scrollReaderToPage,
  type ReaderViewport,
} from './epubPagination';
import type {
  ReaderFlow,
  ReaderLocation,
  ReaderPageInfo,
} from './epubReaderTypes';
import {
  chapterAtProgress,
  findReaderAnchor,
  orderedReaderToc,
  readerTitle,
  safeDecode,
  sameLocation,
} from './documentReaderLocation';

interface DocumentPaginationOptions {
  book: Book;
  sourceKey: string;
  flow: ReaderFlow;
  iframeRef: RefObject<HTMLIFrameElement | null>;
  containerRef: RefObject<HTMLDivElement | null>;
  getDoc: () => Document | null;
  getViewport: () => ReaderViewport;
  applyStyles: () => void;
  applyDualLayout: () => void;
  applyFlow: () => void;
}

interface DocumentPaginationApi {
  pageInfo: ReaderPageInfo;
  progress: number;
  location: ReaderLocation | null;
  chapterTitle: string;
  docSnapshot: Document | null;
  onIframeLoad: () => void;
  nextPage: () => void;
  prevPage: () => void;
  goToProgress: (ratio: number) => void;
  goTo: (href: string) => void;
}

export function useDocumentPagination(
  options: DocumentPaginationOptions,
): DocumentPaginationApi {
  const {
    book,
    sourceKey,
    flow,
    iframeRef,
    containerRef,
    getDoc,
    getViewport,
    applyStyles,
    applyDualLayout,
    applyFlow,
  } = options;
  const title = readerTitle(book);
  const orderedToc = useMemo(() => orderedReaderToc(book.toc ?? []), [book.toc]);
  const [pageInfo, setPageInfo] = useState<ReaderPageInfo>({ page: 0, total: 1 });
  const [progress, setProgress] = useState(0);
  const [location, setLocation] = useState<ReaderLocation | null>(null);
  const [chapterTitle, setChapterTitle] = useState(title);
  const [docSnapshot, setDocSnapshot] = useState<Document | null>(null);
  const targetPageRef = useRef<number | null>(null);
  const relocationTimerRef = useRef<number | null>(null);
  const documentCleanupRef = useRef<(() => void) | null>(null);

  const getPageCount = useCallback(() => {
    const doc = getDoc();
    return doc ? readerPageCount(doc, flow, getViewport()) : 1;
  }, [flow, getDoc, getViewport]);

  const getCurrentPage = useCallback(() => {
    const win = iframeRef.current?.contentWindow;
    const extent = readerPageExtent(flow, getViewport());
    if (!win || extent <= 0) return 0;
    return Math.max(0, Math.floor((readerScrollOffset(win, flow) + 1) / extent));
  }, [flow, getViewport, iframeRef]);

  const publishPage = useCallback((page: number, total: number) => {
    setPageInfo((current) =>
      current.page === page && current.total === total ? current : { page, total },
    );
    const ratio = total <= 1 ? 0 : page / (total - 1);
    setProgress((current) => Math.abs(current - ratio) < 0.0001 ? current : ratio);
    setChapterTitle(chapterAtProgress(orderedToc, ratio, title));
  }, [orderedToc, title]);

  const syncRelocation = useCallback(() => {
    const total = getPageCount();
    const actualPage = Math.min(total - 1, getCurrentPage());
    const requestedTarget = targetPageRef.current;
    const boundedTarget = requestedTarget === null
      ? null
      : Math.min(total - 1, requestedTarget);
    const visiblePage = boundedTarget ?? actualPage;
    const targetReached = boundedTarget === null || boundedTarget === actualPage;
    if (targetReached) targetPageRef.current = null;
    publishPage(visiblePage, total);
    if (!targetReached) return;

    const nextLocation = { href: `#mt-page-${actualPage + 1}`, offset: 0 };
    setLocation((current) => sameLocation(current, nextLocation) ? current : nextLocation);
  }, [getCurrentPage, getPageCount, publishPage]);

  const scheduleRelocationSync = useCallback((delay: number) => {
    if (!iframeRef.current?.contentWindow) return;
    /* 定时器必须用宿主 realm 的 window.setTimeout——文档模式与 EPUB 共用同一
       sandbox iframe（无 allow-scripts），win.setTimeout 的回调永不执行。 */
    if (relocationTimerRef.current !== null) {
      window.clearTimeout(relocationTimerRef.current);
    }
    relocationTimerRef.current = window.setTimeout(() => {
      relocationTimerRef.current = null;
      syncRelocation();
    }, delay);
  }, [iframeRef, syncRelocation]);

  const goToPage = useCallback((page: number, smooth = true) => {
    const win = iframeRef.current?.contentWindow;
    const extent = readerPageExtent(flow, getViewport());
    if (!win || extent <= 0) return;
    const total = getPageCount();
    const target = Math.max(0, Math.min(total - 1, Math.trunc(page)));
    targetPageRef.current = target;
    publishPage(target, total);
    scrollReaderToPage(win, flow, extent, target, smooth ? 'smooth' : 'auto');
    scheduleRelocationSync(smooth ? 500 : 50);
  }, [flow, getPageCount, getViewport, iframeRef, publishPage, scheduleRelocationSync]);

  const nextPage = useCallback(() => {
    goToPage((targetPageRef.current ?? getCurrentPage()) + 1);
  }, [getCurrentPage, goToPage]);

  const prevPage = useCallback(() => {
    goToPage((targetPageRef.current ?? getCurrentPage()) - 1);
  }, [getCurrentPage, goToPage]);

  const goToProgress = useCallback((ratio: number) => {
    const clamped = Math.min(1, Math.max(0, ratio));
    goToPage(Math.round(clamped * Math.max(0, getPageCount() - 1)), false);
  }, [getPageCount, goToPage]);

  const goTo = useCallback((href: string) => {
    const anchor = safeDecode(href.includes('#') ? href.slice(href.indexOf('#') + 1) : href);
    const pageMatch = anchor.match(/^mt-page-(\d+)$/);
    if (pageMatch) {
      goToPage(Number(pageMatch[1]) - 1, false);
      return;
    }

    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    const extent = readerPageExtent(flow, getViewport());
    const element = findReaderAnchor(doc, anchor);
    if (element && win) {
      goToPage(readerElementPage(element, win, flow, extent), false);
      return;
    }
    const tocItem = book.toc?.find((item) => item.href === href);
    if (tocItem) goToProgress(tocItem.progress / 100);
  }, [book.toc, flow, getDoc, getViewport, goToPage, goToProgress, iframeRef]);

  const reflowPreservingPage = useCallback(() => {
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    if (!doc || !win) return;
    const savedPage = targetPageRef.current ?? getCurrentPage();
    applyStyles();
    applyDualLayout();
    applyFlow();
    /* rAF 用宿主 realm（sandbox iframe 事件循环已死，见 scheduleRelocationSync 注释）。 */
    window.requestAnimationFrame(() => {
      if (getDoc() === doc) goToPage(savedPage, false);
    });
  }, [applyDualLayout, applyFlow, applyStyles, getCurrentPage, getDoc, goToPage, iframeRef]);

  const detachDocumentListeners = useCallback(() => {
    documentCleanupRef.current?.();
    documentCleanupRef.current = null;
  }, []);

  const onIframeLoad = useCallback(() => {
    detachDocumentListeners();
    const doc = getDoc();
    const win = iframeRef.current?.contentWindow;
    if (!doc?.body || !win) return;

    applyStyles();
    applyDualLayout();
    applyFlow();
    setDocSnapshot(doc);
    const onScroll = () => scheduleRelocationSync(100);
    const pendingImages = Array.from(doc.images).filter((image) => !image.complete);
    for (const image of pendingImages) {
      image.addEventListener('load', reflowPreservingPage);
      image.addEventListener('error', reflowPreservingPage);
    }
    win.addEventListener('scroll', onScroll, { passive: true });
    documentCleanupRef.current = () => {
      win.removeEventListener('scroll', onScroll);
      if (relocationTimerRef.current !== null) {
        window.clearTimeout(relocationTimerRef.current);
        relocationTimerRef.current = null;
      }
      for (const image of pendingImages) {
        image.removeEventListener('load', reflowPreservingPage);
        image.removeEventListener('error', reflowPreservingPage);
      }
    };

    window.requestAnimationFrame(syncRelocation); /* 宿主 realm rAF，见上。 */
    void doc.fonts?.ready.then(reflowPreservingPage).catch((error: unknown) => {
      console.warn('Document font layout stabilization failed:', error);
    });
  }, [
    applyDualLayout,
    applyFlow,
    applyStyles,
    detachDocumentListeners,
    getDoc,
    iframeRef,
    reflowPreservingPage,
    scheduleRelocationSync,
    syncRelocation,
  ]);

  useEffect(() => {
    setPageInfo({ page: 0, total: 1 });
    setProgress(0);
    setLocation(null);
    setChapterTitle(title);
    setDocSnapshot(null);
    targetPageRef.current = null;
    detachDocumentListeners();
  }, [detachDocumentListeners, sourceKey, title]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => reflowPreservingPage());
    observer.observe(container);
    return () => observer.disconnect();
  }, [containerRef, reflowPreservingPage]);

  useEffect(() => {
    if (docSnapshot) reflowPreservingPage();
  }, [docSnapshot, reflowPreservingPage]);

  useEffect(() => () => detachDocumentListeners(), [detachDocumentListeners]);

  return {
    pageInfo,
    progress,
    location,
    chapterTitle,
    docSnapshot,
    onIframeLoad,
    nextPage,
    prevPage,
    goToProgress,
    goTo,
  };
}
