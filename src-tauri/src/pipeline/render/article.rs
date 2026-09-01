use super::super::layout::{is_figure_or_table_label, normalize_figure_table_label};

pub(super) fn prepare_markdown_for_article_body(
    markdown: &str,
    title: &str,
    abstract_heading_zh: &str,
    references_heading_zh: &str,
) -> String {
    let without_title = remove_rendered_document_title(markdown, title);
    normalize_article_markdown_labels(
        &remove_author_metadata_lines(&remove_academic_address_sections(&without_title)),
        abstract_heading_zh,
        references_heading_zh,
    )
}

fn normalize_article_markdown_labels(
    markdown: &str,
    abstract_heading_zh: &str,
    references_heading_zh: &str,
) -> String {
    let mut seen_content = false;
    markdown
        .lines()
        .map(|line| {
            normalize_article_markdown_line(
                line,
                &mut seen_content,
                abstract_heading_zh,
                references_heading_zh,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_article_markdown_line(
    line: &str,
    seen_content: &mut bool,
    abstract_heading_zh: &str,
    references_heading_zh: &str,
) -> String {
    let normalized_plain = line.trim().to_ascii_lowercase();
    let mapped = normalized_heading_text(line)
        .as_deref()
        .and_then(|heading| {
            map_article_heading(heading, abstract_heading_zh, references_heading_zh)
        })
        .or_else(|| {
            (!*seen_content && matches!(normalized_plain.as_str(), "review" | "abstract"))
                .then(|| heading_line(abstract_heading_zh))
        })
        .or_else(|| {
            is_standalone_figure_or_table_label(line).then(|| normalize_figure_table_label(line))
        });
    if !line.trim().is_empty() {
        *seen_content = true;
    }
    mapped.unwrap_or_else(|| line.to_string())
}

fn heading_line(text: &str) -> String {
    format!("## {text}")
}

fn map_article_heading(
    heading: &str,
    abstract_heading_zh: &str,
    references_heading_zh: &str,
) -> Option<String> {
    match heading {
        "abstract" | "review" => Some(heading_line(abstract_heading_zh)),
        "references" | "bibliography" | "works cited" | "literature cited" => {
            Some(heading_line(references_heading_zh))
        }
        _ => None,
    }
}

fn remove_academic_address_sections(markdown: &str) -> String {
    let mut output = Vec::new();
    let mut skip = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        let heading = normalized_heading_text(line);
        if is_academic_metadata_heading(trimmed, heading.as_deref()) {
            skip = true;
            continue;
        }
        if skip && trimmed.starts_with('#') {
            skip = false;
        }
        if !skip {
            output.push(line);
        }
    }
    output.join("\n")
}

fn remove_author_metadata_lines(markdown: &str) -> String {
    let mut output = Vec::new();
    let mut inside_author_metadata_block = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        let is_author_line = looks_like_author_line(trimmed);
        if is_author_line {
            inside_author_metadata_block = true;
            output.push(line);
            continue;
        }

        if inside_author_metadata_block {
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') {
                inside_author_metadata_block = false;
                output.push(line);
                continue;
            }
            if is_author_metadata_line(trimmed) || is_author_metadata_detail_line(trimmed) {
                continue;
            }
            inside_author_metadata_block = false;
        }

        output.push(line);
    }

    output.join("\n")
}

fn is_author_metadata_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_chinese_correspondence_line(trimmed) || is_chinese_email_line(trimmed) {
        return true;
    }
    is_academic_affiliation_line(trimmed)
        || looks_like_affiliation_detail_line(trimmed)
        || trimmed
            .to_ascii_lowercase()
            .starts_with("corresponding authors:")
        || trimmed
            .to_ascii_lowercase()
            .starts_with("corresponding author:")
        || trimmed.to_ascii_lowercase().contains("correspondence to:")
}

fn is_author_metadata_detail_line(trimmed: &str) -> bool {
    if is_chinese_metadata_detail_line(trimmed) {
        return true;
    }
    trimmed.contains('@')
        || trimmed.contains("通讯作者")
        || trimmed
            .to_ascii_lowercase()
            .contains("corresponding author")
        || trimmed.starts_with('（')
        || trimmed.starts_with('(')
        || trimmed.contains("接收")
        || trimmed.contains("修订")
        || trimmed.contains("接受")
        || trimmed.contains("首次在线发表")
        || trimmed.contains("电子邮箱")
        || looks_like_affiliation_detail_line(trimmed)
}

fn is_academic_metadata_heading(trimmed_line: &str, normalized_heading: Option<&str>) -> bool {
    if !trimmed_line.starts_with('#') {
        return false;
    }
    matches!(
        normalized_heading,
        Some("addresses" | "affiliations" | "author information" | "author details")
    )
}

fn is_academic_affiliation_line(trimmed_line: &str) -> bool {
    let Some(rest) = trimmed_line.strip_prefix("$ ^{") else {
        return false;
    };
    let Some((marker, content)) = rest.split_once("} $") else {
        return false;
    };
    let marker = marker.trim();
    let content = content.trim();
    !marker.is_empty()
        && !content.is_empty()
        && marker
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == ',' || ch.is_ascii_whitespace())
}

fn looks_like_author_line(trimmed: &str) -> bool {
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('$') {
        return false;
    }
    if trimmed.contains("$ ^{") {
        return true;
    }
    if looks_like_ocr_author_line(trimmed) {
        return true;
    }
    if is_author_metadata_detail_line(trimmed) {
        return false;
    }
    (trimmed.contains('（') || trimmed.contains('('))
        && (trimmed.contains('与')
            || trimmed.to_ascii_lowercase().contains(" and ")
            || trimmed.ends_with('）')
            || trimmed.ends_with(')'))
}

fn looks_like_ocr_author_line(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("corresponding author")
        || lower.contains("correspondence to:")
        || lower.contains("department")
        || lower.contains("university")
        || trimmed.contains('@')
    {
        return false;
    }
    if !(lower.contains(" and ") || trimmed.contains(" \u{548C} ")) || !trimmed.contains(" ID") {
        return false;
    }
    split_ocr_author_segments(trimmed).all(|segment| {
        let candidate = segment
            .trim()
            .trim_end_matches(" ID")
            .trim_end_matches('*')
            .trim();
        let tokens = candidate.split_whitespace().collect::<Vec<_>>();
        !candidate.is_empty()
            && tokens.len() >= 2
            && tokens.iter().all(|token| {
                token
                    .chars()
                    .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '-' | '\'' | '.'))
            })
    })
}

fn looks_like_affiliation_detail_line(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    if contains_chinese_academic_affiliation(trimmed) {
        return true;
    }
    trimmed.contains("大学")
        || trimmed.contains("学院")
        || trimmed.contains("研究所")
        || trimmed.contains("系")
        || lower.contains("department")
        || lower.contains("college")
        || lower.contains("university")
}

fn split_ocr_author_segments(trimmed: &str) -> impl Iterator<Item = &str> {
    trimmed
        .split(" and ")
        .flat_map(|segment| segment.split(" \u{548C} "))
}

fn is_chinese_metadata_detail_line(trimmed: &str) -> bool {
    is_chinese_correspondence_line(trimmed)
        || is_chinese_email_line(trimmed)
        || contains_chinese_submission_history(trimmed)
        || trimmed.starts_with('\u{FF08}')
}

fn contains_chinese_academic_affiliation(trimmed: &str) -> bool {
    (trimmed.contains('\u{5927}') && trimmed.contains('\u{5B66}'))
        || (trimmed.contains('\u{5B66}') && trimmed.contains('\u{9662}'))
        || trimmed.contains('\u{7CFB}')
}

fn is_chinese_correspondence_line(trimmed: &str) -> bool {
    trimmed.contains("\u{901A}\u{8BAF}\u{4F5C}\u{8005}")
}

fn is_chinese_email_line(trimmed: &str) -> bool {
    trimmed.contains("\u{7535}\u{5B50}\u{90AE}\u{7BB1}")
}

fn contains_chinese_submission_history(trimmed: &str) -> bool {
    [
        "\u{6536}\u{7A3F}",
        "\u{4FEE}\u{56DE}",
        "\u{63A5}\u{53D7}",
        "\u{9996}\u{6B21}\u{5728}\u{7EBF}",
    ]
    .iter()
    .any(|marker| trimmed.contains(marker))
}
fn normalized_heading_text(line: &str) -> Option<String> {
    markdown_heading_text(line).map(|value| {
        value
            .trim()
            .trim_end_matches(':')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    })
}

fn is_standalone_figure_or_table_label(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.starts_with('#') && is_figure_or_table_label(trimmed)
}

fn remove_rendered_document_title(markdown: &str, title: &str) -> String {
    let mut removed = false;
    let mut skip_leading_blank_after_title = false;
    let mut output = Vec::new();

    for line in markdown.lines() {
        if should_remove_title_line(line, title, removed) {
            removed = true;
            skip_leading_blank_after_title = true;
            continue;
        }
        if skip_leading_blank_after_title && line.trim().is_empty() {
            continue;
        }
        skip_leading_blank_after_title = false;
        output.push(line);
    }

    output.join("\n")
}

fn should_remove_title_line(line: &str, title: &str, removed: bool) -> bool {
    !removed
        && markdown_heading_text(line).is_some_and(|heading| same_markdown_label(heading, title))
}

fn markdown_heading_text(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let text = trimmed.get(hashes..)?.trim();
    (!text.is_empty()).then_some(text)
}

fn same_markdown_label(left: &str, right: &str) -> bool {
    normalize_markdown_label(left) == normalize_markdown_label(right)
}

fn normalize_markdown_label(value: &str) -> String {
    value
        .trim()
        .trim_matches('#')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_ocr_style_author_metadata_block_before_abstract() {
        let markdown = "# Nudge plus: incorporating reflection into behavioral public policy\n\nSanchayan Banerjee ID and Peter John* ID\n\nDepartment of Geography and Environment, London School of Economics, London, UK and Department of Political Economy, King's College London, London, UK\n\n$ ^{*} $Correspondence to: Department of Political Economy, King's College London, Bush House, 40 Aldwych, London WC2B 4PH, UK.\n\nE-mail: peter.john@kcl.ac.uk\n\n(Received 3 January 2020; revised 22 December 2020; accepted 20 January 2021; first published online 16 February 2021)\n\n## Abstract\n\nBody.\n";

        let prepared = prepare_markdown_for_article_body(
            markdown,
            "Nudge plus: incorporating reflection into behavioral public policy",
            "摘要",
            "参考文献",
        );

        assert!(prepared.contains("Sanchayan Banerjee ID and Peter John* ID"));
        assert!(!prepared.contains("Department of Geography and Environment"));
        assert!(!prepared.contains("Correspondence to:"));
        assert!(!prepared.contains("peter.john@kcl.ac.uk"));
        assert!(!prepared.contains("Received 3 January 2020"));
        assert!(prepared.contains("## 摘要"));
    }

    #[test]
    fn removes_translated_author_metadata_block_before_abstract() {
        let markdown = "# Nudge plus\n\nSanchayan Banerjee ID \u{548C} Peter John* ID\n\n\u{82F1}\u{56FD}\u{4F26}\u{6566}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{5B66}\u{9662}\u{5730}\u{7406}\u{4E0E}\u{73AF}\u{5883}\u{7CFB}\u{FF0C}\u{4EE5}\u{53CA}\u{82F1}\u{56FD}\u{4F26}\u{6566}\u{56FD}\u{738B}\u{5B66}\u{9662}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{7CFB}\n\n$ ^{*} $\u{901A}\u{8BAF}\u{4F5C}\u{8005}\u{FF1A}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{7CFB}\u{FF0C}\u{56FD}\u{738B}\u{5B66}\u{9662}\u{3002}\n\n\u{7535}\u{5B50}\u{90AE}\u{7BB1}\u{FF1A}peter.john@kcl.ac.uk\n\n\u{FF08}\u{6536}\u{7A3F}\u{65E5}\u{671F}\u{FF1A}2020\u{5E74}5\u{6708}6\u{65E5}\u{FF1B}\u{4FEE}\u{56DE}\u{65E5}\u{671F}\u{FF1A}2020\u{5E74}12\u{6708}22\u{65E5}\u{FF09}\n\n## \u{6458}\u{8981}\n\nBody.\n";

        let prepared = prepare_markdown_for_article_body(
            markdown,
            "Nudge plus",
            "\u{6458}\u{8981}",
            "\u{53C2}\u{8003}\u{6587}\u{732E}",
        );

        assert!(prepared.contains("Sanchayan Banerjee ID \u{548C} Peter John* ID"));
        assert!(
            !prepared.contains("\u{4F26}\u{6566}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{5B66}\u{9662}")
        );
        assert!(!prepared.contains("\u{901A}\u{8BAF}\u{4F5C}\u{8005}"));
        assert!(!prepared.contains("peter.john@kcl.ac.uk"));
        assert!(!prepared.contains("\u{6536}\u{7A3F}\u{65E5}\u{671F}"));
        assert!(prepared.contains("## \u{6458}\u{8981}"));
    }

    #[test]
    fn keeps_abstract_body_after_author_metadata_block() {
        let markdown = "# Nudge plus\n\nSanchayan Banerjee ID and Peter John* ID\n\nDepartment of Geography and Environment, London School of Economics, London, UK\n\n$ ^{*} $Correspondence to: Department of Political Economy, King's College London.\n\nE-mail: peter.john@kcl.ac.uk\n\n(Received 6 May 2020; revised 22 December 2020)\n\n## \u{6458}\u{8981}\n\n\u{8BE5}\u{8BBA}\u{70B9}\u{57FA}\u{4E8E}\u{5BF9}\u{53CC}\u{91CD}\u{7CFB}\u{7EDF}\u{7684}\u{5F00}\u{521B}\u{6027}\u{7814}\u{7A76}\u{FF0C}\u{8FD9}\u{4E9B}\u{7814}\u{7A76}\u{63ED}\u{793A}\u{4E86}\u{5FEB}\u{901F}\u{601D}\u{8003}\u{4E0E}\u{6162}\u{901F}\u{601D}\u{8003}\u{4E4B}\u{95F4}\u{7684}\u{5173}\u{7CFB}\u{3002}\n\n\u{5173}\u{952E}\u{8BCD}\u{FF1A}\u{52A9}\u{63A8}\u{FF1B}\u{53CC}\u{8FC7}\u{7A0B}\u{7406}\u{8BBA}\n";

        let prepared = prepare_markdown_for_article_body(
            markdown,
            "Nudge plus",
            "\u{6458}\u{8981}",
            "\u{53C2}\u{8003}\u{6587}\u{732E}",
        );

        assert!(!prepared.contains("Department of Geography"));
        assert!(!prepared.contains("Correspondence to:"));
        assert!(!prepared.contains("peter.john@kcl.ac.uk"));
        assert!(prepared.contains("\u{53CC}\u{91CD}\u{7CFB}\u{7EDF}"));
        assert!(prepared.contains("\u{5173}\u{952E}\u{8BCD}"));
    }
}
