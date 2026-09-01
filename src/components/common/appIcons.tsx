/* 应用主导航手绘图标：左栏/底部栏七景，笔画与 lucide 1.75 描边同节奏。
   每枚取其物象——书架、便笺、记忆卡、校样、词卡盒、洞察线、译稿队列。 */
import type { SVGProps } from 'react';

type IconProps = SVGProps<SVGSVGElement> & { size?: number | string };

function base({ size = 20, ...rest }: IconProps): SVGProps<SVGSVGElement> {
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

/** 书库：架上三册，一册斜倚。 */
export function IconLibrary(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M4 20.5h16" />
      <path d="M6.2 20.5V6.8h3v13.7" />
      <path d="M11 20.5V4.8h3v15.7" />
      <path d="m15.4 20.5 2.6-13 2.9.6-2.6 12.9" />
      <path d="M7.7 9.4v2.2M12.5 7.6v2.2" />
    </svg>
  );
}

/** 笔记：横线便笺，右上角夹一支笔。 */
export function IconNotes(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M4.5 5A1.5 1.5 0 0 1 6 3.5h9.5l4 4V19a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 19V5Z" />
      <path d="M15.5 3.5v4h4" />
      <path d="M8 12h8M8 15.5h5.5" />
    </svg>
  );
}

/** 学习：翻动的记忆卡，卡面一道间隔重现的循环弧。 */
export function IconLearning(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M6.8 4.5h11.4A1.3 1.3 0 0 1 19.5 5.8v11.4a1.3 1.3 0 0 1-1.3 1.3H6.8a1.3 1.3 0 0 1-1.3-1.3V5.8a1.3 1.3 0 0 1 1.3-1.3Z" />
      <path d="M17.5 20.5H5.5A1.5 1.5 0 0 1 4 19v-11" />
      <path d="M9.2 10.2a3.2 3.2 0 0 1 5.5-.6l1 .9" />
      <path d="M15.7 7.9v2.6h-2.6" />
      <path d="M14.8 13.3a3.2 3.2 0 0 1-5.5.6l-1-.9" />
      <path d="M8.3 15.6V13h2.6" />
    </svg>
  );
}

/** 校对：稿纸三行，朱笔校勾。 */
export function IconReview(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M5 3.5h14v17H5v-17Z" />
      <path d="M8 7.5h8M8 11h5.4M8 14.5h4" />
      <path d="m13.4 16.8 1.6 1.7 3.4-3.6" />
    </svg>
  );
}

/** 术语：词卡盒里抽出一张索引卡。 */
export function IconTerms(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M4.5 9h15V18a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 18V9Z" />
      <path d="M4.5 9 6 5.5h12L19.5 9" />
      <path d="M9 12.5h6M9 15.5h3.8" />
    </svg>
  );
}

/** 洞察：走势线 + 放大镜聚焦的峰值点。 */
export function IconInsights(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M3.5 16.5 8 11l3.2 2.8L15.5 8" />
      <circle cx="17.5" cy="6.2" r="2.7" />
      <path d="m19.5 8.2 1.6 1.7" />
      <path d="M3.5 20h17" />
    </svg>
  );
}

/** 任务：待译稿堆，头页进行中一半实线一半虚线（翻译进度意象）。 */
export function IconTasks(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M7 5.5h13v13.5H7z" />
      <path d="M4.5 8v13H17" />
      <path d="M10 9.5h7M10 12.5h7" />
      <path d="M10 15.5h3.4" strokeDasharray="1.5 2.4" />
    </svg>
  );
}
