import { invoke } from '@tauri-apps/api/core';
import { motion } from 'motion/react';
import {
  Check,
  Clock,
  Loader2,
  PauseCircle,
  Play,
  Plus,
  RotateCcw,
  Trash2,
  XCircle,
  type LucideIcon,
} from 'lucide-react';
import { useState } from 'react';
import { useT } from '@/i18n';
import type { AppView } from '@/appShared';
import type { Book, TranslationTask } from '@/types';

interface TasksViewProps {
  tasks: TranslationTask[];
  books: Book[];
  onDeleteTask: (taskId: string) => void;
  onOpenBook: (book: Book) => void;
  onNavigate: (view: AppView) => void;
}

function statusKey(status: TranslationTask['status']) {
  switch (status) {
    case 'completed': return 'done' as const;
    case 'failed': return 'failed' as const;
    case 'processing': return 'running' as const;
    case 'paused': return 'paused' as const;
    default: return 'pending' as const;
  }
}

export function TasksView({ tasks, books, onDeleteTask, onOpenBook, onNavigate }: TasksViewProps) {
  const t = useT();
  const sorted = [...tasks].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }}
      className="page-stage"
    >
      <header className="page-head">
        <h1 className="page-title">{t.tasks.title}</h1>
        <div className="actions">
          <button type="button" className="btn sm" onClick={() => onNavigate('library')}>
            <Plus size={14} />
            {t.tasks.new}
          </button>
        </div>
      </header>

      {sorted.length === 0 && <div className="empty-hint">{t.tasks.empty}</div>}

      <div className="flex flex-col gap-3">
        {sorted.map((task) => {
          const key = statusKey(task.status);
          const pct = task.totalChunks > 0 ? Math.round((task.translatedChunks / task.totalChunks) * 100) : task.progress;
          const book = task.status === 'completed' ? books.find((b) => b.id === task.id) : undefined;
          const tone = task.status === 'completed'
            ? 'done'
            : task.status === 'failed' || task.status === 'paused'
              ? task.status
              : '';
          return (
            <div key={task.id} className="card task-row">
              <div className={`task-spin ${tone}`}>
                {task.status === 'processing' ? (
                  <Loader2 size={18} className="spinning" />
                ) : task.status === 'completed' ? (
                  <Check size={16} strokeWidth={2.5} />
                ) : task.status === 'failed' ? (
                  <XCircle size={18} />
                ) : task.status === 'paused' ? (
                  <PauseCircle size={18} />
                ) : (
                  <Clock size={18} />
                )}
              </div>
              <div className="min-w-0 flex-1">
                <div className="truncate font-semibold" style={{ fontSize: 14 }}>{task.filename}</div>
                <div className="text-muted" style={{ fontSize: 12.5, marginTop: 2 }}>
                  {t.tasks[key]}
                  {task.status === 'completed' && task.updatedAt
                    ? ` · ${t.tasks.doneOn} ${task.updatedAt.slice(0, 10)}`
                    : ''}
                  {' · '}{task.translatedChunks}/{task.totalChunks} {t.tasks.chunks}
                </div>
              </div>
              {task.status === 'processing' && (
                <>
                  <div className="task-bar"><i style={{ width: `${pct}%` }} /></div>
                  <span className="task-pct">{pct}%</span>
                </>
              )}
              {book && (
                <button type="button" className="btn ghost sm" onClick={() => onOpenBook(book)}>
                  {t.tasks.open}
                </button>
              )}
              {task.status === 'paused' && (
                <TaskCommandButton
                  taskId={task.id}
                  label={t.tasks.resume}
                  command="resume_translation_task"
                  icon={Play}
                />
              )}
              {task.status === 'failed' && (
                <TaskCommandButton
                  taskId={task.id}
                  label={t.tasks.retry}
                  command="retry_translation_task"
                  icon={RotateCcw}
                />
              )}
              <button
                type="button"
                className="iconbtn danger"
                aria-label={t.tasks.cancel}
                onClick={() => void onDeleteTask(task.id)}
              >
                <Trash2 size={15} />
              </button>
            </div>
          );
        })}
      </div>
    </motion.div>
  );
}

function TaskCommandButton({
  taskId,
  label,
  command,
  icon: Icon,
}: {
  taskId: string;
  label: string;
  command: 'resume_translation_task' | 'retry_translation_task';
  icon: LucideIcon;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="btn ghost sm"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        try {
          await invoke(command, { taskId });
        } catch (error) {
          console.error(`${command} failed:`, error);
        } finally {
          setBusy(false);
        }
      }}
    >
      <Icon size={14} />
      {label}
    </button>
  );
}
