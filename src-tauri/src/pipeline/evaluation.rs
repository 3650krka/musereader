use super::checkpoint;
use super::types::{ArtifactManifest, ChunkCheckpoint, ChunkSegmentKind, TaskEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EvaluationSampleManifest {
    sample_id: String,
    input_kind: String,
    article_type: String,
    source_path: String,
    expected_focus: Vec<String>,
    manual_review_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EvaluationMetrics {
    sample_id: String,
    input_kind: String,
    article_type: String,
    quality: QualityMetrics,
    speed: SpeedMetrics,
    cost: CostMetrics,
    maintainability: MaintainabilityMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct QualityMetrics {
    validation_issue_count: usize,
    residual_candidate_count: usize,
    residual_final_count: usize,
    proper_noun_multi_translation_count: usize,
    protected_region_translation_count: usize,
    mojibake_count: usize,
    toc_entry_count: usize,
    toc_low_confidence_entry_count: usize,
    toc_duplicate_title_count: usize,
    toc_missing_locator_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SpeedMetrics {
    pipeline_total_ms: u128,
    chunk_translate_avg_ms: u128,
    chunk_translate_p95_ms: u128,
    checkpoint_write_total_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CostMetrics {
    model_call_count: usize,
    residual_repair_call_count: usize,
    retry_count: usize,
    estimated_prompt_tokens: usize,
    estimated_completion_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct MaintainabilityMetrics {
    stage_contract_passed: bool,
    checkpoint_contract_passed: bool,
    artifact_contract_passed: bool,
    report_has_stage_breakdown: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EvaluationComparison {
    sample_id: String,
    before: EvaluationMetrics,
    after: EvaluationMetrics,
    diff: EvaluationMetricDiff,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ManualReviewChecklist {
    sample_id: String,
    required_checks: Vec<ManualReviewCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ManualReviewCheck {
    category: String,
    prompt: String,
    pass_condition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EvaluationMetricDiff {
    validation_issue_delta: isize,
    residual_final_delta: isize,
    proper_noun_multi_translation_delta: isize,
    toc_entry_delta: isize,
    toc_low_confidence_entry_delta: isize,
    toc_duplicate_title_delta: isize,
    toc_missing_locator_delta: isize,
    pipeline_total_ms_delta: i128,
    model_call_delta: isize,
    retry_delta: isize,
}

fn validate_event_log_contract(events: &[TaskEvent]) -> Result<(), String> {
    if events.is_empty() {
        return Err("event log is empty".to_string());
    }
    if events.first().is_some_and(|event| event.stage != "queued") {
        return Err("event log must start with queued".to_string());
    }
    if events
        .windows(2)
        .any(|pair| pair[1].progress < pair[0].progress)
    {
        return Err("event progress must be monotonic".to_string());
    }
    let stages = events
        .iter()
        .map(|event| event.stage.as_str())
        .collect::<HashSet<_>>();
    for required in ["queued", "translating", "finalized"] {
        if !stages.contains(required) {
            return Err(format!("missing required stage: {required}"));
        }
    }
    Ok(())
}

fn validate_checkpoint_contract(checkpoint: &ChunkCheckpoint) -> Result<(), String> {
    if checkpoint.chunks.is_empty() {
        return Err("checkpoint has no chunks".to_string());
    }
    for (expected_index, chunk) in checkpoint.chunks.iter().enumerate() {
        if chunk.index != expected_index {
            return Err(format!(
                "chunk index is not contiguous: expected {expected_index}, got {}",
                chunk.index
            ));
        }
        if chunk.segment_kind == ChunkSegmentKind::Body && chunk.source.trim().is_empty() {
            return Err(format!("body chunk {} has empty source", chunk.index));
        }
        if let Some(translated) = chunk.translated.as_deref() {
            checkpoint::reject_mojibake_translation(chunk.index, translated)
                .map_err(|error| error.message)?;
        }
    }
    Ok(())
}

fn validate_artifact_contract(manifest: &ArtifactManifest) -> Result<(), String> {
    if manifest.version == 0 {
        return Err("manifest version must be positive".to_string());
    }
    if manifest.translated_chunks > manifest.total_chunks {
        return Err("translated chunks cannot exceed total chunks".to_string());
    }
    let kinds = manifest
        .files
        .iter()
        .map(|file| file.kind.as_str())
        .collect::<HashSet<_>>();
    for required in [
        "sourceMarkdown",
        "translatedMarkdown",
        "translatedHtml",
        "checkpoint",
        "eventLog",
        "validationReport",
        "glossary",
        "metrics",
    ] {
        if !kinds.contains(required) {
            return Err(format!("manifest missing artifact file: {required}"));
        }
    }
    Ok(())
}

fn build_comparison(before: EvaluationMetrics, after: EvaluationMetrics) -> EvaluationComparison {
    EvaluationComparison {
        sample_id: after.sample_id.clone(),
        diff: EvaluationMetricDiff {
            validation_issue_delta: after.quality.validation_issue_count as isize
                - before.quality.validation_issue_count as isize,
            residual_final_delta: after.quality.residual_final_count as isize
                - before.quality.residual_final_count as isize,
            proper_noun_multi_translation_delta: after.quality.proper_noun_multi_translation_count
                as isize
                - before.quality.proper_noun_multi_translation_count as isize,
            toc_entry_delta: after.quality.toc_entry_count as isize
                - before.quality.toc_entry_count as isize,
            toc_low_confidence_entry_delta: after.quality.toc_low_confidence_entry_count as isize
                - before.quality.toc_low_confidence_entry_count as isize,
            toc_duplicate_title_delta: after.quality.toc_duplicate_title_count as isize
                - before.quality.toc_duplicate_title_count as isize,
            toc_missing_locator_delta: after.quality.toc_missing_locator_count as isize
                - before.quality.toc_missing_locator_count as isize,
            pipeline_total_ms_delta: after.speed.pipeline_total_ms as i128
                - before.speed.pipeline_total_ms as i128,
            model_call_delta: after.cost.model_call_count as isize
                - before.cost.model_call_count as isize,
            retry_delta: after.cost.retry_count as isize - before.cost.retry_count as isize,
        },
        before,
        after,
    }
}

fn render_comparison_markdown(comparison: &EvaluationComparison) -> String {
    format!(
        concat!(
            "# Evaluation Comparison: {sample_id}\n\n",
            "| Dimension | Metric | Before | After | Delta |\n",
            "|---|---:|---:|---:|---:|\n",
            "| Quality | validation issues | {before_issues} | {after_issues} | {issue_delta} |\n",
            "| Quality | final residual English | {before_residual} | {after_residual} | {residual_delta} |\n",
            "| Quality | proper noun multi translations | {before_nouns} | {after_nouns} | {noun_delta} |\n",
            "| Quality | TOC entries | {before_toc_entries} | {after_toc_entries} | {toc_entry_delta} |\n",
            "| Quality | low confidence TOC entries | {before_toc_low} | {after_toc_low} | {toc_low_delta} |\n",
            "| Quality | duplicate TOC titles | {before_toc_dupes} | {after_toc_dupes} | {toc_dupe_delta} |\n",
            "| Quality | TOC missing locators | {before_toc_missing} | {after_toc_missing} | {toc_missing_delta} |\n",
            "| Speed | pipeline total ms | {before_ms} | {after_ms} | {ms_delta} |\n",
            "| Cost | model calls | {before_calls} | {after_calls} | {call_delta} |\n",
            "| Cost | retries | {before_retries} | {after_retries} | {retry_delta} |\n\n",
            "## Contract Gates\n\n",
            "| Gate | Before | After |\n",
            "|---|---:|---:|\n",
            "| stage contract | {before_stage} | {after_stage} |\n",
            "| checkpoint contract | {before_checkpoint} | {after_checkpoint} |\n",
            "| artifact contract | {before_artifact} | {after_artifact} |\n",
            "| stage breakdown | {before_breakdown} | {after_breakdown} |\n"
        ),
        sample_id = comparison.sample_id,
        before_issues = comparison.before.quality.validation_issue_count,
        after_issues = comparison.after.quality.validation_issue_count,
        issue_delta = comparison.diff.validation_issue_delta,
        before_residual = comparison.before.quality.residual_final_count,
        after_residual = comparison.after.quality.residual_final_count,
        residual_delta = comparison.diff.residual_final_delta,
        before_nouns = comparison.before.quality.proper_noun_multi_translation_count,
        after_nouns = comparison.after.quality.proper_noun_multi_translation_count,
        noun_delta = comparison.diff.proper_noun_multi_translation_delta,
        before_toc_entries = comparison.before.quality.toc_entry_count,
        after_toc_entries = comparison.after.quality.toc_entry_count,
        toc_entry_delta = comparison.diff.toc_entry_delta,
        before_toc_low = comparison.before.quality.toc_low_confidence_entry_count,
        after_toc_low = comparison.after.quality.toc_low_confidence_entry_count,
        toc_low_delta = comparison.diff.toc_low_confidence_entry_delta,
        before_toc_dupes = comparison.before.quality.toc_duplicate_title_count,
        after_toc_dupes = comparison.after.quality.toc_duplicate_title_count,
        toc_dupe_delta = comparison.diff.toc_duplicate_title_delta,
        before_toc_missing = comparison.before.quality.toc_missing_locator_count,
        after_toc_missing = comparison.after.quality.toc_missing_locator_count,
        toc_missing_delta = comparison.diff.toc_missing_locator_delta,
        before_ms = comparison.before.speed.pipeline_total_ms,
        after_ms = comparison.after.speed.pipeline_total_ms,
        ms_delta = comparison.diff.pipeline_total_ms_delta,
        before_calls = comparison.before.cost.model_call_count,
        after_calls = comparison.after.cost.model_call_count,
        call_delta = comparison.diff.model_call_delta,
        before_retries = comparison.before.cost.retry_count,
        after_retries = comparison.after.cost.retry_count,
        retry_delta = comparison.diff.retry_delta,
        before_stage = comparison.before.maintainability.stage_contract_passed,
        after_stage = comparison.after.maintainability.stage_contract_passed,
        before_checkpoint = comparison.before.maintainability.checkpoint_contract_passed,
        after_checkpoint = comparison.after.maintainability.checkpoint_contract_passed,
        before_artifact = comparison.before.maintainability.artifact_contract_passed,
        after_artifact = comparison.after.maintainability.artifact_contract_passed,
        before_breakdown = comparison.before.maintainability.report_has_stage_breakdown,
        after_breakdown = comparison.after.maintainability.report_has_stage_breakdown,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::types::{ArtifactFile, GlossaryArtifact, ValidationSummary};

    #[test]
    fn sample_manifest_schema_round_trips_as_camel_case_json() {
        let manifest = EvaluationSampleManifest {
            sample_id: "academic_pdf_001".to_string(),
            input_kind: "pdf".to_string(),
            article_type: "academic".to_string(),
            source_path: "samples/academic_pdf/sample.pdf".to_string(),
            expected_focus: vec!["references".to_string(), "ocr_layout".to_string()],
            manual_review_points: vec!["reference metadata must stay protected".to_string()],
        };

        let raw = serde_json::to_string(&manifest).expect("manifest should serialize");
        assert!(raw.contains("sampleId"));
        assert!(raw.contains("manualReviewPoints"));

        let decoded: EvaluationSampleManifest =
            serde_json::from_str(&raw).expect("manifest should deserialize");
        assert_eq!(decoded, manifest);
    }

    #[test]
    fn evaluation_metrics_schema_keeps_four_dimensions() {
        let metrics = sample_metrics("sample-1", 4, 2, 1_000);

        let raw = serde_json::to_value(&metrics).expect("metrics should serialize");

        assert!(raw.get("quality").is_some());
        assert!(raw.get("speed").is_some());
        assert!(raw.get("cost").is_some());
        assert!(raw.get("maintainability").is_some());
    }

    #[test]
    fn comparison_report_quantifies_before_after_deltas() {
        let before = sample_metrics("sample-1", 4, 3, 1_000);
        let after = sample_metrics("sample-1", 2, 2, 1_200);

        let comparison = build_comparison(before, after);

        assert_eq!(comparison.diff.validation_issue_delta, -2);
        assert_eq!(comparison.diff.model_call_delta, -1);
        assert_eq!(comparison.diff.pipeline_total_ms_delta, 200);
        assert_eq!(comparison.diff.toc_entry_delta, -2);
        assert_eq!(comparison.diff.toc_missing_locator_delta, -1);
    }

    #[test]
    fn comparison_report_renders_four_dimension_markdown() {
        let before = sample_metrics("sample-1", 4, 3, 1_000);
        let after = sample_metrics("sample-1", 2, 2, 1_200);
        let comparison = build_comparison(before, after);

        let report = render_comparison_markdown(&comparison);

        assert!(report.contains("| Quality | validation issues | 4 | 2 | -2 |"));
        assert!(report.contains("| Quality | TOC entries | 6 | 4 | -2 |"));
        assert!(report.contains("| Quality | TOC missing locators | 2 | 1 | -1 |"));
        assert!(report.contains("| Speed | pipeline total ms | 1000 | 1200 | 200 |"));
        assert!(report.contains("| Cost | model calls | 3 | 2 | -1 |"));
        assert!(report.contains("Contract Gates"));
    }

    #[test]
    fn manual_review_checklist_schema_keeps_pass_conditions() {
        let checklist = ManualReviewChecklist {
            sample_id: "academic_pdf_001".to_string(),
            required_checks: vec![ManualReviewCheck {
                category: "protected_regions".to_string(),
                prompt: "references and metadata should not be translated".to_string(),
                pass_condition: "protected region translations remain zero".to_string(),
            }],
        };

        let raw = serde_json::to_string(&checklist).expect("checklist should serialize");
        assert!(raw.contains("passCondition"));

        let decoded: ManualReviewChecklist =
            serde_json::from_str(&raw).expect("checklist should deserialize");
        assert_eq!(decoded, checklist);
    }

    #[test]
    fn event_log_contract_rejects_non_monotonic_progress() {
        let events = vec![
            event("queued", 5),
            event("translating", 40),
            event("finalized", 30),
        ];

        let error = validate_event_log_contract(&events).expect_err("contract should fail");

        assert!(error.contains("monotonic"));
    }

    #[test]
    fn checkpoint_contract_rejects_index_gaps_and_mojibake() {
        let mut checkpoint = sample_checkpoint();
        checkpoint.chunks[1].index = 3;

        let error = validate_checkpoint_contract(&checkpoint).expect_err("index gap should fail");

        assert!(error.contains("contiguous"));

        let mut checkpoint = sample_checkpoint();
        checkpoint.chunks[0].translated = Some("��鏋佸寳涔嬪湴��".to_string());

        let error = validate_checkpoint_contract(&checkpoint).expect_err("mojibake should fail");

        assert!(error.contains("encoding pollution"));
    }

    #[test]
    fn artifact_contract_requires_core_output_files() {
        let mut manifest = sample_artifact_manifest();
        manifest.files.retain(|file| file.kind != "metrics");

        let error = validate_artifact_contract(&manifest).expect_err("missing metrics should fail");

        assert!(error.contains("metrics"));
    }

    fn sample_metrics(
        sample_id: &str,
        validation_issue_count: usize,
        model_call_count: usize,
        pipeline_total_ms: u128,
    ) -> EvaluationMetrics {
        EvaluationMetrics {
            sample_id: sample_id.to_string(),
            input_kind: "epub".to_string(),
            article_type: "fiction".to_string(),
            quality: QualityMetrics {
                validation_issue_count,
                residual_candidate_count: 2,
                residual_final_count: validation_issue_count / 2,
                proper_noun_multi_translation_count: 1,
                protected_region_translation_count: 0,
                mojibake_count: 0,
                toc_entry_count: validation_issue_count + 2,
                toc_low_confidence_entry_count: 1,
                toc_duplicate_title_count: 0,
                toc_missing_locator_count: validation_issue_count / 2,
            },
            speed: SpeedMetrics {
                pipeline_total_ms,
                chunk_translate_avg_ms: 100,
                chunk_translate_p95_ms: 150,
                checkpoint_write_total_ms: 20,
            },
            cost: CostMetrics {
                model_call_count,
                residual_repair_call_count: 1,
                retry_count: 0,
                estimated_prompt_tokens: 1_000,
                estimated_completion_tokens: 800,
            },
            maintainability: MaintainabilityMetrics {
                stage_contract_passed: true,
                checkpoint_contract_passed: true,
                artifact_contract_passed: true,
                report_has_stage_breakdown: true,
            },
        }
    }

    fn event(stage: &str, progress: u8) -> TaskEvent {
        TaskEvent {
            timestamp: "2026-05-27T00:00:00Z".to_string(),
            stage: stage.to_string(),
            message: stage.to_string(),
            progress,
            level: "info".to_string(),
        }
    }

    fn sample_checkpoint() -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "task-1".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            source_hash: "source-hash".to_string(),
            chunk_size: 1000,
            created_at: "2026-05-27T00:00:00Z".to_string(),
            updated_at: "2026-05-27T00:00:01Z".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![
                super::super::types::ChunkState {
                    index: 0,
                    source: "Satampra Zeiros crossed the square.".to_string(),
                    translated: Some("萨坦普拉路泽罗斯穿过了广场。".to_string()),
                    confirmed_terms: Vec::new(),
                    attempts: 1,
                    updated_at: "2026-05-27T00:00:01Z".to_string(),
                    segment_kind: ChunkSegmentKind::Body,
                    processing_stage: None,
                },
                super::super::types::ChunkState {
                    index: 1,
                    source: "References".to_string(),
                    translated: Some("References".to_string()),
                    confirmed_terms: Vec::new(),
                    attempts: 0,
                    updated_at: "2026-05-27T00:00:01Z".to_string(),
                    segment_kind: ChunkSegmentKind::Reference,
                    processing_stage: None,
                },
            ],
        }
    }

    fn sample_artifact_manifest() -> ArtifactManifest {
        ArtifactManifest {
            version: 1,
            task_id: "task-1".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: None,
            glossary: Some(GlossaryArtifact {
                article_type: "fiction".to_string(),
                entries: Vec::new(),
                grouped_hits: Vec::new(),
            }),
            source_hash: "source-hash".to_string(),
            created_at: "2026-05-27T00:00:00Z".to_string(),
            updated_at: "2026-05-27T00:00:01Z".to_string(),
            total_chunks: 2,
            translated_chunks: 2,
            validation_summary: ValidationSummary {
                issue_count: 0,
                untranslated_chunk_count: 0,
                empty_chunk_count: 0,
                markdown_heading_count: 0,
            },
            files: [
                "sourceMarkdown",
                "sourceBlocks",
                "translatedMarkdown",
                "translatedHtml",
                "translatedBlocks",
                "checkpoint",
                "eventLog",
                "validationReport",
                "glossary",
                "metrics",
            ]
            .into_iter()
            .map(|kind| ArtifactFile {
                kind: kind.to_string(),
                path: format!("{kind}.json"),
                bytes: 1,
            })
            .collect(),
        }
    }
}
