import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { BookOpen, BookText, Languages, Package, PenLine, Star, Trash2 } from 'lucide-react';
import { useT } from '@/i18n';
import type { Book } from '@/types';

/** 书库右键菜单：打开/收藏/翻译/词汇透析/校对/详情/删除。 */
export interface BookContextMenuState {
  x: number;
  y: number;
  book: Book;
}

interface BookContextMenuProps {
  state: BookContextMenuState;
  onClose: () => void;
  onOpen: (book: Book) => void;
  onToggleFavorite: (bookId: string, favorite: boolean) => void;
  onTranslate: (book: Book) => void;
  onVocab: (book: Book) => void;
  onReview: (book: Book) => void;
  onDetail: (book: Book) => void;
  /** 导出单书迁移包（工件+进度+笔记 zip）。 */
  onExport: (book: Book) => void;
  onDelete: (taskId: string) => void;
}

export function BookContextMenu({
  state,
  onClose,
  onOpen,
  onToggleFavorite,
  onTranslate,
  onVocab,
  onReview,
  onDetail,
  onExport,
  onDelete,
}: BookContextMenuProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onDown = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('pointerdown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [onClose]);

  /* 视口钳制：菜单不超出屏幕 */
  const menuWidth = 190;
  const left = Math.min(state.x, window.innerWidth - menuWidth - 12);
  const top = Math.min(state.y, window.innerHeight - 340);

  const act = (run: () => void) => {
    run();
    onClose();
  };

  return createPortal(
    <div ref={ref} className="book-ctx-menu" style={{ left, top }} role="menu">
      <button type="button" onClick={() => act(() => onOpen(state.book))}>
        <BookOpen size={14} />
        {t.library.ctxOpen}
      </button>
      <button type="button" onClick={() => act(() => onToggleFavorite(state.book.id, !state.book.favorite))}>
        <Star size={14} />
        {state.book.favorite ? t.library.ctxUnfavorite : t.library.ctxFavorite}
      </button>
      <button type="button" onClick={() => act(() => onTranslate(state.book))}>
        <Languages size={14} />
        {t.library.ctxRetranslate}
      </button>
      <button type="button" onClick={() => act(() => onVocab(state.book))}>
        <BookText size={14} />
        {t.library.ctxVocab}
      </button>
      <button type="button" onClick={() => act(() => onReview(state.book))}>
        <PenLine size={14} />
        {t.library.ctxReview}
      </button>
      <button type="button" onClick={() => act(() => onDetail(state.book))}>
        <BookOpen size={14} className="ctx-muted" />
        {t.library.ctxDetail}
      </button>
      <button type="button" onClick={() => act(() => onExport(state.book))}>
        <Package size={14} />
        {t.library.exportBundle}
      </button>
      <button type="button" className="ctx-danger" onClick={() => act(() => onDelete(state.book.id))}>
        <Trash2 size={14} />
        {t.library.ctxDelete}
      </button>
    </div>,
    document.body,
  );
}
