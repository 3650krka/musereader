import { X } from 'lucide-react';
import { useEffect } from 'react';
import type { Book, TranslationTask } from '@/types';
import { formatTaskStatus, UI } from './libraryViewShared';
import { displayBookTitle } from './bookDisplay';

interface LibraryTaskHistorySheetProps {
  tasks: TranslationTask[];
  books: Book[];
  selectedTaskId?: string;
  onClose: () => void;
  onSelectTask: (task: TranslationTask) => void;
}

export function LibraryTaskHistorySheet({
  tasks,
  books,
  selectedTaskId,
  onClose,
  onSelectTask,
}: LibraryTaskHistorySheetProps) {
  /* 滚动穿透锁 */
  useEffect(() => {
    const prev = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = prev; };
  }, []);
  return (
    <div className="fixed inset-0 z-[109]">
      <button
        type="button"
        className="absolute inset-0 bg-black/20"
        onClick={onClose}
        aria-label={UI.closeHistory}
      />
      <aside className="task-detail-sheet">
        <div className="flex items-center justify-between" style={{ borderBottom: '1px solid var(--paper-3)', paddingBottom: 14 }}>
          <div>
            <p className="shelf-label" style={{ margin: 0, marginBottom: 6 }}>{UI.history}</p>
            <h3 className="text-xl font-semibold">{UI.taskHistory}</h3>
          </div>
          <button type="button" className="iconbtn" onClick={onClose} title={UI.close} aria-label={UI.close}>
            <X size={18} />
          </button>
        </div>
        <div className="mt-6 space-y-4">
          {tasks.map((task) => {
            const book = books.find((candidate) => candidate.id === task.id);
            const fallbackTitle = task.filename.replace(/\.[^.]+$/, '').replace(/[_-]+/g, ' ');
            const title = book ? displayBookTitle(book, fallbackTitle) : fallbackTitle;
            return (
              <button
                key={`history-${task.id}`}
                type="button"
                className="task-history-row w-full text-left"
                data-active={selectedTaskId === task.id}
                onClick={() => onSelectTask(task)}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="truncate font-medium">{title}</p>
                    <p className="mt-1 text-sm text-muted">{formatTaskStatus(task.status)}</p>
                  </div>
                  <span className="shrink-0 text-sm text-muted">{task.progress}%</span>
                </div>
              </button>
            );
          })}
        </div>
      </aside>
    </div>
  );
}
