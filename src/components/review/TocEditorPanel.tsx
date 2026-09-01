import { invoke } from '@tauri-apps/api/core';
import { ChevronLeft, ChevronRight, FolderPlus, Hash, Merge, Pencil, Plus, Trash2, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import type { TocEditItem } from '@/types';
import { Slider } from '@/components/common/Slider';
import './reviewExtra.css';

interface TocEditorPanelProps {
  taskId: string;
  /** 总页数（工作台统一数据源：ReviewView 经 get_page_preview 获取，0=无分页工件）。 */
  pageTotal: number;
  /** 页码变化时左侧正文区跟随跳转（工作台级页码导航，目录面板只负责选页）。 */
  onPageJump?: (page: number) => void;
  onClose: () => void;
}

/** 目录管理面板（校对工作台内）：
    - 重命名：行内编辑标题；
    - 合并删除：移除多余目录条目（内容并入相邻章节，不删正文）；
    - 新建：选插入锚点 + 页码选择（调整页码时左侧正文跟随跳转，直接目视核对）；
    - 已有条目：页码修订（同一套页码选择器 + 左侧跟随）。
    页码数据源：PDF=source_layouts.json 每页一版面；
    EPUB=与校对段对同坐标的伪分页（30 段对/页，见 lib/pagination.ts）。 */

/** 页码选择器：两行布局（适配 250px 侧栏，绝不溢出）——
    行一：‹ 数字输入 › ＋ 页码指示；行二：全宽渐变滑条。
    onJump：页码变化的同步副作用（如左侧正文区跟随跳转），属于工作台级能力，
    面板仅负责选定页码。 */
function PagePicker({ page, totalPages, onChange, label, onJump }: {
  page: number;
  totalPages: number;
  onChange: (page: number) => void;
  label: string;
  onJump?: (page: number) => void;
}) {
  const clamped = () => Math.min(Math.max(1, page), totalPages);
  const set = (next: number) => {
    const target = Math.min(Math.max(1, Number.isFinite(next) ? next : 1), totalPages);
    onChange(target);
    onJump?.(target);
  };
  return (
    <div className="toc-editor-pagerow" role="group" aria-label={label}>
      <div className="toc-editor-pageinputs">
        <button
          type="button"
          className="iconbtn"
          style={{ width: 24, height: 24, flex: 'none' }}
          onClick={() => set(clamped() - 1)}
          disabled={page <= 1}
          aria-label="上一页"
          title="上一页（左侧内容跟随跳转核对）"
        >
          <ChevronLeft size={13} />
        </button>
        <input
          type="number"
          min={1}
          max={totalPages}
          value={clamped()}
          onChange={(e) => set(Number(e.target.value) || 1)}
          style={{ width: 56, flex: 'none' }}
          aria-label="页码"
        />
        <button
          type="button"
          className="iconbtn"
          style={{ width: 24, height: 24, flex: 'none' }}
          onClick={() => set(clamped() + 1)}
          disabled={page >= totalPages}
          aria-label="下一页"
          title="下一页（左侧内容跟随跳转核对）"
        >
          <ChevronRight size={13} />
        </button>
        <em>第 {clamped()} / {totalPages} 页</em>
      </div>
      <Slider
        min={1}
        max={totalPages}
        value={clamped()}
        onChange={(v) => set(v)}
        ariaLabel="页码滑条"
      />
    </div>
  );
}

export function TocEditorPanel({ taskId, pageTotal, onPageJump, onClose }: TocEditorPanelProps) {
  const [items, setItems] = useState<TocEditItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState('');
  const [adding, setAdding] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [newAfterId, setNewAfterId] = useState<string>('');
  const [newPage, setNewPage] = useState(1);
  /* 已有条目页码修订：点击 Hash 进入编辑，页码选择 + 左侧跟随目视核对 */
  const [pageEditId, setPageEditId] = useState<string | null>(null);
  const [pageDraft, setPageDraft] = useState(1);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setItems(await invoke<TocEditItem[]>('list_toc_edit_items', { taskId }));
    } catch (error) {
      console.error('load toc edit items failed:', error);
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, [taskId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const startRename = (item: TocEditItem) => {
    setEditingId(item.id);
    setDraftTitle(item.title);
  };

  const commitRename = async () => {
    if (!editingId || busy) return;
    const title = draftTitle.trim();
    setBusy(true);
    try {
      if (title) setItems(await invoke<TocEditItem[]>('rename_toc_entry', { taskId, entryId: editingId, title }));
    } catch (error) {
      console.error('rename toc entry failed:', error);
    } finally {
      setBusy(false);
      setEditingId(null);
    }
  };

  const handleRemove = async (id: string) => {
    if (busy) return;
    setBusy(true);
    try {
      setItems(await invoke<TocEditItem[]>('remove_toc_entry', { taskId, entryId: id }));
    } catch (error) {
      console.error('remove toc entry failed:', error);
    } finally {
      setBusy(false);
    }
  };

  const startPageEdit = (item: TocEditItem) => {
    setPageEditId(item.id);
    setPageDraft(item.page ?? 1);
  };

  const commitPageEdit = async () => {
    if (!pageEditId || busy || pageTotal <= 0) return;
    setBusy(true);
    try {
      setItems(await invoke<TocEditItem[]>('set_toc_entry_page', {
        taskId,
        entryId: pageEditId,
        page: Math.min(Math.max(1, pageDraft), pageTotal),
      }));
      setPageEditId(null);
    } catch (error) {
      console.error('set toc page failed:', error);
    } finally {
      setBusy(false);
    }
  };

  const handleAdd = async () => {
    const title = newTitle.trim();
    if (!title || busy) return;
    setBusy(true);
    try {
      setItems(
        await invoke<TocEditItem[]>('add_toc_entry', {
          taskId,
          title,
          afterId: newAfterId || null,
          page: pageTotal > 0 ? newPage : null,
        }),
      );
      setNewTitle('');
      setAdding(false);
    } catch (error) {
      console.error('add toc entry failed:', error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="toc-editor">
      <div className="toc-editor-head">
        <span className="toc-editor-title">
          <FolderPlus size={15} />
          目录管理
        </span>
        <span className="toc-editor-hint">重命名 / 合并删除多余条目 / 新建条目</span>
        <button type="button" className="iconbtn" style={{ width: 28, height: 28 }} onClick={onClose} aria-label="关闭目录管理">
          <X size={14} />
        </button>
      </div>

      <div className="toc-editor-body">
        {loading && <p className="text-muted" style={{ fontSize: 12.5 }}>加载目录…</p>}
        {!loading && items.length === 0 && (
          <p className="text-muted" style={{ fontSize: 12.5 }}>该书没有目录工件（翻译产物缺失或未生成）。</p>
        )}
        {items.map((item) => (
          <div key={item.id} className="toc-editor-row">
            {editingId === item.id ? (
              <input
                className="toc-editor-input"
                value={draftTitle}
                autoFocus
                onChange={(e) => setDraftTitle(e.target.value)}
                onBlur={() => void commitRename()}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void commitRename();
                  if (e.key === 'Escape') setEditingId(null);
                }}
              />
            ) : (
              <span className="toc-editor-item-title" style={{ paddingLeft: (item.level - 1) * 14 }}>
                {item.title}
                {item.kind === 'added' && <em className="toc-editor-added-tag">新建</em>}
                {item.page != null && <em className="toc-editor-page">p.{item.page}</em>}
              </span>
            )}
            <span className="toc-editor-row-actions">
              {pageTotal > 0 && (
                <button
                  type="button"
                  className="iconbtn"
                  style={{ width: 26, height: 26 }}
                  title="修订页码"
                  onClick={() => startPageEdit(item)}
                >
                  <Hash size={13} />
                </button>
              )}
              <button
                type="button"
                className="iconbtn"
                style={{ width: 26, height: 26 }}
                title="重命名"
                onClick={() => startRename(item)}
              >
                <Pencil size={13} />
              </button>
              <button
                type="button"
                className="iconbtn"
                style={{ width: 26, height: 26 }}
                title="删除条目（内容并入相邻章节）"
                onClick={() => void handleRemove(item.id)}
              >
                <Merge size={13} />
              </button>
            </span>
            {pageEditId === item.id && pageTotal > 0 && (
              <div className="toc-editor-pageform">
                <PagePicker
                  page={pageDraft}
                  totalPages={pageTotal}
                  onChange={setPageDraft}
                  label="修订页码"
                  onJump={onPageJump}
                />
                <div className="toc-editor-addactions">
                  <button type="button" className="btn ghost sm" onClick={() => setPageEditId(null)}>取消</button>
                  <button type="button" className="btn sm" onClick={() => void commitPageEdit()} disabled={busy}>保存页码</button>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      <div className="toc-editor-foot">
        {adding ? (
          <div className="toc-editor-addform">
            <input
              className="toc-editor-input"
              placeholder="新目录标题…"
              value={newTitle}
              autoFocus
              onChange={(e) => setNewTitle(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && void handleAdd()}
            />
            <select
              className="settings-select"
              value={newAfterId}
              onChange={(e) => setNewAfterId(e.target.value)}
              aria-label="插入位置"
            >
              <option value="">插到最前</option>
              {items
                .filter((item) => item.kind !== 'added')
                .map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.title} 之后
                  </option>
                ))}
            </select>
            {pageTotal > 0 && (
              <PagePicker
                page={newPage}
                totalPages={pageTotal}
                onChange={setNewPage}
                label="新条目页码"
                onJump={onPageJump}
              />
            )}
            <div className="toc-editor-addactions">
              <button type="button" className="btn ghost sm" onClick={() => setAdding(false)} disabled={busy}>
                <Trash2 size={13} />
                取消
              </button>
              <button type="button" className="btn sm" onClick={() => void handleAdd()} disabled={busy || !newTitle.trim()}>
                <Plus size={13} />
                添加
              </button>
            </div>
          </div>
        ) : (
          <button type="button" className="btn ghost sm" onClick={() => setAdding(true)}>
            <Plus size={13} />
            新建目录条目
          </button>
        )}
      </div>
    </div>
  );
}
