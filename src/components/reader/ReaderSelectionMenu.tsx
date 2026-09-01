import { motion } from 'motion/react';
import { BookOpenText, Bookmark, Eraser, Highlighter, Languages, Palette, PenTool, Sparkles, Underline, Waves, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useT } from '@/i18n';
import type { ReaderSelection } from './readerShared';

const MARK_PALETTE_COLORS = ['#f0c040', '#9ae6b4', '#8fd0ff', '#f2a0b5', '#c9a8f0', '#a8b8c8'];

/* 划线颜色记忆：各样式上次使用的颜色，跨会话持久；
   初始=色板最左色。 */
const MARK_COLORS_KEY = 'mt.reader.markColors';
const DEFAULT_MARK_COLORS: Record<string, string> = {
  highlight: MARK_PALETTE_COLORS[0],
  underline: MARK_PALETTE_COLORS[0],
  wave: MARK_PALETTE_COLORS[0],
};
function loadMarkColors(): Record<string, string> {
  try {
    const parsed = JSON.parse(localStorage.getItem(MARK_COLORS_KEY) ?? '{}') as Record<string, string>;
    return { ...DEFAULT_MARK_COLORS, ...parsed };
  } catch {
    return { ...DEFAULT_MARK_COLORS };
  }
}
function persistMarkColors(colors: Record<string, string>): void {
  try {
    localStorage.setItem(MARK_COLORS_KEY, JSON.stringify(colors));
  } catch { /* 存储不可用不影响功能 */ }
}

/** 划线/高亮样式：id 存入书签 style 字段，供笔记列表与正文标记复用。 */
export const SELECTION_MARK_STYLES: { id: string; icon: typeof Highlighter; swatchClass: string }[] = [
  { id: 'highlight', icon: Highlighter, swatchClass: 'mark-sw mark-highlight' },
  { id: 'underline', icon: Underline, swatchClass: 'mark-sw mark-underline' },
  { id: 'wave', icon: Waves, swatchClass: 'mark-sw mark-wave' },
  { id: 'none', icon: Eraser, swatchClass: 'mark-sw mark-eraser' },
];

interface SelectionMenuProps {
  selection: ReaderSelection | null;
  onSaveBookmark: (quote: string, style: string) => void;
  onSaveNote: (quote: string, note: string) => void;
  onAskAi: (text: string) => void;
  onClose: () => void;
  /** 划线点击唤出：直接进入批注输入态。 */
  initialNoteOpen?: boolean;
  /** 划选翻译：后端 ai_assist_paragraph action=translate。 */
  onTranslate?: (text: string) => Promise<string>;
  /** 翻译结果存生词库（词=划选文本，context=所在例句，customDefinition=译文）。 */
  onAddToVocab?: (text: string, translation: string, context: string) => void;
  /** 存到自建生词本（弹出生词本选择）。 */
  onAddToNotebook?: (text: string, definition: string, context: string) => void;
  /** 划词词典：source=上下文句，question=目标词；无词典结果时 AI 提供。 */
  onDefine?: (context: string, word: string) => Promise<string>;
  /** 划选自动翻译（anx autoTranslateSelection 对齐）：新划选到达即自动查翻译。 */
  autoTranslate?: boolean;
  /** 划选自动划线（anx autoMarkSelection 对齐）：新划选到达即自动应用上次的样式。 */
  autoMark?: boolean;
  /** 清除划线（橡皮擦）：删除该引文的既有书签并摘除正文可视标记。 */
  onClearMark?: (quote: string) => void;
  /** 当前页可见文本（供提取划词所在句）。 */
  pageText?: string;
}

/** 划选浮窗（短交互）：点外部/Esc 收起；批注输入框内嵌于浮窗，
    输入期间点侧栏/任意区域不再消失（composing 保护），保存后自动收起。
    位置做视口钳制：永不遮住划选文字（贴下沿显示，越界则翻转/收缩）。 */
export function ReaderSelectionMenu({ selection, onSaveBookmark, onSaveNote, onAskAi, onClose, initialNoteOpen = false, onTranslate, onAddToVocab, onAddToNotebook, onDefine, pageText = '', autoTranslate = false, autoMark = false, onClearMark }: SelectionMenuProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);
  const [noteOpen, setNoteOpen] = useState(false);
  const [noteDraft, setNoteDraft] = useState('');
  const [adjustedTop, setAdjustedTop] = useState<number | null>(null);
  /* 色板：paletteFor=目标样式；选色即应用并记住该样式最近色。 */
  const [paletteFor, setPaletteFor] = useState<string | null>(null);
  const swatchRef = useRef<HTMLDivElement | null>(null);
  const [lastColors, setLastColors] = useState<Record<string, string>>(loadMarkColors);
  /* 应用划线样式：样式按钮直接用该样式记忆色完成，
     不依赖色板打开（此前 onApplyColor 要求 paletteFor 非空——样式按钮点击被吞，
     用户只能经色板划线）。色板选色 = 换色后立即应用。 */
  const applyMark = (styleId: string, color: string) => {
    if (!selection) return;
    const next = { ...lastColors, [styleId]: color };
    setLastColors(next);
    persistMarkColors(next);
    onSaveBookmark(selection.text, `${styleId}:${color}`);
    setPaletteFor(null);
  };
  const onApplyColor = (color: string) => {
    if (!paletteFor || !selection) return;
    applyMark(paletteFor, color);
  };
  /* 划选翻译 */
  const [translated, setTranslated] = useState<string | null>(null);
  const [translating, setTranslating] = useState(false);
  /* 划词词典：单词划选时可查语境义 */
  const [defined, setDefined] = useState<string | null>(null);
  const [defining, setDefining] = useState(false);
  /* 竞态防护：划选切换使在途翻译/词典响应失效 */
  const requestIdRef = useRef(0);
  /* 词典查询对象：单个词或 1–4 词短语 */
  const isSingleWord = /^[A-Za-z][A-Za-z'-]*(?:\s+[A-Za-z][A-Za-z'-]*){0,3}$/.test(selection?.text.trim() ?? '');

  /* 新划选进来时重置批注态（划线点击唤出时直接进入输入） */
  useEffect(() => {
    requestIdRef.current += 1; // 使所有在途请求失效
    setNoteOpen(Boolean(initialNoteOpen));
    setNoteDraft('');
    setAdjustedTop(null);
    setPaletteFor(null);
    setTranslated(null);
    setTranslating(false);
    setDefined(null);
    setDefining(false);
  }, [selection?.text, selection?.top, selection?.left, initialNoteOpen]);

  /* 划选自动化（Y1 死设置修复，anx autoMark/autoTranslate 对齐）：
     autoMark：新划选立即应用默认高亮样式并收起（点既有划线唤起时跳过）；
     autoTranslate：新划选自动查翻译并在浮窗展示。 */
  useEffect(() => {
    if (!selection || initialNoteOpen) return;
    if (autoMark) {
      onSaveBookmark(selection.text, `highlight:${lastColors.highlight}`);
      onClose();
      return;
    }
    if (autoTranslate && onTranslate) {
      const myRequestId = ++requestIdRef.current;
      setTranslating(true);
      onTranslate(selection.text)
        .then((result) => {
          if (requestIdRef.current !== myRequestId) return;
          setTranslated(result);
        })
        .catch((error) => {
          if (requestIdRef.current !== myRequestId) return;
          console.error('auto translate failed:', error);
          setTranslated('');
        })
        .finally(() => {
          if (requestIdRef.current === myRequestId) setTranslating(false);
        });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selection?.text, initialNoteOpen]);

  /* 视口钳制：渲染后量取自身尺寸，避免超出顶部/左右边界；
     批注态：浮窗变高后若会压到划选文字（top..bottom 区间），
     翻转到划选区下方显示；下方也放不下再整体上提。 */
  useEffect(() => {
    if (!selection || !ref.current) return;
    const el = ref.current;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const selTop = selection.top;
    const selBottom = selection.bottom ?? selection.top + 28;
    let top = selection.top;
    const overlapsSelection = noteOpen && top + rect.height > selTop - 4 && top < selBottom + 4;
    if (overlapsSelection) {
      top = selBottom + 6;
    }
    if (top + rect.height > window.innerHeight - 8) {
      top = Math.max(8, window.innerHeight - rect.height - 8);
    }
    if (top < 8) top = Math.max(8, selection.top);
    const half = rect.width / 2;
    const minLeft = half + 8;
    const maxLeft = vw - half - 8;
    const clampedLeft = Math.min(Math.max(selection.left, minLeft), maxLeft);
    el.style.left = `${clampedLeft}px`;
    if (top !== selection.top) setAdjustedTop(top);
    else setAdjustedTop(null);
  }, [selection, noteOpen, paletteFor, translated, defined]);

  /* 点外收起：批注输入框打开期间（composing）不响应外部点击——
     用户可去点侧栏 tab 再回来继续写，浮窗不消失。
     iframe 正文点击不冒泡到宿主 document——经 pointerdown 桥由宿主转发
     （EpubReaderView 内 onDocPointerDismiss），这里同时监听两个事件名。 */
  useEffect(() => {
    if (!selection) return;
    const onPointerDown = (event: Event) => {
      if (ref.current?.contains(event.target as Node)) return;
      if (noteOpen) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('mt:iframe-pointerdown', onPointerDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('mt:iframe-pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [selection, noteOpen, onClose]);

  if (!selection) return null;

  const runTranslate = async () => {
    if (!selection || !onTranslate || translating) return;
    const myRequestId = ++requestIdRef.current;
    setTranslating(true);
    try {
      const result = await onTranslate(selection.text);
      if (requestIdRef.current !== myRequestId) return; // 过期响应丢弃
      setTranslated(result);
    } catch (error) {
      if (requestIdRef.current !== myRequestId) return;
      console.error('translate failed:', error);
      setTranslated('');
    } finally {
      if (requestIdRef.current === myRequestId) setTranslating(false);
    }
  };

  /* 取划词所在句（作为 define 的 context；找不到则用划选文本本身）。 */
  const sentenceOfSelection = (): string => {
    const text = selection?.text.trim() ?? '';
    if (!text || !pageText) return text;
    const idx = pageText.indexOf(text);
    if (idx === -1) return text;
    const start = Math.max(0, pageText.lastIndexOf('.', idx - 1));
    const endCandidates = [pageText.indexOf('.', idx + text.length), pageText.length];
    const end = Math.min(...endCandidates.filter((v) => v >= idx)) + 1;
    return pageText.slice(start === 0 ? 0 : start + 1, end).trim() || text;
  };

  const runDefine = async () => {
    if (!selection || !onDefine || defining || !isSingleWord) return;
    const myRequestId = ++requestIdRef.current;
    setDefining(true);
    try {
      const result = await onDefine(sentenceOfSelection(), selection.text.trim());
      if (requestIdRef.current !== myRequestId) return; // 过期响应丢弃
      setDefined(result);
    } catch (error) {
      if (requestIdRef.current !== myRequestId) return;
      console.error('define failed:', error);
      setDefined('');
    } finally {
      if (requestIdRef.current === myRequestId) setDefining(false);
    }
  };

  const submitNote = () => {
    const note = noteDraft.trim();
    if (!note) return;
    onSaveNote(selection.text, note);
    setNoteOpen(false);
    setNoteDraft('');
  };

  return (
    <motion.div
      ref={ref}
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 4 }}
      className="selmenu"
      style={{ top: adjustedTop ?? selection.top, left: selection.left, transform: 'translateX(-50%)' }}
    >
      {/* 工具栏常驻（点高亮唤出批注输入时也要有工具栏浮窗，
          批注框追加在下方，不再互斥隐藏） */}
      {(paletteFor ? (
          <div className="sel-palette" ref={swatchRef}>
            {/* 选色即应用当前样式（颜色独立按钮，选完直接划线） */}
            {MARK_PALETTE_COLORS.map((color) => (
              <button
                key={color}
                type="button"
                className="sel-palette-sw"
                style={{ background: color }}
                aria-label={color}
                onClick={() => onApplyColor(color)}
              />
            ))}
            <label className="sel-palette-custom" title={t.reader.customColor}>
              <input
                type="color"
                onChange={(e) => onApplyColor(e.target.value)}
              />
            </label>
            <button type="button" className="sel-palette-cancel" onClick={() => setPaletteFor(null)} aria-label={t.reader.cancel}>
              <X size={13} />
            </button>
          </div>
        ) : (
        <div className="sel-actions">
          {/* 样式按钮：单击=用上次颜色直接应用（无二次确认） */}
          {SELECTION_MARK_STYLES.map(({ id, icon: Icon, swatchClass }) => (
            <button
              key={id}
              type="button"
              className={swatchClass}
              title={id === 'none' ? t.reader.removeMark : ({ highlight: t.reader.markHighlight, underline: t.reader.markUnderline, wave: t.reader.markWave } as Record<string, string>)[id]}
              onClick={() => {
                if (id === 'highlight' || id === 'underline' || id === 'wave') {
                  applyMark(id, lastColors[id]);
                } else {
                  /* 橡皮擦：清除该引文的既有划线（此前误存 none 样式书签——
                     可视标记不动、数据越积越多，清除形同无效）。 */
                  onClearMark?.(selection.text);
                  onClose();
                }
              }}
            >
              <Icon size={13} />
            </button>
          ))}
          {/* 独立颜色按钮：打开色板为「当前样式」选色；选中即应用 */}
          <button
            type="button"
            className="mark-sw mark-color-btn"
            title={t.reader.customColor}
            aria-label={t.reader.customColor}
            onClick={() => setPaletteFor('highlight')}
          >
            <Palette size={13} />
          </button>
          {/* 批注/问AI/翻译/词典/书签：纯图标（浮窗不出文字），title 悬停提示 */}
          <button
            type="button"
            title={t.reader.annotate}
            aria-label={t.reader.annotate}
            onClick={() => setNoteOpen(true)}
          >
            <PenTool size={14} />
          </button>
          <button
            type="button"
            title={t.reader.askAi}
            aria-label={t.reader.askAi}
            onClick={() => onAskAi(selection.text)}
          >
            <Sparkles size={14} />
          </button>
          {onTranslate && (
            <button
              type="button"
              title={t.reader.translateSel}
              aria-label={t.reader.translateSel}
              onClick={() => void runTranslate()}
              disabled={translating}
            >
              <Languages size={14} />
            </button>
          )}
          {onDefine && isSingleWord && (
            <button
              type="button"
              title={t.reader.defineWord}
              aria-label={t.reader.defineWord}
              onClick={() => void runDefine()}
              disabled={defining}
            >
              <BookOpenText size={14} />
            </button>
          )}
          <button
            type="button"
            title={t.reader.addBookmark}
            aria-label={t.reader.addBookmark}
            onClick={() => onSaveBookmark(selection.text, 'bookmark')}
          >
            <Bookmark size={14} />
          </button>
          <button type="button" onClick={onClose} title={t.reader.cancel} aria-label="close">
            <X size={14} />
          </button>
        </div>
        ))}
      {!paletteFor && defined !== null && (
        <div className="sel-translate-box">
          <p className="sel-translate-text">
            <b>{selection.text}</b> {defined || t.reader.translateFailed}
          </p>
          {defined && onAddToVocab && (
            <div className="sel-translate-actions">
              <button
                type="button"
                className="sel-translate-save"
                onClick={() => {
                  onAddToVocab(selection.text, defined, sentenceOfSelection());
                  onClose();
                }}
              >
                {t.reader.saveToVocab}
              </button>
              {onAddToNotebook && (
                <button
                  type="button"
                  className="sel-translate-save ghost"
                  onClick={() => {
                    onAddToNotebook(selection.text, defined, sentenceOfSelection());
                    onClose();
                  }}
                >
                  {t.reader.saveToNotebook}
                </button>
              )}
            </div>
          )}
        </div>
      )}
      {!paletteFor && translated !== null && (
        <div className="sel-translate-box">
          <p className="sel-translate-text">{translated || t.reader.translateFailed}</p>
          {translated && onAddToVocab && (
            <button
              type="button"
              className="sel-translate-save"
              onClick={() => {
                onAddToVocab(selection.text, translated, sentenceOfSelection());
                onClose();
              }}
            >
              {t.reader.saveToVocab}
            </button>
          )}
        </div>
      )}
      {noteOpen && (
        <div className="sel-note-box">
          <div className="sel-note-quote">“{selection.text.length > 60 ? `${selection.text.slice(0, 60)}…` : selection.text}”</div>
          <textarea
            autoFocus
            value={noteDraft}
            onChange={(e) => setNoteDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) submitNote();
            }}
            placeholder={t.reader.notePlaceholder}
            rows={3}
          />
          <div className="sel-note-actions">
            <button type="button" className="sel-note-cancel" onClick={() => setNoteOpen(false)}>
              {t.reader.cancel}
            </button>
            <button type="button" className="sel-note-save" onClick={submitNote} disabled={!noteDraft.trim()}>
              {t.reader.save}
            </button>
          </div>
        </div>
      )}
    </motion.div>
  );
}
