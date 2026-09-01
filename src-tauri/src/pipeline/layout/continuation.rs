use crate::ocr::{OcrLayoutBlock, OcrPageLayout};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
struct ContinuationJoin {
    continuation: String,
    strip_terminal_period: bool,
}

pub(super) fn normalize_with_layout(markdown: &str, page_layouts: &[OcrPageLayout]) -> String {
    let mut output_lines: Vec<String> = Vec::new();
    let mut line_iter = markdown.lines().peekable();
    let interrupted_continuations = collect_interrupted_figure_continuations(page_layouts);
    let cross_page_continuations = collect_cross_page_continuations(page_layouts);
    let direct_text_continuations = collect_direct_text_continuations(page_layouts);
    let mut pending_skip_continuation_keys = HashSet::new();

    while let Some(line) = line_iter.next() {
        let trimmed = line.trim();
        let line_key = continuation_match_key(trimmed);
        if let Some(continuation) = direct_text_continuations.get(&line_key) {
            output_lines.push(join_paragraph_with_mode(trimmed, continuation));
            pending_skip_continuation_keys
                .insert(continuation_match_key(&continuation.continuation));
            continue;
        }
        if pending_skip_continuation_keys.remove(&line_key) {
            skip_following_blank_lines(&mut line_iter);
            continue;
        }
        if let Some(continuation) = cross_page_continuations.get(&line_key) {
            output_lines.push(join_paragraph_with_mode(trimmed, continuation));
            pending_skip_continuation_keys
                .insert(continuation_match_key(&continuation.continuation));
            continue;
        }
        if pending_skip_continuation_keys.remove(&line_key) {
            skip_following_blank_lines(&mut line_iter);
            continue;
        }
        if interrupted_continuations.contains(trimmed) {
            merge_interrupted_line(&mut output_lines, trimmed);
            skip_following_blank_lines(&mut line_iter);
            continue;
        }
        output_lines.push(line.to_string());
    }

    output_lines.join("\n")
}

fn continuation_match_key(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn join_interrupted_paragraph(previous: &str, continuation: &str) -> String {
    format!("{} {}", previous.trim_end(), continuation.trim_start())
}

fn join_soft_break_paragraph(previous: &str, continuation: &str) -> String {
    let previous = previous.trim_end();
    let previous = previous.strip_suffix('.').unwrap_or(previous).trim_end();
    format!("{} {}", previous, continuation.trim_start())
}

fn join_paragraph_with_mode(previous: &str, continuation: &ContinuationJoin) -> String {
    if continuation.strip_terminal_period {
        join_soft_break_paragraph(previous, &continuation.continuation)
    } else {
        join_interrupted_paragraph(previous, &continuation.continuation)
    }
}

pub(super) fn looks_like_paragraph_continuation(block: &str) -> bool {
    let trimmed = block.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("![](") {
        return false;
    }
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    first.is_ascii_lowercase()
        || matches!(
            first,
            ')' | ']' | ',' | ';' | ':' | '.' | '-' | '，' | '；' | '：' | '）' | '】' | '》'
        )
}

pub(super) fn paragraph_looks_complete(block: &str) -> bool {
    let trimmed = block.trim();
    trimmed.starts_with('#')
        || trimmed.ends_with('.')
        || trimmed.ends_with('!')
        || trimmed.ends_with('?')
        || trimmed.ends_with('。')
        || trimmed.ends_with('！')
        || trimmed.ends_with('？')
        || trimmed.ends_with(']')
        || trimmed.ends_with(')')
}

fn merge_interrupted_line(output_lines: &mut Vec<String>, continuation: &str) {
    if let Some(previous) = output_lines.pop() {
        output_lines.push(join_interrupted_paragraph(&previous, continuation));
    } else {
        output_lines.push(continuation.to_string());
    }
}

fn collect_interrupted_figure_continuations(page_layouts: &[OcrPageLayout]) -> HashSet<String> {
    let mut continuations = HashSet::new();
    for page in page_layouts {
        collect_page_interrupted_continuations(&page.blocks, &mut continuations);
    }
    continuations
}

fn collect_cross_page_continuations(
    page_layouts: &[OcrPageLayout],
) -> HashMap<String, ContinuationJoin> {
    let mut continuations = HashMap::new();
    for pages in page_layouts.windows(2) {
        let previous = &pages[0].blocks;
        let next = &pages[1].blocks;
        let Some(continuation) = first_body_continuation_after_page_lead(next) else {
            continue;
        };
        let Some((previous_text, strip_terminal_period)) =
            last_cross_page_body_text(previous, &continuation)
        else {
            continue;
        };
        continuations.insert(
            continuation_match_key(&previous_text),
            ContinuationJoin {
                continuation,
                strip_terminal_period,
            },
        );
    }
    continuations
}

fn collect_direct_text_continuations(
    page_layouts: &[OcrPageLayout],
) -> HashMap<String, ContinuationJoin> {
    let mut continuations = HashMap::new();
    for page in page_layouts {
        for pair in page.blocks.windows(2) {
            let previous = &pair[0];
            let continuation = &pair[1];
            if !is_body_text_block(previous) || !is_body_text_block(continuation) {
                continue;
            }
            if !looks_like_direct_text_continuation(previous, continuation) {
                continue;
            }
            continuations.insert(
                continuation_match_key(previous.content.trim()),
                ContinuationJoin {
                    continuation: continuation.content.trim().to_string(),
                    strip_terminal_period: false,
                },
            );
        }
    }
    continuations
}

fn collect_page_interrupted_continuations(
    blocks: &[OcrLayoutBlock],
    continuations: &mut HashSet<String>,
) {
    let mut index = 0usize;
    while index < blocks.len() {
        if !block_can_start_media_interruption(&blocks[index]) {
            index += 1;
            continue;
        }
        let continuation_index = next_non_figure_block_index(blocks, index + 1);
        if let Some(text) = interrupted_continuation_text(blocks, index, continuation_index) {
            continuations.insert(text);
        }
        index = continuation_index;
    }
}

fn last_incomplete_body_text(blocks: &[OcrLayoutBlock]) -> Option<String> {
    blocks
        .iter()
        .rev()
        .find(|block| is_body_text_block(block) && !paragraph_looks_complete(&block.content))
        .map(|block| block.content.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn last_cross_page_body_text(
    blocks: &[OcrLayoutBlock],
    continuation: &str,
) -> Option<(String, bool)> {
    if let Some(text) = last_incomplete_body_text(blocks) {
        return Some((text, false));
    }
    let block = blocks
        .iter()
        .rev()
        .find(|block| is_body_text_block(block))?;
    soft_page_break_candidate(block, continuation).then(|| {
        (
            block.content.trim().to_string(),
            block.content.trim().ends_with('.'),
        )
    })
}

fn first_body_continuation_after_page_lead(blocks: &[OcrLayoutBlock]) -> Option<String> {
    blocks
        .iter()
        .filter(|block| !is_page_chrome_block(block))
        .find(|block| {
            is_body_text_block(block) && looks_like_paragraph_continuation(&block.content)
        })
        .map(|block| block.content.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn looks_like_direct_text_continuation(
    previous: &OcrLayoutBlock,
    continuation: &OcrLayoutBlock,
) -> bool {
    let previous_text = previous.content.trim();
    let continuation_text = continuation.content.trim();
    if previous_text.is_empty() || continuation_text.is_empty() {
        return false;
    }
    if !looks_like_paragraph_continuation(continuation_text) {
        return false;
    }
    if layout_order_is_reversed(previous, continuation) {
        return false;
    }
    if continuation.page.unwrap_or_default() != previous.page.unwrap_or_default() {
        return false;
    }
    let previous_len = previous_text.chars().count();
    previous_len >= 80
}

fn soft_page_break_candidate(previous: &OcrLayoutBlock, continuation: &str) -> bool {
    if !looks_like_paragraph_continuation(continuation) {
        return false;
    }
    let previous_text = previous.content.trim();
    if previous_text.is_empty() {
        return false;
    }
    let previous_len = previous_text.chars().count();
    let previous_bottom = bbox_bottom(previous).unwrap_or_default();
    previous_len >= 80 && previous_bottom >= 1300
}

fn block_can_start_media_interruption(block: &OcrLayoutBlock) -> bool {
    is_body_text_block(block) && !paragraph_looks_complete(&block.content)
}

fn next_non_figure_block_index(blocks: &[OcrLayoutBlock], start: usize) -> usize {
    let mut index = start;
    while blocks.get(index).is_some_and(is_layout_figure_block) {
        index += 1;
    }
    index
}

fn interrupted_continuation_text(
    blocks: &[OcrLayoutBlock],
    first_index: usize,
    continuation_index: usize,
) -> Option<String> {
    let first = blocks.get(first_index)?;
    let continuation = blocks.get(continuation_index)?;
    let figure_blocks = &blocks[first_index + 1..continuation_index];
    if !looks_like_paragraph_continuation(&continuation.content) {
        return None;
    }
    if !layout_transition_looks_like_media_break(first, figure_blocks, continuation) {
        return None;
    }
    Some(continuation.content.trim().to_string())
}

fn is_body_text_block(block: &OcrLayoutBlock) -> bool {
    matches!(block.label.as_str(), "text" | "abstract")
}

fn is_layout_figure_block(block: &OcrLayoutBlock) -> bool {
    matches!(
        block.label.as_str(),
        "image" | "figure_title" | "chart" | "table" | "vision_footnote"
    )
}

fn is_page_chrome_block(block: &OcrLayoutBlock) -> bool {
    matches!(block.label.as_str(), "header" | "footer" | "number") || is_layout_figure_block(block)
}

fn layout_transition_looks_like_media_break(
    previous: &OcrLayoutBlock,
    figure_blocks: &[OcrLayoutBlock],
    continuation: &OcrLayoutBlock,
) -> bool {
    if figure_blocks.is_empty() || !is_body_text_block(continuation) {
        return false;
    }
    if layout_order_is_reversed(previous, continuation) {
        return false;
    }
    media_position_allows_rejoin(previous, figure_blocks, continuation)
}

fn layout_order_is_reversed(previous: &OcrLayoutBlock, continuation: &OcrLayoutBlock) -> bool {
    let previous_page = previous.page.unwrap_or_default();
    let continuation_page = continuation.page.unwrap_or(previous_page);
    if continuation_page < previous_page {
        return true;
    }

    let previous_order = previous.order.unwrap_or_default();
    let continuation_order = continuation.order.unwrap_or(previous_order + 1);
    continuation_page == previous_page && continuation_order <= previous_order
}

fn media_position_allows_rejoin(
    previous: &OcrLayoutBlock,
    figure_blocks: &[OcrLayoutBlock],
    continuation: &OcrLayoutBlock,
) -> bool {
    let previous_page = previous.page.unwrap_or_default();
    let continuation_page = continuation.page.unwrap_or(previous_page);
    let previous_bottom = bbox_bottom(previous).unwrap_or_default();
    let figure_top = figure_blocks
        .iter()
        .filter_map(bbox_top)
        .min()
        .unwrap_or_default();
    let continuation_top = bbox_top(continuation).unwrap_or_default();

    continuation_page > previous_page
        || (figure_top >= previous_bottom && continuation_top >= figure_top)
}

fn bbox_top(block: &OcrLayoutBlock) -> Option<i32> {
    block.bbox.get(1).copied()
}

fn bbox_bottom(block: &OcrLayoutBlock) -> Option<i32> {
    block.bbox.get(3).copied()
}

fn skip_following_blank_lines<'a, I>(line_iter: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = &'a str>,
{
    while matches!(line_iter.peek(), Some(next) if next.trim().is_empty()) {
        line_iter.next();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejoins_cross_page_body_continuation_before_intervening_media() {
        let markdown = concat!(
            "Results were driven by both the imprecision and\n\n",
            "Finally, another paragraph.\n\n",
            "Figure 3\n\n",
            "![](imgs/fig3.jpg)\n\n",
            "the polarization of news content."
        );
        let layouts = vec![
            OcrPageLayout {
                blocks: vec![block(
                    "text",
                    "Results were driven by both the imprecision and",
                )],
            },
            OcrPageLayout {
                blocks: vec![
                    block("figure_title", "Fig. 3 | Caption."),
                    block("chart", ""),
                    block("text", "the polarization of news content."),
                    block("text", "Finally, another paragraph."),
                ],
            },
        ];

        let output = normalize_with_layout(markdown, &layouts);

        assert!(output.contains(
            "Results were driven by both the imprecision and the polarization of news content."
        ));
        assert_eq!(
            output.matches("the polarization of news content.").count(),
            1
        );
        assert!(output.contains("Finally, another paragraph."));
    }

    #[test]
    fn rejoins_same_page_lowercase_text_block_split() {
        let markdown = concat!(
            "Headlines in the real world often do not overtly appear true or false, but instead fall into an ambiguous gray area, which makes them more difficult to evaluate. Using a novel experimental design, we carefully selected nonpartisan and non-ego-relevant news that offer various levels of content.\n\n",
            "imprecision and polarization. The study was designed to address the complexity of evaluating ambiguous news."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block_with_page_order(
                    "text",
                    "Headlines in the real world often do not overtly appear true or false, but instead fall into an ambiguous gray area, which makes them more difficult to evaluate. Using a novel experimental design, we carefully selected nonpartisan and non-ego-relevant news that offer various levels of content.",
                    Some(7),
                    Some(5),
                ),
                block_with_page_order(
                    "text",
                    "imprecision and polarization. The study was designed to address the complexity of evaluating ambiguous news.",
                    Some(8),
                    Some(5),
                ),
            ],
        }];

        let output = normalize_with_layout(markdown, &layouts);

        assert!(output.contains(
            "offer various levels of content. imprecision and polarization. The study was designed to address the complexity of evaluating ambiguous news."
        ));
        assert_eq!(
            output
                .matches("imprecision and polarization. The study was designed")
                .count(),
            1
        );
    }

    #[test]
    fn rejoins_cross_page_soft_break_when_previous_ends_at_page_bottom() {
        let markdown = concat!(
            "Headlines in the real world often do not overtly appear true or false, but instead fall into an ambiguous gray area, which makes them more difficult to evaluate. Using a novel experimental design, we carefully selected nonpartisan and non-ego-relevant news that offer various levels of content.\n\n",
            "imprecision and polarization. The study was designed to address the complexity of evaluating ambiguous news."
        );
        let layouts = vec![
            OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "Headlines in the real world often do not overtly appear true or false, but instead fall into an ambiguous gray area, which makes them more difficult to evaluate. Using a novel experimental design, we carefully selected nonpartisan and non-ego-relevant news that offer various levels of content.".to_string(),
                    bbox: vec![606, 1401, 1127, 1489],
                    order: Some(8),
                    page: Some(7),
                }],
            },
            OcrPageLayout {
                blocks: vec![
                    OcrLayoutBlock {
                        label: "header".to_string(),
                        content: "Article".to_string(),
                        bbox: vec![1058, 46, 1126, 69],
                        order: None,
                        page: Some(8),
                    },
                    OcrLayoutBlock {
                        label: "text".to_string(),
                        content: "imprecision and polarization. The study was designed to address the complexity of evaluating ambiguous news.".to_string(),
                        bbox: vec![73, 95, 596, 438],
                        order: Some(1),
                        page: Some(8),
                    },
                ],
            },
        ];

        let output = normalize_with_layout(markdown, &layouts);

        assert!(output.contains(
            "offer various levels of content imprecision and polarization. The study was designed to address the complexity of evaluating ambiguous news."
        ));
        assert_eq!(
            output
                .matches("imprecision and polarization. The study was designed")
                .count(),
            1
        );
    }

    #[test]
    fn rejoins_cross_page_continuation_when_rendered_spacing_differs() {
        let markdown = concat!(
            "In addition, the sample contains field studies on social media $ [55] $. Field studies on\n\n",
            "social media thus represent a new paradigm."
        );
        let layouts = vec![
            OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "text".to_string(),
                    content:
                        "In addition, the sample contains field studies on social media  $ [55] $. Field studies on"
                            .to_string(),
                    bbox: vec![242, 871, 996, 1232],
                    order: Some(4),
                    page: Some(7),
                }],
            },
            OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "social media thus represent a new paradigm.".to_string(),
                    bbox: vec![193, 151, 944, 197],
                    order: Some(1),
                    page: Some(8),
                }],
            },
        ];

        let output = normalize_with_layout(markdown, &layouts);

        assert!(output.contains("Field studies on social media thus represent a new paradigm."));
        assert_eq!(
            output
                .matches("social media thus represent a new paradigm.")
                .count(),
            1
        );
    }

    fn block(label: &str, content: &str) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: label.to_string(),
            content: content.to_string(),
            bbox: Vec::new(),
            order: None,
            page: None,
        }
    }

    fn block_with_page_order(
        label: &str,
        content: &str,
        order: Option<i32>,
        page: Option<i32>,
    ) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: label.to_string(),
            content: content.to_string(),
            bbox: Vec::new(),
            order,
            page,
        }
    }
}
