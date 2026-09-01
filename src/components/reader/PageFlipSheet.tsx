import { useEffect, useRef } from 'react';
import type { CSSProperties } from 'react';

/** 翻页前捕获的旧页快照（iframe 正文克隆 + 滚动偏移 + 注入样式）。 */
export interface PageSnapshot {
  html: string;
  css: string;
  scrollLeft: number;
  scrollTop: number;
  width: number;
  height: number;
}

export interface FlipSheetState {
  /** 折叠纸页（翻走前的活动页/列）。 */
  front: PageSnapshot;
  /** 静态底页（spread 的对面旧列）：折页落下前遮挡 iframe 已跳变的新列，
      否则整视口瞬变穿帮。 */
  under: PageSnapshot | null;
  direction: 'forward' | 'back';
}

interface PageFlipSheetProps {
  state: FlipSheetState;
  /** 舞台尺寸（全宽/高）。 */
  stageWidth: number;
  stageHeight: number;
  /** 布局：portrait=单页满幅 / spread=双页（右页向左折 / 左页向右折）。 */
  layout: 'portrait' | 'spread';
  /** 纸页底色（跟随阅读主题）。 */
  paperColor?: string;
  /** 动画时长（flip 快折传短值；默认按舞台宽度自适应）。 */
  durationMs?: number;
  onDone: () => void;
}

/** 快照 → 可滚动纸页内容（克隆 HTML 平移到指定列偏移）。 */
function SnapshotPane({ snap, extraStyle }: { snap: PageSnapshot; extraStyle?: CSSProperties }) {
  return (
    <div style={{ position: 'absolute', inset: 0, overflow: 'hidden', ...extraStyle }}>
      <div
        style={{
          width: snap.width,
          height: snap.height,
          transform: `translate(${-snap.scrollLeft}px, ${-snap.scrollTop}px)`,
          transformOrigin: '0 0',
        }}
        dangerouslySetInnerHTML={{ __html: `<style>${snap.css}</style>${snap.html}` }}
      />
    </div>
  );
}

/** 点击触发的 3D 纸页翻页层（重写版）。
    模型统一为「旧页翻走」：
    - portrait forward：旧整页绕左缘向左翻走（rotateY 0→-180°），露出 iframe 新页；
    - portrait back：旧整页绕右缘向右翻走（0→+180°）；
    - spread forward：旧右列绕中缝向左折（0→-180°），落下覆盖左半（纸背朝上）；
    - spread back：旧左列绕中缝向右折（0→+180°），落下覆盖右半；
    - under（spread 对面旧列）静态垫底防跳变穿帮；结尾整层快速淡出落定。
    纸页为双面元素：正面=旧页快照，背面=纸色。
    rAF 直接写 style，不走 setState；perspective 投影让远端收缩，近似真实纸页透视。 */
export function PageFlipSheet({
  state,
  stageWidth,
  stageHeight,
  layout,
  paperColor,
  durationMs,
  onDone,
}: PageFlipSheetProps) {
  const sheetRef = useRef<HTMLDivElement | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const frontShadeRef = useRef<HTMLDivElement | null>(null);
  const backShadeRef = useRef<HTMLDivElement | null>(null);
  const underRef = useRef<HTMLDivElement | null>(null);

  const isSpread = layout === 'spread';
  const pageW = isSpread ? stageWidth / 2 : stageWidth;
  const pageH = stageHeight;
  /* sheet 舞台定位：spread forward=右半、back=左半；portrait=满幅 */
  const sheetLeft = isSpread && state.direction === 'forward' ? stageWidth / 2 : 0;
  /* 旋转轴（书脊）：forward 绕左缘（spread=中缝）、back 绕右缘 */
  const originX = state.direction === 'forward' ? 'left center' : 'right center';

  useEffect(() => {
    const sheet = sheetRef.current;
    const wrap = wrapRef.current;
    const frontShade = frontShadeRef.current;
    const backShade = backShadeRef.current;
    const under = underRef.current;
    if (!sheet || !wrap || stageWidth <= 0 || stageHeight <= 0) {
      onDone();
      return;
    }

    /* forward=向左翻（负角）；back=向右翻（正角） */
    const dir = state.direction === 'forward' ? -1 : 1;
    const duration = durationMs ?? Math.max(320, Math.min(720, pageW * 0.42));
    const startedAt = performance.now();
    let rafId = 0;
    let finished = false;

    const tick = (now: number) => {
      if (finished) return;
      const t = Math.min(1, (now - startedAt) / duration);
      /* smoothstep：起手缓、中段快、落定缓——纸页被掀起后受重力加速、落定前减速 */
      const eased = t * t * (3 - 2 * t);
      const rad = dir * Math.PI * eased;
      sheet.style.transform = `rotateY(${rad}rad)`;
      /* 阴影随翻转角变化：正对时无影，侧立时最深 */
      const lift = Math.sin(Math.PI * eased);
      if (frontShade) frontShade.style.opacity = String(0.32 * lift);
      if (backShade) backShade.style.opacity = String(0.4 * lift * (1 - 0.5 * t));
      /* 结尾淡出：末段 12% 让纸页与底页渐隐落定，避免硬切 */
      if (t > 0.88) {
        const fade = (t - 0.88) / 0.12;
        wrap.style.opacity = String(1 - fade);
        if (under) under.style.opacity = String(1 - fade);
      }

      if (t >= 1) {
        finished = true;
        window.clearTimeout(safety);
        onDone();
        return;
      }
      rafId = requestAnimationFrame(tick);
    };

    /* 兜底收尾：窗口遮挡/最小化时 WebView 暂停 rAF，动画会冻在中途；
       超时后强制落定移除翻页层，防止旧页快照永久盖住正文。 */
    const safety = window.setTimeout(() => {
      if (finished) return;
      finished = true;
      cancelAnimationFrame(rafId);
      onDone();
    }, duration + 300);

    rafId = requestAnimationFrame(tick);
    return () => {
      finished = true;
      cancelAnimationFrame(rafId);
      window.clearTimeout(safety);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, stageWidth, stageHeight, layout, durationMs]);

  const paper = paperColor ?? 'var(--card, #fff)';

  /* under（spread 对面旧列）：整段动画期间静态垫底，末段随整体淡出 */
  const underPane = state.under && isSpread
    ? {
        position: 'absolute' as const,
        top: 0,
        left: state.direction === 'forward' ? 0 : stageWidth / 2,
        width: stageWidth / 2,
        height: stageHeight,
        zIndex: 1,
      }
    : null;

  return (
    <div
      ref={wrapRef}
      className="page-flip-mask"
      style={{
        position: 'absolute',
        inset: 0,
        overflow: 'hidden',
        zIndex: 4,
        pointerEvents: 'none',
        /* 透视：视距≈1.2 舞台宽，远端纸缘收缩近似真实纸页 */
        perspective: `${Math.max(600, stageWidth * 1.2)}px`,
        perspectiveOrigin: '50% 50%',
      }}
    >
      {underPane && (
        <div ref={underRef} className="page-flip-under" style={underPane}>
          <SnapshotPane snap={state.under!} />
        </div>
      )}
      <div
        ref={sheetRef}
        className="page-flip-sheet"
        style={{
          position: 'absolute',
          top: 0,
          left: sheetLeft,
          width: pageW,
          height: pageH,
          transformOrigin: originX,
          transformStyle: 'preserve-3d',
          zIndex: 2,
        }}
      >
        {/* 正面：旧页快照（翻起前半程可见） */}
        <div
          style={{
            position: 'absolute',
            inset: 0,
            backfaceVisibility: 'hidden',
            background: paper,
            overflow: 'hidden',
            boxShadow: '0 0 32px rgba(28,28,26,.28)',
          }}
        >
          <SnapshotPane snap={state.front} />
          {/* 书脊侧静态渐变 + 动态翻转阴影 */}
          <div
            ref={frontShadeRef}
            style={{
              position: 'absolute',
              inset: 0,
              opacity: 0,
              background:
                state.direction === 'forward'
                  ? 'linear-gradient(to left, transparent 70%, rgba(28,28,26,.18))'
                  : 'linear-gradient(to right, transparent 70%, rgba(28,28,26,.18))',
            }}
          />
        </div>
        {/* 背面：纸背色（落地后整层淡出，由 iframe 新列接管） */}
        <div
          style={{
            position: 'absolute',
            inset: 0,
            backfaceVisibility: 'hidden',
            transform: 'rotateY(180deg)',
            background: paper,
            backgroundImage:
              'linear-gradient(105deg, rgba(28,28,26,.05) 0%, transparent 30%, transparent 70%, rgba(28,28,26,.04) 100%)',
            boxShadow: '0 0 32px rgba(28,28,26,.32)',
          }}
        >
          <div
            ref={backShadeRef}
            style={{
              position: 'absolute',
              inset: 0,
              opacity: 0,
              background:
                state.direction === 'forward'
                  ? 'linear-gradient(to right, rgba(28,28,26,.22), transparent 55%)'
                  : 'linear-gradient(to left, rgba(28,28,26,.22), transparent 55%)',
            }}
          />
        </div>
      </div>
    </div>
  );
}
