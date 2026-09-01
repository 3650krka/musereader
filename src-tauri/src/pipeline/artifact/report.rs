use super::*;

#[path = "glossary.rs"]
mod glossary;
#[path = "manifest.rs"]
mod manifest;
#[path = "metrics.rs"]
mod metrics;
#[cfg(test)]
#[path = "toc.rs"]
mod toc;
#[path = "validation.rs"]
mod validation;

pub(crate) use glossary::build_glossary_artifact;
pub(crate) use manifest::build_manifest;
pub(crate) use metrics::build_metrics_artifact;
#[cfg(test)]
pub(crate) use toc::build_translated_toc_artifact;
pub(crate) use validation::build_validation_report;

pub(crate) fn build_translated_chunk_artifact(
    checkpoint: &ChunkCheckpoint,
) -> Vec<TranslatedChunkArtifact> {
    checkpoint
        .chunks
        .iter()
        .filter_map(|chunk| {
            chunk
                .translated
                .as_ref()
                .map(|translated| TranslatedChunkArtifact {
                    marker_blocks: build_translated_chunk_marker_blocks(&chunk.source, translated),
                    chunk_index: chunk.index,
                    segment_kind: format!("{:?}", chunk.segment_kind),
                    processing_stage: format!(
                        "{:?}",
                        crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                    ),
                    source: chunk.source.clone(),
                    translated: translated.clone(),
                    confirmed_terms: chunk.confirmed_terms.clone(),
                })
        })
        .collect()
}

fn build_translated_chunk_marker_blocks(
    source: &str,
    translated: &str,
) -> Vec<TranslatedChunkMarkerBlock> {
    let source_blocks = split_artifact_markdown_blocks(source);
    let translated_blocks = split_artifact_markdown_blocks(translated);
    source_blocks
        .into_iter()
        .enumerate()
        .map(|(index, source_markdown)| TranslatedChunkMarkerBlock {
            marker_id: format!("B{index:03}"),
            source_markdown,
            translated_markdown: translated_blocks.get(index).cloned().unwrap_or_default(),
        })
        .collect()
}

fn split_artifact_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod artifact_tests {
    use super::*;
    use crate::document::{TocArtifact, TocEntry, TocSourceKind};
    use crate::ocr::OcrDocument;
    use crate::ocr::{OcrLayoutBlock, OcrPageLayout};
    use base64::Engine as _;
    use chrono::Utc;
    use std::collections::HashMap;

    #[test]
    fn translated_chunk_artifact_preserves_chunk_identity_and_stage() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Satampra Zeiros",
            "萨坦普拉·泽罗斯",
            "Satampra Zeiros entered the city.",
            "萨坦普拉·泽罗斯走进了城市。",
        );

        let artifact = build_translated_chunk_artifact(&checkpoint);

        assert_eq!(artifact.len(), 1);
        assert_eq!(artifact[0].chunk_index, 0);
        assert_eq!(artifact[0].segment_kind, "Body");
        assert_eq!(artifact[0].processing_stage, "Translated");
        assert!(artifact[0].translated.contains("萨坦普拉"));
    }

    #[test]
    fn strict_proper_noun_requires_exact_target_translation() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Satampra Zeiros",
            "萨坦普拉路泽罗斯",
            "Satampra Zeiros entered the city.",
            "泽罗斯进入了城市。",
        );

        let report = build_validation_report("test", &checkpoint);

        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
    }

    #[test]
    fn contextual_proper_noun_allows_natural_adjectival_translation() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Contextual,
            "Martian",
            "火星人",
            "Martian rulers watched the sky.",
            "火星统治者注视着天空。",
        );

        let report = build_validation_report("test", &checkpoint);

        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
    }

    #[test]
    fn fiction_unknown_proper_noun_can_be_localized_without_warning() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Queen Cunambria",
            "",
            "The jewels of Queen Cunambria were hidden.",
            "库南布里亚女王的珠宝被藏起来了。",
        );
        let mut checkpoint = checkpoint;
        checkpoint.translation_context = Some(TranslationContext {
            proper_nouns: vec![ProperNounHint {
                source: "Queen Cunambria".to_string(),
                target: None,
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 1,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            }],
            static_terms: Vec::new(),
        });

        let report = build_validation_report("test", &checkpoint);

        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "UNKNOWN_PROPER_NOUN_WITHOUT_CANONICAL_TARGET"));
    }

    #[test]
    fn root_term_does_not_false_match_longer_variant_in_source() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Hyperborea",
            "许珀耳玻瑞亚",
            "Hyperborean rulers watched the sky.",
            "许珀耳玻瑞亚统治者注视着天空。",
        );

        let report = build_validation_report("test", &checkpoint);

        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
    }

    #[test]
    fn builds_metrics_artifact_from_checkpoint_and_validation() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Satampra Zeiros",
            "萨坦普拉路泽罗斯",
            "Satampra Zeiros entered the city.",
            "萨坦普拉路泽罗斯走进了城市。",
        );
        let mut checkpoint = checkpoint;
        checkpoint.chunks[0].translated =
            Some("钀ㄥ潶鏅媺璺辰缃楁柉走进了城市。助推+在这里发挥作用。".to_string());
        checkpoint.chunks[0].confirmed_terms.push(ConfirmedTerm {
            source: "nudge plus".to_string(),
            translation: "助推+".to_string(),
            term_type: "coinage".to_string(),
            confidence: 95,
            usage_role: "coinage_noun".to_string(),
        });
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("metrics build");

        assert_eq!(metrics.total_chunks, 1);
        assert_eq!(metrics.translated_chunks, 1);
        assert!(metrics.strict_glossary_entries >= 1);
        assert_eq!(
            metrics.validation_issue_count,
            validation.summary.issue_count
        );
    }

    #[test]
    fn metrics_artifact_records_source_toc_observability() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Example",
            "Example",
            "Example source.",
            "Example translation.",
        );
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);
        let toc = sample_toc_artifact();
        let source_markdown = "# Chapter 1\n\n# Chapter 2\n\n# Chapter 2";
        let translated_markdown = "# 第一章\n\n# 第二章甲\n\n# 第二章乙";
        let translated_toc =
            build_translated_toc_artifact(&toc, source_markdown, translated_markdown, &[], &[]);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            None,
            Some(&toc),
            Some(&translated_toc),
            Some(source_markdown),
            Some(translated_markdown),
            None,
        )
        .expect("metrics build");

        assert_eq!(metrics.toc_source_kind.as_deref(), Some("epubNcx"));
        assert_eq!(metrics.body_chunk_count, 1);
        assert_eq!(metrics.reference_chunk_count, 0);
        assert_eq!(metrics.protected_metadata_chunk_count, 0);
        assert_eq!(metrics.body_translated_stage_chunk_count, 1);
        assert_eq!(metrics.body_merge_ready_stage_chunk_count, 0);
        assert_eq!(metrics.toc_entry_count, 3);
        assert_eq!(metrics.toc_confidence, Some(0.95));
        assert_eq!(metrics.toc_low_confidence_entry_count, 1);
        assert_eq!(metrics.toc_duplicate_title_count, 1);
        assert_eq!(metrics.toc_missing_locator_count, 1);
        assert_eq!(metrics.translated_toc_entry_count, 3);
        assert_eq!(metrics.translated_toc_missing_title_count, 2);
        assert_eq!(metrics.translated_toc_structural_mismatch_count, 0);
        assert_eq!(metrics.translated_toc_locator_mutation_count, 0);
        assert_eq!(metrics.translated_toc_matched_title_count, 1);
        assert_eq!(metrics.translated_toc_unmatched_title_count, 2);
        assert_eq!(metrics.translated_toc_exact_structure_match_count, 3);
        assert!(metrics.translated_toc_title_backfill_rate > 0.3);
    }

    fn checkpoint_with_context(
        enforcement: ProperNounEnforcement,
        source_term: &str,
        target_term: &str,
        source: &str,
        translated: &str,
    ) -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: Some(TranslationContext {
                proper_nouns: vec![ProperNounHint {
                    source: source_term.to_string(),
                    target: Some(target_term.to_string()),
                    enforcement,
                    occurrences: 1,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                }],
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 1000,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: source.to_string(),
                translated: Some(translated.to_string()),
                confirmed_terms: Vec::new(),
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            }],
        }
    }

    #[test]
    fn metrics_artifact_records_checkpoint_stage_distribution() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 1000,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:05Z".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![
                chunk_state(
                    0,
                    "正文一",
                    None,
                    ChunkSegmentKind::Body,
                    Some(ChunkProcessingStage::Pending),
                ),
                chunk_state(
                    1,
                    "正文二",
                    Some("译文二"),
                    ChunkSegmentKind::Body,
                    Some(ChunkProcessingStage::Translated),
                ),
                chunk_state(
                    2,
                    "正文三",
                    Some("译文三"),
                    ChunkSegmentKind::Body,
                    Some(ChunkProcessingStage::TermConsistencyApplied),
                ),
                chunk_state(
                    3,
                    "正文四",
                    Some("译文四"),
                    ChunkSegmentKind::Body,
                    Some(ChunkProcessingStage::ResidualReviewed),
                ),
                chunk_state(
                    4,
                    "正文五",
                    Some("译文五"),
                    ChunkSegmentKind::Body,
                    Some(ChunkProcessingStage::MergeReady),
                ),
                chunk_state(
                    5,
                    "References",
                    Some("References"),
                    ChunkSegmentKind::Reference,
                    Some(ChunkProcessingStage::ProtectedPassthrough),
                ),
                chunk_state(
                    6,
                    "<!-- Authors -->",
                    Some("<!-- Authors -->"),
                    ChunkSegmentKind::ProtectedMetadata,
                    Some(ChunkProcessingStage::ProtectedPassthrough),
                ),
            ],
        };
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("metrics build");

        assert_eq!(metrics.body_chunk_count, 5);
        assert_eq!(metrics.reference_chunk_count, 1);
        assert_eq!(metrics.protected_metadata_chunk_count, 1);
        assert_eq!(metrics.body_pending_chunk_count, 1);
        assert_eq!(metrics.body_translated_stage_chunk_count, 1);
        assert_eq!(metrics.body_term_consistency_stage_chunk_count, 1);
        assert_eq!(metrics.body_residual_reviewed_stage_chunk_count, 1);
        assert_eq!(metrics.body_merge_ready_stage_chunk_count, 1);
        assert_eq!(metrics.protected_passthrough_chunk_count, 2);
    }

    #[test]
    fn metrics_artifact_records_event_log_stage_costs() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-event-metrics-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let event_log_path = root.join("events.ndjson");
        std::fs::create_dir_all(&root).expect("temp dir should exist");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                stage: "ocr_attempt".to_string(),
                message: "OCR attempt 1".to_string(),
                progress: 20,
                level: "debug".to_string(),
            },
        )
        .expect("ocr event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:01Z".to_string(),
                stage: "chunk_translated".to_string(),
                message:
                    "translated chunk 1/2; index=0; elapsed_ms=900; wait_ms=30; llm_elapsed_ms=800; llm_attempts=1"
                        .to_string(),
                progress: 66,
                level: "info".to_string(),
            },
        )
        .expect("translation event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:02Z".to_string(),
                stage: "chunk_translated".to_string(),
                message:
                    "translated chunk 2/2; index=1; elapsed_ms=1200; wait_ms=45; llm_elapsed_ms=1000; llm_attempts=2"
                        .to_string(),
                progress: 92,
                level: "info".to_string(),
            },
        )
        .expect("translation event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:02.500Z".to_string(),
                stage: "chunk_structure_check".to_string(),
                message:
                    "chunk_index=0; source_blocks=3; translated_blocks=3; block_delta=0; source_headings=1; translated_headings=1; heading_mismatch=false; mismatch=false"
                        .to_string(),
                progress: 66,
                level: "info".to_string(),
            },
        )
        .expect("structure check event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:02.750Z".to_string(),
                stage: "chunk_structure_check".to_string(),
                message:
                    "chunk_index=1; source_blocks=4; translated_blocks=2; block_delta=2; source_headings=1; translated_headings=0; heading_mismatch=true; mismatch=true"
                        .to_string(),
                progress: 92,
                level: "warn".to_string(),
            },
        )
        .expect("structure mismatch event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:02.900Z".to_string(),
                stage: "llm_attempt_failed".to_string(),
                message:
                    "chunk_index=1; attempt=1; code=LLM_NETWORK_ERROR; request_elapsed_ms=700; delay_ms=5000; message=connection reset"
                        .to_string(),
                progress: 70,
                level: "warn".to_string(),
            },
        )
        .expect("attempt failure event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:03Z".to_string(),
                stage: "validation_repaired".to_string(),
                message: "applied residual repair to chunk=1".to_string(),
                progress: 93,
                level: "info".to_string(),
            },
        )
        .expect("repair event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:04Z".to_string(),
                stage: "validation_review_skipped".to_string(),
                message: "chunk=2 residual review skipped".to_string(),
                progress: 93,
                level: "warn".to_string(),
            },
        )
        .expect("skipped event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:05Z".to_string(),
                stage: "wave_term_delta".to_string(),
                message: "wave completed; new_term_count=4; chunk_count=2; matched_terms=3; deduped_terms=1; applied_terms=2; already_correct_terms=1; llm_patch_count=1; llm_correct_count=1; fallback_patch_count=1; patched_chunks=1".to_string(),
                progress: 82,
                level: "info".to_string(),
            },
        )
        .expect("wave event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:06Z".to_string(),
                stage: "pipeline_stage_timing".to_string(),
                message: "stage=source_prep; elapsed_ms=400; chars=1200".to_string(),
                progress: 27,
                level: "info".to_string(),
            },
        )
        .expect("pipeline stage event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:07Z".to_string(),
                stage: "pipeline_stage_timing".to_string(),
                message: "stage=translation; elapsed_ms=2100; chunks=2".to_string(),
                progress: 90,
                level: "info".to_string(),
            },
        )
        .expect("pipeline stage event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:07.500Z".to_string(),
                stage: "residual_llm_attempt_started".to_string(),
                message: "chunk_index=1; phase=sentence; attempt=1; model=deepseek-v4-flash; prompt_chars=3000"
                    .to_string(),
                progress: 92,
                level: "debug".to_string(),
            },
        )
        .expect("residual attempt started event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:07.600Z".to_string(),
                stage: "residual_llm_attempt_completed".to_string(),
                message: "chunk_index=1; phase=sentence; attempt=1; model=deepseek-v4-flash; request_elapsed_ms=2500"
                    .to_string(),
                progress: 92,
                level: "debug".to_string(),
            },
        )
        .expect("residual attempt completed event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:07.700Z".to_string(),
                stage: "residual_llm_attempt_completed".to_string(),
                message: "chunk_index=1; phase=chunk; attempt=1; model=deepseek-v4-flash; request_elapsed_ms=4200"
                    .to_string(),
                progress: 92,
                level: "debug".to_string(),
            },
        )
        .expect("residual attempt completed event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:07.800Z".to_string(),
                stage: "residual_llm_attempt_failed".to_string(),
                message: "chunk_index=2; phase=chunk; attempt=1; code=LLM_TIMEOUT; request_elapsed_ms=30000; delay_ms=0; final=true; message=timed out"
                    .to_string(),
                progress: 92,
                level: "warn".to_string(),
            },
        )
        .expect("residual attempt failed event should write");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:08Z".to_string(),
                stage: "resume_audit_warning".to_string(),
                message: "body_pending=0; body_translated=1; body_term_consistency=0; body_residual_reviewed=0; body_merge_ready=0; missing_translation_after_pending=0; translated_pending=0; non_body_unprotected=0; frozen_index_stale=true".to_string(),
                progress: 33,
                level: "warn".to_string(),
            },
        )
        .expect("resume audit event should write");
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Example",
            "Example",
            "Example source.",
            "Example translation.",
        );
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            Some(&event_log_path),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("metrics build");

        assert_eq!(metrics.event_count, 16);
        assert_eq!(metrics.ocr_attempt_event_count, 1);
        assert_eq!(metrics.pipeline_stage_event_count, 2);
        assert_eq!(
            metrics.pipeline_stage_elapsed_ms.get("source_prep"),
            Some(&400)
        );
        assert_eq!(
            metrics.pipeline_stage_elapsed_ms.get("translation"),
            Some(&2_100)
        );
        assert_eq!(
            metrics.slowest_pipeline_stage.as_deref(),
            Some("translation")
        );
        assert_eq!(metrics.slowest_pipeline_stage_elapsed_ms, 2_100);
        assert_eq!(metrics.current_run_started_at, None);
        assert_eq!(metrics.current_run_event_count, 16);
        assert_eq!(metrics.current_run_pipeline_stage_event_count, 2);
        assert_eq!(
            metrics
                .current_run_pipeline_stage_elapsed_ms
                .get("source_prep"),
            Some(&400)
        );
        assert_eq!(
            metrics
                .current_run_pipeline_stage_elapsed_ms
                .get("translation"),
            Some(&2_100)
        );
        assert_eq!(
            metrics.current_run_slowest_pipeline_stage.as_deref(),
            Some("translation")
        );
        assert_eq!(metrics.current_run_slowest_pipeline_stage_elapsed_ms, 2_100);
        assert_eq!(metrics.resume_audit_event_count, 1);
        assert_eq!(metrics.resume_audit_warning_event_count, 1);
        assert_eq!(metrics.chunk_translated_event_count, 2);
        assert_eq!(metrics.current_run_chunk_translated_event_count, 2);
        assert_eq!(metrics.chunk_structure_check_event_count, 2);
        assert_eq!(metrics.chunk_structure_mismatch_count, 1);
        assert_eq!(metrics.chunk_structure_error_rate, 0.5);
        assert_eq!(metrics.chunk_structure_block_delta_total, 2);
        assert_eq!(metrics.chunk_structure_heading_mismatch_count, 1);
        assert_eq!(metrics.llm_attempt_failed_event_count, 1);
        assert_eq!(metrics.llm_attempt_failed_request_elapsed_ms_total, 700);
        assert_eq!(metrics.llm_attempt_failed_delay_ms_total, 5_000);
        assert_eq!(metrics.residual_repair_event_count, 1);
        assert_eq!(metrics.residual_review_skipped_event_count, 1);
        assert_eq!(metrics.residual_llm_attempt_started_event_count, 1);
        assert_eq!(metrics.residual_llm_attempt_completed_event_count, 2);
        assert_eq!(metrics.residual_llm_attempt_failed_event_count, 1);
        assert_eq!(metrics.residual_llm_completed_elapsed_ms_total, 6_700);
        assert_eq!(metrics.residual_llm_slowest_elapsed_ms, 4_200);
        assert_eq!(metrics.translation_event_llm_elapsed_ms_total, 1_800);
        assert_eq!(metrics.translation_event_wait_ms_total, 75);
        assert_eq!(metrics.translation_event_llm_attempts_total, 3);
        assert_eq!(
            metrics.current_run_translation_event_llm_elapsed_ms_total,
            1_800
        );
        assert_eq!(metrics.current_run_translation_event_wait_ms_total, 75);
        assert_eq!(metrics.current_run_translation_event_llm_attempts_total, 3);
        assert_eq!(metrics.wave_term_delta_event_count, 1);
        assert_eq!(metrics.wave_term_delta_total, 4);
        assert_eq!(metrics.wave_term_match_total, 3);
        assert_eq!(metrics.wave_term_deduped_total, 1);
        assert_eq!(metrics.wave_term_applied_total, 2);
        assert_eq!(metrics.wave_term_already_correct_total, 1);
        assert_eq!(metrics.wave_term_llm_patch_total, 1);
        assert_eq!(metrics.wave_term_llm_correct_total, 1);
        assert_eq!(metrics.wave_term_fallback_patch_total, 1);
        assert_eq!(metrics.wave_term_patched_chunk_total, 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn metrics_artifact_separates_current_run_stage_costs_after_resume() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-current-run-metrics-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let event_log_path = root.join("events.ndjson");
        std::fs::create_dir_all(&root).expect("temp dir should exist");
        for event in [
            TaskEvent {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                stage: "queued".to_string(),
                message: "first run".to_string(),
                progress: 5,
                level: "info".to_string(),
            },
            TaskEvent {
                timestamp: "2026-01-01T00:00:01Z".to_string(),
                stage: "pipeline_stage_timing".to_string(),
                message: "stage=translation; elapsed_ms=1000; chunks=2".to_string(),
                progress: 90,
                level: "info".to_string(),
            },
            TaskEvent {
                timestamp: "2026-01-01T00:00:02Z".to_string(),
                stage: "chunk_translated".to_string(),
                message:
                    "translated chunk 1/2; index=0; elapsed_ms=1100; wait_ms=10; llm_elapsed_ms=900; llm_attempts=1"
                        .to_string(),
                progress: 66,
                level: "info".to_string(),
            },
            TaskEvent {
                timestamp: "2026-01-01T00:10:00Z".to_string(),
                stage: "queued".to_string(),
                message: "resumed run".to_string(),
                progress: 5,
                level: "info".to_string(),
            },
            TaskEvent {
                timestamp: "2026-01-01T00:10:01Z".to_string(),
                stage: "pipeline_stage_timing".to_string(),
                message: "stage=checkpoint_load; elapsed_ms=100; chunks=2".to_string(),
                progress: 32,
                level: "info".to_string(),
            },
            TaskEvent {
                timestamp: "2026-01-01T00:10:02Z".to_string(),
                stage: "pipeline_stage_timing".to_string(),
                message: "stage=residual_review; elapsed_ms=200; chunks=2".to_string(),
                progress: 93,
                level: "info".to_string(),
            },
            TaskEvent {
                timestamp: "2026-01-01T00:10:03Z".to_string(),
                stage: "chunk_translated".to_string(),
                message:
                    "translated chunk 2/2; index=1; elapsed_ms=350; wait_ms=20; llm_elapsed_ms=300; llm_attempts=2"
                        .to_string(),
                progress: 90,
                level: "info".to_string(),
            },
        ] {
            append_json_line(&event_log_path, &event).expect("event should write");
        }

        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Example",
            "Example",
            "Example source.",
            "Example translation.",
        );
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            Some(&event_log_path),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("metrics build");

        assert_eq!(
            metrics.pipeline_stage_elapsed_ms.get("translation"),
            Some(&1_000)
        );
        assert_eq!(
            metrics.pipeline_stage_elapsed_ms.get("checkpoint_load"),
            Some(&100)
        );
        assert_eq!(
            metrics.pipeline_stage_elapsed_ms.get("residual_review"),
            Some(&200)
        );
        assert_eq!(
            metrics.current_run_started_at.as_deref(),
            Some("2026-01-01T00:10:00Z")
        );
        assert_eq!(metrics.current_run_event_count, 4);
        assert_eq!(metrics.current_run_pipeline_stage_event_count, 2);
        assert_eq!(
            metrics
                .current_run_pipeline_stage_elapsed_ms
                .get("translation"),
            None
        );
        assert_eq!(
            metrics
                .current_run_pipeline_stage_elapsed_ms
                .get("checkpoint_load"),
            Some(&100)
        );
        assert_eq!(
            metrics
                .current_run_pipeline_stage_elapsed_ms
                .get("residual_review"),
            Some(&200)
        );
        assert_eq!(
            metrics.current_run_slowest_pipeline_stage.as_deref(),
            Some("residual_review")
        );
        assert_eq!(metrics.current_run_chunk_translated_event_count, 1);
        assert_eq!(
            metrics.current_run_translation_event_llm_elapsed_ms_total,
            300
        );
        assert_eq!(metrics.current_run_translation_event_wait_ms_total, 20);
        assert_eq!(metrics.current_run_translation_event_llm_attempts_total, 2);
        assert_eq!(metrics.translation_event_llm_elapsed_ms_total, 1_200);
        assert_eq!(metrics.translation_event_wait_ms_total, 30);
        assert_eq!(metrics.translation_event_llm_attempts_total, 3);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn metrics_reuses_residual_second_check_event() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-residual-metrics-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let event_log_path = root.join("events.ndjson");
        std::fs::create_dir_all(&root).expect("temp dir should exist");
        append_json_line(
            &event_log_path,
            &TaskEvent {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                stage: "validation_review_second_check".to_string(),
                message:
                    "remaining_candidate_chunks=0; remaining_candidate_terms=0; candidate_limit=96; hit_candidate_limit=false"
                        .to_string(),
                progress: 93,
                level: "info".to_string(),
            },
        )
        .expect("second check event should write");
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Contextual,
            "deliberation",
            "审议",
            "This mechanism induces deliberation.",
            "这一机制诱发 deliberation。",
        );
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            Some(&event_log_path),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("metrics build");

        assert_eq!(
            metrics.residual_review_stats_source,
            "residual_second_check_event"
        );
        assert_eq!(metrics.residual_review_candidate_chunk_count, 0);
        assert_eq!(metrics.residual_review_candidate_term_count, 0);
        assert_eq!(metrics.residual_review_candidate_limit, 96);

        let _ = std::fs::remove_dir_all(root);
    }

    fn chunk_state(
        index: usize,
        source: &str,
        translated: Option<&str>,
        segment_kind: ChunkSegmentKind,
        processing_stage: Option<ChunkProcessingStage>,
    ) -> ChunkState {
        ChunkState {
            index,
            source: source.to_string(),
            translated: translated.map(str::to_string),
            confirmed_terms: Vec::new(),
            attempts: u8::from(translated.is_some()),
            updated_at: "now".to_string(),
            segment_kind,
            processing_stage,
        }
    }

    #[tokio::test]
    async fn persists_and_restores_source_sidecar_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-artifact-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let paths = ArtifactPaths::new(&root);
        let source_document = OcrDocument {
            markdown: "Figure 1\n\n![](imgs/example.png)\n\ncaption".to_string(),
            cover_data_url: None,
            toc: Some(sample_toc_artifact()),
            page_layouts: vec![OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "image".to_string(),
                    content: "figure".to_string(),
                    bbox: vec![1, 2, 3, 4],
                    order: Some(1),
                    page: Some(1),
                }],
            }],
            markdown_images: HashMap::from([(
                "imgs/example.png".to_string(),
                format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(b"fake-image")
                ),
            )]),
            epub_blocks: Vec::new(),
            source_blocks: crate::document::build_source_blocks_from_markdown(
                "Figure 1\n\n![](imgs/example.png)\n\ncaption",
            ),
        };

        persist_source_document(&paths, &source_document)
            .await
            .expect("persist source document");
        let restored = load_source_document(&paths)
            .await
            .expect("load source document")
            .expect("restored source document");

        assert_eq!(restored.markdown, source_document.markdown);
        assert_eq!(restored.toc.as_ref().map(|toc| toc.entries.len()), Some(3));
        assert_eq!(restored.page_layouts.len(), 1);
        assert!(!restored.source_blocks.is_empty());
        assert_eq!(restored.source_blocks[0].kind, "paragraph");
        assert_eq!(restored.markdown_images.len(), 1);
        assert!(paths.source_toc_path.exists());
        assert!(root.join("imgs/example.png").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    fn sample_toc_artifact() -> TocArtifact {
        TocArtifact {
            source_kind: TocSourceKind::EpubNcx,
            confidence: 0.95,
            entries: vec![
                TocEntry {
                    source_title: "Chapter 1".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 0,
                    page: None,
                    href: Some("chapter-1.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.95,
                },
                TocEntry {
                    source_title: "Chapter 2".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 1,
                    page: None,
                    href: None,
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.55,
                },
                TocEntry {
                    source_title: "Chapter 2".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 2,
                    page: None,
                    href: Some("chapter-2-duplicate.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.95,
                },
            ],
        }
    }
}
