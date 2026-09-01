use super::{canonical_author_key, AcademicAuthor};
use std::collections::HashMap;

const CN_OPEN_PAREN: char = '\u{FF08}';
const CN_CLOSE_PAREN: char = '\u{FF09}';
const CN_SEPARATOR: char = '\u{3001}';
const CN_AND: &str = "\u{4E0E}";
const CN_UNIVERSITY: &str = "\u{5927}\u{5B66}";
const CN_COLLEGE: &str = "\u{5B66}\u{9662}";
const CN_DEPARTMENT: &str = "\u{7CFB}";
const CN_AUTHOR_NOTE: &str = "\u{901A}\u{8BAF}\u{4F5C}\u{8005}";
const CN_EMAIL: &str = "\u{7535}\u{5B50}\u{90AE}\u{7BB1}";
const CN_ABSTRACT: &str = "\u{6458}\u{8981}";

pub(super) fn extract_academic_authors(markdown: &str) -> Vec<AcademicAuthor> {
    let author_line = find_probable_author_line(markdown);
    let affiliations = extract_affiliation_map(markdown);
    let correspondence = extract_correspondence_map(markdown);
    let metadata_affiliations = extract_metadata_affiliation_lines(markdown);
    let mut authors = Vec::new();

    let Some(line) = author_line else {
        return authors;
    };
    for raw_author in split_author_line(line) {
        let Some((name, markers)) = parse_author_markers(&raw_author) else {
            continue;
        };
        let mut author_affiliations = markers
            .iter()
            .filter_map(|marker| affiliations.get(marker).cloned())
            .collect::<Vec<_>>();
        if author_affiliations.is_empty() && !metadata_affiliations.is_empty() {
            author_affiliations.extend(metadata_affiliations.clone());
        }
        let correspondence_entry = correspondence.get(&canonical_author_key(&name));
        let is_corresponding =
            markers.iter().any(|marker| marker == "*") || correspondence_entry.is_some();
        authors.push(AcademicAuthor {
            email: correspondence_entry.and_then(|entry| entry.email.clone()),
            markers,
            name,
            affiliations: author_affiliations,
            is_corresponding,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: correspondence_entry
                .and_then(|entry| entry.note.clone())
                .into_iter()
                .collect(),
            correspondence_note: correspondence_entry.and_then(|entry| entry.note.clone()),
        });
    }

    backfill_single_corresponding_author(&mut authors, &correspondence);

    authors
}

#[derive(Debug, Clone, Default)]
struct CorrespondenceEntry {
    email: Option<String>,
    note: Option<String>,
}

fn find_probable_author_line(markdown: &str) -> Option<&str> {
    let lines = markdown.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if looks_like_author_line(trimmed) {
            return Some(*line);
        }
        if looks_like_unmarked_author_line_with_context(
            trimmed,
            following_nonempty_lines(&lines, index),
        ) {
            return Some(*line);
        }
    }
    None
}

fn looks_like_author_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.starts_with('$')
        && (looks_like_marker_based_author_line(trimmed) || looks_like_ocr_author_line(trimmed))
}

fn looks_like_marker_based_author_line(trimmed: &str) -> bool {
    if looks_like_publication_metadata_line(trimmed) {
        return false;
    }
    (trimmed.contains('$')
        || trimmed.contains('(')
        || trimmed.contains(CN_OPEN_PAREN)
        || contains_author_marker_glyph(trimmed))
        && split_author_line(trimmed)
            .iter()
            .any(|part| parse_author_markers(part).is_some())
}

fn looks_like_publication_metadata_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(open_index) = trimmed.find('(') else {
        return false;
    };
    let Some(close_relative) = trimmed[open_index + 1..].find(')') else {
        return false;
    };
    let close_index = open_index + 1 + close_relative;
    let marker = trimmed[open_index + 1..close_index].trim();
    marker.len() == 4
        && marker.chars().all(|ch| ch.is_ascii_digit())
        && trimmed[close_index + 1..].trim_start().starts_with(',')
}

fn looks_like_ocr_author_line(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("corresponding author")
        || lower.contains("department")
        || lower.contains("university")
        || trimmed.contains('@')
    {
        return false;
    }
    if !(lower.contains(" and ") || trimmed.contains(" \u{548C} ")) || !trimmed.contains(" ID") {
        return false;
    }
    split_ocr_author_segments(trimmed).all(looks_like_ocr_author_segment)
}

fn looks_like_ocr_author_segment(segment: &str) -> bool {
    let candidate = segment
        .trim()
        .trim_end_matches(" ID")
        .trim_end_matches('*')
        .trim();
    if candidate.is_empty() {
        return false;
    }
    let tokens = candidate.split_whitespace().collect::<Vec<_>>();
    tokens.len() >= 2
        && tokens.iter().all(|token| {
            token
                .chars()
                .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '-' | '\'' | '.'))
        })
}

fn split_author_line(line: &str) -> Vec<String> {
    if line.contains('$') {
        return split_author_line_with_math_markers(line);
    }
    if contains_author_marker_glyph(line) {
        return split_author_line_with_marker_glyphs(line);
    }
    if looks_like_ocr_author_line(line) {
        return split_ocr_author_segments(line)
            .map(normalize_author_segment)
            .filter(|segment| !segment.is_empty())
            .collect();
    }
    if looks_like_unmarked_author_line(line) {
        return split_unmarked_author_segments(line)
            .map(normalize_author_segment)
            .filter(|segment| !segment.is_empty())
            .collect();
    }

    split_author_line_with_parenthetical_markers(line)
}

fn split_ocr_author_segments(line: &str) -> impl Iterator<Item = &str> {
    line.split(" and ")
        .flat_map(|segment| segment.split(" \u{548C} "))
}

fn split_author_line_with_math_markers(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cursor = 0usize;

    while let Some(relative_start) = line[cursor..].find('$') {
        let marker_start = cursor + relative_start;
        let marker_content_start = marker_start + '$'.len_utf8();
        let Some(relative_end) = line[marker_content_start..].find('$') else {
            break;
        };
        let marker_end = marker_content_start + relative_end;
        if extract_math_markers(&line[marker_start..=marker_end]).is_none() {
            cursor = marker_end + '$'.len_utf8();
            continue;
        }
        let segment_end = marker_end + '$'.len_utf8();
        let segment = normalize_author_segment(&line[cursor..segment_end]);
        if !segment.is_empty() {
            parts.push(segment);
        }
        cursor = segment_end;
    }

    parts
}

fn split_author_line_with_marker_glyphs(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    let mut index = 0usize;

    while index < line.len() {
        let tail = &line[index..];
        let separator_len = marker_author_separator_len(tail);
        if separator_len > 0 {
            let segment = normalize_author_segment(&line[cursor..index]);
            if !segment.is_empty() {
                parts.push(segment);
            }
            index += separator_len;
            cursor = index;
            continue;
        }
        let Some(ch) = tail.chars().next() else {
            break;
        };
        index += ch.len_utf8();
    }

    let segment = normalize_author_segment(&line[cursor..]);
    if !segment.is_empty() {
        parts.push(segment);
    }
    parts
}

fn marker_author_separator_len(tail: &str) -> usize {
    if tail.starts_with(" & ") {
        return 3;
    }
    if tail.starts_with(" and ") {
        return 5;
    }
    if tail.starts_with(", ") {
        let next = tail[2..].trim_start().chars().next();
        if next.is_some_and(|ch| ch.is_ascii_uppercase()) {
            return 2;
        }
    }
    0
}

fn split_author_line_with_parenthetical_markers(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cursor = 0usize;

    while cursor < line.len() {
        let remaining = &line[cursor..];
        let Some(open_offset) = remaining.find(['(', CN_OPEN_PAREN]) else {
            break;
        };
        let open_index = cursor + open_offset;
        let marker_open = line[open_index..].chars().next().unwrap_or('(');
        let marker_close = if marker_open == CN_OPEN_PAREN {
            CN_CLOSE_PAREN
        } else {
            ')'
        };
        let marker_content_start = open_index + marker_open.len_utf8();
        let Some(close_offset) = line[marker_content_start..].find(marker_close) else {
            break;
        };
        let segment_end = marker_content_start + close_offset + marker_close.len_utf8();
        let segment = normalize_author_segment(&line[cursor..segment_end]);
        if !segment.is_empty() {
            parts.push(segment);
        }
        cursor = segment_end;
    }

    parts
}

fn normalize_author_segment(segment: &str) -> String {
    let trimmed = segment
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, ',' | ';' | '&' | CN_SEPARATOR))
        .trim_start();
    trimmed
        .strip_prefix("and ")
        .or_else(|| trimmed.strip_prefix("And "))
        .or_else(|| trimmed.strip_prefix(CN_AND))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn following_nonempty_lines<'a>(
    lines: &'a [&'a str],
    index: usize,
) -> impl Iterator<Item = &'a str> {
    lines
        .iter()
        .skip(index + 1)
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .take(3)
}

fn looks_like_unmarked_author_line_with_context<'a>(
    line: &str,
    following: impl Iterator<Item = &'a str>,
) -> bool {
    looks_like_unmarked_author_line(line)
        && following
            .take_while(|line| !is_probable_abstract_heading(line))
            .any(|line| looks_like_affiliation_line(line) || is_correspondence_note_line(line))
}

fn looks_like_unmarked_author_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.contains('@')
        || is_probable_abstract_heading(trimmed)
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("department")
        || lower.contains("university")
        || lower.contains("institute")
        || lower.contains("school")
        || lower.contains("college")
        || lower.contains("abstract")
    {
        return false;
    }
    let segments = split_unmarked_author_segments(trimmed).collect::<Vec<_>>();
    segments.len() >= 2
        && segments
            .iter()
            .all(|segment| looks_like_unmarked_author_segment(segment))
}

fn split_unmarked_author_segments(line: &str) -> impl Iterator<Item = &str> {
    line.split(" and ")
        .flat_map(|segment| segment.split(" & "))
        .flat_map(|segment| segment.split(" \u{548C} "))
}

fn looks_like_unmarked_author_segment(segment: &str) -> bool {
    let candidate = segment.trim().trim_end_matches('*').trim();
    if candidate.is_empty() || candidate.contains(',') {
        return false;
    }
    let tokens = candidate.split_whitespace().collect::<Vec<_>>();
    (2..=5).contains(&tokens.len())
        && tokens.iter().all(|token| {
            token
                .trim_matches('.')
                .chars()
                .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '-' | '\'' | '.'))
        })
}

pub(super) fn extract_affiliation_map(markdown: &str) -> HashMap<String, String> {
    let mut items = HashMap::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("$ ^{") else {
            continue;
        };
        let Some((marker, text)) = rest.split_once("} $") else {
            continue;
        };
        let text = text.trim();
        if !text.is_empty() {
            let key = marker.trim().to_string();
            items
                .entry(key)
                .and_modify(|existing: &mut String| {
                    if !existing.split(" | ").any(|part| part.trim() == text) {
                        existing.push_str(" | ");
                        existing.push_str(text);
                    }
                })
                .or_insert_with(|| text.to_string());
        }
    }
    items
}

fn extract_metadata_affiliation_lines(markdown: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut after_author_line = false;
    let lines = markdown.lines().collect::<Vec<_>>();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !after_author_line {
            if looks_like_author_line(trimmed)
                || looks_like_unmarked_author_line_with_context(
                    trimmed,
                    following_nonempty_lines(&lines, index),
                )
            {
                after_author_line = true;
            }
            continue;
        }
        if trimmed.starts_with('#') || is_probable_abstract_heading(trimmed) {
            break;
        }
        if trimmed.starts_with("$ ^{")
            || trimmed.contains('@')
            || trimmed.contains(CN_AUTHOR_NOTE)
            || trimmed
                .to_ascii_lowercase()
                .contains("corresponding author")
        {
            continue;
        }
        if looks_like_affiliation_line(trimmed) {
            items.push(trimmed.to_string());
        }
        if !items.is_empty() {
            break;
        }
    }

    items
}

fn looks_like_affiliation_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("university")
        || lower.contains("college")
        || lower.contains("institute")
        || lower.contains("school")
        || lower.contains("department")
        || line.contains(CN_UNIVERSITY)
        || line.contains(CN_COLLEGE)
        || line.contains(CN_DEPARTMENT)
}

fn is_probable_abstract_heading(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    normalized == "abstract" || normalized == "summary" || line.trim() == CN_ABSTRACT
}

fn extract_correspondence_map(markdown: &str) -> HashMap<String, CorrespondenceEntry> {
    let mut entries = HashMap::new();
    let mut pending_note: Option<String> = None;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_correspondence_note_line(trimmed) {
            pending_note = Some(trimmed.to_string());
            continue;
        }
        if trimmed.contains('@') {
            collect_emails_from_line(trimmed, pending_note.clone(), &mut entries);
        }
    }
    entries
}

fn backfill_single_corresponding_author(
    authors: &mut [AcademicAuthor],
    correspondence: &HashMap<String, CorrespondenceEntry>,
) {
    if authors
        .iter()
        .filter(|author| author.markers.iter().any(|marker| marker == "*"))
        .count()
        != 1
    {
        return;
    }
    let Some(shared_email) = correspondence
        .values()
        .find_map(|entry| entry.email.clone())
    else {
        return;
    };
    let shared_note = correspondence.values().find_map(|entry| entry.note.clone());
    if let Some(author) = authors
        .iter_mut()
        .find(|author| author.markers.iter().any(|marker| marker == "*"))
    {
        if author.email.is_none() {
            author.email = Some(shared_email);
        }
        author.is_corresponding = true;
        if author.correspondence_note.is_none() {
            author.correspondence_note = shared_note;
        }
    }
}

fn collect_emails_from_line(
    line: &str,
    note: Option<String>,
    entries: &mut HashMap<String, CorrespondenceEntry>,
) {
    for email in find_emails(line) {
        for candidate_name in find_candidate_author_names(line) {
            let key = canonical_author_key(&candidate_name);
            let entry = entries.entry(key).or_default();
            entry.email = Some(email.clone());
            if entry.note.is_none() {
                entry.note = note.clone();
            }
        }
    }
}

fn is_correspondence_note_line(line: &str) -> bool {
    let normalized = line.to_ascii_lowercase();
    normalized.contains("corresponding author")
        || line.contains(CN_AUTHOR_NOTE)
        || line.starts_with("$ ^{*} $")
}

fn find_emails(line: &str) -> Vec<String> {
    line.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | CN_SEPARATOR))
        .map(|part| {
            part.trim_matches(|ch: char| matches!(ch, '(' | ')' | '<' | '>' | ':'))
                .rsplit([':', '\u{FF1A}'])
                .next()
                .unwrap_or(part)
                .trim()
        })
        .filter(|part| part.contains('@') && part.contains('.'))
        .map(ToString::to_string)
        .collect()
}

fn find_candidate_author_names(line: &str) -> Vec<String> {
    let sanitized = line
        .replace(CN_AUTHOR_NOTE, " ")
        .replace(CN_EMAIL, " ")
        .replace("Corresponding author", " ")
        .replace("Corresponding authors", " ")
        .replace("E-mail", " ")
        .replace("Email", " ");
    sanitized
        .split(|ch: char| matches!(ch, ';' | CN_SEPARATOR))
        .filter_map(|part| {
            let before_email = part.split('@').next()?.trim();
            let cleaned = before_email
                .split(':')
                .next_back()
                .unwrap_or(before_email)
                .split('(')
                .next()
                .unwrap_or(before_email)
                .trim()
                .trim_matches(|ch: char| matches!(ch, ',' | '(' | ')' | '*'));
            if cleaned.is_empty() {
                return None;
            }
            let normalized = if let Some((last, first)) = cleaned.split_once(',') {
                format!("{} {}", first.trim(), last.trim())
            } else {
                cleaned.to_string()
            };
            Some(normalized)
        })
        .collect()
}

fn parse_author_markers(raw_author: &str) -> Option<(String, Vec<String>)> {
    let trimmed = raw_author.trim();
    if let Some(parsed) = parse_math_author_markers(trimmed) {
        return Some(parsed);
    }
    parse_parenthetical_author_markers(trimmed)
        .or_else(|| parse_ocr_author_markers(trimmed))
        .or_else(|| parse_marker_glyph_author_markers(trimmed))
        .or_else(|| parse_unmarked_author_markers(trimmed))
}

fn parse_math_author_markers(trimmed: &str) -> Option<(String, Vec<String>)> {
    let marker_start = trimmed.find('$')?;
    let name = trimmed[..marker_start].trim().to_string();
    let marker_rest_start = marker_start + '$'.len_utf8();
    let marker_rest = &trimmed[marker_rest_start..];
    let marker_end = marker_rest.find('$')?;
    let markers = extract_math_markers(&marker_rest[..marker_end])?
        .split(',')
        .map(str::trim)
        .map(normalize_marker)
        .filter(|marker| !marker.is_empty())
        .collect::<Vec<_>>();
    (!name.is_empty()).then_some((name, markers))
}

fn extract_math_markers(math_content: &str) -> Option<&str> {
    let marker_start = math_content.find("^{")? + "^{".len();
    let marker_end = math_content[marker_start..].find('}')? + marker_start;
    Some(&math_content[marker_start..marker_end])
}

fn normalize_marker(marker: &str) -> String {
    marker
        .replace("\\dagger", "")
        .replace("\\ddagger", "")
        .replace("\\ast", "*")
        .replace('†', "")
        .replace('‡', "")
        .trim()
        .trim_matches(|ch: char| matches!(ch, ',' | ';'))
        .to_string()
}

fn parse_parenthetical_author_markers(trimmed: &str) -> Option<(String, Vec<String>)> {
    let open_index = trimmed.rfind(['(', CN_OPEN_PAREN])?;
    let open = trimmed[open_index..].chars().next()?;
    let close = if open == CN_OPEN_PAREN {
        CN_CLOSE_PAREN
    } else {
        ')'
    };
    let marker_start = open_index + open.len_utf8();
    let marker_end = trimmed[marker_start..].find(close)? + marker_start;
    let marker_content = trimmed[marker_start..marker_end].trim();
    if marker_content.is_empty() {
        return None;
    }

    let mut name = trimmed[..open_index].trim().to_string();
    let mut markers = marker_content
        .split([',', CN_SEPARATOR])
        .map(str::trim)
        .filter(|marker| !marker.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if let Some(stripped) = name.strip_suffix('*') {
        name = stripped.trim().to_string();
        if !markers.iter().any(|marker| marker == "*") {
            markers.push("*".to_string());
        }
    }

    (!name.is_empty() && !markers.is_empty()).then_some((name, markers))
}

fn parse_ocr_author_markers(trimmed: &str) -> Option<(String, Vec<String>)> {
    let normalized = trimmed.trim();
    if !(normalized.ends_with(" ID") || normalized.ends_with("* ID")) {
        return None;
    }
    let mut name = normalized.trim_end_matches(" ID").trim().to_string();
    let mut markers = vec!["ID".to_string()];
    if let Some(stripped) = name.strip_suffix('*') {
        name = stripped.trim().to_string();
        markers.push("*".to_string());
    }
    (!name.is_empty()).then_some((name, markers))
}

fn parse_marker_glyph_author_markers(trimmed: &str) -> Option<(String, Vec<String>)> {
    let marker_start = trimmed
        .char_indices()
        .find_map(|(index, ch)| is_author_marker_glyph(ch).then_some(index))?;
    let name = trimmed[..marker_start]
        .trim()
        .trim_end_matches([',', '&'])
        .trim()
        .to_string();
    if !looks_like_unmarked_author_segment(&name) {
        return None;
    }
    let markers = trimmed[marker_start..]
        .chars()
        .filter_map(normalize_marker_glyph)
        .collect::<Vec<_>>();
    (!markers.is_empty()).then_some((name, markers))
}

fn contains_author_marker_glyph(value: &str) -> bool {
    value.chars().any(is_author_marker_glyph)
}

fn is_author_marker_glyph(ch: char) -> bool {
    matches!(
        ch,
        '\u{00B9}'
            | '\u{00B2}'
            | '\u{00B3}'
            | '\u{2070}'
            | '\u{2074}'..='\u{2079}'
            | '\u{207B}'
            | '\u{2080}'..='\u{2089}'
            | '\u{2460}'..='\u{2473}'
            | '\u{24EA}'
    )
}

fn normalize_marker_glyph(ch: char) -> Option<String> {
    let normalized = match ch {
        '\u{00B9}' => "1",
        '\u{00B2}' => "2",
        '\u{00B3}' => "3",
        '\u{2070}' | '\u{2080}' | '\u{24EA}' => "0",
        '\u{2074}' | '\u{2084}' | '\u{2463}' => "4",
        '\u{2075}' | '\u{2085}' | '\u{2464}' => "5",
        '\u{2076}' | '\u{2086}' | '\u{2465}' => "6",
        '\u{2077}' | '\u{2087}' | '\u{2466}' => "7",
        '\u{2078}' | '\u{2088}' | '\u{2467}' => "8",
        '\u{2079}' | '\u{2089}' | '\u{2468}' => "9",
        '\u{2460}' => "1",
        '\u{2461}' => "2",
        '\u{2462}' => "3",
        '\u{2469}' => "10",
        '\u{246A}' => "11",
        '\u{246B}' => "12",
        '\u{246C}' => "13",
        '\u{246D}' => "14",
        '\u{246E}' => "15",
        '\u{246F}' => "16",
        '\u{2470}' => "17",
        '\u{2471}' => "18",
        '\u{2472}' => "19",
        '\u{2473}' => "20",
        _ => return None,
    };
    Some(normalized.to_string())
}

fn parse_unmarked_author_markers(trimmed: &str) -> Option<(String, Vec<String>)> {
    if !looks_like_unmarked_author_segment(trimmed) {
        return None;
    }
    let mut name = trimmed.trim().to_string();
    let mut markers = Vec::new();
    if let Some(stripped) = name.strip_suffix('*') {
        name = stripped.trim().to_string();
        markers.push("*".to_string());
    }
    (!name.is_empty()).then_some((name, markers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_affiliation_stops_before_abstract_body() {
        let markdown = "# Title\n\nValentin Guigon (1)\nUniversity of Lyon, France\nAbstract\nThis abstract should not be treated as affiliation metadata.\n";

        let affiliations = extract_metadata_affiliation_lines(markdown);

        assert_eq!(affiliations, vec!["University of Lyon, France".to_string()]);
    }

    #[test]
    fn parses_ocr_style_author_line_with_id_suffix() {
        let markdown = "# Title\n\nSanchayan Banerjee ID and Peter John* ID\nDepartment of Geography and Environment, London School of Economics, London, UK and Department of Political Economy, King's College London, London, UK\n$ ^{*} $Correspondence to: Department of Political Economy, King's College London, Bush House, 40 Aldwych, London WC2B 4PH, UK.\nE-mail: peter.john@kcl.ac.uk\n\n## Abstract\n\nBody.\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].name, "Sanchayan Banerjee");
        assert_eq!(authors[0].markers, vec!["ID"]);
        assert_eq!(authors[1].name, "Peter John");
        assert!(authors[1].markers.iter().any(|marker| marker == "ID"));
        assert!(authors[1].markers.iter().any(|marker| marker == "*"));
        assert_eq!(authors[1].email.as_deref(), Some("peter.john@kcl.ac.uk"));
    }

    #[test]
    fn parses_unmarked_author_line_when_followed_by_affiliation_context() {
        let markdown = "Behavioural Public Policy (2024), 8, 69\u{2013}84\n\ndoi:10.1017/bpp.2021.6\n\n# Title\n\nSanchayan Banerjee and Peter John*\nDepartment of Geography and Environment, London School of Economics, London, UK and Department of Political Economy, King's College London, London, UK\n*Correspondence to: Department of Political Economy, King's College London, Bush House, 40 Aldwych, London WC2B 4PH, UK.\nE-mail: peter.john@kcl.ac.uk\n\nAbstract\nBody.\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].name, "Sanchayan Banerjee");
        assert_eq!(authors[1].name, "Peter John");
        assert!(authors[1].markers.iter().any(|marker| marker == "*"));
        assert_eq!(authors[1].email.as_deref(), Some("peter.john@kcl.ac.uk"));
        assert!(authors.iter().all(|author| !author.affiliations.is_empty()));
    }

    #[test]
    fn parses_unicode_marker_author_line_with_commas_and_ampersand() {
        let markdown = "# Title\n\nValentin Guigon\u{00B9}\u{00B2}, Marie Claire Villeval\u{2468} \u{00B2}\u{00B3},\u{2074} & Jean-Claude Dreher\u{2469} \u{00B9},\u{2074}\n\nAbstract\nBody.\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 3);
        assert_eq!(authors[0].name, "Valentin Guigon");
        assert_eq!(authors[1].name, "Marie Claire Villeval");
        assert_eq!(authors[2].name, "Jean-Claude Dreher");
        assert!(authors[0].markers.iter().any(|marker| marker == "1"));
        assert!(authors[1].markers.iter().any(|marker| marker == "9"));
        assert!(authors[2].markers.iter().any(|marker| marker == "10"));
    }

    #[test]
    fn parses_pdf_math_markers_with_dagger_and_textcircled_prefix() {
        let markdown = "# Title\n\nAnastasia Kozyreva $ ^{1\\dagger} $, Marie Claire Villeval $ \\textcircled{D}^{2,3,4} $\n\n $ ^{1} $Center for Adaptive Rationality, Max Planck Institute for Human Development, Germany.\n\n $ ^{2} $University of Lyon, France.\n\n $ ^{3} $CNRS, France.\n\n $ ^{4} $Institute for Cognitive Science, France.\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].name, "Anastasia Kozyreva");
        assert_eq!(authors[0].markers, vec!["1"]);
        assert!(authors[0]
            .affiliations
            .iter()
            .any(|item| item.contains("Max Planck")));
        assert_eq!(authors[1].name, "Marie Claire Villeval");
        assert_eq!(authors[1].markers, vec!["2", "3", "4"]);
        assert!(authors[1]
            .affiliations
            .iter()
            .any(|item| item.contains("University of Lyon")));
    }

    #[test]
    fn parses_translated_ocr_author_line_with_cn_separator() {
        let markdown = "# Title\n\nSanchayan Banerjee ID \u{548C} Peter John* ID\n\u{82F1}\u{56FD}\u{4F26}\u{6566}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{5B66}\u{9662}\u{5730}\u{7406}\u{4E0E}\u{73AF}\u{5883}\u{7CFB}\u{FF0C}\u{4EE5}\u{53CA}\u{82F1}\u{56FD}\u{4F26}\u{6566}\u{56FD}\u{738B}\u{5B66}\u{9662}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{7CFB}\n$ ^{*} $\u{901A}\u{8BAF}\u{4F5C}\u{8005}\u{FF1A}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{7CFB}\u{FF0C}\u{56FD}\u{738B}\u{5B66}\u{9662}\u{3002}\n\u{7535}\u{5B50}\u{90AE}\u{7BB1}\u{FF1A}peter.john@kcl.ac.uk\n\n## \u{6458}\u{8981}\n\nBody.\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].name, "Sanchayan Banerjee");
        assert_eq!(authors[0].markers, vec!["ID"]);
        assert_eq!(authors[1].name, "Peter John");
        assert!(authors[1].markers.iter().any(|marker| marker == "ID"));
        assert!(authors[1].markers.iter().any(|marker| marker == "*"));
        assert!(authors[1]
            .affiliations
            .iter()
            .any(|item| item.contains("\u{56FD}\u{738B}\u{5B66}\u{9662}")));
        assert_eq!(authors[1].email.as_deref(), Some("peter.john@kcl.ac.uk"));
    }
}
