use super::table::{PDF_TABLE_END_MARKER, PDF_TABLE_IMAGE_MISSING_MARKER, PDF_TABLE_START_MARKER};
use crate::ocr::{OcrLayoutBlock, OcrPageLayout};
use crate::pipeline::chunking::{
    looks_like_reference_chunk, starts_reference_section, starts_with_non_reference_tail_heading,
};
use image::GenericImageView;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

const FIGURE_CROP_MARGIN: i32 = 24;
const SORT_ORDER_SCALE: i32 = 1000;
const UNORDERED_INSERT_OFFSET: i32 = 500;
const CAPTION_ROW_TOP_TOLERANCE: i32 = 24;
const CAPTION_ROW_GAP_TOLERANCE: i32 = 96;
const CAPTION_ROW_HEIGHT_TOLERANCE: i32 = 80;

#[derive(Debug, Clone)]
pub(crate) struct JsonMarkdownRebuild {
    pub(crate) markdown: String,
    pub(crate) report: JsonMarkdownRebuildReport,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JsonMarkdownRebuildReport {
    pub(crate) rendered_blocks: usize,
    pub(crate) figure_blocks: usize,
    pub(crate) table_blocks: usize,
    pub(crate) skipped_media_blocks: usize,
    pub(crate) reference_blocks_backfilled: usize,
    pub(crate) used_layout_renderer: bool,
}

pub(super) fn rebuild_pdf_markdown_from_layout_json(
    source_markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> JsonMarkdownRebuild {
    if page_layouts.is_empty() {
        return JsonMarkdownRebuild {
            markdown: source_markdown.to_string(),
            report: JsonMarkdownRebuildReport::default(),
        };
    }

    let mut report = JsonMarkdownRebuildReport {
        used_layout_renderer: true,
        ..Default::default()
    };
    let mut output = Vec::new();
    let mut figure_index = 1usize;

    for (page_index, page) in page_layouts.iter().enumerate() {
        let page_plan = build_page_plan(page, page_index, &mut figure_index, artifact_root);
        report.figure_blocks += page_plan.figure_blocks;
        report.table_blocks += page_plan.table_blocks;
        report.skipped_media_blocks += page_plan.skipped_media_blocks;
        render_page(page, &page_plan, &mut output, &mut report);
    }

    let rendered = output.join("\n\n");
    let (markdown, reference_blocks_backfilled) =
        backfill_references_from_source_markdown(&rendered, source_markdown, page_layouts);
    report.reference_blocks_backfilled = reference_blocks_backfilled;

    JsonMarkdownRebuild { markdown, report }
}

struct PagePlan {
    replacements: HashMap<usize, String>,
    skipped: HashSet<usize>,
    figure_blocks: usize,
    table_blocks: usize,
    skipped_media_blocks: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptionKind {
    Figure,
    Table,
}

#[derive(Clone, Copy, Debug)]
struct BBox {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn build_page_plan(
    page: &OcrPageLayout,
    page_index: usize,
    figure_index: &mut usize,
    artifact_root: &Path,
) -> PagePlan {
    let mut plan = PagePlan {
        replacements: HashMap::new(),
        skipped: HashSet::new(),
        figure_blocks: 0,
        table_blocks: 0,
        skipped_media_blocks: 0,
    };
    let ordered = sorted_block_indexes(&page.blocks);

    for index in ordered {
        if plan.skipped.contains(&index) {
            continue;
        }
        let block = &page.blocks[index];
        if block.label.eq_ignore_ascii_case("table") {
            add_table_plan(page, index, &mut plan);
            continue;
        }
        if caption_kind(&block.content) == Some(CaptionKind::Figure) {
            add_figure_plan(
                page,
                page_index,
                index,
                figure_index,
                artifact_root,
                &mut plan,
            );
        }
    }
    plan
}

fn render_page(
    page: &OcrPageLayout,
    plan: &PagePlan,
    output: &mut Vec<String>,
    report: &mut JsonMarkdownRebuildReport,
) {
    for index in sorted_block_indexes(&page.blocks) {
        if let Some(replacement) = plan.replacements.get(&index) {
            output.push(replacement.clone());
            report.rendered_blocks += 1;
            continue;
        }
        if plan.skipped.contains(&index) {
            continue;
        }
        let block = &page.blocks[index];
        if let Some(rendered) = render_layout_block(block) {
            output.push(rendered);
            report.rendered_blocks += 1;
        }
    }
}

fn render_layout_block(block: &OcrLayoutBlock) -> Option<String> {
    let content = normalize_text(&block.content);
    if content.is_empty() || is_skipped_layout_label(&block.label) {
        return None;
    }
    if block.label.eq_ignore_ascii_case("doc_title") {
        return Some(format!("# {content}"));
    }
    if block.label.eq_ignore_ascii_case("paragraph_title") {
        return Some(format!("## {content}"));
    }
    if block.label.eq_ignore_ascii_case("figure_title") {
        return Some(content);
    }
    Some(content)
}

fn add_table_plan(page: &OcrPageLayout, table_index: usize, plan: &mut PagePlan) {
    let range = table_media_group_range(&page.blocks, table_index);
    let caption = table_caption_in_range(&page.blocks, range.clone());
    let start = caption.map_or(table_index, |caption_index| caption_index.min(table_index));
    let mut parts = Vec::new();
    if let Some(caption_index) = caption {
        let caption = normalize_text(&page.blocks[caption_index].content);
        if !caption.is_empty() {
            parts.push(caption);
        }
    }
    parts.push(format!(
        "{PDF_TABLE_START_MARKER}\n{PDF_TABLE_IMAGE_MISSING_MARKER}\n{PDF_TABLE_END_MARKER}"
    ));
    plan.replacements.insert(start, parts.join("\n\n"));
    for index in range {
        if is_counted_media_label(&page.blocks[index].label) {
            plan.skipped_media_blocks += 1;
        }
        plan.skipped.insert(index);
    }
    plan.skipped.insert(table_index);
    plan.table_blocks += 1;
}

fn add_figure_plan(
    page: &OcrPageLayout,
    page_index: usize,
    caption_index: usize,
    figure_index: &mut usize,
    artifact_root: &Path,
    plan: &mut PagePlan,
) {
    let range = figure_media_group_range(&page.blocks, caption_index);
    let mut group_indexes = range.clone().collect::<Vec<_>>();
    group_indexes.extend(figure_caption_text_continuations(
        &page.blocks,
        caption_index,
        &group_indexes,
    ));
    group_indexes.sort_unstable();
    group_indexes.dedup();

    let visual_blocks = range
        .clone()
        .filter_map(|index| visual_bbox_block(&page.blocks[index]))
        .collect::<Vec<_>>();
    if visual_blocks.is_empty() {
        return;
    }
    let caption = figure_caption_for_group(&page.blocks, range.clone(), caption_index);
    let image =
        materialize_figure_snapshot(&visual_blocks, *figure_index, page_index, artifact_root)
            .unwrap_or_else(|| {
                format!(
                    "<!-- musetranslate:pdf-figure:image-missing:{} -->",
                    *figure_index
                )
            });
    let replacement = format!("{}\n\n::: figure-caption\n{}\n:::", image, caption.trim());
    if let Some(start) = group_indexes.iter().min().copied() {
        plan.replacements.insert(start, replacement);
    }
    for index in group_indexes {
        if is_counted_media_label(&page.blocks[index].label) {
            plan.skipped_media_blocks += 1;
        }
        plan.skipped.insert(index);
    }
    plan.figure_blocks += 1;
    *figure_index += 1;
}

fn sorted_block_indexes(blocks: &[OcrLayoutBlock]) -> Vec<usize> {
    let mut indexes = (0..blocks.len()).collect::<Vec<_>>();
    indexes.sort_by_key(|index| {
        let block = &blocks[*index];
        (
            block_sort_order(blocks, *index),
            block.bbox.get(1).copied().unwrap_or(i32::MAX),
            block.bbox.first().copied().unwrap_or(i32::MAX),
            *index,
        )
    });
    indexes
}

fn block_sort_order(blocks: &[OcrLayoutBlock], index: usize) -> i32 {
    let block = &blocks[index];
    if let Some(order) = block.order {
        return order.saturating_mul(SORT_ORDER_SCALE);
    }
    inferred_unordered_sort_order(blocks, block).unwrap_or(i32::MAX / 2)
}

fn inferred_unordered_sort_order(
    blocks: &[OcrLayoutBlock],
    target: &OcrLayoutBlock,
) -> Option<i32> {
    let target_box = bbox_from_block(target)?;
    if let Some(previous) = previous_order_before(blocks, target_box.top) {
        return Some(
            previous
                .saturating_mul(SORT_ORDER_SCALE)
                .saturating_add(UNORDERED_INSERT_OFFSET),
        );
    }
    next_order_after(blocks, target_box.bottom).map(|next| {
        next.saturating_mul(SORT_ORDER_SCALE)
            .saturating_sub(UNORDERED_INSERT_OFFSET)
    })
}

fn previous_order_before(blocks: &[OcrLayoutBlock], top: i32) -> Option<i32> {
    blocks
        .iter()
        .filter_map(|block| Some((block.order?, bbox_from_block(block)?)))
        .filter(|(_, bbox)| bbox.bottom <= top)
        .map(|(order, _)| order)
        .max()
}

fn next_order_after(blocks: &[OcrLayoutBlock], bottom: i32) -> Option<i32> {
    blocks
        .iter()
        .filter_map(|block| Some((block.order?, bbox_from_block(block)?)))
        .filter(|(_, bbox)| bbox.top >= bottom)
        .map(|(order, _)| order)
        .min()
}

fn table_media_group_range(blocks: &[OcrLayoutBlock], anchor: usize) -> std::ops::Range<usize> {
    let mut start = anchor;
    while start > 0 && is_table_group_prefix(blocks, start - 1) {
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < blocks.len() && is_table_group_suffix(blocks, end) {
        end += 1;
    }
    start..end
}

fn figure_media_group_range(blocks: &[OcrLayoutBlock], anchor: usize) -> std::ops::Range<usize> {
    let mut start = anchor;
    while start > 0 && is_figure_group_prefix(blocks, start - 1) {
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < blocks.len() && is_figure_group_suffix(blocks, end) {
        end += 1;
    }
    start..end
}

fn is_table_group_prefix(blocks: &[OcrLayoutBlock], index: usize) -> bool {
    let block = &blocks[index];
    if block.label.eq_ignore_ascii_case("figure_title") {
        return caption_kind(&block.content) != Some(CaptionKind::Figure);
    }
    matches!(
        block.label.to_ascii_lowercase().as_str(),
        "vision_footnote" | "header" | "footer" | "number"
    )
}

fn is_table_group_suffix(blocks: &[OcrLayoutBlock], index: usize) -> bool {
    let block = &blocks[index];
    if block.label.eq_ignore_ascii_case("figure_title") {
        return caption_kind(&block.content) != Some(CaptionKind::Figure);
    }
    matches!(
        block.label.to_ascii_lowercase().as_str(),
        "table" | "vision_footnote" | "header" | "footer" | "number"
    )
}

fn is_figure_group_prefix(blocks: &[OcrLayoutBlock], index: usize) -> bool {
    let block = &blocks[index];
    if block.label.eq_ignore_ascii_case("figure_title") {
        return caption_kind(&block.content).is_none();
    }
    matches!(
        block.label.to_ascii_lowercase().as_str(),
        "chart" | "image" | "header_image" | "header" | "footer" | "number"
    )
}

fn is_figure_group_suffix(blocks: &[OcrLayoutBlock], index: usize) -> bool {
    let block = &blocks[index];
    if block.label.eq_ignore_ascii_case("figure_title") {
        return caption_kind(&block.content).is_none();
    }
    matches!(
        block.label.to_ascii_lowercase().as_str(),
        "chart" | "image" | "header_image" | "vision_footnote" | "header" | "footer" | "number"
    )
}

fn table_caption_in_range(
    blocks: &[OcrLayoutBlock],
    range: std::ops::Range<usize>,
) -> Option<usize> {
    range.into_iter().find(|index| {
        blocks[*index].label.eq_ignore_ascii_case("figure_title")
            && caption_kind(&blocks[*index].content) == Some(CaptionKind::Table)
    })
}

fn figure_caption_in_range(
    blocks: &[OcrLayoutBlock],
    range: std::ops::Range<usize>,
    caption_index: usize,
) -> String {
    range
        .filter(|index| *index >= caption_index)
        .take_while(|index| {
            *index == caption_index
                || caption_kind(&blocks[*index].content).is_none()
                || blocks[*index].label.eq_ignore_ascii_case("figure_title")
        })
        .filter(|index| blocks[*index].label.eq_ignore_ascii_case("figure_title"))
        .map(|index| normalize_text(&blocks[index].content))
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn figure_caption_for_group(
    blocks: &[OcrLayoutBlock],
    range: std::ops::Range<usize>,
    caption_index: usize,
) -> String {
    let mut parts = Vec::new();
    let caption = figure_caption_in_range(blocks, range.clone(), caption_index);
    if !caption.is_empty() {
        parts.push(caption);
    }
    let group_indexes = range.collect::<Vec<_>>();
    for index in figure_caption_text_continuations(blocks, caption_index, &group_indexes) {
        let content = normalize_text(&blocks[index].content);
        if !content.is_empty() {
            parts.push(content);
        }
    }
    parts.join(" ")
}

fn figure_caption_text_continuations(
    blocks: &[OcrLayoutBlock],
    caption_index: usize,
    group_indexes: &[usize],
) -> Vec<usize> {
    let caption = &blocks[caption_index];
    let Some(caption_box) = bbox_from_block(caption) else {
        return Vec::new();
    };
    blocks
        .iter()
        .enumerate()
        .filter(|(index, _)| !group_indexes.contains(index))
        .filter(|(_, block)| block.label.eq_ignore_ascii_case("text"))
        .filter(|(_, block)| !normalize_text(&block.content).is_empty())
        .filter(|(_, block)| same_page(caption, block))
        .filter_map(|(index, block)| Some((index, bbox_from_block(block)?)))
        .filter(|(_, bbox)| is_same_row_caption_continuation(caption_box, *bbox))
        .map(|(index, _)| index)
        .collect()
}

fn is_same_row_caption_continuation(caption: BBox, candidate: BBox) -> bool {
    if candidate.left < caption.right {
        return false;
    }
    let horizontal_gap = candidate.left - caption.right;
    let top_delta = (candidate.top - caption.top).abs();
    let height_limit = caption
        .height()
        .saturating_add(CAPTION_ROW_HEIGHT_TOLERANCE);
    top_delta <= CAPTION_ROW_TOP_TOLERANCE
        && horizontal_gap <= CAPTION_ROW_GAP_TOLERANCE
        && candidate.height() <= height_limit
        && vertical_overlap(caption, candidate) * 2 >= candidate.height().min(caption.height())
}

fn vertical_overlap(left: BBox, right: BBox) -> i32 {
    left.bottom.min(right.bottom) - left.top.max(right.top)
}

fn same_page(left: &OcrLayoutBlock, right: &OcrLayoutBlock) -> bool {
    match (left.page, right.page) {
        (Some(left_page), Some(right_page)) => left_page == right_page,
        _ => true,
    }
}

fn caption_kind(content: &str) -> Option<CaptionKind> {
    let content = content.trim();
    let lower = content.to_ascii_lowercase();
    if (lower.starts_with("fig. ") || lower.starts_with("figure "))
        && lower.chars().any(|ch| ch.is_ascii_digit())
    {
        return Some(CaptionKind::Figure);
    }
    if lower.starts_with("table ") && lower.chars().any(|ch| ch.is_ascii_digit()) {
        return Some(CaptionKind::Table);
    }
    None
}

fn is_counted_media_label(label: &str) -> bool {
    matches!(
        label.to_ascii_lowercase().as_str(),
        "chart" | "image" | "header_image" | "figure_title" | "vision_footnote" | "table"
    )
}

fn is_skipped_layout_label(label: &str) -> bool {
    matches!(
        label.to_ascii_lowercase().as_str(),
        "header"
            | "footer"
            | "number"
            | "chart"
            | "image"
            | "header_image"
            | "table"
            | "vision_footnote"
    )
}

fn visual_bbox_block(block: &OcrLayoutBlock) -> Option<BBox> {
    let is_visual = matches!(
        block.label.to_ascii_lowercase().as_str(),
        "chart" | "image" | "header_image"
    );
    if !is_visual {
        return None;
    }
    bbox_from_block(block)
}

fn bbox_from_block(block: &OcrLayoutBlock) -> Option<BBox> {
    let bbox = BBox {
        left: *block.bbox.first()?,
        top: *block.bbox.get(1)?,
        right: *block.bbox.get(2)?,
        bottom: *block.bbox.get(3)?,
    };
    (bbox.right > bbox.left && bbox.bottom > bbox.top).then_some(bbox)
}

fn materialize_figure_snapshot(
    boxes: &[BBox],
    figure_index: usize,
    page_index: usize,
    artifact_root: &Path,
) -> Option<String> {
    let source_path = artifact_root
        .join("ocr_input")
        .join(format!("page_{page_index}.jpg"));
    let output_relative = format!("imgs/figure_snapshot_{figure_index:03}.jpg");
    let output_path = artifact_root.join(sanitize_relative_path(&output_relative));
    if output_path.exists() {
        return Some(markdown_figure_image(figure_index, &output_relative));
    }

    let image = image::open(&source_path).ok()?;
    let (width, height) = image.dimensions();
    let crop = union_bbox(boxes)?.expand(FIGURE_CROP_MARGIN, width as i32, height as i32)?;
    let cropped = image.crop_imm(
        crop.left as u32,
        crop.top as u32,
        crop.width() as u32,
        crop.height() as u32,
    );
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    cropped.save(&output_path).ok()?;
    Some(markdown_figure_image(figure_index, &output_relative))
}

fn markdown_figure_image(figure_index: usize, path: &str) -> String {
    format!("![PDF figure {figure_index}]({path})")
}

fn union_bbox(boxes: &[BBox]) -> Option<BBox> {
    let first = *boxes.first()?;
    Some(boxes.iter().copied().fold(first, |acc, item| BBox {
        left: acc.left.min(item.left),
        top: acc.top.min(item.top),
        right: acc.right.max(item.right),
        bottom: acc.bottom.max(item.bottom),
    }))
}

impl BBox {
    fn expand(self, margin: i32, image_width: i32, image_height: i32) -> Option<Self> {
        let left = (self.left - margin).max(0);
        let top = (self.top - margin).max(0);
        let right = (self.right + margin).min(image_width);
        let bottom = (self.bottom + margin).min(image_height);
        (right > left && bottom > top).then_some(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }
}

fn backfill_references_from_source_markdown(
    rendered: &str,
    source_markdown: &str,
    page_layouts: &[OcrPageLayout],
) -> (String, usize) {
    let source_reference_blocks = extract_numbered_reference_blocks(source_markdown);
    if source_reference_blocks.len() >= 3 {
        return replace_rendered_reference_section(rendered, &source_reference_blocks);
    }

    let source_reference_blocks =
        extract_unnumbered_reference_section_blocks(source_markdown, page_layouts);
    if source_reference_blocks.len() < 3 {
        return (rendered.to_string(), 0);
    }
    replace_rendered_reference_section(rendered, &source_reference_blocks)
}

fn replace_rendered_reference_section(
    rendered: &str,
    source_reference_blocks: &[String],
) -> (String, usize) {
    let mut blocks = split_blocks(rendered);
    let Some(start) = blocks
        .iter()
        .position(|block| normalize_heading_text(block).eq_ignore_ascii_case("references"))
    else {
        let replacement = format!("## References\n\n{}", source_reference_blocks.join("\n\n"));
        blocks.push(replacement);
        return (blocks.join("\n\n"), source_reference_blocks.len());
    };
    let end = next_heading_index(&blocks, start + 1).unwrap_or(blocks.len());
    let merged_references =
        merge_reference_blocks(&blocks[start + 1..end], source_reference_blocks);
    let replacement = format!("## References\n\n{}", merged_references.join("\n\n"));
    blocks.splice(start..end, [replacement]);
    (blocks.join("\n\n"), merged_references.len())
}

fn extract_numbered_reference_blocks(markdown: &str) -> Vec<String> {
    let blocks = split_blocks(markdown);
    numbered_reference_runs(&blocks)
        .into_iter()
        .filter(|run| is_supported_reference_run(run))
        .max_by_key(Vec::len)
        .map(|run| run.into_iter().map(|entry| entry.block).collect::<Vec<_>>())
        .unwrap_or_default()
}

fn extract_unnumbered_reference_section_blocks(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
) -> Vec<String> {
    let blocks = split_blocks(markdown);
    let Some(start) = blocks
        .iter()
        .position(|block| starts_reference_section(block))
    else {
        return Vec::new();
    };
    let page_chrome = layout_page_chrome_blocks(page_layouts);
    blocks
        .into_iter()
        .skip(start + 1)
        .take_while(|block| !starts_with_non_reference_tail_heading(block))
        .filter(|block| !is_layout_page_chrome_block(block, &page_chrome))
        .collect()
}

#[derive(Debug, Clone)]
struct NumberedReferenceBlock {
    number: usize,
    block: String,
}

fn numbered_reference_runs(blocks: &[String]) -> Vec<Vec<NumberedReferenceBlock>> {
    let mut runs = Vec::new();
    let mut current = Vec::new();
    let mut previous_number = None;
    for block in blocks {
        let Some(number) = reference_number(block) else {
            continue;
        };

        let continues = match previous_number {
            Some(previous) => number == previous + 1,
            None => true,
        };
        if continues {
            current.push(NumberedReferenceBlock {
                number,
                block: block.clone(),
            });
            previous_number = Some(number);
            continue;
        }

        push_reference_run(&mut runs, &mut current);
        current.push(NumberedReferenceBlock {
            number,
            block: block.clone(),
        });
        previous_number = Some(number);
    }
    push_reference_run(&mut runs, &mut current);
    runs
}

fn push_reference_run(
    runs: &mut Vec<Vec<NumberedReferenceBlock>>,
    current: &mut Vec<NumberedReferenceBlock>,
) {
    if current.is_empty() {
        return;
    }
    runs.push(std::mem::take(current));
}

fn is_supported_reference_run(run: &[NumberedReferenceBlock]) -> bool {
    if run.len() < 3 || !has_strictly_continuous_numbers(run) {
        return false;
    }
    let markdown = run
        .iter()
        .map(|entry| entry.block.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    looks_like_reference_chunk(&markdown)
}

fn has_strictly_continuous_numbers(run: &[NumberedReferenceBlock]) -> bool {
    let mut previous = None;
    for entry in run {
        let continues = match previous {
            Some(number) => entry.number == number + 1,
            None => true,
        };
        if !continues {
            return false;
        }
        previous = Some(entry.number);
    }
    true
}

fn merge_reference_blocks(rendered: &[String], source: &[String]) -> Vec<String> {
    if source.iter().any(|block| reference_number(block).is_none()) {
        return source.to_vec();
    }

    let mut by_number = std::collections::BTreeMap::<usize, String>::new();
    for block in rendered.iter().chain(source.iter()) {
        if let Some(number) = reference_number(block) {
            by_number.insert(number, block.clone());
        }
    }
    by_number.into_values().collect()
}

fn layout_page_chrome_blocks(page_layouts: &[OcrPageLayout]) -> HashSet<String> {
    let mut chrome = HashSet::new();
    for page in page_layouts {
        let headers = page
            .blocks
            .iter()
            .filter(|block| block.label.eq_ignore_ascii_case("header"))
            .map(|block| normalize_text(&block.content))
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>();
        let numbers = page
            .blocks
            .iter()
            .filter(|block| block.label.eq_ignore_ascii_case("number"))
            .map(|block| normalize_text(&block.content))
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>();

        for block in &page.blocks {
            if is_page_chrome_label(&block.label) {
                let content = normalize_text(&block.content);
                if !content.is_empty() {
                    chrome.insert(content);
                }
            }
        }

        for header in &headers {
            for number in &numbers {
                chrome.insert(format!("{header} {number}"));
                chrome.insert(format!("{number} {header}"));
            }
        }
    }
    chrome
}

fn is_page_chrome_label(label: &str) -> bool {
    matches!(
        label.to_ascii_lowercase().as_str(),
        "header" | "footer" | "number"
    )
}

fn is_layout_page_chrome_block(block: &str, page_chrome: &HashSet<String>) -> bool {
    page_chrome.contains(&normalize_text(block))
}

fn reference_number(block: &str) -> Option<usize> {
    let trimmed = block.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        return bracketed_reference_number(rest);
    }

    let digit_end = ascii_digit_prefix_end(trimmed)?;
    if digit_end > 4 {
        return None;
    }
    let rest = trimmed.get(digit_end..)?;
    if rest.starts_with(". ") || rest.starts_with(") ") {
        let content = rest.get(2..)?.trim();
        if !content.is_empty() {
            return trimmed.get(..digit_end)?.parse::<usize>().ok();
        }
    }
    None
}

fn bracketed_reference_number(rest: &str) -> Option<usize> {
    let digit_end = ascii_digit_prefix_end(rest)?;
    if digit_end > 4 {
        return None;
    }
    let after_digits = rest.get(digit_end..)?;
    if !after_digits.starts_with("] ") {
        return None;
    }
    let content = after_digits.get(2..)?.trim();
    if content.is_empty() {
        return None;
    }
    rest.get(..digit_end)?.parse::<usize>().ok()
}

fn ascii_digit_prefix_end(text: &str) -> Option<usize> {
    let mut end = 0usize;
    for (index, ch) in text.char_indices() {
        if !ch.is_ascii_digit() {
            break;
        }
        end = index + ch.len_utf8();
    }
    (end > 0).then_some(end)
}

fn next_heading_index(blocks: &[String], start: usize) -> Option<usize> {
    blocks
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, block)| block.trim_start().starts_with("## ").then_some(index))
}

fn normalize_heading_text(block: &str) -> String {
    block
        .trim()
        .trim_start_matches('#')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn sanitize_relative_path(raw: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for component in Path::new(raw).components() {
        if let Component::Normal(value) = component {
            path.push(value);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(label: &str, content: &str, bbox: Vec<i32>, order: i32) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: label.to_string(),
            content: content.to_string(),
            bbox,
            order: (order > 0).then_some(order),
            page: Some(0),
        }
    }

    fn temp_artifact_root(prefix: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "{}-{}",
            prefix,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("ocr_input")).expect("create ocr input");
        image::RgbImage::from_pixel(1200, 1600, image::Rgb([255, 255, 255]))
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page image");
        root
    }

    #[test]
    fn preserves_figure_title_as_caption_but_skips_chart_text_flow() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-json-figure-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("ocr_input")).expect("create ocr input");
        image::RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255]))
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page image");
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block("text", "Before figure.", vec![0, 0, 100, 20], 1),
                block("chart", "", vec![10, 30, 180, 150], 2),
                block(
                    "figure_title",
                    "Fig. 1 | Calibration analysis.",
                    vec![10, 152, 180, 170],
                    3,
                ),
                block("text", "After figure.", vec![0, 180, 100, 198], 4),
            ],
        }];

        let rebuilt = rebuild_pdf_markdown_from_layout_json("1. Ref.\n\n2. Ref.", &layouts, &root);

        assert!(rebuilt.markdown.contains("Before figure."));
        assert!(rebuilt
            .markdown
            .contains("![PDF figure 1](imgs/figure_snapshot_001.jpg)"));
        assert!(rebuilt
            .markdown
            .contains("::: figure-caption\nFig. 1 | Calibration analysis.\n:::"));
        assert!(rebuilt.markdown.contains("After figure."));
        assert_eq!(rebuilt.report.figure_blocks, 1);
        assert!(root.join("imgs/figure_snapshot_001.jpg").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn joins_same_row_text_continuation_into_figure_caption() {
        let root = temp_artifact_root("musetranslate-json-caption-row");
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "text",
                    "To select our stimuli, we set up a pre-test.",
                    vec![606, 972, 1126, 1061],
                    10,
                ),
                block("image", "", vec![178, 1087, 1021, 1329], 0),
                block(
                    "figure_title",
                    "Fig. 1 | Description of the task. Given their",
                    vec![74, 1342, 591, 1485],
                    0,
                ),
                block(
                    "text",
                    "choice, they had to indicate how much they were willing to pay.",
                    vec![608, 1342, 1126, 1465],
                    11,
                ),
                block(
                    "text",
                    "The most difficult stimulus to evaluate had a 6.92% success rate.",
                    vec![75, 1500, 1126, 1540],
                    12,
                ),
            ],
        }];

        let rebuilt = rebuild_pdf_markdown_from_layout_json("", &layouts, &root);

        assert!(rebuilt
            .markdown
            .contains("Description of the task. Given their choice, they had to indicate"));
        assert!(!rebuilt
            .markdown
            .contains("\n\nchoice, they had to indicate how much they were willing to pay."));
        assert!(
            rebuilt.markdown.find("pre-test.").unwrap_or_default()
                < rebuilt.markdown.find("![PDF figure 1]").unwrap_or_default()
        );
        assert!(
            rebuilt.markdown.find("![PDF figure 1]").unwrap_or_default()
                < rebuilt
                    .markdown
                    .find("The most difficult stimulus")
                    .unwrap_or_default()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unordered_figure_group_renders_before_following_ordered_text() {
        let root = temp_artifact_root("musetranslate-json-unordered-figure");
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block("image", "", vec![89, 636, 579, 890], 0),
                block(
                    "figure_title",
                    "Fig. 5 | Mediation effect of confidence.",
                    vec![73, 907, 588, 1065],
                    0,
                ),
                block(
                    "text",
                    "more to receive it than to avoid it.",
                    vec![75, 1102, 592, 1144],
                    1,
                ),
                block(
                    "paragraph_title",
                    "Discussion",
                    vec![609, 1378, 721, 1400],
                    7,
                ),
            ],
        }];

        let rebuilt = rebuild_pdf_markdown_from_layout_json("", &layouts, &root);

        assert!(rebuilt
            .markdown
            .contains("![PDF figure 1](imgs/figure_snapshot_001.jpg)"));
        assert!(
            rebuilt.markdown.find("![PDF figure 1]").unwrap_or_default()
                < rebuilt
                    .markdown
                    .find("more to receive it")
                    .unwrap_or_default()
        );
        assert!(
            rebuilt.markdown.find("![PDF figure 1]").unwrap_or_default()
                < rebuilt.markdown.find("## Discussion").unwrap_or_default()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn table_group_does_not_consume_following_figure_on_same_page() {
        let root = temp_artifact_root("musetranslate-json-table-then-figure");
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "figure_title",
                    "Table 2 | The moderated mediation model",
                    vec![75, 94, 1123, 138],
                    0,
                ),
                block(
                    "table",
                    "<table><tr><td>Outcome</td><td>Predictor</td></tr></table>",
                    vec![77, 150, 1124, 486],
                    0,
                ),
                block(
                    "vision_footnote",
                    "The table displays a moderated mediation model.",
                    vec![76, 491, 1124, 542],
                    0,
                ),
                block("image", "", vec![89, 636, 579, 890], 0),
                block(
                    "figure_title",
                    "Fig. 5 | Mediation effect of confidence.",
                    vec![73, 907, 588, 1065],
                    0,
                ),
                block(
                    "text",
                    "A moderated mediation analysis further extended the role of confidence.",
                    vec![606, 954, 1128, 1361],
                    6,
                ),
                block(
                    "paragraph_title",
                    "Discussion",
                    vec![609, 1378, 721, 1400],
                    7,
                ),
            ],
        }];

        let rebuilt = rebuild_pdf_markdown_from_layout_json("", &layouts, &root);

        assert!(rebuilt
            .markdown
            .contains("Table 2 | The moderated mediation model"));
        assert!(rebuilt.markdown.contains(PDF_TABLE_IMAGE_MISSING_MARKER));
        assert!(rebuilt
            .markdown
            .contains("![PDF figure 1](imgs/figure_snapshot_001.jpg)"));
        assert!(rebuilt
            .markdown
            .contains("::: figure-caption\nFig. 5 | Mediation effect of confidence.\n:::"));
        assert!(
            rebuilt
                .markdown
                .find(PDF_TABLE_START_MARKER)
                .unwrap_or_default()
                < rebuilt.markdown.find("![PDF figure 1]").unwrap_or_default()
        );
        assert!(
            rebuilt.markdown.find("![PDF figure 1]").unwrap_or_default()
                < rebuilt
                    .markdown
                    .find("A moderated mediation analysis")
                    .unwrap_or_default()
        );
        assert_eq!(rebuilt.report.table_blocks, 1);
        assert_eq!(rebuilt.report.figure_blocks, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn backfills_numbered_references_from_source_markdown() {
        let rendered = "## References\n\n1. Partial ref.\n\n## Acknowledgements\n\nThanks.";
        let source = concat!(
            "Body\n\n",
            "1. First, A. Journal of Tests 1, 1-2 (2020).\n\n",
            "2. Second, B. Journal of Tests 2, 3-4 (2021).\n\n",
            "3. Third, C. Journal of Tests 3, 5-6 (2022).\n\n",
            "## Author contributions"
        );

        let (output, count) = backfill_references_from_source_markdown(rendered, source, &[]);

        assert_eq!(count, 3);
        assert!(output.contains("1. First, A."));
        assert!(!output.contains("1. Partial ref."));
        assert!(output.contains("## Acknowledgements"));
    }

    #[test]
    fn backfills_reference_run_that_does_not_start_at_one() {
        let rendered = "## References\n\n1. First ref.\n\n19. Partial nineteenth.";
        let source = concat!(
            "Body\n\n",
            "19. Nineteenth, A. Journal of Tests 19, 19-20 (2019).\n\n",
            "20. Twentieth, B. Journal of Tests 20, 20-21 (2020).\n\n",
            "21. Twenty First, C. Journal of Tests 21, 21-22 (2021)."
        );

        let (output, count) = backfill_references_from_source_markdown(rendered, source, &[]);

        assert_eq!(count, 4);
        assert!(output.contains("1. First ref."));
        assert!(output.contains("19. Nineteenth, A."));
        assert!(output.contains("21. Twenty First, C."));
    }

    #[test]
    fn backfills_reference_run_across_page_chrome_gap() {
        let rendered = concat!(
            "## References\n\n",
            "65. Partial sixty fifth.\n\n",
            "66. Partial sixty sixth.\n\n",
            "67. Partial sixty seventh."
        );
        let source = concat!(
            "Body\n\n",
            "65. Ferreira, F. Current Directions in Psychological Science 11, 11-15 (2002).\n\n",
            "66. Tomasello, M. Episteme 17, 301-315 (2020).\n\n",
            "67. Stengelen, R. Cognitive Development 48, 146-154 (2018).\n\n",
            "Communications Psychology | (2024)2:122\n\n",
            "11\n\n",
            "https://doi.org/10.1038/example\n\n",
            "Article\n\n",
            "68. Capraro, V. Journal of Behavioral Experimental Economics 79, 93-99 (2019).\n\n",
            "69. Marsh, E. Psychological Learning and Motivation 64, 93-132 (2016).\n\n",
            "70. Pennycook, G. Collabra Psychology 7, 1-13 (2021)."
        );

        let (output, count) = backfill_references_from_source_markdown(rendered, source, &[]);

        assert_eq!(count, 6);
        assert!(output.contains("65. Ferreira, F."));
        assert!(output.contains("68. Capraro, V."));
        assert!(output.contains("70. Pennycook, G."));
        assert!(!output.contains("Communications Psychology | (2024)2:122"));
    }

    #[test]
    fn backfills_unnumbered_references_from_source_section() {
        let rendered = concat!(
            "## References\n\n",
            "Atkins, S. and K. Murphy (1993), partial.\n\n",
            "Lurquin, J. H. and A. Miyake (2017), partial.\n\n",
            "Yeung, K. (2012), partial.\n\n",
            "## Acknowledgements\n\n",
            "Thanks."
        );
        let source = concat!(
            "Body\n\n",
            "References\n\n",
            "Atkins, S. and K. Murphy (1993), 'Reflection: A review of the literature', Journal of Advanced Nursing, 18(8): 1188-92.\n\n",
            "Baldwin, R. (2014), 'From regulation to behaviour change: Giving nudge the third degree', The Modern Law Review, 77(6): 831-57.\n\n",
            "Behavioural Public Policy 83\n\n",
            "Lurquin, J. H. and A. Miyake (2017), 'Challenges to ego-depletion research go beyond the replication crisis', Frontiers in Psychology, 8: 568.\n\n",
            "84 Sanchayan Banerjee and Peter John\n\n",
            "Yeung, K. (2012), 'Nudge as fudge', The Modern Law Review, 75(1): 122-148.\n\n",
            "Cite this article: Banerjee S, John P (2024). Nudge plus: incorporating reflection into behavioral public policy.\n\n",
            "https://doi.org/10.1017/bpp.2021.6 Published online by Cambridge University Press"
        );
        let layouts = vec![
            OcrPageLayout {
                blocks: vec![
                    block("header", "Behavioural Public Policy", vec![0, 0, 100, 20], 0),
                    block("number", "83", vec![110, 0, 130, 20], 0),
                    block(
                        "footer",
                        "https://doi.org/10.1017/bpp.2021.6 Published online by Cambridge University Press",
                        vec![0, 1000, 700, 1020],
                        0,
                    ),
                ],
            },
            OcrPageLayout {
                blocks: vec![
                    block("number", "84", vec![0, 0, 20, 20], 0),
                    block(
                        "header",
                        "Sanchayan Banerjee and Peter John",
                        vec![30, 0, 300, 20],
                        0,
                    ),
                ],
            },
        ];

        let (output, count) = backfill_references_from_source_markdown(rendered, source, &layouts);

        assert_eq!(count, 5);
        assert!(output.contains("Baldwin, R. (2014)"));
        assert!(output.contains("Cite this article: Banerjee S"));
        assert!(output.contains("## Acknowledgements"));
        assert!(!output.contains("Behavioural Public Policy 83"));
        assert!(!output.contains("84 Sanchayan Banerjee"));
        assert!(!output.contains("Published online by Cambridge University Press"));
    }

    #[test]
    fn rebuilds_json_markdown_from_env_artifact_dir() {
        let Ok(artifact_dir) = std::env::var("MUSETRANSLATE_JSON_REBUILD_ARTIFACT_DIR") else {
            return;
        };
        let artifact_dir = std::path::PathBuf::from(artifact_dir);
        let source_path = artifact_dir.join("source_raw_ocr.md");
        let layouts_path = artifact_dir.join("source_layouts.json");
        if !source_path.exists() || !layouts_path.exists() {
            eprintln!(
                "skip JSON rebuild artifact test: missing {} or {}",
                source_path.display(),
                layouts_path.display()
            );
            return;
        }

        let source = std::fs::read_to_string(&source_path).expect("read source_raw_ocr.md");
        let layouts_json =
            std::fs::read_to_string(&layouts_path).expect("read source_layouts.json");
        let layouts: Vec<OcrPageLayout> =
            serde_json::from_str(&layouts_json).expect("parse source_layouts.json");
        let rebuilt = rebuild_pdf_markdown_from_layout_json(&source, &layouts, &artifact_dir);

        if let Ok(expected) = std::env::var("MUSETRANSLATE_JSON_REBUILD_EXPECT") {
            for token in expected.split('|').filter(|token| !token.is_empty()) {
                assert!(
                    rebuilt.markdown.contains(token),
                    "rebuilt markdown missing expected token: {token}"
                );
            }
        }
        if let Ok(rejected) = std::env::var("MUSETRANSLATE_JSON_REBUILD_REJECT") {
            for token in rejected.split('|').filter(|token| !token.is_empty()) {
                assert!(
                    !rebuilt.markdown.contains(token),
                    "rebuilt markdown contains rejected token: {token}"
                );
            }
        }
        println!(
            "json rebuild artifact check: chars={}; rendered_blocks={}; reference_blocks_backfilled={}",
            rebuilt.markdown.chars().count(),
            rebuilt.report.rendered_blocks,
            rebuilt.report.reference_blocks_backfilled
        );
    }
}
