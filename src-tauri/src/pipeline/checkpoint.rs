mod chunk_policy;
mod encoding;
mod load;
mod resume;
mod stage;

use super::artifact;
use super::paths::ArtifactPaths;
use super::types::{ChunkCheckpoint, PipelineRunConfig, RuntimePromptConfig};
use crate::error::AppError;
use chrono::Utc;
use std::time::Instant;

pub(super) enum CheckpointLoadStatus {
    Resumed {
        invalidated_chunks: usize,
        resumed_translated_chunks: usize,
        total_chunks: usize,
        source_hash_refreshed: bool,
    },
    Created,
    RebuiltFromUnreadable,
}

pub(super) struct CheckpointLoadOutcome {
    pub(super) checkpoint: ChunkCheckpoint,
    pub(super) status: CheckpointLoadStatus,
}

pub(super) fn load_or_initialize_checkpoint(
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    source_markdown: &str,
    source_hash: &str,
) -> Result<CheckpointLoadOutcome, AppError> {
    load::load_or_initialize_checkpoint(config, runtime_prompt, paths, source_markdown, source_hash)
}

#[cfg(test)]
pub(super) fn apply_reference_chunk_policy(checkpoint: &mut ChunkCheckpoint) {
    chunk_policy::apply_reference_chunk_policy(checkpoint);
}

pub(super) fn reject_mojibake_translation(
    chunk_index: usize,
    translated: &str,
) -> Result<(), AppError> {
    encoding::reject_mojibake_translation(chunk_index, translated)
}

pub(super) fn ensure_chunk_processing_stage(chunk: &mut super::types::ChunkState) {
    stage::ensure_chunk_processing_stage(chunk)
}

pub(super) fn advance_chunk_processing_stage(
    chunk: &mut super::types::ChunkState,
    next: super::types::ChunkProcessingStage,
) -> bool {
    stage::advance_chunk_processing_stage(chunk, next)
}

pub(super) fn current_chunk_processing_stage(
    chunk: &super::types::ChunkState,
) -> super::types::ChunkProcessingStage {
    stage::current_chunk_processing_stage(chunk)
}

pub(super) fn persist_dirty_checkpoint(
    paths: &ArtifactPaths,
    checkpoint: &mut ChunkCheckpoint,
) -> Result<(), AppError> {
    checkpoint.updated_at = Utc::now().to_rfc3339();
    checkpoint.write_metrics.write_count = checkpoint.write_metrics.write_count.saturating_add(1);
    checkpoint.write_metrics.pending_write_count = checkpoint
        .write_metrics
        .pending_write_count
        .saturating_add(1);
    let _ = artifact::append_checkpoint_wal(
        &paths.checkpoint_wal_path,
        "checkpoint_write_started",
        checkpoint,
    );
    let flush_interval = artifact::checkpoint_flush_interval(checkpoint.write_metrics.write_count);
    // 稳态（write_count ≥ 20）加时间窗：除块数间隔外，距上次 flush 至少 10s，
    // 避免长书中后期每 4 块就全量序列化落盘（O(n²) 磁盘放大）。
    let min_interval_ok = if checkpoint.write_metrics.write_count >= 20 {
        let last_flush_at = checkpoint
            .write_metrics
            .last_flush_timestamp
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
        match last_flush_at {
            Some(t) => {
                Utc::now()
                    .signed_duration_since(t.with_timezone(&Utc))
                    .num_seconds()
                    >= artifact::CHECKPOINT_FLUSH_MIN_INTERVAL_SECS as i64
            }
            None => true,
        }
    } else {
        true
    };
    let should_flush =
        checkpoint.write_metrics.pending_write_count >= flush_interval && min_interval_ok;
    if !should_flush {
        return Ok(());
    }
    let started = Instant::now();
    artifact::persist_checkpoint(&paths.checkpoint_path, checkpoint)?;
    checkpoint.write_metrics.total_write_ms = checkpoint
        .write_metrics
        .total_write_ms
        .saturating_add(started.elapsed().as_millis());
    checkpoint.write_metrics.last_flush_write_count = checkpoint.write_metrics.write_count;
    checkpoint.write_metrics.last_flush_timestamp = Some(Utc::now().to_rfc3339());
    checkpoint.write_metrics.pending_write_count = 0;
    let _ = artifact::append_checkpoint_wal(
        &paths.checkpoint_wal_path,
        "checkpoint_flush_committed",
        checkpoint,
    );
    Ok(())
}

pub(super) fn flush_checkpoint(
    paths: &ArtifactPaths,
    checkpoint: &mut ChunkCheckpoint,
) -> Result<(), AppError> {
    if checkpoint.write_metrics.pending_write_count == 0 && paths.checkpoint_path.exists() {
        return Ok(());
    }
    checkpoint.updated_at = Utc::now().to_rfc3339();
    let _ = artifact::append_checkpoint_wal(
        &paths.checkpoint_wal_path,
        "checkpoint_flush_started",
        checkpoint,
    );
    let started = Instant::now();
    artifact::persist_checkpoint(&paths.checkpoint_path, checkpoint)?;
    checkpoint.write_metrics.total_write_ms = checkpoint
        .write_metrics
        .total_write_ms
        .saturating_add(started.elapsed().as_millis());
    checkpoint.write_metrics.last_flush_write_count = checkpoint.write_metrics.write_count;
    checkpoint.write_metrics.pending_write_count = 0;
    let _ = artifact::append_checkpoint_wal(
        &paths.checkpoint_wal_path,
        "checkpoint_write_committed",
        checkpoint,
    );
    Ok(())
}
