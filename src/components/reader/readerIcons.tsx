/* 阅读器专属图标：为顶栏手绘的语义化 SVG（笔画节奏与 lucide 1.75 描边对齐，
   语义比通用库图标更贴近动作本身——书页、目录书脊、批注墨线、阅读灯、纸纹）。 */
import type { SVGProps } from 'react';

type IconProps = SVGProps<SVGSVGElement> & { size?: number | string };

function base({ size = 17, ...rest }: IconProps): SVGProps<SVGSVGElement> {
  return {
    width: size,
    height: size,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.75,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
    ...rest,
  };
}

/** 双语对照：左右两片书页自中缝展开，页心各留一行文脉。 */
export function IconBilingual(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M12 5.5C10.4 4.2 8.2 3.6 5 3.6c-.9 0-1.5.6-1.5 1.4v11.6c0 .8.6 1.4 1.5 1.4 3.2 0 5.4.6 7 1.9 1.6-1.3 3.8-1.9 7-1.9.9 0 1.5-.6 1.5-1.4V5c0-.8-.6-1.4-1.5-1.4-3.2 0-5.4.6-7 1.9Z" />
      <path d="M12 5.5v14.4" />
      <path d="M6.4 8.2c1.5.1 2.7.4 3.6 1M6.4 11.4c1.5.1 2.7.4 3.6 1" />
      <path d="M17.6 8.2c-1.5.1-2.7.4-3.6 1M17.6 11.4c-1.5.1-2.7.4-3.6 1" />
    </svg>
  );
}

/** 生词透析：单词三字母底下浮出放大透镜（词被逐字看清）。 */
export function IconWordWise(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M3 7.5 5.4 4 7.8 7.5" />
      <path d="M3.9 6.2h3" />
      <path d="M10.2 4h3.6M12 4v3.5" />
      <circle cx="15" cy="14.6" r="4.6" />
      <path d="m18.4 17.9 2.6 2.6" />
      <path d="M13.2 14.6h3.6" />
    </svg>
  );
}

/** 单页：一张带折角的纸。 */
export function IconPageSingle(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M6.5 3.5h7.2L18.5 8.3v12.2h-12V3.5Z" />
      <path d="M13.7 3.5v4.8h4.8" />
      <path d="M9.3 12.4h5.4M9.3 15.6h5.4" />
    </svg>
  );
}

/** 双页展开：中缝对开的两张纸。 */
export function IconPageSpread(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M12 4.5C10.2 3.4 7.8 3 4.5 3v15.5c3.3 0 5.7.4 7.5 1.5 1.8-1.1 4.2-1.5 7.5-1.5V3c-3.3 0-5.7.4-7.5 1.5Z" />
      <path d="M12 4.5V20" />
      <path d="M7 8h2.4M7 11.2h2.4M14.6 8H17M14.6 11.2H17" />
    </svg>
  );
}

/** 滚动：卷起的纸卷垂下一条文本流。 */
export function IconPageScroll(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M7 5.5A2.5 2.5 0 0 1 9.5 3h8A2.5 2.5 0 0 1 20 5.5 2.5 2.5 0 0 1 17.5 8h-8" />
      <path d="M9.5 3A2.5 2.5 0 0 0 7 5.5V18.5A2.5 2.5 0 0 0 9.5 21h6a2.5 2.5 0 0 0 2.5-2.5V8" />
      <path d="M10.4 11.5h5.2M10.4 14.7h5.2" />
    </svg>
  );
}

/** 目录：书脊 + 参差目录条（章回长短不一）。 */
export function IconToc(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M5 3.5v17" />
      <path d="M8.5 5.5H19M8.5 9h7.2M8.5 12.5h9.4M8.5 16h5.8" />
    </svg>
  );
}

/** 笔记批注：页面折角垂落一行墨迹笔触。 */
export function IconNote(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M4.5 3.5h15v11.2L15.2 19h-10.7v-15.5Z" />
      <path d="M19.5 14.7h-4.3V19" />
      <path d="M7.6 8h6.8M7.6 11.2h4.6" />
    </svg>
  );
}

/** 朗读出声：书页上方两道音波。 */
export function IconReadAloud(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M5 11.5v7.6c0 .7.6 1.3 1.3 1.3 2.8 0 4.7.5 5.7 1.4 1-.9 2.9-1.4 5.7-1.4.7 0 1.3-.6 1.3-1.3v-7.6" />
      <path d="M12 11.5v10.2" />
      <path d="M9.6 8.4a3.4 3.4 0 0 1 4.8 0" />
      <path d="M7.4 5.9a6.6 6.6 0 0 1 9.2 0" />
    </svg>
  );
}

/** 阅读灯：台灯罩 + 灯柱 + 投下的光带（主题/氛围切换）。 */
export function IconReadingLamp(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="m8.2 3.6 9.4 3.2-2.1 3.9-9.4-3.2 2.1-3.9Z" />
      <path d="M13.4 10.5 10 20.4" />
      <path d="M6 20.4h8" />
      <path d="M16.8 10.9 18 13M18.9 8.3l2.3 1" />
    </svg>
  );
}

/** 校对模式：段后勾验。 */
export function IconReviewMode(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M4 5.5h16M4 9.5h9M4 13.5h6.6M4 17.5h7.4" />
      <path d="m13.5 16.6 2.3 2.3 4.4-4.8" />
    </svg>
  );
}

/** 打开校对工作台：书页上的放大镜。 */
export function IconReviewDesk(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M4.5 4.5h12v11.2l-3.2 3.3H4.5V4.5Z" />
      <path d="M16.5 15.7l3 3" />
      <circle cx="12" cy="10.6" r="3.1" />
    </svg>
  );
}
