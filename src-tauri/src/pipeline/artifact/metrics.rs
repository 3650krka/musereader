use super::toc::{count_exact_heading_structure_matches, count_numbering_normalized_toc_matches};
use super::*;
use crate::llm::LlmUsageTotals;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant as StdInstant;

const LONG_TAIL_CHUNK_THRESHOLD_MS: u128 = 120_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetricsArtifact {
    pub created_at: String,
    pub elapsed_ms: Option<u128>,
    pub total_chunks: usize,
    pub translated_chunks: usize,
    pub body_chunk_count: usize,
    pub reference_chunk_count: usize,
    pub protected_metadata_chunk_count: usize,
    pub pdf_table_chunk_count: usize,
    pub body_pending_chunk_count: usize,
    pub body_translated_stage_chunk_count: usize,
    pub body_term_consistency_stage_chunk_count: usize,
    pub body_residual_reviewed_stage_chunk_count: usize,
    pub body_merge_ready_stage_chunk_count: usize,
    pub body_translation_failed_chunk_count: usize,
    pub protected_passthrough_chunk_count: usize,
    pub event_count: usize,
    pub chunk_translated_event_count: usize,
    pub chunk_structure_check_event_count: usize,
    pub chunk_structure_mismatch_count: usize,
    pub chunk_structure_error_rate: f32,
    pub chunk_structure_block_delta_total: usize,
    pub chunk_structure_heading_mismatch_count: usize,
    pub residual_repair_event_count: usize,
    pub residual_review_skipped_event_count: usize,
    pub ocr_attempt_event_count: usize,
    pub pipeline_stage_event_count: usize,
    pub pipeline_stage_elapsed_ms: BTreeMap<String, u128>,
    pub slowest_pipeline_stage: Option<String>,
    pub slowest_pipeline_stage_elapsed_ms: u128,
    pub current_run_started_at: Option<String>,
    pub current_run_event_count: usize,
    pub current_run_chunk_translated_event_count: usize,
    pub current_run_pipeline_stage_event_count: usize,
    pub current_run_pipeline_stage_elapsed_ms: BTreeMap<String, u128>,
    pub current_run_slowest_pipeline_stage: Option<String>,
    pub current_run_slowest_pipeline_stage_elapsed_ms: u128,
    pub current_run_translation_event_llm_elapsed_ms_total: u128,
    pub current_run_translation_event_wait_ms_total: u128,
    pub current_run_translation_event_route_wait_ms_total: u128,
    pub current_run_translation_event_llm_attempts_total: usize,
    pub resume_audit_event_count: usize,
    pub resume_audit_warning_event_count: usize,
    pub translation_event_llm_elapsed_ms_total: u128,
    pub translation_event_wait_ms_total: u128,
    pub translation_event_route_wait_ms_total: u128,
    pub translation_event_llm_attempts_total: usize,
    pub long_tail_chunk_threshold_ms: u128,
    pub long_tail_chunk_count: usize,
    pub slowest_chunk_index: Option<usize>,
    pub slowest_chunk_elapsed_ms: u128,
    pub slowest_chunk_llm_elapsed_ms: u128,
    pub chunk_elapsed_p95_ms: u128,
    pub llm_usage_response_count: u64,
    pub llm_prompt_tokens: u64,
    pub llm_cached_prompt_tokens: u64,
    pub llm_uncached_prompt_tokens: u64,
    pub llm_completion_tokens: u64,
    pub llm_total_tokens: u64,
    pub estimated_deepseek_v4_flash_cost_usd: Option<f64>,
    pub llm_attempt_failed_event_count: usize,
    pub llm_attempt_failed_request_elapsed_ms_total: u128,
    pub llm_attempt_failed_delay_ms_total: u128,
    pub wave_term_delta_event_count: usize,
    pub wave_term_delta_total: usize,
    pub wave_term_match_total: usize,
    pub wave_term_deduped_total: usize,
    pub wave_term_applied_total: usize,
    pub wave_term_already_correct_total: usize,
    pub wave_term_llm_patch_total: usize,
    pub wave_term_llm_correct_total: usize,
    pub wave_term_fallback_patch_total: usize,
    pub wave_term_patched_chunk_total: usize,
    pub frozen_glossary_seq: Option<u64>,
    pub frozen_patch_index_version: Option<u32>,
    pub frozen_glossary_indexed_chunk_count: usize,
    pub frozen_glossary_patch_term_count: usize,
    pub toc_source_kind: Option<String>,
    pub toc_entry_count: usize,
    pub toc_confidence: Option<f32>,
    pub toc_low_confidence_entry_count: usize,
    pub toc_duplicate_title_count: usize,
    pub toc_missing_locator_count: usize,
    pub translated_toc_entry_count: usize,
    pub translated_toc_missing_title_count: usize,
    pub translated_toc_structural_mismatch_count: usize,
    pub translated_toc_locator_mutation_count: usize,
    pub translated_toc_matched_title_count: usize,
    pub translated_toc_unmatched_title_count: usize,
    pub translated_toc_fuzzy_match_count: usize,
    pub translated_toc_exact_structure_match_count: usize,
    pub translated_toc_title_backfill_rate: f32,
    pub chunks_with_confirmed_terms: usize,
    pub confirmed_term_count: usize,
    pub strict_glossary_entries: usize,
    pub contextual_glossary_entries: usize,
    pub proper_noun_hint_count: usize,
    pub targeted_proper_noun_hint_count: usize,
    pub algorithmic_proper_noun_hint_count: usize,
    pub strict_proper_noun_hint_count: usize,
    pub contextual_proper_noun_hint_count: usize,
    pub missing_proper_noun_target_count: usize,
    pub duplicate_proper_noun_source_count: usize,
    pub proper_noun_hint_occurrence_total: usize,
    pub proper_noun_hint_indexed_chunk_total: usize,
    pub proper_noun_hint_max_indexed_chunk_count: usize,
    pub multi_translation_confirmed_term_count: usize,
    pub validation_issue_count: usize,
    pub residual_english_issue_count: usize,
    pub residual_review_candidate_chunk_count: usize,
    pub residual_review_candidate_term_count: usize,
    pub residual_review_candidate_limit: usize,
    pub residual_review_hit_candidate_limit: bool,
    pub residual_review_stats_source: String,
    pub residual_second_check_candidate_chunk_count: usize,
    pub residual_second_check_candidate_term_count: usize,
    pub residual_second_check_hit_candidate_limit: bool,
    pub residual_llm_attempt_started_event_count: usize,
    pub residual_llm_attempt_completed_event_count: usize,
    pub residual_llm_attempt_failed_event_count: usize,
    pub residual_llm_completed_elapsed_ms_total: u128,
    pub residual_llm_slowest_elapsed_ms: u128,
    pub strict_consistency_issue_count: usize,
    pub checkpoint_write_count: usize,
    pub checkpoint_write_total_ms: u128,
    pub metrics_build_stage_elapsed_ms: BTreeMap<String, u128>,
}

pub(crate) fn build_metrics_artifact(
    checkpoint: &ChunkCheckpoint,
    validation_report: &ValidationReport,
    glossary: &GlossaryArtifact,
    event_log_path: Option<&Path>,
    toc: Option<&crate::document::TocArtifact>,
    translated_toc: Option<&crate::document::TocArtifact>,
    source_markdown: Option<&str>,
    translated_markdown: Option<&str>,
    llm_usage: Option<LlmUsageTotals>,
) -> Result<MetricsArtifact, AppError> {
    let mut metrics_build_stage_elapsed_ms = BTreeMap::new();
    let stage_started = StdInstant::now();
    let elapsed_ms = checkpoint_elapsed_ms(checkpoint)?;
    record_metrics_build_stage(
        &mut metrics_build_stage_elapsed_ms,
        "checkpoint_elapsed",
        stage_started,
    );
    let stage_started = StdInstant::now();
    let translated_chunks = checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.translated.is_some())
        .count();
    let stage_metrics = collect_chunk_stage_metrics(checkpoint);
    record_metrics_build_stage(
        &mut metrics_build_stage_elapsed_ms,
        "chunk_stage_scan",
        stage_started,
    );
    let stage_started = StdInstant::now();
    let event_metrics = collect_event_log_metrics(event_log_path)?;
    record_metrics_build_stage(
        &mut metrics_build_stage_elapsed_ms,
        "event_log_scan",
        stage_started,
    );
    let stage_started = StdInstant::now();
    let chunks_with_confirmed_terms = checkpoint
        .chunks
        .iter()
        .filter(|chunk| !chunk.confirmed_terms.is_empty())
        .count();
    let confirmed_term_count = checkpoint
        .chunks
        .iter()
        .map(|chunk| chunk.confirmed_terms.len())
        .sum();
    let strict_glossary_entries = glossary
        .entries
        .iter()
        .filter(|entry| entry.category == "strict_term")
        .count();
    let contextual_glossary_entries = glossary
        .entries
        .iter()
        .filter(|entry| entry.category == "contextual_term")
        .count();
    let proper_noun_metrics = collect_proper_noun_metrics(checkpoint);
    record_metrics_build_stage(
        &mut metrics_build_stage_elapsed_ms,
        "term_and_proper_noun_scan",
        stage_started,
    );
    let stage_started = StdInstant::now();
    let residual_english_issue_count = validation_report
        .issues
        .iter()
        .filter(|issue| issue.code == "RESIDUAL_ENGLISH_WORD")
        .count();
    let residual_review_stats = select_residual_review_stats(checkpoint, &event_metrics);
    record_metrics_build_stage(
        &mut metrics_build_stage_elapsed_ms,
        "residual_stats",
        stage_started,
    );
    let stage_started = StdInstant::now();
    let toc_quality_metrics = collect_toc_quality_metrics(toc);
    let translated_toc_metrics =
        collect_translated_toc_metrics(toc, translated_toc, source_markdown, translated_markdown);
    record_metrics_build_stage(
        &mut metrics_build_stage_elapsed_ms,
        "toc_metrics",
        stage_started,
    );
    let strict_consistency_issue_count = validation_report
        .issues
        .iter()
        .filter(|issue| {
            matches!(
                issue.code.as_str(),
                "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION" | "UNRESOLVED_PROPER_NOUN"
            )
        })
        .count();
    let usage = llm_usage.unwrap_or_default();

    Ok(MetricsArtifact {
        created_at: Utc::now().to_rfc3339(),
        elapsed_ms,
        total_chunks: checkpoint.chunks.len(),
        translated_chunks,
        body_chunk_count: stage_metrics.body_chunk_count,
        reference_chunk_count: stage_metrics.reference_chunk_count,
        protected_metadata_chunk_count: stage_metrics.protected_metadata_chunk_count,
        pdf_table_chunk_count: stage_metrics.pdf_table_chunk_count,
        body_pending_chunk_count: stage_metrics.body_pending_chunk_count,
        body_translated_stage_chunk_count: stage_metrics.body_translated_stage_chunk_count,
        body_term_consistency_stage_chunk_count: stage_metrics
            .body_term_consistency_stage_chunk_count,
        body_residual_reviewed_stage_chunk_count: stage_metrics
            .body_residual_reviewed_stage_chunk_count,
        body_merge_ready_stage_chunk_count: stage_metrics.body_merge_ready_stage_chunk_count,
        body_translation_failed_chunk_count: stage_metrics.body_translation_failed_chunk_count,
        protected_passthrough_chunk_count: stage_metrics.protected_passthrough_chunk_count,
        event_count: event_metrics.event_count,
        chunk_translated_event_count: event_metrics.chunk_translated_event_count,
        chunk_structure_check_event_count: event_metrics.chunk_structure_check_event_count,
        chunk_structure_mismatch_count: event_metrics.chunk_structure_mismatch_count,
        chunk_structure_error_rate: event_metrics.chunk_structure_error_rate(),
        chunk_structure_block_delta_total: event_metrics.chunk_structure_block_delta_total,
        chunk_structure_heading_mismatch_count: event_metrics
            .chunk_structure_heading_mismatch_count,
        residual_repair_event_count: event_metrics.residual_repair_event_count,
        residual_review_skipped_event_count: event_metrics.residual_review_skipped_event_count,
        ocr_attempt_event_count: event_metrics.ocr_attempt_event_count,
        pipeline_stage_event_count: event_metrics.pipeline_stage_event_count,
        pipeline_stage_elapsed_ms: event_metrics.pipeline_stage_elapsed_ms.clone(),
        slowest_pipeline_stage: event_metrics.slowest_pipeline_stage.clone(),
        slowest_pipeline_stage_elapsed_ms: event_metrics.slowest_pipeline_stage_elapsed_ms,
        current_run_started_at: event_metrics.current_run_started_at.clone(),
        current_run_event_count: event_metrics.current_run_event_count,
        current_run_chunk_translated_event_count: event_metrics
            .current_run_chunk_translated_event_count,
        current_run_pipeline_stage_event_count: event_metrics
            .current_run_pipeline_stage_event_count,
        current_run_pipeline_stage_elapsed_ms: event_metrics
            .current_run_pipeline_stage_elapsed_ms
            .clone(),
        current_run_slowest_pipeline_stage: event_metrics
            .current_run_slowest_pipeline_stage
            .clone(),
        current_run_slowest_pipeline_stage_elapsed_ms: event_metrics
            .current_run_slowest_pipeline_stage_elapsed_ms,
        current_run_translation_event_llm_elapsed_ms_total: event_metrics
            .current_run_translation_event_llm_elapsed_ms_total,
        current_run_translation_event_wait_ms_total: event_metrics
            .current_run_translation_event_wait_ms_total,
        current_run_translation_event_route_wait_ms_total: event_metrics
            .current_run_translation_event_route_wait_ms_total,
        current_run_translation_event_llm_attempts_total: event_metrics
            .current_run_translation_event_llm_attempts_total,
        resume_audit_event_count: event_metrics.resume_audit_event_count,
        resume_audit_warning_event_count: event_metrics.resume_audit_warning_event_count,
        translation_event_llm_elapsed_ms_total: event_metrics
            .translation_event_llm_elapsed_ms_total,
        translation_event_wait_ms_total: event_metrics.translation_event_wait_ms_total,
        translation_event_route_wait_ms_total: event_metrics.translation_event_route_wait_ms_total,
        translation_event_llm_attempts_total: event_metrics.translation_event_llm_attempts_total,
        long_tail_chunk_threshold_ms: LONG_TAIL_CHUNK_THRESHOLD_MS,
        long_tail_chunk_count: event_metrics.long_tail_chunk_count,
        slowest_chunk_index: event_metrics.slowest_chunk_index,
        slowest_chunk_elapsed_ms: event_metrics.slowest_chunk_elapsed_ms,
        slowest_chunk_llm_elapsed_ms: event_metrics.slowest_chunk_llm_elapsed_ms,
        chunk_elapsed_p95_ms: event_metrics.chunk_elapsed_p95_ms(),
        llm_usage_response_count: usage.response_count,
        llm_prompt_tokens: usage.prompt_tokens,
        llm_cached_prompt_tokens: usage.cached_prompt_tokens,
        llm_uncached_prompt_tokens: usage.uncached_prompt_tokens,
        llm_completion_tokens: usage.completion_tokens,
        llm_total_tokens: usage.total_tokens,
        estimated_deepseek_v4_flash_cost_usd: estimate_deepseek_v4_flash_cost_usd(usage),
        llm_attempt_failed_event_count: event_metrics.llm_attempt_failed_event_count,
        llm_attempt_failed_request_elapsed_ms_total: event_metrics
            .llm_attempt_failed_request_elapsed_ms_total,
        llm_attempt_failed_delay_ms_total: event_metrics.llm_attempt_failed_delay_ms_total,
        wave_term_delta_event_count: event_metrics.wave_term_delta_event_count,
        wave_term_delta_total: event_metrics.wave_term_delta_total,
        wave_term_match_total: event_metrics.wave_term_match_total,
        wave_term_deduped_total: event_metrics.wave_term_deduped_total,
        wave_term_applied_total: event_metrics.wave_term_applied_total,
        wave_term_already_correct_total: event_metrics.wave_term_already_correct_total,
        wave_term_llm_patch_total: event_metrics.wave_term_llm_patch_total,
        wave_term_llm_correct_total: event_metrics.wave_term_llm_correct_total,
        wave_term_fallback_patch_total: event_metrics.wave_term_fallback_patch_total,
        wave_term_patched_chunk_total: event_metrics.wave_term_patched_chunk_total,
        frozen_glossary_seq: checkpoint.delta_sync.frozen_at_glossary_seq,
        frozen_patch_index_version: checkpoint.delta_sync.frozen_patch_index_version,
        frozen_glossary_indexed_chunk_count: checkpoint.delta_sync.frozen_chunk_patch_terms.len(),
        frozen_glossary_patch_term_count: checkpoint
            .delta_sync
            .frozen_chunk_patch_terms
            .values()
            .map(Vec::len)
            .sum(),
        toc_source_kind: toc.map(|toc| toc_source_kind_label(toc.source_kind).to_string()),
        toc_entry_count: toc.map(|toc| toc.entries.len()).unwrap_or_default(),
        toc_confidence: toc.map(|toc| toc.confidence),
        toc_low_confidence_entry_count: toc_quality_metrics.low_confidence_entry_count,
        toc_duplicate_title_count: toc_quality_metrics.duplicate_title_count,
        toc_missing_locator_count: toc_quality_metrics.missing_locator_count,
        translated_toc_entry_count: translated_toc
            .map(|toc| toc.entries.len())
            .unwrap_or_default(),
        translated_toc_missing_title_count: translated_toc_metrics.missing_title_count,
        translated_toc_structural_mismatch_count: translated_toc_metrics.structural_mismatch_count,
        translated_toc_locator_mutation_count: translated_toc_metrics.locator_mutation_count,
        translated_toc_matched_title_count: translated_toc_metrics.matched_title_count,
        translated_toc_unmatched_title_count: translated_toc_metrics.unmatched_title_count,
        translated_toc_fuzzy_match_count: translated_toc_metrics.fuzzy_match_count,
        translated_toc_exact_structure_match_count: translated_toc_metrics
            .exact_structure_match_count,
        translated_toc_title_backfill_rate: translated_toc_metrics.title_backfill_rate,
        chunks_with_confirmed_terms,
        confirmed_term_count,
        strict_glossary_entries,
        contextual_glossary_entries,
        proper_noun_hint_count: proper_noun_metrics.hint_count,
        targeted_proper_noun_hint_count: proper_noun_metrics.targeted_hint_count,
        algorithmic_proper_noun_hint_count: proper_noun_metrics.algorithmic_hint_count,
        strict_proper_noun_hint_count: proper_noun_metrics.strict_hint_count,
        contextual_proper_noun_hint_count: proper_noun_metrics.contextual_hint_count,
        missing_proper_noun_target_count: proper_noun_metrics.missing_target_count,
        duplicate_proper_noun_source_count: proper_noun_metrics.duplicate_source_count,
        proper_noun_hint_occurrence_total: proper_noun_metrics.occurrence_total,
        proper_noun_hint_indexed_chunk_total: proper_noun_metrics.indexed_chunk_total,
        proper_noun_hint_max_indexed_chunk_count: proper_noun_metrics.max_indexed_chunk_count,
        multi_translation_confirmed_term_count: proper_noun_metrics.multi_translation_term_count,
        validation_issue_count: validation_report.summary.issue_count,
        residual_english_issue_count,
        residual_review_candidate_chunk_count: residual_review_stats.candidate_chunk_count,
        residual_review_candidate_term_count: residual_review_stats.candidate_term_count,
        residual_review_candidate_limit: residual_review_stats.candidate_limit,
        residual_review_hit_candidate_limit: residual_review_stats.hit_candidate_limit,
        residual_review_stats_source: residual_review_stats.source.to_string(),
        residual_second_check_candidate_chunk_count: event_metrics
            .residual_second_check_candidate_chunk_count,
        residual_second_check_candidate_term_count: event_metrics
            .residual_second_check_candidate_term_count,
        residual_second_check_hit_candidate_limit: event_metrics
            .residual_second_check_hit_candidate_limit,
        residual_llm_attempt_started_event_count: event_metrics
            .residual_llm_attempt_started_event_count,
        residual_llm_attempt_completed_event_count: event_metrics
            .residual_llm_attempt_completed_event_count,
        residual_llm_attempt_failed_event_count: event_metrics
            .residual_llm_attempt_failed_event_count,
        residual_llm_completed_elapsed_ms_total: event_metrics
            .residual_llm_completed_elapsed_ms_total,
        residual_llm_slowest_elapsed_ms: event_metrics.residual_llm_slowest_elapsed_ms,
        strict_consistency_issue_count,
        checkpoint_write_count: checkpoint.write_metrics.write_count,
        checkpoint_write_total_ms: checkpoint.write_metrics.total_write_ms,
        metrics_build_stage_elapsed_ms,
    })
}

fn record_metrics_build_stage(
    target: &mut BTreeMap<String, u128>,
    stage: &str,
    started: StdInstant,
) {
    target.insert(stage.to_string(), started.elapsed().as_millis());
}

fn estimate_deepseek_v4_flash_cost_usd(usage: LlmUsageTotals) -> Option<f64> {
    if usage.response_count == 0 {
        return None;
    }
    const CACHED_INPUT_USD_PER_M: f64 = 0.028;
    const UNCACHED_INPUT_USD_PER_M: f64 = 0.14;
    const OUTPUT_USD_PER_M: f64 = 0.28;
    let cost = (usage.cached_prompt_tokens as f64 / 1_000_000.0) * CACHED_INPUT_USD_PER_M
        + (usage.uncached_prompt_tokens as f64 / 1_000_000.0) * UNCACHED_INPUT_USD_PER_M
        + (usage.completion_tokens as f64 / 1_000_000.0) * OUTPUT_USD_PER_M;
    Some((cost * 1_000_000.0).round() / 1_000_000.0)
}

struct ResidualStatsForMetrics {
    candidate_chunk_count: usize,
    candidate_term_count: usize,
    candidate_limit: usize,
    hit_candidate_limit: bool,
    source: &'static str,
}

fn select_residual_review_stats(
    checkpoint: &ChunkCheckpoint,
    event_metrics: &EventLogMetrics,
) -> ResidualStatsForMetrics {
    if event_metrics.residual_second_check_seen {
        return ResidualStatsForMetrics {
            candidate_chunk_count: event_metrics.residual_second_check_candidate_chunk_count,
            candidate_term_count: event_metrics.residual_second_check_candidate_term_count,
            candidate_limit: event_metrics.residual_second_check_candidate_limit,
            hit_candidate_limit: event_metrics.residual_second_check_hit_candidate_limit,
            source: "residual_second_check_event",
        };
    }
    let stats = crate::pipeline::review::collect_residual_review_stats(checkpoint);
    ResidualStatsForMetrics {
        candidate_chunk_count: stats.candidate_chunk_count,
        candidate_term_count: stats.candidate_term_count,
        candidate_limit: stats.candidate_limit,
        hit_candidate_limit: stats.hit_candidate_limit,
        source: "full_residual_scan",
    }
}

#[derive(Debug, Default)]
struct ChunkStageMetrics {
    body_chunk_count: usize,
    reference_chunk_count: usize,
    protected_metadata_chunk_count: usize,
    pdf_table_chunk_count: usize,
    body_pending_chunk_count: usize,
    body_translated_stage_chunk_count: usize,
    body_term_consistency_stage_chunk_count: usize,
    body_residual_reviewed_stage_chunk_count: usize,
    body_merge_ready_stage_chunk_count: usize,
    body_translation_failed_chunk_count: usize,
    protected_passthrough_chunk_count: usize,
}

#[derive(Debug, Default)]
struct EventLogMetrics {
    event_count: usize,
    chunk_translated_event_count: usize,
    chunk_structure_check_event_count: usize,
    chunk_structure_mismatch_count: usize,
    chunk_structure_block_delta_total: usize,
    chunk_structure_heading_mismatch_count: usize,
    residual_repair_event_count: usize,
    residual_review_skipped_event_count: usize,
    ocr_attempt_event_count: usize,
    pipeline_stage_event_count: usize,
    pipeline_stage_elapsed_ms: BTreeMap<String, u128>,
    slowest_pipeline_stage: Option<String>,
    slowest_pipeline_stage_elapsed_ms: u128,
    current_run_started_at: Option<String>,
    current_run_event_count: usize,
    current_run_chunk_translated_event_count: usize,
    current_run_pipeline_stage_event_count: usize,
    current_run_pipeline_stage_elapsed_ms: BTreeMap<String, u128>,
    current_run_slowest_pipeline_stage: Option<String>,
    current_run_slowest_pipeline_stage_elapsed_ms: u128,
    current_run_translation_event_llm_elapsed_ms_total: u128,
    current_run_translation_event_wait_ms_total: u128,
    current_run_translation_event_route_wait_ms_total: u128,
    current_run_translation_event_llm_attempts_total: usize,
    resume_audit_event_count: usize,
    resume_audit_warning_event_count: usize,
    translation_event_llm_elapsed_ms_total: u128,
    translation_event_wait_ms_total: u128,
    translation_event_route_wait_ms_total: u128,
    translation_event_llm_attempts_total: usize,
    long_tail_chunk_count: usize,
    slowest_chunk_index: Option<usize>,
    slowest_chunk_elapsed_ms: u128,
    slowest_chunk_llm_elapsed_ms: u128,
    chunk_elapsed_ms: Vec<u128>,
    llm_attempt_failed_event_count: usize,
    llm_attempt_failed_request_elapsed_ms_total: u128,
    llm_attempt_failed_delay_ms_total: u128,
    wave_term_delta_event_count: usize,
    wave_term_delta_total: usize,
    wave_term_match_total: usize,
    wave_term_deduped_total: usize,
    wave_term_applied_total: usize,
    wave_term_already_correct_total: usize,
    wave_term_llm_patch_total: usize,
    wave_term_llm_correct_total: usize,
    wave_term_fallback_patch_total: usize,
    wave_term_patched_chunk_total: usize,
    residual_second_check_seen: bool,
    residual_second_check_candidate_chunk_count: usize,
    residual_second_check_candidate_term_count: usize,
    residual_second_check_candidate_limit: usize,
    residual_second_check_hit_candidate_limit: bool,
    residual_llm_attempt_started_event_count: usize,
    residual_llm_attempt_completed_event_count: usize,
    residual_llm_attempt_failed_event_count: usize,
    residual_llm_completed_elapsed_ms_total: u128,
    residual_llm_slowest_elapsed_ms: u128,
}

impl EventLogMetrics {
    fn chunk_structure_error_rate(&self) -> f32 {
        if self.chunk_structure_check_event_count == 0 {
            0.0
        } else {
            self.chunk_structure_mismatch_count as f32
                / self.chunk_structure_check_event_count as f32
        }
    }

    fn chunk_elapsed_p95_ms(&self) -> u128 {
        if self.chunk_elapsed_ms.is_empty() {
            return 0;
        }
        let mut values = self.chunk_elapsed_ms.clone();
        values.sort_unstable();
        let index = ((values.len() as f64) * 0.95).ceil() as usize;
        values[index.saturating_sub(1).min(values.len() - 1)]
    }
}

fn collect_chunk_stage_metrics(checkpoint: &ChunkCheckpoint) -> ChunkStageMetrics {
    let mut metrics = ChunkStageMetrics::default();

    for chunk in &checkpoint.chunks {
        match chunk.segment_kind {
            ChunkSegmentKind::Body => {
                metrics.body_chunk_count += 1;
                match crate::pipeline::checkpoint::current_chunk_processing_stage(chunk) {
                    ChunkProcessingStage::Pending => metrics.body_pending_chunk_count += 1,
                    ChunkProcessingStage::Translated => {
                        metrics.body_translated_stage_chunk_count += 1
                    }
                    ChunkProcessingStage::TermConsistencyApplied => {
                        metrics.body_term_consistency_stage_chunk_count += 1
                    }
                    ChunkProcessingStage::ResidualReviewed => {
                        metrics.body_residual_reviewed_stage_chunk_count += 1
                    }
                    ChunkProcessingStage::MergeReady => {
                        metrics.body_merge_ready_stage_chunk_count += 1
                    }
                    ChunkProcessingStage::TranslationFailed => {
                        metrics.body_translation_failed_chunk_count += 1
                    }
                    ChunkProcessingStage::ProtectedPassthrough => {}
                }
            }
            ChunkSegmentKind::Reference => {
                metrics.reference_chunk_count += 1;
                if crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                    == ChunkProcessingStage::ProtectedPassthrough
                {
                    metrics.protected_passthrough_chunk_count += 1;
                }
            }
            ChunkSegmentKind::ProtectedMetadata => {
                metrics.protected_metadata_chunk_count += 1;
                if crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                    == ChunkProcessingStage::ProtectedPassthrough
                {
                    metrics.protected_passthrough_chunk_count += 1;
                }
            }
            ChunkSegmentKind::PdfTable => {
                metrics.pdf_table_chunk_count += 1;
                if crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                    == ChunkProcessingStage::ProtectedPassthrough
                {
                    metrics.protected_passthrough_chunk_count += 1;
                }
            }
        }
    }

    metrics
}

fn collect_event_log_metrics(event_log_path: Option<&Path>) -> Result<EventLogMetrics, AppError> {
    let Some(event_log_path) = event_log_path else {
        return Ok(EventLogMetrics::default());
    };
    let events = super::read_json_lines::<TaskEvent>(event_log_path)?;
    let current_run_start_index = events
        .iter()
        .rposition(|event| event.stage == "queued")
        .unwrap_or_default();
    let mut metrics = EventLogMetrics {
        event_count: events.len(),
        current_run_started_at: events
            .get(current_run_start_index)
            .filter(|event| event.stage == "queued")
            .map(|event| event.timestamp.clone()),
        ..Default::default()
    };

    for (event_index, event) in events.into_iter().enumerate() {
        let in_current_run = event_index >= current_run_start_index;
        if in_current_run {
            metrics.current_run_event_count += 1;
        }
        match event.stage.as_str() {
            "chunk_translated" => {
                metrics.chunk_translated_event_count += 1;
                let elapsed_ms = parse_metric_from_message(&event.message, "elapsed_ms");
                let llm_elapsed_ms = parse_metric_from_message(&event.message, "llm_elapsed_ms");
                metrics.chunk_elapsed_ms.push(elapsed_ms);
                if elapsed_ms >= LONG_TAIL_CHUNK_THRESHOLD_MS {
                    metrics.long_tail_chunk_count += 1;
                }
                if elapsed_ms > metrics.slowest_chunk_elapsed_ms {
                    metrics.slowest_chunk_elapsed_ms = elapsed_ms;
                    metrics.slowest_chunk_llm_elapsed_ms = llm_elapsed_ms;
                    metrics.slowest_chunk_index =
                        Some(parse_metric_from_message(&event.message, "index") as usize);
                }
                metrics.translation_event_llm_elapsed_ms_total = metrics
                    .translation_event_llm_elapsed_ms_total
                    .saturating_add(llm_elapsed_ms);
                metrics.translation_event_wait_ms_total = metrics
                    .translation_event_wait_ms_total
                    .saturating_add(parse_metric_from_message(&event.message, "wait_ms"));
                metrics.translation_event_route_wait_ms_total = metrics
                    .translation_event_route_wait_ms_total
                    .saturating_add(parse_metric_from_message(&event.message, "route_wait_ms"));
                metrics.translation_event_llm_attempts_total =
                    metrics.translation_event_llm_attempts_total.saturating_add(
                        parse_metric_from_message(&event.message, "llm_attempts") as usize,
                    );
                if in_current_run {
                    metrics.current_run_chunk_translated_event_count += 1;
                    metrics.current_run_translation_event_llm_elapsed_ms_total = metrics
                        .current_run_translation_event_llm_elapsed_ms_total
                        .saturating_add(llm_elapsed_ms);
                    metrics.current_run_translation_event_wait_ms_total = metrics
                        .current_run_translation_event_wait_ms_total
                        .saturating_add(parse_metric_from_message(&event.message, "wait_ms"));
                    metrics.current_run_translation_event_route_wait_ms_total = metrics
                        .current_run_translation_event_route_wait_ms_total
                        .saturating_add(parse_metric_from_message(&event.message, "route_wait_ms"));
                    metrics.current_run_translation_event_llm_attempts_total = metrics
                        .current_run_translation_event_llm_attempts_total
                        .saturating_add(
                            parse_metric_from_message(&event.message, "llm_attempts") as usize
                        );
                }
            }
            "chunk_structure_check" => {
                metrics.chunk_structure_check_event_count += 1;
                metrics.chunk_structure_block_delta_total =
                    metrics.chunk_structure_block_delta_total.saturating_add(
                        parse_metric_from_message(&event.message, "block_delta") as usize,
                    );
                if parse_bool_metric_from_message(&event.message, "heading_mismatch") {
                    metrics.chunk_structure_heading_mismatch_count += 1;
                }
                if parse_bool_metric_from_message(&event.message, "mismatch") {
                    metrics.chunk_structure_mismatch_count += 1;
                }
            }
            "validation_repaired" => metrics.residual_repair_event_count += 1,
            "validation_review_skipped" => metrics.residual_review_skipped_event_count += 1,
            "residual_llm_attempt_started" => {
                metrics.residual_llm_attempt_started_event_count += 1;
            }
            "residual_llm_attempt_completed" => {
                metrics.residual_llm_attempt_completed_event_count += 1;
                let elapsed = parse_metric_from_message(&event.message, "request_elapsed_ms");
                metrics.residual_llm_completed_elapsed_ms_total = metrics
                    .residual_llm_completed_elapsed_ms_total
                    .saturating_add(elapsed);
                if elapsed > metrics.residual_llm_slowest_elapsed_ms {
                    metrics.residual_llm_slowest_elapsed_ms = elapsed;
                }
            }
            "residual_llm_attempt_failed" => {
                metrics.residual_llm_attempt_failed_event_count += 1;
            }
            "ocr_attempt" => metrics.ocr_attempt_event_count += 1,
            "pipeline_stage_timing" => {
                record_pipeline_stage_timing(&event.message, &mut metrics);
                if in_current_run {
                    record_current_run_pipeline_stage_timing(&event.message, &mut metrics);
                }
            }
            "resume_audit" => metrics.resume_audit_event_count += 1,
            "resume_audit_warning" => {
                metrics.resume_audit_warning_event_count += 1;
                metrics.resume_audit_event_count += 1;
            }
            "llm_attempt_failed" => {
                metrics.llm_attempt_failed_event_count += 1;
                metrics.llm_attempt_failed_request_elapsed_ms_total = metrics
                    .llm_attempt_failed_request_elapsed_ms_total
                    .saturating_add(parse_metric_from_message(
                        &event.message,
                        "request_elapsed_ms",
                    ));
                metrics.llm_attempt_failed_delay_ms_total = metrics
                    .llm_attempt_failed_delay_ms_total
                    .saturating_add(parse_metric_from_message(&event.message, "delay_ms"));
            }
            "wave_term_delta" => {
                metrics.wave_term_delta_event_count += 1;
                metrics.wave_term_delta_total =
                    metrics
                        .wave_term_delta_total
                        .saturating_add(
                            parse_metric_from_message(&event.message, "new_term_count") as usize,
                        );
                metrics.wave_term_match_total =
                    metrics
                        .wave_term_match_total
                        .saturating_add(
                            parse_metric_from_message(&event.message, "matched_terms") as usize
                        );
                metrics.wave_term_deduped_total =
                    metrics
                        .wave_term_deduped_total
                        .saturating_add(
                            parse_metric_from_message(&event.message, "deduped_terms") as usize
                        );
                metrics.wave_term_applied_total =
                    metrics
                        .wave_term_applied_total
                        .saturating_add(
                            parse_metric_from_message(&event.message, "applied_terms") as usize
                        );
                metrics.wave_term_already_correct_total =
                    metrics.wave_term_already_correct_total.saturating_add(
                        parse_metric_from_message(&event.message, "already_correct_terms") as usize,
                    );
                metrics.wave_term_llm_patch_total = metrics
                    .wave_term_llm_patch_total
                    .saturating_add(
                        parse_metric_from_message(&event.message, "llm_patch_count") as usize
                    );
                metrics.wave_term_llm_correct_total = metrics
                    .wave_term_llm_correct_total
                    .saturating_add(
                        parse_metric_from_message(&event.message, "llm_correct_count") as usize,
                    );
                metrics.wave_term_fallback_patch_total =
                    metrics.wave_term_fallback_patch_total.saturating_add(
                        parse_metric_from_message(&event.message, "fallback_patch_count") as usize,
                    );
                metrics.wave_term_patched_chunk_total =
                    metrics.wave_term_patched_chunk_total.saturating_add(
                        parse_metric_from_message(&event.message, "patched_chunks") as usize,
                    );
            }
            "validation_review_second_check" => {
                metrics.residual_second_check_seen = true;
                metrics.residual_second_check_candidate_chunk_count =
                    parse_metric_from_message(&event.message, "remaining_candidate_chunks")
                        as usize;
                metrics.residual_second_check_candidate_term_count =
                    parse_metric_from_message(&event.message, "remaining_candidate_terms") as usize;
                metrics.residual_second_check_candidate_limit =
                    parse_metric_from_message(&event.message, "candidate_limit") as usize;
                metrics.residual_second_check_hit_candidate_limit =
                    parse_bool_metric_from_message(&event.message, "hit_candidate_limit");
            }
            _ => {}
        }
    }

    Ok(metrics)
}

fn record_pipeline_stage_timing(message: &str, metrics: &mut EventLogMetrics) {
    let Some(stage_name) = parse_metric_value_from_message(message, "stage")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let elapsed_ms = parse_metric_from_message(message, "elapsed_ms");
    metrics.pipeline_stage_event_count += 1;
    let total = metrics
        .pipeline_stage_elapsed_ms
        .entry(stage_name.to_string())
        .or_default();
    *total = total.saturating_add(elapsed_ms);
    if *total > metrics.slowest_pipeline_stage_elapsed_ms {
        metrics.slowest_pipeline_stage_elapsed_ms = *total;
        metrics.slowest_pipeline_stage = Some(stage_name.to_string());
    }
}

fn record_current_run_pipeline_stage_timing(message: &str, metrics: &mut EventLogMetrics) {
    let Some(stage_name) = parse_metric_value_from_message(message, "stage")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let elapsed_ms = parse_metric_from_message(message, "elapsed_ms");
    metrics.current_run_pipeline_stage_event_count += 1;
    let total = metrics
        .current_run_pipeline_stage_elapsed_ms
        .entry(stage_name.to_string())
        .or_default();
    *total = total.saturating_add(elapsed_ms);
    if *total > metrics.current_run_slowest_pipeline_stage_elapsed_ms {
        metrics.current_run_slowest_pipeline_stage_elapsed_ms = *total;
        metrics.current_run_slowest_pipeline_stage = Some(stage_name.to_string());
    }
}

fn parse_bool_metric_from_message(message: &str, field: &str) -> bool {
    parse_metric_value_from_message(message, field)
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn parse_metric_from_message(message: &str, field: &str) -> u128 {
    parse_metric_value_from_message(message, field)
        .unwrap_or_default()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<u128>()
        .unwrap_or(0)
}

fn parse_metric_value_from_message<'a>(message: &'a str, field: &str) -> Option<&'a str> {
    message.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key.trim() == field).then_some(value.trim())
    })
}

#[derive(Debug, Default)]
struct TranslatedTocMetrics {
    missing_title_count: usize,
    structural_mismatch_count: usize,
    locator_mutation_count: usize,
    matched_title_count: usize,
    unmatched_title_count: usize,
    fuzzy_match_count: usize,
    exact_structure_match_count: usize,
    title_backfill_rate: f32,
}

fn collect_translated_toc_metrics(
    source_toc: Option<&crate::document::TocArtifact>,
    translated_toc: Option<&crate::document::TocArtifact>,
    source_markdown: Option<&str>,
    translated_markdown: Option<&str>,
) -> TranslatedTocMetrics {
    let Some(translated_toc) = translated_toc else {
        return TranslatedTocMetrics::default();
    };
    let missing_title_count = translated_toc
        .entries
        .iter()
        .filter(|entry| entry.translated_title.as_deref().is_none_or(str::is_empty))
        .count();
    let matched_title_count = translated_toc.entries.len() - missing_title_count;
    let mut metrics = TranslatedTocMetrics {
        missing_title_count,
        matched_title_count,
        unmatched_title_count: missing_title_count,
        title_backfill_rate: title_backfill_rate(matched_title_count, translated_toc.entries.len()),
        exact_structure_match_count: match (source_markdown, translated_markdown) {
            (Some(source), Some(translated)) => {
                count_exact_heading_structure_matches(source, translated)
            }
            _ => 0,
        },
        ..Default::default()
    };
    let Some(source_toc) = source_toc else {
        metrics.structural_mismatch_count = translated_toc.entries.len();
        metrics.locator_mutation_count = translated_toc.entries.len();
        return metrics;
    };
    if source_toc.entries.len() != translated_toc.entries.len() {
        metrics.structural_mismatch_count += source_toc
            .entries
            .len()
            .abs_diff(translated_toc.entries.len());
    }
    for (source, translated) in source_toc.entries.iter().zip(translated_toc.entries.iter()) {
        if source.level != translated.level
            || source.order != translated.order
            || source.source_kind != translated.source_kind
        {
            metrics.structural_mismatch_count += 1;
        }
        if source.page != translated.page || source.href != translated.href {
            metrics.locator_mutation_count += 1;
        }
    }
    metrics.fuzzy_match_count = match (source_markdown, translated_markdown) {
        (Some(source), Some(translated)) => {
            count_numbering_normalized_toc_matches(source_toc, source, translated)
        }
        _ => 0,
    };
    metrics
}

fn title_backfill_rate(matched: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        matched as f32 / total as f32
    }
}

fn toc_source_kind_label(kind: crate::document::TocSourceKind) -> &'static str {
    match kind {
        crate::document::TocSourceKind::EpubNcx => "epubNcx",
        crate::document::TocSourceKind::EpubNav => "epubNav",
        crate::document::TocSourceKind::EpubHtmlTitle => "epubHtmlTitle",
        crate::document::TocSourceKind::EpubSpineFallback => "epubSpineFallback",
        crate::document::TocSourceKind::PdfOutline => "pdfOutline",
        crate::document::TocSourceKind::HeadingInferred => "headingInferred",
        crate::document::TocSourceKind::LayoutInferred => "layoutInferred",
        crate::document::TocSourceKind::None => "none",
    }
}

#[derive(Debug, Default)]
struct TocQualityMetrics {
    low_confidence_entry_count: usize,
    duplicate_title_count: usize,
    missing_locator_count: usize,
}

fn collect_toc_quality_metrics(toc: Option<&crate::document::TocArtifact>) -> TocQualityMetrics {
    let Some(toc) = toc else {
        return TocQualityMetrics::default();
    };
    let mut metrics = TocQualityMetrics::default();
    let mut seen_titles = BTreeSet::<String>::new();
    let mut duplicate_titles = BTreeSet::<String>::new();
    for entry in &toc.entries {
        if entry.confidence < 0.6 {
            metrics.low_confidence_entry_count += 1;
        }
        if entry.page.is_none() && entry.href.as_deref().is_none_or(str::is_empty) {
            metrics.missing_locator_count += 1;
        }
        let title_key = entry
            .source_title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let title_key = title_key.to_ascii_lowercase();
        if !title_key.is_empty() && !seen_titles.insert(title_key.clone()) {
            duplicate_titles.insert(title_key);
        }
    }
    metrics.duplicate_title_count = duplicate_titles.len();
    metrics
}

#[derive(Debug, Default)]
struct ProperNounMetrics {
    hint_count: usize,
    targeted_hint_count: usize,
    algorithmic_hint_count: usize,
    strict_hint_count: usize,
    contextual_hint_count: usize,
    missing_target_count: usize,
    duplicate_source_count: usize,
    occurrence_total: usize,
    indexed_chunk_total: usize,
    max_indexed_chunk_count: usize,
    multi_translation_term_count: usize,
}

fn collect_proper_noun_metrics(checkpoint: &ChunkCheckpoint) -> ProperNounMetrics {
    let mut metrics = ProperNounMetrics::default();
    if let Some(context) = checkpoint.translation_context.as_ref() {
        let mut seen_sources = BTreeSet::<String>::new();
        let mut duplicate_sources = BTreeSet::<String>::new();
        for hint in &context.proper_nouns {
            metrics.hint_count += 1;
            match hint.enforcement {
                ProperNounEnforcement::Strict => metrics.strict_hint_count += 1,
                ProperNounEnforcement::Contextual => metrics.contextual_hint_count += 1,
            }
            if hint.target.as_deref().is_none_or(str::is_empty) {
                metrics.missing_target_count += 1;
                metrics.algorithmic_hint_count += 1;
            } else {
                metrics.targeted_hint_count += 1;
            }
            metrics.occurrence_total = metrics.occurrence_total.saturating_add(hint.occurrences);
            metrics.indexed_chunk_total = metrics
                .indexed_chunk_total
                .saturating_add(hint.chunk_indexes.len());
            metrics.max_indexed_chunk_count = metrics
                .max_indexed_chunk_count
                .max(hint.chunk_indexes.len());
            let source_key = hint.source.trim().to_ascii_lowercase();
            if !seen_sources.insert(source_key.clone()) {
                duplicate_sources.insert(source_key);
            }
        }
        metrics.duplicate_source_count = duplicate_sources.len();
    }
    metrics.multi_translation_term_count = count_multi_translation_confirmed_terms(checkpoint);
    metrics
}

fn count_multi_translation_confirmed_terms(checkpoint: &ChunkCheckpoint) -> usize {
    let mut translations_by_source = BTreeMap::<String, BTreeSet<String>>::new();
    for chunk in &checkpoint.chunks {
        for term in &chunk.confirmed_terms {
            let source = term.source.trim();
            let translation = term.translation.trim();
            if source.is_empty() || translation.is_empty() {
                continue;
            }
            translations_by_source
                .entry(source.to_ascii_lowercase())
                .or_default()
                .insert(translation.to_string());
        }
    }
    translations_by_source
        .values()
        .filter(|translations| translations.len() > 1)
        .count()
}

fn checkpoint_elapsed_ms(checkpoint: &ChunkCheckpoint) -> Result<Option<u128>, AppError> {
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(&checkpoint.created_at) else {
        return Ok(None);
    };
    let Ok(updated) = chrono::DateTime::parse_from_rfc3339(&checkpoint.updated_at) else {
        return Ok(None);
    };
    let elapsed = updated.signed_duration_since(created).num_milliseconds();
    Ok((elapsed >= 0).then_some(elapsed as u128))
}
