import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, CheckCheck, Loader2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useT } from '@/i18n';
import type { ReviewSegment } from '@/types';

interface ReaderReviewPanelProps {
  bookId: string;
  chapterTitle: string;
}

interface ReviewCheckIssue {
  blockId: string;
  rule: string;
  detail: string;
  source: string;
  effectiveTarget: string;
}

/** 阅读器校对模式的侧栏：问题导航台（正文内联编辑的辅助面板）。
    职责：① 当前章问题扫描（未译/残留，点击定位到正文段落）；
         ② 校对进度统计；③ 跳转完整校对工作台入口。
    段落编辑本体在正文（点段落直接改，useInlineReview），此处不重复编辑 UI。 */
export function ReaderReviewPanel({ bookId, chapterTitle }: ReaderReviewPanelProps) {
  const t = useT();
  const [issues, setIssues] = useState<ReviewCheckIssue[] | null>(null);
  const [running, setRunning] = useState(false);
  const [segments, setSegments] = useState<ReviewSegment[]>([]);
  const [flashBlockId, setFlashBlockId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<ReviewSegment[]>('list_review_segments', { taskId: bookId })
      .then((rows) => { if (!cancelled) setSegments(rows); })
      .catch(() => { if (!cancelled) setSegments([]); });
    return () => { cancelled = true; };
  }, [bookId]);

  /* 当前章段对（章节名匹配） */
  const scoped = chapterTitle
    ? segments.filter((seg) => (seg.chapter ?? '').trim() === chapterTitle.trim())
    : segments;

  const runChecks = useCallback(async () => {
    if (running) return;
    setRunning(true);
    try {
      const rows = await invoke<ReviewCheckIssue[]>('run_review_checks', {
        taskId: bookId,
        articleType: null,
        options: { untranslated: true, residual: true, digits: false, newlines: false, terminology: false },
      });
      /* 仅当前章的问题（按 source 前缀匹配 scoped 段） */
      const scopedSources = new Set(scoped.map((s) => s.source.slice(0, 40)));
      setIssues(rows.filter((row) => scopedSources.has(row.source.slice(0, 40))));
    } catch (error) {
      console.error('review checks failed:', error);
      setIssues([]);
    } finally {
      setRunning(false);
    }
  }, [bookId, running, scoped]);

  /* 点击问题 → 通知正文定位闪烁（正文段落有 data-block-id 时直接滚+闪） */
  const locateIssue = (issue: ReviewCheckIssue) => {
    setFlashBlockId(issue.blockId);
    const frame = document.querySelector('iframe.epub-frame') as HTMLIFrameElement | null;
    const doc = frame?.contentDocument ?? null;
    if (!doc) return;
    const target =
      doc.querySelector(`[data-block-id="${issue.blockId}"]`) ??
      Array.from(doc.querySelectorAll<Element>('p,li'))
        .find((el) => (el.textContent ?? '').trim().startsWith(issue.source.slice(0, 24))) ?? null;
    if (target) {
      target.scrollIntoView({ behavior: 'smooth', block: 'center' });
      target.classList.add('mt-flash');
      window.setTimeout(() => target.classList.remove('mt-flash'), 2200);
    }
  };

  const editedCount = scoped.filter((s) => s.overrideEdited).length;
  const pct = scoped.length > 0 ? Math.round((editedCount / scoped.length) * 100) : 0;

  return (
    <div className="rpane on rreview">
      <p className="rreview-hint">{t.reader.reviewInlineHint}</p>

      {/* 进度 */}
      <div className="card" style={{ padding: 12, marginBottom: 10, display: 'flex', alignItems: 'center', gap: 10 }}>
        <CheckCheck size={16} style={{ color: 'var(--ok, #3F7D54)' }} />
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600 }}>{editedCount} / {scoped.length} · {pct}%</div>
          <div className="bar" style={{ marginTop: 4 }}><i style={{ width: `${pct}%` }} /></div>
        </div>
      </div>

      {/* 问题扫描 */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
        <button type="button" className="chip sm" onClick={() => void runChecks()} disabled={running}>
          {running ? <Loader2 size={12} className="wwc-spin" /> : <AlertTriangle size={12} />}
          {'扫描本章问题'}
        </button>
        {issues && <span style={{ fontSize: 12, color: 'var(--ink-3)' }}>{issues.length}</span>}
      </div>
      {issues && issues.length === 0 && (
        <p style={{ fontSize: 12.5, color: 'var(--ok, #3F7D54)' }}>{'本章未发现问题'}</p>
      )}
      {issues?.map((issue) => (
        <button
          key={`${issue.blockId}-${issue.rule}`}
          type="button"
          className={`revissue ${flashBlockId === issue.blockId ? 'flash' : ''}`}
          style={{ marginBottom: 6, width: '100%' }}
          onClick={() => locateIssue(issue)}
        >
          <span className={`revissue-rule rule-${issue.rule}`}>
            {issue.rule === 'untranslated' ? '未译' : '残留'}
          </span>
          <span className="revissue-detail">{issue.detail}</span>
        </button>
      ))}
    </div>
  );
}
