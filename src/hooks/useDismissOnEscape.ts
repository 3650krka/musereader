import { useEffect } from 'react';

/**
 * 统一浮层关闭：Escape 触发 onClose。传 null 表示当前未激活（不挂监听）。
 * 多个浮层并存时各自独立挂接，行为一致，替代散落的重复 keydown 样板。
 */
export function useDismissOnEscape(onClose: (() => void) | null) {
  useEffect(() => {
    if (!onClose) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
}
