import { motion } from 'motion/react';
import { createPortal } from 'react-dom';
import { ArrowLeft } from 'lucide-react';
import { useEffect } from 'react';
import { useT } from '@/i18n';

/**
 * 全屏背诵子界面（Anki 式独立全屏，非右栏内嵌）：
 *  - 独立层级（fixed 覆盖应用），顶栏返回 + 标题
 *  - 内容区由牌组组件渲染（卡片居中放大 + 底部四档按钮）
 *  - Esc 退出；卡片翻面与评分快捷键由牌组组件处理
 */
export function StudyOverlay({ title, onExit, children }: {
  title: string;
  onExit: () => void;
  children: React.ReactNode;
}) {
  const t = useT();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      // 输入框内不拦截
      if (target?.closest('input, textarea, select, [contenteditable]')) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        onExit();
      }
    };
    window.addEventListener('keydown', onKey);
    /* 滚动穿透锁：全屏期间冻结底层页面滚动。 */
    const prev = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      window.removeEventListener('keydown', onKey);
      document.body.style.overflow = prev;
    };
  }, [onExit]);

  return createPortal(
    <motion.div
      className="study-overlay"
      initial={{ opacity: 0, y: 24 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 24 }}
      transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
      role="dialog"
      aria-label={title}
    >
      <header className="study-top">
        <button
          type="button"
          className="iconbtn"
          onClick={onExit}
          aria-label={t.reader.back}
          title={t.reader.back}
        >
          <ArrowLeft size={20} />
        </button>
        <span className="study-top-title truncate">{title}</span>
        <span style={{ width: 44 }} aria-hidden="true" />
      </header>
      <div className="study-body">{children}</div>
    </motion.div>,
    document.body,
  );
}
