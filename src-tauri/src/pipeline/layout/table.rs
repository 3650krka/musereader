use crate::ocr::{OcrLayoutBlock, OcrPageLayout};
use image::GenericImageView;
use std::path::{Component, Path, PathBuf};

pub(crate) const PDF_TABLE_START_MARKER: &str = "<!-- musetranslate:pdf-table:start -->";
pub(crate) const PDF_TABLE_END_MARKER: &str = "<!-- musetranslate:pdf-table:end -->";
pub(crate) const PDF_TABLE_IMAGE_MISSING_MARKER: &str =
    "<!-- musetranslate:pdf-table:image-missing -->";
const TABLE_CROP_MARGIN: i32 = 18;

pub(super) fn protect_pdf_table_blocks(markdown: &str, image_paths: &[String]) -> String {
    if markdown.contains(PDF_TABLE_START_MARKER) {
        return normalize_pdf_table_block_boundaries(markdown);
    }

    let mut output = String::with_capacity(markdown.len());
    let mut cursor = 0usize;
    let mut table_index = 0usize;

    while let Some(relative_start) = find_ascii_case_insensitive(&markdown[cursor..], "<table") {
        let table_start = cursor + relative_start;
        let Some(relative_end) = find_ascii_case_insensitive(&markdown[table_start..], "</table>")
        else {
            break;
        };
        let table_end = table_start + relative_end + "</table>".len();

        output.push_str(&markdown[cursor..table_start]);
        push_protected_table(&mut output, table_index, image_paths);
        table_index += 1;
        cursor = table_end;
    }

    output.push_str(&markdown[cursor..]);
    output
}

pub(crate) fn is_pdf_table_block(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.contains(PDF_TABLE_START_MARKER) && trimmed.contains(PDF_TABLE_END_MARKER)
}

pub(super) fn replace_missing_pdf_table_snapshots(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> String {
    if !markdown.contains(PDF_TABLE_IMAGE_MISSING_MARKER) {
        return markdown.to_string();
    }

    let candidates = table_snapshot_candidates(page_layouts);
    if candidates.is_empty() {
        return markdown.to_string();
    }

    let mut output = markdown.to_string();
    for (index, candidate) in candidates.iter().enumerate() {
        if !output.contains(PDF_TABLE_IMAGE_MISSING_MARKER) {
            break;
        }
        let Some(image) = materialize_table_snapshot(candidate, index + 1, artifact_root) else {
            continue;
        };
        output = output.replacen(PDF_TABLE_IMAGE_MISSING_MARKER, &image, 1);
    }
    output
}

pub(super) fn replace_layout_tables_with_snapshots(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> String {
    let table_index_start = count_protected_table_blocks(markdown) + 1;
    replace_layout_tables_with_snapshots_from_index(
        markdown,
        page_layouts,
        artifact_root,
        table_index_start,
    )
}

pub(super) fn replace_layout_tables_with_stable_snapshots(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> String {
    replace_layout_tables_with_snapshots_from_index(markdown, page_layouts, artifact_root, 1)
}

fn replace_layout_tables_with_snapshots_from_index(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
    table_index_start: usize,
) -> String {
    let candidates = table_snapshot_candidates(page_layouts)
        .into_iter()
        .filter(|candidate| !candidate.content.trim().is_empty())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return markdown.to_string();
    }

    let mut blocks = split_markdown_blocks(markdown);
    let mut table_index = table_index_start;
    let grouped_candidates = group_table_candidates(candidates);
    for group in grouped_candidates {
        if replace_table_candidate_group(&mut blocks, &group, &mut table_index, artifact_root) {
            continue;
        }
        for candidate in group {
            let snippets = table_text_snippets(&candidate.content);
            if snippets.is_empty() {
                continue;
            }
            let mut image: Option<String> = None;
            let mut replaced_count = 0usize;
            let row_anchors = table_row_anchors(&candidate.content);
            let windows = table_replacement_windows(&blocks, &candidate, &snippets, &row_anchors);
            let earliest_window_start = windows.iter().map(|(start, _)| *start).min();
            for (start, end) in windows.into_iter().rev() {
                let image_value = match image.as_ref() {
                    Some(value) => value.clone(),
                    None => {
                        let Some(value) =
                            materialize_table_snapshot(&candidate, table_index, artifact_root)
                        else {
                            break;
                        };
                        image = Some(value.clone());
                        value
                    }
                };
                blocks.splice(start..end, [protected_table_image_block(&image_value)]);
                replaced_count += 1;
                if replaced_count >= 12 {
                    break;
                }
            }
            if replaced_count > 0 {
                if let Some(window_start) =
                    earliest_window_start.filter(|_| !candidate.caption.trim().is_empty())
                {
                    let residual_start = (window_start + 1).min(blocks.len());
                    let residual_end = next_table_range_boundary(&blocks, residual_start)
                        .min(references_boundary(&blocks))
                        .min(residual_start + 12);
                    remove_remaining_table_text_windows_in_range(
                        &mut blocks,
                        &snippets,
                        residual_start,
                        residual_end,
                        8,
                    );
                    remove_remaining_table_row_anchor_windows_in_range(
                        &mut blocks,
                        &row_anchors,
                        residual_start,
                        residual_end,
                        4,
                    );
                } else {
                    remove_remaining_table_text_windows(&mut blocks, &snippets, 12);
                    remove_remaining_table_row_anchor_windows(&mut blocks, &row_anchors, 8);
                }
                table_index += 1;
            } else if !candidate.caption.trim().is_empty() {
                // If a captioned table could not be replaced, keep the source text intact
                // rather than letting later cleanup silently erase part of the table.
                continue;
            }
        }
    }
    blocks.join("\n\n")
}

fn replace_table_candidate_group(
    blocks: &mut Vec<String>,
    group: &[TableCandidate],
    table_index: &mut usize,
    artifact_root: &Path,
) -> bool {
    if group.len() <= 1 {
        return false;
    }
    let caption = group[0].caption.trim();
    if caption.is_empty() {
        return false;
    }
    let boundary = references_boundary(blocks);
    let Some((start, end)) = caption_anchored_search_ranges(blocks, caption, boundary)
        .into_iter()
        .next()
    else {
        return false;
    };
    let search_start = skip_existing_pdf_table_blocks(blocks, start, end);
    if search_start >= end {
        return false;
    }

    let mut replacement = Vec::with_capacity(group.len());
    for candidate in group {
        let Some(image) = materialize_table_snapshot(candidate, *table_index, artifact_root) else {
            return false;
        };
        replacement.push(protected_table_image_block(&image));
        *table_index += 1;
    }
    let inserted_len = replacement.len();
    blocks.splice(search_start..end, replacement);
    let residual_start = search_start + inserted_len;
    let residual_end =
        next_table_range_boundary(blocks, residual_start).min(references_boundary(blocks));
    for candidate in group {
        let snippets = table_text_snippets(&candidate.content);
        let row_anchors = table_row_anchors(&candidate.content);
        remove_remaining_table_text_windows_in_range(
            blocks,
            &snippets,
            residual_start,
            residual_end,
            8,
        );
        remove_remaining_table_row_anchor_windows_in_range(
            blocks,
            &row_anchors,
            residual_start,
            residual_end,
            6,
        );
    }
    clear_table_group_residual_blocks(blocks, residual_start);
    true
}

fn remove_remaining_table_text_windows(
    blocks: &mut Vec<String>,
    snippets: &[String],
    limit: usize,
) {
    let mut removed = 0usize;
    while let Some((start, end)) =
        find_table_text_window_bounded(blocks, 0, references_boundary(blocks), snippets)
    {
        blocks.splice(start..end, std::iter::empty::<String>());
        removed += 1;
        if removed >= limit {
            break;
        }
    }
}

fn remove_remaining_table_text_windows_in_range(
    blocks: &mut Vec<String>,
    snippets: &[String],
    start: usize,
    end: usize,
    limit: usize,
) {
    if snippets.is_empty() || start >= end {
        return;
    }
    let mut removed = 0usize;
    while let Some((window_start, window_end)) =
        find_table_text_window_bounded(blocks, start, end.min(blocks.len()), snippets)
    {
        blocks.splice(window_start..window_end, std::iter::empty::<String>());
        removed += 1;
        if removed >= limit {
            break;
        }
    }
}

fn remove_remaining_table_row_anchor_windows(
    blocks: &mut Vec<String>,
    row_anchors: &[String],
    limit: usize,
) {
    if row_anchors.is_empty() {
        return;
    }
    let mut removed = 0usize;
    while let Some((start, end)) =
        find_table_row_anchor_window_bounded(blocks, 0, references_boundary(blocks), row_anchors)
    {
        blocks.splice(start..end, std::iter::empty::<String>());
        removed += 1;
        if removed >= limit {
            break;
        }
    }
}

fn remove_remaining_table_row_anchor_windows_in_range(
    blocks: &mut Vec<String>,
    row_anchors: &[String],
    start: usize,
    end: usize,
    limit: usize,
) {
    if row_anchors.is_empty() || start >= end {
        return;
    }
    let mut removed = 0usize;
    while let Some((window_start, window_end)) =
        find_table_row_anchor_window_bounded(blocks, start, end.min(blocks.len()), row_anchors)
    {
        blocks.splice(window_start..window_end, std::iter::empty::<String>());
        removed += 1;
        if removed >= limit {
            break;
        }
    }
}

fn clear_table_group_residual_blocks(blocks: &mut Vec<String>, start: usize) {
    if start >= blocks.len() {
        return;
    }
    let end = next_table_range_boundary(blocks, start).min(references_boundary(blocks));
    if start >= end {
        return;
    }
    blocks.splice(start..end, std::iter::empty::<String>());
}

fn table_replacement_windows(
    blocks: &[String],
    candidate: &TableCandidate,
    snippets: &[String],
    row_anchors: &[String],
) -> Vec<(usize, usize)> {
    let reference_start = references_boundary(blocks);
    let ranges = caption_anchored_search_ranges(blocks, &candidate.caption, reference_start);
    if ranges.is_empty() {
        return find_table_text_window_bounded(blocks, 0, reference_start, snippets)
            .or_else(|| {
                find_table_row_anchor_window_bounded(blocks, 0, reference_start, row_anchors)
            })
            .into_iter()
            .collect();
    }

    let mut windows = Vec::new();
    for (start, end) in ranges {
        let search_start = skip_existing_pdf_table_blocks(blocks, start, end);
        if search_start >= end {
            continue;
        }
        if let Some(window) = find_table_text_window_bounded(blocks, search_start, end, snippets)
            .or_else(|| {
                find_table_row_anchor_window_bounded(blocks, search_start, end, row_anchors)
            })
        {
            windows.push(window);
        }
    }
    if windows.is_empty() {
        find_table_text_window_bounded(blocks, 0, reference_start, snippets)
            .or_else(|| {
                find_table_row_anchor_window_bounded(blocks, 0, reference_start, row_anchors)
            })
            .into_iter()
            .collect()
    } else {
        windows
    }
}

fn skip_existing_pdf_table_blocks(blocks: &[String], start: usize, end: usize) -> usize {
    let mut index = start;
    while index < end
        && blocks
            .get(index)
            .is_some_and(|block| is_pdf_table_block(block.trim()))
    {
        index += 1;
    }
    index
}

fn references_boundary(blocks: &[String]) -> usize {
    blocks
        .iter()
        .position(|block| {
            let normalized = normalize_table_text(block);
            normalized == "references" || normalized == "## references"
        })
        .unwrap_or(blocks.len())
}

fn caption_anchored_search_ranges(
    blocks: &[String],
    caption: &str,
    boundary: usize,
) -> Vec<(usize, usize)> {
    let caption_key = caption_match_key(caption);
    if caption_key.is_empty() {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    for (index, block) in blocks.iter().take(boundary).enumerate() {
        let block_key = caption_match_key(block);
        if !block_key.contains(&caption_key) && !caption_key.contains(&block_key) {
            continue;
        }
        let start = index + 1;
        let end = next_table_range_boundary(blocks, start)
            .min(start + 48)
            .min(boundary);
        if start < end {
            ranges.push((start, end));
        }
    }
    ranges
}

fn caption_match_key(caption: &str) -> String {
    if is_continued_table_caption(caption) {
        return "table continued".to_string();
    }
    normalize_table_text(caption)
        .split_whitespace()
        .take(16)
        .collect::<Vec<_>>()
        .join(" ")
}

fn next_table_range_boundary(blocks: &[String], start: usize) -> usize {
    for (offset, block) in blocks.iter().enumerate().skip(start) {
        let trimmed = block.trim_start();
        if is_hard_table_range_boundary(trimmed)
            || (offset > start && trimmed.starts_with("Table "))
        {
            return offset;
        }
    }
    blocks.len()
}

fn is_hard_table_range_boundary(trimmed: &str) -> bool {
    trimmed.starts_with('#')
        || trimmed.starts_with("Fig.")
        || trimmed.starts_with("Figure ")
        || trimmed.starts_with("![PDF figure")
        || trimmed.starts_with("::: figure-caption")
        || trimmed.starts_with("<!-- musetranslate:pdf-figure:")
        || is_pdf_table_block(trimmed)
}

pub(super) fn collapse_adjacent_pdf_table_blocks(markdown: &str) -> String {
    let mut output = Vec::new();
    let mut previous_table_key: Option<String> = None;
    for block in split_markdown_blocks(&normalize_pdf_table_block_boundaries(markdown)) {
        if is_duplicate_continued_table_caption(output.last(), &block) {
            continue;
        }
        let is_table = is_pdf_table_block(&block);
        let table_key = is_table.then(|| normalize_table_text(&block));
        if table_key.is_some() && table_key == previous_table_key {
            continue;
        }
        previous_table_key = table_key;
        output.push(block);
    }
    output.join("\n\n")
}

fn normalize_pdf_table_block_boundaries(markdown: &str) -> String {
    split_markdown_blocks(markdown)
        .into_iter()
        .flat_map(|block| isolate_pdf_table_blocks(&block))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn isolate_pdf_table_blocks(block: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut rest = block.trim();
    while let Some(start) = rest.find(PDF_TABLE_START_MARKER) {
        let before = rest[..start].trim();
        if !before.is_empty() {
            output.push(before.to_string());
        }
        let table_start = start + PDF_TABLE_START_MARKER.len();
        let Some(relative_end) = rest[table_start..].find(PDF_TABLE_END_MARKER) else {
            output.push(rest[start..].trim().to_string());
            return output;
        };
        let end = table_start + relative_end;
        let after_end = end + PDF_TABLE_END_MARKER.len();
        let inner = rest[table_start..end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let mut table_block = String::from(PDF_TABLE_START_MARKER);
        for line in inner {
            table_block.push('\n');
            table_block.push_str(line);
        }
        table_block.push('\n');
        table_block.push_str(PDF_TABLE_END_MARKER);
        output.push(table_block);
        rest = rest[after_end..].trim();
    }
    if !rest.is_empty() {
        output.push(rest.to_string());
    }
    output
}

fn is_duplicate_continued_table_caption(previous: Option<&String>, current: &str) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    is_continued_table_caption(previous)
        && is_continued_table_caption(current)
        && normalize_table_text(previous) == normalize_table_text(current)
}

fn is_continued_table_caption(block: &str) -> bool {
    let normalized = normalize_table_text(block).replace(['-', '–', '—'], " ");
    normalized.starts_with("table ") && normalized.contains(" continued from previous page")
}

fn push_protected_table(output: &mut String, table_index: usize, image_paths: &[String]) {
    ensure_blank_boundary(output);
    output.push_str(PDF_TABLE_START_MARKER);
    output.push('\n');
    if let Some(path) = image_paths.get(table_index) {
        output.push_str(&format!("![PDF table {}]({})", table_index + 1, path));
    } else {
        output.push_str(PDF_TABLE_IMAGE_MISSING_MARKER);
    }
    output.push('\n');
    output.push_str(PDF_TABLE_END_MARKER);
    output.push_str("\n\n");
}

fn ensure_blank_boundary(output: &mut String) {
    if output.is_empty() || output.ends_with("\n\n") {
        return;
    }
    if output.ends_with('\n') {
        output.push('\n');
    } else {
        output.push_str("\n\n");
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[derive(Clone, Debug)]
struct TableCandidate {
    page: usize,
    bbox: BBox,
    order: i32,
    content: String,
    caption: String,
}

#[derive(Clone, Copy, Debug)]
struct BBox {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn table_snapshot_candidates(page_layouts: &[OcrPageLayout]) -> Vec<TableCandidate> {
    let mut candidates = page_layouts
        .iter()
        .enumerate()
        .flat_map(|(page_index, layout)| table_blocks_on_page(page_index, &layout.blocks))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| (candidate.page, candidate.order, candidate.bbox.top));
    let merged = merge_overlapping_candidates(candidates);
    inherit_missing_table_captions(merged)
}

fn table_blocks_on_page(page_index: usize, blocks: &[OcrLayoutBlock]) -> Vec<TableCandidate> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| is_table_region_label(&block.label))
        .filter_map(|(block_index, block)| {
            let bbox = BBox::from_values(&block.bbox)?;
            let page = block
                .page
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(page_index);
            let caption = nearest_table_caption(blocks, block_index, &bbox);
            Some(TableCandidate {
                page,
                bbox,
                order: block.order.unwrap_or(i32::MAX),
                content: block.content.clone(),
                caption,
            })
        })
        .collect()
}

fn nearest_table_caption(
    blocks: &[OcrLayoutBlock],
    table_index: usize,
    table_bbox: &BBox,
) -> String {
    blocks
        .iter()
        .take(table_index)
        .rev()
        .find(|block| {
            is_table_caption_label(&block.label)
                && BBox::from_values(&block.bbox)
                    .is_none_or(|bbox| bbox.bottom <= table_bbox.top + 24)
                && !block.content.trim().is_empty()
        })
        .map(|block| block.content.trim().to_string())
        .unwrap_or_default()
}

fn is_table_caption_label(label: &str) -> bool {
    let lower = label.trim().to_ascii_lowercase();
    lower.contains("title") || lower.contains("caption")
}

fn is_table_region_label(label: &str) -> bool {
    let lower = label.trim().to_ascii_lowercase();
    if !(lower.contains("table") || lower.contains("tbl")) {
        return false;
    }
    !(lower.contains("caption") || lower.contains("title"))
}

fn count_protected_table_blocks(markdown: &str) -> usize {
    markdown.matches(PDF_TABLE_START_MARKER).count()
}

fn merge_overlapping_candidates(candidates: Vec<TableCandidate>) -> Vec<TableCandidate> {
    let mut merged: Vec<TableCandidate> = Vec::new();
    for candidate in candidates {
        if let Some(existing) = merged.iter_mut().find(|existing| {
            existing.page == candidate.page && existing.bbox.overlaps_or_touches(&candidate.bbox)
        }) {
            existing.bbox = existing.bbox.union(&candidate.bbox);
            existing.order = existing.order.min(candidate.order);
            if !candidate.content.trim().is_empty() {
                if !existing.content.is_empty() {
                    existing.content.push(' ');
                }
                existing.content.push_str(&candidate.content);
            }
            if existing.caption.trim().is_empty() && !candidate.caption.trim().is_empty() {
                existing.caption = candidate.caption;
            }
        } else {
            merged.push(candidate);
        }
    }
    merged
}

fn inherit_missing_table_captions(mut candidates: Vec<TableCandidate>) -> Vec<TableCandidate> {
    for index in 0..candidates.len() {
        if !candidates[index].caption.trim().is_empty() {
            continue;
        }
        let header_key = table_header_key(&candidates[index].content);
        if header_key.is_empty() {
            continue;
        }
        let inherited = candidates[..index].iter().rev().find(|previous| {
            !previous.caption.trim().is_empty()
                && previous.page <= candidates[index].page
                && candidates[index].page.saturating_sub(previous.page) <= 2
                && table_header_key(&previous.content) == header_key
        });
        if let Some(previous) = inherited {
            candidates[index].caption = previous.caption.clone();
        }
    }
    candidates
}

fn materialize_table_snapshot(
    candidate: &TableCandidate,
    table_index: usize,
    artifact_root: &Path,
) -> Option<String> {
    let output_relative = format!("imgs/table_snapshot_{table_index:03}.jpg");
    materialize_table_snapshot_at(candidate, &output_relative, artifact_root)
        .map(|path| markdown_table_image(table_index, &path))
}

fn materialize_table_snapshot_at(
    candidate: &TableCandidate,
    output_relative: &str,
    artifact_root: &Path,
) -> Option<String> {
    let source_path = artifact_root
        .join("ocr_input")
        .join(format!("page_{}.jpg", candidate.page));
    let output_path = artifact_root.join(sanitize_relative_path(&output_relative));
    if output_path.exists() {
        return Some(output_relative.to_string());
    }

    let image = image::open(&source_path).ok()?;
    let (width, height) = image.dimensions();
    let crop = candidate
        .bbox
        .expand(TABLE_CROP_MARGIN, width as i32, height as i32)?;
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
    Some(output_relative.to_string())
}

fn markdown_table_image(table_index: usize, path: &str) -> String {
    format!("![PDF table {table_index}]({path})")
}

fn protected_table_image_block(image: &str) -> String {
    format!("{PDF_TABLE_START_MARKER}\n{image}\n{PDF_TABLE_END_MARKER}")
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn table_text_snippets(html: &str) -> Vec<String> {
    let mut snippets = Vec::new();
    let mut current = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                push_table_snippet(&mut snippets, &mut current);
                in_tag = true;
            }
            '>' => {
                in_tag = false;
            }
            _ if !in_tag => current.push(ch),
            _ => {}
        }
    }
    push_table_snippet(&mut snippets, &mut current);
    snippets
}

fn table_row_anchors(html: &str) -> Vec<String> {
    extract_table_rows(html)
        .into_iter()
        .map(|row| normalize_table_text(&decode_html_entities(&strip_html_tags(&row))))
        .filter(|row| row.chars().count() >= 24)
        .filter(|row| row.split_whitespace().count() >= 4)
        .collect()
}

fn extract_table_rows(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut rows = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative_start) = lower[search_start..].find("<tr") {
        let tag_start = search_start + relative_start;
        let Some(open_end_relative) = lower[tag_start..].find('>') else {
            break;
        };
        let content_start = tag_start + open_end_relative + 1;
        let Some(close_relative) = lower[content_start..].find("</tr>") else {
            break;
        };
        let content_end = content_start + close_relative;
        rows.push(html[content_start..content_end].to_string());
        search_start = content_end + "</tr>".len();
    }
    rows
}

fn table_header_key(html: &str) -> String {
    extract_table_rows(html)
        .into_iter()
        .next()
        .map(|row| normalize_table_text(&decode_html_entities(&strip_html_tags(&row))))
        .unwrap_or_default()
}

fn group_table_candidates(candidates: Vec<TableCandidate>) -> Vec<Vec<TableCandidate>> {
    let mut groups: Vec<Vec<TableCandidate>> = Vec::new();
    for candidate in candidates {
        let header_key = table_header_key(&candidate.content);
        let can_append = groups
            .last()
            .and_then(|group| group.last())
            .is_some_and(|previous| {
                !candidate.caption.trim().is_empty()
                    && candidate.caption == previous.caption
                    && !header_key.is_empty()
                    && table_header_key(&previous.content) == header_key
                    && candidate.page >= previous.page
                    && candidate.page.saturating_sub(previous.page) <= 2
            });
        if can_append {
            groups
                .last_mut()
                .expect("group exists when append condition is true")
                .push(candidate);
        } else {
            groups.push(vec![candidate]);
        }
    }
    groups
}

fn strip_html_tags(value: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

fn push_table_snippet(snippets: &mut Vec<String>, current: &mut String) {
    let decoded = decode_html_entities(current);
    let normalized = normalize_table_text(&decoded);
    current.clear();
    if normalized.chars().count() >= 3 && !snippets.contains(&normalized) {
        snippets.push(normalized);
    }
}

fn decode_html_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn find_table_text_window_bounded(
    blocks: &[String],
    cursor: usize,
    range_end: usize,
    snippets: &[String],
) -> Option<(usize, usize)> {
    let required_score = snippets.len().min(4).max(2);
    find_table_text_window_bounded_with_score(blocks, cursor, range_end, snippets, required_score)
}

fn find_table_text_window_bounded_with_score(
    blocks: &[String],
    cursor: usize,
    range_end: usize,
    snippets: &[String],
    required_score: usize,
) -> Option<(usize, usize)> {
    let range_end = range_end.min(blocks.len());
    for start in cursor..range_end {
        let mut joined = String::new();
        let mut matched_start = None;
        for end in start..range_end.min(start + 12) {
            if is_hard_table_range_boundary(blocks[end].trim_start()) {
                break;
            }
            joined.push(' ');
            let normalized_block = normalize_table_text(&blocks[end]);
            if matched_start.is_none()
                && snippets
                    .iter()
                    .any(|snippet| normalized_block.contains(snippet.as_str()))
            {
                matched_start = Some(end);
            }
            joined.push_str(&normalized_block);
            let score = snippets
                .iter()
                .filter(|snippet| joined.contains(snippet.as_str()))
                .count();
            if score >= required_score {
                return matched_start.map(|matched| {
                    expand_table_window_to_cover_trailing_snippets(
                        blocks,
                        matched,
                        end + 1,
                        range_end,
                        snippets,
                    )
                });
            }
        }
    }
    None
}

fn expand_table_window_to_cover_trailing_snippets(
    blocks: &[String],
    start: usize,
    end: usize,
    range_end: usize,
    snippets: &[String],
) -> (usize, usize) {
    let mut joined = blocks[start..end]
        .iter()
        .map(|block| normalize_table_text(block))
        .collect::<Vec<_>>()
        .join(" ");
    let mut current_end = end.min(range_end).min(blocks.len());

    while current_end < range_end.min(blocks.len()) {
        if is_hard_table_range_boundary(blocks[current_end].trim_start()) {
            break;
        }
        let next_block = normalize_table_text(&blocks[current_end]);
        if next_block.is_empty() {
            break;
        }
        let adds_new_snippet = snippets
            .iter()
            .any(|snippet| !joined.contains(snippet.as_str()) && next_block.contains(snippet));
        if !adds_new_snippet {
            break;
        }
        joined.push(' ');
        joined.push_str(&next_block);
        current_end += 1;
    }

    (start, current_end)
}

fn find_table_row_anchor_window_bounded(
    blocks: &[String],
    cursor: usize,
    range_end: usize,
    row_anchors: &[String],
) -> Option<(usize, usize)> {
    let range_end = range_end.min(blocks.len());
    let required_matches = row_anchors.len().min(2).max(1);
    for start in cursor..range_end {
        let mut joined = String::new();
        let mut matched = 0usize;
        let mut anchor_start = None;
        for end in start..range_end.min(start + 8) {
            if is_hard_table_range_boundary(blocks[end].trim_start()) {
                break;
            }
            let normalized_block = normalize_table_text(&blocks[end]);
            if normalized_block.is_empty() {
                continue;
            }
            joined.push(' ');
            joined.push_str(&normalized_block);
            let current_matched = row_anchors
                .iter()
                .filter(|anchor| joined.contains(anchor.as_str()))
                .count();
            if current_matched > matched {
                matched = current_matched;
                if anchor_start.is_none() {
                    anchor_start = Some(end);
                }
            }
            if matched >= required_matches && looks_like_row_anchor_table_window(&joined) {
                return anchor_start.map(|anchor| (anchor, end + 1));
            }
        }
    }
    None
}

fn looks_like_row_anchor_table_window(value: &str) -> bool {
    let word_count = value.split_whitespace().count();
    word_count >= 12
}

fn normalize_table_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

impl BBox {
    fn from_values(values: &[i32]) -> Option<Self> {
        if values.len() < 4 {
            return None;
        }
        let bbox = Self {
            left: values[0],
            top: values[1],
            right: values[2],
            bottom: values[3],
        };
        bbox.is_valid().then_some(bbox)
    }

    fn expand(self, margin: i32, image_width: i32, image_height: i32) -> Option<Self> {
        let left = (self.left - margin).max(0);
        let top = (self.top - margin).max(0);
        let right = (self.right + margin).min(image_width);
        let bottom = (self.bottom + margin).min(image_height);
        let bbox = Self {
            left,
            top,
            right,
            bottom,
        };
        bbox.is_valid().then_some(bbox)
    }

    fn union(self, other: &Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn overlaps_or_touches(self, other: &Self) -> bool {
        let horizontal = self.left <= other.right + 12 && other.left <= self.right + 12;
        let vertical = self.top <= other.bottom + 12 && other.top <= self.bottom + 12;
        horizontal && vertical
    }

    fn is_valid(self) -> bool {
        self.right > self.left && self.bottom > self.top
    }

    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }
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

    #[test]
    fn protects_raw_pdf_table_html() {
        let markdown = "Before\n\n<table><tr><td>A</td></tr></table>\n\nAfter";

        let protected = protect_pdf_table_blocks(markdown, &[]);

        assert!(protected.contains(PDF_TABLE_START_MARKER));
        assert!(protected.contains(PDF_TABLE_IMAGE_MISSING_MARKER));
        assert!(!protected.contains("<table><tr><td>A</td></tr></table>"));
        assert!(protected.contains(PDF_TABLE_END_MARKER));
    }

    #[test]
    fn does_not_double_protect_existing_table_block() {
        let markdown = format!("{PDF_TABLE_START_MARKER}\n<table></table>\n{PDF_TABLE_END_MARKER}");

        assert_eq!(protect_pdf_table_blocks(&markdown, &[]), markdown);
    }

    #[test]
    fn replaces_raw_pdf_table_with_snapshot_image_when_available() {
        let markdown = "Before\n\n<table><tr><td>A</td></tr></table>\n\nAfter";
        let images = vec!["imgs/table_1.png".to_string()];

        let protected = protect_pdf_table_blocks(markdown, &images);

        assert!(protected.contains("![PDF table 1](imgs/table_1.png)"));
        assert!(!protected.contains("<table>"));
    }

    #[test]
    fn leaves_missing_marker_when_no_layout_table_candidate_exists() {
        let markdown = format!(
            "{PDF_TABLE_START_MARKER}\n{PDF_TABLE_IMAGE_MISSING_MARKER}\n{PDF_TABLE_END_MARKER}"
        );

        let output = replace_missing_pdf_table_snapshots(&markdown, &[], Path::new("."));

        assert_eq!(output, markdown);
    }

    #[test]
    fn backfills_missing_marker_from_table_layout_crop() {
        let root = temp_artifact_root("table-crop");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(180, 120, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = format!(
            "Caption\n\n{PDF_TABLE_START_MARKER}\n{PDF_TABLE_IMAGE_MISSING_MARKER}\n{PDF_TABLE_END_MARKER}"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![OcrLayoutBlock {
                label: "table".to_string(),
                content: String::new(),
                bbox: vec![20, 30, 160, 90],
                order: Some(4),
                page: Some(0),
            }],
        }];

        let output = replace_missing_pdf_table_snapshots(&markdown, &layouts, &root);

        assert!(output.contains("![PDF table 1](imgs/table_snapshot_001.jpg)"));
        assert!(!output.contains(PDF_TABLE_IMAGE_MISSING_MARKER));
        assert!(root.join("imgs/table_snapshot_001.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn maps_multiple_missing_markers_to_multiple_table_regions() {
        let root = temp_artifact_root("multi-table-crop");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = format!(
            "{PDF_TABLE_START_MARKER}\n{PDF_TABLE_IMAGE_MISSING_MARKER}\n{PDF_TABLE_END_MARKER}\n\n{PDF_TABLE_START_MARKER}\n{PDF_TABLE_IMAGE_MISSING_MARKER}\n{PDF_TABLE_END_MARKER}"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                table_block(vec![20, 20, 180, 80], 1),
                table_block(vec![20, 110, 180, 180], 2),
            ],
        }];

        let output = replace_missing_pdf_table_snapshots(&markdown, &layouts, &root);

        assert!(output.contains("imgs/table_snapshot_001.jpg"));
        assert!(output.contains("imgs/table_snapshot_002.jpg"));
        assert!(!output.contains(PDF_TABLE_IMAGE_MISSING_MARKER));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn replaces_flattened_layout_table_text_with_snapshot() {
        let root = temp_artifact_root("layout-table-crop");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = concat!(
            "Table 1. Some working examples of nudge plus.\n\n",
            "Timing of nudge plus\n\n",
            "Simultaneous     Sequential\n\n",
            "Type of nudge plus\n\n",
            "One-part GPS Dual self-pledge device\n\n",
            "Two-part A nudge with an information signal A default followed by a pledge\n\n",
            "After table paragraph."
        );
        let html = concat!(
            "<table><tr><td colspan=\"2\">Timing of nudge plus</td></tr>",
            "<tr><td>Simultaneous</td><td>Sequential</td></tr>",
            "<tr><td>Type of nudge plus</td><td>One-part</td></tr>",
            "<tr><td>GPS</td><td>Dual self-pledge device</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![table_block_with_content(vec![85, 136, 766, 313], 4, html)],
        }];

        let output = replace_layout_tables_with_snapshots(markdown, &layouts, &root);

        assert!(output.contains("Table 1. Some working examples of nudge plus."));
        assert!(output.contains(PDF_TABLE_START_MARKER));
        assert!(output.contains("![PDF table 1](imgs/table_snapshot_001.jpg)"));
        assert!(output.contains(PDF_TABLE_END_MARKER));
        assert!(output.contains("After table paragraph."));
        assert!(!output.contains("Timing of nudge plus\n\nSimultaneous"));
        assert!(root.join("imgs/table_snapshot_001.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn replaces_realistic_captioned_table_when_table_block_uses_late_page_index() {
        let root = temp_artifact_root("layout-table-realistic-page-index");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_6.jpg"))
            .expect("save page");
        let markdown = concat!(
            "https://doi.org/10.1017/bpp.2021.6 Published online by Cambridge University Press\n\n",
            "Behavioural Public Policy 75\n\n",
            "Table 1. Some working examples of nudge plus.\n\n",
            "Timing of nudge plus\n\n",
            "Simultaneous     Sequential\n\n",
            "Type of nudge plus\n\n",
            "One-part GPS Dual self-pledge device\n\n",
            "Two-part A nudge (e.g., traffic lighting scheme) with an information signal\n\n",
            "A nudge (e.g., default) either preceded or followed by a pledge\n\n",
            "After table paragraph."
        );
        let html = concat!(
            "<table>",
            "<tr><td rowspan=\"2\"></td><td rowspan=\"2\"></td><td colspan=\"2\">Timing of nudge plus</td></tr>",
            "<tr><td>Simultaneous</td><td>Sequential</td></tr>",
            "<tr><td rowspan=\"2\">Type of nudge plus</td><td>One-part</td><td>GPS</td><td>Dual self-pledge device</td></tr>",
            "<tr><td>Two-part</td><td>A nudge (e.g., traffic lighting scheme) with an information signal</td><td>A nudge (e.g., default) either preceded or followed by a pledge</td></tr>",
            "</table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                OcrLayoutBlock {
                    label: "figure_title".to_string(),
                    content: "Table 1. Some working examples of nudge plus.".to_string(),
                    bbox: vec![82, 110, 410, 130],
                    order: None,
                    page: Some(6),
                },
                OcrLayoutBlock {
                    label: "table".to_string(),
                    content: html.to_string(),
                    bbox: vec![85, 136, 766, 313],
                    order: None,
                    page: Some(6),
                },
            ],
        }];

        let output = replace_layout_tables_with_stable_snapshots(markdown, &layouts, &root);

        assert!(output.contains("Table 1. Some working examples of nudge plus."));
        assert!(output.contains(PDF_TABLE_START_MARKER), "{output}");
        assert!(output.contains("![PDF table 1](imgs/table_snapshot_001.jpg)"));
        assert!(output.contains("After table paragraph."));
        assert!(!output.contains("Timing of nudge plus\n\nSimultaneous"));
        assert!(root.join("imgs/table_snapshot_001.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn replaces_policy_pdf_raw_ocr_table_sequence_with_snapshot() {
        let root = temp_artifact_root("layout-table-policy-raw-ocr-sequence");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_6.jpg"))
            .expect("save page");
        let markdown = concat!(
            "How can nudge plus be designed and administered? This depends on two factors: the timing of the delivery of the plus with the nudge and the combination strategy. The plus can be conceived by the policy-maker to be delivered before, after, or as part of the classic nudge, as either a one- or two-part device (see Table 1). The preferred order of the nudge and plus depends on the task, generating different treatment effects. Although nudge and plus can be separable, as in the two-part device, both elements are complementary in the functioning of nudge plus. The agent receiving the reflective plus switches from thinking fast to thinking slow in a way that helps responding to nudge. When stand-alone, plus reduces to simple think. While nudge involves any change in the choice architecture, it can prompt either an unconscious, reflexive action (a system 1 nudge) or a conscious, reflective action (system 2 nudge) but not both. A think involves a purely educative strategy that prompts deliberation. Nudge plus is a hybrid as it modifies the nudge by prompting both conscious and unconscious actions. It must have nudge as its fully functional and central unit, with the reflective device designed to enhance the reflection of the receipt of the nudge. While nudge plus promises greater autonomy and token transparency relative\n\n",
            "https://doi.org/10.1017/bpp.2021.6 Published online by Cambridge University Press\n\n",
            "Behavioural Public Policy 75\n\n",
            "Table 1. Some working examples of nudge plus.\n\n",
            "Timing of nudge plus\n\n",
            "Simultaneous     Sequential\n\n",
            "Type of nudge plus\n\n",
            "One-part GPS Dual self-pledge device\n\n",
            "Two-part A nudge (e.g., traffic lighting scheme) with an information signal\n\n",
            "A nudge (e.g., default) either preceded or followed by a pledge\n\n",
            "to nudge, each works differently. Some classes of plus may work by making the design and construct of the existing nudge more salient to the receiver, while others might allow the agent to reflect deeply on their own preferences. However, the change in effectiveness might be an ambiguous signal to the policy-maker in these latter instances. This remains a normative judgment, for what is considered best for the agent by the policy-maker might not be true for the agent."
        );
        let html = concat!(
            "<table>",
            "<tr><td rowspan=\"2\"></td><td rowspan=\"2\"></td><td colspan=\"2\">Timing of nudge plus</td></tr>",
            "<tr><td>Simultaneous</td><td>Sequential</td></tr>",
            "<tr><td rowspan=\"2\">Type of nudge plus</td><td>One-part</td><td>GPS</td><td>Dual self-pledge device</td></tr>",
            "<tr><td>Two-part</td><td>A nudge (e.g., traffic lighting scheme) with an information signal</td><td>A nudge (e.g., default) either preceded or followed by a pledge</td></tr>",
            "</table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                OcrLayoutBlock {
                    label: "figure_title".to_string(),
                    content: "Table 1. Some working examples of nudge plus.".to_string(),
                    bbox: vec![82, 110, 410, 130],
                    order: None,
                    page: Some(6),
                },
                OcrLayoutBlock {
                    label: "table".to_string(),
                    content: html.to_string(),
                    bbox: vec![85, 136, 766, 313],
                    order: None,
                    page: Some(6),
                },
            ],
        }];

        let output = replace_layout_tables_with_stable_snapshots(markdown, &layouts, &root);

        assert!(
            output.contains("Table 1. Some working examples of nudge plus."),
            "{output}"
        );
        assert!(output.contains(PDF_TABLE_START_MARKER), "{output}");
        assert!(
            output.contains("![PDF table 1](imgs/table_snapshot_001.jpg)"),
            "{output}"
        );
        assert!(!output.contains("Timing of nudge plus"), "{output}");
        assert!(!output.contains("Simultaneous     Sequential"), "{output}");
        assert!(
            output.contains("to nudge, each works differently."),
            "{output}"
        );
        assert!(root.join("imgs/table_snapshot_001.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn layout_table_snapshot_numbering_skips_existing_protected_tables() {
        let root = temp_artifact_root("layout-table-after-existing");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = concat!(
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n\n",
            "Timing of nudge plus\n\n",
            "Simultaneous     Sequential\n\n",
            "After table paragraph."
        );
        let html = concat!(
            "<table><tr><td>Timing of nudge plus</td></tr>",
            "<tr><td>Simultaneous</td><td>Sequential</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![table_block_with_content(vec![85, 136, 766, 313], 4, html)],
        }];

        let output = replace_layout_tables_with_snapshots(markdown, &layouts, &root);

        assert!(output.contains("imgs/table_snapshot_001.jpg"));
        assert!(output.contains("imgs/table_snapshot_002.jpg"));
        assert!(!output.contains("Timing of nudge plus\n\nSimultaneous"));
        assert!(root.join("imgs/table_snapshot_002.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn collapse_pdf_table_blocks_isolates_inline_trailing_text() {
        let markdown = format!("Before.\n\n",);
        let markdown = format!(
            "{markdown}{PDF_TABLE_START_MARKER} ![PDF table 1](imgs/table_snapshot_001.jpg) {PDF_TABLE_END_MARKER} to increase transparency; for example, a default can be administered.\n\nAfter."
        );

        let output = collapse_adjacent_pdf_table_blocks(&markdown);

        let expected = format!(
            "{PDF_TABLE_START_MARKER}\n![PDF table 1](imgs/table_snapshot_001.jpg)\n{PDF_TABLE_END_MARKER}\n\nto increase transparency; for example"
        );
        assert!(output.contains(&expected));
        assert_eq!(output.matches(PDF_TABLE_START_MARKER).count(), 1);
        assert!(output.contains("\n\nAfter."));
    }

    #[test]
    fn replaces_repeated_flattened_windows_for_same_layout_table() {
        let root = temp_artifact_root("layout-table-repeated");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let flattened = concat!(
            "Moderated mediation model\n\n",
            "Outcome Predictor SE Z p b 95% CI (b)\n\n",
            "Reception Confidence 0 -13.96 <0.001 -0.01"
        );
        let markdown = format!(
            "Before first.\n\n{flattened}\n\nBetween copies.\n\n{flattened}\n\nAfter second."
        );
        let html = concat!(
            "<table><tr><td>Moderated mediation model</td></tr>",
            "<tr><td>Outcome</td><td>Predictor</td><td>SE</td><td>Z</td><td>p</td></tr>",
            "<tr><td>Reception</td><td>Confidence</td><td>0</td><td>-13.96</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![table_block_with_content(vec![85, 136, 766, 313], 4, html)],
        }];

        let output = replace_layout_tables_with_snapshots(&markdown, &layouts, &root);

        assert_eq!(output.matches(PDF_TABLE_START_MARKER).count(), 1);
        assert_eq!(output.matches("imgs/table_snapshot_001.jpg").count(), 1);
        assert!(!output.contains("Outcome Predictor SE Z p"));
        assert!(root.join("imgs/table_snapshot_001.jpg").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn caption_search_skips_existing_snapshot_before_continued_table_text() {
        let root = temp_artifact_root("layout-table-continued-after-existing");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let existing = protected_table_image_block("![PDF table 1](imgs/table_snapshot_001.jpg)");
        let markdown = format!(
            "Table 1 | - continued from previous page\n\n{existing}\n\nSource-credibility labels show\n\nNewsGuard labels indicate\n\nSharing intentions\n\nAfter table."
        );
        let html = concat!(
            "<table><tr><td>Intervention</td><td>Description</td><td>Example</td><td>Outcome variables</td></tr>",
            "<tr><td>Source-credibility labels</td>",
            "<td>Source-credibility labels show</td>",
            "<td>NewsGuard labels indicate</td>",
            "<td>Sharing intentions</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                caption_block(
                    "Table 1 - continued from previous page",
                    vec![85, 80, 766, 110],
                    3,
                ),
                table_block_with_content(vec![85, 136, 766, 313], 4, html),
            ],
        }];

        let output = replace_layout_tables_with_snapshots(&markdown, &layouts, &root);

        assert!(output.contains("imgs/table_snapshot_001.jpg"));
        assert!(output.contains("imgs/table_snapshot_002.jpg"));
        assert!(!output.contains("Source-credibility labels show"));
        assert!(output.contains("After table."));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inherits_caption_anchor_for_continued_tables_without_local_title() {
        let root = temp_artifact_root("layout-table-caption-inherit");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 900, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page 0");
        image
            .save(root.join("ocr_input/page_1.jpg"))
            .expect("save page 1");
        image
            .save(root.join("ocr_input/page_2.jpg"))
            .expect("save page 2");
        let markdown = concat!(
            "## A.2 Co-authors and Their Areas of Expertise\n\n",
            "Table A1: Co-authors\n\n",
            "Anastasia Kozyreva MPI for Human Development Germany General coordination Philosophy and cognitive science\n\n",
            "Philipp Schmid University of Erfurt Germany Debunking and rebuttals Psychology\n\n",
            "Paula Szewach University of Exeter Barcelona Supercomputing Center UK Spain Warning and fact-checking labels Political science\n\n",
            "Sam Wineburg Stanford University USA Lateral reading and verification strategies Educational psychology\n\n",
            "## References\n\n",
            "[1] Schmid, P.: Reference entry.\n\n",
            "[2] Wineburg, S.: Reference entry."
        );
        let header_row = concat!(
            "<tr><td>Name</td><td>Affiliation</td><td>Country</td>",
            "<td>Contribution to the toolbox</td><td>Academic discipline</td></tr>"
        );
        let first_html = concat!(
            "<table>",
            "<tr><td>Name</td><td>Affiliation</td><td>Country</td>",
            "<td>Contribution to the toolbox</td><td>Academic discipline</td></tr>",
            "<tr><td>Anastasia Kozyreva</td><td>MPI for Human Development</td><td>Germany</td>",
            "<td>General coordination</td><td>Philosophy and cognitive science</td></tr></table>"
        );
        let second_html = format!(
            "<table>{header_row}<tr><td>Philipp Schmid</td><td>University of Erfurt</td><td>Germany</td><td>Debunking and rebuttals</td><td>Psychology</td></tr></table>"
        );
        let third_html = format!(
            "<table>{header_row}<tr><td>Paula Szewach</td><td>University of Exeter Barcelona Supercomputing Center</td><td>UK Spain</td><td>Warning and fact-checking labels</td><td>Political science</td></tr><tr><td>Sam Wineburg</td><td>Stanford University</td><td>USA</td><td>Lateral reading and verification strategies</td><td>Educational psychology</td></tr></table>"
        );
        let layouts = vec![
            OcrPageLayout {
                blocks: vec![
                    caption_block("Table A1: Co-authors", vec![85, 80, 766, 110], 3),
                    OcrLayoutBlock {
                        label: "table".to_string(),
                        content: first_html.to_string(),
                        bbox: vec![85, 136, 766, 313],
                        order: Some(4),
                        page: Some(0),
                    },
                ],
            },
            OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "table".to_string(),
                    content: second_html.clone(),
                    bbox: vec![85, 120, 766, 313],
                    order: Some(4),
                    page: Some(1),
                }],
            },
            OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "table".to_string(),
                    content: third_html.clone(),
                    bbox: vec![85, 120, 766, 360],
                    order: Some(4),
                    page: Some(2),
                }],
            },
        ];
        let candidates = table_snapshot_candidates(&layouts);
        assert_eq!(candidates.len(), 3, "{candidates:#?}");
        assert_eq!(candidates[0].caption, "Table A1: Co-authors");
        assert_eq!(candidates[1].caption, "Table A1: Co-authors");
        assert_eq!(candidates[2].caption, "Table A1: Co-authors");

        let output = replace_layout_tables_with_stable_snapshots(markdown, &layouts, &root);

        assert_eq!(
            output.matches(PDF_TABLE_START_MARKER).count(),
            3,
            "{output}"
        );
        assert!(!output.contains("Philipp Schmid University of Erfurt"));
        assert!(!output.contains("Paula Szewach University of Exeter"));
        assert!(!output.contains("Sam Wineburg Stanford University"));
        assert!(output.contains("## References"));
        assert!(output.contains("[1] Schmid, P.: Reference entry."));
        assert!(output.contains("[2] Wineburg, S.: Reference entry."));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scans_each_layout_table_from_document_start() {
        let root = temp_artifact_root("layout-table-independent-cursor");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = concat!(
            "Second table early body.\n\n",
            "Moderated mediation model\n\n",
            "Outcome Predictor SE Z p\n\n",
            "Intervening prose.\n\n",
            "First table later body.\n\n",
            "Model comparison with WAIC\n\n",
            "Models WAIC Weight"
        );
        let first_html = concat!(
            "<table><tr><td>Model comparison with WAIC</td></tr>",
            "<tr><td>Models</td><td>WAIC</td><td>Weight</td></tr></table>"
        );
        let second_html = concat!(
            "<table><tr><td>Moderated mediation model</td></tr>",
            "<tr><td>Outcome</td><td>Predictor</td><td>SE</td><td>Z</td><td>p</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                table_block_with_content(vec![85, 136, 766, 313], 4, first_html),
                table_block_with_content(vec![85, 330, 766, 410], 8, second_html),
            ],
        }];

        let output = replace_layout_tables_with_snapshots(markdown, &layouts, &root);

        assert!(output.contains("imgs/table_snapshot_001.jpg"));
        assert!(output.contains("imgs/table_snapshot_002.jpg"));
        assert!(!output.contains("Model comparison with WAIC\n\nModels"));
        assert!(!output.contains("Moderated mediation model\n\nOutcome"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn collapses_only_duplicate_adjacent_pdf_table_blocks() {
        let table_one = protected_table_image_block("![PDF table 1](imgs/table_snapshot_001.jpg)");
        let table_two = protected_table_image_block("![PDF table 2](imgs/table_snapshot_002.jpg)");
        let markdown =
            format!("Caption A\n\n{table_one}\n\n{table_two}\n\n{table_two}\n\nCaption B");

        let output = collapse_adjacent_pdf_table_blocks(&markdown);

        assert_eq!(output.matches(PDF_TABLE_START_MARKER).count(), 2);
        assert!(output.contains("Caption A"));
        assert!(output.contains("Caption B"));
        assert!(output.contains("imgs/table_snapshot_001.jpg"));
        assert!(output.contains("imgs/table_snapshot_002.jpg"));
    }

    #[test]
    fn table_range_boundary_stops_at_pdf_figure_blocks() {
        let blocks = vec![
            "table residual text".to_string(),
            "![PDF figure 1](imgs/figure_snapshot_001.jpg)".to_string(),
            "::: figure-caption\nFig. 1 Structure of the toolbox.\n:::".to_string(),
        ];

        assert_eq!(next_table_range_boundary(&blocks, 1), 1);
    }

    #[test]
    fn table_group_residual_cleanup_preserves_following_pdf_figure() {
        let mut blocks = vec![
            protected_table_image_block("![PDF table 1](imgs/table_snapshot_001.jpg)"),
            "leftover table text".to_string(),
            "![PDF figure 1](imgs/figure_snapshot_001.jpg)".to_string(),
            "::: figure-caption\nFig. 1 Structure of the toolbox.\n:::".to_string(),
            "After figure.".to_string(),
        ];

        clear_table_group_residual_blocks(&mut blocks, 1);

        assert!(!blocks.iter().any(|block| block.contains("leftover table")));
        assert!(blocks
            .iter()
            .any(|block| block.contains("figure_snapshot_001.jpg")));
        assert!(blocks
            .iter()
            .any(|block| block.contains("Fig. 1 Structure")));
        assert!(blocks.iter().any(|block| block.contains("After figure.")));
    }

    #[test]
    fn fallback_table_matching_does_not_cross_pdf_figure_block() {
        let root = temp_artifact_root("layout-table-figure-boundary");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = concat!(
            "Before table text.\n\n",
            "Alpha header\n\n",
            "![PDF figure 1](imgs/figure_snapshot_001.jpg)\n\n",
            "::: figure-caption\n",
            "Fig. 1 Structure of the toolbox.\n",
            ":::\n\n",
            "Beta row\n\n",
            "Gamma tail\n\n",
            "After figure."
        );
        let html = concat!(
            "<table>",
            "<tr><td>Alpha header</td></tr>",
            "<tr><td>Beta row</td></tr>",
            "<tr><td>Gamma tail</td></tr>",
            "</table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![table_block_with_content(vec![85, 136, 766, 313], 4, html)],
        }];

        let output = replace_layout_tables_with_snapshots(markdown, &layouts, &root);

        assert!(!output.contains(PDF_TABLE_START_MARKER), "{output}");
        assert!(output.contains("![PDF figure 1](imgs/figure_snapshot_001.jpg)"));
        assert!(output.contains("Fig. 1 Structure of the toolbox."));
        assert!(output.contains("Alpha header"));
        assert!(output.contains("Beta row"));
        assert!(output.contains("Gamma tail"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fallback_table_matching_stops_before_references() {
        let root = temp_artifact_root("layout-table-references-boundary");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = concat!(
            "Main text before references.\n\n",
            "## References\n\n",
            "Anastasia Kozyreva MPI for Human Development Germany General coordination Philosophy.\n\n",
            "Philipp Lorenz-Spreen MPI for Human Development Germany Network science."
        );
        let html = concat!(
            "<table><tr><td>Name</td><td>Affiliation</td><td>Country</td></tr>",
            "<tr><td>Anastasia Kozyreva</td><td>MPI for Human Development</td><td>Germany</td></tr>",
            "<tr><td>Philipp Lorenz-Spreen</td><td>MPI for Human Development</td><td>Germany</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![table_block_with_content(vec![85, 136, 766, 313], 4, html)],
        }];

        let output = replace_layout_tables_with_snapshots(markdown, &layouts, &root);

        assert!(!output.contains(PDF_TABLE_START_MARKER));
        assert!(output.contains("Anastasia Kozyreva"));
        assert!(output.contains("Philipp Lorenz-Spreen"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn caption_anchored_table_matching_stops_before_references() {
        let root = temp_artifact_root("layout-table-caption-references-boundary");
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let markdown = concat!(
            "Main text before references.\n\n",
            "## References\n\n",
            "Table 2. Contributors.\n\n",
            "Name Affiliation Country Contribution to the toolbox\n\n",
            "Anastasia Kozyreva MPI Germany General coordination."
        );
        let html = concat!(
            "<table><tr><td>Name</td><td>Affiliation</td><td>Country</td></tr>",
            "<tr><td>Anastasia Kozyreva</td><td>MPI</td><td>Germany</td></tr></table>"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                caption_block("Table 2. Contributors.", vec![85, 80, 766, 110], 3),
                table_block_with_content(vec![85, 136, 766, 313], 4, html),
            ],
        }];

        let output = replace_layout_tables_with_snapshots(markdown, &layouts, &root);

        assert!(!output.contains(PDF_TABLE_START_MARKER));
        assert!(output.contains("Table 2. Contributors."));
        assert!(output.contains("Anastasia Kozyreva"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn collapses_duplicate_continued_table_captions() {
        let table = protected_table_image_block("![PDF table 1](imgs/table_snapshot_001.jpg)");
        let markdown = format!(
            "Table 1 | - continued from previous page\n\nTable 1 | - continued from previous page\n\n{table}\n\nAfter."
        );

        let output = collapse_adjacent_pdf_table_blocks(&markdown);

        assert_eq!(
            output
                .matches("Table 1 | - continued from previous page")
                .count(),
            1
        );
        assert!(output.contains(PDF_TABLE_START_MARKER));
        assert!(output.contains("After."));
    }

    fn table_block(bbox: Vec<i32>, order: i32) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: "html_table".to_string(),
            content: String::new(),
            bbox,
            order: Some(order),
            page: Some(0),
        }
    }

    fn table_block_with_content(bbox: Vec<i32>, order: i32, content: &str) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: "table".to_string(),
            content: content.to_string(),
            bbox,
            order: Some(order),
            page: Some(0),
        }
    }

    fn caption_block(content: &str, bbox: Vec<i32>, order: i32) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: "figure_title".to_string(),
            content: content.to_string(),
            bbox,
            order: Some(order),
            page: Some(0),
        }
    }

    fn temp_artifact_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "musetranslate-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
