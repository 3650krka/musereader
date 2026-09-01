use super::types::{ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind, ChunkState};
use chrono::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResumeAudit {
    body_pending_count: usize,
    body_translated_count: usize,
    body_term_consistency_count: usize,
    body_residual_reviewed_count: usize,
    body_merge_ready_count: usize,
    missing_translation_after_pending_count: usize,
    translated_pending_count: usize,
    non_body_unprotected_count: usize,
    frozen_index_stale: bool,
}

impl ResumeAudit {
    pub(super) fn to_event_message(&self) -> String {
        format!(
            "body_pending={}; body_translated={}; body_term_consistency={}; body_residual_reviewed={}; body_merge_ready={}; missing_translation_after_pending={}; translated_pending={}; non_body_unprotected={}; frozen_index_stale={}",
            self.body_pending_count,
            self.body_translated_count,
            self.body_term_consistency_count,
            self.body_residual_reviewed_count,
            self.body_merge_ready_count,
            self.missing_translation_after_pending_count,
            self.translated_pending_count,
            self.non_body_unprotected_count,
            self.frozen_index_stale
        )
    }

    pub(super) fn has_inconsistency(&self) -> bool {
        self.missing_translation_after_pending_count > 0
            || self.translated_pending_count > 0
            || self.non_body_unprotected_count > 0
            || self.frozen_index_stale
    }
}

pub(super) fn audit_checkpoint_resume_state(checkpoint: &ChunkCheckpoint) -> ResumeAudit {
    let mut audit = ResumeAudit {
        frozen_index_stale: checkpoint.delta_sync.frozen_at_glossary_seq
            != Some(checkpoint.delta_sync.glossary_seq)
            || checkpoint.delta_sync.frozen_patch_index_version.is_none(),
        ..ResumeAudit::default()
    };

    for chunk in &checkpoint.chunks {
        audit_chunk_resume_state(chunk, &mut audit);
    }

    audit
}

impl Default for ResumeAudit {
    fn default() -> Self {
        Self {
            body_pending_count: 0,
            body_translated_count: 0,
            body_term_consistency_count: 0,
            body_residual_reviewed_count: 0,
            body_merge_ready_count: 0,
            missing_translation_after_pending_count: 0,
            translated_pending_count: 0,
            non_body_unprotected_count: 0,
            frozen_index_stale: false,
        }
    }
}

fn audit_chunk_resume_state(chunk: &ChunkState, audit: &mut ResumeAudit) {
    let stage = crate::pipeline::checkpoint::current_chunk_processing_stage(chunk);
    if chunk.segment_kind == ChunkSegmentKind::Body {
        audit_body_chunk_resume_state(chunk, stage, audit);
        return;
    }
    if chunk.translated.as_deref() != Some(chunk.source.as_str())
        && stage != ChunkProcessingStage::Pending
    {
        audit.non_body_unprotected_count += 1;
    }
}

fn audit_body_chunk_resume_state(
    chunk: &ChunkState,
    stage: ChunkProcessingStage,
    audit: &mut ResumeAudit,
) {
    match stage {
        ChunkProcessingStage::Pending => audit.body_pending_count += 1,
        ChunkProcessingStage::Translated => audit.body_translated_count += 1,
        ChunkProcessingStage::TermConsistencyApplied => audit.body_term_consistency_count += 1,
        ChunkProcessingStage::ResidualReviewed => audit.body_residual_reviewed_count += 1,
        ChunkProcessingStage::MergeReady => audit.body_merge_ready_count += 1,
        ChunkProcessingStage::TranslationFailed | ChunkProcessingStage::ProtectedPassthrough => {}
    }
    if stage != ChunkProcessingStage::Pending
        && stage != ChunkProcessingStage::TranslationFailed
        && chunk.translated.is_none()
    {
        audit.missing_translation_after_pending_count += 1;
    }
    if stage == ChunkProcessingStage::Pending && chunk.translated.is_some() {
        audit.translated_pending_count += 1;
    }
}

pub(super) fn collect_pending_indexes(checkpoint: &ChunkCheckpoint) -> Vec<usize> {
    checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.segment_kind == ChunkSegmentKind::Body)
        .filter(|chunk| {
            crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                == ChunkProcessingStage::Pending
        })
        .map(|chunk| chunk.index)
        .collect()
}

pub(super) fn compute_translate_progress(current: usize, total: usize) -> u8 {
    if total == 0 {
        return 40;
    }
    let translated = ((current * 50) / total) as u8;
    40u8.saturating_add(translated).min(90)
}

pub(super) fn needs_term_consistency(checkpoint: &ChunkCheckpoint) -> bool {
    checkpoint.chunks.iter().any(|chunk| {
        chunk.segment_kind == ChunkSegmentKind::Body
            && chunk.translated.is_some()
            && crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                == ChunkProcessingStage::Translated
    })
}

pub(super) fn needs_residual_review(checkpoint: &ChunkCheckpoint) -> bool {
    checkpoint.chunks.iter().any(|chunk| {
        chunk.segment_kind == ChunkSegmentKind::Body
            && chunk.translated.is_some()
            && crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                == ChunkProcessingStage::TermConsistencyApplied
    })
}

pub(super) fn needs_merge_ready_advance(checkpoint: &ChunkCheckpoint) -> bool {
    checkpoint
        .chunks
        .iter()
        .any(|chunk| match chunk.segment_kind {
            ChunkSegmentKind::Body => {
                chunk.translated.is_some()
                    && crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                        == ChunkProcessingStage::ResidualReviewed
            }
            ChunkSegmentKind::Reference
            | ChunkSegmentKind::ProtectedMetadata
            | ChunkSegmentKind::PdfTable => {
                chunk.translated.as_deref() == Some(chunk.source.as_str())
                    && crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                        != ChunkProcessingStage::ProtectedPassthrough
            }
        })
}

pub(super) fn merge_translated_chunks(
    checkpoint: &ChunkCheckpoint,
) -> Result<String, crate::error::AppError> {
    let mut ordered = checkpoint.chunks.clone();
    ordered.sort_by_key(|chunk| chunk.index);

    let mut sections = Vec::with_capacity(ordered.len());
    for chunk in ordered {
        if chunk.segment_kind == ChunkSegmentKind::Body {
            let stage = crate::pipeline::checkpoint::current_chunk_processing_stage(&chunk);
            // 块级失败隔离：重试耗尽的块直接保留原文参与合并，
            // 产物可交付，失败清单随 PipelineResult 上报。
            if stage == ChunkProcessingStage::TranslationFailed {
                sections.push(chunk.source.clone());
                continue;
            }
            ensure_body_chunk_merge_ready(&chunk)?;
        }
        let translated = chunk.translated.ok_or_else(|| {
            crate::error::AppError::internal(format!(
                "missing translated chunk while merging; chunk={}",
                chunk.index
            ))
        })?;
        if chunk.segment_kind == ChunkSegmentKind::Body {
            crate::pipeline::checkpoint::reject_mojibake_translation(chunk.index, &translated)?;
        }
        sections.push(translated);
    }
    Ok(crate::translation_validation::sanitize_translated_markdown(
        &sections.join("\n\n"),
    ))
}

/// 块级失败隔离汇总：返回重试耗尽的正文块 index（按块序升序）。
pub(super) fn failed_body_chunk_indexes(checkpoint: &ChunkCheckpoint) -> Vec<usize> {
    checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.segment_kind == ChunkSegmentKind::Body)
        .filter(|chunk| {
            crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                == ChunkProcessingStage::TranslationFailed
        })
        .map(|chunk| chunk.index)
        .collect()
}

pub(super) fn mark_checkpoint_merge_ready(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut advanced = 0usize;
    for chunk in &mut checkpoint.chunks {
        match chunk.segment_kind {
            ChunkSegmentKind::Body
                if chunk.translated.is_some()
                    && crate::pipeline::checkpoint::current_chunk_processing_stage(chunk)
                        == ChunkProcessingStage::ResidualReviewed =>
            {
                if crate::pipeline::checkpoint::advance_chunk_processing_stage(
                    chunk,
                    ChunkProcessingStage::MergeReady,
                ) {
                    advanced += 1;
                }
            }
            ChunkSegmentKind::Reference
            | ChunkSegmentKind::ProtectedMetadata
            | ChunkSegmentKind::PdfTable => {
                if chunk.translated.as_deref() == Some(chunk.source.as_str())
                    && crate::pipeline::checkpoint::advance_chunk_processing_stage(
                        chunk,
                        ChunkProcessingStage::ProtectedPassthrough,
                    )
                {
                    advanced += 1;
                }
            }
            _ => {}
        }
    }
    if advanced > 0 {
        checkpoint.updated_at = Utc::now().to_rfc3339();
    }
    advanced
}

fn ensure_body_chunk_merge_ready(chunk: &ChunkState) -> Result<(), crate::error::AppError> {
    let stage = crate::pipeline::checkpoint::current_chunk_processing_stage(chunk);
    if stage == ChunkProcessingStage::MergeReady {
        return Ok(());
    }
    Err(crate::error::AppError::internal(format!(
        "body chunk is not merge-ready; chunk={}; stage={stage:?}",
        chunk.index
    )))
}

pub(super) fn repair_body_chunks_with_final_context(checkpoint: &mut ChunkCheckpoint) -> usize {
    let Some(context) = checkpoint.translation_context.clone() else {
        return 0;
    };
    let mut repaired_chunks = 0usize;

    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(current_text) = chunk.translated.as_ref() else {
            continue;
        };
        let strict_hits =
            crate::pipeline::context::collect_strict_chunk_hits(&context, &chunk.source);
        if strict_hits.is_empty() {
            continue;
        }

        let mut repaired_text = crate::pipeline::context::repair_strict_hit_aliases(
            current_text,
            &chunk.confirmed_terms,
            &strict_hits,
        );
        for hit in &strict_hits {
            if repaired_text.contains(&hit.source) {
                repaired_text = repaired_text.replace(&hit.source, &hit.target);
            }
        }

        let mut repaired_terms = chunk.confirmed_terms.clone();
        crate::pipeline::context::normalize_strict_hit_confirmed_terms(
            &strict_hits,
            &mut repaired_terms,
        );
        crate::pipeline::context::synthesize_missing_strict_hit_terms(
            &strict_hits,
            &repaired_text,
            &mut repaired_terms,
        );

        if repaired_text != *current_text || repaired_terms != chunk.confirmed_terms {
            chunk.translated = Some(repaired_text);
            chunk.confirmed_terms = repaired_terms;
            chunk.updated_at = Utc::now().to_rfc3339();
            repaired_chunks += 1;
        }
    }

    if repaired_chunks > 0 {
        checkpoint.updated_at = Utc::now().to_rfc3339();
    }
    repaired_chunks
}
