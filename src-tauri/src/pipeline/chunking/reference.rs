use super::academic::looks_like_protected_metadata_chunk;
use super::{is_academic_article, split_markdown};

pub(crate) fn split_reference_aware_markdown(
    text: &str,
    chunk_size: usize,
    article_type: &str,
) -> Vec<String> {
    let Some(reference_start) = find_reference_section_byte_start(text) else {
        return split_non_reference_text(text, chunk_size, article_type);
    };
    split_text_with_reference_section(text, reference_start, chunk_size, article_type)
}

pub(crate) fn should_skip_residual_review(article_type: &str, source: &str) -> bool {
    looks_like_reference_chunk(source)
        || looks_like_back_matter_metadata_chunk(source)
        || (is_academic_article(article_type) && looks_like_protected_metadata_chunk(source))
}

pub(crate) fn looks_like_reference_chunk(text: &str) -> bool {
    if is_reference_heading_chunk(text) {
        return true;
    }
    let starts_like_reference =
        first_nonempty_line(text).is_some_and(looks_like_reference_entry_line);
    starts_like_reference
        && !contains_substantive_non_reference_prose(text)
        && !has_reference_breaking_body_intro(text)
}

pub(crate) fn starts_with_non_reference_tail_heading(text: &str) -> bool {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| is_non_reference_tail_heading(line.trim()))
}

pub(crate) fn starts_reference_section(text: &str) -> bool {
    is_reference_heading_chunk(text)
}

pub(crate) fn is_non_reference_tail_heading(line: &str) -> bool {
    if !line.trim_start().starts_with('#') {
        return false;
    }
    let normalized = line
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    !is_reference_heading_line(line)
        && !matches!(
            normalized.as_str(),
            "of special interest" | "of outstanding interest"
        )
}

fn split_non_reference_text(text: &str, chunk_size: usize, article_type: &str) -> Vec<String> {
    if is_academic_article(article_type) {
        return split_academic_body_and_metadata_chunks(text, chunk_size);
    }
    split_markdown(text, chunk_size)
}

fn split_text_with_reference_section(
    text: &str,
    reference_start: usize,
    chunk_size: usize,
    article_type: &str,
) -> Vec<String> {
    let (body, reference_and_tail) = text.split_at(reference_start);
    let (references, tail) = split_reference_section_and_tail(reference_and_tail);
    let mut chunks = split_non_reference_text(body.trim_end(), chunk_size, article_type);
    chunks.extend(split_markdown(references.trim_start(), chunk_size));
    append_reference_tail_chunks(&mut chunks, tail, chunk_size, article_type);
    chunks
}

fn append_reference_tail_chunks(
    chunks: &mut Vec<String>,
    tail: &str,
    chunk_size: usize,
    article_type: &str,
) {
    if tail.trim().is_empty() {
        return;
    }
    chunks.extend(split_non_reference_text(
        tail.trim_start(),
        chunk_size,
        article_type,
    ));
}

fn split_reference_section_and_tail(reference_and_tail: &str) -> (&str, &str) {
    let mut byte_index = 0usize;
    let mut seen_reference_heading = false;
    for line in reference_and_tail.split_inclusive('\n') {
        let trimmed = line.trim();
        if !seen_reference_heading {
            seen_reference_heading = is_reference_heading_line(trimmed);
            byte_index = byte_index.saturating_add(line.len());
            continue;
        }
        if is_non_reference_tail_heading(trimmed) {
            return reference_and_tail.split_at(byte_index);
        }
        byte_index = byte_index.saturating_add(line.len());
    }
    (reference_and_tail, "")
}

fn find_reference_section_byte_start(text: &str) -> Option<usize> {
    let mut byte_index = 0usize;
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n').trim_end_matches('\r');
        if is_reference_heading_line(line_without_newline.trim()) {
            return Some(byte_index);
        }
        byte_index = byte_index.saturating_add(line.len());
    }
    None
}

fn is_reference_heading_line(line: &str) -> bool {
    let normalized = line
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "references" | "bibliography" | "works cited" | "literature cited"
    )
}

fn looks_like_back_matter_metadata_chunk(text: &str) -> bool {
    first_nonempty_line(text)
        .filter(|line| line.trim_start().starts_with('#'))
        .is_some_and(is_back_matter_metadata_heading)
}

fn is_back_matter_metadata_heading(line: &str) -> bool {
    let normalized = line
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "abbreviations"
            | "abbreviation"
            | "acknowledgements"
            | "acknowledgments"
            | "bibliography"
            | "works cited"
            | "about the author"
            | "about the editor"
            | "copyright"
            | "credits"
    )
}

fn is_reference_heading_chunk(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .any(is_reference_heading_line)
}

fn looks_like_reference_entry_line(line: &str) -> bool {
    if line.len() < 24 {
        return false;
    }
    if starts_with_numbered_reference_marker(line) {
        return has_reference_year_signal(line)
            || has_reference_publication_signal(line)
            || line.to_ascii_lowercase().contains("doi.org");
    }
    if !starts_with_author_year_reference_prefix(line) {
        return false;
    }
    let has_year = has_reference_year_signal(line);
    let has_publication_marker = has_reference_publication_signal(line);
    let has_author_punctuation = line.contains(", ") && line.contains('.');
    has_year && has_author_punctuation && has_publication_marker
}

fn contains_substantive_non_reference_prose(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.len() < 120 {
            return false;
        }
        if trimmed.starts_with('#') {
            return false;
        }
        if looks_like_reference_entry_line(trimmed) {
            return false;
        }
        if starts_with_numbered_reference_marker(trimmed) {
            return false;
        }
        let sentence_punct = ['.', '?', '!', ';', ':']
            .into_iter()
            .filter(|mark| trimmed.contains(*mark))
            .count();
        let word_count = trimmed
            .split_whitespace()
            .filter(|token| token.chars().any(|ch| ch.is_ascii_alphabetic()))
            .count();
        sentence_punct >= 2 && word_count >= 20
    })
}

fn split_academic_body_and_metadata_chunks(text: &str, chunk_size: usize) -> Vec<String> {
    super::academic::split_academic_body_and_metadata_chunks(text, chunk_size)
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn has_reference_year_signal(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    line.contains("(19")
        || line.contains("(20")
        || lower.contains(" 19")
        || lower.contains(" 20")
        || lower.contains("(forthcoming)")
        || lower.contains("(in press)")
}

fn has_reference_publication_signal(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "doi",
        "doi.org",
        "journal",
        "vol.",
        "pp.",
        "proc ",
        "proceedings",
        "press",
        "commun",
        "psychol",
        "science",
        "acad",
        "reports",
        "letters",
        "routledge",
        "springer",
        "wiley",
        "elsevier",
        "sage",
        "cambridge",
        "oxford",
        "nature",
        "plos",
        "eds.",
        "(ed.)",
        "(eds.)",
        "memory and language",
        "public interest",
        "north-holland",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn starts_with_numbered_reference_marker(line: &str) -> bool {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        let digits = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if digits > 0 {
            return rest[digits..].starts_with(']');
        }
    }
    let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return false;
    }
    trimmed[digit_count..].starts_with(". ") || trimmed[digit_count..].starts_with(") ")
}

fn starts_with_author_year_reference_prefix(line: &str) -> bool {
    let trimmed = line.trim_start();
    let prefix = trimmed.chars().take(96).collect::<String>();
    let comma_index = prefix.find(',').unwrap_or(usize::MAX);
    let year_index = prefix
        .find("(19")
        .or_else(|| prefix.find("(20"))
        .unwrap_or(usize::MAX);
    let first_period_index = prefix.find('.').unwrap_or(usize::MAX);
    comma_index <= 40 && year_index <= 72 && first_period_index <= 90
}

fn has_reference_breaking_body_intro(text: &str) -> bool {
    let mut seen_reference_entry = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if looks_like_reference_entry_line(trimmed) {
            seen_reference_entry = true;
            continue;
        }
        if !seen_reference_entry {
            let word_count = trimmed.split_whitespace().count();
            if word_count >= 12 && has_sentence_punctuation(trimmed) {
                return true;
            }
        }
    }
    false
}

fn has_sentence_punctuation(text: &str) -> bool {
    ['.', '?', '!', ';', ':']
        .into_iter()
        .any(|mark| text.contains(mark))
}

#[cfg(test)]
mod tests {
    use super::{looks_like_reference_chunk, should_skip_residual_review};

    #[test]
    fn does_not_treat_long_body_prose_as_reference_chunk() {
        let chunk = "Nudge plus is a modification of the toolkit of behavioral public policy. It incorporates an element of reflection into the delivery of a nudge, either blended in or made proximate. Thaler, R. H. and C. R. Sunstein (2009), Nudge, Yale University Press.\n\nThese objections are addressed in an alternate program of think, which implies that debate and deliberation can help individuals achieve their objectives. The individual has to spend considerable time they may not be willing to give, and it relies on a strong commitment.";

        assert!(
            !looks_like_reference_chunk(chunk),
            "substantive body prose must not be classified as references"
        );
    }

    #[test]
    fn treats_numbered_reference_chunk_as_reference() {
        let chunk = concat!(
            "[22] Kozyreva, A., Lewandowsky, S., Hertwig, R.: Citizens versus the internet: ",
            "Confronting digital challenges with cognitive tools. Psychological Science in the Public Interest 21(3), 103-156 (2020) ",
            "https://doi.org/10.1177/1529100620946707\n",
            "[23] Guess, A.M., Lerner, M., Lyons, B., Montgomery, J.M.: A digital media literacy intervention increases discernment ",
            "between mainstream and false news in the United States and India. Proceedings of the National Academy of Sciences 117(27), 15536-15545 (2020)"
        );

        assert!(looks_like_reference_chunk(chunk));
    }

    #[test]
    fn treats_author_year_reference_chunk_as_reference_with_books_and_editors() {
        let chunk = concat!(
            "Edwards, W. (1983), 'Human cognitive capabilities, representativeness, and ground rules for research', ",
            "In P. Humphreys, O. Svenson, and A. Vari (eds), Analysing and aiding decision processes, Advances in psychology, Vol. 14, Amsterdam, North-Holland.\n",
            "Jacoby, L. L. (1991), 'A process dissociation framework: Separating automatic from intentional uses of memory', Journal of Memory and Language, 30(5): 513-541."
        );

        assert!(looks_like_reference_chunk(chunk));
    }

    #[test]
    fn skips_back_matter_metadata_for_residual_review_without_skipping_appendix_notes() {
        assert!(should_skip_residual_review(
            "fiction",
            "# Abbreviations\n\nHPL: Howard Phillips Lovecraft\nCAS: Clark Ashton Smith"
        ));
        assert!(!should_skip_residual_review(
            "fiction",
            "# Appendix One: Story Notes\n\nThis note explains why the narrator changed the ending."
        ));
    }
}
