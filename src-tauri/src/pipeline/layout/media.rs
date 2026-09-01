pub(super) fn parse_markdown_image(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("![") {
        return None;
    }
    let alt_end = trimmed.find("](")?;
    let src_end = trimmed.rfind(')')?;
    if src_end <= alt_end + 2 {
        return None;
    }
    let alt = trimmed[2..alt_end].to_string();
    let src = trimmed[alt_end + 2..src_end].trim().to_string();
    (!src.is_empty()).then_some((alt, src))
}

pub(super) fn is_figure_or_table_label(label: &str) -> bool {
    parse_figure_or_table_label(label).is_some()
}

pub(super) fn normalize_figure_table_label(label: &str) -> String {
    let Some((kind, number)) = parse_figure_or_table_label(label) else {
        return label.trim().to_string();
    };
    match kind {
        MediaLabelKind::Figure => figure_label(&number),
        MediaLabelKind::Table => table_label(&number),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaLabelKind {
    Figure,
    Table,
}

fn parse_figure_or_table_label(label: &str) -> Option<(MediaLabelKind, String)> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    if let Some(parsed) = parse_compact_zh_media_label(trimmed) {
        return Some(parsed);
    }
    let tokens = tokenize_media_label(trimmed);
    let first = normalize_media_kind_token(tokens.first()?)?;
    let kind = media_label_kind_from_token(first)?;
    if tokens.len() > 2
        && tokens[2..]
            .iter()
            .any(|token| token.chars().any(|ch| ch.is_alphanumeric()))
    {
        return None;
    }
    let number_token = media_label_number_token(trimmed, first, &tokens)?;
    normalize_media_label_number(kind, number_token)
}

fn parse_compact_zh_media_label(label: &str) -> Option<(MediaLabelKind, String)> {
    if let Some(rest) = label.strip_prefix('图') {
        return normalize_compact_media_label(MediaLabelKind::Figure, rest);
    }
    if let Some(rest) = label.strip_prefix('表') {
        return normalize_compact_media_label(MediaLabelKind::Table, rest);
    }
    None
}

fn tokenize_media_label(label: &str) -> Vec<&str> {
    label
        .split(|ch: char| matches!(ch, ':' | '：' | '.' | ',' | '，') || ch.is_whitespace())
        .filter(|token| !token.is_empty())
        .collect()
}

fn normalize_media_kind_token(token: &str) -> Option<&str> {
    let trimmed = token.trim_matches(|ch: char| matches!(ch, '.' | ':' | '：'));
    (!trimmed.is_empty()).then_some(trimmed)
}

fn media_label_kind_from_token(token: &str) -> Option<MediaLabelKind> {
    match token.to_ascii_lowercase().as_str() {
        "figure" | "fig" => Some(MediaLabelKind::Figure),
        "table" | "tbl" => Some(MediaLabelKind::Table),
        _ => None,
    }
}

fn media_label_number_token<'a>(
    label: &'a str,
    kind_token: &str,
    tokens: &[&'a str],
) -> Option<&'a str> {
    tokens.get(1).copied().or_else(|| {
        label
            .split_once(kind_token)
            .map(|(_, rest)| rest.trim())
            .filter(|rest| !rest.is_empty())
    })
}

fn normalize_compact_media_label(
    kind: MediaLabelKind,
    rest: &str,
) -> Option<(MediaLabelKind, String)> {
    normalize_media_label_number(kind, rest.trim())
}

fn normalize_media_label_number(
    kind: MediaLabelKind,
    raw_number: &str,
) -> Option<(MediaLabelKind, String)> {
    let cleaned = raw_number
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, '.' | ':' | '：' | ',' | '，' | ';' | '；'))
        .trim()
        .trim_end_matches(|ch: char| matches!(ch, '.' | ':' | '：' | ',' | '，' | ';' | '；'))
        .trim();
    if cleaned.is_empty() {
        return None;
    }
    let number = cleaned
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>();
    if number.is_empty() || !looks_like_media_label_number(&number) {
        return None;
    }
    let remainder = cleaned[number.len()..].trim();
    if !remainder.is_empty()
        && remainder
            .chars()
            .any(|ch| ch.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch))
    {
        return None;
    }
    Some((kind, number))
}

fn looks_like_media_label_number(value: &str) -> bool {
    value.chars().all(|ch| ch.is_ascii_digit())
        || value
            .chars()
            .all(|ch| matches!(ch, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
}

fn figure_label(number: &str) -> String {
    format!("Figure {number}")
}

fn table_label(number: &str) -> String {
    format!("Table {number}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figure_and_table_labels_are_parsed_generically() {
        assert_eq!(
            parse_figure_or_table_label("Figure 1:"),
            Some((MediaLabelKind::Figure, "1".to_string()))
        );
        assert_eq!(
            parse_figure_or_table_label("Fig. 2"),
            Some((MediaLabelKind::Figure, "2".to_string()))
        );
        assert_eq!(
            parse_figure_or_table_label("图3"),
            Some((MediaLabelKind::Figure, "3".to_string()))
        );
        assert_eq!(
            parse_figure_or_table_label("TABLE IV"),
            Some((MediaLabelKind::Table, "IV".to_string()))
        );
        assert!(parse_figure_or_table_label("Figure 1 caption.").is_none());
        assert!(parse_figure_or_table_label("Table 2 note").is_none());
        assert!(parse_figure_or_table_label("figure of speech").is_none());
        assert!(parse_figure_or_table_label("table manners").is_none());
    }
}
