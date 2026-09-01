use super::super::super::chunking::is_academic_article;
use super::super::super::chunking::should_skip_residual_review;
#[cfg(test)]
use crate::pipeline::FrozenChunkPatchTerm;
use crate::pipeline::{
    checkpoint, ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind, ChunkState,
    ResidualPatchTerm, ResidualReviewChunk, WaveTermDelta,
};
use crate::translation_validation::{detect_residual_english_terms, ResidualEnglishTerm};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResidualReviewStats {
    pub(crate) candidate_chunk_count: usize,
    pub(crate) candidate_term_count: usize,
    pub(crate) candidate_limit: usize,
    pub(crate) hit_candidate_limit: bool,
}

pub(super) fn collect_residual_review_chunks(
    checkpoint: &ChunkCheckpoint,
) -> Vec<ResidualReviewChunk> {
    collect_residual_review_candidates(checkpoint).chunks
}

pub(super) fn collect_pending_residual_review_stats(
    checkpoint: &ChunkCheckpoint,
) -> ResidualReviewStats {
    let candidates = collect_residual_review_candidates(checkpoint);
    let candidate_term_count = candidates
        .chunks
        .iter()
        .map(|chunk| chunk.terms.len())
        .sum::<usize>();
    let candidate_limit = residual_review_candidate_limit(&checkpoint.article_type);
    ResidualReviewStats {
        candidate_chunk_count: candidates.chunks.len(),
        candidate_term_count,
        candidate_limit,
        hit_candidate_limit: candidate_term_count >= candidate_limit,
    }
}

pub(crate) fn collect_residual_review_stats(checkpoint: &ChunkCheckpoint) -> ResidualReviewStats {
    collect_residual_review_stats_for_all_body_chunks(checkpoint)
}

struct ResidualReviewCandidates {
    chunks: Vec<ResidualReviewChunk>,
}

fn collect_residual_review_candidates(checkpoint: &ChunkCheckpoint) -> ResidualReviewCandidates {
    let mut chunks = Vec::new();
    let mut total_terms = 0usize;
    let limit = residual_review_candidate_limit(&checkpoint.article_type);

    for chunk in &checkpoint.chunks {
        if !should_review_chunk(checkpoint, chunk) {
            continue;
        }
        let translated = chunk.translated.as_deref().unwrap_or_default();
        let patch_terms = collect_chunk_patch_terms(checkpoint, chunk);
        let terms = collect_chunk_residual_terms_with_patch_terms(
            &checkpoint.article_type,
            &chunk.source,
            translated,
            &patch_terms,
            limit.saturating_sub(total_terms),
        );
        total_terms += terms.len();

        if !terms.is_empty() {
            chunks.push(ResidualReviewChunk {
                chunk_index: chunk.index,
                source: trim_review_text(&chunk.source),
                translated: trim_review_text(translated),
                terms,
                patch_terms,
            });
        }
        if total_terms >= limit {
            break;
        }
    }
    ResidualReviewCandidates { chunks }
}

fn collect_chunk_patch_terms(
    checkpoint: &ChunkCheckpoint,
    chunk: &ChunkState,
) -> Vec<ResidualPatchTerm> {
    let mut seen = HashSet::<String>::new();
    let mut terms = Vec::new();
    push_static_glossary_patch_terms(checkpoint, chunk, &mut terms, &mut seen);
    push_confirmed_patch_terms(&mut terms, &mut seen, &chunk.confirmed_terms);
    push_wave_patch_terms(checkpoint, chunk, &mut terms, &mut seen);
    push_global_confirmed_patch_terms(checkpoint, chunk, &mut terms, &mut seen);
    terms
}

fn push_static_glossary_patch_terms(
    checkpoint: &ChunkCheckpoint,
    chunk: &ChunkState,
    terms: &mut Vec<ResidualPatchTerm>,
    seen: &mut HashSet<String>,
) {
    let Some(context) = checkpoint.translation_context.as_ref() else {
        return;
    };
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(&chunk.source);
    for entry in &context.static_terms {
        if entry.source.trim().is_empty() || entry.target.trim().is_empty() {
            continue;
        }
        if crate::pipeline::context::source_mentions_hint_cached(
            &chunk.source,
            &entry.source,
            &source_cache,
        ) {
            push_patch_term(terms, seen, &entry.source, &entry.target);
        }
    }
}

fn push_confirmed_patch_terms(
    terms: &mut Vec<ResidualPatchTerm>,
    seen: &mut HashSet<String>,
    confirmed_terms: &[crate::llm::ConfirmedTerm],
) {
    for term in confirmed_terms {
        push_patch_term(terms, seen, &term.source, &term.translation);
    }
}

fn push_global_confirmed_patch_terms(
    checkpoint: &ChunkCheckpoint,
    chunk: &ChunkState,
    terms: &mut Vec<ResidualPatchTerm>,
    seen: &mut HashSet<String>,
) {
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(&chunk.source);
    for source_chunk in &checkpoint.chunks {
        if source_chunk.index == chunk.index || source_chunk.segment_kind != ChunkSegmentKind::Body
        {
            continue;
        }
        for term in &source_chunk.confirmed_terms {
            if !should_use_global_confirmed_patch_term(term, &checkpoint.article_type) {
                continue;
            }
            if crate::pipeline::context::source_mentions_hint_cached(
                &chunk.source,
                &term.source,
                &source_cache,
            ) {
                push_patch_term(terms, seen, &term.source, &term.translation);
            }
        }
    }
}

fn should_use_global_confirmed_patch_term(
    term: &crate::llm::ConfirmedTerm,
    article_type: &str,
) -> bool {
    term.confidence >= 85
        && !term.has_untranslated_source_token()
        && (matches!(
            term.term_type.as_str(),
            "technical" | "organization" | "title" | "disputed" | "coinage"
        ) || should_use_narrative_named_patch_term(term, article_type))
}

fn should_use_narrative_named_patch_term(
    term: &crate::llm::ConfirmedTerm,
    article_type: &str,
) -> bool {
    if !is_narrative_article(article_type) {
        return false;
    }
    match term.term_type.as_str() {
        "person" => source_ascii_token_count(&term.source) >= 2,
        "location" => source_ascii_token_count(&term.source) >= 1,
        _ => false,
    }
}

fn is_narrative_article(article_type: &str) -> bool {
    crate::article_policy::ArticlePolicy::from_article_type(article_type).is_narrative()
}

fn source_ascii_token_count(source: &str) -> usize {
    let mut count = 0usize;
    let mut in_token = false;
    for ch in source.chars() {
        if ch.is_ascii_alphabetic() {
            if !in_token {
                count = count.saturating_add(1);
                in_token = true;
            }
        } else {
            in_token = false;
        }
    }
    count
}

fn push_wave_patch_terms(
    checkpoint: &ChunkCheckpoint,
    chunk: &ChunkState,
    terms: &mut Vec<ResidualPatchTerm>,
    seen: &mut HashSet<String>,
) {
    if let Some(frozen_terms) = checkpoint
        .delta_sync
        .frozen_chunk_patch_terms
        .get(&chunk.index)
    {
        for term in frozen_terms {
            push_patch_term(terms, seen, &term.source, &term.target);
        }
        return;
    }

    let started_seq = checkpoint
        .delta_sync
        .chunk_started_glossary_seq
        .get(&chunk.index)
        .copied()
        .unwrap_or_default();
    let patched_seq = checkpoint
        .delta_sync
        .chunk_patched_through_glossary_seq
        .get(&chunk.index)
        .copied()
        .unwrap_or(checkpoint.delta_sync.glossary_seq);
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(&chunk.source);
    for term in indexed_visible_wave_terms(
        checkpoint,
        chunk.index,
        &source_cache,
        started_seq,
        patched_seq,
    ) {
        let Some(source) =
            residual_wave_term_source_for_chunk(checkpoint, chunk, &source_cache, term)
        else {
            continue;
        };
        push_patch_term(terms, seen, &source, &term.target);
    }
}

fn indexed_visible_wave_terms<'a>(
    checkpoint: &'a ChunkCheckpoint,
    chunk_index: usize,
    source_cache: &crate::pipeline::context::SourceCandidateCache,
    started_seq: u64,
    patched_seq: u64,
) -> Vec<&'a WaveTermDelta> {
    let mut matched_indexes = std::collections::BTreeSet::<usize>::new();
    for key in source_cache.normalized_keys() {
        if let Some(indexes) = checkpoint.delta_sync.wave_term_key_index.get(key) {
            matched_indexes.extend(indexes.iter().copied());
        }
    }

    let mut terms = matched_indexes
        .into_iter()
        .filter_map(|index| checkpoint.delta_sync.wave_delta_log.get(index))
        .filter(|term| term.chunk_index != chunk_index)
        .filter(|term| term.seq > started_seq && term.seq <= patched_seq)
        .collect::<Vec<_>>();

    if terms.is_empty() {
        terms.extend(
            checkpoint
                .delta_sync
                .wave_delta_log
                .iter()
                .filter(|term| term.chunk_index != chunk_index)
                .filter(|term| term.seq > started_seq && term.seq <= patched_seq),
        );
    }

    terms
}

fn residual_wave_term_source_for_chunk(
    checkpoint: &ChunkCheckpoint,
    chunk: &ChunkState,
    source_cache: &crate::pipeline::context::SourceCandidateCache,
    term: &WaveTermDelta,
) -> Option<String> {
    if let Some(alias) = residual_algorithmic_alias_for_chunk(checkpoint, chunk.index, &term.source)
    {
        return Some(alias);
    }
    crate::pipeline::context::source_mentions_hint_cached(&chunk.source, &term.source, source_cache)
        .then(|| term.source.clone())
}

fn residual_algorithmic_alias_for_chunk(
    checkpoint: &ChunkCheckpoint,
    chunk_index: usize,
    source: &str,
) -> Option<String> {
    let context = checkpoint.translation_context.as_ref()?;
    context
        .proper_nouns
        .iter()
        .filter(|hint| hint.target.is_none())
        .filter(|hint| residual_hint_belongs_to_chunk(hint, chunk_index))
        .find(|hint| crate::pipeline::context::source_terms_are_similar(&hint.source, source))
        .map(|hint| hint.source.clone())
}

fn residual_hint_belongs_to_chunk(
    hint: &crate::pipeline::ProperNounHint,
    chunk_index: usize,
) -> bool {
    if hint.chunk_indexes.is_empty() {
        return hint.first_chunk_index == chunk_index;
    }
    hint.chunk_indexes.binary_search(&chunk_index).is_ok()
}

fn push_patch_term(
    terms: &mut Vec<ResidualPatchTerm>,
    seen: &mut HashSet<String>,
    source: &str,
    target: &str,
) {
    let source = source.trim();
    let target = target.trim();
    if source.is_empty() || target.is_empty() || source.eq_ignore_ascii_case(target) {
        return;
    }
    if seen.insert(source.to_ascii_lowercase()) {
        terms.push(ResidualPatchTerm {
            source: source.to_string(),
            target: target.to_string(),
        });
    }
}

fn should_review_chunk(checkpoint: &ChunkCheckpoint, chunk: &ChunkState) -> bool {
    chunk.translated.is_some()
        && chunk.segment_kind == ChunkSegmentKind::Body
        && checkpoint::current_chunk_processing_stage(chunk)
            == ChunkProcessingStage::TermConsistencyApplied
        && !should_skip_residual_review(&checkpoint.article_type, &chunk.source)
}

fn collect_residual_review_stats_for_all_body_chunks(
    checkpoint: &ChunkCheckpoint,
) -> ResidualReviewStats {
    let mut candidate_chunk_count = 0usize;
    let mut candidate_term_count = 0usize;
    let limit = residual_review_candidate_limit(&checkpoint.article_type);

    for chunk in &checkpoint.chunks {
        if chunk.translated.is_none()
            || chunk.segment_kind != ChunkSegmentKind::Body
            || should_skip_residual_review(&checkpoint.article_type, &chunk.source)
        {
            continue;
        }
        let translated = chunk.translated.as_deref().unwrap_or_default();
        let patch_terms = collect_chunk_patch_terms(checkpoint, chunk);
        let remaining = limit.saturating_sub(candidate_term_count);
        let terms = collect_chunk_residual_terms_with_patch_terms(
            &checkpoint.article_type,
            &chunk.source,
            translated,
            &patch_terms,
            remaining,
        );
        if !terms.is_empty() {
            candidate_chunk_count += 1;
            candidate_term_count += terms.len();
        }
        if candidate_term_count >= limit {
            break;
        }
    }

    ResidualReviewStats {
        candidate_chunk_count,
        candidate_term_count,
        candidate_limit: limit,
        hit_candidate_limit: candidate_term_count >= limit,
    }
}

#[cfg(test)]
pub(super) fn collect_chunk_residual_terms(
    article_type: &str,
    source: &str,
    translated: &str,
    limit: usize,
) -> Vec<ResidualEnglishTerm> {
    collect_chunk_residual_terms_with_patch_terms(article_type, source, translated, &[], limit)
}

pub(super) fn collect_chunk_residual_terms_with_patch_terms(
    article_type: &str,
    source: &str,
    translated: &str,
    patch_terms: &[ResidualPatchTerm],
    limit: usize,
) -> Vec<ResidualEnglishTerm> {
    if limit == 0 {
        return Vec::new();
    }

    let mut seen_terms = HashSet::<String>::new();
    let mut terms = Vec::new();
    for issue in detect_residual_english_terms(article_type, source, translated) {
        if seen_terms.insert(issue.term.clone()) {
            terms.push(issue);
        }
        if terms.len() >= limit {
            break;
        }
    }
    collect_patch_term_residuals(
        source,
        translated,
        patch_terms,
        &mut seen_terms,
        &mut terms,
        limit,
    );
    terms
}

fn collect_patch_term_residuals(
    source: &str,
    translated: &str,
    patch_terms: &[ResidualPatchTerm],
    seen_terms: &mut HashSet<String>,
    terms: &mut Vec<ResidualEnglishTerm>,
    limit: usize,
) {
    for patch_term in patch_terms {
        if terms.len() >= limit {
            break;
        }
        if translated.contains(&patch_term.target)
            || !crate::pipeline::context::source_mentions_hint(source, &patch_term.source)
        {
            continue;
        }
        let Some(residual) = crate::pipeline::review::find_source_like_patch_residual(
            translated,
            &patch_term.source,
            &patch_term.target,
        ) else {
            continue;
        };
        let key = residual.term.to_ascii_lowercase();
        if !seen_terms.insert(key.clone()) {
            continue;
        }
        terms.push(ResidualEnglishTerm {
            term: key,
            count: 1,
            source_excerpt: patch_term.source.clone(),
            translated_excerpt: residual.translated_excerpt,
        });
    }
}

fn trim_review_text(text: &str) -> String {
    const REVIEW_CHAR_LIMIT: usize = 6_000;
    if text.chars().count() <= REVIEW_CHAR_LIMIT {
        return text.to_string();
    }
    text.chars().take(REVIEW_CHAR_LIMIT).collect()
}

fn residual_review_candidate_limit(article_type: &str) -> usize {
    if is_academic_article(article_type) {
        48
    } else {
        96
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ConfirmedTerm;
    use crate::pipeline::{
        ProperNounEnforcement, ProperNounHint, StaticGlossaryEntry, TranslationContext,
    };

    #[test]
    fn residual_review_only_collects_term_consistency_stage_body_chunks() {
        let mut translated = chunk(
            0,
            "The Hyperborea harbor echoed at dawn.",
            Some("Hyperborea harbor echoed at dawn."),
            ChunkSegmentKind::Body,
        );
        translated.processing_stage = Some(ChunkProcessingStage::Translated);

        let mut reviewable = chunk(
            1,
            "The Hyperborea harbor echoed again.",
            Some("Hyperborea harbor echoed again."),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let mut reviewed = chunk(
            2,
            "The Hyperborea harbor fell silent.",
            Some("Hyperborea harbor fell silent."),
            ChunkSegmentKind::Body,
        );
        reviewed.processing_stage = Some(ChunkProcessingStage::ResidualReviewed);

        let mut reference = chunk(
            3,
            "Smith, J. (2020). Harbor Studies.",
            Some("Smith, J. (2020). Harbor Studies."),
            ChunkSegmentKind::Reference,
        );
        reference.processing_stage = Some(ChunkProcessingStage::ProtectedPassthrough);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![translated, reviewable, reviewed, reference],
        };

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        assert_eq!(review_chunks[0].chunk_index, 1);
    }

    #[test]
    fn residual_review_stats_include_reviewed_body_chunks_for_final_metrics() {
        let mut reviewed = chunk(
            0,
            "The plus provides additional knowledge that induces deliberation.",
            Some("This plus provides extra knowledge that induces deliberation."),
            ChunkSegmentKind::Body,
        );
        reviewed.processing_stage = Some(ChunkProcessingStage::ResidualReviewed);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![reviewed],
        };

        assert!(collect_residual_review_chunks(&checkpoint).is_empty());
        let stats = collect_residual_review_stats(&checkpoint);

        assert_eq!(stats.candidate_chunk_count, 1);
        assert!(stats.candidate_term_count >= 1);
    }

    #[test]
    fn pending_residual_review_stats_only_count_reviewable_chunks() {
        let mut reviewed = chunk(
            0,
            "The plus provides additional knowledge.",
            Some("This plus provides additional knowledge."),
            ChunkSegmentKind::Body,
        );
        reviewed.processing_stage = Some(ChunkProcessingStage::ResidualReviewed);

        let mut reviewable = chunk(
            1,
            "The coaster landed.",
            Some("The coaster landed."),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![reviewed, reviewable],
        };

        let pending = collect_pending_residual_review_stats(&checkpoint);
        let final_metrics = collect_residual_review_stats(&checkpoint);

        assert_eq!(pending.candidate_chunk_count, 1);
        assert!(pending.candidate_term_count >= 1);
        assert!(final_metrics.candidate_chunk_count >= pending.candidate_chunk_count);
    }

    #[test]
    fn residual_review_collects_static_glossary_patch_terms() {
        let mut reviewable = chunk(
            1,
            "The deliberation instrument stayed visible.",
            Some("The deliberation instrument stayed visible."),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let checkpoint = ChunkCheckpoint {
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
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                }],
                static_terms: vec![StaticGlossaryEntry {
                    source: "deliberation instrument".to_string(),
                    target: "deliberation-device".to_string(),
                    scope: "global".to_string(),
                    enforcement: "strict".to_string(),
                    notes: String::new(),
                }],
            }),
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![reviewable],
        };

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        assert!(review_chunks[0]
            .patch_terms
            .iter()
            .any(|term| term.source == "deliberation instrument"
                && term.target == "deliberation-device"));
    }

    #[test]
    fn residual_review_collects_symbolic_static_glossary_patch_terms_via_variant_match() {
        let mut reviewable = chunk(
            1,
            "The nudge+ instrument stayed visible.",
            Some("The nudge+ instrument stayed visible."),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: vec![StaticGlossaryEntry {
                    source: "nudge plus instrument".to_string(),
                    target: "nudge-plus-device".to_string(),
                    scope: "global".to_string(),
                    enforcement: "strict".to_string(),
                    notes: String::new(),
                }],
            }),
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![reviewable],
        };

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        assert!(review_chunks[0].patch_terms.iter().any(|term| term.source
            == "nudge plus instrument"
            && term.target == "nudge-plus-device"));
    }

    #[test]
    fn residual_terms_include_patch_term_mixed_source_token() {
        let patch_terms = vec![ResidualPatchTerm {
            source: "Admiral Carfax".to_string(),
            target: "卡法克斯海军上将".to_string(),
        }];

        let terms = collect_chunk_residual_terms_with_patch_terms(
            "fiction",
            "Admiral Carfax stood beside the bunk.",
            "卡法克斯 Admiral 站在铺位旁。",
            &patch_terms,
            96,
        );

        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].term, "admiral");
        assert_eq!(terms[0].source_excerpt, "Admiral Carfax");
    }

    #[test]
    fn residual_terms_skip_patch_term_when_target_is_present() {
        let patch_terms = vec![ResidualPatchTerm {
            source: "Admiral Carfax".to_string(),
            target: "卡法克斯海军上将".to_string(),
        }];

        let terms = collect_chunk_residual_terms_with_patch_terms(
            "fiction",
            "Admiral Carfax stood beside the bunk.",
            "卡法克斯海军上将站在铺位旁。",
            &patch_terms,
            96,
        );

        assert!(terms.is_empty());
    }

    #[test]
    fn residual_review_uses_global_confirmed_person_patch_term() {
        let mut anchor = chunk(
            0,
            "Admiral Carfax entered.",
            Some("卡法克斯海军上将进来了。"),
            ChunkSegmentKind::Body,
        );
        anchor.confirmed_terms.push(ConfirmedTerm {
            source: "Admiral Carfax".to_string(),
            translation: "卡法克斯海军上将".to_string(),
            term_type: "person".to_string(),
            confidence: 95,
            usage_role: "character_title".to_string(),
        });
        anchor.processing_stage = Some(ChunkProcessingStage::MergeReady);

        let mut reviewable = chunk(
            1,
            "Admiral Carfax stood beside me.",
            Some("卡法克斯 Admiral 站在我身旁。"),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![anchor, reviewable],
        };

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        assert_eq!(review_chunks[0].terms[0].term, "admiral");
        assert!(review_chunks[0]
            .patch_terms
            .iter()
            .any(|term| term.source == "Admiral Carfax" && term.target == "卡法克斯海军上将"));
    }

    #[test]
    fn residual_review_uses_global_confirmed_coinage_patch_term() {
        let mut anchor = chunk(
            0,
            "The coaster landed.",
            Some("滑行器降落了。"),
            ChunkSegmentKind::Body,
        );
        anchor.confirmed_terms.push(ConfirmedTerm {
            source: "coaster".to_string(),
            translation: "滑行器".to_string(),
            term_type: "coinage".to_string(),
            confidence: 95,
            usage_role: "vehicle".to_string(),
        });
        anchor.processing_stage = Some(ChunkProcessingStage::MergeReady);

        let mut reviewable = chunk(
            1,
            "We retreated toward the coaster.",
            Some("我们退向 coaster。"),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![anchor, reviewable],
        };

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        assert!(review_chunks[0]
            .patch_terms
            .iter()
            .any(|term| term.source == "coaster" && term.target == "滑行器"));
    }

    #[test]
    fn residual_review_uses_global_confirmed_location_patch_term() {
        let mut anchor = chunk(
            0,
            "Hyperborea was ancient.",
            Some("海波波利亚很古老。"),
            ChunkSegmentKind::Body,
        );
        anchor.confirmed_terms.push(ConfirmedTerm {
            source: "Hyperborea".to_string(),
            translation: "海波波利亚".to_string(),
            term_type: "location".to_string(),
            confidence: 95,
            usage_role: "place_name".to_string(),
        });
        anchor.processing_stage = Some(ChunkProcessingStage::MergeReady);

        let mut reviewable = chunk(
            1,
            "The god ruled Hyperborea.",
            Some("这位神祇统治Hyperborea。"),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![anchor, reviewable],
        };

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        assert_eq!(review_chunks[0].terms[0].term, "hyperborea");
        assert!(review_chunks[0]
            .patch_terms
            .iter()
            .any(|term| term.source == "Hyperborea" && term.target == "海波波利亚"));
    }

    #[test]
    fn residual_review_prefers_frozen_wave_term_over_global_confirmed_term() {
        let mut anchor = chunk(
            0,
            "The vehicle landed.",
            Some("GLOBAL_TARGET landed."),
            ChunkSegmentKind::Body,
        );
        anchor.confirmed_terms.push(ConfirmedTerm {
            source: "vehicle".to_string(),
            translation: "GLOBAL_TARGET".to_string(),
            term_type: "coinage".to_string(),
            confidence: 95,
            usage_role: "vehicle".to_string(),
        });
        anchor.processing_stage = Some(ChunkProcessingStage::MergeReady);

        let mut reviewable = chunk(
            1,
            "We returned to the vehicle.",
            Some("We returned to the vehicle."),
            ChunkSegmentKind::Body,
        );
        reviewable.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);

        let mut checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 200,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![anchor, reviewable],
        };
        checkpoint.delta_sync.frozen_chunk_patch_terms.insert(
            1,
            vec![FrozenChunkPatchTerm {
                seq: 2,
                source: "vehicle".to_string(),
                target: "WAVE_TARGET".to_string(),
            }],
        );

        let review_chunks = collect_residual_review_chunks(&checkpoint);

        assert_eq!(review_chunks.len(), 1);
        let vehicle_terms = review_chunks[0]
            .patch_terms
            .iter()
            .filter(|term| term.source == "vehicle")
            .collect::<Vec<_>>();
        assert_eq!(vehicle_terms.len(), 1);
        assert_eq!(vehicle_terms[0].target, "WAVE_TARGET");
    }

    fn chunk(
        index: usize,
        source: &str,
        translated: Option<&str>,
        segment_kind: ChunkSegmentKind,
    ) -> ChunkState {
        ChunkState {
            index,
            source: source.to_string(),
            translated: translated.map(str::to_string),
            confirmed_terms: Vec::new(),
            attempts: u8::from(translated.is_some()),
            updated_at: "now".to_string(),
            segment_kind,
            processing_stage: None,
        }
    }
}
