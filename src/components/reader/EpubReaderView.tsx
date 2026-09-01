import { AnimatePresence } from 'motion/react';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { AiAssistResponse, Book, ReaderState, ReaderTab } from '@/types';
import { useT } from '@/i18n';
import { cn } from '@/lib/utils';
import { useToast } from '@/components/common/Toast';
import { displayBookTitle } from '@/components/library/bookDisplay';
import type { ParagraphAction } from './useAiAssistant';
import type {
  ReaderFrameApi,
  ReaderInfoSlot,
  ReaderLang,
  ReaderLocation,
  ReaderTheme,
} from './epubReaderTypes';
import { PageFlipSheet } from './PageFlipSheet';
import { customZoneAction, zoneIndex, ZONE_PRESETS, type ReaderZoneAction } from './readerZones';
import { usePageFlip } from './usePageFlip';
import { useInlineReview } from './useInlineReview';
import { useTextMarks, type MarkStyle } from './useTextMarks';
import { convertDocument, convertText } from './chineseVariant';
import { ReaderChrome } from './ReaderChrome';
import { ReaderSelectionMenu } from './ReaderSelectionMenu';
import { NotebookPicker } from '../learning/NotebookPicker';
import { ReaderSidebar } from './ReaderSidebar';
import { ReaderSettingsPanel } from './ReaderSettingsPanel';
import { ReaderAdvancedPanel } from './ReaderAdvancedPanel';
import { TocDrawer } from './TocDrawer';
import { IframeParaMenu, useIframeParaDot, type ParaDotState } from './IframeParaMenu';
import { WordWiseCard, type WordCardState } from './WordWiseCard';
import { extractSentenceContext, type WordLevelInfo } from './wordWiseInject';
import { nextReaderThemeName, type ReaderPrefs } from './useReaderPrefs';

export type { ReaderPrefs };

interface EpubReaderViewProps {
  book: Book;
  onBack: () => void;
  /** 当前阅读主题色（背景图暗化层用）。 */
  readerTheme: ReaderTheme | null;
  /** 阅读器运行时（划线/笔记/生词/进度/AI），由 ReaderView 注入。 */
  runtime: {
    readerState: ReaderState | null;
    activeTab: ReaderTab;
    setActiveTab: (tab: ReaderTab) => void;
    sidebarOpen: boolean;
    setSidebarOpen: (open: boolean | ((v: boolean) => boolean)) => void;
    selection: { text: string; top: number; left: number; bottom?: number } | null;
    setSelection: (s: { text: string; top: number; left: number; bottom?: number } | null) => void;
    noteDraft: string;
    setNoteDraft: (v: string) => void;
    openNotes: () => void;
    openToc: () => void;
    openSearch: () => void;
    saveBookmark: (quote: string, href?: string | null, style?: string) => void;
    saveNote: (quote: string, note: string, href?: string | null) => void;
    removeBookmark: (bookmarkId: string) => void;
    removeBookmarkMark: (quote: string) => void;
    removeNote: (noteId: string) => void;
    reviewMode: boolean;
    toggleReviewMode: () => void;
    markWord: (w: string, s: import('@/types').WordMarkStatus | null, c: string) => void;
    addVocabCard: (w: string, c: string, d?: string, zh?: string) => void;
    saveWordDefinition: (w: string, d: string, c: string) => void;
    openAiPanel: (quote?: string) => void;
    handleParagraphAction: (action: ParagraphAction, paragraph: string) => void;
    aiAssistant: ReturnType<typeof import('./useAiAssistant').useAiAssistant>;
    tts: ReturnType<typeof import('./useReaderTts').useReaderTts>;
  };
  epub: ReaderFrameApi;
  prefs: ReaderPrefs;
  onPrefsChange: (patch: Partial<ReaderPrefs>) => void;
  onLocationChange: (loc: ReaderLocation, chapter: string, progressRatio: number) => void;
  /** 提取当前章节可见文本（TTS 朗读 / 本页生词透析数据源）。 */
  extractPageText: () => string;
  /** WordWise 已标注词的信息表（宿主弹卡查释义用）。 */
  wordInfoMapRef: React.RefObject<Map<string, WordLevelInfo>>;
  /** 打开该书的校对工作台（可选：未提供则顶栏不显示校对入口）。 */
  onOpenReview?: () => void;
}

/** 沉浸阅读器（EPUB 真实渲染层）：固定横向分页 + 双语双栏 + 非遮挡侧栏。 */
export function EpubReaderView({
  book, onBack, readerTheme, runtime, epub, prefs, onPrefsChange, onLocationChange, extractPageText, wordInfoMapRef, onOpenReview,
}: EpubReaderViewProps) {
  const t = useT();
  const toast = useToast();
  const selRef = useRef<HTMLDivElement | null>(null);

  /* 划选菜单的页文本上下文：每次划选算一次即可。放在渲染里直调会令每次
     重渲染都全量扫描可见块并触发同步布局（分页/滚动下代价高）。 */
  const selectionPageText = useMemo(() => (runtime.selection ? extractPageText() : ''),
    [runtime.selection, extractPageText],
  );

  /* 底栏进度滑条：可点击 + 可拖动。拖动期间仅移动滑块预览（跨章跳转代价高，
     不逐帧提交），松手一次性 goToProgress；setPointerCapture 保证拖出条外
     仍持续收到事件、松手即提交。 */
  const pgbarRef = useRef<HTMLDivElement | null>(null);
  const pgDraggingRef = useRef(false);
  const [pgDragRatio, setPgDragRatio] = useState<number | null>(null);
  const pgRatioFromClientX = useCallback((clientX: number): number | null => {
    const el = pgbarRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return null;
    return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  }, []);
  const handlePgPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    const ratio = pgRatioFromClientX(e.clientX);
    if (ratio === null) return;
    pgDraggingRef.current = true;
    try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* 捕获失败退化为点按 */ }
    setPgDragRatio(ratio);
  }, [pgRatioFromClientX]);
  const handlePgPointerMove = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!pgDraggingRef.current) return;
    const ratio = pgRatioFromClientX(e.clientX);
    if (ratio !== null) setPgDragRatio(ratio);
  }, [pgRatioFromClientX]);
  const handlePgPointerUp = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!pgDraggingRef.current) return;
    pgDraggingRef.current = false;
    setPgDragRatio(null);
    const ratio = pgRatioFromClientX(e.clientX);
    if (ratio !== null) epub.goToProgress(ratio);
  }, [epub, pgRatioFromClientX]);
  const handlePgPointerCancel = useCallback(() => {
    pgDraggingRef.current = false;
    setPgDragRatio(null);
  }, []);

  /* 生词本选择弹窗 */
  const [notebookPick, setNotebookPick] = useState<{ word: string; definition: string; context: string } | null>(null);

  /* 点击触发的 3D 纸页翻页（快照捕获/忙碌闸/自动翻页/章切复位，见 usePageFlip）。 */
  const selectionActive = Boolean(runtime.selection?.text);
  const { flipSheet, flipNext, flipPrev, finishFlip } = usePageFlip({
    epub,
    prefs,
    selectionActive,
    reviewMode: runtime.reviewMode,
  });

  /* 划线标记：划线时包 mark 着色；点击已划区域直接进批注输入。 */
  /* 内联校对：校对模式下点段落直接改译文，失焦保存/Esc 取消。 */
  useInlineReview({ getDoc: epub.getDoc, enabled: runtime.reviewMode, taskId: book.id });

  const markClickRef = useRef(false);
  const { applyMark, restoreMarks } = useTextMarks({ getDoc: epub.getDoc });
  /* 划线持久化回注：章节文档就绪/书签变化时重画可视划线。
     过滤：style 可能带 ":color" 后缀（色板自定义色），先 split 再判 kind；
     href 匹配当前章（无 href 老数据退回全文匹配——引文匹配自然过滤）。
     指纹缓存：书签内容未变时跳过重画（防 runtime 往返摧毁活动选区）。 */
  const restoredMarksFingerprintRef = useRef('');
  useEffect(() => {
    if (!epub.docSnapshot) return;
    const href = epub.location?.href ?? null;
    const marks = (runtime.readerState?.bookmarks ?? [])
      .map((bm) => {
        const [kind, color] = (bm.style ?? '').split(':');
        return { quote: bm.quote, style: kind, color: color ?? null, href: bm.href ?? null };
      })
      .filter((m) => m.style === 'highlight' || m.style === 'underline' || m.style === 'wave')
      .filter((m) => {
        if (!m.href || !href) return true; // 无定位数据：退回引文全文匹配
        // 章节定位：书签 href 与当前章 href 同源（去锚点比较）
        const chapterOf = (h: string) => h.split('#')[0];
        return chapterOf(m.href) === chapterOf(href);
      })
      .map((m) => ({ quote: m.quote, style: m.style, color: m.color }));
    const fingerprint = `${href ?? ''}|${prefs.chineseVariant}|${marks.map((m) => `${m.style}:${m.color ?? ''}:${m.quote}`).join('§')}`;
    if (fingerprint === restoredMarksFingerprintRef.current) return;
    restoredMarksFingerprintRef.current = fingerprint;
    /* 简繁转换开启时正文已转换：quote 同向转换后再匹配（修复回注先于转换的丢失） */
    const variant = prefs.chineseVariant;
    const matchMarks = variant === 'none'
      ? marks
      : marks.map((m) => ({ ...m, quote: convertText(m.quote, variant) }));
    restoreMarks(matchMarks);
  }, [epub.docSnapshot, epub.location?.href, runtime.readerState?.bookmarks, restoreMarks, prefs.chineseVariant]);
  /* iframe 文档就绪后绑定 mark 点击（每次 section 重建重绑） */
  useEffect(() => {
    const doc = epub.getDoc();
    if (!doc?.body) return;
    const handler = (event: Event) => {
      const target = event.target as Element | null;
      const mark = target?.closest?.('mark.mt-mark');
      if (!mark) return;
      markClickRef.current = true;
      const rect = mark.getBoundingClientRect();
      const frame = epub.iframeRef.current;
      if (!frame) return;
      const frameRect = frame.getBoundingClientRect();
      runtime.setSelection({
        text: (mark.textContent ?? '').trim(),
        top: frameRect.top + rect.top - 8,
        left: frameRect.left + rect.left + rect.width / 2,
        bottom: frameRect.top + rect.bottom + 8,
      });
    };
    doc.addEventListener('click', handler);
    return () => doc.removeEventListener('click', handler);
  }, [epub.docSnapshot, epub.getDoc, epub.iframeRef, runtime]);

  /* 监听 iframe 内划选 → 浮出小红点菜单（位置映射到宿主坐标） */
  useEffect(() => {
    const doc = epub.getDoc();
    const frame = epub.iframeRef.current;
    if (!doc || !frame) return;
    const onMouseUp = () => {
      const sel = doc.getSelection?.();
      const text = sel?.toString().trim() ?? '';
      if (!text || !sel || sel.rangeCount === 0) return;
      markClickRef.current = false;
      const range = sel.getRangeAt(0);
      const rect = range.getBoundingClientRect();
      const frameRect = frame.getBoundingClientRect();
      runtime.setSelection({
        text,
        top: frameRect.top + rect.top - 8,
        left: frameRect.left + rect.left + rect.width / 2,
        bottom: frameRect.top + rect.bottom + 8,
      });
    };
    // 触屏划选：长按选词后手指抬起时 selection 才稳定，延迟一拍再读。
    const onTouchEnd = () => {
      window.setTimeout(onMouseUp, 120);
    };
    doc.addEventListener('mouseup', onMouseUp);
    doc.addEventListener('touchend', onTouchEnd, { passive: true });
    return () => {
      doc.removeEventListener('mouseup', onMouseUp);
      doc.removeEventListener('touchend', onTouchEnd);
    };
  }, [epub.docSnapshot, epub.getDoc, epub.iframeRef, epub.sectionIndex, runtime.setSelection]);

  /* iframe 点击桥（常驻）：正文 pointerdown 不冒泡到宿主 document，
     转发为宿主事件 mt:iframe-pointerdown——划选浮窗/词卡等点外关闭逻辑监听它。
     生词标注与拖拽起手有自己的语义，浮窗内部自行过滤目标。 */
  useEffect(() => {
    const doc = epub.getDoc();
    if (!doc) return;
    const onDocPointerDown = () => {
      document.dispatchEvent(new Event('mt:iframe-pointerdown'));
      runtime.setSelection(null);
    };
    doc.addEventListener('pointerdown', onDocPointerDown);
    return () => doc.removeEventListener('pointerdown', onDocPointerDown);
  }, [epub.docSnapshot, epub.getDoc, runtime.setSelection]);

  /* 定位变化 → 持久化进度 + 章节名 */
  useEffect(() => {
    if (!epub.location) return;
    onLocationChange(epub.location, epub.chapterTitle, epub.progress);
  }, [epub.chapterTitle, epub.location, epub.progress, onLocationChange]);

  /* ── 沉浸 chrome：默认隐藏；中央热区唤出/收起；侧栏开启自动隐藏（用户可再唤起）；
        设置面板打开时强制显示（面板锚定在顶栏按钮旁）。 ── */
  const [chromeVisible, setChromeVisible] = useState(false);
  useEffect(() => {
    if (runtime.sidebarOpen) setChromeVisible(false);
  }, [runtime.sidebarOpen]);
  useEffect(() => {
    if (prefs.settingsOpen) setChromeVisible(true);
  }, [prefs.settingsOpen]);
  /* 简繁转换：新文档就绪或模式切换时对正文做显示层转换。
     叠加防护：每次切换先从原始 srcDoc 重建 iframe 内容再单向转换
     （避免 t2s→s2t 等多级转换时映射表重复键产生不可逆漂移）。 */
  const variantAppliedRef = useRef('');
  /* 换章：新文档未转换，重置记录（docSnapshot 变化触发上方 effect）。 */
  useEffect(() => {
    variantAppliedRef.current = '';
  }, [epub.sectionKey]);
  useEffect(() => {
    const frame = epub.iframeRef.current;
    const doc = epub.getDoc();
    if (!frame || !doc?.body) return;
    const mode = prefs.chineseVariant;
    const applied = variantAppliedRef.current;
    if (mode === 'none') {
      if (applied !== 'none') {
        // 从转换态回原文：重建文档（srcDoc 是未转换原始内容）
        variantAppliedRef.current = 'none';
        const scrollX = doc.defaultView?.scrollX ?? 0;
        const scrollY = doc.defaultView?.scrollY ?? 0;
        frame.srcdoc = epub.sectionSrcDoc;
        doc.defaultView?.scrollTo(scrollX, scrollY);
      }
      return;
    }
    if (applied === mode) return; // 同模式幂等
    if (applied !== 'none' && applied !== '') {
      // 换向：先回到原文再应用新方向
      variantAppliedRef.current = mode;
      frame.srcdoc = epub.sectionSrcDoc;
      // srcdoc 重载是异步的：下一帧再转换
      window.setTimeout(() => {
        const fresh = epub.getDoc();
        if (fresh?.body) convertDocument(fresh, mode);
      }, 60);
      return;
    }
    variantAppliedRef.current = mode;
    convertDocument(doc, mode);
  }, [epub.docSnapshot, epub.getDoc, epub.iframeRef, epub.sectionSrcDoc, prefs.chineseVariant]);

  const toggleChrome = useCallback(() => setChromeVisible((v) => !v), []);

  /* 首次进入阅读器的分区提示条（一次性 coach hint）：告知唤出/翻页手势，
     6 秒后自动消隐并持久化已读标记。 */
  const [zoneHintShown, setZoneHintShown] = useState(() => {
    try { return window.localStorage.getItem('mt.reader.zoneHint') === '1'; } catch { return true; }
  });
  const zoneHintRef = useRef(zoneHintShown);
  useEffect(() => {
    if (zoneHintShown) return;
    const timer = window.setTimeout(() => {
      try { window.localStorage.setItem('mt.reader.zoneHint', '1'); } catch { /* 忽略 */ }
      setZoneHintShown(true);
    }, 6000);
    return () => window.clearTimeout(timer);
  }, [zoneHintShown]);

  /* 触控/点按分区（相对当前阅读区：侧栏开启阅读区收缩，分区随之等比变化）。
     UX 设计原则：
     1) 任何模式/预设下永远存在唤出带（双页此前中缝 0.5 分界=无 menu 区，已修）。
     2) 边带宽度比例驱动但做像素钳制（翻页带 72–200px、双页中缝半宽 28–64px、
        中央翻页两侧带 48–96px）：窄阅读区保留完整 0.3 比例拇指区，超宽舞台
        不会出现 300px+ 误触翻页带；侧栏开合时随阅读区等比缩放。
     3) 滚动模式两侧带=平滑滚动一屏（而非全区域 toggle——想划选正文常被误唤出）、
        中央唤出工具栏。
     4) topMenu/leftHand/custom 走 3×3 网格查表（anx 对齐，单双页同语义）：
        滚动模式下网格的 prev/next 同样映射为滚动。
     5) standard/swapped（分页）：工具栏=顶部/底部页边距横带（双页另含中缝带），
        翻页=页边左右侧边带；正文中间点按不做任何动作（点词弹卡走词典链路），
        杜绝"多点几下界面闪一下"的误触。 */
  const scrollZoneStep = useCallback((dir: 1 | -1) => {
    const win = epub.iframeRef.current?.contentWindow;
    if (!win) return;
    win.scrollBy({ top: dir * Math.max(220, win.innerHeight * 0.85), behavior: 'smooth' });
  }, [epub.iframeRef]);
  const runZoneAction = useCallback(
    (ratioX: number, ratioY: number, width: number, height: number) => {
      /* 首次分区动作即消隐提示条（用户已上手，无需等 6s） */
      if (!zoneHintRef.current) {
        zoneHintRef.current = true;
        try { window.localStorage.setItem('mt.reader.zoneHint', '1'); } catch { /* 忽略 */ }
        setZoneHintShown(true);
      }
      const clampPx = (value: number, lo: number, hi: number) => Math.min(Math.max(value, lo), hi);
      const pxRatio = (px: number) => (width > 0 ? px / width : 0.3);
      /* 翻页边带收窄到页边距量级：约 6% 比例、钳 28–96px——边带落在栏
         内边距（无正文）上，"点页边距翻页"；正文区不再被宽翻页带覆盖，
         点词查义与翻页互不误触。 */
      const edge = pxRatio(clampPx(0.06 * width, 28, 96));
      const zones = prefs.pageTurnZones;
      const gridAction = (): ReaderZoneAction =>
        zones === 'custom'
          ? customZoneAction(prefs.pageTurnCustomZones, ratioX, ratioY)
          : ZONE_PRESETS[zones as 'topMenu' | 'leftHand'][zoneIndex(ratioX, ratioY)];

      /* 滚动模式：边带滚动一屏、中央（或网格 menu 格）唤出 */
      if (prefs.flow === 'scrolled') {
        const action =
          zones === 'topMenu' || zones === 'leftHand' || zones === 'custom'
            ? gridAction()
            : ratioX < edge ? 'prev' : ratioX > 1 - edge ? 'next' : 'menu';
        if (action === 'none') return;
        if (action === 'menu') toggleChrome();
        else scrollZoneStep(action === 'prev' ? -1 : 1);
        return;
      }

      if (zones === 'topMenu' || zones === 'leftHand' || zones === 'custom') {
        const action = gridAction();
        if (action === 'none') return;
        if (action === 'menu') {
          toggleChrome();
          return;
        }
        if (action === 'prev') flipPrev();
        else flipNext();
        return;
      }
      const spread = prefs.flow === 'spread';
      if (zones === 'centerTurn') {
        /* 中央翻页：两侧唤出带（单页保持 0.3 宽唤出带不受窄边带影响；双页钳 48–96px） */
        const menuEdge = spread ? pxRatio(clampPx(0.14 * width, 48, 96)) : pxRatio(clampPx(0.3 * width, 72, 200));
        if (ratioX < menuEdge || ratioX > 1 - menuEdge) {
          toggleChrome();
          return;
        }
        if (ratioX < 0.5) {
          flipPrev();
          return;
        }
        flipNext();
        return;
      }
      const prevOnLeft = zones !== 'swapped';
      /* 工具栏（顶部/底部栏）唤出区：顶部/底部页边距横带（约 7% 高、钳 36–72px）。
         正文中间不再整片唤出——点词弹卡与工具栏互不误触，杜绝"多点几下界面闪一下"。 */
      const vBand = height > 0 ? clampPx(0.07 * height, 36, 72) / height : 0.07;
      if (ratioY < vBand || ratioY > 1 - vBand) {
          toggleChrome();
        return;
      }
      if (spread) {
        /* 双页：外缘边带翻页（左页左缘=向前、右页右缘=向后），中缝带唤出，
           其余正文区不动作（点词弹卡走词典逻辑）。 */
        const gutter = pxRatio(clampPx(0.035 * width, 20, 48));
        if (ratioX < edge) {
          (prevOnLeft ? flipPrev : flipNext)();
          return;
        }
        if (ratioX > 1 - edge) {
        (prevOnLeft ? flipNext : flipPrev)();
          return;
        }
        if (Math.abs(ratioX - 0.5) < gutter) {
          toggleChrome();
          return;
        }
        return;
      }
      if (ratioX < edge) {
        (prevOnLeft ? flipPrev : flipNext)();
        return;
      }
      if (ratioX > 1 - edge) {
        (prevOnLeft ? flipNext : flipPrev)();
        return;
      }
    },
    [flipNext, flipPrev, prefs.flow, prefs.pageTurnZones, prefs.pageTurnCustomZones, scrollZoneStep, toggleChrome],
  );

  /* 浮层关闭点击吞掉：划选浮窗/单词卡/小红点浮窗因「点外关闭」后，
     同一次 click 仍会到达舞台/iframe 分区逻辑 → 误翻页/误唤出工具栏。
     关闭时打 600ms 时间戳，分区动作在窗口内一律吞掉。 */
  const suppressZoneUntilRef = useRef(0);
  const dismissOverlayStamp = useCallback(() => {
    suppressZoneUntilRef.current = performance.now() + 600;
  }, []);
  const zoneSuppressed = () => performance.now() < suppressZoneUntilRef.current;

  const handleStageClick = useCallback(
    (e: React.MouseEvent) => {
      if (zoneSuppressed()) return;
      if (runtime.selection?.text.trim()) return;
      if ((e.target as HTMLElement).closest('.chrome, .rside, .selmenu, button, textarea, input, [contenteditable], .settings-panel')) return;
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      runZoneAction(
        (e.clientX - rect.left) / rect.width,
        (e.clientY - rect.top) / rect.height,
        rect.width,
        rect.height,
      );
    },
    [runZoneAction, runtime.selection?.text],
  );

  /* iframe 内点按桥：正文点击不冒泡到宿主，需在 iframe 文档内监听并映射分区。
     让位：划选有文本、生词标注、段落红点、链接、主动回忆译文（有点按揭示语义）。 */
  useEffect(() => {
    const doc = epub.getDoc();
    const frame = epub.iframeRef.current;
    if (!doc || !frame) return;
    const onDocClick = (event: MouseEvent) => {
      /* 浮层刚被这次点击关闭（点外关闭）：吞掉，不翻页/不唤出工具栏 */
      if (zoneSuppressed()) return;
      if (runtime.selection?.text.trim()) return;
      /* 划选/双击选词残留的 click 一律让位（原守卫只罩查词分支——
         双击 detail≥2 直落分区动作，双击点词的肌肉记忆会误翻页/误唤出）。 */
      if (doc.getSelection?.()?.toString().trim()) return;
      const target = event.target as Element | null;
      if (!target) return;
      if (target.closest('span.mt-ww, #mt-para-dot, a[href], mark.mt-mark')) return;
      if (prefs.translationBlur && target.closest('.mt-zh,.mt-ch')) return;
      /* 单击查词：单击命中英文词 → 必弹词典卡（未收录/空释义弹空态卡，
         引导 AI 语境义补全——不再回透分区动作，消除点词"没反应"）；
         长按/划选由上方 selection 守卫与触摸语义天然让位。 */
      const width = frame.clientWidth;
      const height = frame.clientHeight;
      const zoneFallback = () => {
        if (width <= 0 || height <= 0) return;
        runZoneAction(event.clientX / width, event.clientY / height, width, height);
      };
      if (prefs.clickDictionary && event.detail === 1) {
        const hit = wordAtPoint(doc, event.clientX, event.clientY);
        if (hit) {
          void invoke<WordLevelInfo | null>(
            'lookup_word_level', { word: hit.word },
          ).then((local) => {
            /* 章节已切换（lookup 期间翻页）：过期结果丢弃，防新页弹旧词卡。 */
            if (epub.getDoc() !== doc) return;
            const info: WordLevelInfo = {
              word: local?.word?.trim() || hit.word,
              level: local?.level ?? '',
              phonetic: local?.phonetic ?? '',
              definition: local?.definition ?? '',
              translation: local?.translation ?? '',
              tag: local?.tag ?? '',
              exchange: local?.exchange ?? '',
              root: local?.root ?? '',
              collins: local?.collins ?? 0,
              oxford: local?.oxford ?? 0,
              bnc: local?.bnc ?? 0,
              frq: local?.frq ?? 0,
            };
            const frameRect = frame.getBoundingClientRect();
            const block = hit.node.parentElement?.closest('p,li,blockquote,h1,h2,h3,h4,h5,h6,figcaption');
            /* 例句只取英文栏文本（双语块混有中文/上标噪声会污染断句）。
               翻译是段落级对齐而非句子对齐——例句不带中文对照，避免张冠李戴。 */
            const enHost = hit.node.parentElement?.closest('.mt-en,.mt-en-h') ?? block ?? null;
            const blockText = englishTextOf(enHost).trim() || hit.word;
            const context = extractSentenceContext(blockText, hit.word) ?? blockText.slice(0, 300);
            /* 点击选中反馈：命中词短暂淡高亮（非 WordWise 标注词没有标注外壳，
               用 Range 包一层临时高亮 span，动画结束自移除）。 */
            flashHitRange(doc, hit);
            setWordCard({
              info,
              context,
              rect: {
                left: frameRect.left + hit.rect.left,
                top: frameRect.top + hit.rect.top,
                width: hit.rect.width,
                height: hit.rect.height,
              },
            });
          }).catch(() => undefined);
          return;
        }
      }
      zoneFallback();
    };
    doc.addEventListener('click', onDocClick);
    return () => {
      doc.removeEventListener('click', onDocClick);
    };
  }, [
    epub.docSnapshot,
    epub.getDoc,
    epub.iframeRef,
    prefs.clickDictionary,
    prefs.translationBlur,
    runZoneAction,
    runtime.selection?.text,
  ]);

  /* 键盘翻页（桌面阅读习惯：←/→ 与 PageUp/PageDown，Space 下一页 / Shift+Space 上一页） */
  useEffect(() => {
    if (prefs.flow === 'scrolled' || !prefs.keyboardShortcutTurnPage) return;
    const onKey = (event: KeyboardEvent) => {
      if ((event.target as HTMLElement | null)?.closest('input, textarea, select, [contenteditable]')) return;
      if (event.key === 'ArrowLeft' || event.key === 'PageUp') flipPrev();
      else if (event.key === 'ArrowRight' || event.key === 'PageDown') flipNext();
      else if (event.key === ' ') {
        event.preventDefault();
        if (event.shiftKey) flipPrev();
        else flipNext();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [flipNext, flipPrev, prefs.flow, prefs.keyboardShortcutTurnPage]);

  /* 边带滑动翻页（横向分页）：页边距带内起手的水平拖拽 → 翻页（触屏滑动
     与鼠标拖拽同语义）。边带落在栏内边距（无正文），与划选天然不冲突；
     水平主导（|dx|>|dy| 且 ≥8px）+ 位移 ≥40px + 松手时无选区，三重防误触。 */
  useEffect(() => {
    if (prefs.flow === 'scrolled') return;
    const doc = epub.getDoc();
    if (!doc) return;
    const bandPx = (w: number) => Math.min(Math.max(0.06 * w, 28), 96);
    let startX = 0;
    let startY = 0;
    let inBand = false;
    let horizontal = false;
    const onDown = (event: PointerEvent) => {
      if (event.button !== 0) return;
      const width = doc.defaultView?.innerWidth ?? 0;
      if (width <= 0) return;
      const band = bandPx(width);
      inBand = event.clientX <= band || event.clientX >= width - band;
      horizontal = false;
      startX = event.clientX;
      startY = event.clientY;
    };
    const onMove = (event: PointerEvent) => {
      if (!inBand || horizontal) return;
      const dx = event.clientX - startX;
      const dy = event.clientY - startY;
      if (Math.abs(dx) > 8 && Math.abs(dx) > Math.abs(dy)) horizontal = true;
    };
    const onUp = (event: PointerEvent) => {
      if (!inBand) return;
      inBand = false;
      if (!horizontal) return;
      const dx = event.clientX - startX;
      if (Math.abs(dx) < 40) return;
      /* 拖拽跨入正文可能带出选区：有选区让位（保划选），无选区才翻页。
         翻页后打卡抑制窗：滑动松手仍会派发 click（目标=按下/抬起元素的
         公共祖先），不吞掉会紧接着弹词卡/再翻一页。 */
      if (doc.getSelection?.()?.toString().trim()) return;
      suppressZoneUntilRef.current = performance.now() + 600;
      if (dx < 0) flipNext();
      else flipPrev();
    };
    doc.addEventListener('pointerdown', onDown);
    doc.addEventListener('pointermove', onMove);
    doc.addEventListener('pointerup', onUp);
    return () => {
      doc.removeEventListener('pointerdown', onDown);
      doc.removeEventListener('pointermove', onMove);
      doc.removeEventListener('pointerup', onUp);
    };
  }, [epub.docSnapshot, epub.getDoc, prefs.flow, flipNext, flipPrev]);

  /* 屏幕常亮（anx keepScreenOn 对齐）：WebView WakeLock API；
     阅读器挂载期间申请，不可用（旧 WebView/无电池设备）静默降级。 */
  useEffect(() => {
    if (!prefs.keepScreenOn) return;
    let lock: WakeLockSentinel | null = null;
    let released = false;
    const nav = navigator as Navigator & { wakeLock?: { request: (type: 'screen') => Promise<WakeLockSentinel> } };
    const acquire = async () => {
      try {
        if (!nav.wakeLock || released) return;
        lock = await nav.wakeLock.request('screen');
        lock.addEventListener('release', () => { lock = null; });
      } catch { /* 不支持或被系统拒绝：静默降级 */ }
    };
    void acquire();
    /* 页面切后台锁会自动释放，回前台重新申请。 */
    const onVisibility = () => { if (document.visibilityState === 'visible') void acquire(); };
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      released = true;
      document.removeEventListener('visibilitychange', onVisibility);
      lock?.release().catch(() => undefined);
    };
  }, [prefs.keepScreenOn]);

  const langForToggle: ReaderLang = prefs.lang;

  /* 翻页动画：页码变化时给舞台挂一次性 class（CSS 动画播完摘除）。
     slide 由 iframe 原生平滑滚动承担；fade 在此层播放；none 无动画。
     book/flip 均由 PageFlipSheet 软页折叠层承担（快照几何 + 底页遮挡）——
     不走 CSS 通道（flip 此前的整帧 rotateY 在双页下无物理意义且会闪）。 */
  const [pageAnim, setPageAnim] = useState<{ kind: 'fade'; dir: 'next' | 'prev' } | null>(null);
  const lastAnimPageRef = useRef<number | null>(null);
  useEffect(() => {
    const prev = lastAnimPageRef.current;
    lastAnimPageRef.current = epub.pageInfo.page;
    if (prev === null || prev === epub.pageInfo.page || prefs.flow === 'scrolled') return;
    const style = prefs.pageTurnStyle;
    if (style !== 'fade') return;
    const dir = epub.pageInfo.page > prev ? 'next' : 'prev';
    setPageAnim({ kind: style, dir });
    const duration = 460;
    const timer = window.setTimeout(() => setPageAnim(null), duration);
    return () => window.clearTimeout(timer);
  }, [epub.pageInfo.page, prefs.flow, prefs.pageTurnStyle]);

  /* 页脚时间（仅时间槽位启用 30s 轮询） */
  const [clock, setClock] = useState(() => formatClock(new Date()));

  /* 页眉/页脚槽位文本 */
  const slotText = useCallback(
    (slot: ReaderInfoSlot): string => {
      switch (slot) {
        case 'chapter':
          return epub.chapterTitle;
        case 'chapterProgress':
          return `${epub.pageInfo.page + 1} / ${epub.pageInfo.total} · ${Math.round(((epub.pageInfo.page + 1) / Math.max(1, epub.pageInfo.total)) * 100)}%`;
        case 'bookProgress':
          return `${Math.round(epub.progress * 100)}%`;
        case 'time':
          return clock;
        default:
          return '';
      }
    },
    [clock, epub.chapterTitle, epub.pageInfo.page, epub.pageInfo.total, epub.progress],
  );
  const slotsHaveTime = [prefs.headerSlots, prefs.footerSlots].some(
    (s) => s.left === 'time' || s.center === 'time' || s.right === 'time',
  );
  useEffect(() => {
    if (!slotsHaveTime) return;
    const timer = window.setInterval(() => setClock(formatClock(new Date())), 30_000);
    return () => window.clearInterval(timer);
  }, [slotsHaveTime]);

  /* 书本立体样式的双页中缝阴影仅在真实 spread（容器宽 >=1000）时渲染 */
  const [stageWide, setStageWide] = useState(false);
  useEffect(() => {
    const el = epub.containerRef.current;
    if (!el) return;
    const update = () => setStageWide(el.clientWidth >= 1000);
    const observer = new ResizeObserver(update);
    observer.observe(el);
    update();
    return () => observer.disconnect();
  }, [epub.containerRef]);

  /* 双页在窄视口的静默回落提示：用户开了 spread 但窗口/竖屏 <1000px 时分页层
     内部回落单页，顶栏却仍显示「双页」——形同设置无效。回落时给一次性 toast
     说明原因与恢复方式（拉宽/转横屏自动恢复），不再需要用户猜。 */
  const narrowNotifiedRef = useRef(false);
  useEffect(() => {
    if (prefs.flow !== 'spread' || stageWide) {
      if (stageWide) narrowNotifiedRef.current = false;
      return;
    }
    if (narrowNotifiedRef.current) return;
    narrowNotifiedRef.current = true;
    toast.info(t.reader.spreadNarrowHint);
  }, [prefs.flow, stageWide, t.reader.spreadNarrowHint, toast]);

  /* 目录抽屉（左侧）：独立于右侧栏，顶栏目录按钮唤出 */
  const [tocOpen, setTocOpen] = useState(false);
  /* 不常用设置居中弹窗（辅助/生词细节） */
  const [advOpen, setAdvOpen] = useState(false);

  /* 段落小红点：iframe 内悬停段落出现 5px 红点，点击浮出动作菜单 */
  const [paraDot, setParaDot] = useState<ParaDotState | null>(null);
  const handleDotClick = useCallback((state: ParaDotState) => {
    setParaDot(state);
  }, []);
  /* 点外关闭时打卡（小红点浮窗不随点外消失 + 关闭点击误触分区）：
     监听宿主与 iframe 转发的 pointerdown，点外即收 + 600ms 分区抑制。 */
  useEffect(() => {
    if (!paraDot) return;
    const onDown = (event: Event) => {
      const target = event.target as Element | null;
      if (target?.closest?.('.para-dot-menu, .selmenu, .word-wise-card')) return;
      dismissOverlayStamp();
      setParaDot(null);
    };
    document.addEventListener('pointerdown', onDown);
    document.addEventListener('mt:iframe-pointerdown', onDown);
    return () => {
      document.removeEventListener('pointerdown', onDown);
      document.removeEventListener('mt:iframe-pointerdown', onDown);
    };
  }, [paraDot, dismissOverlayStamp]);
  useIframeParaDot(epub.getDoc, epub.iframeRef, !!epub.docSnapshot && !runtime.sidebarOpen && prefs.showParaDot, handleDotClick);
  // 章节切换/关闭侧栏时收起段落菜单
  useEffect(() => {
    setParaDot(null);
  }, [epub.sectionIndex, epub.sectionKey]);

  /* WordWise 生词点击桥：iframe 内点 span.mt-ww 标注 → 宿主坐标弹注释卡片。
     stopPropagation + preventDefault：不触发翻页/划选。 */  const [wordCard, setWordCard] = useState<WordCardState | null>(null);
  useEffect(() => {
    const doc = epub.getDoc();
    const frame = epub.iframeRef.current;
    if (!doc || !frame || !prefs.wordWise) return;
    const onClick = (event: MouseEvent) => {
      /* 滑动翻页松手后的残留 click（目标=按下/抬起元素的公共祖先）：
       抑制窗内不弹卡——滑动与点词互不误触。 */
      if (zoneSuppressed()) return;
      const mark = (event.target as Element | null)?.closest?.('span.mt-ww');
      if (!mark) return;
      event.preventDefault();
      event.stopPropagation();
      const word = mark.getAttribute('data-word') ?? '';
      const info = wordInfoMapRef.current.get(word);
      if (!info) return;
      /* 点击选中反馈：词形短暂淡高亮（动画结束自然回落） */
      mark.classList.remove('mt-ww-hit');
      void (mark as HTMLElement).offsetWidth; // 重启动画（连续点同一词）
      mark.classList.add('mt-ww-hit');
      const block = mark.closest('p,li,blockquote,h1,h2,h3,h4,h5,h6,figcaption');
      /* 英文例句只取英文栏文本（双语配对块里整块 textContent 混有中文与
         上标噪声，会污染断句）；克隆摘除上标节点后取纯词形文本。
         翻译是段落级对齐而非句子对齐——例句不带中文对照，避免张冠李戴。 */
      const enHost = mark.closest('.mt-en,.mt-en-h') ?? block;
      const blockText = englishTextOf(enHost ?? mark).trim() || word;
      /* 例句取所在句子（而非段落）；句提取失败退回短语窗口。 */
      const context = extractSentenceContext(blockText, word)
        ?? blockText.slice(0, 300);
      const rect = mark.getBoundingClientRect();
      const frameRect = frame.getBoundingClientRect();
      setWordCard({
        info,
        context,
        rect: {
          left: frameRect.left + rect.left,
          top: frameRect.top + rect.top,
          width: rect.width,
          height: rect.height,
        },
      });
    };
    doc.addEventListener('click', onClick);
    return () => doc.removeEventListener('click', onClick);
  }, [epub.docSnapshot, epub.getDoc, epub.iframeRef, prefs.wordWise, wordInfoMapRef]);
  // 翻页/换章/关词义时收起词卡
  useEffect(() => {
    setWordCard(null);
  }, [epub.sectionIndex, epub.sectionKey, epub.pageInfo.page, prefs.wordWise]);

  /* 侧栏生词行点击 → 视口上方居中弹出 WordWise 词卡（语境取生词库成卡时的例句）。 */
  const openWordFromSidebar = (
    info: { word: string; level: string; phonetic: string; definition: string },
    context?: string,
    contextZh?: string,
  ) => {
    setWordCard({
      info,
      context: context ?? '',
      contextZh,
      rect: { left: window.innerWidth / 2 - 40, top: 150, width: 80, height: 1 },
    });
  };

  return (
    <div
      className="reader-immersive"
      /* 阅读主题接管沉浸视图底色（切主题只闪屏=iframe 外舞台不变）；
         用户背景图模式仍用主题色叠加层，不重复铺底。 */
      style={readerTheme && !prefs.themeBgImagePath ? { background: readerTheme.bg } : undefined}
    >
      <ReaderChrome
        book={book}
        visible={chromeVisible}
        chapter={epub.chapterTitle}
        bilingualMode={prefs.lang === 'both'}
        wordWiseEnabled={prefs.wordWise}
        spreadMode={prefs.flow !== 'scrolled'}
        flowMode={prefs.flow}
        typographyOpen={prefs.settingsOpen}
        sidebarOpen={runtime.sidebarOpen}
        ttsSpeaking={runtime.tts.speaking}
        ttsLoading={runtime.tts.loading}
        onBack={onBack}
        onCycleTheme={() => {
          /* 立即反馈当前切到的主题（此前循环多次无感，形同"无效"）。 */
          const next = nextReaderThemeName(prefs.themeName);
          onPrefsChange({ themeCycle: true });
          toast.info(`${t.reader.themeSwitched} · ${t.reader.themeNames[next] ?? next}`);
        }}
        onToggleBilingual={() =>
          onPrefsChange({
            lang: langForToggle === 'both' ? 'zh' : langForToggle === 'zh' ? 'en' : 'both',
          })
        }
        onToggleWordWise={() => onPrefsChange({ wordWise: !prefs.wordWise })}
        onToggleSpread={() =>
          onPrefsChange({
            flow:
              prefs.flow === 'paginated'
                ? 'spread'
                : prefs.flow === 'spread'
                  ? 'scrolled'
                  : 'paginated',
          })
        }
        onToggleTypography={() => onPrefsChange({ settingsOpen: !prefs.settingsOpen })}
        onOpenToc={() => setTocOpen(true)}
        onOpenSearch={runtime.openSearch}
        onOpenNotes={runtime.openNotes}
        onOpenReview={onOpenReview}
        reviewMode={runtime.reviewMode}
        onToggleReviewMode={onOpenReview ? runtime.toggleReviewMode : undefined}
        onToggleTts={() => {
          if (runtime.tts.speaking) {
            runtime.tts.stop();
            return;
          }
          const text = extractPageText();
          if (text) void runtime.tts.speak(text);
        }}
        langMode={prefs.lang}
        onSetLang={(lang) => onPrefsChange({ lang })}
      />

      <AnimatePresence>
        {prefs.settingsOpen && (
          <ReaderSettingsPanel
            prefs={prefs}
            onChange={onPrefsChange}
            onClose={() => onPrefsChange({ settingsOpen: false })}
            onOpenAdvanced={() => {
              onPrefsChange({ settingsOpen: false });
              setAdvOpen(true);
            }}
          />
        )}
      </AnimatePresence>

      {/* 不常用设置居中弹窗（辅助 / 生词细节） */}
      <AnimatePresence>
        {advOpen && (
          <ReaderAdvancedPanel
            prefs={prefs}
            onChange={onPrefsChange}
            onClose={() => setAdvOpen(false)}
          />
        )}
      </AnimatePresence>

      {/* 目录抽屉（左侧）：目录 + 全文搜索，独立于右侧栏 */}
      <AnimatePresence>
        {tocOpen && (
          <TocDrawer
            open={tocOpen}
            toc={epub.toc}
            currentHref={epub.location?.href ?? null}
            taskId={book.id}
            onClose={() => setTocOpen(false)}
            onGoTo={(href) => {
              epub.goTo(href);
              setTocOpen(false);
            }}
          />
        )}
      </AnimatePresence>

      {/* 主区：阅读舞台 + 非遮挡侧栏 */}
      <div className="reader-main">
        <div
          ref={epub.containerRef}
          className={cn(
            'epub-stage',
            prefs.flow,
            prefs.bookLook && 'book-look',
            pageAnim?.kind === 'fade' && 'page-anim-fade',
          )}
          onClick={handleStageClick}
          data-sidebar={runtime.sidebarOpen || undefined}
          style={prefs.themeBgImagePath ? {
            backgroundImage: `url(${safeAssetUrl(prefs.themeBgImagePath)})`,
            backgroundSize: 'cover',
            backgroundPosition: 'center',
            backgroundRepeat: 'no-repeat',
          } : undefined}
        >
          {/* 背景图暗化层：正文可读性（主题色叠加，透明度可调） */}
          {prefs.themeBgImagePath && (
            <div
              aria-hidden="true"
              style={{
                position: 'absolute', inset: 0, zIndex: 0, pointerEvents: 'none',
                background: readerTheme
                  ? `linear-gradient(${readerTheme.bg}, ${readerTheme.bg})`
                  : 'var(--card, #fff)',
                opacity: 1 - prefs.themeBgImageOpacity,
              }}
            />
          )}
          {epub.loading && (
            <div className="epub-status">{t.reader.openingBook}</div>
          )}
          {epub.error && (
            <div className="epub-status error">{t.reader.loadFailed}：{epub.error}</div>
          )}
          {!epub.loading && !epub.error && (
            <iframe
              key={epub.sectionKey}
              ref={epub.iframeRef}
              title={displayBookTitle(book, t.library.untitledBook)}
              className="epub-frame"
              sandbox="allow-same-origin"
              srcDoc={epub.sectionSrcDoc}
              onLoad={epub.onIframeLoad}
              data-chinese-variant={prefs.chineseVariant}
            />
          )}
          {/* 页眉/页脚三槽位：非交互 overlay */}
          {hasSlot(prefs.headerSlots) && (
            <div className="reader-page-head" aria-hidden="true" style={{ fontSize: prefs.hfFontSizePx }}>
              <span className="slot left">{slotText(prefs.headerSlots.left)}</span>
              <span className="slot center">{slotText(prefs.headerSlots.center)}</span>
              <span className="slot right">{slotText(prefs.headerSlots.right)}</span>
            </div>
          )}
          {hasSlot(prefs.footerSlots) && (
            <div className="reader-page-foot" aria-hidden="true" style={{ fontSize: prefs.hfFontSizePx }}>
              <span className="slot left">{slotText(prefs.footerSlots.left)}</span>
              <span className="slot center">{slotText(prefs.footerSlots.center)}</span>
              <span className="slot right">{slotText(prefs.footerSlots.right)}</span>
            </div>
          )}
          {/* 书本立体样式：spread 中缝阴影（纸页折缝层次） */}
          {prefs.flow === 'spread' && prefs.bookLook && stageWide && (
            <div className="book-gutter" aria-hidden="true" />
          )}
          {/* 点击触发 3D 纸页翻页层：旧页快照绕书脊 rotateY 翻走（perspective 投影）；
              spread 附静态底页（对面旧列）防 iframe 跳变穿帮；
              book=仿真纸页（时长自适应），flip=快速翻页（380ms）。 */}
          {flipSheet && (
            <PageFlipSheet
              state={flipSheet}
              stageWidth={epub.iframeRef.current?.clientWidth ?? 0}
              stageHeight={epub.iframeRef.current?.clientHeight ?? 0}
              layout={prefs.flow === 'spread' ? 'spread' : 'portrait'}
              paperColor={readerTheme?.bg}
              durationMs={prefs.pageTurnStyle === 'flip' ? 380 : undefined}
              onDone={finishFlip}
            />
          )}
        </div>

        <ReaderSidebar
          book={book}
          open={runtime.sidebarOpen}
          activeTab={runtime.activeTab}
          selectionText={runtime.selection?.text ?? null}
          noteDraft={runtime.noteDraft}
          readerState={runtime.readerState}
          aiAssistant={runtime.aiAssistant}
          onOpenWord={openWordFromSidebar}
          onClose={() => runtime.setSidebarOpen(false)}
          onChangeTab={runtime.setActiveTab}
          onChangeDraft={runtime.setNoteDraft}
          onCancelSelection={() => runtime.setSelection(null)}
          onSaveNote={(q, n) => runtime.saveNote(q, n, epub.location?.href ?? null)}
          onJumpToHref={(href, quote) => {
            epub.goTo(href);
            /* 跳转后定位引文元素并闪烁：章切换是异步的，
               轮询等待文档就绪（≤2s）；优先命中 mark.mt-mark，其次全文扫描引文。 */
            const needle = (quote ?? '').trim();
            let attempts = 0;
            const locate = () => {
              attempts += 1;
              const doc = epub.getDoc();
              if (!doc?.body) {
                if (attempts < 10) window.setTimeout(locate, 200);
                return;
              }
              let target: Element | null = null;
              if (needle) {
                target = Array.from(doc.body.querySelectorAll('mark.mt-mark'))
                  .find((el) => (el.textContent ?? '').trim() === needle) ?? null;
                if (!target) {
                  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
                  let node = walker.nextNode() as Text | null;
                  outer: while (node) {
                    if ((node.textContent ?? '').includes(needle.slice(0, Math.min(24, needle.length)))) {
                      target = node.parentElement;
                      break outer;
                    }
                    node = walker.nextNode() as Text | null;
                  }
                }
              }
              if (target) {
                (target as Element).scrollIntoView({ behavior: 'smooth', block: 'center' });
                target.classList.add('mt-flash');
                window.setTimeout(() => target?.classList.remove('mt-flash'), 2200);
                /* 复检（跳转不稳定）：章切换后的 annotate/字体回流会重置
                   滚动位置，600ms 后复扫一次，被拉回则重新定位。 */
                window.setTimeout(() => {
                  const doc2 = epub.getDoc();
                  if (!doc2?.body || !target.isConnected) return;
                  const targetRect = (target as Element).getBoundingClientRect();
                  const win = epub.iframeRef.current?.contentWindow;
                  if (!win) return;
                  if (targetRect.top < 0 || targetRect.bottom > win.innerHeight) {
                    (target as Element).scrollIntoView({ behavior: 'smooth', block: 'center' });
                  }
                }, 600);
              }
            };
            window.setTimeout(locate, 120);
          }}
          onRemoveBookmark={(id) => {
            /* 先摘正文划线再删数据（quote 从当前书签列表取） */
            const bm = runtime.readerState?.bookmarks?.find((b) => b.id === id);
            if (bm) runtime.removeBookmarkMark(bm.quote);
            void runtime.removeBookmark(id);
          }}
          onRemoveNote={(id) => void runtime.removeNote(id)}
        />
      </div>

      {/* 首次进入：分区手势提示（一次性，见 zoneHintShown） */}
      {!zoneHintShown && !chromeVisible && (
        <div className="zone-hint" onClick={() => setZoneHintShown(true)}>
          <span>{t.reader.zoneHintBar}</span>
          <span className="zone-hint-dismiss" aria-hidden="true">✕</span>
        </div>
      )}

      {/* 底栏：分页进度（随 chrome 显隐） */}
      <div className={cn('chrome bottom', chromeVisible && 'show')}>
        <button type="button" className="iconbtn" style={{ width: 38, height: 38 }} onClick={flipPrev} aria-label={t.reader.turnPrev}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="M14.5 5.5L8 12l6.5 6.5" /></svg>
        </button>
        <div
          ref={pgbarRef}
          className={cn('pgbar', pgDragRatio !== null && 'dragging')}
          role="slider"
          aria-label={t.reader.progressLabel}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round((pgDragRatio ?? epub.progress) * 100)}
          onPointerDown={handlePgPointerDown}
          onPointerMove={handlePgPointerMove}
          onPointerUp={handlePgPointerUp}
          onPointerCancel={handlePgPointerCancel}
        >
          <i style={{ width: `${(pgDragRatio ?? epub.progress) * 100}%` }} />
          <b style={{ left: `${(pgDragRatio ?? epub.progress) * 100}%` }} />
        </div>
        <span className="pglabel">
          {epub.pageInfo.page + 1} / {epub.pageInfo.total} · {Math.round(epub.progress * 100)}%
        </span>
        <button type="button" className="iconbtn" style={{ width: 38, height: 38 }} onClick={flipNext} aria-label={t.reader.turnNext}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="M9.5 5.5L16 12l-6.5 6.5" /></svg>
        </button>
        <button
          type="button"
          className={cn('iconbtn', runtime.sidebarOpen && 'on')}
          style={{ width: 38, height: 38 }}
          onClick={() => runtime.setSidebarOpen((v) => !v)}
          aria-label={t.reader.sidebar}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="M13 4.5h6.5v15H13z" /><path d="M4.5 4.5H13v15H4.5z" /></svg>
        </button>
      </div>

      {/* 划选小红点菜单 */}
      <AnimatePresence>
        {runtime.selection && (
          <div ref={selRef}>
            <ReaderSelectionMenu
              selection={runtime.selection}
              initialNoteOpen={markClickRef.current}
              autoTranslate={prefs.autoTranslateSelection}
              autoMark={prefs.autoMarkSelection}
              onClearMark={(q) => {
                /* 橡皮擦：摘除正文可视划线 + 删除该引文全部书签。 */
                runtime.removeBookmarkMark(q);
                const hits = (runtime.readerState?.bookmarks ?? []).filter((b) => b.quote === q);
                for (const b of hits) void runtime.removeBookmark(b.id);
              }}
              onSaveBookmark={(q, style) => {
                /* 色板格式 "style:color"：解析后带色应用可视划线 */
                const [kind, color] = style.split(':');
                if (kind === 'highlight' || kind === 'underline' || kind === 'wave') {
                  applyMark(kind as MarkStyle, color);
                }
                void runtime.saveBookmark(q, epub.location?.href ?? null, style);
              }}
              onSaveNote={(q, n) => void runtime.saveNote(q, n, epub.location?.href ?? null)}
              onAskAi={(text) => {
                runtime.openAiPanel(text);
                runtime.setSelection(null);
              }}
              onTranslate={async (text) => {
                const resp = await invoke<AiAssistResponse>('ai_assist_paragraph', {
                  action: 'translate',
                  sourceText: text,
                  question: null,
                  targetLanguage: null,
                  history: null,
                });
                return resp.answer;
              }}
              onAddToVocab={(text, translation, context) => void runtime.addVocabCard(text, context, translation)}
              onAddToNotebook={(text, definition, context) => setNotebookPick({ word: text, definition, context })}
              onDefine={async (context, word) => {
                /* 本地分级词库优先（离线秒回）；未收录再走 AI 语境义。 */
                try {
                  const local = await invoke<WordLevelInfo | null>('lookup_word_level', { word });
                  if (local?.definition?.trim()) {
                    return `${local.level} ${local.phonetic}\n${local.definition}`;
                  }
                } catch {
                  /* 本地查询失败继续 AI */
                }
                const resp = await invoke<AiAssistResponse>('ai_assist_paragraph', {
                  action: 'define',
                  sourceText: context,
                  question: word,
                  targetLanguage: null,
                  history: null,
                });
                return resp.answer;
              }}
              pageText={selectionPageText}
              onClose={() => runtime.setSelection(null)}
            />
          </div>
        )}
      </AnimatePresence>

      {/* 生词本选择弹窗 */}
      {notebookPick && (
        <NotebookPicker
          word={notebookPick.word}
          definition={notebookPick.definition}
          context={notebookPick.context}
          bookId={book.id}
          chapter={epub.chapterTitle}
          onDone={() => setNotebookPick(null)}
          onCancel={() => setNotebookPick(null)}
        />
      )}

      {/* 段落小红点动作菜单 */}
      <AnimatePresence>
        {paraDot && (
          <IframeParaMenu
            anchor={paraDot.anchor}
            placeAbove={paraDot.placeAbove}
            onAction={(action) => {
              const text = paraDot.paragraphText;
              setParaDot(null);
              if (action === 'ask') {
                runtime.openAiPanel(text);
              } else {
                runtime.handleParagraphAction(action, text);
              }
            }}
            onClose={() => setParaDot(null)}
          />
        )}
      </AnimatePresence>

      {/* WordWise 生词注释卡片 */}
      <AnimatePresence>
        {wordCard && (
          <WordWiseCard
            state={wordCard}
            mark={runtime.readerState?.wordMarks?.[wordCard.info.word]}
            onMarkWord={runtime.markWord}
            onAddCard={runtime.addVocabCard}
            onSaveDefinition={runtime.saveWordDefinition}
            onSpeak={(text) => void runtime.tts.speak(text)}
            ttsSpeaking={runtime.tts.speaking}
            onClose={() => { dismissOverlayStamp(); setWordCard(null); }}
          />
        )}
      </AnimatePresence>
    </div>
  );
}

/** 页脚时钟：HH:MM。 */
function formatClock(date: Date): string {
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

/** 英文栏纯文本：克隆摘除上标与中文节点后取 textContent（弹卡例句断句用）。 */
function englishTextOf(host: Element | null): string {
  if (!host) return '';
  const clone = host.cloneNode(true) as Element;
  clone.querySelectorAll('span.mt-ww-g, .mt-zh, .mt-ch').forEach((el) => el.remove());
  return clone.textContent ?? '';
}

/** 单击查词命中闪亮（非 WordWise 标注词的选中反馈）：
    词区间包一层临时 span（1s 淡出动画结束后自移除，还原纯文本）。
    与 WordWise 标注的 mt-ww-hit 动效同观感。 */
function flashHitRange(doc: Document, hit: { node: Text; word: string }): void {
  const text = hit.node.textContent ?? '';
  const lowered = text.toLowerCase();
  const word = hit.word.toLowerCase();
  const at = lowered.indexOf(word);
  if (at < 0) return;
  try {
    const range = doc.createRange();
    range.setStart(hit.node, at);
    range.setEnd(hit.node, at + word.length);
    const flash = doc.createElement('span');
    flash.className = 'mt-ww-flash';
    range.surroundContents(flash);
    window.setTimeout(() => {
      const parent = flash.parentNode;
      if (!parent) return;
      while (flash.firstChild) parent.insertBefore(flash.firstChild, flash);
      flash.remove();
      parent.normalize();
    }, 1000);
  } catch {
    /* 包裹失败（罕见嵌套）：跳过闪亮，不影响弹卡。 */
  }
}

/** 点击点所在英文词（caretRangeFromPoint 精确定位；限 3–20 字符的字母词）。
    长按/划选不触发：调用方已用 selection 守卫让位。
    点击点必须在词的实际渲染盒内（caret 邻近返回的词若不在点击位置
    视觉覆盖范围内则不命中——空白处/行距点击不弹卡，杜绝"跨栏认词"）。 */
function wordAtPoint(
  doc: Document,
  x: number,
  y: number,
): { word: string; rect: DOMRect; node: Text } | null {
  let caret: Range | null = null;
  if (doc.caretRangeFromPoint) {
    caret = doc.caretRangeFromPoint(x, y);
  } else if (doc.caretPositionFromPoint) {
    const pos = doc.caretPositionFromPoint(x, y);
    if (pos) {
      caret = doc.createRange();
      caret.setStart(pos.offsetNode, pos.offset);
      caret.setEnd(pos.offsetNode, pos.offset);
    }
  }
  const node = caret?.startContainer;
  if (!caret || node?.nodeType !== Node.TEXT_NODE) return null;
  /* 上标节点不参与单击查词：caretRangeFromPoint 无视 pointer-events，
     点到上标区域会定位进上标文本（英文档/音标档会误提取出词）。 */
  if ((node as Text).parentElement?.closest('.mt-ww-g')) return null;
  const text = node.textContent ?? '';
  const offset = caret.startOffset;
  const isWordChar = (ch: string) => /[A-Za-z']/.test(ch);
  if (offset < text.length && !isWordChar(text[offset]) && !(offset > 0 && isWordChar(text[offset - 1]))) return null;
  let start = offset;
  while (start > 0 && isWordChar(text[start - 1])) start -= 1;
  let end = offset;
  while (end < text.length && isWordChar(text[end])) end += 1;
  const word = text.slice(start, end).replace(/^'+|'+$/g, '');
  if (word.length < 3 || word.length > 20) return null;
  const probe = doc.createRange();
  probe.setStart(node, start);
  probe.setEnd(node, end);
  const rect = probe.getBoundingClientRect();
  /* 视觉覆盖判定：点击点落在词盒内（横向严格、纵向放宽行盒高度，
     允许命中带上下标延伸的行内区域）。 */
  const inBox = x >= rect.left - 1 && x <= rect.right + 1 && y >= rect.top - 2 && y <= rect.bottom + 2;
  if (!inBox) return null;
  return { word: word.toLowerCase(), rect, node: node as Text };
}

/** 本地路径 → asset URL（非 Tauri 环境或转换失败回退原值）。 */
function safeAssetUrl(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

/** 三槽位中是否配置了任一非空内容。 */
function hasSlot(slots: { left: ReaderInfoSlot; center: ReaderInfoSlot; right: ReaderInfoSlot }): boolean {
  return slots.left !== 'none' || slots.center !== 'none' || slots.right !== 'none';
}
