import { motion } from 'motion/react';
import { AlertCircle, BookPlus, Check, GraduationCap, Loader2, Maximize2, Minimize2, RotateCcw, Sparkles, Volume2, X } from 'lucide-react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useT } from '@/i18n';
import { decodeExchange, freqText, tagLabelsFrom } from '@/lib/wordCardBits';
import type { WordMark } from '@/types';
import type { WordLevelInfo } from './wordWiseInject';
import './wordwise.css';

export interface WordCardState {
  info: WordLevelInfo;
  context: string;
  /** 例句译文（已停用：翻译是段落级对齐而非句子对齐，句级中文会张冠李戴；
      保留字段兼容生词本成卡链路的旧签名，不再展示）。 */
  contextZh?: string;
  /** 词条在宿主坐标系中的矩形（词位映射）：卡片据此实测避让四边。 */
  rect: { left: number; top: number; width: number; height: number };
}

interface WordWiseCardProps {
  state: WordCardState;
  mark?: WordMark;
  onMarkWord: (word: string, status: 'mastered' | 'learning' | null, context: string) => void;
  onAddCard: (word: string, context: string, customDefinition?: string, contextZh?: string) => void;
  onSaveDefinition: (word: string, definition: string, context: string) => void;
  /** TTS 朗读：不传则隐藏发音按钮。 */
  onSpeak?: (text: string) => void;
  ttsSpeaking?: boolean;
  onClose: () => void;
}

/* 尺寸约束：收起态自适应内容；展开态加宽加长但不覆盖全屏（≤92vw/85vh）；
   resize 手柄在 [min, 视口上限] 内拖动。 */
const SIZE_MIN = { w: 260, h: 150 };
const SIZE_EXPANDED = { w: 560, h: 420 };
const VIEWPORT_MARGIN = 8;

/** AI 释义状态机：idle → loading（含流式缓冲）→ done / error（可重试）。
    requestId 竞态防护：卡片切换词或连续点击时，过期响应直接丢弃
    。 */
type AiDefineState =
  | { phase: 'idle' }
  | { phase: 'loading'; streamText: string }
  | { phase: 'done' }
  | { phase: 'error'; message: string };

/** 生词注释卡片：档级/音标/释义 + 图标动作（已掌握/加入学习计划/AI 释义）；
    支持展开（加宽加长、内容区滚动）与右下角拖拽调尺寸；四向边缘避让。 */
export function WordWiseCard({ state, mark, onMarkWord, onAddCard, onSaveDefinition, onSpeak, ttsSpeaking = false, onClose }: WordWiseCardProps) {
  const t = useT();
  const wc = t.reader.wordCard;
  const ref = useRef<HTMLDivElement | null>(null);
  const [ai, setAi] = useState<AiDefineState>({ phase: 'idle' });
  const [expanded, setExpanded] = useState(false);
  const [customSize, setCustomSize] = useState<{ w: number; h: number } | null>(null);
  const requestIdRef = useRef(0);
  const { info } = state;
  const customDefinition = mark?.customDefinition?.trim();
  const senses = customDefinition ? [customDefinition] : splitDefinitions(info.definition);
  /* ECDICT 增量（翻译/标签/星级/词形/词根/词频）：与词表释义同体时不重复展示。 */
  const translationSenses = useMemo(() => {
    if (customDefinition) return [];
    const lines = splitDefinitions(info.translation ?? '');
    return lines.filter((line) => !senses.includes(line));
  }, [customDefinition, info.translation, senses]);
  const tagLabels = useMemo(() => tagLabelsFrom(info.tag), [info.tag]);
  const forms = useMemo(() => decodeExchange(info.exchange, info.word, t.reader.formLabels),
    [info.exchange, info.word, t.reader.formLabels],
  );
  const freq = freqText(info);

  /* 换词即重置 AI 状态并使在途请求失效（竞态防护的另一侧）。 */
  useEffect(() => {
    requestIdRef.current += 1;
    setAi({ phase: 'idle' });
  }, [info.word]);

  /* 展开/收起：切换目标尺寸并清除 resize 记忆。 */
  const toggleExpanded = useCallback(() => {
    setCustomSize(null);
    setExpanded((v) => !v);
  }, []);

  /* 拖拽调尺寸：pointermove 内 clamp 到 [min, 视口 92%/85%]。 */
  const startResize = useCallback((event: React.PointerEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startY = event.clientY;
    const base = {
      w: ref.current?.offsetWidth ?? SIZE_EXPANDED.w,
      h: ref.current?.offsetHeight ?? SIZE_EXPANDED.h,
    };
    const maxW = Math.floor(window.innerWidth * 0.92) - VIEWPORT_MARGIN * 2;
    const maxH = Math.floor(window.innerHeight * 0.85) - VIEWPORT_MARGIN * 2;
    const onMove = (moveEvent: PointerEvent) => {
      setCustomSize({
        w: clamp(base.w + (moveEvent.clientX - startX), SIZE_MIN.w, maxW),
        h: clamp(base.h + (moveEvent.clientY - startY), SIZE_MIN.h, maxH),
      });
      setExpanded(true);
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }, []);

  useEffect(() => {
    const onPointerDown = (event: Event) => {
      if (ref.current && !ref.current.contains(event.target as Node)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    // iframe 正文点击经宿主转发事件（mt:iframe-pointerdown）——否则点正文关不掉卡片
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('mt:iframe-pointerdown', onPointerDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('mt:iframe-pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [onClose]);

  const handleAiDefine = () => {
    if (ai.phase === 'loading' || customDefinition) return;
    const myRequestId = ++requestIdRef.current;
    setAi({ phase: 'loading', streamText: '' });
    // 流式通道：逐 chunk 追加到弹卡。
    const channel = new Channel<string>();
    channel.onmessage = (delta) => {
      if (requestIdRef.current !== myRequestId) return;
      setAi((prev) =>
        prev.phase === 'loading'
          ? { phase: 'loading', streamText: prev.streamText + delta }
          : prev,
      );
    };
    invoke<{ answer: string }>('ai_assist_paragraph_streaming', {
      action: 'define',
      sourceText: state.context,
      question: info.word,
      targetLanguage: null,
      history: null,
      onChunk: channel,
    })
      .then((response) => {
        if (requestIdRef.current !== myRequestId) return; // 过期响应丢弃
        const definition = response.answer.trim();
        if (definition) onSaveDefinition(info.word, definition, state.context);
        setAi({ phase: 'done' });
        if (definition) onClose();
      })
      .catch((error) => {
        if (requestIdRef.current !== myRequestId) return; // 过期失败丢弃
        console.error('AI define failed:', error);
        setAi({
          phase: 'error',
          message: error instanceof Error ? error.message : String(error),
        });
      });
  };

  /* 尺寸：resize 记忆 > 展开态预设 > 收起态（max-content 自适应）。
     useMemo 稳定引用：落位 effect 以它为依赖，避免每次渲染新对象触发重测循环。 */
  const dims = useMemo(() => {
    if (customSize) return { width: customSize.w, height: customSize.h };
    if (expanded) return { width: SIZE_EXPANDED.w, height: SIZE_EXPANDED.h };
    return null;
  }, [customSize, expanded]);

  /* 四向避让（实测式）：以词条矩形为锚，先量卡片自身尺寸再落位。
     - 纵向：下方放得下就放下方；放不下翻到上方；两边都放不下靠向余量更大的一侧并夹回视口；
     - 横向：以词位为中心，右侧/左侧贴边时整卡平移回视口（右端词 → 卡片右缘贴视口边）。
     useLayoutEffect 在绘制前完成测量与定位，首帧不闪。 */
  const [pos, setPos] = useState<CardPlacement | null>(null);
  const above0 = state.rect.top + state.rect.height + SIZE_MIN.h > window.innerHeight;
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const compute = () => {
      const width = dims ? Math.min(dims.width, Math.max(SIZE_MIN.w, window.innerWidth - VIEWPORT_MARGIN * 2)) : el.offsetWidth;
      const height = dims ? Math.min(dims.height, Math.max(SIZE_MIN.h, window.innerHeight - VIEWPORT_MARGIN * 2)) : el.offsetHeight;
      setPos(placeCard(state.rect, width, height));
    };
    compute();
    window.addEventListener('resize', compute);
    return () => window.removeEventListener('resize', compute);
  }, [state.rect, dims]);

  const scrollable = Boolean(dims);
  const style: React.CSSProperties = pos
    ? {
        left: pos.left,
        top: pos.top,
        ...(dims ? { width: Math.min(dims.width, Math.max(SIZE_MIN.w, window.innerWidth - VIEWPORT_MARGIN * 2)), height: Math.min(dims.height, Math.max(SIZE_MIN.h, window.innerHeight - VIEWPORT_MARGIN * 2)) } : {}),
      }
    : { left: -9999, top: 0, visibility: 'hidden' };
  const above = pos ? pos.above : above0;

  return (
    <motion.div
      ref={ref}
      initial={{ opacity: 0, y: above ? -6 : 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: above ? -4 : 4 }}
      className={`word-wise-card ${expanded ? 'word-wise-card-expanded' : ''} ${scrollable ? 'word-wise-card-scroll' : ''}`}
      role="dialog"
      style={style}
    >
      <div className="wwc-head">
        <strong className="wwc-word">{info.word}</strong>
        {onSpeak && (
          <button
            type="button"
            className="iconbtn wwc-speak"
            title={ttsSpeaking ? wc.speakStop : wc.speak}
            aria-label={wc.speak}
            onClick={(e) => {
              e.stopPropagation();
              onSpeak(info.word);
            }}
          >
            <Volume2 size={13} />
          </button>
        )}
        {info.phonetic && <span className="wwc-phonetic">{info.phonetic}</span>}
        <span className="wwc-level">{info.level}</span>
        {customDefinition && <span className="wwc-ai-tag">{wc.aiTag}</span>}
        <div className="wwc-head-actions">
          <button
            type="button"
            className="iconbtn"
            style={{ width: 28, height: 28 }}
            onClick={toggleExpanded}
            aria-label={expanded ? wc.collapse : wc.expand}
            title={expanded ? wc.collapseShort : wc.expandHint}
          >
            {expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
          </button>
          <button
            type="button"
            className="iconbtn"
            style={{ width: 28, height: 28 }}
            onClick={onClose}
            aria-label={wc.close}
          >
            <X size={14} />
          </button>
        </div>
      </div>
      <div className="wwc-body">
        <p className="wwc-def">
          {expanded ? senses.join('\n') : (senses[0] ?? info.definition)}
          {!expanded && senses.length === 0 && (
            <span className="wwc-def-empty">{wc.defEmpty}</span>
          )}
        </p>
        {/* ECDICT 完整翻译：比词表释义更全（词性分段）；与词表释义重复时不重复展示 */}
        {expanded && translationSenses.length > 0 && (
          <p className="wwc-def wwc-def-alt">{translationSenses.join('\n')}</p>
        )}
        {/* 考试标签 / 牛津3000 / 柯林斯星级（难度分级 × 词典数据合流） */}
        {(tagLabels.length > 0 || Boolean(info.oxford) || Boolean(info.collins)) && (
          <div className="wwc-meta">
            {tagLabels.map((label) => (
              <span key={label} className="wwc-chip">{label}</span>
            ))}
            {info.oxford ? <span className="wwc-chip wwc-chip-oxford">{wc.oxford}</span> : null}
            {info.collins ? (
              <span className="wwc-stars" title={`${wc.collinsTitle} ${info.collins} ★`}>
                {'★'.repeat(info.collins)}{'☆'.repeat(Math.max(0, 5 - info.collins))}
              </span>
            ) : null}
          </div>
        )}
        {/* 词形变换（ECDICT exchange 解码）与词根助记、词频排名（展开态） */}
        {expanded && forms.length > 0 && (
          <p className="wwc-forms">
            {forms.map((f) => `${f.label} ${f.value}`).join(' · ')}
          </p>
        )}
        {expanded && info.root && <p className="wwc-root">{wc.root}：{info.root}</p>}
        {expanded && freq && <p className="wwc-freq">{freq}</p>}
        {/* 流式缓冲区：生成中的逐字上屏（含光标闪烁） */}
        {ai.phase === 'loading' && ai.streamText && (
          <p className="wwc-def wwc-stream" aria-live="polite">
            {ai.streamText}
            <span className="wwc-cursor" />
          </p>
        )}
        <p className="wwc-ctx">
          <span className="wwc-ctx-mark">“</span>
          {expanded ? state.context : truncate(state.context, 90)}
          <span className="wwc-ctx-mark">”</span>
          {onSpeak && (
            <button
              type="button"
              className="iconbtn wwc-speak wwc-speak-ctx"
              title={wc.speakCtx}
              aria-label={wc.speakCtx}
              onClick={(e) => {
                e.stopPropagation();
                onSpeak(state.context);
              }}
            >
              <Volume2 size={12} />
            </button>
          )}
        </p>
        {ai.phase === 'error' && (
          <p className="wwc-error" role="alert">
            <AlertCircle size={13} />
            <span>{truncate(ai.message, 80)}</span>
            <button type="button" className="wwc-retry" onClick={handleAiDefine}>
              <RotateCcw size={12} />
              {wc.retry}
            </button>
          </p>
        )}
      </div>
      <div className="wwc-actions" role="toolbar" aria-label={wc.actions}>
        <button
          type="button"
          className={`iconbtn ${mark?.status === 'mastered' ? 'on' : ''}`}
          style={{ width: 32, height: 32 }}
          title={mark?.status === 'mastered' ? wc.unmaster : wc.mastered}
          onClick={() => {
            onMarkWord(info.word, mark?.status === 'mastered' ? null : 'mastered', state.context);
            onClose();
          }}
        >
          <Check size={16} />
        </button>
        <button
          type="button"
          className={`iconbtn ${mark?.status === 'learning' ? 'on' : ''}`}
          style={{ width: 32, height: 32 }}
          title={wc.learning}
          onClick={() => {
            onMarkWord(info.word, mark?.status === 'learning' ? null : 'learning', state.context);
          }}
        >
          <GraduationCap size={16} />
        </button>
        <button
          type="button"
          className="iconbtn"
          style={{ width: 32, height: 32 }}
          title={wc.addCard}
          onClick={() => {
            onAddCard(info.word, state.context, mark?.customDefinition?.trim() || undefined, state.contextZh);
            onClose();
          }}
        >
          <BookPlus size={16} />
        </button>
        <button
          type="button"
          className="iconbtn"
          style={{ width: 32, height: 32 }}
          title={customDefinition ? wc.aiDone : wc.aiDefine}
          disabled={Boolean(customDefinition) || ai.phase === 'loading'}
          onClick={handleAiDefine}
        >
          <Sparkles size={16} />
        </button>
        {ai.phase === 'loading' && !ai.streamText && (
          <span className="wwc-busy" style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <Loader2 size={11} className="wwc-spin" />
            {wc.generating}
          </span>
        )}
      </div>
      {/* 右下角 resize 手柄（拖拽调宽高；展开态下显示） */}
      {expanded && (
        <button
          type="button"
          className="wwc-resizer"
          onPointerDown={startResize}
          aria-label="调整卡片大小"
        />
      )}
    </motion.div>
  );
}

/** 卡片落位结果（宿主坐标，已含四向避让）。 */
interface CardPlacement {
  left: number;
  top: number;
  above: boolean;
}

/** 四向避让定位（实测卡片尺寸后调用）：
    - 纵向：词条下方放得下（含边距）就放下方；否则翻到上方；
      两侧都放不下（极矮视口）靠向余量更大的一侧并夹回视口内。
    - 横向：以词条中心为基准居中；整卡右缘/左缘超出视口时平移回贴边，
      右端词自然形成「卡片右缘贴视口右边缘」的收敛。 */
function placeCard(
  rect: { left: number; top: number; width: number; height: number },
  cardWidth: number,
  cardHeight: number,
): CardPlacement {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const belowTop = rect.top + rect.height + 8;
  const aboveTop = rect.top - 8 - cardHeight;
  const fitsBelow = belowTop + cardHeight <= vh - VIEWPORT_MARGIN;
  const fitsAbove = aboveTop >= VIEWPORT_MARGIN;
  let above: boolean;
  let top: number;
  if (fitsBelow) {
    above = false;
    top = belowTop;
  } else if (fitsAbove) {
    above = true;
    top = aboveTop;
  } else {
    /* 两侧都放不下：靠向剩余空间更大的一侧，整体夹回视口。 */
    above = rect.top > vh / 2;
    top = clamp(above ? aboveTop : belowTop, VIEWPORT_MARGIN, Math.max(VIEWPORT_MARGIN, vh - cardHeight - VIEWPORT_MARGIN));
  }
  const centerX = rect.left + rect.width / 2;
  const left = clamp(centerX - cardWidth / 2, VIEWPORT_MARGIN, Math.max(VIEWPORT_MARGIN, vw - cardWidth - VIEWPORT_MARGIN));
  return { left, top, above };
}

function splitDefinitions(definition: string): string[] {
  return definition
    .split(/\n+/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function truncate(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max)}…`;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
