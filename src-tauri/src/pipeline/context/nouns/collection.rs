use super::super::super::chunking::{
    is_academic_article, looks_like_protected_metadata_chunk, looks_like_reference_chunk,
};
use super::rules::{
    built_in_name_seed_map, canonicalize_proper_noun_phrase, is_noise_phrase,
    proper_noun_enforcement, should_keep_proper_noun,
};
use super::{collect_capitalized_phrases, ProperNounHint};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn extract_proper_noun_hints(
    chunks: &[String],
    article_type: &str,
) -> Vec<ProperNounHint> {
    let collection = collect_proper_noun_candidates(chunks, article_type);
    let seed_map = built_in_name_seed_map();
    let mut hints = build_proper_noun_hints(collection, article_type, &seed_map);
    sort_proper_noun_hints(&mut hints);
    hints
}

#[derive(Default)]
pub(super) struct ProperNounCandidateCollection {
    ordered: Vec<String>,
    counts: BTreeMap<String, usize>,
    structural_evidence_counts: BTreeMap<String, usize>,
    first_chunk_indexes: BTreeMap<String, usize>,
    chunk_indexes: BTreeMap<String, BTreeSet<usize>>,
}

fn collect_proper_noun_candidates(
    chunks: &[String],
    article_type: &str,
) -> ProperNounCandidateCollection {
    let mut collection = ProperNounCandidateCollection::default();
    let running_title_surfaces = collect_running_title_surfaces(chunks, article_type);
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        collect_chunk_proper_nouns(
            &mut collection,
            chunk,
            chunk_index,
            article_type,
            &running_title_surfaces,
        );
    }
    collection
}

fn collect_running_title_surfaces(chunks: &[String], article_type: &str) -> BTreeSet<String> {
    if is_academic_article(article_type) {
        return BTreeSet::new();
    }
    let mut chunk_indexes_by_surface = BTreeMap::<String, BTreeSet<usize>>::new();
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        for line in chunk.lines().map(str::trim) {
            let Some(surface) = normalized_standalone_title_surface(line) else {
                continue;
            };
            chunk_indexes_by_surface
                .entry(surface)
                .or_default()
                .insert(chunk_index);
        }
    }
    chunk_indexes_by_surface
        .into_iter()
        .filter_map(|(surface, chunk_indexes)| (chunk_indexes.len() >= 3).then_some(surface))
        .collect()
}

fn collect_chunk_proper_nouns(
    collection: &mut ProperNounCandidateCollection,
    chunk: &str,
    chunk_index: usize,
    article_type: &str,
    running_title_surfaces: &BTreeSet<String>,
) {
    if is_academic_article(article_type)
        && (looks_like_reference_chunk(chunk) || looks_like_protected_metadata_chunk(chunk))
    {
        return;
    }
    let mut inside_front_matter = false;
    for line in chunk.lines().map(str::trim) {
        if line == super::super::super::chunking::FRONT_MATTER_PROTECTED_START {
            inside_front_matter = true;
            continue;
        }
        if line == super::super::super::chunking::FRONT_MATTER_PROTECTED_END {
            inside_front_matter = false;
            continue;
        }
        if inside_front_matter {
            continue;
        }
        collect_line_proper_nouns(collection, line, chunk_index, running_title_surfaces);
    }
}

fn collect_line_proper_nouns(
    collection: &mut ProperNounCandidateCollection,
    line: &str,
    chunk_index: usize,
    running_title_surfaces: &BTreeSet<String>,
) {
    if line.is_empty()
        || line.starts_with("<!--")
        || is_table_or_figure_surface(line)
        || looks_like_multi_column_layout_surface(line)
    {
        return;
    }
    if normalized_standalone_title_surface(line)
        .as_ref()
        .is_some_and(|surface| running_title_surfaces.contains(surface))
    {
        return;
    }
    let sanitized = sanitize_markdown_noise_for_noun_collection(line);
    for phrase in collect_capitalized_phrases(&sanitized) {
        push_proper_noun_candidate(
            collection,
            &phrase,
            chunk_index,
            has_structural_proper_noun_evidence(&sanitized, &phrase),
        );
    }
}

fn normalized_standalone_title_surface(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("!")
        || trimmed.starts_with('|')
        || trimmed.starts_with("<!--")
        || trimmed.contains("://")
        || trimmed.len() > 96
    {
        return None;
    }
    let without_heading = trimmed.trim_start_matches('#').trim();
    if without_heading.is_empty()
        || without_heading.contains(". ")
        || without_heading.split_whitespace().count() > 8
    {
        return None;
    }
    let sanitized = sanitize_markdown_noise_for_noun_collection(without_heading);
    let surface = sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_string();
    if surface.split_whitespace().count() < 2 || !looks_like_title_surface(&surface) {
        return None;
    }
    Some(surface.to_ascii_lowercase())
}

fn looks_like_title_surface(surface: &str) -> bool {
    let mut token_count = 0usize;
    let mut title_token_count = 0usize;
    for token in surface.split_whitespace() {
        let normalized = token.trim_matches(|ch: char| !ch.is_ascii_alphabetic());
        if normalized.is_empty() {
            continue;
        }
        token_count += 1;
        if normalized
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
            || super::PROPER_NOUN_NOISE_WORDS.contains(&normalized)
        {
            title_token_count += 1;
        }
    }
    token_count >= 2 && title_token_count == token_count
}

fn sanitize_markdown_noise_for_noun_collection(line: &str) -> String {
    let without_destinations = mask_markdown_link_destinations(line);
    without_destinations
        .trim_start_matches('#')
        .replace(['*', '_', '`', '!', '[', ']'], " ")
}

fn mask_markdown_link_destinations(line: &str) -> String {
    let chars = line.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(line.len());
    let mut index = 0usize;
    while index < chars.len() {
        if index + 1 < chars.len() && chars[index] == ']' && chars[index + 1] == '(' {
            output.push_str("] ");
            index += 2;
            while index < chars.len() && chars[index] != ')' {
                output.push(' ');
                index += 1;
            }
            if index < chars.len() {
                output.push(' ');
                index += 1;
            }
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn is_table_or_figure_surface(line: &str) -> bool {
    let trimmed = line.trim_start_matches('#').trim_start();
    starts_with_structural_label(trimmed, "Table")
        || starts_with_structural_label(trimmed, "Fig.")
        || starts_with_structural_label(trimmed, "Figure")
}

fn looks_like_multi_column_layout_surface(line: &str) -> bool {
    let cells = split_wide_space_cells(line);
    if cells.len() < 3 {
        return false;
    }
    let short_cells = cells
        .iter()
        .filter(|cell| cell.split_whitespace().count() <= 6)
        .count();
    short_cells == cells.len()
}

fn split_wide_space_cells(line: &str) -> Vec<&str> {
    let mut cells = Vec::new();
    let mut start = 0usize;
    let mut space_run_start = None;
    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            space_run_start.get_or_insert(index);
            continue;
        }
        if let Some(run_start) = space_run_start.take() {
            if index - run_start >= 2 {
                push_nonempty_cell(&mut cells, &line[start..run_start]);
                start = index;
            }
        }
    }
    push_nonempty_cell(&mut cells, &line[start..]);
    cells
}

fn push_nonempty_cell<'a>(cells: &mut Vec<&'a str>, cell: &'a str) {
    let trimmed = cell.trim();
    if !trimmed.is_empty() {
        cells.push(trimmed);
    }
}

fn starts_with_structural_label(value: &str, label: &str) -> bool {
    value
        .strip_prefix(label)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

fn push_proper_noun_candidate(
    collection: &mut ProperNounCandidateCollection,
    phrase: &str,
    chunk_index: usize,
    has_structural_evidence: bool,
) {
    let canonical = canonicalize_proper_noun_phrase(phrase);
    if is_noise_phrase(&canonical) {
        return;
    }
    let counter = collection.counts.entry(canonical.clone()).or_insert(0);
    *counter += 1;
    let is_first_occurrence = *counter == 1;
    collection
        .chunk_indexes
        .entry(canonical.clone())
        .or_default()
        .insert(chunk_index);
    if has_structural_evidence {
        *collection
            .structural_evidence_counts
            .entry(canonical.clone())
            .or_insert(0) += 1;
    }
    if is_first_occurrence {
        collection.ordered.push(canonical.clone());
        collection
            .first_chunk_indexes
            .insert(canonical, chunk_index);
    }
}

fn build_proper_noun_hints(
    collection: ProperNounCandidateCollection,
    article_type: &str,
    seed_map: &BTreeMap<&'static str, &'static str>,
) -> Vec<ProperNounHint> {
    let mut ordered = Vec::<String>::new();
    ordered.extend(collection.ordered);
    let mut hints = Vec::with_capacity(ordered.len());
    for source in ordered {
        let occurrences = collection.counts.get(&source).copied().unwrap_or(0);
        let structural_evidence_count = collection
            .structural_evidence_counts
            .get(&source)
            .copied()
            .unwrap_or(0);
        if !should_keep_proper_noun(
            &source,
            occurrences,
            structural_evidence_count,
            article_type,
            seed_map,
        ) {
            continue;
        }
        let target = seed_map
            .get(source.as_str())
            .map(|value| (*value).to_string());
        hints.push(ProperNounHint {
            enforcement: proper_noun_enforcement(&source, target.as_deref(), seed_map),
            target,
            first_chunk_index: collection
                .first_chunk_indexes
                .get(&source)
                .copied()
                .unwrap_or(0),
            chunk_indexes: collection
                .chunk_indexes
                .get(&source)
                .map(|indexes| indexes.iter().copied().collect())
                .unwrap_or_default(),
            occurrences,
            source,
        });
    }
    hints
}

fn has_structural_proper_noun_evidence(line: &str, phrase: &str) -> bool {
    if phrase.split_whitespace().count() >= 2 {
        return true;
    }
    let Some(start) = line.find(phrase) else {
        return false;
    };
    let end = start + phrase.len();
    let before = &line[..start];
    let after = &line[end..];
    if starts_inside_parenthetical_or_citation(before) || follows_citation_delimiter(after) {
        return true;
    }
    if has_adjacent_name_sequence_evidence(before, after) {
        return true;
    }
    !starts_sentence_or_line(before)
        && after
            .trim_start()
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '\'' | '’' | ',' | ';' | ')'))
}

fn starts_sentence_or_line(before: &str) -> bool {
    let trimmed = before.trim_end();
    trimmed.is_empty()
        || trimmed
            .chars()
            .rev()
            .find(|ch| !ch.is_whitespace())
            .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ':' | ';'))
}

fn starts_inside_parenthetical_or_citation(before: &str) -> bool {
    before
        .chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_some_and(|ch| matches!(ch, '(' | '['))
}

fn follows_citation_delimiter(after: &str) -> bool {
    let trimmed = after.trim_start();
    comma_followed_by_year(trimmed)
        || trimmed.starts_with(')')
        || trimmed.starts_with(']')
        || trimmed.starts_with(" et al")
        || trimmed.starts_with("’s")
        || trimmed.starts_with("'s")
}

fn comma_followed_by_year(value: &str) -> bool {
    let Some(after_comma) = value.strip_prefix(',') else {
        return false;
    };
    after_comma
        .trim_start()
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
}

fn has_adjacent_name_sequence_evidence(before: &str, after: &str) -> bool {
    let previous = immediate_prefix_allows_name_sequence(before)
        && before
            .split_whitespace()
            .rev()
            .take(3)
            .any(token_looks_like_name_part);
    let next = immediate_suffix_allows_name_sequence(after)
        && after
            .split_whitespace()
            .take(2)
            .any(token_looks_like_name_part);
    previous || next
}

fn immediate_prefix_allows_name_sequence(before: &str) -> bool {
    let tail = before
        .chars()
        .rev()
        .take(24)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if tail
        .chars()
        .any(|ch| matches!(ch, '.' | '?' | '!' | ':' | ';' | ','))
    {
        return false;
    }
    let tokens = tail.split_whitespace().collect::<Vec<_>>();
    !tokens.is_empty()
        && tokens.iter().any(|token| token_looks_like_name_part(token))
        && tokens
            .iter()
            .all(|token| token_looks_like_name_part(token) || token_is_short_connector(token))
}

fn immediate_suffix_allows_name_sequence(after: &str) -> bool {
    let head = after.chars().take(24).collect::<String>();
    !head
        .chars()
        .any(|ch| matches!(ch, '.' | '?' | '!' | ':' | ';'))
}

fn token_looks_like_name_part(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '-');
    trimmed
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
        && trimmed.chars().skip(1).any(|ch| ch.is_ascii_lowercase())
}

fn token_is_short_connector(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphabetic());
    !trimmed.is_empty() && trimmed.len() <= 3 && trimmed.chars().all(|ch| ch.is_ascii_lowercase())
}

fn sort_proper_noun_hints(hints: &mut [ProperNounHint]) {
    hints.sort_by_key(|hint| {
        (
            std::cmp::Reverse(hint.target.is_some()),
            std::cmp::Reverse(hint.occurrences),
            hint.first_chunk_index,
            hint.source.clone(),
        )
    });
}
