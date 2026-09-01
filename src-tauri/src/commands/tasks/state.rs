use super::types::TaskStopReason;
use super::{AppError, TaskArtifactPaths, TaskError, TaskPhase, TaskStatus, TranslationTask};
use crate::pipeline::{PipelineProgressUpdate, PipelineResult};
use std::path::Path;

pub(super) fn repair_interrupted_task(task: &mut TranslationTask, updated_at: String) {
    if task.status == TaskStatus::Pending || task.status == TaskStatus::Processing {
        task.status = TaskStatus::Failed;
        task.phase = interrupted_phase(task.phase.clone());
        task.message = "任务在上次运行中断，可重新发起".to_string();
        task.last_error = Some(TaskError {
            code: "INTERRUPTED".to_string(),
            message: task.message.clone(),
            retryable: true,
            retry_after_ms: None,
        });
        task.updated_at = updated_at;
    }
}

fn interrupted_phase(previous_phase: TaskPhase) -> TaskPhase {
    match previous_phase {
        TaskPhase::Completed | TaskPhase::Failed | TaskPhase::Cancelled | TaskPhase::Paused => {
            TaskPhase::Failed
        }
        other => other,
    }
}

pub(super) fn apply_running_progress(
    task: &mut TranslationTask,
    update: &PipelineProgressUpdate,
    retry_count: u8,
) {
    task.status = TaskStatus::Processing;
    task.phase = phase_from_pipeline_stage(update.stage);
    task.progress = update.progress.min(100);
    task.message = update.message.clone();
    task.retry_count = retry_count;
    task.last_error = None;
}

pub(super) fn apply_task_success(
    task: &mut TranslationTask,
    result: &PipelineResult,
    artifact_dir: &Path,
) {
    task.status = TaskStatus::Completed;
    task.phase = TaskPhase::Completed;
    task.progress = 100;
    task.message = "翻译完成".to_string();
    task.output_path = Some(result.output_markdown_path.clone());
    task.html_output_path = Some(result.output_html_path.clone());
    task.cover_path = result.cover_data_url.clone();
    task.total_chunks = result.total_chunks;
    task.translated_chunks = result.translated_chunks;
    task.source_hash = Some(result.source_hash.clone());
    task.last_error = None;
    task.artifact_paths = TaskArtifactPaths {
        artifact_dir: Some(artifact_dir.to_string_lossy().to_string()),
        source_markdown_path: Some(result.source_markdown_path.clone()),
        source_toc_path: Some(
            artifact_dir
                .join("source_toc.json")
                .to_string_lossy()
                .to_string(),
        ),
        translated_toc_path: Some(
            artifact_dir
                .join("translated_toc.json")
                .to_string_lossy()
                .to_string(),
        ),
        output_markdown_path: Some(result.output_markdown_path.clone()),
        output_html_path: Some(result.output_html_path.clone()),
        translated_epub_path: result.translated_epub_path.clone(),
        bilingual_epub_path: result.bilingual_epub_path.clone(),
        checkpoint_path: Some(result.checkpoint_path.clone()),
        manifest_path: Some(result.artifact_manifest_path.clone()),
        event_log_path: Some(result.event_log_path.clone()),
        validation_report_path: Some(result.validation_report_path.clone()),
        glossary_path: Some(result.glossary_path.clone()),
        metrics_path: Some(artifact_dir.join("metrics.json").to_string_lossy().to_string()),
    };
}

pub(super) fn apply_task_failure(task: &mut TranslationTask, error: &AppError, retry_count: u8) {
    task.status = TaskStatus::Failed;
    task.phase = failed_phase(error, None);
    task.message = error.message.clone();
    task.retry_count = retry_count;
    task.last_error = Some(TaskError::from(error));
}

pub(super) fn apply_task_requeued(task: &mut TranslationTask) {
    task.status = TaskStatus::Pending;
    task.phase = TaskPhase::Imported;
    task.progress = 0;
    task.message = "任务重新排队".to_string();
    task.retry_count = task.retry_count.saturating_add(1);
    task.last_error = None;
}

pub(super) fn apply_task_cancelled(task: &mut TranslationTask) {
    task.status = TaskStatus::Failed;
    task.phase = TaskPhase::Cancelled;
    task.message = "任务已取消".to_string();
    task.last_error = Some(TaskError {
        code: "CANCELLED".to_string(),
        message: task.message.clone(),
        retryable: true,
        retry_after_ms: None,
    });
}

pub(super) fn apply_task_paused(task: &mut TranslationTask) {
    task.status = TaskStatus::Paused;
    task.phase = TaskPhase::Paused;
    task.message = "任务已暂停，可继续".to_string();
    task.last_error = Some(TaskError {
        code: "PAUSED".to_string(),
        message: task.message.clone(),
        retryable: true,
        retry_after_ms: None,
    });
}

pub(super) fn phase_from_pipeline_stage(
    stage: crate::pipeline::PipelineProgressStage,
) -> TaskPhase {
    match stage {
        crate::pipeline::PipelineProgressStage::Imported => TaskPhase::Imported,
        crate::pipeline::PipelineProgressStage::OcrRunning => TaskPhase::OcrRunning,
        crate::pipeline::PipelineProgressStage::Chunking => TaskPhase::Chunking,
        crate::pipeline::PipelineProgressStage::Translating
        | crate::pipeline::PipelineProgressStage::TermConsistency
        | crate::pipeline::PipelineProgressStage::ResidualReview => TaskPhase::Translating,
        crate::pipeline::PipelineProgressStage::Rendering => TaskPhase::Rendering,
        crate::pipeline::PipelineProgressStage::WritingArtifacts => TaskPhase::WritingArtifacts,
        crate::pipeline::PipelineProgressStage::Completed => TaskPhase::Completed,
    }
}

pub(super) fn failed_phase(error: &AppError, stop_reason: Option<TaskStopReason>) -> TaskPhase {
    match (error.code.as_ref(), stop_reason) {
        ("CANCELLED", Some(TaskStopReason::Pause)) => TaskPhase::Paused,
        ("CANCELLED", _) => TaskPhase::Cancelled,
        _ => TaskPhase::Failed,
    }
}

pub(super) fn apply_task_failure_with_reason(
    task: &mut TranslationTask,
    error: &AppError,
    retry_count: u8,
    stop_reason: Option<TaskStopReason>,
) {
    match (error.code.as_ref(), stop_reason) {
        ("CANCELLED", Some(TaskStopReason::Pause)) => {
            task.retry_count = retry_count;
            apply_task_paused(task);
        }
        _ => apply_task_failure(task, error, retry_count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(task_id: &str) -> TranslationTask {
        TranslationTask {
            id: task_id.to_string(),
            filename: "demo.pdf".to_string(),
            pdf_path: "D:/demo.pdf".to_string(),
            status: TaskStatus::Pending,
            phase: TaskPhase::Imported,
            progress: 0,
            message: String::new(),
            article_type: "fiction".to_string(),
            concurrent: true,
            max_workers: 4,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            output_path: None,
            html_output_path: None,
            cover_path: None,
            artifact_paths: TaskArtifactPaths::default(),
            total_chunks: 0,
            translated_chunks: 0,
            retry_count: 0,
            source_hash: None,
            last_error: None,
        }
    }

    #[test]
    fn requeue_transition_clears_last_error_and_resets_progress() {
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Failed;
        task.phase = TaskPhase::Failed;
        task.progress = 66;
        task.retry_count = 1;
        task.last_error = Some(TaskError {
            code: "LLM_TIMEOUT".to_string(),
            message: "timeout".to_string(),
            retryable: true,
            retry_after_ms: Some(1000),
        });

        apply_task_requeued(&mut task);

        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.phase, TaskPhase::Imported);
        assert_eq!(task.progress, 0);
        assert_eq!(task.message, "任务重新排队");
        assert_eq!(task.retry_count, 2);
        assert!(task.last_error.is_none());
    }

    #[test]
    fn success_transition_replaces_artifact_contract() {
        let mut task = sample_task("task-1");
        task.last_error = Some(TaskError {
            code: "OLD".to_string(),
            message: "stale".to_string(),
            retryable: false,
            retry_after_ms: None,
        });
        let result = PipelineResult {
            translated_markdown: "译文".to_string(),
            translated_html: "<p>译文</p>".to_string(),
            cover_data_url: Some("data:image/png;base64,abc".to_string()),
            source_markdown_path: "source.md".to_string(),
            output_markdown_path: "translated.md".to_string(),
            output_html_path: "translated.html".to_string(),
            translated_epub_path: Some("book_zh.epub".to_string()),
            bilingual_epub_path: Some("book_bilingual.epub".to_string()),
            artifact_manifest_path: "manifest.json".to_string(),
            checkpoint_path: "checkpoint.json".to_string(),
            event_log_path: "events.ndjson".to_string(),
            validation_report_path: "validation.json".to_string(),
            glossary_path: "glossary.json".to_string(),
            metrics_path: "metrics.json".to_string(),
            source_hash: "hash-1".to_string(),
            translated_chunks: 3,
            total_chunks: 5,
            failed_chunks: 0,
            failed_chunk_indexes: Vec::new(),
        };

        apply_task_success(&mut task, &result, Path::new("D:/artifacts/task-1"));

        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.phase, TaskPhase::Completed);
        assert_eq!(task.progress, 100);
        assert_eq!(task.message, "翻译完成");
        assert_eq!(task.output_path.as_deref(), Some("translated.md"));
        assert_eq!(task.html_output_path.as_deref(), Some("translated.html"));
        assert_eq!(
            task.artifact_paths.bilingual_epub_path.as_deref(),
            Some("book_bilingual.epub")
        );
        assert_eq!(
            task.cover_path.as_deref(),
            Some("data:image/png;base64,abc")
        );
        assert_eq!(task.translated_chunks, 3);
        assert_eq!(task.total_chunks, 5);
        assert_eq!(task.source_hash.as_deref(), Some("hash-1"));
        assert!(task.last_error.is_none());
        assert_eq!(
            task.artifact_paths.artifact_dir.as_deref(),
            Some("D:/artifacts/task-1")
        );
    }

    #[test]
    fn cancelled_failure_transition_is_retryable() {
        let mut task = sample_task("task-1");

        apply_task_failure(&mut task, &AppError::cancelled("stop requested"), 2);

        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.phase, TaskPhase::Cancelled);
        assert_eq!(task.retry_count, 2);
        let error = task.last_error.as_ref().expect("error should exist");
        assert_eq!(error.code, "CANCELLED");
        assert!(error.retryable);
    }

    #[test]
    fn paused_failure_transition_is_retryable() {
        let mut task = sample_task("task-1");

        apply_task_failure_with_reason(
            &mut task,
            &AppError::cancelled("pause requested"),
            2,
            Some(TaskStopReason::Pause),
        );

        assert_eq!(task.status, TaskStatus::Paused);
        assert_eq!(task.phase, TaskPhase::Paused);
        assert_eq!(task.retry_count, 2);
        let error = task.last_error.as_ref().expect("error should exist");
        assert_eq!(error.code, "PAUSED");
        assert!(error.retryable);
    }

    #[test]
    fn running_progress_transition_tracks_explicit_pipeline_stage() {
        let mut task = sample_task("task-1");
        task.last_error = Some(TaskError {
            code: "OLD".to_string(),
            message: "stale".to_string(),
            retryable: false,
            retry_after_ms: None,
        });
        let update = PipelineProgressUpdate::new(
            crate::pipeline::PipelineProgressStage::TermConsistency,
            "checking term consistency",
            91,
        );

        apply_running_progress(&mut task, &update, 2);

        assert_eq!(task.status, TaskStatus::Processing);
        assert_eq!(task.phase, TaskPhase::Translating);
        assert_eq!(task.progress, 91);
        assert_eq!(task.message, "checking term consistency");
        assert_eq!(task.retry_count, 2);
        assert!(task.last_error.is_none());
    }

    #[test]
    fn interrupted_repair_only_changes_non_terminal_tasks() {
        let mut pending = sample_task("task-1");
        pending.phase = TaskPhase::Translating;
        repair_interrupted_task(&mut pending, "2026-01-02T00:00:00Z".to_string());
        assert_eq!(pending.status, TaskStatus::Failed);
        assert_eq!(pending.phase, TaskPhase::Translating);
        let pending_error = pending
            .last_error
            .as_ref()
            .expect("interrupted task should expose error");
        assert_eq!(pending_error.code, "INTERRUPTED");
        assert_eq!(pending_error.message, "任务在上次运行中断，可重新发起");
        assert!(pending_error.retryable);

        let mut completed = sample_task("task-2");
        completed.status = TaskStatus::Completed;
        completed.phase = TaskPhase::Completed;
        completed.message = "翻译完成".to_string();
        repair_interrupted_task(&mut completed, "2026-01-02T00:00:00Z".to_string());
        assert_eq!(completed.status, TaskStatus::Completed);
        assert_eq!(completed.phase, TaskPhase::Completed);
        assert_eq!(completed.message, "翻译完成");
    }

    #[test]
    fn interrupted_repair_falls_back_to_failed_for_terminal_phase_values() {
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Processing;
        task.phase = TaskPhase::Cancelled;

        repair_interrupted_task(&mut task, "2026-01-02T00:00:00Z".to_string());

        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.phase, TaskPhase::Failed);
    }
}
