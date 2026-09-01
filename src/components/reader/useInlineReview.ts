import { useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ReviewSegment } from '@/types';

/** 阅读器内联校对：reviewMode 开启后点击正文段落即可直接改译文，
    失焦保存（Esc 取消），通过文本匹配定位 segment 后走既有 save_review_edit 修订层。 */

interface UseInlineReviewOptions {
  getDoc: () => Document | null;
  /** 内联校对总开关（阅读器校对模式）。 */
  enabled: boolean;
  taskId: string;
}

export function useInlineReview({ getDoc, enabled, taskId }: UseInlineReviewOptions) {
  const segmentsRef = useRef<ReviewSegment[]>([]);
  const loadedRef = useRef(false);
  const editingRef = useRef<HTMLElement | null>(null);

  /* 首次开启时拉取段对（按书缓存于组件生命周期内）。 */
  useEffect(() => {
    if (!enabled || loadedRef.current) return;
    loadedRef.current = true;
    invoke<ReviewSegment[]>('list_review_segments', { taskId })
      .then((rows) => { segmentsRef.current = rows; })
      .catch(() => { segmentsRef.current = []; });
  }, [enabled, taskId]);

  /* 按可见译文匹配 segment（容差：取相似度最高的条目）。 */
  const matchSegment = useCallback((text: string): ReviewSegment | null => {
    const normalized = text.replace(/\s+/g, ' ').trim();
    if (!normalized) return null;
    let best: ReviewSegment | null = null;
    let bestScore = 0;
    for (const seg of segmentsRef.current) {
      const target = seg.effectiveTarget.replace(/\s+/g, ' ').trim();
      if (!target) continue;
      if (target === normalized) return seg; // 精确命中
      const common = commonPrefixLength(target, normalized);
      const score = common / Math.max(target.length, normalized.length, 1);
      if (score > bestScore) {
        bestScore = score;
        best = seg;
      }
    }
    return bestScore >= 0.6 ? best : null;
  }, []);

  /* iframe 内点击 → 段落变 contenteditable；失焦保存 / Esc 取消。 */
  useEffect(() => {
    const doc = getDoc();
    if (!doc?.body || !enabled) return;

    const onClick = (event: Event) => {
      const target = event.target as HTMLElement | null;
      if (!target) return;
      const para = target.closest<HTMLElement>('p, [data-mt-pair]');
      if (!para || editingRef.current) return;
      event.stopPropagation();
      startEditing(para);
    };

    const startEditing = (el: HTMLElement) => {
      editingRef.current = el;
      const original = el.textContent ?? '';
      el.setAttribute('contenteditable', 'true');
      el.classList.add('mt-inline-edit');
      el.focus();
      const onBlur = () => {
        cleanup();
        const next = el.textContent ?? '';
        if (next.trim() !== original.trim()) {
          void saveEdit(original, next);
        }
      };
      const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === 'Escape') {
          el.textContent = original; // Esc 还原
          el.blur();
        }
      };
      const cleanup = () => {
        el.removeAttribute('contenteditable');
        el.classList.remove('mt-inline-edit');
        el.removeEventListener('blur', onBlur);
        el.removeEventListener('keydown', onKeyDown);
        editingRef.current = null;
      };
      el.addEventListener('blur', onBlur);
      el.addEventListener('keydown', onKeyDown);
    };

    const saveEdit = async (originalText: string, nextText: string) => {
      const seg = matchSegment(originalText);
      if (!seg) return; // 未匹配到段对（无译文映射的页面）：仅本地生效
      try {
        const next = await invoke<ReviewSegment[]>('save_review_edit', {
          taskId,
          blockId: seg.blockId,
          editedTarget: nextText,
          comment: null,
        });
        segmentsRef.current = next;
      } catch (error) {
        console.error('inline review save failed:', error);
      }
    };

    doc.addEventListener('click', onClick);
    return () => doc.removeEventListener('click', onClick);
  }, [getDoc, enabled, matchSegment, taskId]);

  return {
    /** 当前段落数（供提示展示）。 */
    segmentCount: () => segmentsRef.current.length,
  };
}

function commonPrefixLength(a: string, b: string): number {
  const len = Math.min(a.length, b.length);
  let i = 0;
  while (i < len && a[i] === b[i]) i++;
  return i;
}
