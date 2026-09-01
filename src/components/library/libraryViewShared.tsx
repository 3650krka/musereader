import {
  AlertCircle,
  CheckCircle,
  Clock,
  Loader2,
} from 'lucide-react';
import type { TranslationTask } from '@/types';

export const UI = {
  completed: '已完成',
  failed: '失败',
  processing: '处理中',
  paused: '已暂停',
  pending: '等待中',
  essay: '随笔',
  research: '研究',
  fiction: '小说',
  all: '全部',
  selectDocument: '请选择 PDF 或 EPUB',
  importFailed: '导入失败：',
  captureFailed: '网页剪藏失败：',
  capturePlaceholder: '粘贴网页链接，剪藏为文档…',
  capture: '剪藏',
  selectDocumentFirst: '请先选择文件',
  taskCreated: '任务已创建',
  startFailed: '启动失败：',
  searchBook: '搜索书名',
  noBooks: '还没有书',
  noMatch: '未找到',
  continueReading: '继续阅读',
  books: '本',
  processingTitle: '处理',
  unselected: '未选择',
  chooseFile: '选择文件',
  remove: '移除',
  noFile: '未选择文件',
  fileHint: '导入后会直接进入处理队列',
  type: '类型',
  academic: '学术论文',
  fictionType: '小说',
  nonfiction: '非虚构',
  textbook: '教材',
  threads: '线程',
  readyToStart: '可开始处理',
  waitingFile: '等待文件',
  start: '开始',
  startDisabled: '先选文件',
  recentTasks: '最近任务',
  noTasks: '暂无任务',
  history: '查看历史',
  collection: '馆藏',
  closeTaskDetail: '关闭任务详情',
  taskDetail: '任务详情',
  close: '关闭',
  status: '状态',
  concurrent: '并发',
  on: '开启',
  off: '关闭',
  progress: '进度',
  createdAt: '创建时间',
  closeHistory: '关闭任务历史',
  taskHistory: '任务历史',
  cancel: '取消',
  resume: '继续',
  retry: '重试',
  openBilingualEpub: '打开双语 EPUB',
  openTranslatedEpub: '打开中文版 EPUB',
  proofingAndDiagnostics: '校对与诊断',
  metricsTitle: '翻译指标',
  metricsElapsed: '总耗时',
  metricsTokens: 'Tokens',
  metricsSpeed: '平均速度',
  metricsFailed: '失败请求',
  metricsCost: '估算成本',
  metricsUnavailable: '任务完成后生成',
  openHtml: '打开 HTML',
  validation: '校验报告',
  events: '事件日志',
  reveal: '打开目录',
  actionFailed: '操作失败：',
  retryable: '可重试',
  retryAfter: '建议等待',
  diagnostics: '运行诊断',
  diagnosticsHint: '以下提示用于定位运行边界问题，不等同于任务失败。',
  noDiagnostics: '暂无诊断事件',
  clearDiagnostics: '清空诊断',
  recentDiagnostics: '最近诊断',
  clearAllDiagnostics: '清空全部',
  taskRunner: '后台任务',
  runtimeLayer: '运行时',
  runtimeAdvisory: '诊断提示',
  runtimeInternal: '内部处理提示',
  seconds: '秒',
  delete: '删除',
} as const;

export function formatTaskStatus(status: TranslationTask['status']) {
  switch (status) {
    case 'completed':
      return UI.completed;
    case 'failed':
      return UI.failed;
    case 'processing':
      return UI.processing;
    case 'paused':
      return UI.paused;
    default:
      return UI.pending;
  }
}

const PHASE_LABELS: Record<string, string> = {
  imported: '已导入',
  ocrRunning: '识别中',
  ocrReady: '识别完成',
  chunking: '分块中',
  translating: '翻译中',
  rendering: '渲染中',
  writingArtifacts: '写入产物',
  paused: '已暂停',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

export function formatTaskPhase(phase: TranslationTask['phase'], progress: number): string {
  const label = PHASE_LABELS[phase] ?? phase;
  if (phase === 'translating' && progress > 0 && progress < 100) {
    return `${label} ${progress}%`;
  }
  return label;
}

export function formatShelfLabel(key: 'all' | 'essay' | 'research' | 'fiction') {
  switch (key) {
    case 'essay':
      return UI.essay;
    case 'research':
      return UI.research;
    case 'fiction':
      return UI.fiction;
    default:
      return UI.all;
  }
}

export function parseAppError(error: unknown): { message: string; retryAfterMs?: number } {
  if (error && typeof error === 'object') {
    const candidate = error as {
      message?: unknown;
      retryAfterMs?: unknown;
      retry_after_ms?: unknown;
    };
    const message =
      typeof candidate.message === 'string' && candidate.message.trim().length > 0
        ? candidate.message
        : null;
    const retryAfterMs =
      typeof candidate.retryAfterMs === 'number'
        ? candidate.retryAfterMs
        : typeof candidate.retry_after_ms === 'number'
          ? candidate.retry_after_ms
          : undefined;
    if (message) {
      return { message, retryAfterMs };
    }
  }

  if (typeof error === 'string') {
    return { message: error };
  }

  return { message: '未知错误' };
}

export function formatActionError(prefix: string, error: unknown) {
  const detail = parseAppError(error);
  if (typeof detail.retryAfterMs === 'number' && detail.retryAfterMs > 0) {
    const seconds = Math.max(1, Math.ceil(detail.retryAfterMs / 1000));
    return `${prefix}${detail.message}，约 ${seconds} 秒后重试`;
  }
  return `${prefix}${detail.message}`;
}

export function formatRetryAfter(retryAfterMs?: number) {
  if (typeof retryAfterMs !== 'number' || retryAfterMs <= 0) return null;
  return `${UI.retryAfter} ${Math.max(1, Math.ceil(retryAfterMs / 1000))} ${UI.seconds}`;
}

export function getStatusIcon(status: TranslationTask['status']) {
  switch (status) {
    case 'completed':
      return <CheckCircle size={18} className="text-[var(--ok)]" />;
    case 'failed':
      return <AlertCircle size={18} className="text-[var(--danger)]" />;
    case 'processing':
      return <Loader2 size={18} className="animate-spin text-accent" />;
    default:
      return <Clock size={18} className="text-muted" />;
  }
}
