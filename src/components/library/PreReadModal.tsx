import { AnimatePresence, motion } from 'motion/react';
import { ArrowRight, Clock, Plus, Star, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useT } from '@/i18n';
import type { Book } from '@/types';
import { displayBookTitle } from './bookDisplay';

interface PreReadModalProps {
  book: Book | null;
  onClose: () => void;
  onStartReading: (book: Book) => void;
  onToggleFavorite: (bookId: string, favorite: boolean) => void;
  onSetTags: (bookId: string, tags: string[]) => void;
}

export function PreReadModal({ book, onClose, onStartReading, onToggleFavorite, onSetTags }: PreReadModalProps) {
  const t = useT();
  const [tagDraft, setTagDraft] = useState('');
  /* 滚动穿透锁：book 存在（模态可见）期间冻结底层滚动。 */
  useEffect(() => {
    if (!book) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = prev; };
  }, [book]);

  if (!book) return null;

  const addTag = () => {
    const tag = tagDraft.trim();
    if (!tag) return;
    if (!(book.tags ?? []).includes(tag)) {
      onSetTags(book.id, [...(book.tags ?? []), tag]);
    }
    setTagDraft('');
  };

  const progressLabel = book.progress > 0 ? `${book.progress}%` : t.library.notStarted;
  const title = displayBookTitle(book, t.library.untitledBook);

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-[100] flex items-end justify-center p-0 md:items-center md:p-6">
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
          onClick={onClose}
          className="absolute inset-0 bg-black/40 backdrop-blur-sm"
        />

        <motion.div
          initial={{ opacity: 0, y: 24, scale: 0.985 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 20, scale: 0.99 }}
          transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
          className="pre-read-sheet relative w-full max-w-[880px]"
        >
          <div className="grid md:grid-cols-[232px_minmax(0,1fr)]">
            <div className="border-b border-line p-8 md:border-b-0 md:border-r">
              {/* 移动端关闭钮：用包裹 div 控制可见性——.icon-button 是未分层样式，
                  会压过 Tailwind 分层工具类（md:hidden 对其无效），不可直接加在按钮上 */}
              <div className="absolute right-4 top-4 md:hidden">
                <button onClick={onClose} className="icon-button" title={t.library.close}>
                  <X size={20} />
                </button>
              </div>
              <div className="book-spine mx-auto w-[184px] md:w-[184px]">
                <img src={book.cover} className="h-full w-full object-cover" alt={title} />
              </div>
              <div className="mt-6 text-center md:text-left">
                <h3 className="font-display text-2xl leading-tight">{title}</h3>
                <p className="mt-2 text-sm text-muted">{book.author}</p>
              </div>
            </div>

            <div className="p-8">
              <div className="mb-8 flex items-center justify-between">
                <p className="shelf-label" style={{ margin: 0 }}>{t.library.ready}</p>
                <div className="hidden md:block">
                  <button onClick={onClose} className="icon-button" title={t.library.close}>
                    <X size={20} />
                  </button>
                </div>
              </div>

              <div className="space-y-8">
                <div className="grid gap-4">
                  <div className="row2">
                    <div className="grow">
                      <div className="name">{t.library.category}</div>
                      <div className="desc">{book.category}</div>
                    </div>
                    <div style={{ textAlign: 'right' }}>
                      <div className="name" style={{ justifyContent: 'flex-end' }}>{t.library.level}</div>
                      <div className="desc">{book.level}</div>
                    </div>
                  </div>

                  <div className="row2">
                    <div className="grow">
                      <div className="name"><Clock size={15} style={{ color: 'var(--signal)' }} />{t.library.progress}</div>
                    </div>
                    <span className="chip">{progressLabel}</span>
                  </div>
                </div>

                {/* 收藏 + 标签（book_profiles.json 持久化） */}
                <div className="row2" style={{ alignItems: 'center' }}>
                  <div className="grow">
                    <div className="name">{t.library.tags}</div>
                    <div className="preread-tags">
                      {(book.tags ?? []).map((tag) => (
                        <span key={tag} className="chip">
                          {tag}
                          <button
                            type="button"
                            className="tag-x"
                            aria-label={`${t.library.removeTag} ${tag}`}
                            onClick={() => onSetTags(book.id, (book.tags ?? []).filter((x) => x !== tag))}
                          >
                            <X size={10} />
                          </button>
                        </span>
                      ))}
                      <span className="tag-add">
                        <Plus size={11} />
                        <input
                          value={tagDraft}
                          onChange={(e) => setTagDraft(e.target.value)}
                          onKeyDown={(e) => e.key === 'Enter' && addTag()}
                          placeholder={t.library.addTag}
                        />
                      </span>
                    </div>
                  </div>
                  <button
                    type="button"
                    className={`iconbtn ${book.favorite ? 'on' : ''}`}
                    style={{ width: 38, height: 38, flex: 'none' }}
                    aria-label={t.library.favorite}
                    aria-pressed={Boolean(book.favorite)}
                    onClick={() => onToggleFavorite(book.id, !book.favorite)}
                  >
                    <Star size={16} fill={book.favorite ? 'currentColor' : 'none'} />
                  </button>
                </div>

                <div className="flex flex-wrap gap-2">
                  <span className="chip">{book.category}</span>
                  <span className="chip">{t.library.bilingual}</span>
                </div>

                <div className="flex justify-end">
                  <button
                    onClick={() => {
                      onStartReading(book);
                      onClose();
                    }}
                    className="btn"
                    style={{ minWidth: 180, justifyContent: 'space-between' }}
                  >
                    <span>{t.library.startReading}</span>
                    <ArrowRight size={16} />
                  </button>
                </div>
              </div>
            </div>
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
}
