use crate::ocr::OcrPageLayout;
use std::collections::HashMap;
use std::path::Path;

mod captions;
mod continuation;
mod figure_crop;
mod json_render;
mod media;
mod table;

pub(crate) use table::{
    is_pdf_table_block, PDF_TABLE_END_MARKER, PDF_TABLE_IMAGE_MISSING_MARKER,
    PDF_TABLE_START_MARKER,
};

pub(super) fn parse_markdown_image(line: &str) -> Option<(String, String)> {
    media::parse_markdown_image(line)
}

pub(super) fn is_figure_or_table_label(label: &str) -> bool {
    media::is_figure_or_table_label(label)
}

pub(super) fn normalize_figure_table_label(label: &str) -> String {
    media::normalize_figure_table_label(label)
}

pub(super) fn replace_split_figure_clusters_with_crops(
    markdown: &str,
    markdown_images: &HashMap<String, String>,
    artifact_root: &Path,
) -> String {
    figure_crop::replace_split_figure_clusters_with_crops(markdown, markdown_images, artifact_root)
}

pub(super) fn rebuild_pdf_markdown_from_layout_json(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> json_render::JsonMarkdownRebuild {
    json_render::rebuild_pdf_markdown_from_layout_json(markdown, page_layouts, artifact_root)
}

pub(super) fn replace_missing_pdf_table_snapshots(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> String {
    table::replace_missing_pdf_table_snapshots(markdown, page_layouts, artifact_root)
}

pub(super) fn replace_layout_tables_with_snapshots(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> String {
    table::replace_layout_tables_with_snapshots(markdown, page_layouts, artifact_root)
}

pub(super) fn replace_layout_tables_with_stable_snapshots(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    artifact_root: &Path,
) -> String {
    table::replace_layout_tables_with_stable_snapshots(markdown, page_layouts, artifact_root)
}

pub(super) fn collapse_adjacent_pdf_table_blocks(markdown: &str) -> String {
    table::collapse_adjacent_pdf_table_blocks(markdown)
}

pub(super) fn inject_layout_captions(markdown: &str, page_layouts: &[OcrPageLayout]) -> String {
    captions::inject_layout_captions(markdown, page_layouts)
}

pub(super) fn normalize_pdf_markdown_after_front_matter_rewrite(markdown: &str) -> String {
    let normalized = rejoin_parenthetical_reference_breaks(markdown);
    let normalized = relocate_interrupting_footnotes(&normalized);
    strip_top_update_notice_blocks(&normalized)
}

pub(super) fn normalize_pdf_source_markdown(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
    markdown_images: &HashMap<String, String>,
    _article_type: &str,
) -> String {
    let layout_normalized = if page_layouts.is_empty() {
        markdown.to_string()
    } else {
        continuation::normalize_with_layout(markdown, page_layouts)
    };
    let normalized = move_interrupted_figure_blocks(&layout_normalized);
    let normalized = strip_top_update_notice_blocks(&normalized);
    let normalized = cleanup_pdf_structure_noise(&normalized);
    let normalized = rejoin_parenthetical_reference_breaks(&normalized);
    let normalized = relocate_interrupting_footnotes(&normalized);
    let normalized = strip_layout_page_chrome_blocks(&normalized, page_layouts);
    let normalized = drop_near_duplicate_following_blocks(&normalized);
    let normalized = insert_missing_layout_headings(&normalized, page_layouts);
    let normalized = strengthen_pdf_section_headings(&normalized, page_layouts);
    table::protect_pdf_table_blocks(&normalized, &table_snapshot_image_paths(markdown_images))
}

fn strip_top_update_notice_blocks(markdown: &str) -> String {
    let blocks = split_markdown_blocks_preserve_order(markdown);
    if blocks.is_empty() {
        return String::new();
    }

    let mut result = Vec::with_capacity(blocks.len());
    let mut in_leading_region = true;
    for block in blocks {
        if in_leading_region && is_top_update_notice_block(&block) {
            continue;
        }
        if in_leading_region && looks_like_body_start_block(&block) {
            in_leading_region = false;
        }
        result.push(block);
    }
    result.join("\n\n")
}

fn is_top_update_notice_block(block: &str) -> bool {
    let normalized = block
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    normalized == "check for updates"
        || normalized == "check for update"
        || normalized == "view pdf"
}

fn looks_like_body_start_block(block: &str) -> bool {
    let trimmed = block.trim();
    if trimmed.starts_with('#')
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("doi:")
    {
        return false;
    }
    trimmed.chars().count() >= 180 && continuation::paragraph_looks_complete(trimmed)
}

fn table_snapshot_image_paths(markdown_images: &HashMap<String, String>) -> Vec<String> {
    let mut paths = markdown_images
        .keys()
        .filter(|path| looks_like_table_snapshot_path(path))
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn looks_like_table_snapshot_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("table") || lower.contains("tbl")
}

fn move_interrupted_figure_blocks(markdown: &str) -> String {
    let blocks = split_markdown_blocks_preserve_order(markdown);
    let mut output = Vec::new();
    let mut pending_media = Vec::new();
    let mut index = 0usize;

    while index < blocks.len() {
        if let Some((media_block, consumed)) = collect_media_block(&blocks, index) {
            if table::is_pdf_table_block(media_block.trim()) {
                output.push(media_block);
                index += consumed;
                continue;
            }
            if should_rejoin_paragraph_around_media(&output, &blocks, index + consumed) {
                rejoin_paragraph_around_media(&mut output, &blocks, index, consumed, media_block);
                index += consumed + 1;
                continue;
            }
            if media_can_stay_in_place(&output)
                || next_block_is_structural_media_boundary(&blocks, index + consumed)
            {
                output.push(media_block);
            } else {
                pending_media.push(media_block);
            }
            index += consumed;
            continue;
        }

        output.push(blocks[index].clone());
        if !pending_media.is_empty() && continuation::paragraph_looks_complete(&blocks[index]) {
            output.append(&mut pending_media);
        }
        index += 1;
    }

    output.append(&mut pending_media);
    output.join("\n\n")
}

fn cleanup_pdf_structure_noise(markdown: &str) -> String {
    let blocks = split_markdown_blocks_preserve_order(markdown);
    let mut output: Vec<String> = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        let cleaned_block = clean_pdf_license_footer_fragments(&blocks[index]);
        let block = cleaned_block.trim();
        if block.is_empty() {
            index += 1;
            continue;
        }
        if is_table_continuation_noise(block) {
            index += 1;
            continue;
        }
        if let Some(previous) = output.last() {
            if is_duplicate_table_title(previous, block) {
                index += 1;
                continue;
            }
        }
        output.push(cleaned_block);
        index += 1;
    }

    output.join("\n\n")
}

fn rejoin_parenthetical_reference_breaks(markdown: &str) -> String {
    let blocks = split_markdown_blocks_preserve_order(markdown);
    let mut output: Vec<String> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let trimmed = block.trim();
        if output
            .last()
            .is_some_and(|previous| should_join_parenthetical_reference_break(previous, trimmed))
        {
            if let Some(previous) = output.pop() {
                output.push(continuation::join_interrupted_paragraph(&previous, trimmed));
            }
            continue;
        }
        output.push(block);
    }
    output.join("\n\n")
}

fn should_join_parenthetical_reference_break(previous: &str, current: &str) -> bool {
    if previous.trim().is_empty() || current.trim().is_empty() {
        return false;
    }
    has_unclosed_parenthesis(previous) && starts_with_citation_closing_fragment(current)
}

fn has_unclosed_parenthesis(value: &str) -> bool {
    let mut balance = 0i32;
    for ch in value.chars() {
        match ch {
            '(' => balance += 1,
            ')' if balance > 0 => balance -= 1,
            _ => {}
        }
    }
    balance > 0
}

fn starts_with_citation_closing_fragment(value: &str) -> bool {
    let trimmed = value.trim_start();
    let close_index = trimmed.find(')').unwrap_or(usize::MAX);
    if close_index == usize::MAX || close_index > 96 {
        return false;
    }
    let before_close = &trimmed[..close_index];
    before_close.chars().any(|ch| ch.is_ascii_digit())
        && before_close.contains(',')
        && trimmed[close_index + 1..]
            .trim_start()
            .chars()
            .find(|ch| {
                !ch.is_whitespace() && !matches!(ch, '.' | ',' | ';' | ':' | ')' | ']' | '}')
            })
            .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn relocate_interrupting_footnotes(markdown: &str) -> String {
    let blocks = split_markdown_blocks_preserve_order(markdown);
    let mut output = Vec::<String>::with_capacity(blocks.len());
    let mut index = 0usize;
    while index < blocks.len() {
        if index + 1 < blocks.len()
            && is_footnote_definition_block(&blocks[index])
            && output.last().is_some_and(|previous| {
                should_join_around_interrupting_footnote(previous, &blocks[index + 1])
            })
        {
            let footnote = blocks[index].clone();
            let continuation_block = blocks[index + 1].clone();
            if let Some(previous) = output.pop() {
                output.push(continuation::join_interrupted_paragraph(
                    &previous,
                    &continuation_block,
                ));
                output.push(footnote);
            }
            index += 2;
            continue;
        }
        output.push(blocks[index].clone());
        index += 1;
    }
    output.join("\n\n")
}

fn should_join_around_interrupting_footnote(previous: &str, continuation_block: &str) -> bool {
    !continuation::paragraph_looks_complete(previous)
        && continuation::looks_like_paragraph_continuation(continuation_block)
}

fn is_footnote_definition_block(block: &str) -> bool {
    let trimmed = block.trim_start();
    let Some(rest) = trimmed.strip_prefix("$ ^{") else {
        return trimmed
            .strip_prefix("$^{")
            .and_then(|rest| rest.split_once("}$"))
            .is_some_and(|(number, tail)| {
                !number.is_empty()
                    && number.chars().all(|ch| ch.is_ascii_digit())
                    && !tail.trim().is_empty()
            });
    };
    rest.split_once("} $").is_some_and(|(number, tail)| {
        !number.is_empty()
            && number.chars().all(|ch| ch.is_ascii_digit())
            && !tail.trim().is_empty()
    })
}

fn clean_pdf_license_footer_fragments(block: &str) -> String {
    block
        .lines()
        .filter(|line| !looks_like_pdf_license_footer_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_like_pdf_license_footer_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    (lower.contains("the author(s)") || lower.contains("the authors"))
        && lower.contains("published by")
        && (lower.contains("creative commons") || lower.contains("open access"))
}

fn strip_layout_page_chrome_blocks(markdown: &str, page_layouts: &[OcrPageLayout]) -> String {
    if page_layouts.is_empty() {
        return markdown.to_string();
    }
    let exact_chrome = layout_page_chrome_set(page_layouts);
    split_markdown_blocks_preserve_order(markdown)
        .into_iter()
        .map(clean_inline_pdf_page_chrome_fragments)
        .filter(|block| {
            let normalized = normalize_section_title_key(block);
            !exact_chrome.contains(&normalized) && !looks_like_pdf_page_chrome_block(block)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn insert_missing_layout_headings(markdown: &str, page_layouts: &[OcrPageLayout]) -> String {
    let plans = missing_heading_insertion_plans(page_layouts);
    if plans.is_empty() {
        return markdown.to_string();
    }
    let existing_title_keys = split_markdown_blocks_preserve_order(markdown)
        .into_iter()
        .filter(|block| title_should_be_markdown_heading(block))
        .map(|block| normalize_section_title_key(&block))
        .collect::<std::collections::HashSet<_>>();
    let mut inserted = std::collections::HashSet::new();
    let mut output = Vec::new();
    let blocks = split_markdown_blocks_preserve_order(markdown);
    for (index, block) in blocks.iter().enumerate() {
        if let Some(plan) = plans.iter().find(|plan| {
            !inserted.contains(&plan.title_key)
                && !existing_title_keys.contains(&plan.title_key)
                && previous_block_is_not_heading(&blocks, index, &plan.title)
                && block_matches_heading_insertion_anchor(block, &plan.anchor)
        }) {
            output.push(format!("## {}", plan.title));
            inserted.insert(plan.title_key.clone());
        }
        output.push(block.clone());
    }
    output.join("\n\n")
}

#[derive(Clone)]
struct MissingHeadingInsertionPlan {
    title: String,
    title_key: String,
    anchor: String,
}

fn missing_heading_insertion_plans(
    page_layouts: &[OcrPageLayout],
) -> Vec<MissingHeadingInsertionPlan> {
    let mut plans = Vec::new();
    for page in page_layouts {
        for pair in page.blocks.windows(2) {
            let title = pair[0].content.trim();
            if !pair[0].label.eq_ignore_ascii_case("paragraph_title")
                || !missing_heading_can_be_inserted(title)
            {
                continue;
            }
            let anchor = pair[1].content.trim();
            if anchor.is_empty() {
                continue;
            }
            plans.push(MissingHeadingInsertionPlan {
                title: title.to_string(),
                title_key: normalize_section_title_key(title),
                anchor: anchor.chars().take(120).collect(),
            });
        }
    }
    plans
}

fn missing_heading_can_be_inserted(title: &str) -> bool {
    matches!(
        normalize_section_title_key(title).as_str(),
        "abstract" | "keywords"
    )
}

fn previous_block_is_not_heading(blocks: &[String], index: usize, title: &str) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|value| blocks.get(value)) else {
        return true;
    };
    !previous.trim().starts_with('#')
        || normalize_section_title_key(previous) != normalize_section_title_key(title)
}

fn block_matches_heading_insertion_anchor(block: &str, anchor: &str) -> bool {
    let block = block.split_whitespace().collect::<Vec<_>>().join(" ");
    let anchor = anchor.split_whitespace().collect::<Vec<_>>().join(" ");
    !anchor.is_empty() && (block.starts_with(&anchor) || anchor.starts_with(&block))
}

fn clean_inline_pdf_page_chrome_fragments(block: String) -> String {
    if !block.to_ascii_lowercase().contains("published online by") {
        return block;
    }
    let cleaned = remove_published_online_footer_fragments(&block);
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_published_online_footer_fragments(block: &str) -> String {
    let mut output = block.to_string();
    while let Some(start) = output.to_ascii_lowercase().find("https://doi.org/") {
        let tail = &output[start..];
        let tail_lower = tail.to_ascii_lowercase();
        let Some(marker_offset) = tail_lower.find(" published online by ") else {
            break;
        };
        let end = tail_lower
            .find("cambridge university press")
            .map(|offset| offset + "cambridge university press".len())
            .unwrap_or(marker_offset + " published online by ".len());
        output.replace_range(start..start + end, "");
    }
    output
}

fn layout_page_chrome_set(page_layouts: &[OcrPageLayout]) -> std::collections::HashSet<String> {
    let mut chrome = std::collections::HashSet::new();
    for page in page_layouts {
        for block in &page.blocks {
            if block.label.eq_ignore_ascii_case("footer")
                || block.label.eq_ignore_ascii_case("number")
                || (block.page.unwrap_or_default() > 0
                    && block.label.eq_ignore_ascii_case("header"))
            {
                let text = normalize_section_title_key(&block.content);
                if !text.is_empty() {
                    chrome.insert(text);
                }
            }
        }
        insert_running_header_number_pairs(page, &mut chrome);
    }
    chrome
}

fn insert_running_header_number_pairs(
    page: &OcrPageLayout,
    chrome: &mut std::collections::HashSet<String>,
) {
    let headers = page
        .blocks
        .iter()
        .filter(|block| block.label.eq_ignore_ascii_case("header"))
        .map(|block| block.content.trim())
        .filter(|content| !content.is_empty());
    let numbers = page
        .blocks
        .iter()
        .filter(|block| block.label.eq_ignore_ascii_case("number"))
        .map(|block| block.content.trim())
        .filter(|content| content.chars().all(|ch| ch.is_ascii_digit()));

    let numbers = numbers.collect::<Vec<_>>();
    for header in headers {
        for number in &numbers {
            chrome.insert(normalize_section_title_key(&format!("{header} {number}")));
            chrome.insert(normalize_section_title_key(&format!("{number} {header}")));
        }
    }
}

fn looks_like_pdf_page_chrome_block(block: &str) -> bool {
    let normalized = block.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("published online by") && lower.starts_with("http") {
        return true;
    }
    if normalized.chars().count() > 90 {
        return false;
    }
    let mut chars = normalized.chars().peekable();
    let mut digit_count = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digit_count += 1;
        chars.next();
    }
    digit_count > 0
        && chars.peek().is_some_and(|ch| ch.is_whitespace())
        && normalized[digit_count..].trim().split_whitespace().count() >= 2
        || looks_like_running_header_with_trailing_page_number(&normalized)
}

fn looks_like_running_header_with_trailing_page_number(normalized: &str) -> bool {
    if normalized.chars().count() > 90 {
        return false;
    }
    let Some((prefix, suffix)) = normalized.rsplit_once(' ') else {
        return false;
    };
    suffix.chars().all(|ch| ch.is_ascii_digit())
        && prefix.split_whitespace().count() >= 2
        && prefix
            .split_whitespace()
            .all(|word| word.chars().next().is_some_and(|ch| ch.is_uppercase()))
        && !prefix.ends_with('.')
        && !prefix.ends_with('?')
        && !prefix.ends_with('!')
}

fn drop_near_duplicate_following_blocks(markdown: &str) -> String {
    let blocks = split_markdown_blocks_preserve_order(markdown);
    let mut output: Vec<String> = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Some(previous) = output.last() {
            match trim_following_block_overlap(previous, &block) {
                OverlapAction::Drop => continue,
                OverlapAction::Trimmed(trimmed) => {
                    output.push(trimmed);
                    continue;
                }
                OverlapAction::Keep => {}
            }
        }
        output.push(block);
    }
    output.join("\n\n")
}

enum OverlapAction {
    Drop,
    Trimmed(String),
    Keep,
}

fn trim_following_block_overlap(previous: &str, following: &str) -> OverlapAction {
    let following_key = normalized_overlap_key(following);
    if following_key.chars().count() < 80 {
        return OverlapAction::Keep;
    }
    let previous_key = normalized_overlap_key(previous);
    if previous_key.contains(&following_key) {
        return OverlapAction::Drop;
    }

    let Some(trimmed) = trim_repeated_prefix_by_sentence(previous, following) else {
        return OverlapAction::Keep;
    };
    if normalized_overlap_key(&trimmed).chars().count() < 40 {
        OverlapAction::Drop
    } else {
        OverlapAction::Trimmed(trimmed)
    }
}

fn normalized_overlap_key(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn trim_repeated_prefix_by_sentence(previous: &str, following: &str) -> Option<String> {
    let following = following.trim();
    for boundary in sentence_boundary_indices(following).into_iter().rev() {
        let prefix = &following[..boundary];
        if normalized_overlap_key(prefix).chars().count() < 80 {
            continue;
        }
        if normalized_overlap_key(previous).contains(&normalized_overlap_key(prefix)) {
            let tail = following[boundary..].trim_start();
            return Some(tail.to_string());
        }
    }
    None
}

fn sentence_boundary_indices(text: &str) -> Vec<usize> {
    let mut indices = Vec::new();
    for (index, ch) in text.char_indices() {
        if matches!(ch, '.' | '?' | '!') {
            let next = index + ch.len_utf8();
            if text[next..]
                .chars()
                .next()
                .map_or(true, |next_ch| next_ch.is_whitespace())
            {
                indices.push(next);
            }
        }
    }
    indices
}

fn strengthen_pdf_section_headings(markdown: &str, page_layouts: &[OcrPageLayout]) -> String {
    if page_layouts.is_empty() {
        return markdown.to_string();
    }
    let title_set = layout_paragraph_title_set(page_layouts);
    if title_set.is_empty() {
        return split_tail_heading_paragraphs(markdown);
    }

    let blocks = split_markdown_blocks_preserve_order(markdown);
    let output = blocks
        .into_iter()
        .flat_map(|block| strengthen_pdf_section_block(&block, &title_set))
        .collect::<Vec<_>>();
    split_tail_heading_paragraphs(&output.join("\n\n"))
}

fn strengthen_pdf_section_block(
    block: &str,
    title_set: &std::collections::HashSet<String>,
) -> Vec<String> {
    let trimmed = block.trim();
    if trimmed.starts_with('#')
        || trimmed.starts_with('>')
        || trimmed.starts_with("![")
        || table::is_pdf_table_block(trimmed)
    {
        return vec![block.to_string()];
    }
    let normalized = normalize_section_title_key(trimmed);
    if title_set.contains(&normalized) && title_should_be_markdown_heading(trimmed) {
        return vec![format!("## {trimmed}")];
    }
    vec![block.to_string()]
}

fn split_tail_heading_paragraphs(markdown: &str) -> String {
    split_markdown_blocks_preserve_order(markdown)
        .into_iter()
        .flat_map(split_tail_heading_paragraph_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn split_tail_heading_paragraph_block(block: String) -> Vec<String> {
    let trimmed = block.trim();
    if trimmed.starts_with('#') || trimmed.starts_with("![") || table::is_pdf_table_block(trimmed) {
        return vec![block];
    }
    let Some((heading, body)) = tail_heading_prefix(trimmed) else {
        return vec![block];
    };
    if body.trim().is_empty() {
        return vec![format!("## {heading}")];
    }
    vec![format!("## {heading}"), body.trim().to_string()]
}

fn tail_heading_prefix(block: &str) -> Option<(&'static str, &str)> {
    let lower = block.to_ascii_lowercase();
    for heading in [
        "Acknowledgments",
        "Acknowledgements",
        "Funding",
        "Conflict of interest",
    ] {
        let key = format!("{}.", heading.to_ascii_lowercase());
        if lower.starts_with(&key) {
            return Some((heading, block[key.len()..].trim_start()));
        }
        let key = format!("{}:", heading.to_ascii_lowercase());
        if lower.starts_with(&key) {
            return Some((heading, block[key.len()..].trim_start()));
        }
    }
    None
}

fn layout_paragraph_title_set(page_layouts: &[OcrPageLayout]) -> std::collections::HashSet<String> {
    page_layouts
        .iter()
        .flat_map(|page| page.blocks.iter())
        .filter(|block| block.label.eq_ignore_ascii_case("paragraph_title"))
        .map(|block| normalize_section_title_key(&block.content))
        .filter(|title| !title.is_empty())
        .collect()
}

fn normalize_section_title_key(text: &str) -> String {
    text.trim()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn title_should_be_markdown_heading(title: &str) -> bool {
    let normalized = normalize_section_title_key(title);
    if normalized.is_empty() || normalized.len() > 120 {
        return false;
    }
    if normalized.starts_with("figure ") || normalized.starts_with("table ") {
        return false;
    }
    true
}

fn rejoin_paragraph_around_media(
    output: &mut Vec<String>,
    blocks: &[String],
    index: usize,
    consumed: usize,
    media_block: String,
) {
    let previous = output.pop().unwrap_or_default();
    let next = blocks[index + consumed].clone();
    output.push(continuation::join_interrupted_paragraph(&previous, &next));
    output.push(media_block);
}

fn should_rejoin_paragraph_around_media(
    output: &[String],
    blocks: &[String],
    next_index: usize,
) -> bool {
    let Some(previous) = output.last() else {
        return false;
    };
    let Some(next) = blocks.get(next_index) else {
        return false;
    };
    !continuation::paragraph_looks_complete(previous)
        && continuation_is_not_media(next)
        && continuation::looks_like_paragraph_continuation(next)
}

fn media_can_stay_in_place(output: &[String]) -> bool {
    output
        .last()
        .is_none_or(|previous| continuation::paragraph_looks_complete(previous))
}

fn continuation_is_not_media(block: &str) -> bool {
    let trimmed = block.trim();
    !media::is_figure_or_table_label(trimmed)
        && media::parse_markdown_image(trimmed).is_none()
        && !table::is_pdf_table_block(trimmed)
        && !is_figure_caption_block(trimmed)
        && !trimmed.starts_with("## ")
}

fn next_block_is_structural_media_boundary(blocks: &[String], next_index: usize) -> bool {
    blocks
        .get(next_index)
        .is_none_or(|block| !continuation_is_not_media(block))
}

fn is_figure_caption_block(block: &str) -> bool {
    block.trim_start().starts_with("::: figure-caption")
}

fn split_markdown_blocks_preserve_order(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn collect_media_block(blocks: &[String], start: usize) -> Option<(String, usize)> {
    let label = blocks.get(start)?.trim();
    if label.starts_with("## ") && start + 1 < blocks.len() {
        let next = blocks[start + 1].trim();
        if media::is_figure_or_table_label(next)
            || media::parse_markdown_image(next).is_some()
            || table::is_pdf_table_block(next)
        {
            let (media_block, consumed) = collect_media_block(blocks, start + 1)?;
            return Some((format!("{}\n\n{}", label, media_block), consumed + 1));
        }
    }
    if media::parse_markdown_image(label).is_some() {
        let mut consumed = 1usize;
        let mut parts = vec![label.to_string()];
        while start + consumed < blocks.len() && consumed < 3 {
            let block = blocks[start + consumed].trim();
            if !is_figure_caption_block(block) {
                break;
            }
            parts.push(block.to_string());
            consumed += 1;
        }
        return Some((parts.join("\n\n"), consumed));
    }
    if table::is_pdf_table_block(label) || is_figure_caption_block(label) {
        return Some((label.to_string(), 1));
    }
    if !media::is_figure_or_table_label(label) {
        return None;
    }
    let mut consumed = 1usize;
    let mut parts = vec![media::normalize_figure_table_label(label).to_string()];
    let mut saw_image = false;
    while start + consumed < blocks.len() && consumed < 8 {
        let block = blocks[start + consumed].trim();
        if block.starts_with("# ")
            || block.starts_with("## ")
            || media::is_figure_or_table_label(block)
        {
            break;
        }
        if is_figure_caption_block(block) {
            parts.push(block.to_string());
            consumed += 1;
            break;
        }
        if media::parse_markdown_image(block).is_some() {
            parts.push(block.to_string());
            saw_image = true;
            consumed += 1;
            continue;
        }
        if consumed == 1 || saw_image {
            parts.push(block.to_string());
            consumed += 1;
        }
        break;
    }
    Some((parts.join("\n\n"), consumed))
}

fn is_table_continuation_noise(block: &str) -> bool {
    let normalized = block
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    normalized.contains("continued from previous page")
        || normalized == "- 0.000000"
        || normalized == "0.000000"
}

fn is_duplicate_table_title(previous: &str, current: &str) -> bool {
    let previous = previous.trim();
    let current = current.trim();
    !previous.is_empty()
        && previous == current
        && previous.starts_with("Table ")
        && previous.contains('|')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_block_keeps_adjacent_split_images_under_one_label() {
        let markdown = "Before.\n\nFigure 2:\n\n![panel a](imgs/fig2_a.png)\n\n![panel b](imgs/fig2_b.png)\n\nComposite caption.\n\nAfter.";

        let normalized = move_interrupted_figure_blocks(markdown);

        assert!(normalized.contains(
            "![panel a](imgs/fig2_a.png)\n\n![panel b](imgs/fig2_b.png)\n\nComposite caption."
        ));
        assert!(normalized.contains("After."));
    }

    #[test]
    fn media_block_stops_before_next_media_label() {
        let markdown = "Figure 1:\n\n![one](imgs/fig1.png)\n\nFigure 2:\n\n![two](imgs/fig2.png)";

        let normalized = move_interrupted_figure_blocks(markdown);

        assert!(normalized.contains("![one](imgs/fig1.png)"));
        assert!(normalized.contains("![two](imgs/fig2.png)"));
        assert!(
            normalized.find("![one](imgs/fig1.png)") < normalized.find("![two](imgs/fig2.png)")
        );
    }

    #[test]
    fn standalone_media_moves_after_rejoined_paragraph_continuation() {
        let markdown = "We examined models predicting the effects of\n\n![PDF figure cluster](imgs/figure_cluster_002.jpg)\n\nimprecision and polarization predictors on confidence.";

        let normalized = move_interrupted_figure_blocks(markdown);

        assert!(normalized.contains(
            "We examined models predicting the effects of imprecision and polarization predictors on confidence.\n\n![PDF figure cluster](imgs/figure_cluster_002.jpg)"
        ));
    }

    #[test]
    fn standalone_media_stays_when_previous_paragraph_is_complete() {
        let markdown = "All analyzed data were documented.\n\n![PDF figure cluster](imgs/figure_cluster_001.jpg)\n\nwas higher for true news.";

        let normalized = move_interrupted_figure_blocks(markdown);

        assert!(normalized.contains(
            "All analyzed data were documented.\n\n![PDF figure cluster](imgs/figure_cluster_001.jpg)\n\nwas higher for true news."
        ));
    }

    #[test]
    fn pdf_figure_caption_stays_with_image_before_structural_boundary() {
        let table = format!(
            "{PDF_TABLE_START_MARKER}\n![PDF table 2](imgs/table_snapshot_002.jpg)\n{PDF_TABLE_END_MARKER}"
        );
        let markdown = format!(
            "Boosting interventions are transparent. In\n\n\
             ![PDF figure 1](imgs/figure_snapshot_001.jpg)\n\n\
             ::: figure-caption\nFig. 1 Structure of the toolbox.\n:::\n\n\
             {table}\n\n\
             ## Refutation Strategies"
        );

        let normalized = move_interrupted_figure_blocks(&markdown);

        let image_pos = normalized.find("![PDF figure 1]").unwrap_or_default();
        let caption_pos = normalized.find("::: figure-caption").unwrap_or_default();
        let table_pos = normalized.find("![PDF table 2]").unwrap_or_default();
        assert!(image_pos < caption_pos);
        assert!(caption_pos < table_pos);
        assert!(!normalized.contains("In ::: figure-caption"));
    }

    #[test]
    fn pdf_figure_caption_moves_with_image_after_rejoined_continuation() {
        let markdown = concat!(
            "Boosting interventions are transparent. In\n\n",
            "![PDF figure 1](imgs/figure_snapshot_001.jpg)\n\n",
            "::: figure-caption\n",
            "Fig. 1 Structure of the toolbox.\n",
            ":::\n\n",
            "online environments, boosts can foster digital competences."
        );

        let normalized = move_interrupted_figure_blocks(markdown);

        assert!(normalized.contains(
            "Boosting interventions are transparent. In online environments, boosts can foster digital competences.\n\n![PDF figure 1](imgs/figure_snapshot_001.jpg)\n\n::: figure-caption"
        ));
        assert!(!normalized.contains("In ::: figure-caption"));
    }

    #[test]
    fn strips_check_for_updates_from_top_pdf_front_matter() {
        let markdown = concat!(
            "https://doi.org/example\n\n",
            "# Article Title\n\n",
            "Check for updates\n\n",
            "Author One $ ^{1} $\n\n",
            "This is a long enough opening abstract paragraph with punctuation. It should count as the first body-like block because it contains many words and completes a full sentence for the detector to stop stripping notices."
        );

        let normalized = strip_top_update_notice_blocks(markdown);

        assert!(!normalized.contains("Check for updates"));
        assert!(normalized.contains("# Article Title"));
        assert!(normalized.contains("Author One $ ^{1} $"));
    }

    #[test]
    fn cleanup_removes_duplicate_table_titles_and_continuation_noise() {
        let markdown = concat!(
            "Table 1 | Overview of intervention types in the toolbox.\n\n",
            "Table 1 | Overview of intervention types in the toolbox.\n\n",
            "- 0.000000\n\n",
            "Table 1 | continued from previous page\n\n",
            "<!-- musetranslate:pdf-table:start -->\n![PDF table 1](imgs/table_snapshot_001.jpg)\n<!-- musetranslate:pdf-table:end -->"
        );

        let normalized = cleanup_pdf_structure_noise(markdown);

        assert_eq!(
            normalized
                .matches("Table 1 | Overview of intervention types in the toolbox.")
                .count(),
            1
        );
        assert!(!normalized.contains("continued from previous page"));
        assert!(!normalized.contains("0.000000"));
    }

    #[test]
    fn rejoins_parenthetical_reference_breaks_by_structure() {
        let markdown = concat!(
            "This prior should be rerepresented (see\n\n",
            "Kastrup, 2017). However, the policy implication remains contested."
        );

        let normalized = rejoin_parenthetical_reference_breaks(markdown);

        assert!(
            normalized.contains("This prior should be rerepresented (see Kastrup, 2017). However")
        );
        assert!(!normalized.contains("(see\n\nKastrup"));
    }

    #[test]
    fn relocates_footnote_definition_out_of_interrupted_paragraph() {
        let markdown = concat!(
            "The same reflective logic can be extended to other nudges\n\n",
            "$ ^{1} $The theory of ecological rationality specifies norms for bounded agents.\n\n",
            "to increase transparency; for example, pledges can make defaults more explicit."
        );

        let normalized = relocate_interrupting_footnotes(markdown);

        assert!(normalized.contains(
            "The same reflective logic can be extended to other nudges to increase transparency; for example"
        ));
        assert!(normalized.contains("explicit.\n\n$ ^{1} $The theory of ecological rationality"));
    }

    #[test]
    fn promotes_layout_paragraph_titles_to_markdown_headings() {
        let markdown = concat!(
            "# Article Title\n\n",
            "From nudge to nudge plus\n\n",
            "Body paragraph.\n\n",
            "References\n\n",
            "Ref A."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                ocr_block("paragraph_title", "From nudge to nudge plus"),
                ocr_block("paragraph_title", "References"),
            ],
        }];

        let normalized = strengthen_pdf_section_headings(markdown, &layouts);

        assert!(normalized.contains("## From nudge to nudge plus"));
        assert!(normalized.contains("## References"));
    }

    #[test]
    fn cleanup_removes_pdf_license_footer_without_dropping_metadata() {
        let markdown = concat!(
            "Behavioural Public Policy (2024), 8, 69-84\n",
            "doi:10.1017/bpp.2021.6\n",
            "(Received 6 May 2020; accepted 10 February 2021)\n",
            "© The Author(s), 2021. Published by Cambridge University Press. This is an Open Access article, distributed under the terms of the Creative Commons Attribution licence.\n\n",
            "Main body paragraph."
        );

        let normalized = cleanup_pdf_structure_noise(markdown);

        assert!(normalized.contains("Behavioural Public Policy (2024)"));
        assert!(normalized.contains("doi:10.1017/bpp.2021.6"));
        assert!(!normalized.contains("Creative Commons"));
        assert!(normalized.contains("Main body paragraph."));
    }

    #[test]
    fn splits_acknowledgments_prefix_into_tail_heading() {
        let markdown = concat!(
            "Conclusion body.\n\n",
            "Acknowledgments. We thank the project team.\n\n",
            "European Studies seminar comments.\n\n",
            "References\n\n",
            "Ref A."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![ocr_block("paragraph_title", "References")],
        }];

        let normalized = strengthen_pdf_section_headings(markdown, &layouts);

        assert!(normalized.contains("## Acknowledgments\n\nWe thank the project team."));
        assert!(normalized.contains("## References"));
    }

    #[test]
    fn strips_layout_page_chrome_without_removing_body() {
        let markdown = concat!(
            "Conclusion body.\n\n",
            "https://doi.org/10.1017/example Published online by Cambridge University Press\n\n",
            "82 Sanchayan Banerjee and Peter John\n\n",
            "European Studies seminar comments.\n\n",
            "References"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                ocr_block("footer", "https://doi.org/10.1017/example Published online by Cambridge University Press"),
                ocr_block("header", "Sanchayan Banerjee and Peter John"),
                ocr_block("number", "82"),
            ],
        }];

        let normalized = strip_layout_page_chrome_blocks(markdown, &layouts);

        assert!(!normalized.contains("Published online by"));
        assert!(!normalized.contains("82 Sanchayan"));
        assert!(normalized.contains("European Studies seminar comments."));
    }

    #[test]
    fn strips_running_header_with_trailing_page_number_from_layout_pair() {
        let markdown = concat!(
            "Body paragraph before header.\n\n",
            "Behavioural Public Policy 71\n\n",
            "## From nudge to nudge plus"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                ocr_block("header", "Behavioural Public Policy"),
                ocr_block("number", "71"),
            ],
        }];

        let normalized = strip_layout_page_chrome_blocks(markdown, &layouts);

        assert!(!normalized.contains("Behavioural Public Policy 71"));
        assert!(normalized.contains("Body paragraph before header."));
        assert!(normalized.contains("## From nudge to nudge plus"));
    }

    #[test]
    fn strips_inline_published_online_footer_fragment() {
        let markdown = concat!(
            "The claim remains debated (see https://doi.org/10.1017/example ",
            "Published online by Cambridge University Press De Neys, 2012)."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![ocr_block(
                "footer",
                "https://doi.org/10.1017/example Published online by Cambridge University Press",
            )],
        }];

        let normalized = strip_layout_page_chrome_blocks(markdown, &layouts);

        assert!(!normalized.contains("Published online by"));
        assert!(!normalized.contains("https://doi.org/10.1017/example"));
        assert!(normalized.contains("The claim remains debated (see De Neys, 2012)."));
    }

    #[test]
    fn inserts_missing_abstract_heading_from_layout_anchor() {
        let markdown = concat!(
            "# Article Title\n\n",
            "This study examines a policy intervention in detail.\n\n",
            "Keywords: policy; behavior"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                ocr_block("paragraph_title", "Abstract"),
                ocr_block(
                    "abstract",
                    "This study examines a policy intervention in detail.",
                ),
            ],
        }];

        let normalized = insert_missing_layout_headings(markdown, &layouts);

        assert!(normalized.contains("## Abstract\n\nThis study examines"));
        assert_eq!(normalized.matches("## Abstract").count(), 1);
    }

    #[test]
    fn does_not_duplicate_existing_abstract_heading() {
        let markdown = concat!(
            "# Article Title\n\n",
            "## Abstract\n\n",
            "This study examines a policy intervention in detail."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                ocr_block("paragraph_title", "Abstract"),
                ocr_block(
                    "abstract",
                    "This study examines a policy intervention in detail.",
                ),
            ],
        }];

        let normalized = insert_missing_layout_headings(markdown, &layouts);

        assert_eq!(normalized.matches("## Abstract").count(), 1);
    }

    #[test]
    fn does_not_insert_abstract_when_bare_title_exists() {
        let markdown = concat!(
            "# Article Title\n\n",
            "Abstract\n\n",
            "This study examines a policy intervention in detail."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                ocr_block("paragraph_title", "Abstract"),
                ocr_block(
                    "abstract",
                    "This study examines a policy intervention in detail.",
                ),
            ],
        }];

        let normalized = insert_missing_layout_headings(markdown, &layouts);

        assert_eq!(normalized.matches("Abstract").count(), 1);
    }

    #[test]
    fn trims_repeated_following_prefix_without_dropping_new_tail() {
        let markdown = concat!(
            "Most fitness trackers prompt the user about their activity level on a daily or weekly basis. ",
            "This prompt can be enhanced to include a reflective element by adding the option of setting up future fitness goals. ",
            "Additional prompts could be built in to engage the user.\n\n",
            "This prompt can be enhanced to include a reflective element by adding the option of setting up future fitness goals. ",
            "Additional prompts could be built in to engage the user. ",
            "The application of the dual self-pledge device can be conveniently delivered in online contexts."
        );

        let normalized = drop_near_duplicate_following_blocks(markdown);

        assert_eq!(normalized.matches("This prompt can be enhanced").count(), 1);
        assert!(normalized.contains(
            "The application of the dual self-pledge device can be conveniently delivered"
        ));
    }

    #[test]
    fn heading_plus_media_block_can_move_after_rejoined_paragraph() {
        let markdown = concat!(
            "Boosting is a behavioral policy approach that enlists human cognition. In\n\n",
            "## Refutation Strategies\n\n",
            "Figure 1\n\n",
            "![PDF figure cluster](imgs/figure_cluster_001.jpg)\n\n",
            "::: figure-caption\n",
            "Structure of the toolbox and map of evidence.\n",
            ":::\n\n",
            "online environments, boosts and other educational interventions aim to foster digital competences."
        );

        let normalized = move_interrupted_figure_blocks(markdown);

        assert!(normalized.contains(
            "Boosting is a behavioral policy approach that enlists human cognition. In online environments, boosts and other educational interventions aim to foster digital competences.\n\n## Refutation Strategies"
        ));
    }

    fn ocr_block(label: &str, content: &str) -> crate::ocr::OcrLayoutBlock {
        crate::ocr::OcrLayoutBlock {
            label: label.to_string(),
            content: content.to_string(),
            bbox: Vec::new(),
            order: None,
            page: Some(0),
        }
    }

    #[test]
    fn move_interrupted_figure_blocks_keeps_pdf_tables_in_place() {
        let markdown = concat!(
            "Various media and science literacy studies: To\n\n",
            "## A.2 Co-authors and Their Areas of Expertise\n\n",
            "Table A1: Co-authors\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 5](imgs/table_snapshot_005.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n\n",
            "Residual row text\n\n",
            "## References\n"
        );

        let output = move_interrupted_figure_blocks(markdown);

        let table_pos = output
            .find("![PDF table 5](imgs/table_snapshot_005.jpg)")
            .unwrap();
        let a2_pos = output
            .find("## A.2 Co-authors and Their Areas of Expertise")
            .unwrap();
        let references_pos = output.find("## References").unwrap();
        assert!(a2_pos < table_pos);
        assert!(table_pos < references_pos);
    }
}
