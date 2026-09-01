use super::*;
use crate::pipeline::context::normalize_resource_term_key;
use std::collections::BTreeMap;

pub(crate) fn build_glossary_artifact(
    article_type: &str,
    checkpoint: &ChunkCheckpoint,
) -> GlossaryArtifact {
    let indexed_chunks = IndexedSourceChunks::new(checkpoint);
    let mut entries = collect_glossary_entries(checkpoint, &indexed_chunks);

    entries.sort_by(|left, right| {
        right
            .matched_chunk_count
            .cmp(&left.matched_chunk_count)
            .then_with(|| right.occurrences.cmp(&left.occurrences))
            .then_with(|| left.first_chunk_index.cmp(&right.first_chunk_index))
            .then_with(|| left.source.cmp(&right.source))
    });
    entries.dedup_by(|left, right| left.source == right.source);

    GlossaryArtifact {
        article_type: article_type.to_string(),
        grouped_hits: build_grouped_hits(article_type, &entries),
        entries,
    }
}

fn collect_glossary_entries(
    checkpoint: &ChunkCheckpoint,
    indexed_chunks: &IndexedSourceChunks,
) -> Vec<GlossaryEntry> {
    let Some(context) = checkpoint.translation_context.as_ref() else {
        return Vec::new();
    };
    let mut entries = BTreeMap::<(String, String, String, String), GlossaryEntry>::new();

    for entry in &context.static_terms {
        let stats = indexed_chunks.count_hint_occurrences(&entry.source);
        let glossary_entry = GlossaryEntry {
            source: entry.source.clone(),
            translation: entry.target.trim().to_string(),
            category: static_glossary_category(entry).to_string(),
            occurrences: stats.occurrences,
            first_chunk_index: stats.first_chunk_index,
            matched_chunk_count: stats.matched_chunk_count,
            heading_groups: stats.heading_groups,
            origin: "static".to_string(),
        };
        entries.insert(glossary_entry_key(&glossary_entry), glossary_entry);
    }

    for chunk in &checkpoint.chunks {
        let Some(translated) = chunk.translated.as_deref() else {
            continue;
        };
        for term in &chunk.confirmed_terms {
            let Some(term) = crate::pipeline::consistency::sanitize_dynamic_confirmed_term(
                term,
                translated,
                &checkpoint.article_type,
            ) else {
                continue;
            };
            let stats =
                indexed_chunks.count_confirmed_term_occurrences(&term.source, chunk.index, context);
            let glossary_entry = GlossaryEntry {
                source: term.source.clone(),
                translation: term.translation.clone(),
                category: dynamic_glossary_category(&term).to_string(),
                occurrences: stats.occurrences,
                first_chunk_index: stats.first_chunk_index,
                matched_chunk_count: stats.matched_chunk_count,
                heading_groups: stats.heading_groups,
                origin: "dynamic".to_string(),
            };
            entries
                .entry(glossary_entry_key(&glossary_entry))
                .and_modify(|existing| merge_glossary_entry(existing, &glossary_entry))
                .or_insert(glossary_entry);
        }
    }

    entries.into_values().collect()
}

fn dynamic_glossary_category(term: &crate::llm::ConfirmedTerm) -> &'static str {
    match term.term_type.as_str() {
        "person" => "person_name",
        "location" => "place_name",
        "organization" => "organization_name",
        "title" => "title",
        "coinage" | "disputed" => "strict_term",
        _ => "strict_term",
    }
}

fn static_glossary_category(entry: &StaticGlossaryEntry) -> &'static str {
    if entry.enforcement.trim().eq_ignore_ascii_case("contextual") {
        "contextual_term"
    } else {
        "strict_term"
    }
}

fn glossary_entry_key(entry: &GlossaryEntry) -> (String, String, String, String) {
    (
        normalize_resource_term_key(&entry.source),
        entry.translation.clone(),
        entry.category.clone(),
        entry.origin.clone(),
    )
}

fn merge_glossary_entry(existing: &mut GlossaryEntry, incoming: &GlossaryEntry) {
    existing.occurrences = existing.occurrences.max(incoming.occurrences);
    existing.first_chunk_index = existing.first_chunk_index.min(incoming.first_chunk_index);
    existing.matched_chunk_count = existing
        .matched_chunk_count
        .max(incoming.matched_chunk_count);
    let mut headings = existing.heading_groups.clone();
    headings.extend(incoming.heading_groups.clone());
    existing.heading_groups = unique_non_empty(headings);
}

fn build_grouped_hits(article_type: &str, entries: &[GlossaryEntry]) -> Vec<GlossaryHitGroup> {
    let mut grouped = BTreeMap::<String, Vec<GlossaryHitStat>>::new();
    for entry in entries {
        let headings = if entry.heading_groups.is_empty() {
            vec![fallback_heading(article_type)]
        } else {
            entry.heading_groups.clone()
        };
        for heading in headings {
            grouped.entry(heading).or_default().push(GlossaryHitStat {
                source: entry.source.clone(),
                translation: entry.translation.clone(),
                hit_count: entry.occurrences,
                matched_chunk_count: entry.matched_chunk_count,
                first_chunk_index: entry.first_chunk_index,
                category: entry.category.clone(),
                origin: entry.origin.clone(),
            });
        }
    }

    grouped
        .into_iter()
        .map(|(heading, mut entries)| {
            entries.sort_by(|left, right| {
                right
                    .hit_count
                    .cmp(&left.hit_count)
                    .then_with(|| right.matched_chunk_count.cmp(&left.matched_chunk_count))
                    .then_with(|| left.first_chunk_index.cmp(&right.first_chunk_index))
                    .then_with(|| left.source.cmp(&right.source))
            });
            entries.dedup_by(|left, right| {
                left.source == right.source
                    && left.translation == right.translation
                    && left.origin == right.origin
            });
            GlossaryHitGroup { heading, entries }
        })
        .collect()
}

fn fallback_heading(article_type: &str) -> String {
    if crate::article_policy::ArticlePolicy::from_article_type(article_type).is_academic() {
        "Ungrouped".to_string()
    } else {
        "Body".to_string()
    }
}

struct IndexedSourceChunks<'a> {
    chunks: Vec<IndexedSourceChunk<'a>>,
}

struct IndexedSourceChunk<'a> {
    index: usize,
    source: &'a str,
    source_key: String,
    heading_group: String,
}

impl<'a> IndexedSourceChunks<'a> {
    fn new(checkpoint: &'a ChunkCheckpoint) -> Self {
        let mut current_heading = "Ungrouped".to_string();
        let chunks = checkpoint
            .chunks
            .iter()
            .map(|chunk| {
                update_heading_group(&mut current_heading, &chunk.source);
                IndexedSourceChunk {
                    index: chunk.index,
                    source: &chunk.source,
                    source_key: normalize_resource_term_key(&chunk.source),
                    heading_group: current_heading.clone(),
                }
            })
            .collect();
        Self { chunks }
    }

    fn count_hint_occurrences(&self, needle: &str) -> HintOccurrenceStats {
        let mut occurrences = 0usize;
        let mut first_chunk_index = usize::MAX;
        let mut matched_chunk_count = 0usize;
        let mut heading_groups = Vec::new();
        let needle_key = normalize_resource_term_key(needle);
        for chunk in &self.chunks {
            let count = chunk.source.matches(needle).count();
            let matched =
                count > 0 || normalized_source_mentions_hint(&chunk.source_key, &needle_key);
            if matched {
                occurrences += count;
                first_chunk_index = first_chunk_index.min(chunk.index);
                matched_chunk_count += 1;
                heading_groups.push(chunk.heading_group.clone());
            }
        }
        HintOccurrenceStats {
            occurrences: occurrences.max(1),
            first_chunk_index: if first_chunk_index == usize::MAX {
                0
            } else {
                first_chunk_index
            },
            matched_chunk_count,
            heading_groups: unique_non_empty(heading_groups),
        }
    }

    fn count_confirmed_term_occurrences(
        &self,
        needle: &str,
        confirmed_chunk_index: usize,
        context: &TranslationContext,
    ) -> HintOccurrenceStats {
        let needle_key = normalize_resource_term_key(needle);
        let indexed_hint = context
            .proper_nouns
            .iter()
            .find(|hint| normalize_resource_term_key(&hint.source) == needle_key);
        if let Some(hint) = indexed_hint {
            let indexes = if hint.chunk_indexes.is_empty() {
                vec![hint.first_chunk_index]
            } else {
                hint.chunk_indexes.clone()
            };
            return self.stats_from_chunk_indexes(&indexes, hint.occurrences.max(1));
        }
        self.stats_from_chunk_indexes(&[confirmed_chunk_index], 1)
    }

    fn stats_from_chunk_indexes(
        &self,
        chunk_indexes: &[usize],
        fallback_occurrences: usize,
    ) -> HintOccurrenceStats {
        let mut first_chunk_index = usize::MAX;
        let mut matched_chunk_count = 0usize;
        let mut heading_groups = Vec::new();
        for chunk_index in chunk_indexes {
            let Some(chunk) = self.chunks.iter().find(|chunk| chunk.index == *chunk_index) else {
                continue;
            };
            first_chunk_index = first_chunk_index.min(chunk.index);
            matched_chunk_count += 1;
            heading_groups.push(chunk.heading_group.clone());
        }
        HintOccurrenceStats {
            occurrences: fallback_occurrences.max(matched_chunk_count).max(1),
            first_chunk_index: if first_chunk_index == usize::MAX {
                0
            } else {
                first_chunk_index
            },
            matched_chunk_count,
            heading_groups: unique_non_empty(heading_groups),
        }
    }
}

fn normalized_source_mentions_hint(source_key: &str, needle_key: &str) -> bool {
    !needle_key.is_empty() && source_key.contains(needle_key)
}

fn update_heading_group(current: &mut String, source: &str) {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim();
            if !heading.is_empty() {
                *current = heading.to_string();
            }
        }
    }
}

fn unique_non_empty(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        if value.trim().is_empty() || deduped.iter().any(|existing| existing == &value) {
            continue;
        }
        deduped.push(value);
    }
    deduped
}

struct HintOccurrenceStats {
    occurrences: usize,
    first_chunk_index: usize,
    matched_chunk_count: usize,
    heading_groups: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ConfirmedTerm;
    use crate::pipeline::{
        ChunkCheckpoint, ChunkSegmentKind, ChunkState, ProperNounEnforcement, ProperNounHint,
        StaticGlossaryEntry, TranslationContext,
    };

    #[test]
    fn grouped_hits_follow_real_heading_groups() {
        let checkpoint = sample_checkpoint();
        let glossary = build_glossary_artifact("academic", &checkpoint);

        assert_eq!(glossary.grouped_hits.len(), 2);
        assert_eq!(glossary.grouped_hits[0].heading, "Abstract");
        assert_eq!(glossary.grouped_hits[1].heading, "Methods");
        assert!(glossary.grouped_hits[0]
            .entries
            .iter()
            .any(|entry| entry.source == "deliberation" && entry.origin == "static"));
        assert!(glossary.grouped_hits[1]
            .entries
            .iter()
            .any(|entry| entry.source == "MiniMax" && entry.origin == "dynamic"));
    }

    #[test]
    fn glossary_entries_rank_by_matched_chunks_then_hits() {
        let checkpoint = sample_checkpoint();
        let glossary = build_glossary_artifact("academic", &checkpoint);

        assert_eq!(glossary.entries[0].source, "deliberation");
        assert_eq!(glossary.entries[0].matched_chunk_count, 2);
        assert_eq!(glossary.entries[1].source, "MiniMax");
        assert_eq!(glossary.entries[1].matched_chunk_count, 1);
    }

    #[test]
    fn glossary_groups_symbolic_term_variants_under_confirmed_dynamic_term() {
        let mut checkpoint = sample_checkpoint();
        checkpoint.chunks.push(chunk_with_terms(
            3,
            "# Discussion\n\nThe nudge+ instrument stayed visible.",
            Some("## Discussion\n\nThe target-device stayed visible."),
            vec![ConfirmedTerm {
                source: "nudge plus instrument".to_string(),
                translation: "target-device".to_string(),
                term_type: "coinage".to_string(),
                confidence: 95,
                usage_role: "coinage_noun".to_string(),
            }],
        ));

        let glossary = build_glossary_artifact("academic", &checkpoint);

        assert!(glossary.entries.iter().any(|entry| {
            entry.source == "nudge plus instrument"
                && entry.translation == "target-device"
                && entry.matched_chunk_count >= 1
        }));
        assert!(glossary.grouped_hits.iter().any(|group| {
            group.heading == "Discussion"
                && group.entries.iter().any(|entry| {
                    entry.source == "nudge plus instrument" && entry.origin == "dynamic"
                })
        }));
    }

    fn sample_checkpoint() -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: Some(TranslationContext {
                proper_nouns: vec![ProperNounHint {
                    source: "MiniMax".to_string(),
                    target: Some("MiniMax".to_string()),
                    enforcement: ProperNounEnforcement::Strict,
                    occurrences: 1,
                    first_chunk_index: 2,
                    chunk_indexes: vec![2],
                }],
                static_terms: vec![StaticGlossaryEntry {
                    source: "deliberation".to_string(),
                    target: "deliberation-target".to_string(),
                    scope: "global".to_string(),
                    enforcement: "strict".to_string(),
                    notes: String::new(),
                }],
            }),
            source_hash: "hash".to_string(),
            chunk_size: 1000,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![
                chunk(0, "# Abstract\n\nThe deliberation begins here."),
                chunk(1, "Further deliberation clarifies the policy."),
                chunk_with_terms(
                    2,
                    "# Methods\n\nMiniMax generated the baseline.",
                    Some("## Methods\n\nMiniMax generated the baseline."),
                    vec![ConfirmedTerm {
                        source: "MiniMax".to_string(),
                        translation: "MiniMax".to_string(),
                        term_type: "organization".to_string(),
                        confidence: 95,
                        usage_role: "organization_name".to_string(),
                    }],
                ),
            ],
        }
    }

    fn chunk(index: usize, source: &str) -> ChunkState {
        ChunkState {
            index,
            source: source.to_string(),
            translated: None,
            confirmed_terms: Vec::new(),
            attempts: 0,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }
    }

    fn chunk_with_terms(
        index: usize,
        source: &str,
        translated: Option<&str>,
        confirmed_terms: Vec<ConfirmedTerm>,
    ) -> ChunkState {
        ChunkState {
            index,
            source: source.to_string(),
            translated: translated.map(str::to_string),
            confirmed_terms,
            attempts: u8::from(translated.is_some()),
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }
    }
}
