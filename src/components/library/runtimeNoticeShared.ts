import type { RuntimeNotice } from '@/types';
import { UI } from './libraryViewShared';

export function formatNoticeScope(scope: RuntimeNotice['scope']) {
  switch (scope) {
    case 'task-runner':
      return UI.taskRunner;
    case 'commands':
      return UI.runtimeLayer;
    default:
      return scope;
  }
}

export function formatNoticeCodeLabel(code: string) {
  if (code === 'INTERNAL') return UI.runtimeInternal;
  return UI.runtimeAdvisory;
}

export function noticeToneClassName(code: string) {
  return code === 'INTERNAL'
    ? 'text-[var(--danger)]'
    : 'text-[var(--accent)]';
}
