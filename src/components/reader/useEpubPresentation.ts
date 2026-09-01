import { useCallback, useRef } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import type { ReaderFontFamily, ReaderLang, ReaderTheme, ReaderTypeExtra } from './epubReaderTypes';
import { markCss } from './useTextMarks';

interface EpubPresentationOptions {
  getDoc: () => Document | null;
  hasBilingualMarkup: boolean;
  lang: ReaderLang;
  fontSize: number;
  lineHeight: number;
  paraSpacing: number;
  theme: ReaderTheme | null;
  /** 主动回忆：双语模式下译文（.mt-zh/.mt-ch）默认模糊，悬停/点按显示。 */
  translationBlur?: boolean;
  /** 细粒度排版（字距/词距/字重/缩进/标题缩放/对齐）。 */
  typeExtra?: ReaderTypeExtra;
  /** 正文字体偏好（中文主导：译文栏与无标记正文）。 */
  fontFamily?: ReaderFontFamily;
  /** 英文字体偏好（问题：中英文分开设置——双语书英文栏 .mt-en；未双语原书整体）。 */
  enFontFamily?: ReaderFontFamily;
  /** 英文自定义字体文件路径（enFontFamily='custom' 时生效）。 */
  customEnFontPath?: string;
  /** 用户自定义 CSS：启用时原文追加注入正文文档。 */
  customCss?: string;
  /** 亮度百分比（0–100，通过 CSS filter 模拟；100 为不处理）。 */
  brightnessPercent?: number;
  /** 段间距 px 下限（0=仅用段距 em；>0 时与段距取较大者，不叠加）。 */
  paragraphGapPx?: number;
  /** 使用书籍内置样式（关闭排版覆盖，仅保留主题色）。 */
  useBookStyles?: boolean;
  /** 栏数（0=自动：宽屏双栏；1/2 固定）。 */
  columnCount?: number;
  /** 自动分栏阈值 px（columnCount=0 时生效；视口超过才双栏）。 */
  columnThresholdPx?: number;
  /** 上/下页边距 px（0–200；注入 body padding，随分页自然生效）。 */
  topMarginPx?: number;
  bottomMarginPx?: number;
  /** 自定义字体文件绝对路径（fontFamily='custom' 时生效，asset 协议转 URL）。 */
  customFontPath?: string;
}

const DUAL_CLASS = 'mt-reader-dual';

const CUSTOM_FONT_NAME = 'MTCustomFont';
const CUSTOM_EN_FONT_NAME = 'MTCustomEnFont';

/** 自定义字体 @font-face：asset 协议 URL + 按扩展映射 format（name 参数支持中/英双自定义字体）。 */
function customFontFace(path: string, name = CUSTOM_FONT_NAME): string {
  let url: string;
  try {
    url = convertFileSrc(path);
  } catch {
    return '';
  }
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  const format =
    ext === 'woff2' ? 'woff2' : ext === 'woff' ? 'woff' : ext === 'otf' ? 'opentype' : 'truetype';
  return `@font-face{font-family:'${name}';src:url('${url}') format('${format}');font-display:swap;}`;
}

const FONT_STACK: Record<Exclude<ReaderFontFamily, 'auto' | 'custom'>, string> = {
  serif: '"Source Han Serif SC","Noto Serif CJK SC","Songti SC",Georgia,serif',
  sans: '"Source Han Sans SC","Noto Sans CJK SC","PingFang SC","Microsoft YaHei",sans-serif',
};

/** 英文专用栈（中文栈前置系统 serif/sans 拉丁字形，中文缺失时天然回退）。 */
const EN_FONT_STACK: Record<Exclude<ReaderFontFamily, 'auto' | 'custom'>, string> = {
  serif: 'Georgia,"Times New Roman",serif',
  sans: '"Helvetica Neue",Arial,"Segoe UI",sans-serif',
};

/** 字体偏好 → font-family 声明（custom 时引用 @font-face 名）。
    fallbackStack：拉丁字体栈之后的 CJK 兜底（英文宽作用域命中中文段落时不丢中文字体）。 */
function fontFamilyRuleOf(
  pref: ReaderFontFamily,
  customPath: string,
  stacks: typeof FONT_STACK,
  customName: string,
  fallbackStack = '',
): { rule: string; face: string } {
  if (pref === 'auto') return { rule: '', face: '' };
  if (pref === 'custom') {
    if (!customPath) return { rule: '', face: '' };
    const list = fallbackStack ? `'${customName}',${fallbackStack}` : `'${customName}'`;
    return { rule: `font-family:${list}!important;`, face: customFontFace(customPath, customName) };
  }
  const list = fallbackStack ? `${stacks[pref]},${fallbackStack}` : stacks[pref];
  return { rule: `font-family:${list}!important;`, face: '' };
}

export function useEpubPresentation(options: EpubPresentationOptions) {
  const styleRef = useRef<HTMLStyleElement | null>(null);
  const revealDocRef = useRef<Document | null>(null);
  const {
    getDoc,
    hasBilingualMarkup,
    lang,
    fontSize,
    lineHeight,
    paraSpacing,
    theme,
    translationBlur = false,
    typeExtra,
    fontFamily = 'auto',
    enFontFamily = 'auto',
    customCss = '',
    brightnessPercent = 100,
    paragraphGapPx = 0,
    useBookStyles = false,
    columnCount = 1,
    columnThresholdPx = 900,
    topMarginPx = 0,
    bottomMarginPx = 0,
    customFontPath = '',
    customEnFontPath = '',
  } = options;

  const buildReaderCss = useCallback(() => {
    /* 单语模式隐藏另一语种后，配对段内的分隔 <br> 会残留空行：
       英语模式=段首空行（缩进被空行吃掉）；中文模式=段尾空行（抬高段高）。
       连同其后紧跟的第二个 <br>（列表/容器配对的连续空行形态）一并隐藏。 */
    const brSeparatorRule =
      '.mt-zh+br,.mt-ch+br,.mt-en+br,.mt-en-h+br,.mt-zh+br+br,.mt-ch+br+br,.mt-en+br+br,.mt-en-h+br+br{display:none!important}';
    /* display:revert 而非 initial：回退到 UA 默认（span→inline、p→block），
       同时撤销书籍自带样式表对 .mt-en 的 display 设定（中文版打包书
       .mt-en{display:none} 在双语展开时需解除隐藏；flat 兜底书的
       p.mt-col.mt-en 保持块级不被压成行内）。 */
    const languageRule =
      lang === 'zh'
        ? `.mt-en,.mt-en-h{display:none!important}${brSeparatorRule}`
        : lang === 'en'
          ? `.mt-zh,.mt-ch{display:none!important}${brSeparatorRule}`
          : '.mt-en,.mt-en-h{display:revert}';
    const bilingual = hasBilingualMarkup && lang === 'both';
    /* 亮度：降为灰度滤镜会损失色彩，仅用 brightness()（保留色相）。 */
    const brightnessRule =
      brightnessPercent < 100 ? `filter:brightness(${(brightnessPercent / 100).toFixed(2)});` : '';
    /* 书籍内置样式：跳过段级排版覆盖（段距/缩进/对齐/标题缩放/细节字距），
       保留字号/行高/字体/主题色与分栏（字号行高保留是为稳定 rem 基准）。 */
    const skipType = useBookStyles;
    /* 中英文字体分开：
       body 应用中文字体（译文栏/中文正文）；英文栏 .mt-en 与原文书英文正文
       再叠英文专用栈（enFontFamily 仅在显式选择时覆盖）。 */
    const zh = fontFamilyRuleOf(fontFamily, customFontPath, FONT_STACK, CUSTOM_FONT_NAME);
    /* 英文 auto 的效果一致语义：中文已显式选 serif/sans 时，英文自动用同风格
       英文优化栈（Georgia/Helvetica 系），避免被中文字体的拉丁字形渲染；
       中文 auto 或 custom 时英文跟随（不改变书籍原字体）。 */
    const enEffective =
      enFontFamily !== 'auto'
        ? enFontFamily
        : fontFamily === 'serif' || fontFamily === 'sans'
          ? fontFamily
          : 'auto';
    /* 英文栈后的 CJK 兜底：中文显式选字体时接同一栈；中文 auto 时英文栈自带
       serif/sans 通用族即可兜底（不引入额外字体，避免覆盖原书 CJK 选择）。 */
    const zhFallback =
      fontFamily === 'serif' || fontFamily === 'sans'
        ? FONT_STACK[fontFamily]
        : fontFamily === 'custom' && customFontPath
          ? `'${CUSTOM_FONT_NAME}'`
          : '';
    const en = fontFamilyRuleOf(enEffective, customEnFontPath, EN_FONT_STACK, CUSTOM_EN_FONT_NAME, zhFallback);
    /* 作用域分级：显式选择英文字体 → 宽作用域（含未翻译英文书）；
       auto 派生（跟随中文风格）→ 仅双语书英文栏，避免误伤纯中文书。 */
    const enExplicit = enFontFamily !== 'auto';
    const enScope = hasBilingualMarkup
      ? '.mt-en,.mt-en-h'
      : enExplicit
        ? ':lang(en),p:not(:has(.mt-zh,.mt-ch)),h1,h2,h3,h4,h5,h6,li,blockquote'
        : '';
    const enFontCss = en.rule && enScope ? `${enScope}{${en.rule}}` : '';

    return `
${zh.face}${en.face}
:root{
  --rd-font-size:${fontSize.toFixed(3)}rem;
  --rd-line-height:${lineHeight.toFixed(3)};
  --rd-para-spacing:${paraSpacing.toFixed(3)}em;
  --rd-gap-floor:${Math.max(0, Math.round(paragraphGapPx))}px;
  ${theme ? `--rd-bg:${theme.bg};--rd-fg:${theme.fg};--rd-muted:${theme.muted};--rd-en-fg:${theme.enFg};` : ''}
}
html{font-size:var(--rd-font-size)!important;${brightnessRule}}
body{
  line-height:var(--rd-line-height)!important;
  ${topMarginPx > 0 ? `padding-top:${topMarginPx}px!important;` : ''}
  ${bottomMarginPx > 0 ? `padding-bottom:${bottomMarginPx}px!important;` : ''}
  ${zh.rule}
  ${typeExtra ? typeExtraCss(typeExtra) : ''}
  ${theme ? `background:${theme.bg}!important;color:${theme.fg}!important;` : ''}
}
${enFontCss}
${skipType ? '' : `p{margin-top:0!important;margin-bottom:max(var(--rd-para-spacing), var(--rd-gap-floor, 0px))!important;}`}
${!skipType && typeExtra && typeExtra.indentEm >= 0 ? indentCss(typeExtra.indentEm, bilingual) : ''}
${!skipType ? alignCss(typeExtra ? typeExtra.textAlignment : 'auto') : ''}
${!skipType && typeExtra && typeExtra.headingScale !== 1 ? headingCss(typeExtra.headingScale) : ''}
.mt-en,.mt-en-h{${bilingual && theme ? `color:${theme.enFg}!important;` : ''}}
${bilingual ? bilingualCss(theme) : ''}
${bilingual && translationBlur ? blurCss() : ''}
${markCss()}
${languageRule}
img,svg,video{max-width:100%!important;height:auto!important;object-fit:contain;break-inside:avoid;}
table{max-width:100%!important;}
pre{max-width:100%!important;white-space:pre-wrap!important;overflow-wrap:anywhere!important;}
code{overflow-wrap:anywhere;}
a{color:inherit;}
::selection{background:rgba(120,140,160,.35);}
${columnCss(columnCount, columnThresholdPx)}
${customCss ? customCssCss(customCss) : ''}
`.trim();
  }, [
    brightnessPercent,
    customCss,
    fontFamily,
    enFontFamily,
    customEnFontPath,
    fontSize,
    hasBilingualMarkup,
    lang,
    lineHeight,
    paragraphGapPx,
    paraSpacing,
    theme,
    translationBlur,
    typeExtra,
    useBookStyles,
    columnCount,
    columnThresholdPx,
    topMarginPx,
    bottomMarginPx,
    customFontPath,
  ]);

  const applyStyles = useCallback(() => {
    const doc = getDoc();
    if (!doc?.head) return;
    /* 断字依赖 lang（Chromium 无 lang 时忽略 hyphens）：原书缺失时按当前模式兜底；
       both 保留原书标注（双语书英文栏由 applyDualLayout 逐段标注）。 */
    if (!doc.documentElement.lang) {
      const fallback = lang === 'en' ? 'en' : lang === 'zh' ? 'zh-CN' : '';
      if (fallback) doc.documentElement.lang = fallback;
    }
    if (!styleRef.current || styleRef.current.ownerDocument !== doc) {
      styleRef.current = doc.createElement('style');
      styleRef.current.id = 'mt-reader-prefs';
      doc.head.appendChild(styleRef.current);
    }
    styleRef.current.textContent = buildReaderCss();

    // 主动回忆：触屏无 hover，点按译文切换 .mt-revealed 常驻揭示。
    // 每个章节 Document 只挂一次；划选文本时不触发（避免选择后误翻转）。
    if (revealDocRef.current !== doc) {
      revealDocRef.current?.removeEventListener('click', handleBlurRevealClick);
      revealDocRef.current = null;
    }
    if (translationBlur && !revealDocRef.current) {
      doc.addEventListener('click', handleBlurRevealClick);
      revealDocRef.current = doc;
    }
  }, [buildReaderCss, getDoc, lang, translationBlur]);

  const applyDualLayout = useCallback(() => {
    const doc = getDoc();
    if (!doc?.body) return;
    doc.body.querySelectorAll(`.${DUAL_CLASS}`).forEach((element) => {
      element.classList.remove(DUAL_CLASS);
    });
    /* 双语片段逐段标注 lang：让 hyphens 断字/字体 :lang() 选择器在
       “html lang 与片段语言不一致”时仍准确（任何阅读模式都需要）。 */
    if (hasBilingualMarkup) {
      doc.body.querySelectorAll('.mt-en,.mt-en-h').forEach((el) => el.setAttribute('lang', 'en'));
      doc.body.querySelectorAll('.mt-zh,.mt-ch').forEach((el) => el.setAttribute('lang', 'zh-CN'));
    }
    if (!hasBilingualMarkup || lang !== 'both') return;

    doc.body.querySelectorAll('[data-mt-pair]').forEach((element) => {
      const hasPair = Boolean(
        element.querySelector(':scope > .mt-zh') && element.querySelector(':scope > .mt-en'),
      );
      if (hasPair) element.classList.add(DUAL_CLASS);
    });

    doc.body.querySelectorAll('p,h1,h2,h3,h4,h5,h6,blockquote,figcaption').forEach((element) => {
      const hasParagraphPair = Boolean(
        element.querySelector(':scope > .mt-zh') && element.querySelector(':scope > .mt-en'),
      );
      const hasHeadingPair = Boolean(
        element.querySelector(':scope > .mt-ch') && element.querySelector(':scope > .mt-en-h'),
      );
      if (hasParagraphPair || hasHeadingPair) element.classList.add(DUAL_CLASS);
    });
  }, [getDoc, hasBilingualMarkup, lang]);

  return { applyDualLayout, applyStyles };
}

/** 细粒度排版 CSS 片段。 */
function typeExtraCss(extra: ReaderTypeExtra): string {
  /* 字重。 */
  const weightRule =
    extra.fontWeight !== 400 ? `p,li,blockquote,dd,dt,figcaption,h1,h2,h3,h4,h5,h6{font-weight:${extra.fontWeight}!important;}` : '';
  const inline: string[] = [];
  if (extra.letterSpacingPx !== 0) inline.push(`letter-spacing:${extra.letterSpacingPx}px!important`);
  if (extra.wordSpacingPx !== 0) inline.push(`word-spacing:${extra.wordSpacingPx}px!important`);
  return [weightRule, inline.length > 0 ? `body{${inline.join(';')}}` : '']
    .filter(Boolean)
    .join('\n');
}

/** 段首缩进：所有正文段落（含章节首段——部分 EPUB 对章首段显式 noindent，
    统一为缩进排版）；列表/引用/表格/图注内的段落排除。
    indentEm < 0：不注入（跟随书籍原样式）；= 0：强制无缩进；> 0：指定缩进。 */
function indentCss(indentEm: number, bilingual: boolean): string {
  if (indentEm < 0) return '';
  /* 配对段缩进排除只在双语网格下需要（网格两列不吃 text-indent）；
     中/英单语模式下配对段就是普通段落——恢复缩进（修复单语模式被双语规则泄漏）。 */
  const pairRule = bilingual ? 'p[data-mt-pair]{text-indent:0!important;}' : '';
  return `p{text-indent:${indentEm.toFixed(2)}em!important;}
li p,blockquote p,figcaption p,td p,th p,dt p,dd p,pre p{text-indent:0!important;}
${pairRule}`;
}

/** 正文对齐：作用于正文块级元素。
    justify（含 auto）不启用断字（hyphens 会把英文单词按音节拆到两行——
    用户明确要求单词始终保持整行）；仅留 overflow-wrap 兜底：单词本身
    超过行宽（如超长 URL）时才强制拆断，保证不溢出页边距（单/双页同治，
    分页 CSS 的 overflow-wrap:break-word!important 同为兜底非主动拆词）。
    left/center/right 自然排版。 */
function alignCss(alignment: string): string {
  const justified = alignment === 'auto' || alignment === 'justify';
  const value = alignment === 'auto' ? 'justify' : alignment;
  const justifyHelpers = justified
    ? 'word-break:normal;overflow-wrap:break-word;orphans:2;widows:2;'
    : '';
  return `body p,body li,body blockquote{text-align:${value}!important;${justifyHelpers}}`;
}

/** 分栏：0=自动（宽屏双栏），1/2 固定。 */
function columnCss(columnCount: number, columnThresholdPx: number): string {
  if (columnCount === 1) return '';
  if (columnCount === 2) {
    return `body{columns:2!important;column-gap:2em!important;}`;
  }
  // 0=auto：视口宽超过阈值才双栏
  const threshold = Math.max(320, Math.min(2400, Math.round(columnThresholdPx)));
  return `@media (min-width: ${threshold}px){body{columns:2!important;column-gap:2em!important;}}`;
}

/** 标题缩放：相对正文字号的 zoom（保持原书标题字号体系）。 */
function headingCss(scale: number): string {
  return `h1,h2,h3,h4,h5,h6{zoom:${scale.toFixed(2)};}`;
}

function bilingualCss(theme: ReaderTheme | null): string {
  return `
.${DUAL_CLASS}{
  display:grid!important;
  grid-template-columns:minmax(0,1fr) minmax(0,1fr)!important;
  column-gap:1.2em!important;
  align-items:start!important;
  break-inside:auto!important;
  /* 配对块按段落节奏参与段距（外层 !important 段距规则不作用于配对 p 本身） */
  margin-top:0!important;margin-bottom:max(var(--rd-para-spacing), var(--rd-gap-floor, 0px))!important;
}
.${DUAL_CLASS}>.mt-zh,.${DUAL_CLASS}>.mt-ch,
.${DUAL_CLASS}>.mt-en,.${DUAL_CLASS}>.mt-en-h{display:block!important;min-width:0;margin:0!important;}
.${DUAL_CLASS}>.mt-en,.${DUAL_CLASS}>.mt-en-h{
  border-left:1px solid ${theme ? theme.muted : 'rgba(120,120,120,.35)'};
  padding-left:1.2em;
  font-size:.9em;
  line-height:calc(var(--rd-line-height)*.92);
  /* 栏内无自身段距（外层段落已按段距排布，避免叠加造成段落间空白翻倍） */
  margin:0!important;
  ${theme ? `color:${theme.enFg}!important;` : 'opacity:.82;'}
}
.${DUAL_CLASS}>br{display:none!important;}

/* 窄屏（手机/窄窗）双列各不足一掌宽，可读性差——降级为段落上下对照：
   中文段在上、英文段以左侧竖线缩进弱化跟随（沉浸式翻译的移动端范式）。 */
@media (max-width: 600px) {
  .${DUAL_CLASS}{grid-template-columns:minmax(0,1fr)!important;}
  .${DUAL_CLASS}>.mt-en,.${DUAL_CLASS}>.mt-en-h{
    border-left:2px solid ${theme ? theme.muted : 'rgba(120,120,120,.35)'};
    padding-left:.8em;
    margin-top:.4em;
  }
}
`;
}

/** 用户自定义 CSS：原文追加注入，优先级最高（末尾生效）。
    剥离 </style> 防止提前闭合 style 标签。 */
function customCssCss(css: string): string {
  return `/* user custom css */\n${css.replace(/<\/style/gi, '<\\/style')}`;
}

/* 主动回忆点按揭示：仅在译文单元上切换，划选文本时让位。 */
function handleBlurRevealClick(event: Event) {
  const doc = event.currentTarget as Document | null;
  if (doc?.getSelection?.()?.toString().trim()) return;
  const target = event.target as Element | null;
  const cell = target?.closest?.('.mt-zh,.mt-ch');
  if (cell) cell.classList.toggle('mt-revealed');
}

/* 主动回忆模式：译文默认模糊，主动悬停/点按才清晰。
   点按通过 .mt-revealed 类常驻揭示（触屏无 hover）。 */
function blurCss(): string {
  return `
.mt-zh,.mt-ch{
  filter:blur(5px);
  opacity:.75;
  cursor:pointer;
  transition:filter .25s ease,opacity .25s ease;
}
.mt-zh:hover,.mt-ch:hover,.mt-zh.mt-revealed,.mt-ch.mt-revealed{
  filter:none;
  opacity:1;
}
`;
}
