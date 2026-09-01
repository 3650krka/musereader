use super::media::parse_markdown_image;
use super::table::{PDF_TABLE_END_MARKER, PDF_TABLE_START_MARKER};
use crate::ocr::{OcrLayoutBlock, OcrPageLayout};
use std::collections::HashSet;

const CAPTION_CONTINUATION_VERTICAL_GAP: i32 = 96;
const TABLE_NOTE_VERTICAL_WINDOW: i32 = 420;

pub(super) fn inject_layout_captions(markdown: &str, page_layouts: &[OcrPageLayout]) -> String {
    let markdown = collapse_duplicate_caption_blocks(markdown);
    let plans = build_caption_plans(page_layouts);
    if plans.0.is_empty() && plans.1.is_empty() {
        return markdown;
    }
    let mut state = CaptionInjectionState::new(plans);
    let blocks = markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;
    while index < blocks.len() {
        if let Some((replacement, consumed)) = state.consume_table(&blocks, index) {
            drop_equivalent_table_captions_before_block(&mut output, &replacement);
            output.push(replacement);
            index += consumed;
            continue;
        }
        if let Some((replacement, consumed)) = state.consume_figure(&blocks, index) {
            output.push(replacement);
            index += consumed;
            continue;
        }
        if state.should_drop_label_block(&blocks[index]) {
            index += 1;
            continue;
        }
        if state.should_drop_block(&blocks[index]) {
            index += 1;
            continue;
        }
        output.push(blocks[index].clone());
        index += 1;
    }
    output.join("\n\n")
}

fn collapse_duplicate_caption_blocks(markdown: &str) -> String {
    let blocks = markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>();
    let mut output = Vec::with_capacity(blocks.len());
    let mut previous_key = String::new();
    for block in blocks {
        let normalized = normalize_text(block);
        if !normalized.is_empty()
            && normalized == previous_key
            && (block.starts_with("::: figure-caption")
                || block.starts_with("Table ")
                || parse_standalone_caption_label(block).is_some())
        {
            continue;
        }
        previous_key = normalized;
        output.push(block.to_string());
    }
    output.join("\n\n")
}

fn drop_equivalent_table_captions_before_block(output: &mut Vec<String>, replacement: &str) {
    let Some(target) = table_caption_signature(replacement) else {
        return;
    };
    while output
        .last()
        .and_then(|block| table_caption_signature(block))
        .is_some_and(|previous| previous == target)
    {
        output.pop();
    }
}

fn table_caption_signature(block: &str) -> Option<(String, String)> {
    let caption = block.split("\n\n").next()?.trim();
    let rest = caption.strip_prefix("Table ")?;
    let number = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    if number.is_empty() {
        return None;
    }
    let body = rest
        .get(number.len()..)?
        .trim_start_matches(caption_separator_char)
        .trim();
    if body.is_empty() {
        return None;
    }
    Some((number, normalize_caption_signature_body(body)))
}

fn caption_separator_char(ch: char) -> bool {
    matches!(ch, ' ' | '|' | ':' | '.' | '-' | '–' | '—')
}

fn normalize_caption_signature_body(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

#[derive(Clone, Debug)]
struct CaptionPlan {
    number: String,
    label: String,
    body: String,
    notes: Vec<String>,
}

struct CaptionInjectionState {
    figures: Vec<CaptionPlan>,
    tables: Vec<CaptionPlan>,
    used_texts: HashSet<String>,
    used_labelled_texts: HashSet<String>,
}

impl CaptionInjectionState {
    fn new((figures, tables): (Vec<CaptionPlan>, Vec<CaptionPlan>)) -> Self {
        let used_texts = figures
            .iter()
            .chain(tables.iter())
            .flat_map(|plan| std::iter::once(&plan.body).chain(plan.notes.iter()))
            .map(|text| normalize_text(text))
            .collect();
        let used_labelled_texts = figures
            .iter()
            .chain(tables.iter())
            .map(|plan| normalize_text(&format!("{} {}", plan.label, plan.body)))
            .collect();
        Self {
            figures,
            tables,
            used_texts,
            used_labelled_texts,
        }
    }

    fn consume_table(&mut self, blocks: &[String], index: usize) -> Option<(String, usize)> {
        let block = blocks.get(index)?;
        if !(block.contains(PDF_TABLE_START_MARKER) && block.contains(PDF_TABLE_END_MARKER)) {
            return None;
        }
        let plan_index = matching_plan_index(&self.tables, "Table", blocks, index)?;
        let plan = self.tables.get(plan_index)?.clone();
        self.tables.remove(plan_index);
        Some((format_table_block(&plan, block), 1))
    }

    fn consume_figure(&mut self, blocks: &[String], index: usize) -> Option<(String, usize)> {
        let block = blocks.get(index)?;
        let (_, src) = parse_markdown_image(block)?;
        if !src.contains("figure_cluster")
            && !src.contains("img_in_image_box")
            && !src.contains("img_in_chart_box")
        {
            return None;
        }
        let plan_index = matching_plan_index(&self.figures, "Figure", blocks, index).unwrap_or(0);
        let plan = self.figures.get(plan_index)?.clone();
        self.figures.remove(plan_index);
        let (plan, consumed_tail) = consume_trailing_caption_fragments(plan, blocks, index + 1);
        Some((format_figure_block(&plan, block), consumed_tail + 1))
    }

    fn should_drop_block(&self, block: &str) -> bool {
        let normalized = normalize_text(block);
        if normalized.is_empty() {
            return false;
        }
        self.used_texts.contains(&normalized)
            || self.used_labelled_texts.contains(&normalized)
            || figure_caption_body(block)
                .is_some_and(|body| matches_used_caption_body(&self.used_texts, body))
            || quoted_note_body(block)
                .is_some_and(|body| matches_used_caption_body(&self.used_texts, body))
    }

    fn should_drop_label_block(&self, block: &str) -> bool {
        parse_standalone_caption_label(block).is_some()
    }
}

fn build_caption_plans(page_layouts: &[OcrPageLayout]) -> (Vec<CaptionPlan>, Vec<CaptionPlan>) {
    let mut figures = Vec::new();
    let mut tables = Vec::new();
    for page in page_layouts {
        let page_plans = page_caption_plans(&page.blocks);
        figures.extend(page_plans.0);
        tables.extend(page_plans.1);
    }
    (dedupe_caption_plans(figures), dedupe_caption_plans(tables))
}

fn page_caption_plans(blocks: &[OcrLayoutBlock]) -> (Vec<CaptionPlan>, Vec<CaptionPlan>) {
    let mut figures = Vec::new();
    let mut tables = Vec::new();
    let mut used = HashSet::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.label != "figure_title" || used.contains(&index) {
            continue;
        }
        let Some((kind, number, text)) = parse_caption_start(&block.content) else {
            continue;
        };
        let (parts, consumed_until) = collect_caption_parts(blocks, index, &text);
        for consumed in index..=consumed_until {
            used.insert(consumed);
        }
        let plan = CaptionPlan {
            number: number.clone(),
            label: format!("{kind} {number}"),
            body: join_caption_parts(&parts),
            notes: if kind == "Table" {
                table_notes_after(block, blocks)
            } else {
                Vec::new()
            },
        };
        if kind == "Table" {
            tables.push(plan);
        } else {
            figures.push(plan);
        }
    }
    (figures, tables)
}

fn dedupe_caption_plans(plans: Vec<CaptionPlan>) -> Vec<CaptionPlan> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for plan in plans {
        let key = format!(
            "{}|{}|{}",
            normalize_text(&plan.label),
            normalize_text(&plan.body),
            plan.notes
                .iter()
                .map(|note| normalize_text(note))
                .collect::<Vec<_>>()
                .join("|")
        );
        if seen.insert(key) {
            deduped.push(plan);
        }
    }
    deduped
}

fn collect_caption_parts(
    blocks: &[OcrLayoutBlock],
    start_index: usize,
    initial_text: &str,
) -> (Vec<String>, usize) {
    let mut parts = vec![initial_text.trim().to_string()];
    let mut last_index = start_index;
    let mut previous = &blocks[start_index];
    for (index, block) in blocks.iter().enumerate().skip(start_index + 1) {
        if block.label != "figure_title" || !looks_like_caption_fragment(block) {
            break;
        }
        if !is_nearby_caption_continuation(previous, block) {
            break;
        }
        parts.push(block.content.trim().to_string());
        last_index = index;
        previous = block;
    }
    (parts, last_index)
}

fn table_notes_after(table_title: &OcrLayoutBlock, blocks: &[OcrLayoutBlock]) -> Vec<String> {
    let Some(title_bottom) = bbox_bottom(table_title) else {
        return Vec::new();
    };
    let next_anchor_top = next_caption_or_media_top(blocks, table_title);
    blocks
        .iter()
        .filter(|block| block.label == "vision_footnote")
        .filter(|block| same_page(block, table_title))
        .filter(|block| {
            bbox_top(block).is_some_and(|top| {
                top > title_bottom
                    && next_anchor_top.is_none_or(|anchor_top| top < anchor_top)
                    && top - title_bottom <= TABLE_NOTE_VERTICAL_WINDOW
            })
        })
        .map(|block| block.content.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

fn parse_caption_start(content: &str) -> Option<(&'static str, String, String)> {
    let trimmed = content.trim();
    let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("Fig. ") {
        ("Figure", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Figure ") {
        ("Figure", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Table ") {
        ("Table", rest)
    } else {
        return None;
    };
    let mut chars = rest.chars();
    let number = chars
        .by_ref()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if number.is_empty() {
        return None;
    }
    let body = chars
        .as_str()
        .trim_start_matches(|ch: char| matches!(ch, ' ' | '|' | ':' | '.' | '-'))
        .trim()
        .to_string();
    Some((kind, number, body))
}

fn looks_like_caption_fragment(block: &OcrLayoutBlock) -> bool {
    let text = block.content.trim();
    text.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
        && block.label == "figure_title"
}

fn is_nearby_caption_continuation(previous: &OcrLayoutBlock, next: &OcrLayoutBlock) -> bool {
    if !same_page(previous, next) {
        return false;
    }
    let order_ok = match (previous.order, next.order) {
        (Some(prev), Some(next_order)) => next_order >= prev && next_order - prev <= 2,
        _ => true,
    };
    let vertical_ok = match (bbox_bottom(previous), bbox_top(next)) {
        (Some(prev_bottom), Some(next_top)) => {
            next_top >= prev_bottom && next_top - prev_bottom <= CAPTION_CONTINUATION_VERTICAL_GAP
        }
        _ => true,
    };
    order_ok && vertical_ok
}

fn continuation_belongs_to_caption(caption: &str, next: &str) -> bool {
    let first = next.trim().chars().next();
    !caption.ends_with('.') && first.is_some_and(|ch| ch.is_ascii_lowercase())
}

fn consume_trailing_caption_fragments(
    mut plan: CaptionPlan,
    blocks: &[String],
    start: usize,
) -> (CaptionPlan, usize) {
    let mut consumed = 0usize;
    while consumed < 4 {
        let Some(block) = blocks.get(start + consumed) else {
            break;
        };
        if should_skip_legacy_label(&plan, block) {
            consumed += 1;
            continue;
        }
        if trailing_fragment_belongs_to_caption(&plan.body, block) {
            plan.body = append_caption_fragment(&plan.body, block);
            consumed += 1;
            continue;
        }
        break;
    }
    (plan, consumed)
}

fn should_skip_legacy_label(plan: &CaptionPlan, block: &str) -> bool {
    parse_standalone_caption_label(block)
        .is_some_and(|(kind, number)| plan.label == format!("{kind} {number}"))
}

fn matching_plan_index(
    plans: &[CaptionPlan],
    kind: &str,
    blocks: &[String],
    index: usize,
) -> Option<usize> {
    let target_number = surrounding_media_number(kind, blocks, index)?;
    plans.iter().position(|plan| plan.number == target_number)
}

fn surrounding_media_number(kind: &str, blocks: &[String], index: usize) -> Option<String> {
    let start = index.saturating_sub(1);
    let end = (index + 2).min(blocks.len().saturating_sub(1));
    for block_index in start..=end {
        let block = blocks.get(block_index)?;
        if let Some(number) = parse_media_label_number(kind, block) {
            return Some(number);
        }
        if let Some((block_kind, number)) = parse_standalone_caption_label(block) {
            if block_kind == kind {
                return Some(number);
            }
        }
    }
    None
}

fn parse_media_label_number(kind: &str, block: &str) -> Option<String> {
    let trimmed = block.trim();
    let prefix = format!("{kind} ");
    let rest = trimmed.strip_prefix(&prefix)?;
    let number = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!number.is_empty()).then_some(number)
}

fn trailing_fragment_belongs_to_caption(caption: &str, block: &str) -> bool {
    if parse_standalone_caption_label(block).is_some() {
        return false;
    }
    if caption_contains_block(caption, block) || continuation_belongs_to_caption(caption, block) {
        return true;
    }
    let trimmed = block.trim();
    is_short_figure_panel_label(trimmed)
}

fn append_caption_fragment(caption: &str, fragment: &str) -> String {
    let fragment = fragment.trim();
    if fragment.is_empty() || caption_contains_block(caption, fragment) {
        return caption.to_string();
    }
    if is_short_figure_panel_label(fragment) {
        return format!("{} {}", caption.trim_end(), fragment);
    }
    format!("{} {}", caption.trim_end(), fragment)
}

fn is_short_figure_panel_label(text: &str) -> bool {
    let words = text.split_whitespace().count();
    words <= 3 && text.chars().any(|ch| ch.is_ascii_digit())
}

fn caption_contains_block(caption: &str, block: &str) -> bool {
    let caption = normalize_text(caption);
    let block = normalize_text(block);
    !block.is_empty() && caption.contains(&block)
}

fn parse_standalone_caption_label(block: &str) -> Option<(&str, String)> {
    let trimmed = block.trim();
    let (kind, number) = trimmed.split_once(' ')?;
    if kind != "Figure" && kind != "Table" {
        return None;
    }
    let number = number.trim();
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((kind, number.to_string()))
}

fn format_table_block(plan: &CaptionPlan, image_block: &str) -> String {
    let mut parts = vec![
        format!("{} | {}", plan.label, plan.body),
        image_block.to_string(),
    ];
    let mut seen_notes = HashSet::new();
    parts.extend(
        plan.notes
            .iter()
            .filter(|note| seen_notes.insert(normalize_text(note)))
            .map(|note| format!("> {note}")),
    );
    parts.join("\n\n")
}

fn format_figure_block(plan: &CaptionPlan, image_block: &str) -> String {
    format!(
        "{}\n\n{}\n\n::: figure-caption\n{}\n:::",
        plan.label, image_block, plan.body
    )
}

fn join_caption_parts(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn figure_caption_body(block: &str) -> Option<&str> {
    let trimmed = block.trim();
    let body = trimmed
        .strip_prefix("::: figure-caption")?
        .strip_suffix(":::")?
        .trim();
    (!body.is_empty()).then_some(body)
}

fn quoted_note_body(block: &str) -> Option<&str> {
    let trimmed = block.trim();
    let body = trimmed.strip_prefix('>')?.trim();
    (!body.is_empty()).then_some(body)
}

fn matches_used_caption_body(used_texts: &HashSet<String>, body: &str) -> bool {
    let normalized = normalize_text(body);
    if normalized.is_empty() {
        return false;
    }
    used_texts.contains(&normalized)
        || used_texts
            .iter()
            .any(|used| used.starts_with(&normalized) || normalized.starts_with(used))
}

fn bbox_top(block: &OcrLayoutBlock) -> Option<i32> {
    block.bbox.get(1).copied()
}

fn bbox_bottom(block: &OcrLayoutBlock) -> Option<i32> {
    block.bbox.get(3).copied()
}

fn next_caption_or_media_top(blocks: &[OcrLayoutBlock], source: &OcrLayoutBlock) -> Option<i32> {
    let source_order = source.order.unwrap_or(i32::MIN);
    let source_top = bbox_top(source).unwrap_or(i32::MIN);
    blocks
        .iter()
        .filter(|block| same_page(block, source))
        .filter(|block| {
            let block_order = block.order.unwrap_or(i32::MIN);
            let block_top = bbox_top(block).unwrap_or(i32::MIN);
            block_order > source_order || block_top > source_top
        })
        .filter(|block| {
            block.label == "figure_title"
                || block.label.eq_ignore_ascii_case("image")
                || block.label.eq_ignore_ascii_case("chart")
                || block.label.to_ascii_lowercase().contains("table")
        })
        .filter_map(bbox_top)
        .min()
}

fn same_page(left: &OcrLayoutBlock, right: &OcrLayoutBlock) -> bool {
    match (left.page, right.page) {
        (Some(left_page), Some(right_page)) => left_page == right_page,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_table_title_and_notes_around_snapshot() {
        let markdown = concat!(
            "Table 1\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n\n",
            "The table displays model comparisons."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "figure_title",
                    "Table 1 | Model comparison",
                    vec![0, 10, 100, 20],
                ),
                block(
                    "vision_footnote",
                    "The table displays model comparisons.",
                    vec![0, 100, 100, 120],
                ),
            ],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert!(output.contains("Table 1 | Model comparison"));
        assert!(output.contains("> The table displays model comparisons."));
        assert_eq!(
            output
                .matches("The table displays model comparisons.")
                .count(),
            1
        );
    }

    #[test]
    fn injects_figure_caption_and_consumes_lowercase_tail() {
        let markdown = concat!(
            "Body.\n\n",
            "![PDF figure cluster](imgs/figure_cluster_001.jpg)\n\n",
            "was higher for news that was actually true."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "figure_title",
                    "Fig. 2 | Distributions of success.",
                    vec![0, 10, 100, 20],
                ),
                block(
                    "figure_title",
                    "was higher for news that was actually true.",
                    vec![120, 10, 200, 20],
                ),
            ],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert!(output.contains("Figure 2\n\n![PDF figure cluster](imgs/figure_cluster_001.jpg)"));
        assert!(output.contains("::: figure-caption"));
        assert!(output.contains("Distributions of success."));
        assert_eq!(output.matches("was higher for news").count(), 1);
    }

    #[test]
    fn skips_existing_label_and_binds_chart_image() {
        let markdown = concat!(
            "Figure 3\n\n",
            "![](imgs/img_in_chart_box_1_2_3_4.jpg)\n\n",
            "Body."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![block(
                "figure_title",
                "Fig. 3 | Calibration analysis.",
                vec![0, 10, 100, 20],
            )],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert_eq!(output.matches("Figure 3").count(), 1);
        assert!(output.contains("::: figure-caption\nCalibration analysis.\n:::"));
        assert!(output.contains("![](imgs/img_in_chart_box_1_2_3_4.jpg)"));
    }

    #[test]
    fn merges_legacy_panel_label_and_lowercase_tail_into_caption() {
        let markdown = concat!(
            "![](imgs/img_in_image_box_1_2_3_4.jpg)\n\n",
            "Figure 1\n\n",
            "Phase 2\n\n",
            "choice, they had to indicate how much they were willing to pay."
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![block(
                "figure_title",
                "Fig. 1 | Description of the task. Given their",
                vec![0, 10, 100, 20],
            )],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert!(output.contains("Figure 1\n\n![](imgs/img_in_image_box_1_2_3_4.jpg)"));
        assert!(output.contains("::: figure-caption"));
        assert!(output.contains("Description of the task."));
        assert!(output.contains("Given their Phase 2 choice, they had"));
        assert_eq!(output.matches("Phase 2").count(), 1);
    }

    #[test]
    fn drops_duplicate_legacy_figure_caption_blocks_after_injection() {
        let markdown = concat!(
            "Figure 1\n\n",
            "![](imgs/img_in_image_box_1_2_3_4.jpg)\n\n",
            "::: figure-caption\n",
            "Description of the task.\n",
            ":::\n\n",
            "::: figure-caption\n",
            "Description of the task.\n",
            ":::\n"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![block(
                "figure_title",
                "Fig. 1 | Description of the task.",
                vec![0, 10, 100, 20],
            )],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert_eq!(output.matches("Description of the task.").count(), 1);
        assert_eq!(output.matches("::: figure-caption").count(), 1);
    }

    #[test]
    fn drops_duplicate_legacy_table_note_blocks_after_injection() {
        let markdown = concat!(
            "Table 1 | Model comparison\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n\n",
            "> The table displays model comparisons.\n\n",
            "> The table displays model comparisons.\n"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "figure_title",
                    "Table 1 | Model comparison",
                    vec![0, 10, 100, 20],
                ),
                block(
                    "vision_footnote",
                    "The table displays model comparisons.",
                    vec![0, 100, 100, 120],
                ),
            ],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert_eq!(
            output
                .matches("The table displays model comparisons.")
                .count(),
            1
        );
    }

    #[test]
    fn table_caption_injection_is_idempotent_with_existing_equivalent_title() {
        let markdown = concat!(
            "Table 1: Overview of intervention types in the toolbox.\n\n",
            "Table 1 | Overview of intervention types in the toolbox.\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![block(
                "figure_title",
                "Table 1 | Overview of intervention types in the toolbox.",
                vec![0, 10, 100, 20],
            )],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert_eq!(
            output
                .matches("Overview of intervention types in the toolbox.")
                .count(),
            1
        );
        assert!(output.contains("Table 1 | Overview of intervention types in the toolbox."));
        assert!(output.contains("![PDF table 1](imgs/table_snapshot_001.jpg)"));
    }

    #[test]
    fn binds_figure_plan_by_nearby_label_number_instead_of_fifo_only() {
        let markdown = concat!(
            "Figure 2\n\n",
            "![PDF figure cluster](imgs/figure_cluster_002.jpg)\n\n",
            "Figure 1\n\n",
            "![PDF figure cluster](imgs/figure_cluster_001.jpg)\n"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "figure_title",
                    "Fig. 1 | First caption.",
                    vec![0, 10, 100, 20],
                ),
                block(
                    "figure_title",
                    "Fig. 2 | Second caption.",
                    vec![0, 40, 100, 50],
                ),
            ],
        }];

        let output = inject_layout_captions(markdown, &layouts);
        assert!(output.contains("Second caption."));
        assert!(output.contains("First caption."));
        assert!(output.find("Second caption.").unwrap() < output.find("First caption.").unwrap());
    }

    #[test]
    fn keeps_distant_table_footnote_out_of_table_note_block() {
        let markdown = concat!(
            "Table 1\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n"
        );
        let layouts = vec![OcrPageLayout {
            blocks: vec![
                block(
                    "figure_title",
                    "Table 1 | Model comparison",
                    vec![0, 10, 100, 20],
                ),
                block("vision_footnote", "Near table note.", vec![0, 80, 100, 96]),
                block(
                    "figure_title",
                    "Fig. 2 | A later figure.",
                    vec![0, 140, 100, 160],
                ),
                block(
                    "vision_footnote",
                    "Far note should stay out.",
                    vec![0, 210, 100, 226],
                ),
            ],
        }];

        let output = inject_layout_captions(markdown, &layouts);

        assert!(output.contains("> Near table note."));
        assert!(!output.contains("Far note should stay out."));
    }

    #[test]
    fn does_not_bind_unmatched_table_plan_by_fifo_fallback() {
        let markdown = concat!(
            "Table 1 | Overview of intervention types in the toolbox.\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n\n",
            "## A.2 Co-authors and Their Areas of Expertise\n\n",
            "Table A1: Co-authors\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 5](imgs/table_snapshot_005.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n"
        );
        let layouts = vec![
            OcrPageLayout {
                blocks: vec![block(
                    "figure_title",
                    "Table 1 | Overview of intervention types in the toolbox.",
                    vec![0, 10, 100, 20],
                )],
            },
            OcrPageLayout {
                blocks: vec![block(
                    "figure_title",
                    "Table A1: Co-authors",
                    vec![0, 30, 100, 40],
                )],
            },
        ];

        let output = inject_layout_captions(markdown, &layouts);

        assert!(output.contains("Table A1: Co-authors"));
        assert!(!output.contains(
            "Table A1: Co-authors\n\nTable 1 | Overview of intervention types in the toolbox.\n\n<!-- musetranslate:pdf-table:start -->\n![PDF table 5]"
        ));
    }

    fn block(label: &str, content: &str, bbox: Vec<i32>) -> OcrLayoutBlock {
        OcrLayoutBlock {
            label: label.to_string(),
            content: content.to_string(),
            bbox,
            order: None,
            page: Some(0),
        }
    }
}
