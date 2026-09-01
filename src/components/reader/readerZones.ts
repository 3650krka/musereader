/** 触控翻页分区模型。
    统一建模为 9 格数组（行优先，0=左上 … 8=右下），运行时按点击坐标查表。
    预设语义：
    - standard   左右翻页、中央唤出工具栏
    - swapped    standard 左右镜像（左利手）
    - centerTurn 两侧唤出工具栏、中央左半上一页/右半下一页
    - topMenu    顶部一行唤出工具栏、其余区域翻页（单手大屏友好）
    - leftHand   绝大部分区域下一页、仅中列中央上一页（通勤单手连读）
    - custom     用户逐格自定义（pageTurnCustomZones） */
import type { ReaderPageTurnZones } from './epubReaderTypes';

export type ReaderZoneAction = 'none' | 'prev' | 'next' | 'menu';

/** 9 格预设（行优先 3×3）。 */
export const ZONE_PRESETS: Record<Exclude<ReaderPageTurnZones, 'custom'>, ReaderZoneAction[]> = {
  standard: [
    'prev', 'menu', 'next',
    'prev', 'menu', 'next',
    'prev', 'menu', 'next',
  ],
  swapped: [
    'next', 'menu', 'prev',
    'next', 'menu', 'prev',
    'next', 'menu', 'prev',
  ],
  /* centerTurn 语义是一维的（左右 30% 唤出、中央左右半翻页），3×3 近似为：
     左列 menu、中列 prev、右列 next——运行时为保兼容仍走一维分支。 */
  centerTurn: [
    'menu', 'prev', 'next',
    'menu', 'prev', 'next',
    'menu', 'prev', 'next',
  ],
  topMenu: [
    'menu', 'menu', 'menu',
    'prev', 'next', 'next',
    'prev', 'next', 'next',
  ],
  leftHand: [
    'next', 'menu', 'next',
    'next', 'prev', 'next',
    'next', 'next', 'next',
  ],
};

/** 点击坐标 → 9 格索引（行列各三等分）。 */
export function zoneIndex(ratioX: number, ratioY: number): number {
  const col = ratioX < 1 / 3 ? 0 : ratioX < 2 / 3 ? 1 : 2;
  const row = ratioY < 1 / 3 ? 0 : ratioY < 2 / 3 ? 1 : 2;
  return row * 3 + col;
}

/** custom 模式下按用户 9 格配置查动作；数组非法时回落 standard。 */
export function customZoneAction(
  zones: ReaderZoneAction[],
  ratioX: number,
  ratioY: number,
): ReaderZoneAction {
  const grid = zones.length === 9 ? zones : ZONE_PRESETS.standard;
  return grid[zoneIndex(ratioX, ratioY)];
}
