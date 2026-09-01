/**
 * 应用 Logo（行内 SVG，主题自适应）：
 * - 曲线笔画用 currentColor——随所在文字色走，亮/暗主题与全部色板自动协调；
 * - 圆点用 var(--signal)——跟随主题强调色（夕珊瑚/翡翠/靛蓝/鎏金/紫罗兰）。
 * 系统图标（任务栏/安装包）见 src-tauri/icons（tauri icon 由 public/logo.svg 生成，
 * 用固定渐变以保证小尺寸可辨识）。
 */
export function Logo({ size = 22 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 200 200"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <circle cx="150" cy="50" r="15" fill="var(--signal, #b45309)" />
      <path
        d="M40 140 C 40 140, 50 80, 90 100 C 110 110, 100 150, 100 150"
        stroke="currentColor"
        strokeWidth="14"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M100 150 C 100 150, 110 100, 160 80"
        stroke="currentColor"
        strokeWidth="14"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.55"
      />
    </svg>
  );
}
