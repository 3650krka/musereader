use crate::document::{SourceBlock, TocArtifact, TocEntry, TocSourceKind, TranslatedBlock};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn build_translated_toc_artifact(
    source_toc: &TocArtifact,
    source_markdown: &str,
    translated_markdown: &str,
    source_blocks: &[SourceBlock],
    translated_blocks: &[TranslatedBlock],
) -> TocArtifact {
    let heading_pairs = pair_markdown_headings(
        source_markdown,
        translated_markdown,
        source_blocks,
        translated_blocks,
    );
    TocArtifact {
        source_kind: source_toc.source_kind,
        confidence: source_toc.confidence,
        entries: source_toc
            .entries
            .iter()
            // Spine 兜底条目（"Chapter N" 占位标题）不承载任何真实目录信息，
            // 却在目录里造成英文序号与中文条目交叉、序号与真实章节脱节
            // （无标题的出版社插页/章内续段文件混入目录）。生成产物时剔除；
            // 覆盖层的 removed/renames 键基于生成后的连续序号才稳定。
            .filter(|entry| entry.source_kind != TocSourceKind::EpubSpineFallback)
            .map(|entry| translated_toc_entry(entry, heading_pairs.as_ref()))
            .collect(),
    }
}

pub(crate) fn ordered_heading_anchors(markdown: &str) -> Vec<String> {
    heading_anchor_map(markdown)
        .anchors
        .into_iter()
        .map(|entry| entry.anchor)
        .collect()
}

pub(crate) fn unique_heading_anchor(
    title: &str,
    duplicate_count: usize,
    duplicate_index: usize,
    used_anchors: &mut BTreeSet<String>,
) -> String {
    let base = fallback_heading_anchor(title);
    let mut candidate = if duplicate_count > 1 {
        format!("{base}-{}", duplicate_index + 1)
    } else {
        base.clone()
    };
    let mut suffix = duplicate_index + 2;
    while !used_anchors.insert(candidate.clone()) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

pub(crate) fn fallback_heading_anchor(title: &str) -> String {
    let mut anchor = String::new();
    let mut last_was_separator = false;

    for ch in title.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            anchor.push(ch.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }
        if is_cjk_unified_ideograph(ch) {
            anchor.push(ch);
            last_was_separator = false;
            continue;
        }
        if ch.is_whitespace() || matches!(ch, '-' | '_' | '.' | ':' | '/' | '\\') {
            if !anchor.is_empty() && !last_was_separator {
                anchor.push('-');
                last_was_separator = true;
            }
        }
    }

    while anchor.ends_with('-') {
        anchor.pop();
    }
    if anchor.is_empty() {
        "section".to_string()
    } else {
        anchor
    }
}

pub(crate) fn count_exact_heading_structure_matches(
    source_markdown: &str,
    translated_markdown: &str,
) -> usize {
    let source_headings = collect_markdown_headings(source_markdown);
    let translated_headings = collect_markdown_headings(translated_markdown);
    if source_headings.len() != translated_headings.len() {
        return 0;
    }
    source_headings
        .iter()
        .zip(translated_headings.iter())
        .filter(|(source, translated)| source.level == translated.level)
        .count()
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn count_numbering_normalized_toc_matches(
    source_toc: &TocArtifact,
    source_markdown: &str,
    translated_markdown: &str,
) -> usize {
    let Some(heading_pairs) =
        pair_markdown_headings(source_markdown, translated_markdown, &[], &[])
    else {
        return 0;
    };
    source_toc
        .entries
        .iter()
        .filter(|entry| {
            let exact_key = (entry.level, normalize_heading_key(&entry.source_title));
            let normalized_key = (
                entry.level,
                normalize_numberless_heading_key(&entry.source_title),
            );
            exact_key != normalized_key
                && !heading_pairs.by_exact_key.contains_key(&exact_key)
                && heading_pairs
                    .by_normalized_key
                    .contains_key(&normalized_key)
        })
        .count()
}

fn translated_toc_entry(entry: &TocEntry, heading_pairs: Option<&HeadingPairs>) -> TocEntry {
    TocEntry {
        source_title: entry.source_title.clone(),
        translated_title: entry
            .translated_title
            .clone()
            .or_else(|| heading_pairs.and_then(|pairs| pairs.translated_title_for(entry)))
            .map(|title| normalize_translated_toc_title(&entry.source_title, &title)),
        level: entry.level,
        order: entry.order,
        page: entry.page,
        href: entry.href.clone(),
        source_kind: entry.source_kind,
        confidence: entry.confidence,
    }
}

#[derive(Debug)]
struct MarkdownHeading {
    level: u8,
    title: String,
}

#[derive(Debug)]
struct HeadingPairs {
    by_exact_key: BTreeMap<(u8, String), String>,
    by_normalized_key: BTreeMap<(u8, String), String>,
    by_exact_title: BTreeMap<String, String>,
    by_normalized_title: BTreeMap<String, String>,
}

impl HeadingPairs {
    fn translated_title_for(&self, entry: &TocEntry) -> Option<String> {
        self.translated_title_for_heading(entry.level, &entry.source_title)
            .map(ToString::to_string)
    }

    fn translated_title_for_heading(&self, level: u8, source_title: &str) -> Option<&str> {
        let exact_key = (level, normalize_heading_key(source_title));
        if let Some(title) = self.by_exact_key.get(&exact_key) {
            return Some(title.as_str());
        }
        let normalized_key = (level, normalize_numberless_heading_key(source_title));
        if let Some(title) = self.by_normalized_key.get(&normalized_key) {
            return Some(title.as_str());
        }
        let exact_title = normalize_heading_key(source_title);
        if let Some(title) = self
            .by_exact_title
            .get(&exact_title)
            .filter(|title| !title.is_empty())
        {
            return Some(title.as_str());
        }
        let normalized_title = normalize_numberless_heading_key(source_title);
        self.by_normalized_title
            .get(&normalized_title)
            .filter(|title| !title.is_empty())
            .map(String::as_str)
    }

    fn fill_missing_from(&mut self, supplemental: HeadingPairs) {
        insert_missing_pairs(&mut self.by_exact_key, supplemental.by_exact_key);
        insert_missing_pairs(&mut self.by_normalized_key, supplemental.by_normalized_key);
        insert_missing_pairs(&mut self.by_exact_title, supplemental.by_exact_title);
        insert_missing_pairs(
            &mut self.by_normalized_title,
            supplemental.by_normalized_title,
        );
    }
}

fn insert_missing_pairs<K: Ord>(target: &mut BTreeMap<K, String>, source: BTreeMap<K, String>) {
    for (key, value) in source {
        target.entry(key).or_insert(value);
    }
}

fn pair_markdown_headings(
    source_markdown: &str,
    translated_markdown: &str,
    source_blocks: &[SourceBlock],
    translated_blocks: &[TranslatedBlock],
) -> Option<HeadingPairs> {
    let block_pairs = pair_headings_from_blocks(source_blocks, translated_blocks);
    let sequence_pairs = pair_headings_from_anchored_sequence(
        source_markdown,
        translated_markdown,
        block_pairs.as_ref(),
    );
    let markdown_pairs = pair_headings_from_markdown(source_markdown, translated_markdown);
    let mut pairs = block_pairs;
    merge_heading_pairs(&mut pairs, sequence_pairs);
    merge_heading_pairs(&mut pairs, markdown_pairs);
    pairs
}

fn merge_heading_pairs(target: &mut Option<HeadingPairs>, supplemental: Option<HeadingPairs>) {
    let Some(supplemental) = supplemental else {
        return;
    };
    match target {
        Some(target) => target.fill_missing_from(supplemental),
        None => *target = Some(supplemental),
    }
}

fn pair_headings_from_markdown(
    source_markdown: &str,
    translated_markdown: &str,
) -> Option<HeadingPairs> {
    let source_headings = collect_markdown_headings(source_markdown);
    let translated_headings = collect_markdown_headings(translated_markdown);
    if source_headings.is_empty() || translated_headings.is_empty() {
        return None;
    }

    let mut exact_counts = BTreeMap::<(u8, String), usize>::new();
    let mut normalized_counts = BTreeMap::<(u8, String), usize>::new();
    for heading in &source_headings {
        *exact_counts
            .entry((heading.level, normalize_heading_key(&heading.title)))
            .or_default() += 1;
        *normalized_counts
            .entry((
                heading.level,
                normalize_numberless_heading_key(&heading.title),
            ))
            .or_default() += 1;
    }

    let mut by_exact_key = BTreeMap::new();
    let mut by_normalized_key = BTreeMap::new();
    let mut by_exact_title = BTreeMap::new();
    let mut by_normalized_title = BTreeMap::new();
    if headings_have_same_level_sequence(&source_headings, &translated_headings) {
        for (source, translated) in source_headings.iter().zip(translated_headings.iter()) {
            insert_unique_heading_pair(
                source,
                &translated.title,
                &exact_counts,
                &normalized_counts,
                &mut by_exact_key,
                &mut by_normalized_key,
                &mut by_exact_title,
                &mut by_normalized_title,
            );
        }
        return Some(HeadingPairs {
            by_exact_key,
            by_normalized_key,
            by_exact_title,
            by_normalized_title,
        });
    }

    let boundary_pair_count = insert_leading_boundary_heading_pair(
        &source_headings,
        &translated_headings,
        &exact_counts,
        &normalized_counts,
        &mut by_exact_key,
        &mut by_normalized_key,
        &mut by_exact_title,
        &mut by_normalized_title,
    );
    let mut translated_cursor = boundary_pair_count;
    for source in source_headings.iter().skip(boundary_pair_count) {
        let Some((matched_index, translated)) = translated_headings
            .iter()
            .enumerate()
            .skip(translated_cursor)
            .find(|(_, translated)| heading_titles_can_pair(source, translated))
        else {
            continue;
        };
        translated_cursor = matched_index + 1;
        insert_unique_heading_pair(
            source,
            &translated.title,
            &exact_counts,
            &normalized_counts,
            &mut by_exact_key,
            &mut by_normalized_key,
            &mut by_exact_title,
            &mut by_normalized_title,
        );
    }

    Some(HeadingPairs {
        by_exact_key,
        by_normalized_key,
        by_exact_title,
        by_normalized_title,
    })
}

fn pair_headings_from_anchored_sequence(
    source_markdown: &str,
    translated_markdown: &str,
    block_pairs: Option<&HeadingPairs>,
) -> Option<HeadingPairs> {
    let source_headings = collect_markdown_headings(source_markdown);
    let translated_headings = collect_markdown_headings(translated_markdown);
    if source_headings.is_empty() || translated_headings.is_empty() {
        return None;
    }

    let (exact_counts, normalized_counts) = source_heading_counts(&source_headings);
    let anchors = anchored_heading_pairs(
        &source_headings,
        &translated_headings,
        block_pairs,
        &exact_counts,
    );
    let mut by_exact_key = BTreeMap::new();
    let mut by_normalized_key = BTreeMap::new();
    let mut by_exact_title = BTreeMap::new();
    let mut by_normalized_title = BTreeMap::new();
    let mut inserted = false;
    let mut source_cursor = 0;
    let mut translated_cursor = 0;

    for (source_index, translated_index) in anchors {
        let source_window = &source_headings[source_cursor..source_index];
        let translated_window = &translated_headings[translated_cursor..translated_index];
        let window_inserted = insert_sequential_heading_window(
            source_window,
            translated_window,
            &exact_counts,
            &normalized_counts,
            &mut by_exact_key,
            &mut by_normalized_key,
            &mut by_exact_title,
            &mut by_normalized_title,
        );
        if !window_inserted {
            let _ = insert_confirmed_leading_boundary_heading_pair(
                source_window,
                translated_window,
                &exact_counts,
                &normalized_counts,
                &mut by_exact_key,
                &mut by_normalized_key,
                &mut by_exact_title,
                &mut by_normalized_title,
            );
        }
        insert_unique_heading_pair(
            &source_headings[source_index],
            &translated_headings[translated_index].title,
            &exact_counts,
            &normalized_counts,
            &mut by_exact_key,
            &mut by_normalized_key,
            &mut by_exact_title,
            &mut by_normalized_title,
        );
        inserted = true;
        source_cursor = source_index + 1;
        translated_cursor = translated_index + 1;
    }

    inserted |= insert_sequential_heading_window(
        &source_headings[source_cursor..],
        &translated_headings[translated_cursor..],
        &exact_counts,
        &normalized_counts,
        &mut by_exact_key,
        &mut by_normalized_key,
        &mut by_exact_title,
        &mut by_normalized_title,
    );

    inserted.then_some(HeadingPairs {
        by_exact_key,
        by_normalized_key,
        by_exact_title,
        by_normalized_title,
    })
}

fn source_heading_counts(
    source_headings: &[MarkdownHeading],
) -> (BTreeMap<(u8, String), usize>, BTreeMap<(u8, String), usize>) {
    let mut exact_counts = BTreeMap::<(u8, String), usize>::new();
    let mut normalized_counts = BTreeMap::<(u8, String), usize>::new();
    for heading in source_headings {
        *exact_counts
            .entry((heading.level, normalize_heading_key(&heading.title)))
            .or_default() += 1;
        *normalized_counts
            .entry((
                heading.level,
                normalize_numberless_heading_key(&heading.title),
            ))
            .or_default() += 1;
    }
    (exact_counts, normalized_counts)
}

fn anchored_heading_pairs(
    source_headings: &[MarkdownHeading],
    translated_headings: &[MarkdownHeading],
    block_pairs: Option<&HeadingPairs>,
    exact_counts: &BTreeMap<(u8, String), usize>,
) -> Vec<(usize, usize)> {
    let Some(block_pairs) = block_pairs else {
        return Vec::new();
    };
    let translated_title_counts = translated_heading_title_counts(translated_headings);
    let mut anchors = Vec::new();
    let mut translated_cursor = 0;

    for (source_index, source) in source_headings.iter().enumerate() {
        let source_key = (source.level, normalize_heading_key(&source.title));
        if exact_counts.get(&source_key) != Some(&1) {
            continue;
        }
        let Some(block_title) =
            block_pairs.translated_title_for_heading(source.level, &source.title)
        else {
            continue;
        };
        let translated_key = normalize_heading_key(block_title);
        if translated_title_counts.get(&translated_key) != Some(&1) {
            continue;
        }
        let Some(relative_index) =
            translated_headings[translated_cursor..]
                .iter()
                .position(|translated| {
                    translated.level == source.level
                        && normalize_heading_key(&translated.title) == translated_key
                })
        else {
            continue;
        };
        let translated_index = translated_cursor + relative_index;
        anchors.push((source_index, translated_index));
        translated_cursor = translated_index + 1;
    }

    anchors
}

fn translated_heading_title_counts(
    translated_headings: &[MarkdownHeading],
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::<String, usize>::new();
    for heading in translated_headings {
        *counts
            .entry(normalize_heading_key(&heading.title))
            .or_default() += 1;
    }
    counts
}

fn insert_sequential_heading_window(
    source_headings: &[MarkdownHeading],
    translated_headings: &[MarkdownHeading],
    exact_counts: &BTreeMap<(u8, String), usize>,
    normalized_counts: &BTreeMap<(u8, String), usize>,
    by_exact_key: &mut BTreeMap<(u8, String), String>,
    by_normalized_key: &mut BTreeMap<(u8, String), String>,
    by_exact_title: &mut BTreeMap<String, String>,
    by_normalized_title: &mut BTreeMap<String, String>,
) -> bool {
    if source_headings.is_empty() || source_headings.len() != translated_headings.len() {
        return false;
    }
    if !source_headings
        .iter()
        .zip(translated_headings.iter())
        .all(|(source, translated)| source.level == translated.level)
    {
        return false;
    }
    for (source, translated) in source_headings.iter().zip(translated_headings.iter()) {
        insert_unique_heading_pair(
            source,
            &translated.title,
            exact_counts,
            normalized_counts,
            by_exact_key,
            by_normalized_key,
            by_exact_title,
            by_normalized_title,
        );
    }
    true
}

fn insert_confirmed_leading_boundary_heading_pair(
    source_headings: &[MarkdownHeading],
    translated_headings: &[MarkdownHeading],
    exact_counts: &BTreeMap<(u8, String), usize>,
    normalized_counts: &BTreeMap<(u8, String), usize>,
    by_exact_key: &mut BTreeMap<(u8, String), String>,
    by_normalized_key: &mut BTreeMap<(u8, String), String>,
    by_exact_title: &mut BTreeMap<String, String>,
    by_normalized_title: &mut BTreeMap<String, String>,
) -> bool {
    if source_headings.len() <= translated_headings.len() {
        return false;
    }
    let Some(source) = source_headings.first() else {
        return false;
    };
    let Some(translated) = translated_headings.first() else {
        return false;
    };
    if !leading_boundary_heading_is_safe(source, translated, exact_counts, normalized_counts) {
        return false;
    }
    if !translated_heading_candidate_is_plausible(&source.title, &translated.title) {
        return false;
    }
    insert_unique_heading_pair(
        source,
        &translated.title,
        exact_counts,
        normalized_counts,
        by_exact_key,
        by_normalized_key,
        by_exact_title,
        by_normalized_title,
    );
    true
}

fn insert_leading_boundary_heading_pair(
    source_headings: &[MarkdownHeading],
    translated_headings: &[MarkdownHeading],
    exact_counts: &BTreeMap<(u8, String), usize>,
    normalized_counts: &BTreeMap<(u8, String), usize>,
    by_exact_key: &mut BTreeMap<(u8, String), String>,
    by_normalized_key: &mut BTreeMap<(u8, String), String>,
    by_exact_title: &mut BTreeMap<String, String>,
    by_normalized_title: &mut BTreeMap<String, String>,
) -> usize {
    let Some(source) = source_headings.first() else {
        return 0;
    };
    let Some(translated) = translated_headings.first() else {
        return 0;
    };
    if translated_headings.len() < source_headings.len() {
        return 0;
    }
    if !leading_boundary_heading_is_safe(source, translated, exact_counts, normalized_counts) {
        return 0;
    }
    insert_unique_heading_pair(
        source,
        &translated.title,
        exact_counts,
        normalized_counts,
        by_exact_key,
        by_normalized_key,
        by_exact_title,
        by_normalized_title,
    );
    1
}

fn leading_boundary_heading_is_safe(
    source: &MarkdownHeading,
    translated: &MarkdownHeading,
    exact_counts: &BTreeMap<(u8, String), usize>,
    normalized_counts: &BTreeMap<(u8, String), usize>,
) -> bool {
    if source.level != translated.level {
        return false;
    }
    let exact_key = (source.level, normalize_heading_key(&source.title));
    let normalized_key = (
        source.level,
        normalize_numberless_heading_key(&source.title),
    );
    exact_counts.get(&exact_key) == Some(&1) || normalized_counts.get(&normalized_key) == Some(&1)
}

fn headings_have_same_level_sequence(
    source_headings: &[MarkdownHeading],
    translated_headings: &[MarkdownHeading],
) -> bool {
    source_headings.len() == translated_headings.len()
        && source_headings
            .iter()
            .zip(translated_headings.iter())
            .all(|(source, translated)| source.level == translated.level)
}

fn insert_unique_heading_pair(
    source: &MarkdownHeading,
    translated_title: &str,
    exact_counts: &BTreeMap<(u8, String), usize>,
    normalized_counts: &BTreeMap<(u8, String), usize>,
    by_exact_key: &mut BTreeMap<(u8, String), String>,
    by_normalized_key: &mut BTreeMap<(u8, String), String>,
    by_exact_title: &mut BTreeMap<String, String>,
    by_normalized_title: &mut BTreeMap<String, String>,
) {
    let exact_key = (source.level, normalize_heading_key(&source.title));
    if exact_counts.get(&exact_key) == Some(&1) {
        by_exact_key.insert(exact_key, translated_title.to_string());
    }
    let normalized_key = (
        source.level,
        normalize_numberless_heading_key(&source.title),
    );
    if normalized_counts.get(&normalized_key) == Some(&1) {
        by_normalized_key.insert(normalized_key, translated_title.to_string());
    }
    insert_unique_title_pair(
        &source.title,
        translated_title,
        by_exact_title,
        normalize_heading_key,
    );
    insert_unique_title_pair(
        &source.title,
        translated_title,
        by_normalized_title,
        normalize_numberless_heading_key,
    );
}

fn pair_headings_from_blocks(
    source_blocks: &[SourceBlock],
    translated_blocks: &[TranslatedBlock],
) -> Option<HeadingPairs> {
    let mut by_exact_key = BTreeMap::new();
    let mut by_normalized_key = BTreeMap::new();
    let mut by_exact_title = BTreeMap::new();
    let mut by_normalized_title = BTreeMap::new();
    let mut exact_counts = BTreeMap::<(u8, String), usize>::new();
    let mut normalized_counts = BTreeMap::<(u8, String), usize>::new();

    let heading_rows = source_blocks
        .iter()
        .zip(translated_blocks.iter())
        .filter(|(source, _)| source.kind == "heading")
        .filter_map(|(source, translated)| {
            let level = heading_level_from_markdown(&source.markdown);
            let source_title = source.text.trim().to_string();
            let translated_title = translated_heading_title_from_block(&source_title, translated)?;
            Some((level, source_title, translated_title))
        })
        .filter(|(level, source_title, translated_title)| {
            *level > 0 && !source_title.is_empty() && !translated_title.is_empty()
        })
        .collect::<Vec<_>>();

    if heading_rows.is_empty() {
        return None;
    }

    for (level, source_title, _) in &heading_rows {
        *exact_counts
            .entry((*level, normalize_heading_key(source_title)))
            .or_default() += 1;
        *normalized_counts
            .entry((*level, normalize_numberless_heading_key(source_title)))
            .or_default() += 1;
    }

    for (level, source_title, translated_title) in heading_rows {
        let exact_key = (level, normalize_heading_key(&source_title));
        if exact_counts.get(&exact_key) == Some(&1) {
            by_exact_key.insert(exact_key, translated_title.clone());
        }
        let normalized_key = (level, normalize_numberless_heading_key(&source_title));
        if normalized_counts.get(&normalized_key) == Some(&1) {
            by_normalized_key.insert(normalized_key, translated_title.clone());
        }
        insert_unique_title_pair(
            &source_title,
            &translated_title,
            &mut by_exact_title,
            normalize_heading_key,
        );
        insert_unique_title_pair(
            &source_title,
            &translated_title,
            &mut by_normalized_title,
            normalize_numberless_heading_key,
        );
    }

    Some(HeadingPairs {
        by_exact_key,
        by_normalized_key,
        by_exact_title,
        by_normalized_title,
    })
}

fn insert_unique_title_pair(
    source_title: &str,
    translated_title: &str,
    target: &mut BTreeMap<String, String>,
    normalize: impl Fn(&str) -> String,
) {
    let key = normalize(source_title);
    if key.is_empty() {
        return;
    }
    match target.get(&key) {
        None => {
            target.insert(key, translated_title.to_string());
        }
        Some(existing) if !existing.is_empty() => {
            target.insert(key, String::new());
        }
        Some(_) => {}
    }
}

fn heading_titles_can_pair(source: &MarkdownHeading, translated: &MarkdownHeading) -> bool {
    if source.level != translated.level {
        return false;
    }
    let source_exact = normalize_heading_key(&source.title);
    let translated_exact = normalize_heading_key(&translated.title);
    if source_exact == translated_exact {
        return true;
    }
    let source_normalized = normalize_numberless_heading_key(&source.title);
    let translated_normalized = normalize_numberless_heading_key(&translated.title);
    source_normalized == translated_normalized
}

fn collect_markdown_headings(markdown: &str) -> Vec<MarkdownHeading> {
    markdown
        .lines()
        .filter_map(parse_markdown_heading)
        .collect()
}

fn parse_markdown_heading(line: &str) -> Option<MarkdownHeading> {
    let trimmed = line.trim_start();
    let marker_count = trimmed.chars().take_while(|ch| *ch == '#').count();
    if marker_count == 0 || marker_count > 6 {
        return None;
    }
    let rest = trimmed.get(marker_count..)?;
    if !rest.starts_with(' ') {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim();
    if title.is_empty() {
        return None;
    }
    Some(MarkdownHeading {
        level: marker_count as u8,
        title: title.to_string(),
    })
}

fn heading_level_from_markdown(markdown: &str) -> u8 {
    parse_markdown_heading(markdown)
        .map(|heading| heading.level)
        .unwrap_or_default()
}

fn translated_heading_title_from_block(
    source_title: &str,
    translated: &TranslatedBlock,
) -> Option<String> {
    if let Some(heading) = first_markdown_heading(&translated.translated_markdown)
        .filter(|heading| translated_heading_candidate_is_plausible(source_title, &heading.title))
    {
        return Some(heading.title);
    }
    let title = translated.translated_text.trim();
    translated_heading_candidate_is_plausible(source_title, title).then(|| title.to_string())
}

fn first_markdown_heading(markdown: &str) -> Option<MarkdownHeading> {
    markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(parse_markdown_heading)
}

fn translated_heading_candidate_is_plausible(source_title: &str, translated_title: &str) -> bool {
    let title = translated_title.trim();
    if title.is_empty() || title.lines().filter(|line| !line.trim().is_empty()).count() > 1 {
        return false;
    }
    let char_count = title.chars().filter(|ch| !ch.is_whitespace()).count();
    if char_count == 0 || char_count > 160 {
        return false;
    }
    let source_char_count = source_title
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count()
        .max(1);
    if sentence_terminator_count(title) >= 2 && char_count > 32 {
        return false;
    }
    let relative_limit = source_char_count
        .saturating_mul(2)
        .saturating_add(18)
        .max(48);
    if heading_ends_with_sentence_terminator(title)
        && char_count > relative_limit
        && contains_clause_punctuation(title)
    {
        return false;
    }
    !(char_count > source_char_count.saturating_mul(5).saturating_add(32) && char_count > 96)
}

fn sentence_terminator_count(title: &str) -> usize {
    let strong_count = title
        .chars()
        .filter(|ch| matches!(ch, '。' | '！' | '？' | '!' | '?'))
        .count();
    strong_count + ascii_sentence_period_count(title)
}

fn ascii_sentence_period_count(title: &str) -> usize {
    title
        .char_indices()
        .filter(|(index, ch)| {
            if *ch != '.' {
                return false;
            }
            title
                .get(index + ch.len_utf8()..)
                .and_then(|rest| rest.chars().next())
                .is_none_or(char::is_whitespace)
        })
        .count()
}

fn heading_ends_with_sentence_terminator(title: &str) -> bool {
    title
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '。' | '！' | '？' | '!' | '?' | '.'))
}

fn contains_clause_punctuation(title: &str) -> bool {
    title
        .chars()
        .any(|ch| matches!(ch, ',' | '，' | ';' | '；'))
}

fn normalize_heading_key(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_numberless_heading_key(title: &str) -> String {
    let normalized = normalize_heading_key(title);
    strip_leading_numbering(&normalized).to_string()
}

fn strip_leading_numbering(title: &str) -> &str {
    let trimmed = title.trim_start();
    if let Some(stripped) = strip_appendix_numbering(trimmed) {
        return stripped;
    }
    let stripped = trimmed.trim_start_matches(|ch: char| {
        ch.is_ascii_digit()
            || ch == '.'
            || ch == ')'
            || ch == '('
            || ch == '-'
            || ch == ':'
            || ch.is_whitespace()
    });
    if stripped.is_empty() {
        trimmed
    } else {
        stripped
    }
}

fn strip_appendix_numbering(title: &str) -> Option<&str> {
    let (prefix, rest) = title.split_once(char::is_whitespace)?;
    let mut chars = prefix.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || !chars.as_str().starts_with('.') {
        return None;
    }
    let suffix = chars.as_str().trim_start_matches('.');
    if suffix.is_empty()
        || !suffix
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
    {
        return None;
    }
    let rest = rest.trim_start();
    (!rest.is_empty()).then_some(rest)
}

// ---- 目录条目译文归一化（序号体系 / 冒号一致性）----
//
// LLM 译出的章节标题偶发两类不一致（用户可见症状）：
// 1. 序号体系混用——同一目录里「第11章」与「第十四章」并存；
// 2. 冒号不一致——「第11章：标题」「第11章:标题」「第11章 标题」混用。
// 条目数量与层级由 prompt 约束（见 SKILL.md「目录与章节标题条目」），
// 这里只做确定性的轻量后处理：序号格式跟随源条目体系、冒号归一为全角，
// 其余文字与页码/锚点等引用信息一律不动。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChapterNumberStyle {
    Arabic,
    Chinese,
}

/// 「第N章/节/篇/部/卷/回」前缀（阿拉伯或中文数字均捕获）。
static CHAPTER_PREFIX_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^(第\s*)([0-9〇零一二两三四五六七八九十百千]+)(\s*[章节篇部卷回])")
        .expect("chapter prefix regex should compile")
});

/// 英文源条目「Chapter 11」式序号——视为阿拉伯数字体系。
static ENGLISH_CHAPTER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^chapter\s+[0-9]+").expect("chapter regex should compile")
});

/// 判定标题前导序号体系：仅区分「第N章」式阿拉伯/中文数字与 Chapter N。
fn chapter_numbering_style(title: &str) -> Option<ChapterNumberStyle> {
    let trimmed = title.trim_start();
    if let Some(captures) = CHAPTER_PREFIX_RE.captures(trimmed) {
        let numeral = captures.get(2)?.as_str();
        return if numeral.chars().all(|ch| ch.is_ascii_digit()) {
            Some(ChapterNumberStyle::Arabic)
        } else {
            Some(ChapterNumberStyle::Chinese)
        };
    }
    ENGLISH_CHAPTER_RE
        .is_match(trimmed)
        .then_some(ChapterNumberStyle::Arabic)
}

/// 序号格式归一：按源条目序号体系统一译文前导「第N[章节…]」的数字写法。
/// 源用阿拉伯数字则译文转阿拉伯，源用中文数字则译文转中文；其余文字不动。
fn normalize_chapter_number_style(source_title: &str, title: &str) -> String {
    match (
        chapter_numbering_style(source_title),
        chapter_numbering_style(title),
    ) {
        (Some(ChapterNumberStyle::Arabic), Some(ChapterNumberStyle::Chinese)) => {
            convert_chapter_numeral_to_arabic(title).unwrap_or_else(|| title.to_string())
        }
        (Some(ChapterNumberStyle::Chinese), Some(ChapterNumberStyle::Arabic)) => {
            convert_chapter_numeral_to_chinese(title).unwrap_or_else(|| title.to_string())
        }
        _ => title.to_string(),
    }
}

fn convert_chapter_numeral_to_arabic(title: &str) -> Option<String> {
    let captures = CHAPTER_PREFIX_RE.captures(title)?;
    let numeral = captures.get(2)?.as_str();
    let value = chinese_numeral_value(numeral)?;
    let mut normalized = String::new();
    normalized.push_str(captures.get(1)?.as_str());
    normalized.push_str(&value.to_string());
    normalized.push_str(captures.get(3)?.as_str());
    normalized.push_str(title.get(captures.get(0)?.end()..)?);
    Some(normalized)
}

fn convert_chapter_numeral_to_chinese(title: &str) -> Option<String> {
    let captures = CHAPTER_PREFIX_RE.captures(title)?;
    let numeral = captures.get(2)?.as_str();
    let value: u32 = numeral.parse().ok()?;
    let chinese = arabic_to_chinese_numeral(value)?;
    let mut normalized = String::new();
    normalized.push_str(captures.get(1)?.as_str());
    normalized.push_str(&chinese);
    normalized.push_str(captures.get(3)?.as_str());
    normalized.push_str(title.get(captures.get(0)?.end()..)?);
    Some(normalized)
}

/// 中文数字解析：支持 〇零一二两三四五六七八九十百千（目录序号常见范围）。
fn chinese_numeral_value(text: &str) -> Option<u32> {
    let mut total: u32 = 0;
    let mut current: u32 = 0;
    let mut matched = false;
    for ch in text.chars() {
        let unit = match ch {
            '十' => 10,
            '百' => 100,
            '千' => 1000,
            _ => 0,
        };
        if unit > 0 {
            matched = true;
            let base = if current == 0 { 1 } else { current };
            total = total.checked_add(base.checked_mul(unit)?)?;
            current = 0;
            continue;
        }
        let digit = match ch {
            '〇' | '零' => 0,
            '一' => 1,
            '两' | '二' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            _ => return None,
        };
        matched = true;
        current = current.checked_mul(10)?.checked_add(digit)?;
    }
    matched.then(|| total.saturating_add(current))
}

/// 阿拉伯数字转中文数字（1..=9999，目录章节序号的现实范围）。
fn arabic_to_chinese_numeral(value: u32) -> Option<String> {
    if value == 0 || value > 9999 {
        return None;
    }
    const DIGITS: [&str; 10] = [
        "零", "一", "二", "三", "四", "五", "六", "七", "八", "九",
    ];
    let (qian, remainder) = (value / 1000, value % 1000);
    let (bai, remainder) = (remainder / 100, remainder % 100);
    let (shi, ge) = (remainder / 10, remainder % 10);
    let mut out = String::new();
    if qian > 0 {
        out.push_str(DIGITS[qian as usize]);
        out.push('千');
    }
    if bai > 0 {
        out.push_str(DIGITS[bai as usize]);
        out.push('百');
    } else if qian > 0 && (shi > 0 || ge > 0) {
        out.push('零');
    }
    if shi > 0 {
        // 「十一」而非「一十一」：十位为 1 且无更高位时省略前导「一」。
        if !(shi == 1 && qian == 0 && bai == 0) {
            out.push_str(DIGITS[shi as usize]);
        }
        out.push('十');
    } else if bai > 0 && ge > 0 {
        out.push('零');
    }
    if ge > 0 {
        out.push_str(DIGITS[ge as usize]);
    }
    Some(out)
}

/// 冒号归一：
/// - 源条目含冒号：译文冒号统一为全角「：」；若译文缺冒号，补在章节序号前缀之后；
/// - 源条目无冒号：移除译文在章节序号前缀后多加的冒号，标题其余位置的冒号不动。
fn normalize_entry_colon(source_title: &str, title: &str) -> String {
    let source_has_colon = source_title.contains(':') || source_title.contains('：');
    if source_has_colon {
        // 冒号统一为全角，并去掉冒号后的多余空格（「第11章: 塔什」→「第11章：塔什」）。
        let normalized = title
            .replace(':', "：")
            .split('：')
            .enumerate()
            .map(|(index, part)| {
                if index == 0 {
                    part.to_string()
                } else {
                    format!("：{}", part.trim_start())
                }
            })
            .collect::<String>();
        if normalized.contains('：') || !CHAPTER_PREFIX_RE.is_match(&normalized) {
            return normalized;
        }
        return insert_colon_after_chapter_prefix(&normalized);
    }
    // 源无冒号：只移除章节序号前缀后紧跟的冒号，避免误伤标题其他位置的冒号。
    drop_colon_after_chapter_prefix(title)
}

fn insert_colon_after_chapter_prefix(title: &str) -> String {
    let Some(captures) = CHAPTER_PREFIX_RE.captures(title) else {
        return title.to_string();
    };
    let Some(prefix_match) = captures.get(0) else {
        return title.to_string();
    };
    let rest = title
        .get(prefix_match.end()..)
        .unwrap_or_default()
        .trim_start();
    if rest.is_empty() {
        return title.to_string();
    }
    format!("{}：{rest}", &title[..prefix_match.end()])
}

fn drop_colon_after_chapter_prefix(title: &str) -> String {
    let Some(captures) = CHAPTER_PREFIX_RE.captures(title) else {
        return title.to_string();
    };
    let Some(prefix_match) = captures.get(0) else {
        return title.to_string();
    };
    let prefix_end = prefix_match.end();
    let rest = title.get(prefix_end..).unwrap_or_default().trim_start();
    let stripped = rest
        .strip_prefix('：')
        .or_else(|| rest.strip_prefix(':'));
    let Some(stripped) = stripped else {
        return title.to_string();
    };
    let stripped = stripped.trim_start();
    if stripped.is_empty() {
        return title[..prefix_end].trim_end().to_string();
    }
    format!("{} {stripped}", &title[..prefix_end])
}

/// 目录条目译文标题归一化入口：序号体系跟随源条目，冒号统一全角。
fn normalize_translated_toc_title(source_title: &str, translated_title: &str) -> String {
    let normalized = normalize_chapter_number_style(source_title, translated_title);
    normalize_entry_colon(source_title, &normalized)
}

#[derive(Debug, Clone)]
struct HeadingAnchorEntry {
    anchor: String,
}

#[derive(Debug)]
struct HeadingAnchorMap {
    anchors: Vec<HeadingAnchorEntry>,
}

fn heading_anchor_map(markdown: &str) -> HeadingAnchorMap {
    let headings = collect_markdown_headings(markdown);
    heading_anchor_map_from_entries(
        &headings
            .iter()
            .map(|heading| (heading.level, heading.title.clone()))
            .collect::<Vec<_>>(),
    )
}

fn heading_anchor_map_from_entries(entries: &[(u8, String)]) -> HeadingAnchorMap {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut seen = BTreeMap::<String, usize>::new();
    for (_, title) in entries {
        *counts.entry(title.clone()).or_default() += 1;
    }

    let mut anchors = Vec::with_capacity(entries.len());
    let mut used_anchors = BTreeSet::new();
    for (_, title) in entries {
        let next_index = seen.entry(title.clone()).or_default();
        let anchor = unique_heading_anchor(
            title,
            counts.get(title).copied().unwrap_or(1),
            *next_index,
            &mut used_anchors,
        );
        *next_index += 1;
        anchors.push(HeadingAnchorEntry { anchor });
    }

    HeadingAnchorMap { anchors }
}

fn is_cjk_unified_ideograph(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::TocSourceKind;
    use std::path::Path;

    #[test]
    fn translated_toc_preserves_structural_fields() {
        let source_toc = TocArtifact {
            source_kind: TocSourceKind::PdfOutline,
            confidence: 0.9,
            entries: vec![TocEntry {
                source_title: "1 Introduction".to_string(),
                translated_title: None,
                level: 2,
                order: 4,
                page: Some(7),
                href: Some("chapter.xhtml".to_string()),
                source_kind: TocSourceKind::PdfOutline,
                confidence: 0.9,
            }],
        };

        let translated_toc = build_translated_toc_artifact(&source_toc, "", "", &[], &[]);

        assert_eq!(translated_toc.entries.len(), source_toc.entries.len());
        assert_eq!(translated_toc.entries[0].source_title, "1 Introduction");
        assert_eq!(translated_toc.entries[0].translated_title, None);
        assert_eq!(translated_toc.entries[0].level, 2);
        assert_eq!(translated_toc.entries[0].order, 4);
        assert_eq!(translated_toc.entries[0].page, Some(7));
        assert_eq!(
            translated_toc.entries[0].href.as_deref(),
            Some("chapter.xhtml")
        );
        assert_eq!(
            translated_toc.entries[0].source_kind,
            TocSourceKind::PdfOutline
        );
    }

    #[test]
    fn translated_toc_backfills_titles_from_matching_headings() {
        let source_toc = toc_with_entries(vec![entry("Introduction", 1), entry("Methods", 2)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Introduction\n\n## Methods",
            "# 寮曡█\n\n## 鏂规硶",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("寮曡█")
        );
        assert_eq!(
            translated_toc.entries[1].translated_title.as_deref(),
            Some("鏂规硶")
        );
    }

    #[test]
    fn translated_toc_allows_numbering_normalized_match() {
        let source_toc = toc_with_entries(vec![entry("Introduction", 1)]);
        let translated_toc =
            build_translated_toc_artifact(&source_toc, "# 1. Introduction", "# 1. 寮曡█", &[], &[]);

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("1. 寮曡█")
        );
    }

    #[test]
    fn translated_toc_backfills_pdf_outline_when_markdown_level_differs() {
        let source_toc = toc_with_entries(vec![entry("Introduction", 1), entry("Methods", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "## 1 Introduction\n\n## 4 Methods",
            "## 1 引言\n\n## 4 方法",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("1 引言")
        );
        assert_eq!(
            translated_toc.entries[1].translated_title.as_deref(),
            Some("4 方法")
        );
    }

    #[test]
    fn translated_toc_backfills_appendix_numbered_headings() {
        let source_toc = toc_with_entries(vec![entry("Selection of Interventions", 2)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "## A.1 Selection of Interventions",
            "## A.1 干预措施的筛选",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("A.1 干预措施的筛选")
        );
    }

    #[test]
    fn translated_toc_leaves_duplicate_heading_blank() {
        let source_toc = toc_with_entries(vec![entry("Results", 2)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "## Results\n\n## Results",
            "## 结果一\n\n## 结果二",
            &[],
            &[],
        );

        assert_eq!(translated_toc.entries[0].translated_title, None);
    }

    #[test]
    fn translated_toc_leaves_duplicate_numberless_heading_blank() {
        let source_toc = toc_with_entries(vec![entry("Boosts and educational interventions", 2)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "## Boosts and educational interventions\n\n## Boosts and educational interventions\n\n## Boosts and educational interventions",
            "## 提升策略一\n\n## 提升策略二\n\n## 提升策略三",
            &[],
            &[],
        );

        assert_eq!(translated_toc.entries[0].translated_title, None);
    }

    #[test]
    fn translated_toc_ignores_protected_front_matter_blocks_without_translated_title() {
        let source_toc = toc_with_entries(vec![entry("Abstract", 2), entry("References", 2)]);
        let source_blocks = vec![
            source_block(
                0,
                "heading",
                "<!-- musetranslate:front-matter:start -->\n# English Title",
                "English Title",
            ),
            source_block(1, "heading", "## Abstract", "Abstract"),
            source_block(2, "heading", "## References", "References"),
        ];
        let translated_blocks = vec![
            translated_block(&source_blocks[0], ""),
            translated_block(&source_blocks[1], "## 摘要"),
            translated_block(&source_blocks[2], "## 参考文献"),
        ];

        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# English Title\n\n## Abstract\n\n## References",
            "# 中文标题\n\n## 摘要\n\n## 参考文献",
            &source_blocks,
            &translated_blocks,
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("摘要")
        );
        assert_eq!(
            translated_toc.entries[1].translated_title.as_deref(),
            Some("参考文献")
        );
    }

    #[test]
    fn translated_toc_backfills_front_matter_root_title_from_markdown_headings() {
        let source_toc = toc_with_entries(vec![
            entry("English Title", 1),
            entry("Abstract", 2),
            entry("References", 2),
        ]);
        let source_blocks = vec![
            source_block(
                0,
                "heading",
                "<!-- musetranslate:front-matter:start -->\n# English Title",
                "English Title",
            ),
            source_block(1, "heading", "## Abstract", "Abstract"),
            source_block(2, "heading", "## References", "References"),
        ];
        let translated_blocks = vec![
            translated_block(&source_blocks[0], ""),
            translated_block(&source_blocks[1], "## 摘要"),
            translated_block(&source_blocks[2], "## 参考文献"),
        ];

        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# English Title\n\n## Abstract\n\n## References",
            "# 中文标题\n\n## 摘要\n\n## 参考文献",
            &source_blocks,
            &translated_blocks,
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("中文标题")
        );
        assert_eq!(
            translated_toc.entries[1].translated_title.as_deref(),
            Some("摘要")
        );
        assert_eq!(
            translated_toc.entries[2].translated_title.as_deref(),
            Some("参考文献")
        );
    }

    #[test]
    fn translated_toc_backfills_root_when_heading_counts_differ() {
        let source_toc = toc_with_entries(vec![entry("Introduction", 1), entry("References", 2)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Introduction\n\n## References",
            "# 引言\n\n## 参考文献\n\n## 额外标题",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("引言")
        );
        assert_eq!(translated_toc.entries[1].translated_title, None);
    }

    #[test]
    fn translated_toc_rejects_paragraph_like_block_heading_candidate() {
        let source_toc = toc_with_entries(vec![entry("The Tale of Satampra Zeiros", 1)]);
        let source_blocks = vec![source_block(
            0,
            "heading",
            "# The Tale of Satampra Zeiros",
            "The Tale of Satampra Zeiros",
        )];
        let translated_blocks = vec![translated_block(
            &source_blocks[0],
            "我，乌祖尔达罗姆的萨坦普拉·塞罗斯，将用我的左手——因我已再无别的手可用了——书写下提鲁夫·翁帕利奥斯与我本人在查图格伽神龛中遭遇的一切。",
        )];

        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# The Tale of Satampra Zeiros",
            "我，乌祖尔达罗姆的萨坦普拉·塞罗斯，将用我的左手——因我已再无别的手可用了——书写下提鲁夫·翁帕利奥斯与我本人在查图格伽神龛中遭遇的一切。",
            &source_blocks,
            &translated_blocks,
        );

        assert_eq!(translated_toc.entries[0].translated_title, None);
    }

    #[test]
    fn translated_toc_does_not_pair_leading_heading_when_translation_has_fewer_headings() {
        let source_toc = toc_with_entries(vec![entry("Introduction", 1), entry("A Note", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Introduction\n\n# A Note",
            "# 文本说明",
            &[],
            &[],
        );

        assert_eq!(translated_toc.entries[0].translated_title, None);
        assert_eq!(translated_toc.entries[1].translated_title, None);
    }

    #[test]
    fn translated_toc_backfills_leading_title_when_later_block_anchor_confirms_offset() {
        let source_toc = toc_with_entries(vec![
            entry(
                "Metacognition biases information seeking in assessing ambiguous news",
                1,
            ),
            entry("Methods", 2),
            entry("Participants", 2),
        ]);
        let source_blocks = vec![
            source_block(
                0,
                "heading",
                "# Metacognition biases information seeking in assessing ambiguous news",
                "Metacognition biases information seeking in assessing ambiguous news",
            ),
            source_block(1, "heading", "## Methods", "Methods"),
            source_block(2, "heading", "## Participants", "Participants"),
        ];
        let translated_blocks = vec![
            translated_block(&source_blocks[0], ""),
            translated_block(&source_blocks[1], ""),
            translated_block(&source_blocks[2], "## 参与者"),
        ];

        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Metacognition biases information seeking in assessing ambiguous news\n\n## Methods\n\n## Participants",
            "# 元认知偏差影响评估模糊新闻时的信息寻求行为\n\n## 参与者",
            &source_blocks,
            &translated_blocks,
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("元认知偏差影响评估模糊新闻时的信息寻求行为")
        );
        assert_eq!(translated_toc.entries[1].translated_title, None);
        assert_eq!(
            translated_toc.entries[2].translated_title.as_deref(),
            Some("参与者")
        );
    }

    #[test]
    fn translated_toc_recovers_trailing_markdown_heading_after_rejected_block_candidate() {
        let source_toc = toc_with_entries(vec![
            entry("Introduction", 1),
            entry("Appendix Four: Bibliography", 1),
            entry("About The Editors", 1),
        ]);
        let source_blocks = vec![
            source_block(0, "heading", "# Introduction", "Introduction"),
            source_block(
                1,
                "heading",
                "# Appendix Four: Bibliography",
                "Appendix Four: Bibliography",
            ),
            source_block(2, "heading", "# About The Editors", "About The Editors"),
        ];
        let translated_blocks = vec![
            translated_block(&source_blocks[0], "# 序言"),
            translated_block(&source_blocks[1], "# 附录四：参考书目"),
            translated_block(
                &source_blocks[2],
                "罗恩·希尔格是一位著名的史密斯学者，曾共同编辑过史密斯文集北极星的红色世界和《星光变幻》。作为北加州本地人，罗恩致力于保护史密斯的遗产。",
            ),
        ];

        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Introduction\n\n# Appendix Four: Bibliography\n\n# About The Editors",
            "# 附录四：参考书目\n\n# 关于编者",
            &source_blocks,
            &translated_blocks,
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("序言")
        );
        assert_eq!(
            translated_toc.entries[1].translated_title.as_deref(),
            Some("附录四：参考书目")
        );
        assert_eq!(
            translated_toc.entries[2].translated_title.as_deref(),
            Some("关于编者")
        );
    }

    #[test]
    fn exact_heading_structure_count_requires_equal_heading_counts() {
        assert_eq!(
            count_exact_heading_structure_matches("# One\n\n## Two", "# 一\n\n## 二"),
            2
        );
        assert_eq!(
            count_exact_heading_structure_matches("# One", "# 一\n\n## 二"),
            0
        );
    }

    #[test]
    fn ordered_heading_anchors_preserve_heading_order_and_duplicate_suffixes() {
        let anchors = ordered_heading_anchors("# 引言\n\n## 方法\n\n## 方法\n\n# Results");

        assert_eq!(anchors, vec!["引言", "方法-1", "方法-2", "results"]);
    }

    #[test]
    fn fallback_heading_anchor_keeps_cjk_and_normalizes_ascii_text() {
        assert_eq!(fallback_heading_anchor("1. 引言 / Intro"), "1-引言-intro");
        assert_eq!(fallback_heading_anchor("***"), "section");
    }

    #[test]
    fn rebuilds_translated_toc_from_env_artifact_dir() {
        let Ok(dir) = std::env::var("MUSETRANSLATE_TOC_REBUILD_ARTIFACT_DIR") else {
            return;
        };
        let dir = Path::new(&dir);
        let source_toc = read_artifact_json::<TocArtifact>(&dir.join("source_toc.json"));
        let source_markdown =
            std::fs::read_to_string(dir.join("source.md")).expect("source.md should read");
        let translated_markdown =
            std::fs::read_to_string(dir.join("translated.md")).expect("translated.md should read");
        let source_blocks: Vec<SourceBlock> = read_artifact_json(&dir.join("source_blocks.json"));
        let translated_blocks: Vec<TranslatedBlock> =
            read_artifact_json(&dir.join("translated_blocks.json"));

        let rebuilt = build_translated_toc_artifact(
            &source_toc,
            &source_markdown,
            &translated_markdown,
            &source_blocks,
            &translated_blocks,
        );

        if let Some(expected_titles) = std::env::var("MUSETRANSLATE_TOC_EXPECT_FILLED").ok() {
            for expected in expected_titles
                .split('|')
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                let entry = rebuilt
                    .entries
                    .iter()
                    .find(|entry| entry.source_title == expected)
                    .unwrap_or_else(|| panic!("expected TOC entry should exist: {expected}"));
                assert!(
                    entry
                        .translated_title
                        .as_deref()
                        .is_some_and(|title| !title.trim().is_empty()),
                    "expected TOC title should be filled: {expected}"
                );
            }
        }

        if let Some(satampra) = rebuilt
            .entries
            .iter()
            .find(|entry| entry.source_title == "The Tale of Satampra Zeiros")
        {
            assert!(
                satampra
                    .translated_title
                    .as_deref()
                    .is_none_or(|title| title.chars().count() < 48),
                "Satampra TOC title must not be a body paragraph"
            );
        }
        if let Some(editors) = rebuilt
            .entries
            .iter()
            .find(|entry| entry.source_title == "About The Editors")
        {
            assert_eq!(editors.translated_title.as_deref(), Some("关于编者"));
        }

        if std::env::var("MUSETRANSLATE_TOC_REBUILD_WRITE")
            .ok()
            .as_deref()
            == Some("1")
        {
            std::fs::write(
                dir.join("translated_toc.json"),
                serde_json::to_string_pretty(&rebuilt).expect("rebuilt TOC should serialize"),
            )
            .expect("rebuilt translated_toc.json should write");
        }
    }

    #[test]
    fn translated_toc_normalizes_chinese_numeral_to_arabic_when_source_uses_arabic() {
        let source_toc = toc_with_entries(vec![entry("Chapter 11: Tash", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Chapter 11: Tash",
            "# 第十一章：塔什",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("第11章：塔什")
        );
    }

    #[test]
    fn translated_toc_normalizes_arabic_numeral_to_chinese_when_source_uses_chinese() {
        let source_toc = toc_with_entries(vec![entry("第十一章 旧事", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# 第十一章 旧事",
            "# 第11章：旧事",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("第十一章 旧事")
        );
    }

    /// 真实事故回归：EPUB 中无标题的出版社插页/章内续段文件会拿到
    /// "Chapter N" spine 兜底标题，混入目录后英文序号与中文条目交叉，
    /// 且让序号与真实章节脱节。产物构建必须剔除这类占位条目并重编号。
    #[test]
    fn translated_toc_drops_spine_fallback_placeholder_entries() {
        let source_toc = TocArtifact {
            source_kind: TocSourceKind::EpubNcx,
            confidence: 0.95,
            entries: vec![
                TocEntry {
                    source_title: "Chapter 1: Tash".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 0,
                    page: None,
                    href: Some("ch01.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.95,
                },
                TocEntry {
                    source_title: "Chapter 6".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 1,
                    page: None,
                    href: Some("ch01_sub01.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubSpineFallback,
                    confidence: 0.45,
                },
                TocEntry {
                    source_title: "Chapter 2: Tash".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 2,
                    page: None,
                    href: Some("ch02.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.95,
                },
            ],
        };
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Chapter 1: Tash\n\nbody\n\n# Chapter 2: Tash",
            "# 第一章：塔什\n\n正文\n\n# 第二章：塔什",
            &[],
            &[],
        );

        assert_eq!(translated_toc.entries.len(), 2, "兜底占位条目应被剔除");
        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("第1章：塔什")
        );
        assert_eq!(
            translated_toc.entries[1].translated_title.as_deref(),
            Some("第2章：塔什"),
            "剔除后序号重编号，标题配对仍应按源标题定位"
        );
    }

    #[test]
    fn translated_toc_unifies_half_width_colon_to_full_width() {
        let source_toc = toc_with_entries(vec![entry("Chapter 11: Tash", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Chapter 11: Tash",
            "# 第11章: 塔什",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("第11章：塔什")
        );
    }

    #[test]
    fn translated_toc_inserts_missing_colon_when_source_has_colon() {
        let source_toc = toc_with_entries(vec![entry("Chapter 13: The Battle", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Chapter 13: The Battle",
            "# 第十三章 战争",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("第13章：战争")
        );
    }

    #[test]
    fn translated_toc_drops_colon_when_source_has_none() {
        let source_toc = toc_with_entries(vec![entry("Chapter 12 Reepicheep", 1)]);
        let translated_toc = build_translated_toc_artifact(
            &source_toc,
            "# Chapter 12 Reepicheep",
            "# 第十二章：雷佩契普",
            &[],
            &[],
        );

        assert_eq!(
            translated_toc.entries[0].translated_title.as_deref(),
            Some("第12章 雷佩契普")
        );
    }

    #[test]
    fn chapter_numeral_roundtrip_covers_common_toc_ranges() {
        assert_eq!(chinese_numeral_value("十一"), Some(11));
        assert_eq!(chinese_numeral_value("十四"), Some(14));
        assert_eq!(chinese_numeral_value("二十三"), Some(23));
        assert_eq!(chinese_numeral_value("一百零三"), Some(103));
        assert_eq!(chinese_numeral_value("九百九十九"), Some(999));
        assert_eq!(arabic_to_chinese_numeral(11), Some("十一".to_string()));
        assert_eq!(arabic_to_chinese_numeral(23), Some("二十三".to_string()));
        assert_eq!(arabic_to_chinese_numeral(103), Some("一百零三".to_string()));
        assert_eq!(arabic_to_chinese_numeral(999), Some("九百九十九".to_string()));
    }

    fn read_artifact_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{} should read: {error}", path.display()));
        serde_json::from_str(&content)
            .unwrap_or_else(|error| panic!("{} should parse: {error}", path.display()))
    }

    fn toc_with_entries(entries: Vec<TocEntry>) -> TocArtifact {
        TocArtifact {
            source_kind: TocSourceKind::HeadingInferred,
            confidence: 0.7,
            entries,
        }
    }

    fn entry(title: &str, level: u8) -> TocEntry {
        TocEntry {
            source_title: title.to_string(),
            translated_title: None,
            level,
            order: 0,
            page: None,
            href: None,
            source_kind: TocSourceKind::HeadingInferred,
            confidence: 0.7,
        }
    }

    fn source_block(order: usize, kind: &str, markdown: &str, text: &str) -> SourceBlock {
        SourceBlock {
            id: format!("block-{order:04}"),
            order,
            kind: kind.to_string(),
            markdown: markdown.to_string(),
            text: text.to_string(),
            translate: true,
            heading_path: Vec::new(),
            chapter_title: None,
            href: None,
            page: None,
        }
    }

    fn translated_block(source: &SourceBlock, markdown: &str) -> TranslatedBlock {
        TranslatedBlock {
            id: source.id.clone(),
            order: source.order,
            chunk_index: None,
            marker_id: None,
            alignment_method: String::new(),
            kind: source.kind.clone(),
            source_markdown: source.markdown.clone(),
            translated_markdown: markdown.to_string(),
            source_text: source.text.clone(),
            translated_text: markdown.trim_start_matches('#').trim().to_string(),
            translate: source.translate,
            heading_path: source.heading_path.clone(),
            chapter_title: source.chapter_title.clone(),
            href: source.href.clone(),
            page: source.page,
        }
    }
}
