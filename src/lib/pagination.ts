/** 伪分页契约（与后端 commands/toc_edit.rs::SOURCE_BLOCKS_PER_PAGE 同源）。
    EPUB 等无版面书籍按每页 30 个「校对段对」（可翻译且原文非空的块）分页：
    - 校对工作台分页模式每页显示的内容与页码预览/目录页码归属同一坐标系；
    - 页码跳转 = 段对序号 (page-1)*30。
    若调整粒度，前后端两处必须同步。 */
export const BLOCKS_PER_PSEUDO_PAGE = 30;

/** 段对序号（0 基）所属伪页码（1 基）。 */
export function pageOfSegmentIndex(index: number): number {
  return Math.floor(Math.max(0, index) / BLOCKS_PER_PSEUDO_PAGE) + 1;
}
