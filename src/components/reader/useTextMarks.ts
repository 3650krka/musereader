import { useCallback } from 'react';

/** 正文划线标记：把选区包进 <mark class="mt-mark">，按样式着色；
    点击已划区域的处理在 EpubReaderView 内联实现（需宿主坐标映射与卸载清理）。 */

export type MarkStyle = 'highlight' | 'underline' | 'wave' | 'none' | 'bookmark';

/** 注入 iframe 的划线样式（useEpubPresentation buildReaderCss 内联）。 */
export function markCss(): string {
  return `
mt-mark, mark.mt-mark { background: transparent; color: inherit; }
/* 搜索命中高亮（侧栏 snippet + 跳转后正文内） */
mark.mt-hit { background: rgba(240,170,64,.45); color: inherit; border-radius: 2px; padding: 0 1px; }
/* 跳转定位闪烁（笔记/书签卡跳入正文后高亮一下） */
.mt-flash { animation: mt-flash 2s ease; border-radius: 3px; }
@keyframes mt-flash {
  0% { background: rgba(240,170,64,.55); box-shadow: 0 0 0 3px rgba(240,170,64,.35); }
  70% { background: rgba(240,170,64,.25); }
  100% { background: transparent; box-shadow: none; }
}
mark.mt-mark[data-style="highlight"] { background: rgba(240,192,64,.38); color: inherit; }
mark.mt-mark[data-style="underline"] { text-decoration: underline; text-decoration-color: rgba(143,208,255,.95); text-decoration-thickness: 2px; text-underline-offset: 3px; }
mark.mt-mark[data-style="wave"] { text-decoration: underline wavy rgba(154,230,180,.95); text-decoration-thickness: 1.5px; text-underline-offset: 3px; }
mark.mt-mark[data-style="bookmark"] { background: rgba(120,140,160,.16); }
mark.mt-mark { cursor: pointer; border-radius: 2px; }
mark.mt-mark:hover { filter: brightness(.96); }
`.trim();
}

interface UseTextMarksOptions {
  getDoc: () => Document | null;
}

/** 划线回注数据（从持久层书签投影）。 */
export interface RestoreMarkItem {
  quote: string;
  /** highlight/underline/wave（bookmark 无可视划线）。 */
  style: string;
  /** 可选自定义色（#rrggbb）。 */
  color?: string | null;
}

/** 在容器文本节点序列中查找引文首次出现处，返回覆盖 Range（找不到返回 null）。
    实现：收集文本节点与累计长度，在拼接文本中定位后映射回节点区间。 */
function findQuoteRange(container: Node, quote: string): Range | null {
  const doc = container.ownerDocument;
  if (!doc) return null;
  const needle = quote.trim();
  if (!needle) return null;
  const nodes: Text[] = [];
  let full = '';
  const walker = doc.createTreeWalker(container, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      /* 跳过 WordWise 上标文本：标注后节点树多了释义文本，
         不跳过会导致引文定位偏移/找不到（user-select:none 不进选区，
         保存的引文永远是纯词形文本）。 */
      if ((node as Text).parentElement?.closest('.mt-ww-g')) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let node = walker.nextNode();
  while (node) {
    nodes.push(node as Text);
    full += node.textContent ?? '';
    node = walker.nextNode();
  }
  const idx = full.indexOf(needle);
  if (idx === -1) return null;
  const end = idx + needle.length;

  // 二分定位起止节点
  const locate = (offset: number): { textNode: Text; inner: number } | null => {
    let acc = 0;
    for (const textNode of nodes) {
      const len = textNode.textContent?.length ?? 0;
      if (offset <= acc + len) return { textNode, inner: offset - acc };
      acc += len;
    }
    return null;
  };
  const startAt = locate(idx);
  const endAt = locate(end);
  if (!startAt || !endAt) return null;
  try {
    const range = doc.createRange();
    range.setStart(startAt.textNode, startAt.inner);
    range.setEnd(endAt.textNode, endAt.inner);
    return range;
  } catch {
    return null;
  }
}

/** 选区端点吸附生词标注：半选中的 span.mt-ww（inline-block）若被包裹会碎裂
    造成换行异常/包裹失败——端点在标注内部或恰好切入标注时，外扩为整个标注。 */
function snapWordMarkBoundaries(range: Range): void {
  const SNAP_SELECTOR = 'span.mt-ww';
  const markOf = (node: Node | null | undefined): Element | null => {
    if (!(node instanceof Element)) return null;
    return node.matches(SNAP_SELECTOR) ? node : node.closest(SNAP_SELECTOR);
  };
  try {
    /* 起点：文本节点落在标注内 → 外扩到标注前；容器切点命中部分标注 → 外扩。 */
    if (range.startContainer.nodeType === Node.TEXT_NODE) {
      const host = range.startContainer.parentElement?.closest(SNAP_SELECTOR);
      if (host) range.setStartBefore(host);
    } else {
      const child = range.startContainer.childNodes[range.startOffset];
      const host = markOf(child);
      if (host) range.setStartBefore(host);
    }
    /* 终点镜像处理。 */
    if (range.endContainer.nodeType === Node.TEXT_NODE) {
      const host = range.endContainer.parentElement?.closest(SNAP_SELECTOR);
      if (host) range.setEndAfter(host);
    } else if (range.endOffset > 0) {
      const child = range.endContainer.childNodes[range.endOffset - 1];
      const host = markOf(child);
      if (host) range.setEndAfter(host);
    }
  } catch {
    /* 端点吸附失败非致命：退回原始选区继续尝试。 */
  }
}

/** 逐文本节点分块包裹（surroundContents/extractContents 均失败时的兜底）：
    选区覆盖的每个文本节点单独包一个 mark，保证可视划线总能落下。
    跳过 WordWise 上标文本（user-select:none 本就不进选区，防御异常路径）。 */
function wrapRangePerTextNode(doc: Document, range: Range, createMark: () => HTMLElement): boolean {
  const walker = doc.createTreeWalker(range.commonAncestorContainer, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if ((node as Text).parentElement?.closest('.mt-ww-g')) return NodeFilter.FILTER_REJECT;
      return range.intersectsNode(node) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });
  const targets: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) targets.push(n as Text);
  let wrapped = false;
  for (const text of targets) {
    const sub = doc.createRange();
    sub.setStart(text, text === range.startContainer ? range.startOffset : 0);
    sub.setEnd(text, text === range.endContainer ? range.endOffset : text.data.length);
    if (sub.toString().length === 0) continue;
    try {
      sub.surroundContents(createMark());
      wrapped = true;
    } catch {
      /* 单节点异常跳过，不影响其余分块。 */
    }
  }
  return wrapped;
}

export function useTextMarks({ getDoc }: UseTextMarksOptions) {
  /** 划线持久化回注：章节文档就绪后按书签数据幂等重画 mark.mt-mark。
      回注前先清掉已有回注标记（防重复包裹）；DOM 划线（用户手动）不动。 */
  const restoreMarks = useCallback((items: RestoreMarkItem[]): number => {
    const doc = getDoc();
    if (!doc?.body) return 0;
    // 清除既有回注（仅 restore 产物，data-restore 标记）
    doc.body.querySelectorAll('mark.mt-mark[data-restore]').forEach((el) => {
      const parent = el.parentNode;
      if (!parent) return;
      while (el.firstChild) parent.insertBefore(el.firstChild, el);
      el.remove();
    });
    if (items.length === 0) return 0;
    /* 已有手动划线索引（一次遍历建表，替代每条书签一次全树 querySelectorAll——
       书签多时由 O(书签 × 划线) 降为 O(书签 + 划线)）。 */
    const manualMarks = new Set<string>;
    doc.body.querySelectorAll('mark.mt-mark').forEach((el) => {
      const style = el.getAttribute('data-style');
      const text = (el.textContent ?? '').trim();
      if (style && text) manualMarks.add(`${style}::${text}`);
    });
    /* 恢复过的也并入索引：同一 quote+style 的多条书签只回注一次（与旧逐条判重语义一致）。 */
    let restored = 0;
    for (const item of items) {
      const style = item.style;
      if (style !== 'highlight' && style !== 'underline' && style !== 'wave') continue;
      // 已有相同 quote 的手动划线则跳过
      const quoteKey = `${style}::${item.quote.trim()}`;
      if (manualMarks.has(quoteKey)) continue;
      const range = findQuoteRange(doc.body, item.quote);
      if (!range) continue;
      /* 划线跨生词标注时端点吸附（与手动划线同规则）：避免 surroundContents
         把 inline-block 标注切碎造成换行异常。 */
      snapWordMarkBoundaries(range);
      const mark = doc.createElement('mark');
      mark.className = 'mt-mark';
      mark.setAttribute('data-style', style);
      mark.setAttribute('data-restore', '');
      if (item.color) {
        if (style === 'highlight') mark.style.background = `${item.color}59`;
        else mark.style.textDecorationColor = item.color;
        mark.setAttribute('data-color', item.color);
      }
      try {
        range.surroundContents(mark);
        restored += 1;
        manualMarks.add(quoteKey);
      } catch {
        try {
          const fragment = range.extractContents();
          mark.appendChild(fragment);
          range.insertNode(mark);
          restored += 1;
          manualMarks.add(quoteKey);
        } catch {
          // 跨块复杂选区：跳过（数据仍在书签列表）
        }
      }
    }
    return restored;
  }, [getDoc]);

  /** 把当前选区包裹为划线。
      选区与 WordWise 生词标注（inline-block span）相交时：
      1. 端点吸附到完整标注（半选标注会把 inline-block 切碎造成换行/失败）；
      2. surroundContents 因跨元素失败时退 extractContents；
      3. 仍失败（复杂嵌套）时逐文本节点分块包裹，保证可视划线总能落下。 */
  const applyMark = useCallback((style: MarkStyle, color?: string): boolean => {
    const doc = getDoc();
    const sel = doc?.getSelection?.();
    if (!doc || !sel || sel.isCollapsed || sel.rangeCount === 0) return false;
    if (style === 'none') return true; // 清除划线：交由书签删除处理
    const range = sel.getRangeAt(0);
    snapWordMarkBoundaries(range);
    const mark = doc.createElement('mark');
    mark.className = 'mt-mark';
    mark.setAttribute('data-style', style);
    if (color) {
      // 带色划线：内联覆盖（高亮=底色，划线=装饰色）
      if (style === 'highlight') mark.style.background = `${color}59`;
      else mark.style.textDecorationColor = color;
      mark.setAttribute('data-color', color);
    }
    try {
      range.surroundContents(mark);
    } catch {
      try {
        const fragment = range.extractContents();
        mark.appendChild(fragment);
        range.insertNode(mark);
      } catch {
        /* 复杂嵌套（跨块/表格/生词标注密集区）：逐文本节点分块包裹。 */
        if (!wrapRangePerTextNode(doc, range, () => {
          const piece = doc.createElement('mark');
          piece.className = 'mt-mark';
          piece.setAttribute('data-style', style);
          if (color) {
            if (style === 'highlight') piece.style.background = `${color}59`;
            else piece.style.textDecorationColor = color;
            piece.setAttribute('data-color', color);
          }
          return piece;
        })) return false;
      }
    }
    sel.removeAllRanges();
    return true;
  }, [getDoc]);

  return { applyMark, restoreMarks };
}
