use super::super::paths::ArtifactPaths;
use super::super::types::{
    ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind, ChunkState, PipelineRunConfig,
    RuntimePromptConfig,
};
use super::super::{
    artifact, chunking::split_reference_aware_markdown, context::build_translation_context,
};
use super::chunk_policy::{apply_reference_chunk_policy, reconcile_reference_aware_chunks};
use super::resume::{
    checkpoint_resume_compatibility, invalidate_polluted_translations, ResumeCompatibility,
};
use crate::error::AppError;
use chrono::Utc;

struct CheckpointCandidate {
    checkpoint: ChunkCheckpoint,
    compatibility: ResumeCompatibility,
    translated_count: usize,
    write_count: usize,
    timestamp: String,
}

struct CheckpointCandidateSelection {
    checkpoint_file_unreadable: bool,
    selected: Option<CheckpointCandidate>,
}

pub(super) fn load_or_initialize_checkpoint(
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    source_markdown: &str,
    source_hash: &str,
) -> Result<super::CheckpointLoadOutcome, AppError> {
    let chunks = split_reference_aware_markdown(
        source_markdown,
        config.chunk_size,
        &runtime_prompt.article_type,
    );
    let selection =
        load_best_checkpoint_candidate(config, runtime_prompt, paths, source_hash, &chunks)?;
    if let Some(candidate) = selection.selected {
        let mut checkpoint = candidate.checkpoint;
        for chunk in &mut checkpoint.chunks {
            super::ensure_chunk_processing_stage(chunk);
        }
        // 块级失败隔离的恢复语义：上轮重试耗尽的失败块重置为 Pending，
        // 本轮重新进入翻译队列（其余已完成块不受影响）。
        reset_failed_chunks_for_retry(&mut checkpoint);
        if candidate.compatibility.should_refresh_source_hash() {
            checkpoint.source_hash = source_hash.to_string();
        }
        if checkpoint.translation_context.is_none() {
            checkpoint.translation_context = build_translation_context(
                source_markdown,
                config.chunk_size,
                &checkpoint.article_type,
                &config.static_glossary_entries,
            );
        }
        let article_type = checkpoint.article_type.clone();
        reconcile_reference_aware_chunks(
            &mut checkpoint,
            source_markdown,
            config.chunk_size,
            &article_type,
        );
        rebuild_wave_term_key_index(&mut checkpoint);
        apply_reference_chunk_policy(&mut checkpoint);
        let invalidated_chunks = invalidate_polluted_translations(&mut checkpoint);
        let resumed_translated_chunks = translated_chunk_count(&checkpoint);
        let total_chunks = checkpoint.chunks.len();
        artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;
        return Ok(super::CheckpointLoadOutcome {
            checkpoint,
            status: super::CheckpointLoadStatus::Resumed {
                invalidated_chunks,
                resumed_translated_chunks,
                total_chunks,
                source_hash_refreshed: candidate.compatibility.should_refresh_source_hash(),
            },
        });
    }

    let now = Utc::now().to_rfc3339();
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: runtime_prompt.article_type.clone(),
        system_prompt_hash: runtime_prompt.system_prompt_hash.clone(),
        skill_ids: runtime_prompt.skill_ids.clone(),
        translation_context: build_translation_context(
            source_markdown,
            config.chunk_size,
            &runtime_prompt.article_type,
            &config.static_glossary_entries,
        ),
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: now.clone(),
        updated_at: now.clone(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        chunks: chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| ChunkState {
                index,
                source,
                translated: None,
                confirmed_terms: Vec::new(),
                attempts: 0,
                updated_at: now.clone(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Pending),
            })
            .collect(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
    };
    apply_reference_chunk_policy(&mut checkpoint);
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;

    Ok(super::CheckpointLoadOutcome {
        checkpoint,
        status: if selection.checkpoint_file_unreadable {
            super::CheckpointLoadStatus::RebuiltFromUnreadable
        } else {
            super::CheckpointLoadStatus::Created
        },
    })
}

/// 块级失败隔离的重试语义：上一轮重试耗尽的失败块重置为 Pending，
/// 恢复运行时重新进入翻译队列补译（已完成的块不受影响）。
fn reset_failed_chunks_for_retry(checkpoint: &mut ChunkCheckpoint) {
    for chunk in &mut checkpoint.chunks {
        if chunk.processing_stage == Some(ChunkProcessingStage::TranslationFailed) {
            chunk.processing_stage = Some(ChunkProcessingStage::Pending);
        }
    }
}

fn rebuild_wave_term_key_index(checkpoint: &mut ChunkCheckpoint) {
    if !checkpoint.delta_sync.wave_term_key_index.is_empty() {
        return;
    }
    for (index, term) in checkpoint.delta_sync.wave_delta_log.iter().enumerate() {
        let key = crate::pipeline::context::normalize_resource_term_key(&term.source);
        if key.is_empty() {
            continue;
        }
        checkpoint
            .delta_sync
            .wave_term_key_index
            .entry(key)
            .or_default()
            .push(index);
    }
}

fn load_best_checkpoint_candidate(
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    source_hash: &str,
    source_chunks: &[String],
) -> Result<CheckpointCandidateSelection, AppError> {
    let mut checkpoint_file_unreadable = false;
    let mut selected = None;
    if paths.checkpoint_path.exists() {
        match artifact::read_json::<ChunkCheckpoint>(&paths.checkpoint_path) {
            Ok(checkpoint) => update_selected_checkpoint_candidate(
                &mut selected,
                checkpoint,
                config,
                runtime_prompt,
                source_hash,
                source_chunks,
                0,
                "checkpoint.json".to_string(),
            ),
            Err(_) => {
                checkpoint_file_unreadable = true;
            }
        }
    }

    for entry in
        artifact::read_json_lines::<artifact::CheckpointWalEntry>(&paths.checkpoint_wal_path)?
    {
        if !is_committed_checkpoint_wal_event(&entry.event) {
            continue;
        }
        let Some(checkpoint) = entry.checkpoint else {
            continue;
        };
        update_selected_checkpoint_candidate(
            &mut selected,
            checkpoint,
            config,
            runtime_prompt,
            source_hash,
            source_chunks,
            entry.write_count,
            entry.timestamp,
        );
    }

    Ok(CheckpointCandidateSelection {
        checkpoint_file_unreadable,
        selected,
    })
}

fn update_selected_checkpoint_candidate(
    selected: &mut Option<CheckpointCandidate>,
    checkpoint: ChunkCheckpoint,
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    source_hash: &str,
    source_chunks: &[String],
    write_count: usize,
    timestamp: String,
) {
    let Some(compatibility) = checkpoint_resume_compatibility(
        &checkpoint,
        source_hash,
        config.chunk_size,
        &runtime_prompt.system_prompt_hash,
        source_chunks,
    ) else {
        return;
    };
    let candidate = CheckpointCandidate {
        translated_count: translated_chunk_count(&checkpoint),
        checkpoint,
        compatibility,
        write_count,
        timestamp,
    };
    if is_better_checkpoint_candidate(selected.as_ref(), &candidate) {
        *selected = Some(candidate);
    }
}

fn is_better_checkpoint_candidate(
    current: Option<&CheckpointCandidate>,
    next: &CheckpointCandidate,
) -> bool {
    let Some(current) = current else {
        return true;
    };
    if next.translated_count != current.translated_count {
        return next.translated_count > current.translated_count;
    }
    let next_is_exact = next.compatibility == ResumeCompatibility::ExactSourceHash;
    let current_is_exact = current.compatibility == ResumeCompatibility::ExactSourceHash;
    if next_is_exact != current_is_exact {
        return next_is_exact;
    }
    if next.write_count != current.write_count {
        return next.write_count > current.write_count;
    }
    next.timestamp > current.timestamp
}

fn translated_chunk_count(checkpoint: &ChunkCheckpoint) -> usize {
    checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.translated.is_some())
        .count()
}

fn is_committed_checkpoint_wal_event(event: &str) -> bool {
    matches!(
        event,
        "checkpoint_write_committed" | "checkpoint_flush_committed"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::CheckpointLoadStatus;
    use crate::pipeline::{checkpoint::artifact::persist_checkpoint, WaveTermDelta};
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    fn sample_run_config(root: &std::path::Path) -> PipelineRunConfig {
        PipelineRunConfig {
            task_id: "task-1".to_string(),
            pdf_path: root.join("sample.epub"),
            article_type: "fiction".to_string(),
            system_prompt: "system prompt".to_string(),
            system_prompt_hash: "prompt-hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            epub_chapter_title: None,
            chunk_size: 120,
            max_parallel: 1,
            artifact_dir: root.to_path_buf(),
            cache_config: crate::pipeline::PipelineCacheConfig::from_artifact_dir(root),
            static_glossary_entries: Vec::new(),
            global_llm_limiter: Arc::new(Semaphore::new(1)),
            cancellation_token: CancellationToken::new(),
        }
    }

    fn sample_prompt_config() -> RuntimePromptConfig {
        RuntimePromptConfig {
            system_prompt: "system prompt".to_string(),
            system_prompt_hash: "prompt-hash".to_string(),
            article_type: "fiction".to_string(),
            skill_ids: vec!["translation:core".to_string()],
        }
    }

    fn make_checkpoint(
        config: &PipelineRunConfig,
        prompt: &RuntimePromptConfig,
        paths: &ArtifactPaths,
        source_markdown: &str,
        source_hash: &str,
    ) -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: config.task_id.clone(),
            source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
            article_type: prompt.article_type.clone(),
            system_prompt_hash: prompt.system_prompt_hash.clone(),
            skill_ids: prompt.skill_ids.clone(),
            translation_context: None,
            source_hash: source_hash.to_string(),
            chunk_size: config.chunk_size,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:01Z".to_string(),
            source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: split_reference_aware_markdown(
                source_markdown,
                config.chunk_size,
                &prompt.article_type,
            )
            .into_iter()
            .enumerate()
            .map(|(index, source)| ChunkState {
                index,
                source,
                translated: None,
                confirmed_terms: Vec::new(),
                attempts: 0,
                updated_at: "2026-01-01T00:00:01Z".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Pending),
            })
            .collect(),
        }
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-load-tests-{}-{}",
            label,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("temp dir should exist");
        root
    }

    #[test]
    fn resumes_checkpoint_and_preserves_completed_chunks() {
        let root = temp_root("resume");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
        let source_hash = "source-hash";
        let mut checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        checkpoint.chunks[0].translated = Some("translated-chunk-0".to_string());
        checkpoint.chunks[0].attempts = 1;
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        persist_checkpoint(&paths.checkpoint_path, &checkpoint).expect("checkpoint should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, source_markdown, source_hash)
                .expect("checkpoint should resume");

        match loaded.status {
            CheckpointLoadStatus::Resumed {
                invalidated_chunks, ..
            } => {
                assert_eq!(invalidated_chunks, 0);
            }
            _ => panic!("expected resumed checkpoint"),
        }
        assert_eq!(
            loaded.checkpoint.chunks[0].translated.as_deref(),
            Some("translated-chunk-0")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_chunks_reset_to_pending_on_resume_for_retry() {
        let root = temp_root("failed-retry");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        // 两段各超 chunk_size/2，确保切成两块。
        let para_one = "Satampra Zeiros crossed the great square and counted the coins in his purse again.";
        let para_two = "Queen Cunambria answered from the shadowed doorway with a voice like cold bronze.";
        let source_markdown = format!("{para_one}\n\n{para_two}");
        let source_hash = "source-hash";
        let mut checkpoint =
            make_checkpoint(&config, &prompt, &paths, &source_markdown, source_hash);
        assert!(
            checkpoint.chunks.len() >= 2,
            "test needs at least two chunks, got {}",
            checkpoint.chunks.len()
        );
        checkpoint.chunks[0].translated = Some("translated-chunk-0".to_string());
        checkpoint.chunks[0].attempts = 1;
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        // 第二块：上轮重试耗尽，标记失败且无译文。
        checkpoint.chunks[1].attempts = 5;
        checkpoint.chunks[1].processing_stage = Some(ChunkProcessingStage::TranslationFailed);
        persist_checkpoint(&paths.checkpoint_path, &checkpoint).expect("checkpoint should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, &source_markdown, source_hash)
                .expect("checkpoint should resume");

        // 已完成块不受影响；失败块重置为 Pending 进入补译队列。
        assert_eq!(
            loaded.checkpoint.chunks[0].processing_stage,
            Some(ChunkProcessingStage::Translated)
        );
        assert_eq!(
            loaded.checkpoint.chunks[1].processing_stage,
            Some(ChunkProcessingStage::Pending)
        );
        assert_eq!(loaded.checkpoint.chunks[1].translated, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rebuilds_when_prompt_hash_changes() {
        let root = temp_root("prompt-hash");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
        let source_hash = "source-hash";
        let mut checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        checkpoint.system_prompt_hash = "older-prompt-hash".to_string();
        checkpoint.chunks[0].translated = Some("stale translation".to_string());
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        persist_checkpoint(&paths.checkpoint_path, &checkpoint).expect("checkpoint should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, source_markdown, source_hash)
                .expect("checkpoint should rebuild when prompt hash changes");

        match loaded.status {
            CheckpointLoadStatus::Created => {}
            _ => panic!("expected fresh checkpoint creation"),
        }
        assert!(loaded
            .checkpoint
            .chunks
            .iter()
            .all(|chunk| chunk.translated.is_none()));
        assert_eq!(
            loaded.checkpoint.system_prompt_hash,
            prompt.system_prompt_hash
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resumes_when_source_hash_changes_but_translated_sources_match() {
        let root = temp_root("source-hash-drift");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
        let mut checkpoint = make_checkpoint(
            &config,
            &prompt,
            &paths,
            source_markdown,
            "stale-source-hash",
        );
        checkpoint.chunks[0].translated = Some("stale translation".to_string());
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        persist_checkpoint(&paths.checkpoint_path, &checkpoint).expect("checkpoint should persist");

        let loaded = load_or_initialize_checkpoint(
            &config,
            &prompt,
            &paths,
            source_markdown,
            "fresh-source-hash",
        )
        .expect("checkpoint should resume when translated sources still match");

        match loaded.status {
            CheckpointLoadStatus::Resumed {
                invalidated_chunks,
                source_hash_refreshed,
                ..
            } => {
                assert_eq!(invalidated_chunks, 0);
                assert!(source_hash_refreshed);
            }
            _ => panic!("expected source-hash drift to resume"),
        }
        assert_eq!(loaded.checkpoint.source_hash, "fresh-source-hash");
        assert_eq!(
            loaded.checkpoint.chunks[0].translated.as_deref(),
            Some("stale translation")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rebuilds_when_source_hash_changes_and_translated_source_mismatches() {
        let root = temp_root("source-hash-mismatch");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let checkpoint_source = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
        let current_source =
            "A different traveler crossed the bridge.\n\nQueen Cunambria answered.";
        let mut checkpoint = make_checkpoint(
            &config,
            &prompt,
            &paths,
            checkpoint_source,
            "stale-source-hash",
        );
        checkpoint.chunks[0].translated = Some("stale translation".to_string());
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        persist_checkpoint(&paths.checkpoint_path, &checkpoint).expect("checkpoint should persist");

        let loaded = load_or_initialize_checkpoint(
            &config,
            &prompt,
            &paths,
            current_source,
            "fresh-source-hash",
        )
        .expect("checkpoint should rebuild when translated source mismatches");

        match loaded.status {
            CheckpointLoadStatus::Created => {}
            _ => panic!("expected fresh checkpoint creation"),
        }
        assert!(loaded
            .checkpoint
            .chunks
            .iter()
            .all(|chunk| chunk.translated.is_none()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resumes_from_committed_wal_when_checkpoint_file_is_missing() {
        let root = temp_root("wal-resume");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
        let source_hash = "source-hash";
        let mut checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        checkpoint.chunks[0].translated = Some("translated-chunk-0".to_string());
        checkpoint.chunks[0].attempts = 1;
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        artifact::append_checkpoint_wal(
            &paths.checkpoint_wal_path,
            "checkpoint_write_committed",
            &checkpoint,
        )
        .expect("wal should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, source_markdown, source_hash)
                .expect("checkpoint should resume from wal");

        match loaded.status {
            CheckpointLoadStatus::Resumed {
                invalidated_chunks, ..
            } => {
                assert_eq!(invalidated_chunks, 0);
            }
            _ => panic!("expected resumed checkpoint"),
        }
        assert_eq!(
            loaded.checkpoint.chunks[0].translated.as_deref(),
            Some("translated-chunk-0")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_committed_wal_when_checkpoint_file_is_unreadable() {
        let root = temp_root("wal-fallback");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
        let source_hash = "source-hash";
        std::fs::write(&paths.checkpoint_path, "{broken json")
            .expect("broken checkpoint should be written");
        let mut checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        checkpoint.chunks[0].translated = Some("translated-chunk-0".to_string());
        checkpoint.chunks[0].attempts = 1;
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        artifact::append_checkpoint_wal(
            &paths.checkpoint_wal_path,
            "checkpoint_write_committed",
            &checkpoint,
        )
        .expect("wal should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, source_markdown, source_hash)
                .expect("checkpoint should resume from wal fallback");

        match loaded.status {
            CheckpointLoadStatus::Resumed {
                invalidated_chunks, ..
            } => {
                assert_eq!(invalidated_chunks, 0);
            }
            _ => panic!("expected resumed checkpoint"),
        }
        assert_eq!(
            loaded.checkpoint.chunks[0].translated.as_deref(),
            Some("translated-chunk-0")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn chooses_highest_progress_wal_candidate_over_newer_low_checkpoint() {
        let root = temp_root("wal-high-watermark");
        let paths = ArtifactPaths::new(&root);
        let mut config = sample_run_config(&root);
        config.chunk_size = 48;
        let prompt = sample_prompt_config();
        let source_markdown = concat!(
            "First translated paragraph has enough text to stand alone.\n",
            "Second translated paragraph also has enough text to split.\n",
            "Third pending paragraph remains for the resumed run.\n"
        );
        let source_hash = "source-hash";
        let mut high_checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        assert!(
            high_checkpoint.chunks.len() >= 3,
            "test fixture must create multiple chunks"
        );
        high_checkpoint.chunks[0].translated = Some("translated chunk zero".to_string());
        high_checkpoint.chunks[1].translated = Some("translated chunk one".to_string());
        high_checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        high_checkpoint.chunks[1].processing_stage = Some(ChunkProcessingStage::Translated);
        high_checkpoint.write_metrics.write_count = 10;
        artifact::append_checkpoint_wal(
            &paths.checkpoint_wal_path,
            "checkpoint_flush_committed",
            &high_checkpoint,
        )
        .expect("high-watermark wal should persist");

        let mut low_checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        low_checkpoint.chunks[0].translated = Some("newer low chunk zero".to_string());
        low_checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        low_checkpoint.write_metrics.write_count = 20;
        persist_checkpoint(&paths.checkpoint_path, &low_checkpoint)
            .expect("low checkpoint file should persist");
        artifact::append_checkpoint_wal(
            &paths.checkpoint_wal_path,
            "checkpoint_flush_committed",
            &low_checkpoint,
        )
        .expect("low wal should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, source_markdown, source_hash)
                .expect("checkpoint should resume from highest progress wal");

        match loaded.status {
            CheckpointLoadStatus::Resumed {
                resumed_translated_chunks,
                total_chunks,
                ..
            } => {
                assert_eq!(resumed_translated_chunks, 2);
                assert_eq!(total_chunks, high_checkpoint.chunks.len());
            }
            _ => panic!("expected resumed checkpoint"),
        }
        assert_eq!(
            loaded.checkpoint.chunks[1].translated.as_deref(),
            Some("translated chunk one")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rebuilds_wave_term_key_index_when_resuming_legacy_checkpoint() {
        let root = temp_root("wave-index");
        let paths = ArtifactPaths::new(&root);
        let config = sample_run_config(&root);
        let prompt = sample_prompt_config();
        let source_markdown = "The nudge+ instrument stayed visible.";
        let source_hash = "source-hash";
        let mut checkpoint =
            make_checkpoint(&config, &prompt, &paths, source_markdown, source_hash);
        checkpoint.chunks[0].translated = Some("translated".to_string());
        checkpoint.chunks[0].attempts = 1;
        checkpoint.chunks[0].processing_stage = Some(ChunkProcessingStage::Translated);
        checkpoint.delta_sync.wave_delta_log = vec![WaveTermDelta {
            seq: 1,
            source: "nudge plus instrument".to_string(),
            target: "target".to_string(),
            chunk_index: 0,
        }];
        persist_checkpoint(&paths.checkpoint_path, &checkpoint).expect("checkpoint should persist");

        let loaded =
            load_or_initialize_checkpoint(&config, &prompt, &paths, source_markdown, source_hash)
                .expect("checkpoint should resume");

        let key = crate::pipeline::context::normalize_resource_term_key("nudge+ instrument");
        assert_eq!(
            loaded.checkpoint.delta_sync.wave_term_key_index.get(&key),
            Some(&vec![0])
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
