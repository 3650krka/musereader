import {
  AlertTriangle,
  BookOpen,
  ChevronDown,
  ExternalLink,
  FileText,
  FolderOpen,
  Play,
  RotateCcw,
  Square,
  X,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import type { Book, RuntimeNotice, TranslationTask } from '@/types';
import {
  formatRetryAfter,
  formatTaskStatus,
  UI,
} from './libraryViewShared';
import {
  formatNoticeCodeLabel,
  formatNoticeScope,
  noticeToneClassName,
} from './runtimeNoticeShared';
import { displayBookTitle } from './bookDisplay';

interface LibraryTaskDetailSheetProps {
  task: TranslationTask;
  book?: Book;
  onClose: () => void;
  onCancel: (task: TranslationTask) => void;
  onResume: (task: TranslationTask) => void;
  onRetry: (task: TranslationTask) => void;
  onOpenArtifact: (path?: string) => void;
  onRevealArtifact: (path?: string) => void;
  notices: RuntimeNotice[];
  onClearNotices: (taskId: string) => void;
}

export function LibraryTaskDetailSheet({
  task,
  book,
  onClose,
  onCancel,
  onResume,
  onRetry,
  onOpenArtifact,
  onRevealArtifact,
  notices,
  onClearNotices,
}: LibraryTaskDetailSheetProps) {
  const primaryEpubPath = task.artifactPaths.bilingualEpubPath
    ?? task.artifactPaths.translatedEpubPath;
  const primaryEpubLabel = task.artifactPaths.bilingualEpubPath
    ? UI.openBilingualEpub
    : UI.openTranslatedEpub;
  const fallbackTitle = task.filename.replace(/\.[^.]+$/, '').replace(/[_-]+/g, ' ');
  const taskTitle = book ? displayBookTitle(book, fallbackTitle) : fallbackTitle;

  return (
    <div className="fixed inset-0 z-[110]">
      <button
        type="button"
        className="absolute inset-0 bg-black/20"
        onClick={onClose}
        aria-label={UI.closeTaskDetail}
      />
      <aside className="task-detail-sheet">
        <div className="flex items-center justify-between" style={{ borderBottom: '1px solid var(--paper-3)', paddingBottom: 14 }}>
          <div>
            <p className="shelf-label" style={{ margin: 0, marginBottom: 6 }}>{UI.taskDetail}</p>
            <h3 className="text-xl font-semibold">{taskTitle}</h3>
            {book?.author && <p className="mt-1 text-sm text-muted">{book.author}</p>}
          </div>
          <button type="button" className="iconbtn" onClick={onClose} title={UI.close} aria-label={UI.close}>
            <X size={18} />
          </button>
        </div>
        <div className="task-detail-stack">
          <div className="task-detail-row">
            <span className="text-sm text-muted">{UI.status}</span>
            <span className="font-medium">{formatTaskStatus(task.status)}</span>
          </div>
          <div className="task-detail-progress-block">
            <div className="mb-3 flex items-center justify-between">
              <span className="text-sm text-muted">{UI.progress}</span>
              <span className="font-mono text-sm">{task.progress}%</span>
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-[var(--paper-2)]">
              <div
                className="h-full bg-[var(--accent)]"
                style={{ width: `${task.progress}%` }}
              />
            </div>
            <p className="mt-4 text-sm text-muted">{task.lastError?.message ?? task.message}</p>
            {task.lastError && (
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                <span className="rounded-full bg-[var(--paper-2)] px-3 py-1 text-muted">
                  {task.lastError.code}
                </span>
                {task.lastError.retryable && (
                  <span className="rounded-full bg-[var(--paper-2)] px-3 py-1 text-muted">
                    {UI.retryable}
                  </span>
                )}
                {formatRetryAfter(task.lastError.retryAfterMs) && (
                  <span className="rounded-full bg-[var(--paper-2)] px-3 py-1 text-muted">
                    {formatRetryAfter(task.lastError.retryAfterMs)}
                  </span>
                )}
              </div>
            )}
          </div>
          <div className="task-detail-row">
            <span className="text-sm text-muted">{UI.createdAt}</span>
            <span className="font-medium">{new Date(task.createdAt).toLocaleString()}</span>
          </div>
          <div className="task-detail-actions">
            {(task.status === 'pending' || task.status === 'processing') && (
              <button
                type="button"
                className="btn ghost sm task-detail-action"
                onClick={() => onCancel(task)}
              >
                <Square size={15} />
                <span>{UI.cancel}</span>
              </button>
            )}
            {task.status === 'failed' && (
              <button
                type="button"
                className="btn ghost sm task-detail-action"
                onClick={() => onRetry(task)}
              >
                <RotateCcw size={15} />
                <span>{UI.retry}</span>
              </button>
            )}
            {task.status === 'paused' && (
              <button
                type="button"
                className="btn ghost sm task-detail-action"
                onClick={() => onResume(task)}
              >
                <Play size={15} />
                <span>{UI.resume}</span>
              </button>
            )}
            {primaryEpubPath && (
              <button
                type="button"
                className="btn sm task-detail-action"
                onClick={() => onOpenArtifact(primaryEpubPath)}
              >
                <BookOpen size={15} />
                <span>{primaryEpubLabel}</span>
              </button>
            )}
          </div>

          <details className="task-detail-disclosure">
            <summary>
              <span>{UI.proofingAndDiagnostics}</span>
              <ChevronDown size={16} aria-hidden="true" />
            </summary>
            <div className="task-detail-disclosure-body">
              <div className="task-detail-row">
                <span className="text-sm text-muted">{UI.type}</span>
                <span className="font-medium">{task.articleType}</span>
              </div>
              <div className="task-detail-row">
                <span className="text-sm text-muted">{UI.threads}</span>
                <span className="font-medium">{task.maxWorkers}</span>
              </div>
              <div className="task-detail-row">
                <span className="text-sm text-muted">{UI.concurrent}</span>
                <span className="font-medium">{task.concurrent ? UI.on : UI.off}</span>
              </div>

              <div className="space-y-3 border-t border-line pt-4">
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <p className="text-sm text-muted">{UI.diagnostics}</p>
                    <p className="text-xs text-muted">
                      {notices.length > 0 ? String(notices.length) : UI.noDiagnostics}
                    </p>
                  </div>
                  {notices.length > 0 && (
                    <button
                      type="button"
                      className="btn ghost sm task-detail-action"
                      onClick={() => onClearNotices(task.id)}
                    >
                      <X size={14} />
                      <span>{UI.clearDiagnostics}</span>
                    </button>
                  )}
                </div>
                <p className="text-xs text-muted">{UI.diagnosticsHint}</p>
                {notices.length > 0 && (
                  <div className="space-y-2">
                    {notices.map((notice) => (
                      <div
                        key={`${notice.timestamp}-${notice.scope}-${notice.detail}`}
                        className="rounded-[8px] border border-line bg-[var(--paper-2)] px-3 py-3"
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <div className="flex items-center gap-2 text-xs text-muted">
                              <AlertTriangle size={14} className={noticeToneClassName(notice.code)} />
                              <span>{formatNoticeScope(notice.scope)}</span>
                              <span>{formatNoticeCodeLabel(notice.code)}</span>
                            </div>
                            <p className="mt-2 text-sm text-foreground/85">{notice.message}</p>
                            <p className="mt-1 text-xs text-muted">{notice.detail}</p>
                          </div>
                          <span className="shrink-0 text-[11px] text-muted">
                            {new Date(notice.timestamp).toLocaleTimeString()}
                          </span>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              <TaskMetricsPanel metricsPath={task.artifactPaths.metricsPath} />

              <div className="task-detail-actions">
                {task.artifactPaths.bilingualEpubPath && task.artifactPaths.translatedEpubPath && (
                  <button
                    type="button"
                    className="btn ghost sm task-detail-action"
                    onClick={() => onOpenArtifact(task.artifactPaths.translatedEpubPath)}
                  >
                    <BookOpen size={15} />
                    <span>{UI.openTranslatedEpub}</span>
                  </button>
                )}
                {task.htmlOutputPath && (
                  <button
                    type="button"
                    className="btn ghost sm task-detail-action"
                    onClick={() => onOpenArtifact(task.htmlOutputPath)}
                  >
                    <ExternalLink size={15} />
                    <span>{UI.openHtml}</span>
                  </button>
                )}
                {task.artifactPaths.validationReportPath && (
                  <button
                    type="button"
                    className="btn ghost sm task-detail-action"
                    onClick={() => onOpenArtifact(task.artifactPaths.validationReportPath)}
                  >
                    <FileText size={15} />
                    <span>{UI.validation}</span>
                  </button>
                )}
                {task.artifactPaths.eventLogPath && (
                  <button
                    type="button"
                    className="btn ghost sm task-detail-action"
                    onClick={() => onOpenArtifact(task.artifactPaths.eventLogPath)}
                  >
                    <FileText size={15} />
                    <span>{UI.events}</span>
                  </button>
                )}
                {(primaryEpubPath || task.htmlOutputPath || task.outputPath || task.artifactPaths.manifestPath) && (
                  <button
                    type="button"
                    className="btn ghost sm task-detail-action"
                    onClick={() => onRevealArtifact(
                      primaryEpubPath
                        ?? task.htmlOutputPath
                        ?? task.outputPath
                        ?? task.artifactPaths.manifestPath,
                    )}
                  >
                    <FolderOpen size={15} />
                    <span>{UI.reveal}</span>
                  </button>
                )}
              </div>
            </div>
          </details>
        </div>
      </aside>
    </div>
  );
}


/** 翻译指标面板：任务完成后读取 metrics.json，展示耗时/Token/速度/失败请求/成本。 */
interface TaskMetrics {
  elapsedMs?: number;
  llmTotalTokens?: number;
  llmAttemptFailedEventCount?: number;
  estimatedDeepseekV4FlashCostUsd?: number | null;
  translatedChunks?: number;
  totalChunks?: number;
}

function formatDuration(ms?: number): string {
  if (!ms) return '—';
  const totalSec = Math.round(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const sec = totalSec % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

function TaskMetricsPanel({ metricsPath }: { metricsPath?: string }) {
  const [metrics, setMetrics] = useState<TaskMetrics | null>(null);
  const [unavailable, setUnavailable] = useState(false);

    /* 滚动穿透锁：模态打开期间冻结底层页面滚动。 */
  useEffect(() => {
    const prev = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = prev; };
  }, []);

  useEffect(() => {
    if (!metricsPath) return;
    let cancelled = false;
    invoke<string>('read_translated_file', { path: metricsPath })
      .then((raw) => {
        if (cancelled) return;
        try { setMetrics(JSON.parse(raw) as TaskMetrics); } catch { setUnavailable(true); }
      })
      .catch(() => { if (!cancelled) setUnavailable(true); });
    return () => { cancelled = true; };
  }, [metricsPath]);

  const speed = metrics?.elapsedMs && metrics?.translatedChunks
    ? Math.round(metrics.translatedChunks / (metrics.elapsedMs / 1000) * 10) / 10
    : null;

  return (
    <div className="space-y-3 border-t border-line pt-4">
      <p className="text-sm text-muted">{UI.metricsTitle}</p>
      {!metrics && (
        <p className="text-xs text-muted">{unavailable ? '—' : UI.metricsUnavailable}</p>
      )}
      {metrics && (
        <div className="metrics-grid">
          <div className="metric">
            <span className="metric-k">{UI.metricsElapsed}</span>
            <span className="metric-v">{formatDuration(metrics.elapsedMs)}</span>
          </div>
          <div className="metric">
            <span className="metric-k">{UI.metricsTokens}</span>
            <span className="metric-v">{(metrics.llmTotalTokens ?? 0).toLocaleString()}</span>
          </div>
          <div className="metric">
            <span className="metric-k">{UI.metricsSpeed}</span>
            <span className="metric-v">{speed != null ? `${speed} 块/s` : '—'}</span>
          </div>
          <div className="metric">
            <span className="metric-k">{UI.metricsFailed}</span>
            <span className="metric-v">{metrics.llmAttemptFailedEventCount ?? 0}</span>
          </div>
          {metrics.estimatedDeepseekV4FlashCostUsd != null && (
            <div className="metric">
              <span className="metric-k">{UI.metricsCost}</span>
              <span className="metric-v">${metrics.estimatedDeepseekV4FlashCostUsd.toFixed(4)}</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
