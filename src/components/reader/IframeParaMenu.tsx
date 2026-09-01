import { useCallback, useEffect, useRef, useState } from 'react';
import { useT } from '@/i18n';
import type { ParagraphAction } from './useAiAssistant';

/** 段落快捷菜单动作类型 */
export interface ParaMenuAction {
  id: ParagraphAction | 'ask';
  label: string;
}

interface IframeParaMenuProps {
  /** 宿主坐标系中的菜单锚点（小红点旁）。 */
  anchor: { top: number; left: number } | null;
  /** 在锚点上方展开（触屏长按场景，防止松手误触）。 */
  placeAbove?: boolean;
  onAction: (action: ParagraphAction | 'ask') => void;
  onClose: () => void;
}

/** 段落红点菜单（宿主侧渲染，iframe 内小红点点击后浮出）。 */
export function IframeParaMenu({ anchor, placeAbove, onAction, onClose }: IframeParaMenuProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  /* 视口钳制：渲染后量取自身尺寸，菜单不超出页面边距
     （8px 安全边）。左右按半宽收缩，上下按高度翻转/上移。 */
  useEffect(() => {
    if (!anchor) {
      setPos(null);
      return;
    }
    const el = ref.current;
    if (!el) {
      setPos({ top: anchor.top, left: anchor.left });
      return;
    }
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      setPos({ top: anchor.top, left: anchor.left });
      return;
    }
    const half = rect.width / 2;
    const left = Math.min(Math.max(anchor.left, half + 8), window.innerWidth - half - 8);
    let top = placeAbove ? anchor.top - rect.height : anchor.top;
    if (top + rect.height > window.innerHeight - 8) top = Math.max(8, window.innerHeight - rect.height - 8);
    if (top < 8) top = 8;
    setPos({ top, left });
  }, [anchor, placeAbove, pos === null]);

  useEffect(() => {
    if (!anchor) return;
    const onPointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [anchor, onClose]);

  if (!anchor) return null;

  const items: ParaMenuAction[] = [
    { id: 'summarize', label: t.reader.paraMenu.summarize },
    { id: 'explain', label: t.reader.paraMenu.explain },
    { id: 'translate', label: t.reader.paraMenu.retranslate },
    { id: 'ask', label: t.reader.paraMenu.ask },
  ];

  return (
    <div
      ref={ref}
      className="selmenu paramenu"
      style={{
        top: pos?.top ?? anchor.top,
        left: pos?.left ?? anchor.left,
        transform: placeAbove ? 'translateY(-100%)' : undefined,
      }}
      role="menu"
    >
      {items.map((item) => (
        <button key={item.id} type="button" role="menuitem" onClick={() => onAction(item.id)}>
          {item.label}
        </button>
      ))}
    </div>
  );
}

/** 段落小红点状态：锚点元素与宿主坐标。 */
export interface ParaDotState {
  paragraphText: string;
  anchor: { top: number; left: number };
  /** 触屏长按触发时为 true：菜单显示在触点上方，避免手指抬起误触菜单项。 */
  placeAbove?: boolean;
}

const DOT_SIZE = 5;
/** 红点悬于段落左侧沟槽，与段落左缘的水平间距（px）。分页模式最小侧 padding 20px，14px 落在沟槽内。 */
const DOT_GUTTER_OFFSET = 14;
const DOT_ID = 'mt-para-dot';

/**
 * 在 EPUB iframe 文档内挂载「真小红点」：
 * - 鼠标悬停段落 → 段落**左侧沟槽**出现 5px 半透明红圆点（无图标、无背景、零干扰）
 * - 绝对定位到文档坐标（与 collectReaderAnchors 同一坐标系），不插入段落内部，
 *   段落文本零位移——行内插入曾把首行整体右推，干扰阅读
 * - 点击红点 → 回调宿主浮出段落动作菜单
 * 滚动/翻页/卸载时自动隐藏与清理。
 */
export function useIframeParaDot(
  getDoc: () => Document | null,
  iframeRef: React.RefObject<HTMLIFrameElement | null>,
  enabled: boolean,
  onDotClick: (state: ParaDotState) => void,
) {
  // hoveredRef 仅用于悬停去重（同一悬停段不重复挂点），无需 state 触发宿主重渲染。
  const hoveredRef = useRef<HTMLElement | null>(null);
  const dotRef = useRef<HTMLButtonElement | null>(null);

  const removeDot = useCallback(() => {
    dotRef.current?.remove();
    dotRef.current = null;
  }, []);

  const placeDot = useCallback(
    (para: HTMLElement) => {
      const doc = getDoc();
      if (!doc?.body) return;
      const win = doc.defaultView;
      const rect = para.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      removeDot();
      // 文档坐标系（canvas）：rect + 滚动量，与分页锚点同一换算；无需任何定位祖先。
      // 垂直对齐：取段落首行文本的真实行盒（Range.getClientRects()[0]）
      // 居中——computed lineHeight 为 normal 时 parseFloat=NaN，旧回退 10px 导致红点偏上不对齐。
      let firstLineTop = rect.top;
      let firstLineHeight = rect.height;
      try {
        const range = doc.createRange();
        range.selectNodeContents(para);
        const firstRect = range.getClientRects()[0];
        if (firstRect && firstRect.height > 0) {
          firstLineTop = firstRect.top;
          firstLineHeight = firstRect.height;
        }
      } catch { /* 空段落/异常内容：退回段落盒 */ }
      const top = firstLineTop + (win?.scrollY ?? 0) + firstLineHeight / 2 - DOT_SIZE / 2;
      const left = Math.max(2, rect.left + (win?.scrollX ?? 0) - DOT_GUTTER_OFFSET);
      const dot = doc.createElement('button');
      dot.id = DOT_ID;
      dot.type = 'button';
      dot.setAttribute('aria-label', '段落助手');
      dot.setAttribute('data-mt-dot', '1');
      const style = dot.style;
      style.cssText = [
        'position:absolute',
        `left:${left}px`,
        `top:${top}px`,
        `width:${DOT_SIZE}px`,
        `height:${DOT_SIZE}px`,
        'border-radius:50%',
        'background:#c0392b',
        'opacity:.38',
        'border:0',
        'padding:0',
        'margin:0',
        'display:block',
        'cursor:pointer',
        'transition:opacity .18s ease, transform .18s ease',
        'box-shadow:0 0 0 0 rgba(192,57,43,0)',
      ].join(';');
      dot.addEventListener('mouseenter', () => {
        style.opacity = '.95';
        style.transform = 'scale(1.35)';
      });
      dot.addEventListener('mouseleave', () => {
        style.opacity = '.38';
        style.transform = 'scale(1)';
      });
      dot.addEventListener('click', (event) => {
        event.preventDefault();
        event.stopPropagation();
        const frame = iframeRef.current;
        if (!frame) return;
        const frameRect = frame.getBoundingClientRect();
        const dotRect = dot.getBoundingClientRect();
        // 双栏模式下段落含中英两列，取中文列文本（双语动作针对原文/译文一体处理）
        const zhCol = para.querySelector(':scope > .mt-zh, :scope > .mt-ch');
        const text = ((zhCol ?? para) as HTMLElement).innerText?.trim() ?? '';
        if (!text) return;
        onDotClick({
          paragraphText: text.slice(0, 3000),
          anchor: {
            top: frameRect.top + dotRect.bottom + 6,
            left: frameRect.left + dotRect.left + dotRect.width / 2,
          },
        });
      });
      // 挂载到根元素、绝对定位于段落左侧沟槽（与尾随 spacer 同一手法）——
      // 段落盒模型与首行文本完全不动，也不受 body 居中/定位影响
      doc.documentElement.appendChild(dot);
      dotRef.current = dot;
    },
    [getDoc, iframeRef, onDotClick, removeDot],
  );

  /* 触屏长按段落助手：移动端无 hover，单指长按段落 550ms 直接浮出动作菜单。
     与文本划选共存：按压中若发生划选（selection 非空）或移动超 10px 则取消，
     让位给原生划选与划选菜单。 */
  useEffect(() => {
    if (!enabled) return;
    const doc = getDoc();
    if (!doc?.body) return;

    let timer: number | null = null;
    let startX = 0;
    let startY = 0;
    let targetPara: HTMLElement | null = null;

    const cancel = () => {
      if (timer !== null) {
        window.clearTimeout(timer);
        timer = null;
      }
      targetPara = null;
    };

    const onTouchStart = (event: TouchEvent) => {
      if (event.touches.length !== 1) {
        cancel();
        return;
      }
      const touch = event.touches[0];
      const para = (touch.target as Element | null)?.closest?.('p');
      if (!para || para.hasAttribute('data-mt-dot')) return;
      targetPara = para as HTMLElement;
      startX = touch.clientX;
      startY = touch.clientY;
      timer = window.setTimeout(() => {
        timer = null;
        const paraEl = targetPara;
        targetPara = null;
        if (!paraEl) return;
        // 用户其实在划选文本：不弹段落菜单
        if (doc.getSelection?.()?.toString().trim()) return;
        const frame = iframeRef.current;
        if (!frame) return;
        const frameRect = frame.getBoundingClientRect();
        const zhCol = paraEl.querySelector(':scope > .mt-zh, :scope > .mt-ch');
        const text = ((zhCol ?? paraEl) as HTMLElement).innerText?.trim() ?? '';
        if (!text) return;
        onDotClick({
          paragraphText: text.slice(0, 3000),
          anchor: {
            top: frameRect.top + startY - 12,
            left: frameRect.left + startX,
          },
          placeAbove: true,
        });
      }, 550);
    };
    const onTouchMove = (event: TouchEvent) => {
      if (timer === null) return;
      const touch = event.touches[0];
      if (!touch || Math.hypot(touch.clientX - startX, touch.clientY - startY) > 10) cancel();
    };

    doc.addEventListener('touchstart', onTouchStart, { passive: true });
    doc.addEventListener('touchmove', onTouchMove, { passive: true });
    doc.addEventListener('touchend', cancel, { passive: true });
    doc.addEventListener('touchcancel', cancel, { passive: true });
    return () => {
      cancel();
      doc.removeEventListener('touchstart', onTouchStart);
      doc.removeEventListener('touchmove', onTouchMove);
      doc.removeEventListener('touchend', cancel);
      doc.removeEventListener('touchcancel', cancel);
    };
  }, [enabled, getDoc, iframeRef, onDotClick]);

  /* 悬停追踪：mouseover 捕获最近段落 */
  useEffect(() => {
    if (!enabled) return;
    const doc = getDoc();
    if (!doc?.body) return;

    // 注意：iframe 内元素属于 iframe 自己的 realm，`instanceof HTMLElement`（宿主 realm）
    // 恒为 false，曾导致红点永不出现。这里只判 tagName/hasAttribute，不跨 realm 判型。
    const isParagraph = (el: Element | null): el is HTMLElement =>
      !!el && el.tagName === 'P' && !el.hasAttribute('data-mt-dot');

    const onMouseOver = (event: MouseEvent) => {
      const target = event.target as Element | null;
      if (!target) return;
      // 悬停在红点自身：保持
      if ((target as HTMLElement).hasAttribute?.('data-mt-dot')) return;
      const para = target.closest?.('p');
      if (isParagraph(para)) {
        if (hoveredRef.current !== para) {
          hoveredRef.current = para;
          placeDot(para);
        }
      }
    };

    const onScroll = () => {
      // 翻页/滚动后红点位置失效，移除（待下次悬停重挂）
      hoveredRef.current = null;
      removeDot();
    };

    const onMouseLeaveDoc = (event: MouseEvent) => {
      // 离开文档（进入宿主 chrome）时收起
      if (!event.relatedTarget) {
        hoveredRef.current = null;
        removeDot();
      }
    };

    doc.addEventListener('mouseover', onMouseOver);
    doc.addEventListener('scroll', onScroll, { passive: true });
    doc.documentElement.addEventListener('mouseleave', onMouseLeaveDoc);
    return () => {
      doc.removeEventListener('mouseover', onMouseOver);
      doc.removeEventListener('scroll', onScroll);
      doc.documentElement.removeEventListener('mouseleave', onMouseLeaveDoc);
      removeDot();
    };
  }, [enabled, getDoc, placeDot, removeDot]);

  return { removeDot };
}
