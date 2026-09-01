use super::{locate_term_patch_target, replace_sentence_once, replace_source_like_phrase};
use crate::error::AppError;
use crate::llm::SensenovaClient;
use crate::pipeline::{
    record_llm_retryable_error, record_llm_success, wait_for_llm_rate_slot, ChunkCheckpoint,
    ChunkSegmentKind, FrozenChunkPatchTerm, ProperNounHint, WavePatchSelection, WaveTermDelta,
    WaveTermRepairStats,
};
use crate::translation_validation::build_llm_sentence_patch_prompt;
use tokio_util::sync::CancellationToken;

const FROZEN_CHUNK_PATCH_INDEX_VERSION: u32 = 2;

pub(crate) fn collect_wave_patch_terms_for_chunk(
    checkpoint: &ChunkCheckpoint,
    chunk_index: usize,
    incoming_terms: &[WaveTermDelta],
) -> WavePatchSelection {
    let Some(chunk) = checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index == chunk_index)
    else {
        return WavePatchSelection::default();
    };
    if chunk.segment_kind != ChunkSegmentKind::Body {
        return WavePatchSelection::default();
    }

    let started_glossary_seq = checkpoint
        .delta_sync
        .chunk_started_glossary_seq
        .get(&chunk_index)
        .copied()
        .unwrap_or_default();
    let patched_through_glossary_seq = checkpoint
        .delta_sync
        .chunk_patched_through_glossary_seq
        .get(&chunk_index)
        .copied()
        .unwrap_or(started_glossary_seq);

    let mut selection = WavePatchSelection::default();
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(&chunk.source);
    for term in incoming_terms
        .iter()
        .filter(|term| term.chunk_index != chunk_index)
        .filter(|term| term.seq > started_glossary_seq)
        .filter(|term| term.seq > patched_through_glossary_seq)
    {
        let Some(matched_term) =
            match_wave_term_for_chunk(checkpoint, chunk_index, &chunk.source, &source_cache, term)
        else {
            continue;
        };
        if local_terms_cover_source(&chunk.confirmed_terms, &term.source) {
            selection.deduped_count += 1;
            continue;
        }
        if matched_term.source == term.source {
            selection.source_scan_match_count += 1;
        } else {
            selection.algorithmic_match_count += 1;
        }
        selection.terms.push(matched_term);
    }
    selection
}

pub(crate) fn frozen_chunk_patch_index_is_current(checkpoint: &ChunkCheckpoint) -> bool {
    checkpoint.delta_sync.frozen_at_glossary_seq == Some(checkpoint.delta_sync.glossary_seq)
        && checkpoint.delta_sync.frozen_patch_index_version
            == Some(FROZEN_CHUNK_PATCH_INDEX_VERSION)
}

pub(crate) fn rebuild_frozen_chunk_patch_terms(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut indexed_chunks = 0usize;
    let mut frozen = std::collections::BTreeMap::<usize, Vec<FrozenChunkPatchTerm>>::new();

    for chunk in checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.segment_kind == ChunkSegmentKind::Body)
    {
        let terms = collect_frozen_patch_terms_for_chunk(checkpoint, chunk.index);
        if terms.is_empty() {
            continue;
        }
        indexed_chunks += 1;
        frozen.insert(chunk.index, terms);
    }

    checkpoint.delta_sync.frozen_at_glossary_seq = Some(checkpoint.delta_sync.glossary_seq);
    checkpoint.delta_sync.frozen_patch_index_version = Some(FROZEN_CHUNK_PATCH_INDEX_VERSION);
    checkpoint.delta_sync.frozen_chunk_patch_terms = frozen;
    indexed_chunks
}

fn collect_frozen_patch_terms_for_chunk(
    checkpoint: &ChunkCheckpoint,
    chunk_index: usize,
) -> Vec<FrozenChunkPatchTerm> {
    let Some(chunk) = checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index == chunk_index)
    else {
        return Vec::new();
    };
    let started_seq = checkpoint
        .delta_sync
        .chunk_started_glossary_seq
        .get(&chunk_index)
        .copied()
        .unwrap_or_default();
    let patched_seq = checkpoint.delta_sync.glossary_seq;
    let mut seen = std::collections::HashSet::<String>::new();
    let mut terms = Vec::new();
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(&chunk.source);
    for term in indexed_visible_wave_terms(checkpoint, chunk_index, started_seq, patched_seq) {
        let Some(matched) =
            match_wave_term_for_chunk(checkpoint, chunk_index, &chunk.source, &source_cache, term)
        else {
            continue;
        };
        if local_terms_cover_source(&chunk.confirmed_terms, &term.source) {
            continue;
        }
        let key = matched.source.to_ascii_lowercase();
        if seen.insert(key) {
            terms.push(FrozenChunkPatchTerm {
                seq: matched.seq,
                source: matched.source,
                target: matched.target,
            });
        }
    }
    terms
}

fn indexed_visible_wave_terms<'a>(
    checkpoint: &'a ChunkCheckpoint,
    chunk_index: usize,
    started_seq: u64,
    patched_seq: u64,
) -> Vec<&'a WaveTermDelta> {
    let Some(chunk) = checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index == chunk_index)
    else {
        return Vec::new();
    };
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(&chunk.source);
    let mut matched_indexes = std::collections::BTreeSet::<usize>::new();

    for key in source_cache_keys(&source_cache) {
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

fn source_cache_keys(
    cache: &crate::pipeline::context::SourceCandidateCache,
) -> impl Iterator<Item = &String> {
    cache.normalized_keys().iter()
}

fn match_wave_term_for_chunk(
    checkpoint: &ChunkCheckpoint,
    chunk_index: usize,
    chunk_source: &str,
    source_cache: &crate::pipeline::context::SourceCandidateCache,
    term: &WaveTermDelta,
) -> Option<WaveTermDelta> {
    if let Some(alias) = algorithmic_hint_alias_for_chunk(checkpoint, chunk_index, &term.source) {
        let mut matched = term.clone();
        matched.source = alias;
        return Some(matched);
    }
    crate::pipeline::context::source_mentions_hint_cached(chunk_source, &term.source, source_cache)
        .then(|| term.clone())
}

fn algorithmic_hint_alias_for_chunk(
    checkpoint: &ChunkCheckpoint,
    chunk_index: usize,
    source: &str,
) -> Option<String> {
    let context = checkpoint.translation_context.as_ref()?;
    context
        .proper_nouns
        .iter()
        .filter(|hint| hint.target.is_none())
        .filter(|hint| algorithmic_hint_belongs_to_chunk(hint, chunk_index))
        .find(|hint| crate::pipeline::context::source_terms_are_similar(&hint.source, source))
        .map(|hint| hint.source.clone())
}

fn algorithmic_hint_belongs_to_chunk(hint: &ProperNounHint, chunk_index: usize) -> bool {
    if hint.chunk_indexes.is_empty() {
        return hint.first_chunk_index == chunk_index;
    }
    hint.chunk_indexes.binary_search(&chunk_index).is_ok()
}

/// 单 chunk 的 wave repair 结果（并发 worker 返回，主循环统一写回 checkpoint）。
pub(crate) struct WaveRepairOutcome {
    pub chunk_index: usize,
    pub repaired: String,
    pub stats: WaveTermRepairStats,
    pub max_seq: u64,
}

/// 纯函数版 wave repair：不修改 checkpoint，返回修补后的文本与统计。
/// 供并发 worker 调用（避免 &mut checkpoint 的可变借用冲突）。
pub(crate) async fn apply_wave_term_repairs_for_chunk(
    llm_client: &SensenovaClient,
    chunk_source: String,
    chunk_translated: String,
    chunk_index: usize,
    patch_terms: Vec<WaveTermDelta>,
    static_glossary_entries: Vec<(String, String)>,
    article_type: String,
) -> Result<WaveRepairOutcome, AppError> {
    let mut stats = WaveTermRepairStats::default();
    let mut repaired = chunk_translated;

    for term in &patch_terms {
        let Some(target) =
            locate_term_patch_target(&chunk_source, &repaired, &term.source, &term.target)
        else {
            continue;
        };
        if target.translated_sentence.contains(&term.target) {
            stats.already_correct_count += 1;
            continue;
        }
        stats.attempted_terms += 1;
        // 确定性短路：源词在译文中残留（未翻）时直接替换，无需 LLM。
        let deterministic = crate::pipeline::review::replace_source_like_phrase(
            &target.translated_sentence,
            &term.source,
            &term.target,
        );
        if deterministic != target.translated_sentence {
            if let Some(next_text) =
                replace_sentence_once(&repaired, &target.translated_sentence, &deterministic)
            {
                repaired = next_text;
                stats.applied_terms += 1;
                stats.fallback_patch_count += 1;
                stats.changed = true;
                continue;
            }
        }
        let static_terms =
            sentence_static_glossary_terms(&static_glossary_entries, &target.source_sentence);
        let mut patch_glossary = vec![(term.source.as_str(), term.target.as_str())];
        for static_term in &static_terms {
            if !patch_glossary
                .iter()
                .any(|(source, _)| *source == static_term.0.as_str())
            {
                patch_glossary.push((static_term.0.as_str(), static_term.1.as_str()));
            }
        }
        let llm_prompt = build_llm_sentence_patch_prompt(&article_type, &target, &patch_glossary);
        let patch_response =
            request_wave_patch_sentence(llm_client, &llm_prompt, &target, term).await;
        let patched_sentence = match patch_response {
            WavePatchResponse::Patched(sentence) => {
                stats.llm_patch_count += 1;
                sentence
            }
            WavePatchResponse::Correct => {
                stats.llm_correct_count += 1;
                target.translated_sentence.clone()
            }
            WavePatchResponse::Fallback(sentence) => {
                stats.fallback_patch_count += 1;
                sentence
            }
        };
        if patched_sentence == target.translated_sentence {
            continue;
        }
        let Some(next_text) =
            replace_sentence_once(&repaired, &target.translated_sentence, &patched_sentence)
        else {
            continue;
        };
        repaired = next_text;
        stats.applied_terms += 1;
        stats.changed = true;
    }

    let max_seq = patch_terms
        .iter()
        .map(|term| term.seq)
        .max()
        .unwrap_or_default();
    Ok(WaveRepairOutcome {
        chunk_index,
        repaired,
        stats,
        max_seq,
    })
}


fn sentence_static_glossary_terms(
    entries: &[(String, String)],
    source_sentence: &str,
) -> Vec<(String, String)> {
    entries
        .iter()
        .filter(|(source, target)| !source.trim().is_empty() && !target.trim().is_empty())
        .filter(|(source, _)| {
            crate::pipeline::context::source_mentions_hint(source_sentence, source)
        })
        .cloned()
        .collect()
}

enum WavePatchResponse {
    Patched(String),
    Correct,
    Fallback(String),
}

async fn request_wave_patch_sentence(
    llm_client: &SensenovaClient,
    llm_prompt: &str,
    target: &crate::translation_validation::SentencePatchTarget,
    term: &WaveTermDelta,
) -> WavePatchResponse {
    let token = CancellationToken::new();
    let route = llm_client.reserve_route();
    let routing_key = route.routing_key().clone();
    let route_permit = match wait_for_llm_rate_slot(&routing_key, &token).await {
        Ok(permit) => permit,
        Err(_) => {
            return WavePatchResponse::Fallback(replace_source_like_phrase(
                &target.translated_sentence,
                &term.source,
                &term.target,
            ));
        }
    };
    let started = std::time::Instant::now();
    match llm_client
        .review_translation_issue_on_route(&route, llm_prompt)
        .await
    {
        Ok(response) => {
            let request_elapsed_ms = started.elapsed().as_millis();
            record_llm_success(&routing_key, request_elapsed_ms);
            drop(route_permit);
            crate::pipeline::review::parse_repaired_translation(&response)
                .map(WavePatchResponse::Patched)
                .unwrap_or(WavePatchResponse::Correct)
        }
        Err(error) => {
            let request_elapsed_ms = started.elapsed().as_millis();
            let error = AppError::from_provider_error(error);
            llm_client.maybe_activate_fallback_after_app_error(&routing_key.model, &error);
            record_llm_retryable_error(&routing_key, &error, request_elapsed_ms);
            drop(route_permit);
            WavePatchResponse::Fallback(replace_source_like_phrase(
                &target.translated_sentence,
                &term.source,
                &term.target,
            ))
        }
    }
}

fn local_terms_cover_source(terms: &[crate::llm::ConfirmedTerm], incoming_source: &str) -> bool {
    terms.iter().any(|term| {
        crate::pipeline::context::source_mentions_hint(&term.source, incoming_source)
            || crate::pipeline::context::source_mentions_hint(incoming_source, &term.source)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ConfirmedTerm;
    use crate::pipeline::{
        ChunkProcessingStage, ChunkState, ProperNounEnforcement, ProperNounHint, TranslationContext,
    };

    #[test]
    fn excludes_incoming_term_when_local_confirmed_term_already_covers_it() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec![],
            translation_context: Some(TranslationContext {
                proper_nouns: vec![],
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: {
                let mut state = crate::pipeline::DeltaSyncState::default();
                state.chunk_started_glossary_seq.insert(2, 0);
                state.chunk_patched_through_glossary_seq.insert(2, 0);
                state
            },
            chunks: vec![ChunkState {
                index: 2,
                source: "Nudge plus preserves autonomy.".to_string(),
                translated: Some("助推升级维护自主性。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "nudge plus".to_string(),
                    translation: "助推+".to_string(),
                    term_type: "coinage".to_string(),
                    confidence: 95,
                    usage_role: "coinage_noun".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let incoming = vec![WaveTermDelta {
            seq: 1,
            source: "Nudge plus".to_string(),
            target: "助推+".to_string(),
            chunk_index: 1,
        }];

        let result = collect_wave_patch_terms_for_chunk(&checkpoint, 2, &incoming);
        assert!(result.terms.is_empty());
        assert_eq!(result.deduped_count, 1);
    }

    #[test]
    fn uses_algorithmic_hint_alias_before_source_scan_for_wave_match() {
        let checkpoint = checkpoint_with_context_and_chunk(
            TranslationContext {
                proper_nouns: vec![ProperNounHint {
                    source: "Hyperborean".to_string(),
                    target: None,
                    enforcement: ProperNounEnforcement::Contextual,
                    occurrences: 1,
                    first_chunk_index: 2,
                    chunk_indexes: vec![2],
                }],
                static_terms: Vec::new(),
            },
            "Hyperborean rulers watched.",
            "许珀耳玻瑞亚统治者注视着。",
            vec![],
        );
        let incoming = vec![WaveTermDelta {
            seq: 1,
            source: "Hyperborea".to_string(),
            target: "许珀耳玻瑞亚".to_string(),
            chunk_index: 1,
        }];

        let result = collect_wave_patch_terms_for_chunk(&checkpoint, 2, &incoming);

        assert_eq!(result.terms.len(), 1);
        assert_eq!(result.terms[0].source, "Hyperborean");
        assert_eq!(result.algorithmic_match_count, 1);
        assert_eq!(result.source_scan_match_count, 0);
    }

    #[test]
    fn keeps_source_scan_fallback_when_no_algorithmic_hint_matches() {
        let checkpoint = checkpoint_with_context_and_chunk(
            TranslationContext {
                proper_nouns: vec![],
                static_terms: Vec::new(),
            },
            "Nudge plus preserves autonomy.",
            "助推升级维护自主性。",
            vec![],
        );
        let incoming = vec![WaveTermDelta {
            seq: 1,
            source: "Nudge plus".to_string(),
            target: "助推+".to_string(),
            chunk_index: 1,
        }];

        let result = collect_wave_patch_terms_for_chunk(&checkpoint, 2, &incoming);

        assert_eq!(result.terms.len(), 1);
        assert_eq!(result.terms[0].source, "Nudge plus");
        assert_eq!(result.algorithmic_match_count, 0);
        assert_eq!(result.source_scan_match_count, 1);
    }

    #[test]
    fn frozen_chunk_patch_terms_are_built_after_glossary_freezes() {
        let mut checkpoint = checkpoint_with_context_and_chunk(
            TranslationContext {
                proper_nouns: vec![],
                static_terms: Vec::new(),
            },
            "Nudge plus preserves autonomy.",
            "助推升级维护自主性。",
            vec![],
        );
        checkpoint.delta_sync.glossary_seq = 3;
        checkpoint
            .delta_sync
            .chunk_started_glossary_seq
            .insert(2, 0);
        checkpoint
            .delta_sync
            .chunk_patched_through_glossary_seq
            .insert(2, 3);
        checkpoint.delta_sync.wave_delta_log = vec![WaveTermDelta {
            seq: 2,
            source: "Nudge plus".to_string(),
            target: "助推+".to_string(),
            chunk_index: 1,
        }];

        let indexed = rebuild_frozen_chunk_patch_terms(&mut checkpoint);

        assert_eq!(indexed, 1);
        assert_eq!(checkpoint.delta_sync.frozen_at_glossary_seq, Some(3));
        let terms = checkpoint
            .delta_sync
            .frozen_chunk_patch_terms
            .get(&2)
            .expect("chunk should have frozen terms");
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].source, "Nudge plus");
        assert_eq!(terms[0].target, "助推+");
    }

    #[test]
    fn frozen_chunk_patch_index_requires_current_version() {
        let mut checkpoint = checkpoint_with_context_and_chunk(
            TranslationContext {
                proper_nouns: vec![],
                static_terms: Vec::new(),
            },
            "Nudge plus preserves autonomy.",
            "Translated.",
            vec![],
        );
        checkpoint.delta_sync.glossary_seq = 3;
        checkpoint.delta_sync.frozen_at_glossary_seq = Some(3);

        assert!(!frozen_chunk_patch_index_is_current(&checkpoint));

        rebuild_frozen_chunk_patch_terms(&mut checkpoint);

        assert!(frozen_chunk_patch_index_is_current(&checkpoint));
    }

    #[test]
    fn frozen_chunk_patch_terms_use_final_glossary_range() {
        let mut checkpoint = checkpoint_with_context_and_chunk(
            TranslationContext {
                proper_nouns: vec![],
                static_terms: Vec::new(),
            },
            "Nudge plus preserves autonomy. Later term is absent.",
            "助推升级维护自主性。",
            vec![],
        );
        checkpoint.delta_sync.glossary_seq = 4;
        checkpoint
            .delta_sync
            .chunk_started_glossary_seq
            .insert(2, 1);
        checkpoint
            .delta_sync
            .chunk_patched_through_glossary_seq
            .insert(2, 3);
        checkpoint.delta_sync.wave_delta_log = vec![
            WaveTermDelta {
                seq: 1,
                source: "Before".to_string(),
                target: "之前".to_string(),
                chunk_index: 0,
            },
            WaveTermDelta {
                seq: 2,
                source: "Nudge plus".to_string(),
                target: "助推+".to_string(),
                chunk_index: 1,
            },
            WaveTermDelta {
                seq: 4,
                source: "Later term".to_string(),
                target: "后来的术语".to_string(),
                chunk_index: 3,
            },
        ];

        rebuild_frozen_chunk_patch_terms(&mut checkpoint);

        let terms = checkpoint
            .delta_sync
            .frozen_chunk_patch_terms
            .get(&2)
            .expect("chunk should have frozen terms");
        assert_eq!(terms.len(), 2);
        assert_eq!(terms[0].source, "Nudge plus");
        assert_eq!(terms[1].source, "Later term");
    }

    #[test]
    fn ignores_terms_emitted_by_same_chunk() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec![],
            translation_context: Some(TranslationContext {
                proper_nouns: vec![],
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 2,
                source: "Nudge plus preserves autonomy.".to_string(),
                translated: Some("already translated".to_string()),
                confirmed_terms: vec![],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let incoming = vec![WaveTermDelta {
            seq: 0,
            source: "Nudge plus".to_string(),
            target: "助推+".to_string(),
            chunk_index: 2,
        }];

        let result = collect_wave_patch_terms_for_chunk(&checkpoint, 2, &incoming);
        assert!(result.terms.is_empty());
        assert_eq!(result.deduped_count, 0);
    }

    #[test]
    fn sentence_static_glossary_terms_include_matching_sentence_terms() {
        let checkpoint = checkpoint_with_context_and_chunk(
            TranslationContext {
                proper_nouns: vec![],
                static_terms: vec![crate::pipeline::StaticGlossaryEntry {
                    source: "deliberation instrument".to_string(),
                    target: "审议工具".to_string(),
                    scope: "global".to_string(),
                    enforcement: "strict".to_string(),
                    notes: String::new(),
                }],
            },
            "The deliberation instrument stayed visible.",
            "The deliberation 工具仍然可见。",
            vec![],
        );

        let hits = sentence_static_glossary_terms(
            &checkpoint
                .translation_context
                .as_ref()
                .unwrap()
                .static_terms
                .iter()
                .map(|entry| (entry.source.clone(), entry.target.clone()))
                .collect::<Vec<_>>(),
            "The deliberation instrument stayed visible.",
        );

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "deliberation instrument");
        assert_eq!(hits[0].1, "审议工具");
    }

    fn checkpoint_with_context_and_chunk(
        translation_context: TranslationContext,
        source: &str,
        translated: &str,
        confirmed_terms: Vec<ConfirmedTerm>,
    ) -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec![],
            translation_context: Some(translation_context),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: {
                let mut state = crate::pipeline::DeltaSyncState::default();
                state.chunk_started_glossary_seq.insert(2, 0);
                state.chunk_patched_through_glossary_seq.insert(2, 0);
                state
            },
            chunks: vec![ChunkState {
                index: 2,
                source: source.to_string(),
                translated: Some(translated.to_string()),
                confirmed_terms,
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        }
    }

    #[test]
    fn parse_based_patch_keeps_sentence_when_response_is_correct_signal() {
        let original = "这项研究使用审议工具。";
        let response = r#"{"decision":"correct"}"#;
        let parsed = crate::pipeline::review::parse_repaired_translation(response);

        assert!(parsed.is_none());
        assert_eq!(original, "这项研究使用审议工具。");
    }

    #[test]
    fn fallback_patch_replaces_ascii_source_token() {
        let sentence = "研究比较了 deliberation 工具与冷静期。";
        let patched = replace_source_like_phrase(sentence, "deliberation", "审议");

        assert_eq!(patched, "研究比较了 审议 工具与冷静期。");
    }
}
