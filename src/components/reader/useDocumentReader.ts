import { useCallback, useRef } from 'react';
import type { Book } from '@/types';
import { applyReaderFlow, readerPageWidthPx, type ReaderViewport } from './epubPagination';
import type {
  ReaderFlow,
  ReaderFontFamily,
  ReaderFrameApi,
  ReaderFrameTocItem,
  ReaderLang,
  ReaderPageWidth,
  ReaderTheme,
  ReaderTypeExtra,
} from './epubReaderTypes';
import { useDocumentReaderSource } from './useDocumentReaderSource';
import { useDocumentPagination } from './useDocumentPagination';
import { useEpubPresentation } from './useEpubPresentation';

interface DocumentReaderOptions {
  lang: ReaderLang;
  flow: ReaderFlow;
  /** 行宽档位（单页模式的内容最大宽度映射）。 */
  pageWidth: ReaderPageWidth;
  /** 主动回忆：译文默认模糊，悬停/点按显示。 */
  translationBlur: boolean;
  fontSize: number;
  lineHeight: number;
  paraSpacing: number;
  theme: ReaderTheme | null;
  typeExtra?: ReaderTypeExtra;
  fontFamily?: ReaderFontFamily;
  /** 自定义字体路径（fontFamily='custom' 时生效）。 */
  customFontPath?: string;
  /** 英文字体（中英文分开设置）。 */
  enFontFamily?: ReaderFontFamily;
  /** 英文自定义字体路径（enFontFamily='custom' 时生效）。 */
  customEnFontPath?: string;
  /** 页边距（视口宽百分比）。 */
  sideMarginPercent?: number;
  /** 用户自定义 CSS。 */
  customCss?: string;
  /** 亮度百分比(0–100)。 */
  brightnessPercent?: number;
  /** 段间距 px。 */
  paragraphGapPx?: number;
  /** 上/下页边距 px（0–200）。 */
  topMarginPx?: number;
  bottomMarginPx?: number;
}

const EMPTY_TOC: ReaderFrameTocItem[] = [];

export function useDocumentReader(
  book: Book,
  enabled: boolean,
  options: DocumentReaderOptions,
): ReaderFrameApi {
  const source = useDocumentReaderSource(book, options.lang, enabled);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const getDoc = useCallback(() => iframeRef.current?.contentDocument ?? null, []);
  const getViewport = useCallback((): ReaderViewport => ({
    width: iframeRef.current?.clientWidth ?? 0,
    height: iframeRef.current?.clientHeight ?? 0,
  }), []);
  const { applyDualLayout, applyStyles } = useEpubPresentation({
    getDoc,
    hasBilingualMarkup: Boolean(book.segments?.length),
    lang: options.lang,
    fontSize: options.fontSize,
    lineHeight: options.lineHeight,
    paraSpacing: options.paraSpacing,
    theme: options.theme,
    translationBlur: options.translationBlur,
    typeExtra: options.typeExtra,
    fontFamily: options.fontFamily,
    customFontPath: options.customFontPath,
    enFontFamily: options.enFontFamily,
    customEnFontPath: options.customEnFontPath,
    customCss: options.customCss,
    brightnessPercent: options.brightnessPercent,
    paragraphGapPx: options.paragraphGapPx,
    topMarginPx: options.topMarginPx,
    bottomMarginPx: options.bottomMarginPx,
  });
  const applyFlow = useCallback(() => {
    const doc = getDoc();
    const viewport = getViewport();
    if (!doc?.body || viewport.width <= 0 || viewport.height <= 0) return;
    const columns = options.flow === 'spread' ? 2 : 1;
    applyReaderFlow(doc, options.flow, viewport, {
      columns,
      maxContentWidth: readerPageWidthPx(options.pageWidth),
      sideMarginPercent: options.sideMarginPercent,
    });
  }, [getDoc, getViewport, options.flow, options.pageWidth, options.sideMarginPercent]);
  const pagination = useDocumentPagination({
    book,
    sourceKey: source.document?.key ?? '',
    flow: options.flow,
    iframeRef,
    containerRef,
    getDoc,
    getViewport,
    applyStyles,
    applyDualLayout,
    applyFlow,
  });

  return {
    loading: source.loading,
    error: source.error,
    chapterTitle: pagination.chapterTitle,
    sectionIndex: 0,
    sectionKey: source.document?.key ?? '',
    sectionSrcDoc: source.document?.srcDoc ?? '',
    pageInfo: pagination.pageInfo,
    progress: pagination.progress,
    location: pagination.location,
    toc: EMPTY_TOC,
    docSnapshot: pagination.docSnapshot,
    iframeRef,
    containerRef,
    onIframeLoad: pagination.onIframeLoad,
    getDoc,
    nextPage: pagination.nextPage,
    prevPage: pagination.prevPage,
    goToProgress: pagination.goToProgress,
    goTo: pagination.goTo,
  };
}
