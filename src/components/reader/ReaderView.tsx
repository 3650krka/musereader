import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Book, ReaderState, Theme, WordMark } from '@/types';
import { useAiAssistant } from './useAiAssistant';
import { useDocumentReader } from './useDocumentReader';
import { useEpubReader } from './useEpubReader';
import { loadLangForBook, saveLangForBook, useReaderPrefs } from './useReaderPrefs';
import { useReaderRuntime } from './useReaderRuntime';
import { useReaderTts } from './useReaderTts';
import { EpubReaderView } from './EpubReaderView';
import type { ParagraphAction } from './useAiAssistant';
import type { ReaderFrameApi, ReaderTheme } from './epubReaderTypes';
import {
  annotateWordWise,
  extractSentenceContext,
  patchWordWiseMarks,
  relieveWordWiseCollisions,
  type WordLevelInfo,
  type WordWiseAnnotateOptions,
} from './wordWiseInject';

const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** 阅读背景主题色板。 */
const THEME_MAP: Record<string, ReaderTheme> = {
  paper: { bg: '#f6f2ea', fg: '#23211d', muted: '#8a847a', enFg: '#6f6a61' },
  light: { bg: '#ffffff', fg: '#1c1c1a', muted: '#8d8d89', enFg: '#6a6a66' },
  sepia: { bg: '#e9e6e0', fg: '#2a2825', muted: '#8f8b84', enFg: '#716d67' },
  green: { bg: '#dfeadf', fg: '#25301f', muted: '#82907c', enFg: '#5c6a55' },
  parchment: { bg: '#f3e7c9', fg: '#3d2f1e', muted: '#97866a', enFg: '#7a6a50' },
  dark: { bg: '#17181a', fg: '#d8d5cf', muted: '#7c7f84', enFg: '#9a9790' },
};

/** 自定义主题：用户背景/文字色，muted/enFg 由 fg 降透明派生。 */
function customReaderTheme(bg: string, fg: string): ReaderTheme {
  return { bg, fg, muted: withAlpha(fg, 0.55), enFg: withAlpha(fg, 0.7) };
}

function withAlpha(hex: string, alpha: number): string {
  const channel = Math.round(Math.min(1, Math.max(0, alpha)) * 255)
    .toString(16)
    .padStart(2, '0');
  return `${hex}${channel}`;
}

/** 判断该书是否具备可由 EPUB 渲染层消费的电子书工件。 */
export function hasEpubArtifact(book: Book): boolean {
  const path = book.primaryArtifactPath ?? book.translatedEpubPath ?? book.originalPath ?? '';
  return /\.epub$/i.test(path);
}

interface ReaderViewProps {
  book: Book;
  onBack: () => void;
  currentTheme: Theme;
  setTheme: (theme: Theme) => void;
  /** 外部指定跳转锚点（笔记/书签跳书；消费后回调清除）。 */
  pendingAnchor?: string | null;
  onPendingAnchorConsumed?: () => void;
  /** 打开该书的校对工作台（可选：未提供则顶栏不显示校对入口）。 */
  onOpenReview?: () => void;
  /** 设置页「阅读」面板全局默认：首次加载偏好时的基线。 */
  readingDefaults?: import('./useReaderPrefs').GlobalReadingDefaults;
}

/**
 * 沉浸阅读器入口：EPUB 渲染层（真实 XHTML + 横向分页 + 非遮挡侧栏）。
 * 预览环境（无 Tauri）不渲染阅读舞台——书库预览卡片不进入阅读视图。
 */
export function ReaderView({ book, onBack, pendingAnchor, onPendingAnchorConsumed, onOpenReview, readingDefaults }: ReaderViewProps) {
  const tts = useReaderTts();
  const aiAssistant = useAiAssistant();
  const runtime = useReaderRuntime(book, { getDoc: () => readerRefForRuntime.current?.getDoc() ?? null });
  /* 桥接：runtime（先创建）需要 reader.getDoc（后创建），经 ref 延迟取用。 */
  const readerRefForRuntime = useRef<ReaderFrameApi | null>(null);
  const { prefs, update: onPrefsChange } = useReaderPrefs(readingDefaults);
  const epubEnabled = IS_TAURI && hasEpubArtifact(book);
  /* 提词黑白名单：只取用户**显式标记**（wordMarks：learning 强制标注 / mastered 抑制）。
     不再把整本 vocabCards 无差别当 learning——否则自动播种的词会永久强制提词，
     架空难度档过滤（选雅思却显示简单词的根因）。生词本仅作 SRS 学习池；
     高于当前档的词本就由后端 lookup 按 entry.level > userLevel 自动标注。 */
  const wordMarks = useMemo(() => runtime.readerState?.wordMarks ?? {},
    [runtime.readerState?.wordMarks],
  );
  const wordInfoMapRef = useRef<Map<string, WordLevelInfo>>(new Map());
  /* WordWise 标注输入（全量重标与局部补丁共用同一份选项）。 */
  const wordWiseOptions = useMemo<WordWiseAnnotateOptions>(() => ({
    enabled: prefs.wordWise,
    userLevel: prefs.userLevel,
    style: prefs.wordWiseStyle,
    gloss: prefs.wordWiseGloss,
    glossSize: prefs.wordWiseSize,
    lineBoost: prefs.wordWiseLineBoost,
    glossGapPx: prefs.wordWiseGapPx,
    accentColor: prefs.wordWiseColor,
    wordMarks,
  }), [prefs.wordWise, prefs.userLevel, prefs.wordWiseStyle, prefs.wordWiseGloss, prefs.wordWiseSize, prefs.wordWiseLineBoost, prefs.wordWiseGapPx, prefs.wordWiseColor, wordMarks]);
  /* WordWise 行内标注钩子：依赖偏好与黑白名单变化时重标整节（幂等）。
     标注完成后把命中的超档词自动播种入本书生词库（蒙哥式透析语义，
     用户选定难度及以上 = 词汇透析全集）。 */
  const wordWiseAnnotator = useCallback(
    async (doc: Document) => {
      const infos = await annotateWordWise(doc, wordWiseOptions);
      /* 查询失败（null）：保留旧词表与既有标注——瞬时失败绝不抹掉正在
         显示的标注（「全部消失、再点又回来」的根源），也不触发播种。 */
      if (!infos) return;
      wordInfoMapRef.current = infos;
      if (infos.size === 0) return;
      const known = runtime.readerState?.vocabCards;
      const chapter = chapterTitleRef.current;
      const seeds: { word: string; context: string; chapter: string }[] = [];
      for (const mark of Array.from(doc.querySelectorAll('span.mt-ww'))) {
        const surface = mark.getAttribute('data-word') ?? '';
        /* 以原形播种（后端词卡主键为原形）：屈折形（abandoned）与原形
           （abandon）共用同一条生词记录——否则每个形态各播一条，且
           「新播种 → 状态变化 → 重标 → 再播种」死循环致界面反复闪烁。 */
        const word = wordInfoMapRef.current.get(surface.toLowerCase())?.word ?? surface;
        if (!word || known?.[word]) continue;
        const block = mark.closest('p,li,blockquote,h1,h2,h3,h4,h5,h6,figcaption');
        /* 例句只取英文栏纯词形文本（摘除上标与中文节点，防断句污染）。 */
        const enHost = mark.closest('.mt-en,.mt-en-h') ?? block;
        const clone = (enHost ?? mark).cloneNode(true) as Element;
        clone.querySelectorAll('span.mt-ww-g, .mt-zh, .mt-ch').forEach((el) => el.remove());
        const blockText = (clone.textContent ?? '').trim();
        const context = extractSentenceContext(blockText, word) ?? blockText.slice(0, 300);
        seeds.push({ word, context, chapter });
      }
      if (seeds.length > 0) {
        invoke<ReaderState>('sync_wordwise_vocab', { bookId: book.id, seeds })
          .then((next) => runtime.setReaderState(next))
          .catch((error) => console.warn('WordWise vocab sync failed:', error));
      }
    },
    [wordWiseOptions, runtime.readerState?.vocabCards, book.id],
  );
  /* WordWise 碰撞自愈：每次分页/排版应用后实测（上标 absolute 定位不随重排移动，
     但同行分组与块首越顶量随断行变化）。字体晚到引发的度量变化由
     useEpubReader 的 fonts.ready → reflowPreservingLocation 全链路重排覆盖，
     此处不重复测量。返回是否修补了流布局（供分页层决定是否重排一次）。
     flow 决定分页边界语义：横向分页按栏（页）核算词距与页界——双页展开
     下跨页上标被页缘收拢收回页内，杜绝上标落入中缝/下一页内容侵上一页；
     滚动模式仅块内行级核算。 */
  const wordWiseGapPx = prefs.wordWiseGapPx;
  const wordWiseFlow = prefs.flow;
  const wordWiseRelayout = useCallback(
    (doc: Document): boolean => relieveWordWiseCollisions(doc, wordWiseGapPx, wordWiseFlow),
    [wordWiseGapPx, wordWiseFlow],
  );
  const epub = useEpubReader(book.id, epubEnabled, { annotate: wordWiseAnnotator, postFlow: wordWiseRelayout });
  /* 标注输入指纹分两级：
     · 结构键（偏好/档位/外观）——任一变化必须全量重标（命中集合与样式全变）；
     · 内容指纹（词+状态+语境义，排序拼接防后端键序漂移）——后端状态回传
       （活动心跳/成卡回写）每次都产生新对象身份，内容不变则不触发任何重标。 */
  const annotatePrefsKey = useMemo(() => `${prefs.wordWise}|${prefs.userLevel}|${prefs.wordWiseStyle}|${prefs.wordWiseGloss}|${prefs.wordWiseSize}|${prefs.wordWiseLineBoost}|${prefs.wordWiseGapPx}|${prefs.wordWiseColor}`,
    [prefs.wordWise, prefs.userLevel, prefs.wordWiseStyle, prefs.wordWiseGloss, prefs.wordWiseSize, prefs.wordWiseLineBoost, prefs.wordWiseGapPx, prefs.wordWiseColor],
  );
  const annotateFingerprint = useMemo(() => {
    const words = Object.keys(wordMarks).sort();
    let fingerprint = '';
    for (const word of words) {
      const mark = wordMarks[word];
      fingerprint += `${word}:${mark.status}:${mark.customDefinition ?? ''};`;
    }
    return `${annotatePrefsKey}|${fingerprint}`;
  }, [annotatePrefsKey, wordMarks]);
  const epubReannotateRef = useRef(epub.reannotate);
  epubReannotateRef.current = epub.reannotate;
  /* 生词局部补丁入口：只摘除/重包受影响词的标注外壳，不重分页、不重测
     全局碰撞修补、阅读位置绝对不动——「加生词/标已掌握后页面莫名跳转、
     双页卡在中间」的根治路径。补丁后的新标注可能改变换行与词距，
     下一帧局部复测，报告流布局变化时才重分页吸收（保位同按页号恢复）。 */
  const patchMarksRef = useRef<(removed: string[], added: { word: string; mark: WordMark }[]) => Promise<void>>(
    async () => undefined,
  );
  patchMarksRef.current = async (removed, added) => {
    const doc = epub.docSnapshot;
    if (!doc || removed.length + added.length === 0) return;
    const outcome = await patchWordWiseMarks(doc, { removed, added }, wordWiseOptions, wordInfoMapRef.current);
    if (!outcome) return;
    wordInfoMapRef.current = outcome.infoMap;
    /* 宿主 realm 帧调度：iframe 沙箱无 allow-scripts，其窗口帧回调不可依赖。 */
    window.requestAnimationFrame(() => {
      if (epub.getDoc() !== doc) return;
      if (wordWiseRelayout(doc)) epub.relayout();
    });
  };

  /* 打开书面整本 WordWise 种子：后台扫描全书超档词一次补齐生词库，不再依赖
     逐页翻阅积累；**按 书+难度档 为键**——改档后再进书自动重播（幂等，只补新增），
     背诵队列/生词统计随之更新；行内 DOM 标注本身仍按章渲染（技术边界）。 */
  const wholeSeedDoneRef = useRef<Set<string>>(new Set);
  useEffect(() => {
    const seedKey = `${book.id}|${prefs.userLevel}`;
    if (!epubEnabled || wholeSeedDoneRef.current.has(seedKey)) return;
    wholeSeedDoneRef.current.add(seedKey);
    invoke<ReaderState>('seed_book_vocab_all', { bookId: book.id, userLevel: prefs.userLevel })
      .then((next) => runtime.setReaderState(next))
      .catch((error) => {
        wholeSeedDoneRef.current.delete(seedKey);
        console.warn('整本 WordWise 种子失败：', error);
      });
  }, [epubEnabled, prefs.userLevel, book.id, runtime]);
  /* 标注输入变化 → 按变化性质分流：
     · 首次/偏好档位变化/补丁在途 → 全量重标（reannotate，页号保位）；
     · 仅词表内容变化（加生词/标已掌握/删词/存语境义）→ 局部补丁，
       版面其余部分逐像素不动。
     章节加载时 onIframeLoad 已用最新输入标注；此处覆盖同一章内的即时生效。 */
  const prevAnnotateInputRef = useRef<{ prefsKey: string; content: string } | null>(null);
  const prevWordContentsRef = useRef<Map<string, string>>(new Map);
  const annotatePatchBusyRef = useRef(false);
  useEffect(() => {
    if (!epubEnabled) return;
    const prev = prevAnnotateInputRef.current;
    const prevWords = prevWordContentsRef.current;
    const nextWords = new Map<string, string>();
    for (const [word, mark] of Object.entries(wordMarks)) {
      nextWords.set(word, `${mark.status}:${mark.customDefinition ?? ''}`);
    }
    prevAnnotateInputRef.current = { prefsKey: annotatePrefsKey, content: annotateFingerprint };
    prevWordContentsRef.current = nextWords;
    /* 内容指纹未变（身份抖动收敛）或首次渲染（章节加载自行标注）→ 不动。 */
    if (!prev || prev.content === annotateFingerprint) return;
    if (annotatePatchBusyRef.current || prev.prefsKey !== annotatePrefsKey) {
      epubReannotateRef.current?.();
      return;
    }
    const removed: string[] = [];
    const added: { word: string; mark: WordMark }[] = [];
    for (const [word, content] of prevWords) {
      const next = nextWords.get(word);
      if (next === undefined) removed.push(word);
      else if (next !== content) added.push({ word, mark: wordMarks[word] });
    }
    for (const word of nextWords.keys()) {
      if (!prevWords.has(word)) added.push({ word, mark: wordMarks[word] });
    }
    if (removed.length === 0 && added.length === 0) return;
    annotatePatchBusyRef.current = true;
    void patchMarksRef.current(removed, added).finally(() => {
      annotatePatchBusyRef.current = false;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [epubEnabled, annotateFingerprint]);
  /* 章节标题最新值：自动成卡播种时取章节名（避免 useCallback 闭包过期）。 */
  const chapterTitleRef = useRef('');
  chapterTitleRef.current = epub.chapterTitle;
  /* 跟随系统深浅色（anx autoAdjustReadingTheme 对齐）：开启时系统深色自动切 dark。 */
  const [systemDark, setSystemDark] = useState(
    () => typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches,
  );
  useEffect(() => {
    if (!prefs.autoTheme) return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, [prefs.autoTheme]);
  /* 当前主题色板：命名主题或自定义主题（bg/fg 用户可编辑） */
  const readerTheme = useMemo(() => {
    if (prefs.autoTheme && systemDark && prefs.themeName !== 'custom') return THEME_MAP.dark ?? null;
    return prefs.themeName === 'custom'
      ? customReaderTheme(prefs.customThemeBg, prefs.customThemeFg)
      : THEME_MAP[prefs.themeName] ?? null;
  }, [prefs.themeName, prefs.customThemeBg, prefs.customThemeFg, prefs.autoTheme, systemDark]);
  const customCss = prefs.customCssEnabled ? prefs.customCss : '';
  const documentReader = useDocumentReader(book, IS_TAURI && !epubEnabled, {
    lang: prefs.lang,
    flow: prefs.flow,
    pageWidth: prefs.pageWidth,
    translationBlur: prefs.translationBlur,
    fontSize: prefs.fontSizeEm,
    lineHeight: prefs.lineHeight,
    paraSpacing: prefs.paragraphSpacingEm,
    theme: readerTheme,
    typeExtra: prefs.typeExtra,
    fontFamily: prefs.fontFamily,
    customFontPath: prefs.customFontPath,
    enFontFamily: prefs.enFontFamily,
    customEnFontPath: prefs.customEnFontPath,
    sideMarginPercent: prefs.sideMarginPercent,
    customCss,
    brightnessPercent: prefs.brightnessPercent,
    paragraphGapPx: prefs.paragraphGapPx,
    topMarginPx: prefs.topMarginPx,
    bottomMarginPx: prefs.bottomMarginPx,
  });
  const reader: ReaderFrameApi = epubEnabled ? epub : documentReader;
  readerRefForRuntime.current = reader;
  const {
    setLang,
    setFlow,
    setPageWidth,
    setTranslationBlur,
    setFontSize,
    setLineHeight,
    setParaSpacing,
    setTheme,
    setTypeExtra,
    setFontFamily,
    setPageTurnStyle,
    setSideMarginPercent,
    setCustomCss,
    setBrightnessPercent,
    setParagraphGapPx,
  } = epub;

  /* 偏好 → EPUB 渲染层排版注入 */
  useEffect(() => {
    setLang(prefs.lang);
  }, [prefs.lang, setLang]);
  useEffect(() => {
    setFlow(prefs.flow);
  }, [prefs.flow, setFlow]);
  useEffect(() => {
    setPageWidth(prefs.pageWidth);
  }, [prefs.pageWidth, setPageWidth]);
  useEffect(() => {
    setTranslationBlur(prefs.translationBlur);
  }, [prefs.translationBlur, setTranslationBlur]);
  useEffect(() => {
    setFontSize(prefs.fontSizeEm);
  }, [prefs.fontSizeEm, setFontSize]);
  useEffect(() => {
    setLineHeight(prefs.lineHeight);
  }, [prefs.lineHeight, setLineHeight]);
  useEffect(() => {
    setParaSpacing(prefs.paragraphSpacingEm);
  }, [prefs.paragraphSpacingEm, setParaSpacing]);
  useEffect(() => {
    setTheme(readerTheme);
  }, [readerTheme, setTheme]);
  useEffect(() => {
    setTypeExtra(prefs.typeExtra);
  }, [prefs.typeExtra, setTypeExtra]);
  useEffect(() => {
    setFontFamily(prefs.fontFamily);
  }, [prefs.fontFamily, setFontFamily]);
  useEffect(() => {
    epub.setCustomFontPath(prefs.customFontPath);
  }, [prefs.customFontPath, epub]);
  useEffect(() => {
    epub.setEnFontFamily(prefs.enFontFamily);
  }, [prefs.enFontFamily, epub]);
  useEffect(() => {
    epub.setCustomEnFontPath(prefs.customEnFontPath);
  }, [prefs.customEnFontPath, epub]);
  useEffect(() => {
    setPageTurnStyle(prefs.pageTurnStyle);
  }, [prefs.pageTurnStyle, setPageTurnStyle]);
  useEffect(() => {
    setSideMarginPercent(prefs.sideMarginPercent);
  }, [prefs.sideMarginPercent, setSideMarginPercent]);
  useEffect(() => {
    setCustomCss(customCss);
  }, [customCss, setCustomCss]);
  useEffect(() => {
    setBrightnessPercent(prefs.brightnessPercent);
  }, [prefs.brightnessPercent, setBrightnessPercent]);
  useEffect(() => {
    setParagraphGapPx(prefs.paragraphGapPx);
  }, [prefs.paragraphGapPx, setParagraphGapPx]);
  useEffect(() => {
    epub.setUseBookStyles(prefs.useBookStyles);
  }, [prefs.useBookStyles, epub]);
  useEffect(() => {
    epub.setColumnCount(prefs.columnCount);
  }, [prefs.columnCount, epub]);
  useEffect(() => {
    epub.setColumnThresholdPx(prefs.columnThresholdPx);
  }, [prefs.columnThresholdPx, epub]);
  useEffect(() => {
    epub.setTopMarginPx(prefs.topMarginPx);
  }, [prefs.topMarginPx, epub]);
  useEffect(() => {
    epub.setBottomMarginPx(prefs.bottomMarginPx);
  }, [prefs.bottomMarginPx, epub]);

  /** 当前页可见文本（供 TTS 朗读与本页生词透析）。 */
  const extractPageText = useCallback((): string => {
    const doc = reader.getDoc();
    const body = doc?.body;
    if (!body) return '';
    const win = reader.iframeRef.current?.contentWindow;
    const vw = reader.iframeRef.current?.clientWidth ?? 0;
    // 分页模式（单页/双页）：按横向滚动位置收集与当前视口交叠的块元素文本
    if (prefs.flow !== 'scrolled' && win && vw > 0) {
      const page = Math.floor((win.scrollX + 1) / vw);
      const left = page * vw;
      const right = left + vw;
      const parts: string[] = [];
      for (const el of Array.from(body.querySelectorAll('p, h1, h2, h3, h4, h5, h6, li, blockquote'))) {
        const r = el.getBoundingClientRect();
        const docLeft = r.left + win.scrollX;
        if (docLeft < right && docLeft + r.width > left) {
          const t = (el as HTMLElement).innerText?.trim();
          if (t) parts.push(t);
        }
        if (parts.join('\n\n').length > 3800) break;
      }
      if (parts.length > 0) return parts.join('\n\n').slice(0, 4000);
    }
    const text = body.innerText ?? '';
    return text.replace(/\n{3,}/g, '\n\n').trim().slice(0, 4000);
  }, [prefs.flow, reader.getDoc, reader.iframeRef]);

  /* 语言模式按书记忆：
     开书恢复该书上次模式；切换时持久化。 */
  const langBookRef = useRef<string | null>(null);
  useEffect(() => {
    if (langBookRef.current === book.id) return;
    langBookRef.current = book.id;
    const remembered = loadLangForBook(book.id);
    if (remembered && remembered !== prefs.lang) onPrefsChange({ lang: remembered });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [book.id]);
  useEffect(() => {
    saveLangForBook(book.id, prefs.lang);
  }, [book.id, prefs.lang]);

  /* 依赖展开到叶子字段：runtime 对象本身每渲染必新（hook 返回字面量），
     以它做依赖等于不 memo。回调字段均为 useCallback 稳定引用，状态字段
     才是真正变化的输入——展开后 bundle 仅在状态真实变化时重建。 */
  const runtimeBundle = useMemo(
    () => ({
      readerState: runtime.readerState,
      activeTab: runtime.activeTab,
      setActiveTab: runtime.setActiveTab,
      sidebarOpen: runtime.sidebarOpen,
      setSidebarOpen: runtime.setSidebarOpen,
      selection: runtime.selection,
      setSelection: runtime.setSelection,
      noteDraft: runtime.noteDraft,
      setNoteDraft: runtime.setNoteDraft,
      openNotes: runtime.openNotes,
      openToc: runtime.openToc,
      openSearch: runtime.openSearch,
      saveBookmark: (q: string, href?: string | null, style?: string) => void runtime.saveBookmark(q, href, style),
      removeBookmark: runtime.removeBookmark,
      removeBookmarkMark: runtime.removeBookmarkMark,
      removeNote: runtime.removeNote,
      reviewMode: runtime.reviewMode,
      toggleReviewMode: runtime.toggleReviewMode,
      saveNote: (q: string, n: string, href?: string | null) => void runtime.saveNote(q, n, href),
      markWord: runtime.markWord,
      addVocabCard: runtime.addVocabCard,
      saveWordDefinition: runtime.saveWordDefinition,
      openAiPanel: (quote?: string) => {
        if (quote?.trim()) aiAssistant.attachQuote(quote);
        runtime.setSidebarOpen(true);
        runtime.setActiveTab('ai');
      },
      handleParagraphAction: (action: ParagraphAction, paragraph: string) => {
        runtime.setSidebarOpen(true);
        runtime.setActiveTab('ai');
        void aiAssistant.runParagraphAction(action, paragraph);
      },
      aiAssistant,
      tts,
    }),
    [
      runtime.readerState, runtime.activeTab, runtime.setActiveTab,
      runtime.sidebarOpen, runtime.setSidebarOpen,
      runtime.selection, runtime.setSelection,
      runtime.noteDraft, runtime.setNoteDraft,
      runtime.openNotes, runtime.openToc, runtime.openSearch,
      runtime.saveBookmark, runtime.removeBookmark, runtime.removeBookmarkMark, runtime.removeNote,
      runtime.reviewMode, runtime.toggleReviewMode,
      runtime.saveNote, runtime.markWord, runtime.addVocabCard, runtime.saveWordDefinition,
      aiAssistant, tts,
    ],
  );

  /* 打开书籍时恢复上次阅读位置：优先精确锚点（href+offset），退回全书进度比例。
     等首帧渲染（docSnapshot）与阅读状态加载完成后再跳，避免空文档上跳转丢失。 */
  const restoredBookRef = useRef<string | null>(null);
  useEffect(() => {
    if (restoredBookRef.current === book.id) return;
    if (!reader.docSnapshot || !runtime.readerState) return;
    restoredBookRef.current = book.id;
    /* 外部锚点优先（笔记/书签跳书），其次上次阅读位置。 */
    if (pendingAnchor) {
      if (reader.goToAnchor) reader.goToAnchor(pendingAnchor, 0);
      else reader.goTo(pendingAnchor);
      onPendingAnchorConsumed?.();
      return;
    }
    const { href, offset, progress } = runtime.readerState;
    if (epubEnabled && href) {
      if (reader.goToAnchor) reader.goToAnchor(href, offset ?? 0);
      else reader.goTo(href);
    } else if ((progress ?? 0) > 0) {
      reader.goToProgress(progress / 100);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    book.id,
    epubEnabled,
    reader.docSnapshot,
    reader.goTo,
    reader.goToAnchor,
    reader.goToProgress,
    runtime.readerState,
    pendingAnchor,
    onPendingAnchorConsumed,
  ]);

  if (!IS_TAURI) {
    return null;
  }

  return (
    <EpubReaderView
      book={book}
      onBack={onBack}
      readerTheme={readerTheme}
      runtime={runtimeBundle}
      epub={reader}
      prefs={prefs}
      onPrefsChange={onPrefsChange}
      onLocationChange={(loc, chapter, ratio) => {
        runtime.setReadingPosition(chapter, ratio * 100, loc.href, loc.offset);
      }}
      extractPageText={extractPageText}
      wordInfoMapRef={wordInfoMapRef}
      onOpenReview={onOpenReview}
    />
  );
}
