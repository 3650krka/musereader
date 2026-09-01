import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { motion } from 'motion/react';
import { AlertTriangle, ArrowLeft, Check, CheckCheck, ChevronLeft, ChevronRight, Download, LayoutList, LoaderCircle, Rows3, Search, ShieldCheck, Sparkles, TableOfContents, Undo2 } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useT } from '@/i18n';
import type { ReviewSegment,  Book, TranslationTask } from '@/types';
import { displayBookTitle } from '@/components/library/bookDisplay';
import { BLOCKS_PER_PSEUDO_PAGE } from '@/lib/pagination';
import { TocEditorPanel } from './TocEditorPanel';
import './reviewExtra.css';


interface ReviewExportResult {
  path: string;
  segmentCount: number;
  editedCount: number;
}

type Filter = 'all' | 'edited' | 'todo';
/** 工作台布局：纵向滚动（原纸页列表）/ 横向分页（一次一页、左右翻页）。 */
type LayoutMode = 'scroll' | 'paged';

/** 规则检查问题（run_review_checks 返回）。 */
interface ReviewCheckIssue {
  blockId: string;
  rule: string;
  detail: string;
  source: string;
  effectiveTarget: string;
}

/** 侧栏功能 tab：进度/检查/搜索替换。 */
type SideTab = 'status' | 'check' | 'search';

/** 横向分页模式：每页段对数与后端伪分页（目录页码坐标系）一致——
    见 lib/pagination.ts 与 commands/toc_edit.rs::SOURCE_BLOCKS_PER_PAGE。
    同一坐标保证：分页视图内容 ≡ 页码预览 ≡ 目录页码归属。 */
const PAGE_SEGMENTS = BLOCKS_PER_PSEUDO_PAGE;
const REGEX_SPECIALS = /[.*+?^${}()|[\\]\\]/g;

const COVER_HUES = ['#B7C4B6', '#C9C2B4', '#A8B8C4', '#C4B4C0', '#B4C4C0', '#C0B8A8', '#B8BCC8', '#C8BCA8'];

function hueFor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 997;
  return COVER_HUES[h % COVER_HUES.length];
}

function chunk<T>(items: T[], size: number): T[][] {
  const pages: T[][] = [];
  for (let i = 0; i < items.length; i += size) pages.push(items.slice(i, i + size));
  return pages;
}

/** 可还原编辑段。 */
function RevertableEditable({
  text,
  onCommit,
}: {
  text: string;
  onCommit: (value: string) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const initialRef = useRef(text);

  // 段落原文变化时刷新初始值
  useEffect(() => {
    initialRef.current = text;
  }, [text]);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      if (ref.current) ref.current.textContent = initialRef.current;
      ref.current?.blur();
      event.preventDefault();
    }
  };

  return (
    <div
      ref={ref}
      className="tgt"
      contentEditable
      suppressContentEditableWarning
      spellCheck={false}
      onKeyDown={handleKeyDown}
      onBlur={(e) => onCommit(e.currentTarget.textContent ?? '')}
    >
      {text}
    </div>
  );
}

/** 段级 hover 操作：AI 重译 / 恢复机翻。
    恢复机翻 = 撤回修订（editedTarget:'' → effectiveTarget 回落机翻，override 标记清除）。 */
function SegmentActions(props: {
  seg: ReviewSegment;
  busy: boolean;
  onRetranslate: (seg: ReviewSegment) => void;
  onRevert: (seg: ReviewSegment) => void;
}) {
  const { seg, busy, onRetranslate, onRevert } = props;
  /* 图标按钮（避免文字换行），tooltip 承载语义 */
  return (
    <span className="seg-actions" aria-label="段操作">
      <button
        type="button"
        title="AI 重译本段"
        aria-label="AI 重译本段"
        disabled={busy}
        onClick={() => onRetranslate(seg)}
      >
        {busy ? <LoaderCircle size={13} className="spinning" /> : <Sparkles size={13} />}
      </button>
      {seg.overrideEdited && (
        <button type="button" title="撤回修订（恢复机翻译文）" aria-label="撤回修订" disabled={busy} onClick={() => onRevert(seg)}>
          <Undo2 size={13} />
        </button>
      )}
    </span>
  );
}

/** 校对：先选书（封面+进度环），再进入工作台。
    工作台双布局：纵向纸页列表 / 横向分页阅读（←/→ 键或按钮翻页）；
    顶栏含目录管理（重命名 / 合并删除 / 新建）。 */
export function ReviewView({ tasks, books, initialTaskId = null }: {
  tasks: TranslationTask[];
  books: Book[];
  /** 从阅读器直达时携带：挂载即载入该书工作台。 */
  initialTaskId?: string | null;
}) {
  const t = useT();
  const completedTasks = useMemo(() => tasks.filter((x) => x.status === 'completed'), [tasks]);
  const [taskId, setTaskId] = useState<string | null>(initialTaskId);
  const [segments, setSegments] = useState<ReviewSegment[]>([]);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState<Filter>('all');
  const [layout, setLayout] = useState<LayoutMode>('scroll');
  const [pageIndex, setPageIndex] = useState(0);
  const [tocOpen, setTocOpen] = useState(false);
  const [exportState, setExportState] = useState<'idle' | 'done'>('idle');
  /* 检查/搜索替换 */
  const [sideTab, setSideTab] = useState<SideTab>('status');
  const [issues, setIssues] = useState<ReviewCheckIssue[]>([]);
  const [checkRunning, setCheckRunning] = useState(false);
  const [flashBlockId, setFlashBlockId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchScope, setSearchScope] = useState<'all' | 'source' | 'target'>('all');
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  /** 正则模式：开启后查询按正则解释，禁用全词按钮。 */
  const [useRegex, setUseRegex] = useState(false);
  const [replaceValue, setReplaceValue] = useState('');
  const [replaceRunning, setReplaceRunning] = useState(false);

  const load = useCallback(async (id: string) => {
    setLoading(true);
    try {
      setSegments(await invoke<ReviewSegment[]>('list_review_segments', { taskId: id }));
    } catch {
      setSegments([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const open = (id: string) => {
    setTaskId(id);
    setPageIndex(0);
    void load(id);
  };

  /* 从阅读器直达（initialTaskId）：挂载即载入该书工作台段对。 */
  useEffect(() => {
    if (taskId) void load(taskId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const editedCount = segments.filter((s) => s.overrideEdited).length;
  const pct = segments.length ? Math.round((editedCount / segments.length) * 100) : 0;
  const matchesFilter = useCallback((seg: ReviewSegment) =>
    filter === 'all' ? true : filter === 'edited' ? Boolean(seg.overrideEdited) : !seg.overrideEdited,
  [filter]);
  const visible = segments.filter(matchesFilter);
  /* 分页坐标 = 目录页码坐标（全量段对，30 段对/页）：过滤不改变页边界，
     被过滤段在分页视图中降透明度而非移除，保证每页内容与 EPUB 伪页一致。 */
  const pages = useMemo(() => chunk(segments, PAGE_SEGMENTS), [segments]);
  const currentPage = Math.min(pageIndex, Math.max(0, pages.length - 1));

  /* 工作台级页码跳转（不止目录编辑可用）：页码 → 段对序号 (page-1)*30。
     分页模式=切页；滚动模式=左侧区域平滑滚动到该页首段并闪烁落点。 */
  const [pageGoto, setPageGoto] = useState(1);
  const jumpToPage = useCallback((page: number) => {
    if (pages.length === 0) return;
    const target = Math.min(Math.max(1, Math.round(page) || 1), pages.length);
    setPageGoto(target);
    if (layout === 'paged') {
      setPageIndex(target - 1);
      return;
    }
    const first = pages[target - 1]?.[0];
    if (!first) return;
    setFilter('all');
    setFlashBlockId(first.blockId);
    window.setTimeout(() => setFlashBlockId(null), 2200);
    window.setTimeout(() => {
      document.getElementById(`revseg-${first.blockId}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }, 60);
  }, [pages, layout]);

  /* 页码权威总数（后端 get_page_preview：PDF=版面页数；EPUB=段对伪分页）。
     与工作台分页坐标一致（== pages.length）时，目录页码调整可驱动左侧跟随跳转；
     不一致（PDF 真实页 ≠ 伪页）时目录面板回退为文本预览核对，不做误导跳转。 */
  const [pageTotal, setPageTotal] = useState(0);
  useEffect(() => {
    if (!taskId) { setPageTotal(0); return; }
    let cancelled = false;
    invoke<{ totalPages: number; preview: string }>('get_page_preview', { taskId, page: 1 })
      .then((result) => { if (!cancelled) setPageTotal(result.totalPages); })
      .catch(() => { if (!cancelled) setPageTotal(0); });
    return () => { cancelled = true; };
  }, [taskId]);
  const paginationUnified = pageTotal > 0 && pageTotal === pages.length;

  const turnPage = useCallback((delta: number) => {
    setPageIndex((index) => Math.max(0, Math.min(pages.length - 1, index + delta)));
  }, [pages.length]);

  /* 分页模式下页码指示与当前页同步（滚动模式由跳转驱动） */
  useEffect(() => {
    if (layout === 'paged') setPageGoto(currentPage + 1);
  }, [layout, currentPage]);

  /* 横向分页模式：←/→ 键翻页（输入框聚焦时让位） */
  useEffect(() => {
    if (layout !== 'paged') return;
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, [contenteditable]')) return;
      if (event.key === 'ArrowLeft') turnPage(-1);
      if (event.key === 'ArrowRight') turnPage(1);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [layout, turnPage]);

  const commitEdit = async (blockId: string, text: string) => {
    const current = segments.find((s) => s.blockId === blockId);
    if (!current || text.trim() === current.effectiveTarget.trim()) return;
    try {
      setSegments(
        await invoke<ReviewSegment[]>('save_review_edit', {
          taskId,
          blockId,
          editedTarget: text,
          comment: null,
        }),
      );
    } catch (error) {
      console.error('save review edit failed:', error);
    }
  };

  /* ── 单段操作：AI 重译 / 清空 / 恢复原译 ── */
  const [segBusy, setSegBusy] = useState<string | null>(null);

  const retranslateSegment = async (seg: ReviewSegment) => {
    if (!taskId || segBusy) return;
    setSegBusy(seg.blockId);
    try {
      const response = await invoke<{ answer: string }>('ai_assist_paragraph', {
        action: 'translate',
        sourceText: seg.source,
        question: null,
        targetLanguage: '中文',
        history: null,
      });
      const translated = response.answer.trim();
      if (!translated) return;
      // 并发防护：await 期间用户可能手工编辑过——用最新 segments 比对，
      // 已被人工改动（effectiveTarget 变化）则丢弃 AI 结果不覆盖。
      const latest = await invoke<ReviewSegment[]>('list_review_segments', { taskId });
      const current = latest.find((s) => s.blockId === seg.blockId);
      if (current && current.effectiveTarget.trim() !== seg.effectiveTarget.trim()) {
        setSegments(latest); // 同步刷新显示用户的新编辑
        return;
      }
      setSegments(
        await invoke<ReviewSegment[]>('save_review_edit', {
          taskId,
          blockId: seg.blockId,
          editedTarget: translated,
          comment: null,
        }),
      );
    } catch (error) {
      console.error('retranslate segment failed:', error);
    } finally {
      setSegBusy(null);
    }
  };

  /* 恢复机翻：撤回修订层（editedTarget:'' → override 清除，回落机翻）。 */
  const revertSegment = async (seg: ReviewSegment) => {
    if (!taskId || segBusy) return;
    setSegBusy(seg.blockId);
    try {
      setSegments(
        await invoke<ReviewSegment[]>('save_review_edit', {
          taskId,
          blockId: seg.blockId,
          editedTarget: '',
          comment: null,
        }),
      );
    } catch (error) {
      console.error('revert segment failed:', error);
    } finally {
      setSegBusy(null);
    }
  };

  const handleExport = async () => {
    if (!taskId) return;
    try {
      const path = await save({ defaultPath: 'reviewed.md', filters: [{ name: 'Markdown', extensions: ['md'] }] });
      if (!path) return;
      await invoke<ReviewExportResult>('export_reviewed_markdown', { taskId, outputPath: path });
      setExportState('done');
      window.setTimeout(() => setExportState('idle'), 2500);
    } catch (error) {
      console.error('export reviewed markdown failed:', error);
    }
  };

  /* ── 规则检查 ── */
  const runChecks = async () => {
    if (!taskId || checkRunning) return;
    setCheckRunning(true);
    setSideTab('check');
    try {
      const result = await invoke<ReviewCheckIssue[]>('run_review_checks', {
        taskId,
        articleType: task?.articleType ?? null,
        options: null,
      });
      setIssues(result);
    } catch (error) {
      console.error('run checks failed:', error);
      setIssues([]);
    } finally {
      setCheckRunning(false);
    }
  };

  /* 点击问题跳转对应段落并闪烁高亮 */
  const jumpToIssue = (issue: ReviewCheckIssue) => {
    setLayout('scroll');
    setFilter('all');
    setFlashBlockId(issue.blockId);
    window.setTimeout(() => setFlashBlockId(null), 2200);
    window.setTimeout(() => {
      document.getElementById(`revseg-${issue.blockId}`)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }, 60);
  };

  /* ── 搜索替换 ── */
  /* 统一编译搜索正则（纯函数）：正则模式直接编译（非法时返回错误）；字面量模式转义。 */
  const searchCompiled = useMemo((): { re: RegExp | null; error: string | null } => {
    if (!searchQuery.trim()) return { re: null, error: null };
    const flags = caseSensitive ? '' : 'i';
    const source = useRegex
      ? searchQuery
      : (wholeWord ? '\\b' + searchQuery.replace(REGEX_SPECIALS, '\\$&') + '\\b' : searchQuery.replace(REGEX_SPECIALS, '\\$&'));
    try {
      return { re: new RegExp(source, flags), error: null };
    } catch (error) {
      return { re: null, error: error instanceof Error ? error.message : String(error) };
    }
  }, [searchQuery, caseSensitive, wholeWord, useRegex]);
  const searchRe = searchCompiled.re;
  const regexError = searchCompiled.error;

  const searchResults = useMemo(() => {
    if (!searchRe) return [];
    return segments.filter((seg) => {
      if (searchScope === 'all' || searchScope === 'source') {
        if (searchRe.test(seg.source)) return true;
      }
      if (searchScope === 'all' || searchScope === 'target') {
        if (searchRe.test(seg.effectiveTarget)) return true;
      }
      return false;
    });
  }, [segments, searchRe, searchScope]);

  const replaceAll = async () => {
    if (!taskId || !searchQuery.trim() || replaceRunning || !searchRe) return;
    // source 范围无译文替换目标：直接返回（按钮已禁用，双保险）
    if (searchScope === 'source') return;
    setReplaceRunning(true);
    try {
      const flags = caseSensitive ? 'g' : 'gi';
      const re = new RegExp(searchRe.source, flags);
      // g 标志下 test 的 lastIndex 跨字符串残留：每段前重置，防漏判
      const targets = searchResults.filter((seg) => {
        re.lastIndex = 0;
        return re.test(seg.effectiveTarget);
      });
      for (const seg of targets) {
        re.lastIndex = 0;
        const newText = seg.effectiveTarget.replace(re, replaceValue);
        await invoke('save_review_edit', { taskId, blockId: seg.blockId, editedTarget: newText, comment: null });
      }
      // 刷新段对
      setSegments(await invoke<ReviewSegment[]>('list_review_segments', { taskId }));
    } catch (error) {
      console.error('replace all failed:', error);
    } finally {
      setReplaceRunning(false);
    }
  };

  /* ── 选书层 ── */
  if (!taskId) {
    return (
      <motion.div initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }} className="page-stage">
        <header className="page-head">
          <h1 className="page-title">{t.review.title}</h1>
        </header>
        <p className="page-sub" style={{ marginBottom: 18 }}>{t.review.pickBook}</p>
        <div className="revpick">
          {completedTasks.map((task) => {
            const book = books.find((b) => b.id === task.id);
            const fallbackTitle = task.filename.replace(/\.[^.]+$/, '').replace(/[_-]+/g, ' ');
            const title = book ? displayBookTitle(book, fallbackTitle) : fallbackTitle;
            return (
              <button key={task.id} type="button" className="revcard" onClick={() => open(task.id)}>
                {book?.cover ? (
                  <img src={book.cover} alt="" />
                ) : (
                  <div className="revcard-cover" style={{ background: hueFor(task.id) }} aria-hidden="true">
                    {task.filename.replace(/\.[^.]+$/, '').slice(0, 2).toUpperCase()}
                  </div>
                )}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <b style={{ fontFamily: 'var(--font-display)', fontSize: 15.5 }}>{title}</b>
                  <p style={{ color: 'var(--ink-3)', fontSize: 12, marginTop: 3 }}>{book?.author || task.articleType}</p>
                  <p style={{ color: 'var(--ink-3)', fontSize: 12, marginTop: 10 }}>
                    {task.totalChunks} {t.review.segs}
                  </p>
                </div>
              </button>
            );
          })}
          {completedTasks.length === 0 && <p className="empty-hint">{t.terms.empty}</p>}
        </div>
      </motion.div>
    );
  }

  /* ── 工作台层 ── */
  const task = completedTasks.find((x) => x.id === taskId);
  const pageSegments = pages[currentPage] ?? [];
  return (
    <motion.div initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }} className="page-stage">
      <div className="lib-topline">
        <button type="button" className="iconbtn" onClick={() => setTaskId(null)} aria-label={t.reader.back}>
          <ArrowLeft size={20} />
        </button>
        <div style={{ flex: 1, minWidth: 0 }}>
          <b style={{ fontFamily: 'var(--font-display)', fontSize: 16 }}>{task?.filename ?? ''}</b>
          {visible[0]?.chapter && (
            <span style={{ color: 'var(--ink-3)', fontSize: 12.5, marginLeft: 10 }}>{visible[0].chapter}</span>
          )}
        </div>
        {/* 布局切换（滑动式分段控件）：纵向滚动 / 横向分页 */}
        <div className="revlayout-ctl" data-active={layout === 'scroll' ? '0' : '1'} role="tablist" aria-label="工作台布局">
          <span className="revlayout-ctl-thumb" aria-hidden="true" />
          <button
            type="button"
            role="tab"
            aria-selected={layout === 'scroll'}
            className={layout === 'scroll' ? 'on' : ''}
            title="纵向滚动"
            onClick={() => setLayout('scroll')}
          >
            <Rows3 size={14} />
            {t.review.layoutScroll}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={layout === 'paged'}
            className={layout === 'paged' ? 'on' : ''}
            title="横向分页（←/→ 翻页）"
            onClick={() => setLayout('paged')}
          >
            <LayoutList size={14} />
            {t.review.layoutPaged}
          </button>
        </div>
        {/* 工作台级页码跳转（不止目录编辑可用）：与目录页码/分页视图同一坐标（30 段对/页），
            分页模式切页、滚动模式左侧内容跟随跳转并闪烁落点。 */}
        {!loading && pages.length > 0 && (
          <div className="revpagejump" role="group" aria-label="页码跳转">
            <button
              type="button"
              className="iconbtn"
              style={{ width: 26, height: 26 }}
              onClick={() => jumpToPage(pageGoto - 1)}
              disabled={pageGoto <= 1}
              aria-label="上一页"
              title="上一页"
            >
              <ChevronLeft size={14} />
            </button>
            <input
              type="number"
              min={1}
              max={pages.length}
              value={pageGoto}
              onChange={(e) => {
                const v = Number(e.target.value) || 1;
                setPageGoto(Math.min(Math.max(1, v), pages.length));
              }}
              onKeyDown={(e) => { if (e.key === 'Enter') jumpToPage(pageGoto); }}
              onBlur={() => jumpToPage(pageGoto)}
              aria-label="页码"
            />
            <button
              type="button"
              className="iconbtn"
              style={{ width: 26, height: 26 }}
              onClick={() => jumpToPage(pageGoto + 1)}
              disabled={pageGoto >= pages.length}
              aria-label="下一页"
              title="下一页"
            >
              <ChevronRight size={14} />
            </button>
            <em>第 {pageGoto} / {pages.length} 页</em>
          </div>
        )}
        <button
          type="button"
          className={`iconbtn ${tocOpen ? 'on' : ''}`}
          onClick={() => setTocOpen((v) => !v)}
          title="目录管理：重命名 / 合并删除 / 新建"
          aria-label="目录管理"
          aria-pressed={tocOpen}
        >
          <TableOfContents size={16} />
        </button>
        <button
          type="button"
          className={`iconbtn ${exportState === 'done' ? 'on' : ''}`}
          onClick={() => void handleExport()}
          disabled={!segments.length}
          title={exportState === 'done' ? t.toast.saved : t.review.export}
          aria-label={t.review.export}
        >
          {exportState === 'done' ? <Check size={16} /> : <Download size={16} />}
        </button>
      </div>

      <div className="revlayout">
        <div className="pagestack">
          {layout === 'paged' ? (
            /* ── 横向分页模式 ── */
            <div className="paper revpaged">
              {loading && <p className="text-muted" style={{ fontSize: 13 }}>…</p>}
              {!loading && pageSegments.length === 0 && (
                <p className="text-muted" style={{ fontSize: 13 }}>{t.terms.empty}</p>
              )}
              {!loading && pageSegments.map((seg) => (
                <div
                  key={seg.blockId}
                  id={`revseg-${seg.blockId}`}
                  className={`revseg ${seg.overrideEdited ? 'edited' : ''} ${flashBlockId === seg.blockId ? 'flash' : ''} ${matchesFilter(seg) ? '' : 'revseg-dim'}`}
                >
                  <span className="flag" />
                  <div className="src">{seg.source}</div>
                  <RevertableEditable
                    text={seg.effectiveTarget}
                    onCommit={(value) => void commitEdit(seg.blockId, value)}
                  />
                  <SegmentActions
                    seg={seg}
                    busy={segBusy === seg.blockId}
                    onRetranslate={(s) => void retranslateSegment(s)}
                    onRevert={(s) => void revertSegment(s)}
                  />
                </div>
              ))}
              {!loading && pages.length > 0 && (
                <div className="revpaged-nav">
                  <button
                    type="button"
                    className="iconbtn"
                    style={{ width: 32, height: 32 }}
                    onClick={() => turnPage(-1)}
                    disabled={currentPage === 0}
                    aria-label="上一页"
                  >
                    <ChevronLeft size={16} />
                  </button>
                  <span className="revpaged-indicator">
                    第 {currentPage + 1} / {pages.length} 页 · 共 {segments.length} {t.review.segs}
                  </span>
                  <button
                    type="button"
                    className="iconbtn"
                    style={{ width: 32, height: 32 }}
                    onClick={() => turnPage(1)}
                    disabled={currentPage >= pages.length - 1}
                    aria-label="下一页"
                  >
                    <ChevronRight size={16} />
                  </button>
                </div>
              )}
            </div>
          ) : (
            /* ── 纵向滚动模式（原纸页列表） ── */
            <div className="paper">
              {loading && <p className="text-muted" style={{ fontSize: 13 }}>…</p>}
              {!loading && visible.map((seg, idx) => (
                <div key={seg.blockId} id={`revseg-${seg.blockId}`} className={`revseg ${seg.overrideEdited ? 'edited' : ''} ${flashBlockId === seg.blockId ? 'flash' : ''}`}>
                  <span className="flag" />
                  <span className="revnum">{String(idx + 1).padStart(2, '0')}</span>
                  <div className="src">{seg.source}</div>
                  <RevertableEditable
                    text={seg.effectiveTarget}
                    onCommit={(value) => void commitEdit(seg.blockId, value)}
                  />
                  <SegmentActions
                    seg={seg}
                    busy={segBusy === seg.blockId}
                    onRetranslate={(s) => void retranslateSegment(s)}
                    onRevert={(s) => void revertSegment(s)}
                  />
                </div>
              ))}
            </div>
          )}
        </div>

        <aside className="revside">
          {tocOpen ? (
            <TocEditorPanel
              taskId={taskId}
              pageTotal={pageTotal}
              onPageJump={paginationUnified ? jumpToPage : undefined}
              onClose={() => setTocOpen(false)}
            />
          ) : (
            <>
              <div className="revside-tabs">
                <button type="button" className={`chip ${sideTab === 'status' ? 'on' : ''}`} onClick={() => setSideTab('status')}>{t.review.progress}</button>
                <button type="button" className={`chip ${sideTab === 'check' ? 'on' : ''}`} onClick={() => setSideTab('check')}>
                  <ShieldCheck size={13} />
                  {t.review.check}
                </button>
                <button type="button" className={`chip ${sideTab === 'search' ? 'on' : ''}`} onClick={() => setSideTab('search')}>
                  <Search size={13} />
                  {t.review.search}
                </button>
              </div>

              {sideTab === 'check' && (
                <div className="revside-section">
                  <div className="card-title" style={{ fontSize: 14 }}>
                    <AlertTriangle size={16} />
                    <span>{t.review.check}</span>
                  </div>
                  <p style={{ fontSize: 12, color: 'var(--ink-3)', lineHeight: 1.7, margin: '6px 0 10px' }}>{t.review.checkDesc}</p>
                  <button type="button" className="btn sm" onClick={() => void runChecks()} disabled={checkRunning}>
                    {checkRunning ? '…' : t.review.runChecks}
                  </button>
                  <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 6, maxHeight: 320, overflowY: 'auto' }}>
                    {issues.length === 0 && !checkRunning && <p className="text-muted" style={{ fontSize: 12 }}>{t.review.noIssues}</p>}
                    {issues.map((issue) => {
                      const ruleLabel: Record<string, string> = {
                        untranslated: t.review.rule_untranslated,
                        residual: t.review.rule_residual,
                        digits: t.review.rule_digits,
                        newlines: t.review.rule_newlines,
                        terminology: '术语不一致',
                      };
                      return (
                        <button key={`${issue.blockId}-${issue.rule}`} type="button" className="revissue" onClick={() => jumpToIssue(issue)}>
                          <span className={`revissue-rule rule-${issue.rule}`}>{ruleLabel[issue.rule] ?? issue.rule}</span>
                          <span className="revissue-detail truncate">{issue.detail}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}

              {sideTab === 'search' && (
                <div className="revside-section">
                  <div className="card-title" style={{ fontSize: 14 }}>
                    <Search size={16} />
                    <span>{t.review.search}</span>
                  </div>
                  <input
                    value={searchQuery}
                    placeholder={t.review.searchPlaceholder}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    style={{ width: '100%', marginTop: 8 }}
                  />
                  <div style={{ display: 'flex', gap: 6, marginTop: 8, flexWrap: 'wrap' }}>
                    {(['all', 'source', 'target'] as const).map((scope) => {
                      const scopeLabel: Record<'all' | 'source' | 'target', string> = {
                        all: t.review.scope_all,
                        source: t.review.scope_source,
                        target: t.review.scope_target,
                      };
                      return (
                        <button key={scope} type="button" className={`chip sm ${searchScope === scope ? 'on' : ''}`} onClick={() => setSearchScope(scope)}>
                          {scopeLabel[scope]}
                        </button>
                      );
                    })}
                    <button type="button" className={`chip sm ${caseSensitive ? 'on' : ''}`} onClick={() => setCaseSensitive(!caseSensitive)}>{t.review.caseSensitive}</button>
                    <button type="button" className={`chip sm ${wholeWord && !useRegex ? 'on' : ''}`} disabled={useRegex} onClick={() => setWholeWord(!wholeWord)}>{t.review.wholeWord}</button>
                    <button type="button" className={`chip sm ${useRegex ? 'on' : ''}`} title=".*+?^$ 等元字符按正则解释；非法正则会在下方提示" onClick={() => setUseRegex(!useRegex)}>.*</button>
                  </div>
                  {regexError && (
                    <p role="alert" style={{ fontSize: 12, color: 'var(--danger, #c0392b)', marginTop: 6 }}>
                      {regexError.length > 90 ? regexError.slice(0, 90) + '…' : regexError}
                    </p>
                  )}
                  <p style={{ fontSize: 12, color: 'var(--ink-3)', marginTop: 8 }}>{t.review.found} {searchResults.length} {t.review.matches}</p>
                  <input
                    value={replaceValue}
                    placeholder={t.review.replaceWith}
                    onChange={(e) => setReplaceValue(e.target.value)}
                    style={{ width: '100%', marginTop: 8 }}
                  />
                  {/* 全部替换：整行横放（挤在输入框右侧窄列换行难看） */}
                  <button
                    type="button"
                    className="btn ghost sm"
                    style={{ width: '100%', marginTop: 8, justifyContent: 'center', display: 'flex' }}
                    onClick={() => void replaceAll()}
                    disabled={replaceRunning || searchResults.length === 0 || searchScope === 'source'}
                    title={searchScope === 'source' ? '原文范围不支持替换' : undefined}
                  >
                    {replaceRunning ? '…' : t.review.replaceAll}
                  </button>
                </div>
              )}

              {sideTab === 'status' && (
                <>
              <div className="revside-section">
                <div className="card-title" style={{ fontSize: 14 }}>
                  <CheckCheck size={18} />
                  <span>{t.review.progress}</span>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 12, margin: '8px 0 14px' }}>
                  <div className="revring" style={{ '--p': pct } as React.CSSProperties}><span>{pct}%</span></div>
                  <div style={{ fontSize: 12.5, color: 'var(--ink-3)' }}>
                    <b style={{ color: 'var(--ink)' }}>{editedCount}</b> / {segments.length} {t.review.segs}
                  </div>
                </div>
                <div className="revfilter">
                  {(['all', 'edited', 'todo'] as Filter[]).map((f) => (
                    <button key={f} type="button" className={`chip ${filter === f ? 'on' : ''}`} onClick={() => { setFilter(f); setPageIndex(0); }}>
                      {f === 'all' ? t.review.all : f === 'edited' ? t.review.edited : t.review.todo}
                    </button>
                  ))}
                </div>
              </div>
              <div className="revside-section">
                <div className="card-title" style={{ fontSize: 14 }}><span>{t.review.how}</span></div>
                <p style={{ fontSize: 12.5, color: 'var(--ink-3)', lineHeight: 1.8 }}>{t.review.howDesc}</p>
              </div>
                </>
              )}
            </>
          )}
        </aside>
      </div>
    </motion.div>
  );
}
