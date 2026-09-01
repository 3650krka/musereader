import { useCallback, useEffect, useRef, useState } from 'react';
import type { FlipSheetState } from './PageFlipSheet';
import type { ReaderFrameApi } from './epubReaderTypes';
import type { ReaderPrefs } from './useReaderPrefs';

interface UsePageFlipOptions {
  epub: ReaderFrameApi;
  prefs: ReaderPrefs;
  /** 划选浮窗打开时自动翻页让位（手动翻页不受影响）。 */
  selectionActive: boolean;
  /** 校对模式下自动翻页停用（点段落编辑与定时翻页互斥）。 */
  reviewMode: boolean;
}

/** 点击触发的 3D 纸页翻页（折页快照层）：翻页前捕获旧页快照，纸页绕书脊投影翻走。
    无拖拽——与划选/唤出等触发区零冲突（冲突根源是 pointerdown 起手劫持，已整体移除）。
    从 EpubReaderView 抽出的自封闭机制层：快照捕获、忙碌闸、自动翻页定时、章切复位。 */
export function usePageFlip({ epub, prefs, selectionActive, reviewMode }: UsePageFlipOptions) {
  const [flipSheet, setFlipSheet] = useState<FlipSheetState | null>(null);
  const flipBusyRef = useRef(false);
  const pageInfoRef = useRef(epub.pageInfo);
  useEffect(() => { pageInfoRef.current = epub.pageInfo; }, [epub.pageInfo]);

  const getDoc = epub.getDoc;
  const iframeRef = epub.iframeRef;

  /** 捕获折页所需的列快照：spread 取左/右列（列起点 scrollLeft 定位），单页取整幅。 */
  const captureColumnSnapshot = useCallback(
    (column: 'left' | 'right' | 'full'): FlipSheetState['front'] | null => {
      const doc = getDoc();
      const frame = iframeRef.current;
      if (!doc?.body || !frame) return null;
      const styles = Array.from(doc.head.querySelectorAll('style'))
        .map((el) => el.textContent ?? '')
        .join('\n');
      const viewW = frame.clientWidth;
      const maxScroll = Math.max(0, (doc.documentElement?.scrollWidth ?? 0) - viewW);
      const base = doc.documentElement?.scrollLeft ?? doc.body.scrollLeft ?? 0;
      let scrollLeft = base;
      if (column === 'right') {
        scrollLeft = Math.min(base + viewW / 2, maxScroll);
      }
      return {
        html: doc.body.innerHTML,
        css: styles,
        scrollLeft,
        scrollTop: doc.documentElement?.scrollTop ?? doc.body.scrollTop ?? 0,
        width: viewW,
        height: frame.clientHeight,
      };
    },
    [getDoc, iframeRef],
  );

  /** 组装纸页翻页态（统一「旧页翻走」模型）：
      portrait=旧整页满幅；spread=活动列折页+对面旧列垫底（防 iframe 跳变穿帮）。 */
  const captureFlipState = useCallback(
    (direction: 'forward' | 'back'): FlipSheetState | null => {
      const isSpread = prefs.flow === 'spread';
      if (!isSpread) {
        const full = captureColumnSnapshot('full');
        return full ? { front: full, under: null, direction } : null;
      }
      const sheet = captureColumnSnapshot(direction === 'forward' ? 'right' : 'left');
      const under = captureColumnSnapshot(direction === 'forward' ? 'left' : 'right');
      return sheet ? { front: sheet, under, direction } : null;
    },
    [captureColumnSnapshot, prefs.flow],
  );

  /* 纸页风格：book（仿真纸页 620ms）与 flip（快速翻页 380ms）均点击触发、共用 3D 翻页层。
     scrolled 模式不叠翻页层（纵向滚动无页可折，快照层会盖住滚动内容）。 */
  const foldStyle =
    prefs.flow !== 'scrolled' &&
    (prefs.pageTurnStyle === 'book' || prefs.pageTurnStyle === 'flip');

  const flipNext = useCallback(() => {
    if (foldStyle && !flipBusyRef.current) {
      const st = captureFlipState('forward');
      if (st) {
        flipBusyRef.current = true;
        setFlipSheet(st);
      }
    }
    epub.nextPage();
  }, [captureFlipState, epub, foldStyle]);

  const flipPrev = useCallback(() => {
    if (foldStyle && !flipBusyRef.current) {
      const st = captureFlipState('back');
      if (st) {
        flipBusyRef.current = true;
        setFlipSheet(st);
      }
    }
    epub.prevPage();
  }, [captureFlipState, epub, foldStyle]);

  /** 动画完成回调：解除忙碌闸并撤快照层。 */
  const finishFlip = useCallback(() => {
    flipBusyRef.current = false;
    setFlipSheet(null);
  }, []);

  const flipNextRef = useRef(flipNext);
  useEffect(() => { flipNextRef.current = flipNext; });

  /* 自动翻页（Y1：死设置修复）：开启时按间隔定时下一页；
     交互中（翻页动画进行中/划选浮窗打开/校对模式）跳过该拍。 */
  useEffect(() => {
    if (!prefs.autoFlip || prefs.flow === 'scrolled' || reviewMode) return;
    const timer = window.setInterval(() => {
      if (flipBusyRef.current || selectionActive) return;
      const { page, total } = pageInfoRef.current;
      if (page >= total - 1) return; // 末页驻留，不反复触发同页滚动
      flipNextRef.current();
    }, Math.max(5, prefs.autoFlipInterval) * 1000);
    return () => window.clearInterval(timer);
  }, [prefs.autoFlip, prefs.autoFlipInterval, prefs.flow, reviewMode, selectionActive]);

  /* 章切/换书：TOC/搜索跳转时复位翻页层，防止旧章快照盖新章。 */
  useEffect(() => {
    flipBusyRef.current = false;
    setFlipSheet(null);
  }, [epub.sectionKey]);

  return { flipSheet, flipNext, flipPrev, finishFlip, flipBusyRef };
}
