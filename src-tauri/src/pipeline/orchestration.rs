use super::*;

pub(super) fn build_runtime_prompt(config: &PipelineRunConfig) -> RuntimePromptConfig {
    RuntimePromptConfig {
        system_prompt: config.system_prompt.clone(),
        system_prompt_hash: config.system_prompt_hash.clone(),
        article_type: config.article_type.clone(),
        skill_ids: config.skill_ids.clone(),
    }
}

pub(super) fn emit_prompt_ready<F>(
    pipeline: &TranslationPipeline,
    paths: &ArtifactPaths,
    runtime_prompt: &RuntimePromptConfig,
    emit_progress: &F,
) -> Result<(), AppError>
where
    F: Fn(PipelineProgressStage, &str, u8),
{
    let message = format!(
        "runtime prompt ready; article_type={}; chars={}",
        runtime_prompt.article_type,
        runtime_prompt.system_prompt.chars().count()
    );
    pipeline.record_event(&paths.event_log_path, "prompt_ready", &message, 28)?;
    emit_progress(PipelineProgressStage::Imported, &message, 28);
    Ok(())
}

pub(super) fn load_checkpoint_with_events<F>(
    pipeline: &TranslationPipeline,
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    source_markdown: &str,
    source_hash: &str,
    progress_callback: &F,
) -> Result<ChunkCheckpoint, AppError>
where
    F: Fn(PipelineProgressStage, &str, u8),
{
    let outcome =
        load_checkpoint_state(config, runtime_prompt, paths, source_markdown, source_hash)?;
    match &outcome.status {
        CheckpointLoadStatus::Resumed {
            invalidated_chunks,
            resumed_translated_chunks,
            total_chunks,
            source_hash_refreshed,
        } => {
            if invalidated_chunks > &0 {
                pipeline.record_event(
                    &paths.event_log_path,
                    "checkpoint_pollution_rebuild",
                    &format!("invalidated {invalidated_chunks} polluted chunks before resume"),
                    31,
                )?;
            }
            let message = format!(
                "loaded resumable checkpoint; translated={resumed_translated_chunks}/{total_chunks}; source_hash_refreshed={source_hash_refreshed}"
            );
            pipeline.record_event(&paths.event_log_path, "resume_ready", &message, 32)?;
            progress_callback(PipelineProgressStage::Chunking, &message, 32);
        }
        CheckpointLoadStatus::Created => {
            let message = format!(
                "initialized checkpoint with {} chunks",
                outcome.checkpoint.chunks.len()
            );
            pipeline.record_event(&paths.event_log_path, "checkpoint_created", &message, 32)?;
            progress_callback(PipelineProgressStage::Chunking, &message, 32);
        }
        CheckpointLoadStatus::RebuiltFromUnreadable => {
            pipeline.record_event(
                &paths.event_log_path,
                "checkpoint_rebuild",
                "checkpoint unreadable, rebuilding from source",
                31,
            )?;
            progress_callback(
                PipelineProgressStage::Chunking,
                "checkpoint unreadable, rebuilding from source",
                31,
            );
            let message = format!(
                "initialized checkpoint with {} chunks",
                outcome.checkpoint.chunks.len()
            );
            pipeline.record_event(&paths.event_log_path, "checkpoint_created", &message, 32)?;
            progress_callback(PipelineProgressStage::Chunking, &message, 32);
        }
    }
    let audit = crate::pipeline::runtime_types::audit_checkpoint_resume_state(&outcome.checkpoint);
    let audit_stage = if audit.has_inconsistency() {
        "resume_audit_warning"
    } else {
        "resume_audit"
    };
    pipeline.record_event(
        &paths.event_log_path,
        audit_stage,
        &audit.to_event_message(),
        33,
    )?;
    Ok(outcome.checkpoint)
}

pub(super) fn apply_term_consistency_with_events<F>(
    pipeline: &TranslationPipeline,
    paths: &ArtifactPaths,
    checkpoint: &mut ChunkCheckpoint,
    progress_callback: &F,
) -> Result<(), AppError>
where
    F: Fn(PipelineProgressStage, &str, u8),
{
    let summary = consistency::apply_first_seen_term_consistency(checkpoint);
    let final_context_repairs = repair_body_chunks_with_final_context(checkpoint);
    let heading_repairs = apply_heading_bilingual_repairs(checkpoint);
    let quote_repairs = apply_quote_imbalance_repairs(checkpoint);
    let advanced_chunks = mark_translated_body_chunks_consistent(checkpoint);
    if !summary.has_changes()
        && final_context_repairs == 0
        && advanced_chunks == 0
        && heading_repairs == 0
        && quote_repairs == 0
    {
        return Ok(());
    }

    checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
    pipeline.record_event(
        &paths.event_log_path,
        "term_consistency",
        &format!(
            "{}; final_context_repairs={final_context_repairs}; heading_repairs={heading_repairs}; quote_repairs={quote_repairs}",
            summary.to_event_message()
        ),
        92,
    )?;
    progress_callback(
        PipelineProgressStage::TermConsistency,
        "checking term consistency",
        92,
    );
    Ok(())
}

fn mark_translated_body_chunks_consistent(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut advanced = 0usize;
    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body || chunk.translated.is_none() {
            continue;
        }
        if checkpoint::advance_chunk_processing_stage(
            chunk,
            ChunkProcessingStage::TermConsistencyApplied,
        ) {
            advanced += 1;
        }
    }
    advanced
}

/// 曲引号配对修复挂载（只动引号本身）：恰好缺1右 + 译文末无右引号 +
/// 源文以引语结尾 → 末尾补1右。其余歧义形态不修（留人工/复审）。
/// 返回修复块数。
fn apply_quote_imbalance_repairs(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut repaired = 0usize;
    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_mut() else {
            continue;
        };
        let (fixed, n) = crate::pipeline::drift::repair_quote_imbalance(&chunk.source, translated);
        if n > 0 && fixed != *translated {
            *translated = fixed;
            repaired += 1;
        }
    }
    repaired
}

/// 章标题双语残留的确定性修复挂载：「Chapter N: 中文名」→「第N章：中文名」。
/// 在 term_consistency 阶段一并完成（与专名锁定同处），利用既有 repair_body_chunks
/// 的持久化与事件通道。返回修复块数。
fn apply_heading_bilingual_repairs(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut repaired = 0usize;
    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_mut() else {
            continue;
        };
        let fixed = crate::pipeline::drift::repair_heading_bilingual_residue(translated);
        if fixed != *translated {
            *translated = fixed;
            repaired += 1;
        }
    }
    repaired
}
