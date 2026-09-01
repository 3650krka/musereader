import { invoke } from '@tauri-apps/api/core';
import type { WordMark } from '@/types';
import type { ReaderFlow } from './epubReaderTypes';

/** 与后端 WordLevelInfo 对齐（camelCase）。 */
export interface WordLevelInfo {
  word: string;
  level: string;
  phonetic: string;
  definition: string;
  /** 英英释义（可选；当前词表未提供，'en' 上标档缺失时退回中文释义）。 */
  enDefinition?: string;
  /** ECDICT 完整中文翻译（词性分段，通常比 definition 更全）。 */
  translation?: string;
  /** 考试标签（zk/gk/cet4/cet6/ky/toefl/gre…，空格分隔）。 */
  tag?: string;
  /** 词形变换（ECDICT exchange 编码：d 过去式/p 过去分词/i 进行时/3 三单/s 复数）。 */
  exchange?: string;
  /** 词根助记（最多两条，分号分隔）。 */
  root?: string;
  /** 柯林斯星级 1–5（缺省=无数据）。 */
  collins?: number;
  /** 牛津3000（1=是）。 */
  oxford?: number;
  /** BNC 词频排名（越小越常用）。 */
  bnc?: number;
  /** COCA 词频排名。 */
  frq?: number;
  /** 批量查询回显：实际查询的词形（可能是屈折形）；与 word（原形）不同才出现。 */
  surface?: string;
}

/** WordWise 行内标注样式：虚线下划线 / 高亮底色 / 仅上标。 */
export type WordWiseStyle = 'underline' | 'highlight' | 'plain';
/** 上标内容：中文释义 / 英文解释 / 音标。 */
export type WordWiseGloss = 'zh' | 'en' | 'phonetic';

export interface WordWiseAnnotateOptions {
  enabled: boolean;
  userLevel: string;
  style: WordWiseStyle;
  gloss: WordWiseGloss;
  /** 上标字号（em 相对正文字号），0.5–0.85。 */
  glossSize: number;
  /** 行距加大系数：含标注词的宿主在基础行距上乘此系数；上标字号另外推导
      自适应行距绝对下限，二者取大。1=无手动加大。仅行距，绝不动字/词间距。 */
  lineBoost: number;
  /** 相邻上标最小间隙（px，用户可设）：同行上标间隙低于该值时平移避让，绝不重叠。 */
  glossGapPx: number;
  /** 标注主色（上标文字 + 划线/高亮装饰）。 */
  accentColor: string;
  wordMarks: Record<string, WordMark>;
}

/* ── 结构（v2）────────────────────────────────────────────────────────
   <span class="mt-ww [--ul|--hl]" data-word data-word-surface data-mark?>
     <span class="mt-ww-w">surface</span>
     <span class="mt-ww-g" aria-hidden="true">gloss</span>
   </span>
   · .mt-ww 为 inline-block：原子不拆行，且为 .mt-ww-g 提供确定的定位 containing block。
   · .mt-ww-g 绝对定位悬于词上方，完全脱离行内流——不占行进宽度，
     词距/断行与无标注时逐像素一致（v1 ruby 负 margin 方案在 CSS 规范上无效的根除）。
   · 纵向（上/下方向）：宿主行距下限 CSS 兜底行间不互压；
     块首行上标越出块顶时由 relieveWordWiseCollisions 实测补 padding。
   · 横向（左/右方向）：相邻上标间隙不足时给后者单词补外边距加宽词距——
     上标随词移动、永远居中于词形之上（绝不平移上标造成错位）。 */
const MARK_CLASS = 'mt-ww';
const WORD_CLASS = 'mt-ww-w';
const GLOSS_CLASS = 'mt-ww-g';
const HOST_CLASS = 'mt-ww-host';
const STYLE_ID = 'mt-ww-style';
const WORD_SOURCE = '[A-Za-z][A-Za-z\'-]*';
/** 单节最多上送批量查询的独立词数（防御异常大章节）。学习词（含屈折形候选）
    在此上限之外恒优先送查，绝不被截断——截断即意味着用户主动标记的词丢标注。 */
const MAX_LOOKUP_WORDS = 3000;
/** 上标释义截断长度（完整释义在弹卡展示）。 */
const MAX_GLOSS_CHARS = 14;
/** 上标单行最大宽度（上标字号 em）：超长释义截断省略，防宽上标挤占
    同行间距（完整释义点词卡展示；绝对定位块化后 ellipsis 生效）。 */
const MAX_GLOSS_WIDTH_EM = 10;
/** 相邻上标间隙的固定安全余量（px），与用户设定的间距相加。 */
const COLLISION_MARGIN_PX = 1;
/** 上标水平位置自适应的左右安全边（px，块内容盒内收）。 */
const EDGE_PX = 2;
/** 上标行高（自身）与上浮间隙（em）：间隙取小值让上标贴近词形。 */
const GLOSS_LINE_HEIGHT = 1.2;
const GLOSS_LIFT_EM = 0.05;
/** 行距绝对下限 = 1.35 + 1.2×字号：上标高度(1.2s) + 上浮(0.05) + 上一行
    降部(~0.2) + 呼吸余量(~0.1)，由上下两行的半行距共同承担，
    保证相邻行字形永不被压（上/下方向防遮挡的结构保证）。 */
const LINE_FLOOR_BASE = 1.35;
/** 块首行越顶修补：原 paddingTop 记录在该属性上，解标/重测时还原。 */
const PAD_ATTR = 'data-mt-ww-pad';
/** 词距补偿标记（横向碰撞）：给标注外壳补的左外边距记录在该属性上；
    复位/解标时清空重来。只动词距、不平移上标（上标恒居中词形之上）。 */
const MARGIN_ATTR = 'data-mt-ww-ml';
/** 页缘收拢标记（横向）：词距让无可让（词已贴页缘）时的终局兜底——
    以 CSS 变量 --mt-ww-dx 平移上标本体收进页内（仅 transform，
    不动行内流）。正常情形上标恒居中词形，绝不平移；唯有词已无处可让
    时才启用，保证上标绝不跨页、绝不落入双页中缝。 */
const CLAMP_ATTR = 'data-mt-ww-dx';
/** 过量补偿回收滞环（px）：布局变化（字号/字体/视口）后旧补偿多余、
    实际间隙超出目标值该余量时才回收，避免浮点抖动来回拉扯。 */
const RECLAIM_SLACK_PX = 6;

/** 每次新建正则：共享 /g 正则有 lastIndex 状态污染风险。 */
function wordPattern(): RegExp {
  return new RegExp(WORD_SOURCE, 'g');
}

/** 同一文档的标注请求串行队列：unwrap 与包裹之间隔着异步 invoke，
    并发重标（图片/字体回流、偏好连点）会让后一次的 walker 扫进前一次刚包出的
    标注文本节点，造成同词双层/重复包裹——按文档排队彻底消除交错。 */
const annotateQueue = new WeakMap<Document, Promise<unknown>>();

/**
 * WordWise 行内标注：把 iframe 文档英文栏中的超档生词包成
 * `<span class="mt-ww" data-word="…"><span class="mt-ww-w">surface</span><span class="mt-ww-g">gloss</span></span>`。
 * 严格幂等：先解开全部既有标注再统一扫描一遍；同一词在同一章节内只标首次出现。
 * 返回本次标注的词信息表（宿主弹卡查释义用）；查询失败返回 null——
 * 此时既有标注原样保留，调用方应沿用旧词表（绝不抹掉在屏标注）。
 */
export function annotateWordWise(
  doc: Document,
  options: WordWiseAnnotateOptions,
): Promise<Map<string, WordLevelInfo> | null> {
  const run = () => runAnnotate(doc, options);
  const chained: Promise<Map<string, WordLevelInfo> | null> =
    (annotateQueue.get(doc) ?? Promise.resolve()).then(run, run);
  annotateQueue.set(doc, chained.then(() => undefined, () => undefined));
  return chained;
}

async function runAnnotate(
  doc: Document,
  options: WordWiseAnnotateOptions,
): Promise<Map<string, WordLevelInfo> | null> {
  /* 零闪烁重标：先在既有标注上取词并完成异步查询，再同帧一次性解标+重包。
     旧序（解标 → await 查询 → 重包）中间隔着 invoke 空窗，浏览器会把
     "裸文本中间帧"渲染出来——偏好/黑白名单变化引发的全量重标闪一下，
     即用户感知的"点了界面刷新"。取词用 host.textContent（含旧标注文本：
     词面相同；释义上标里的英文残片 n./vt. 之类只会多查几个词条，
     包裹阶段在解标后的纯文本上进行，不会误标）。 */
  if (!options.enabled) {
    unwrapWordWise(doc);
    injectWordWiseStyle(doc, options);
    return new Map;
  }

  const hosts = collectEnglishHosts(doc);
  const tokens = collectTokens(hosts, options.wordMarks);
  const infos =
    tokens.length > 0
      ? await lookupBatch(tokens, options)
      : new Map<string, WordLevelInfo>;

  /* 查询失败防御：invoke 异常时绝不能解标——否则一次瞬时失败就会把
     全部既有标注抹掉（即"标注全部消失、再点开关又全回来"的根源）。
     返回 null，保留现状，由调用方沿用旧词表。 */
  if (infos === null) return null;

  // 查询返回后同步完成（同一任务内，无中间渲染）：解标 → 注样式 → 重包。
  unwrapWordWise(doc);
  injectWordWiseStyle(doc, options);
  if (infos.size === 0) return infos;

  // 章节级首现去重：跨宿主共享，保证同词只包一个标注。
  const seen = new Set<string>();
  for (const host of hosts) {
    if (wrapHostWords(host, infos, options, seen)) host.classList.add(HOST_CLASS);
  }
  /* 碰撞自愈不在此时做：分页（applyFlow）会改变列宽与断行，
     由 useEpubReader 在每次 applyFlow 后调用 relieveWordWiseCollisions 实测。 */
  return infos;
}

/** 解除全部 WordWise 标注与修补痕迹（标注还原纯文本、宿主类/越顶 padding/
    词距补偿外边距一并撤销，恢复自然布局）。 */
export function unwrapWordWise(doc: Document): void {
  doc.querySelectorAll(`span.${MARK_CLASS}`).forEach((mark) => {
    const surface = mark.getAttribute('data-word-surface') ?? mark.textContent ?? '';
    mark.replaceWith(doc.createTextNode(surface));
  });
  doc.querySelectorAll(`.${HOST_CLASS}`).forEach((host) => host.classList.remove(HOST_CLASS));
  doc.querySelectorAll(`[${PAD_ATTR}]`).forEach((block) => {
    const original = block.getAttribute(PAD_ATTR) ?? '';
    if (original) (block as HTMLElement).style.paddingTop = original;
    else (block as HTMLElement).style.removeProperty('padding-top');
    block.removeAttribute(PAD_ATTR);
  });
  doc.querySelectorAll(`[${MARGIN_ATTR}]`).forEach((mark) => {
    (mark as HTMLElement).style.removeProperty('margin-left');
    (mark as HTMLElement).style.removeProperty('margin-right');
    mark.removeAttribute(MARGIN_ATTR);
  });
  doc.body?.normalize();
}

/* ── 局部补丁（单词级增量更新）──────────────────────────────────────
   生词增删/状态变化/语境义保存不再全文解标重包：仅摘除与重包受影响词
   的标注外壳（data-word 精确定位，屈折形标注同记原形一并命中），其余
   标注、碰撞修补（越顶内边距/词距外边距/页缘收拢）与分页几何原样保留
   ——版面只在该词所在行可能变化，阅读位置绝无跳变。
   查询失败返回 null：一个字也不动（与全量重标的 null 协议一致）。 */
export interface WordMarksPatch {
  /** 需摘除标注的词（移出词表/生词本）。 */
  removed: string[];
  /** 需（重新）标注的词及其最新标记（新增词、状态或语境义变化词）。 */
  added: { word: string; mark: WordMark }[];
}

export interface WordPatchOutcome {
  /** 合并后的词信息表（旧表 + 本次新查），宿主弹卡桥沿用。 */
  infoMap: Map<string, WordLevelInfo>;
  removedMarks: number;
  wrappedMarks: number;
}

export function patchWordWiseMarks(
  doc: Document,
  patch: WordMarksPatch,
  options: WordWiseAnnotateOptions,
  baseInfoMap: Map<string, WordLevelInfo>,
): Promise<WordPatchOutcome | null> {
  const run = () => runPatch(doc, patch, options, baseInfoMap);
  const chained: Promise<WordPatchOutcome | null> =
    (annotateQueue.get(doc) ?? Promise.resolve()).then(run, run);
  annotateQueue.set(doc, chained.then(() => undefined, () => undefined));
  return chained;
}

async function runPatch(
  doc: Document,
  patch: WordMarksPatch,
  options: WordWiseAnnotateOptions,
  baseInfoMap: Map<string, WordLevelInfo>,
): Promise<WordPatchOutcome | null> {
  if (!options.enabled) return null;
  const removed = patch.removed
    .map((word) => word.trim().toLowerCase())
    .filter(Boolean);
  const added = patch.added
    .filter((entry) => entry.word && entry.mark)
    .map((entry) => ({ word: entry.word.trim().toLowerCase(), mark: entry.mark }));
  if (removed.length === 0 && added.length === 0) return null;

  const hosts = collectEnglishHosts(doc);
  let removedMarks = 0;

  /* 新增词先送查（词本体 + learning 词的屈折形词面）：失败则仅执行摘除，
     既有标注一个字不动。 */
  let looked: Map<string, WordLevelInfo> | null = new Map;
  if (added.length > 0) {
    const learningWords = new Set(
      added.filter((entry) => entry.mark.status === 'learning').map((entry) => entry.word),
    );
    const learningHit = (surface: string): boolean =>
      learningWords.has(surface) ||
      (learningWords.size > 0 && lemmaCandidates(surface).some((candidate) => learningWords.has(candidate)));
    const tokenSet = new Set<string>;
    for (const entry of added) tokenSet.add(entry.word);
    if (learningWords.size > 0) {
      for (const host of hosts) {
        for (const match of host.textContent?.matchAll(wordPattern()) ?? []) {
          const surface = match[0].toLowerCase();
          if (surface.length >= 3 && learningHit(surface)) tokenSet.add(surface);
        }
      }
    }
    looked = await lookupBatch([...tokenSet], options);
  }
  if (looked === null) {
    for (const word of removed) removedMarks += unwrapMarksForWord(doc, word);
    if (removedMarks === 0) return null;
    finalizePatch(doc, hosts);
    return { infoMap: baseInfoMap, removedMarks, wrappedMarks: 0 };
  }

  for (const word of removed) removedMarks += unwrapMarksForWord(doc, word);

  let wrappedMarks = 0;
  for (const entry of added) {
    /* 旧标先摘（无条件）：状态转 mastered/退出词表后查无新标，旧标若保留
       会造成「标了已掌握但上标还在」；重包时全量重建，避免新旧语义混杂。 */
    removedMarks += unwrapMarksForWord(doc, entry.word);
    const info = looked.get(entry.word);
    if (!info) continue; // 词库未收录/已排除：与全量标注一致，不包裹
    const isLearning = entry.mark.status === 'learning';
    let firstWrapped = false;
    for (const host of hosts) {
      wrappedMarks += wrapHostForWord(
        host, entry.word, info, options, isLearning, () => firstWrapped, () => { firstWrapped = true; },
      );
    }
  }

  if (removedMarks === 0 && wrappedMarks === 0) return null;
  const infoMap = new Map(baseInfoMap);
  for (const [key, value] of looked) infoMap.set(key, value);
  finalizePatch(doc, hosts);
  return { infoMap, removedMarks, wrappedMarks };
}

/** 摘除指定词的全部分节标注（屈折形标注的 data-word 同记原形，一并命中）。 */
function unwrapMarksForWord(doc: Document, word: string): number {
  let count = 0;
  doc.querySelectorAll(`span.${MARK_CLASS}[data-word]`).forEach((mark) => {
    if (mark.getAttribute('data-word') !== word) return;
    const surface = mark.getAttribute('data-word-surface') ?? mark.textContent ?? '';
    mark.replaceWith(doc.createTextNode(surface));
    count += 1;
  });
  return count;
}

/** 宿主内包裹目标词：learning 全次、其余首现（跨宿主由闭包旗标共享）。 */
function wrapHostForWord(
  host: HTMLElement,
  targetWord: string,
  info: WordLevelInfo,
  options: WordWiseAnnotateOptions,
  isLearning: boolean,
  isFirstWrapped: () => boolean,
  markFirstWrapped: () => void,
): number {
  const doc = host.ownerDocument;
  const walker = doc.createTreeWalker(host, NodeFilter.SHOW_TEXT);
  const textNodes: Text[] = [];
  while (walker.nextNode) {
    const node = walker.currentNode as Text;
    if (node.data.trim() && !node.parentElement?.closest(`.${MARK_CLASS}`)) textNodes.push(node);
  }
  let wrapped = 0;
  for (const node of textNodes) {
    const text = node.data;
    if (!wordPattern().test(text)) continue;
    const fragment = doc.createDocumentFragment();
    let cursor = 0;
    let hit = false;
    for (const match of text.matchAll(wordPattern())) {
      const surface = match[0];
      const index = match.index ?? 0;
      const lowered = surface.toLowerCase();
      if (lowered !== targetWord && !lemmaCandidates(lowered).includes(targetWord)) continue;
      if (!isLearning) {
        if (isFirstWrapped()) continue;
        markFirstWrapped();
      }
      hit = true;
      fragment.append(doc.createTextNode(text.slice(cursor, index)));
      fragment.append(buildMark(doc, surface, info, options));
      wrapped += 1;
      cursor = index + surface.length;
    }
    if (!hit) continue;
    fragment.append(doc.createTextNode(text.slice(cursor)));
    node.replaceWith(fragment);
  }
  if (wrapped > 0) host.classList.add(HOST_CLASS);
  return wrapped;
}

/** 收尾：无标注宿主摘除宿主类（行距下限随之失效）；文本归一化。 */
function finalizePatch(doc: Document, hosts: HTMLElement[]): void {
  for (const host of hosts) {
    if (!host.querySelector(`span.${MARK_CLASS}`)) host.classList.remove(HOST_CLASS);
  }
  doc.body?.normalize();
}

/** 英文栏宿主元素：双语书取 .mt-en/.mt-en-h；否则全文块级元素（纯英文书/仅英文模式）。 */
function collectEnglishHosts(doc: Document): HTMLElement[] {
  const bilingual = doc.body.querySelectorAll('.mt-en,.mt-en-h');
  if (bilingual.length > 0) return Array.from(bilingual) as HTMLElement[];
  return Array.from(
    doc.body.querySelectorAll('p,li,blockquote,h1,h2,h3,h4,h5,h6,figcaption'),
  ) as HTMLElement[];
}

/** 汇总候选词（去重、小写）：learning 词恒优先送查（不受单节上限截断——
    用户主动标记的词必须保证获得提词），其余按出现顺序计数封顶。
    learning 匹配做词形还原感：用户标记的多为原形（abandon），文中出现的
    常是屈折形（abandoned/abandons/abandoning）——前端用与后端同构的
    屈折规则逆推候选，命中即送查（最终是否强制标注由后端按原形判定）。 */
function collectTokens(hosts: HTMLElement[], wordMarks: Record<string, WordMark>): string[] {
  const learning = new Set(
    Object.entries(wordMarks)
      .filter(([, mark]) => mark.status === 'learning')
      .map(([word]) => word),
  );
  const learningHit = (word: string): boolean =>
    learning.has(word) || (learning.size > 0 && lemmaCandidates(word).some((candidate) => learning.has(candidate)));
  const seen = new Set<string>();
  const ordered: string[] = [];
  const push = (word: string) => {
    if (word.length < 3 || seen.has(word)) return;
    seen.add(word);
    ordered.push(word);
  };
  for (const host of hosts) {
    for (const match of host.textContent?.matchAll(wordPattern()) ?? []) {
      const word = match[0].toLowerCase();
      if (learningHit(word)) push(word);
    }
  }
  for (const host of hosts) {
    if (ordered.length >= MAX_LOOKUP_WORDS) break;
    for (const match of host.textContent?.matchAll(wordPattern()) ?? []) {
      const word = match[0].toLowerCase();
      if (learning.has(word)) continue;
      push(word);
      if (ordered.length >= MAX_LOOKUP_WORDS) break;
    }
  }
  return ordered;
}

/** 屈折形 → 原形候选（与后端 word_levels.rs 的 lemma_candidates 同构）：
    复数/三单（-s/-es/-ies/'s）、进行体（-ing）、过去式（-ed/-ied）、
    比较级最高级（-er/-est），含双写尾辅音还原。仅作匹配提示——
    最终命中以词典为准（候选查无即否）。 */
function lemmaCandidates(word: string): string[] {
  if (word.length < 4) return [];
  const out: string[] = [];
  const stem = word.slice(0, -1);
  const stem2 = word.slice(0, -2);
  const stem3 = word.slice(0, -3);
  if (word.endsWith('s')) {
    if (word.endsWith("'s") || word.endsWith('’s')) out.push(word.slice(0, -2));
    if (word.endsWith('es')) out.push(stem2, `${stem2}e`);
    if (word.endsWith('ies')) out.push(`${stem3}y`);
    out.push(stem, `${stem}e`);
  }
  if (word.endsWith('ing')) {
    const ingStem = stem3;
    out.push(ingStem, `${ingStem}e`);
    const cs = Array.from(ingStem);
    if (cs.length >= 3) {
      const last = cs[cs.length - 1];
      const prev = cs[cs.length - 2];
      if (last === prev && !'aeiou'.includes(last)) out.push(cs.slice(0, -1).join(''));
    }
  }
  if (word.endsWith('ied')) out.push(`${stem3}y`);
  if (word.endsWith('ed')) {
    const edStem = stem2;
    out.push(edStem, `${edStem}e`);
    const cs = Array.from(edStem);
    if (cs.length >= 3) {
      const last = cs[cs.length - 1];
      const prev = cs[cs.length - 2];
      if (last === prev && !'aeiou'.includes(last)) out.push(cs.slice(0, -1).join(''));
    }
  }
  if (word.endsWith('iest')) out.push(`${word.slice(0, -4)}y`);
  if (word.endsWith('er') || word.endsWith('est')) {
    const base = word.endsWith('est') ? word.slice(0, -3) : stem2;
    out.push(base, `${base}e`);
    const cs = Array.from(base);
    if (cs.length >= 3) {
      const last = cs[cs.length - 1];
      const prev = cs[cs.length - 2];
      if (last === prev && !'aeiou'.includes(last)) out.push(cs.slice(0, -1).join(''));
    }
  }
  return out.filter((candidate) => candidate.length >= 2 && candidate !== word);
}

/** 单次 invoke 批量查询：mastered 排除、learning 强制、其余高于用户档。
    查询失败返回 null（而非空表）——调用方据此保留既有标注，避免瞬时
    失败导致全部标注被误抹。 */
async function lookupBatch(
  tokens: string[],
  options: WordWiseAnnotateOptions,
): Promise<Map<string, WordLevelInfo> | null> {
  const mastered: string[] = [];
  const learning: string[] = [];
  for (const [word, mark] of Object.entries(options.wordMarks)) {
    if (mark.status === 'mastered') mastered.push(word);
    else if (mark.status === 'learning') learning.push(word);
  }
  try {
    const rows = await invoke<WordLevelInfo[]>('lookup_words_batch', {
      words: tokens,
      userLevel: options.userLevel,
      mastered,
      learning,
    });
    /* 返回行以原形（row.word）为词目，屈折形查询回显 row.surface（送查词形）：
       surface 为键供包裹阶段按文中词形命中；原形另立别名键——标注外壳的
       data-word 记原形，弹卡桥与自动播种按原形回查此表，双键才两头命中。 */
    const infos = new Map<string, WordLevelInfo>;
    for (const row of rows) {
      if (row.surface) infos.set(row.surface, row);
      infos.set(row.word, row);
    }
    return infos;
  } catch (error) {
    console.warn('WordWise batch lookup failed:', error);
    return null;
  }
}

/** 逐文本节点分词重写：命中词包标注，未命中保持原文本。返回该宿主是否有标注。 */
function wrapHostWords(
  host: HTMLElement,
  infos: Map<string, WordLevelInfo>,
  options: WordWiseAnnotateOptions,
  seen: Set<string>,
): boolean {
  const doc = host.ownerDocument;
  const walker = doc.createTreeWalker(host, NodeFilter.SHOW_TEXT);
  const textNodes: Text[] = [];
  while (walker.nextNode()) {
    const node = walker.currentNode as Text;
    if (node.data.trim()) textNodes.push(node);
  }
  let wrapped = false;
  for (const node of textNodes) {
    const fragment = buildAnnotatedFragment(doc, node.data, infos, options, seen);
    if (fragment) {
      node.replaceWith(fragment);
      wrapped = true;
    }
  }
  return wrapped;
}

/** 文本 → DocumentFragment；无命中返回 null（避免无谓 DOM 写）。 */
function buildAnnotatedFragment(
  doc: Document,
  text: string,
  infos: Map<string, WordLevelInfo>,
  options: WordWiseAnnotateOptions,
  seen: Set<string>,
): DocumentFragment | null {
  const pattern = wordPattern();
  if (!pattern.test(text)) return null;
  const fragment = doc.createDocumentFragment();
  let cursor = 0;
  let hit = false;
  for (const match of text.matchAll(wordPattern())) {
    const surface = match[0];
    const info = infos.get(surface.toLowerCase());
    const index = match.index ?? 0;
    if (!info) continue;
    /* 生词（learning）全次标注：用户主动加入生词本的词每处出现都要提词，
       不受首现去重限制；超档词维持首现一次，避免通篇重复轰炸。 */
    const isLearning = options.wordMarks[info.word]?.status === 'learning';
    if (!isLearning) {
      if (seen.has(info.word)) continue;
      seen.add(info.word);
    }
    hit = true;
    fragment.append(doc.createTextNode(text.slice(cursor, index)));
    fragment.append(buildMark(doc, surface, info, options));
    cursor = index + surface.length;
  }
  if (!hit) return null;
  fragment.append(doc.createTextNode(text.slice(cursor)));
  return fragment;
}

/** 单个命中词 → 标注元素（词形 inline-block + 绝对定位上标，上标不占行进宽度）。 */
function buildMark(
  doc: Document,
  surface: string,
  info: WordLevelInfo,
  options: WordWiseAnnotateOptions,
): HTMLElement {
  const mark = doc.createElement('span');
  mark.className = `${MARK_CLASS} ${MARK_CLASS}--${options.style}`;
  mark.setAttribute('data-word', info.word);
  mark.setAttribute('data-word-surface', surface);
  const markStatus = options.wordMarks[info.word]?.status;
  if (markStatus) mark.setAttribute('data-mark', markStatus);
  const base = doc.createElement('span');
  base.className = WORD_CLASS;
  base.textContent = surface;
  const gloss = doc.createElement('span');
  gloss.className = GLOSS_CLASS;
  gloss.setAttribute('aria-hidden', 'true');
  gloss.textContent = glossText(info, options);
  mark.append(base, gloss);
  return mark;
}

/* ── 碰撞自愈（实测式，收敛检测，分页感知）──────────────────────────
   每次分页/排版应用后调用（列宽与断行已定型）。流程：复位既有修补 →
   在自然布局上实测 → 计算目标修补量 → 与上一轮比对，仅变化时应用并
   报告（调用方据此重分页**吸收**修补——修补改变行内流，若不被分页
   吸收，被推移的词会滞留页界之外：新增生词"凭空消失"、内容跨页的
   根源即此）。
   1. 纵向：块首行上标越出块内容顶 → 越过量超出「上方可用带」的部分补
      为块 paddingTop（上一块横向不相邻（跨栏/跨页）时不计入，防误判）。
   2. 横向（词距补偿）：**自然布局快照 → 纯计算 → 一次性写入**。同行相邻
      上标间隙不足 → 给后者单词的标注外壳补左外边距（上标随词移动、恒
      居中词形之上，绝不错位）。补偿量受「词至行尾剩余空间」硬约束：
      绝不把词挤出当前行——挤出行的词会被分栏流推到下一页。
   3. 页缘收拢 + 互叠兜底（终局，仅 transform，不动行内流）：词距让无可让
      时按行扫描——左缘越界与相邻上标间隙不足向右收（上界=页右界），
      右界吃不下再向左回收左侧上标（下界=页左界/前一上标）。上标绝不
      跨页、绝不落入中缝、互不重叠；词形位置不再改变。
   返回值 = 是否修补了流布局（外边距/内边距）；修补已就位（与上轮一致）
   时返回 false，分页迭代自然收敛终止。 */
export function relieveWordWiseCollisions(doc: Document, glossGapPx: number, flow: ReaderFlow): boolean {
  const marks = Array.from(doc.querySelectorAll(`span.${MARK_CLASS}`)) as HTMLElement[];
  if (marks.length === 0) return false;

  const win = doc.defaultView;
  const viewWidth = win?.innerWidth ?? 0;
  if (!win || viewWidth <= 0) return false;
  const scrollX = win.scrollX || 0;
  /* 分页节距（文档坐标）：横向分页的每一栏即一页——单页模式栏距=视口宽，
     双页展开每视口两栏（栏距=半视口宽，与分页层 ≥1000px 回落阈值一致）；
     滚动模式无横向分页（整篇一栏，无页界）。碰撞核算与页缘收拢一律在
     文档坐标进行——视口坐标随滚动漂移，会把翻页中途的测量归错页。 */
  const paged = flow !== 'scrolled';
  const pitch = !paged ? null : flow === 'spread' && viewWidth >= 1000 ? viewWidth / 2 : viewWidth;

  /* 复位：记录上一轮修补量（收敛比对基准）并还原自然布局。 */
  const prevPad = new Map<HTMLElement, number>;
  doc.querySelectorAll(`[${PAD_ATTR}]`).forEach((block) => {
    const el = block as HTMLElement;
    prevPad.set(el, parseFloat(getComputedStyle(el).paddingTop) || 0);
    const original = el.getAttribute(PAD_ATTR) ?? '';
    if (original) el.style.paddingTop = original;
    else el.style.removeProperty('padding-top');
    el.removeAttribute(PAD_ATTR);
  });
  const prevMargin = new Map<HTMLElement, number>;
  doc.querySelectorAll(`[${MARGIN_ATTR}]`).forEach((mark) => {
    const el = mark as HTMLElement;
    prevMargin.set(el, parseFloat(el.style.marginLeft) || 0);
    /* 右外边距通道已废弃（不改自身位置、只会挤后续行内内容引发级联换行，
       右越界由页缘收拢兜底）：存量残留一并清除。 */
    el.style.removeProperty('margin-left');
    el.style.removeProperty('margin-right');
    el.removeAttribute(MARGIN_ATTR);
  });
  doc.querySelectorAll(`[${CLAMP_ATTR}]`).forEach((mark) => {
    (mark as HTMLElement).style.removeProperty('--mt-ww-dx');
    mark.removeAttribute(CLAMP_ATTR);
  });

  type Entry = { mark: HTMLElement; gloss: HTMLElement; block: HTMLElement };
  const entries: Entry[] = [];
  for (const mark of marks) {
    const gloss = mark.querySelector(`:scope > .${GLOSS_CLASS}`) as HTMLElement | null;
    if (!gloss) continue;
    if (gloss.getBoundingClientRect().width <= 0) continue; // 宿主 display:none（单语模式隐藏英文栏）
    const block = (mark.closest('p,li,blockquote,h1,h2,h3,h4,h5,h6,figcaption,dd,td') ??
      mark.parentElement) as HTMLElement | null;
    if (!block) continue;
    entries.push({ mark, gloss, block });
  }
  if (entries.length === 0) return false;

  /* 1. 纵向：块首行越顶修补（先做——padding 会下移后续行，横向测量随其后取新值）。 */
  const byBlock = new Map<HTMLElement, Entry[]>;
  for (const entry of entries) {
    const list = byBlock.get(entry.block);
    if (list) list.push(entry);
    else byBlock.set(entry.block, [entry]);
  }
  let padded = false;
  for (const [block, list] of byBlock) {
    const cs = getComputedStyle(block);
    const blockRect = block.getBoundingClientRect();
    const basePad = parseFloat(cs.paddingTop) || 0; // 复位后 = 原始内边距
    const contentTop = blockRect.top + basePad;
    const protrusion = Math.max(
      0,
      ...list.map((e) => contentTop - e.gloss.getBoundingClientRect().top),
    );
    if (protrusion <= 0) continue;
    const band = availableBandAbove(block, blockRect);
    const deficit = protrusion - band - COLLISION_MARGIN_PX;
    if (deficit <= 0) continue;
    const target = basePad + deficit;
    const prev = prevPad.get(block) ?? basePad;
    /* 复位已摘掉旧修补：目标量无论是否与上轮一致都必须重新写回，
       仅变化与否决定是否上报（触发重分页）。 */
    if (!block.hasAttribute(PAD_ATTR)) {
      block.setAttribute(PAD_ATTR, block.style.paddingTop);
    }
    block.style.paddingTop = `${Math.round(target * 10) / 10}px`;
    if (Math.abs(target - prev) > 0.1) padded = true;
  }

  /* 2. 横向（词距补偿）：自然布局快照 → 纯计算 → 一次性写入。
     纪律：计算全程只读快照，绝不写后再读——写读交错会把已写入的位移
     二次计入累计量（补偿偏小 → 间隙残留上标重叠；或偏大 → 推挤断行）。
     **页边界在文档坐标核算**：横向分页的每一栏即一页（单页栏距=视口宽，
     双页展开栏距=半视口宽，分页层已保证栏与页精确对齐）——跨栏块的
     bounding rect 是并集盒，必须按页截取实界，且不同页的上标绝不能归入
     同一行桶互推。滚动模式无页界，仅块内行级核算。 */
  const gap = COLLISION_MARGIN_PX + Math.max(0, glossGapPx);

  /* 文档坐标 → 所在页与页界（滚动模式无页界，返回无穷域）。 */
  const pageBoundsFor = (docCenterX: number): { page: number; left: number; right: number } => {
    if (!pitch) return { page: 0, left: Number.MIN_SAFE_INTEGER, right: Number.MAX_SAFE_INTEGER };
    const page = Math.max(0, Math.floor(docCenterX / pitch));
    return { page, left: page * pitch, right: (page + 1) * pitch };
  };

  /* 自然布局快照（复位与纵向修补之后：越顶 padding 会下移后续行，
     横向测量必须在此取新值；此后一切计算只基于快照）。 */
  type Snap = {
    entry: Entry;
    glossLeft: number;
    glossWidth: number;
    glossHeight: number;
    wordRight: number;
    markTop: number;
    glossTop: number;
  };
  type PageGroup = { left: number; right: number; blocks: Map<HTMLElement, Snap[]> };
  const byPage = new Map<number, PageGroup>;
  for (const entry of entries) {
    const rect = entry.gloss.getBoundingClientRect();
    if (rect.width <= 0) continue; // 宿主 display:none（单语模式隐藏英文栏）
    const wordRect = entry.mark.getBoundingClientRect();
    const { page, left, right } = pageBoundsFor(rect.left + rect.width / 2 + scrollX);
    let group = byPage.get(page);
    if (!group) {
      group = { left, right, blocks: new Map };
      byPage.set(page, group);
    }
    const snap: Snap = {
      entry,
      glossLeft: rect.left + scrollX,
      glossWidth: rect.width,
      glossHeight: rect.height,
      wordRight: wordRect.right + scrollX,
      markTop: wordRect.top,
      glossTop: rect.top,
    };
    const list = group.blocks.get(entry.block);
    if (list) list.push(snap);
    else group.blocks.set(entry.block, [snap]);
  }

  /* 行级修补计划：左→右纯计算。间隙不足给后者单词补左外边距（目标为
     相对自然布局的绝对总量，写入后一次生效）。
     累计位移纯算术：前词被推右 m，同行其右所有词在快照坐标上 +m——
     缺口与剩余空间均按「自然位置 + 累计」核算，不再写后读。
     补偿量不得超过该词至行尾的剩余空间——挤出行的词会被分栏流推到
     下一页（词"凭空消失"的直接成因），越界尾部留给步骤 3 变换兜底。 */
  type RowPlan = { snaps: Snap[]; left: number; right: number };
  const rowPlans: RowPlan[] = [];
  const pendingMargins = new Map<HTMLElement, number>;
  const planRow = (snaps: Snap[], leftLimit: number, rightLimit: number, pageLeft: number, pageRight: number) => {
    snaps.sort((a, b) => a.glossLeft - b.glossLeft);
    let prevRight = leftLimit - gap; // 行首越左界由初值自然覆盖
    let shift = 0;
    for (const snap of snaps) {
      const actualLeft = snap.glossLeft + shift;
      const deficit = prevRight + gap - actualLeft;
      let target = 0;
      if (deficit > 0.5) {
        const room = rightLimit - (snap.wordRight + shift);
        if (room > 0) target = Math.min(deficit, room);
      }
      /* 滞环收敛：无碰撞而旧补偿不大时保留原量（净状态未变，避免浮点
         抖动引发空转重分页）；超出滞环余量的旧补偿保持摘除态（回收）。 */
      const prev = prevMargin.get(snap.entry.mark) ?? 0;
      let applied = 0;
      if (target > 0.1) applied = Math.round(target * 10) / 10;
      else if (prev > 0.1 && prev <= RECLAIM_SLACK_PX) applied = Math.round(prev * 10) / 10;
      if (applied > 0) pendingMargins.set(snap.entry.mark, applied);
      shift += applied;
      prevRight = actualLeft + applied + snap.glossWidth;
    }
    rowPlans.push({ snaps, left: pageLeft, right: pageRight });
  };

  for (const group of byPage.values()) {
    for (const [block, list] of group.blocks) {
      const cs = getComputedStyle(block);
      const blockRect = block.getBoundingClientRect();
      /* 列边界（文档坐标）：块自身矩形与所在页内容域的交集
         （跨栏块的 bounding rect 覆盖全部碎片，按页截取得各页实界）。 */
      const contentLeft = Math.max(blockRect.left + scrollX + (parseFloat(cs.paddingLeft) || 0), group.left);
      const contentRight = Math.min(blockRect.right + scrollX - (parseFloat(cs.paddingRight) || 0), group.right);
      if (contentRight - contentLeft < 20) continue;
      const leftLimit = contentLeft + EDGE_PX;
      const rightLimit = contentRight - EDGE_PX;
      const lines = new Map<number, Snap[]>;
      for (const snap of list) {
        /* 行桶：行高量级（上标高度 ×2，≥8px）作容差——相邻行上标
           垂直距离 ≈行高，不会误并进同一桶；同一行上标天然同桶。 */
        const key = Math.round(snap.glossTop / Math.max(8, snap.glossHeight * 2));
        const row = lines.get(key);
        if (row) row.push(snap);
        else lines.set(key, [snap]);
      }
      /* 变换层平移的收拢域：分页模式取页界（上标绝不跨页/落中缝）；
         滚动模式无页界，收进块内容域（上标不越出本块，仍居中词形附近）。 */
      const clampLeftBound = pitch ? group.left : leftLimit;
      const clampRightBound = pitch ? group.right : rightLimit;
      for (const row of lines.values()) planRow(row, leftLimit, rightLimit, clampLeftBound, clampRightBound);
    }
  }

  /* 一次性写入 + 变化上报（与上轮差值决定是否触发重分页吸收；
     复位已摘掉全部旧补偿——新量无论是否与上轮一致都必须写回，
     比对只决定是否上报，否则收敛轮会把补偿丢光）。 */
  let spaced = false;
  for (const [mark, applied] of pendingMargins) {
    const prev = prevMargin.get(mark) ?? 0;
    mark.style.marginLeft = `${applied}px`;
    mark.setAttribute(MARGIN_ATTR, '1');
    if (Math.abs(applied - prev) > 0.1) spaced = true;
  }
  for (const [mark, prev] of prevMargin) {
    if (prev > 0.1 && !pendingMargins.has(mark)) spaced = true;
  }

  /* 断行校验（二分回退）：补偿若把词推过行尾，行内后续内容换行——
     标注词自身掉行会随分栏流被推到下一页。实测受补词的视觉行：掉行者
     将该行补偿对半递减重试，直至不掉行（或退到零）——词绝不被挤出行。 */
  for (const plan of rowPlans) {
    const touched = plan.snaps.filter((snap) => (pendingMargins.get(snap.entry.mark) ?? 0) > 0);
    if (touched.length === 0) continue;
    const baseMargins = new Map(touched.map((snap) => [snap.entry.mark, pendingMargins.get(snap.entry.mark) ?? 0]));
    for (let attempt = 0; attempt < 4; attempt += 1) {
      /* 掉行判定：视觉行位移 ≥行高量级；同行之内 margin-left 不改 top，
         容差取上标高度（远小于行高，远大于亚像素抖动）。 */
      const wrapped = touched.some((snap) => {
        const top = snap.entry.mark.getBoundingClientRect().top;
        return Math.abs(top - snap.markTop) > Math.max(4, snap.glossHeight);
      });
      if (!wrapped) break;
      spaced = true;
      const factor = 1 / 2 ** (attempt + 1);
      for (const snap of touched) {
        const base = baseMargins.get(snap.entry.mark) ?? 0;
        const next = Math.round(base * factor * 10) / 10;
        if (next > 0.05) {
          snap.entry.mark.style.marginLeft = `${next}px`;
          pendingMargins.set(snap.entry.mark, next);
        } else {
          snap.entry.mark.style.removeProperty('margin-left');
          snap.entry.mark.removeAttribute(MARGIN_ATTR);
          pendingMargins.set(snap.entry.mark, 0);
        }
      }
    }
  }

  /* 3. 终局兜底（仅变换层，不动行内流）：词距补偿受「词不出行」硬约束，
     行尾剩余空间不足时必有残差——按行把上标本体平移收位：
     a. 相邻上标间隙不足 → 后者右移（上界：页右界）；右界吃不下再向左
        回收前者（下界：页左界/前一上标），双向挤压把间隙尽量做到位；
     b. 上标越出页缘/落入双页中缝 → 收回自己所在页内。
     上标绝不跨页、绝不落入中缝、互不重叠；词形位置不再改变。
     滚动模式页界为无穷域，仅执行间隙约束。 */
  for (const plan of rowPlans) {
    const live = plan.snaps
      .map((snap) => {
        const rect = snap.entry.gloss.getBoundingClientRect();
        return { snap, left: rect.left + scrollX, width: rect.width, dx: 0 };
      })
      .filter((item) => item.width > 0)
      .sort((a, b) => a.left - b.left);
    if (live.length === 0) continue;
    const clampLeft = plan.left + 1;
    const clampRight = plan.right - 1;
    /* 双向扫描迭代两轮：链式推挤（三个以上相邻上标）一轮只能传播一半。 */
    for (let pass = 0; pass < 2; pass += 1) {
      /* 左→右：左缘收拢 + 间隙约束（右推，受页右界硬上界）。 */
      let prevRight = -Infinity;
      for (const item of live) {
        if (item.left + item.dx < clampLeft) item.dx += clampLeft - (item.left + item.dx);
        const need = prevRight + gap - (item.left + item.dx);
        if (need > 0.5) {
          const roomRight = clampRight - (item.left + item.dx + item.width);
          if (roomRight > 0) item.dx += Math.min(need, roomRight);
        }
        prevRight = item.left + item.dx + item.width;
      }
      /* 右→左：右缘收拢 + 残差间隙向左回收（受页左界/前一上标硬下界）。 */
      let nextLeft = Infinity;
      for (let i = live.length - 1; i >= 0; i -= 1) {
        const item = live[i];
        const pRight = item.left + item.dx + item.width;
        if (pRight > clampRight) item.dx -= pRight - clampRight;
        const over = item.left + item.dx + item.width + gap - nextLeft;
        if (over > 0.5 && i > 0) {
          const prevItem = live[i - 1];
          const minDx = prevItem.left + prevItem.dx + prevItem.width + gap - item.left;
          const reducible = Math.max(0, item.dx - minDx);
          if (reducible > 0) item.dx -= Math.min(over, reducible);
        }
        nextLeft = item.left + item.dx;
      }
    }
    for (const item of live) {
      if (Math.abs(item.dx) > 0.5) {
        item.snap.entry.mark.style.setProperty('--mt-ww-dx', `${Math.round(item.dx * 10) / 10}px`);
        item.snap.entry.mark.setAttribute(CLAMP_ATTR, '1');
      }
    }
  }
  return padded || spaced;
}

/** 块上方可用带：与上一块的外边距带（含 margin 折叠后的实测间隙）
    + 上一块末行下方半行距余量。上一块横向不相邻（跨栏/跨页）或不存在时不计。 */
function availableBandAbove(block: HTMLElement, blockRect: DOMRect): number {
  let prev: Element | null = block.previousElementSibling;
  while (prev) {
    const rect = prev.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      const horizontallyAdjacent = rect.right > blockRect.left + 4 && rect.left < blockRect.right - 4;
      const gapPx = blockRect.top - rect.bottom;
      if (horizontallyAdjacent && gapPx >= 0) {
        return Math.max(0, gapPx + halfLeadingBelow(prev));
      }
      return 0; // 紧邻块跨栏/异常：按无余量处理（保守补 padding）
    }
    prev = prev.previousElementSibling;
  }
  return 0;
}

/** 元素末行下方半行距余量（(行高−字号)/2，'normal' 按 1.2 倍字号近似）。 */
function halfLeadingBelow(el: Element): number {
  const cs = getComputedStyle(el);
  const fontSize = parseFloat(cs.fontSize) || 0;
  if (!fontSize) return 0;
  const lineHeight = cs.lineHeight === 'normal' ? fontSize * 1.2 : parseFloat(cs.lineHeight) || fontSize * 1.2;
  return Math.max(0, (lineHeight - fontSize) / 2);
}

/** 上标文本：AI 语境义（用户保存的 customDefinition）在中文档优先——它是
    本书语境下的释义，比词典通用义更贴切；其余档按序取值。
    中文取第一义项截断；英文档取英英释义（缺失退回中文）；音标档直接展示。
    占位词条（词库未收录的 learning 词）退显档级文案，不留空。 */
function glossText(info: WordLevelInfo, options: WordWiseAnnotateOptions): string {
  const custom = options.wordMarks[info.word]?.customDefinition?.trim();
  if (options.gloss === 'phonetic') return info.phonetic || custom || info.level;
  const source = options.gloss === 'en'
    ? (info.enDefinition?.trim() || info.definition || custom || '')
    : (custom || info.definition);
  const firstSense = (source.split(/\n+/)[0] ?? '').trim();
  if (!firstSense) return info.level || '生词';
  if (firstSense.length <= MAX_GLOSS_CHARS) return firstSense;
  return `${firstSense.slice(0, MAX_GLOSS_CHARS)}…`;
}

/** 注入 iframe 内 WordWise 样式（宿主 CSS 进不了 iframe，必须文档内注入）。 */
function injectWordWiseStyle(doc: Document, options: WordWiseAnnotateOptions): void {
  let style = doc.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = doc.createElement('style');
    style.id = STYLE_ID;
    doc.head.appendChild(style);
  }
  style.textContent = buildWordWiseCss(options);
}

function buildWordWiseCss(options: WordWiseAnnotateOptions): string {
  const size = Math.min(0.85, Math.max(0.5, options.glossSize));
  const boost = Math.min(1.6, Math.max(1, options.lineBoost ?? 1)).toFixed(2);
  /* 行距绝对下限：上标高度 + 上浮 + 上一行降部 + 呼吸余量，
     由上下两行半行距共同承担；与用户手动加大（基数×系数）取大。 */
  const floor = (LINE_FLOOR_BASE + GLOSS_LINE_HEIGHT * size).toFixed(3);
  return `
.${MARK_CLASS}{
  display:inline-block;position:relative;cursor:pointer;
  /* inline-block 天然原子：justify/break-word 下整词换行，绝无词中断裂。 */
  white-space:nowrap;
  /* 容器盒收紧到字形高度：上标锚定盒顶=锚定字形顶（继承宿主大行距会让
      上标虚高半行距，视觉离词偏远）；内部行盒 1em，基线对齐不变、词不位移。 */
  line-height:1;
}
.${GLOSS_CLASS}{
  position:absolute;left:50%;bottom:calc(100% + ${GLOSS_LIFT_EM}em);
  /* 常态恒居中词形（-50%）；页缘收拢兜底时由碰撞管线写入 --mt-ww-dx
     微调平移（仅词已贴页缘、词距让无可让时启用，上标绝不跨页/落中缝）。 */
  transform:translateX(calc(-50% + var(--mt-ww-dx, 0px)));
  white-space:nowrap;line-height:${GLOSS_LINE_HEIGHT};
  /* 单行宽度上限（上标字号 em）：超长释义截断省略——宽上标会把同行
     上下文的间距吃光（"非相邻词上标拥挤"的结构来源），完整释义点词卡
     展示。绝对定位天然块化，text-overflow 生效。 */
  max-width:${MAX_GLOSS_WIDTH_EM}em;overflow:hidden;text-overflow:ellipsis;
  font-size:${size}em;font-family:ui-sans-serif,system-ui,sans-serif;
  font-style:normal;font-weight:600;
  color:${options.accentColor};
  pointer-events:none;user-select:none;-webkit-user-select:none;
  padding:0 .08em;z-index:1;
}
${variantCss(options.style, options.accentColor)}
/* 含标注宿主的行距下限（仅行距，不动字间距/词间距）：
   双语网格 .mt-en 栏自带 .92 行高(!important, 0,2,0)，需更高特异性覆盖。 */
.${HOST_CLASS}{
  line-height:max(calc(var(--rd-line-height, 1.9) * ${boost}), ${floor})!important;
}
.mt-reader-dual>.mt-en.${HOST_CLASS},.mt-reader-dual>.mt-en-h.${HOST_CLASS}{
  line-height:max(calc(var(--rd-line-height, 1.9) * ${boost} * .92), ${floor})!important;
}
/* learning 词强调：上标加深 */
.${MARK_CLASS}[data-mark="learning"] .${GLOSS_CLASS}{font-weight:700;text-decoration:underline;}
/* 点击选中反馈：词形短暂淡高亮（弹卡同时出现，动画结束自然回落）。
   .mt-ww-flash 为单击查词的临时包裹（非标注词），共用同一动效。 */
.${MARK_CLASS}.mt-ww-hit .${WORD_CLASS}, .mt-ww-flash{animation:mt-ww-hit .8s ease;}
@keyframes mt-ww-hit{
  0%{background:color-mix(in srgb, ${options.accentColor} 38%, transparent);border-radius:3px;}
  100%{background:transparent;}
}
`.trim();
}

function variantCss(style: WordWiseStyle, accent: string): string {
  if (style === 'underline') {
    return `.${WORD_CLASS}{border-bottom:1.5px dashed color-mix(in srgb, ${accent} 62%, transparent);padding-bottom:1px;}
.${MARK_CLASS}[data-mark="learning"] .${WORD_CLASS}{border-bottom-style:solid;}`;
  }
  if (style === 'highlight') {
    return `.${WORD_CLASS}{background:color-mix(in srgb, ${accent} 26%, transparent);border-radius:3px;padding:0 1px;}`;
  }
  return '';
}

/** 句边界：英文句号/感叹/问号 + 中文对应符号 + 省略号 + 弯引号（对话边界）。
    不含 ASCII 直引号/撇号（会截断 don't 类缩写与所有格）。 */
const SENTENCE_TERMINAL = /[.!?。！？…“”]/g;

/**
 * 从块文本中提取标注词所在句子（弹卡例句，纯函数）。
 * 定位该词后向前找句首边界、向后找句尾边界；空白归一化后返回整句。
 * 词定位不到返回 null（调用方退回段落截断）；「句子」过长（整段无标点/
 * 超长独白）退回以词为中心的短窗口（词前约 60 字符 + 词后约 100 字符，
 * 再按词边界修剪两端），保证例句短且可读。
 */
export function extractSentenceContext(
  blockText: string,
  word: string,
  maxChars = 240,
): string | null {
  const text = blockText.replace(/\s+/g, ' ').trim();
  if (!text || !word) return null;
  const index = locateWordIndex(text, word);
  if (index < 0) return null;

  let lastBefore = -1;
  SENTENCE_TERMINAL.lastIndex = 0;
  for (let match = SENTENCE_TERMINAL.exec(text); match; match = SENTENCE_TERMINAL.exec(text)) {
    if (match.index >= index) break;
    lastBefore = match.index;
  }
  SENTENCE_TERMINAL.lastIndex = index + word.length;
  const afterMatch = SENTENCE_TERMINAL.exec(text);
  const start = lastBefore + 1;
  const end = afterMatch ? afterMatch.index + 1 : text.length;

  const sentence = text.slice(start, end).trim();
  if (sentence.length > maxChars) return trimWindowAroundWord(text, index, word.length, maxChars);
  return sentence;
}

/** 短窗口兜底：词前 60 字符 + 词后 100 字符（上限内），两端按词边界修剪。 */
function trimWindowAroundWord(text: string, index: number, wordLen: number, maxChars: number): string {
  let lo = Math.max(0, index - 60);
  let hi = Math.min(text.length, index + wordLen + 100);
  if (hi - lo > maxChars) hi = lo + maxChars;
  /* 词边界修剪：避免截断首尾单词 */
  while (lo < index && /\S/.test(text[lo] ?? '')) lo += 1;
  while (hi > index + wordLen && /\S/.test(text[hi - 1] ?? '')) hi -= 1;
  if (hi <= lo) return text.slice(index, Math.min(text.length, index + maxChars)).trim();
  return text.slice(lo, hi).trim();
}

/** 词定位：优先整词边界匹配（大小写不敏感），退回普通子串查找。 */
function locateWordIndex(text: string, word: string): number {
  const escaped = word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`\\b${escaped}\\b`, 'i').exec(text);
  if (match) return match.index;
  return text.toLowerCase().indexOf(word.toLowerCase());
}
