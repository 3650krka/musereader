use super::{lock_mutex, persist_json, report_nonfatal_error, AppError, AppState};
use crate::skill_library::{
    load_translation_prompt_bundle_with_core, supported_article_types, TranslationPromptBundle,
};
use std::path::PathBuf;

mod events;
mod files;
mod lifecycle;
mod operations;
mod runner;
mod runner_controls;
mod runner_state;
mod state;
mod types;

pub(crate) use events::{emit_task_progress, TaskEventSink};
use files::{cleanup_artifact_dir, create_artifact_dir, delete_task_files, ensure_document_path};
use lifecycle::{
    build_pending_task, ensure_resumable, mark_task_cancelled, mark_task_paused,
    mark_task_start_failed, persist_new_task, task_snapshot, validate_article_type,
    validate_max_workers,
};
use operations::{
    delete_task_state_and_persist, prepare_task_delete, resume_translation_task_inner,
    retry_translation_task_inner, start_translation_inner, translate_epub_chapter_once_inner,
};
use runner_controls::{abort_task_control, cancel_task_control, pause_task_control};
pub use types::{ChapterTranslationRunResult, TranslationTask};
pub(super) use types::{TaskArtifactPaths, TaskControl, TaskError, TaskPhase, TaskStatus};

pub(super) const DEFAULT_CHUNK_SIZE: usize = 3000;
/// 分块尺寸钳制区间：下限防止极端碎片化（块开销/上下文断裂），
/// 上限防止单块超出常见模型输出窗口（重试切片也救不回超长块）。
pub(super) const MIN_CHUNK_SIZE: usize = 500;
pub(super) const MAX_CHUNK_SIZE: usize = 20000;

pub(super) fn clamp_chunk_size(value: usize) -> usize {
    value.clamp(MIN_CHUNK_SIZE, MAX_CHUNK_SIZE)
}

/// 任务分块尺寸：优先用户运行配置（设置面板），缺省回落 3000；统一钳制。
pub(super) fn resolve_task_chunk_size(runtime_config: &super::RuntimeConfig) -> usize {
    clamp_chunk_size(runtime_config.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE))
}

pub(super) fn repair_loaded_tasks(
    tasks: std::collections::HashMap<String, TranslationTask>,
) -> std::collections::HashMap<String, TranslationTask> {
    lifecycle::repair_loaded_tasks(tasks)
}

pub(super) fn ensure_epub_path(document_path: &str) -> Result<PathBuf, AppError> {
    files::ensure_epub_path(document_path)
}

pub(super) fn report_task_nonfatal_error(
    state: &AppState,
    operation: &str,
    task_id: &str,
    error: &AppError,
) {
    report_nonfatal_error(
        Some(&state.nonfatal_notices),
        "tasks",
        &format!("task_id={task_id} operation={operation}"),
        error,
    );
}

pub(super) fn handle_task_start_failure(
    state: &AppState,
    task_id: &str,
    error: AppError,
    retry_count: u8,
    operation: &str,
) -> Result<(), AppError> {
    if let Err(writeback_error) = mark_task_start_failed(state, task_id, &error, retry_count) {
        report_task_nonfatal_error(state, operation, task_id, &writeback_error);
    }
    Err(error)
}

pub(super) fn run_with_artifact_cleanup<T, F>(
    artifact_root_dir: &std::path::Path,
    artifact_dir: &std::path::Path,
    operation: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError>,
{
    match operation() {
        Ok(value) => Ok(value),
        Err(error) => {
            cleanup_artifact_dir(artifact_root_dir, artifact_dir);
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn start_translation(
    pdf_path: String,
    article_type: String,
    concurrent: bool,
    max_workers: u8,
    glossary_deck_ids: Option<Vec<String>>,
    state: tauri::State<'_, AppState>,
) -> Result<String, AppError> {
    start_translation_inner(
        &state,
        &pdf_path,
        &article_type,
        concurrent,
        max_workers,
        glossary_deck_ids,
    )
}

#[tauri::command]
pub async fn translate_epub_chapter_once(
    epub_path: String,
    chapter_title: String,
    article_type: String,
    state: tauri::State<'_, AppState>,
) -> Result<ChapterTranslationRunResult, AppError> {
    translate_epub_chapter_once_inner(&state, &epub_path, &chapter_title, &article_type).await
}

#[tauri::command]
pub async fn retry_translation_task(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    retry_translation_task_inner(&state, &task_id)
}

#[tauri::command]
pub async fn resume_translation_task(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let task = task_snapshot(&state, &task_id)?;
    ensure_resumable(&task)?;
    resume_translation_task_inner(&state, &task_id)
}

#[tauri::command]
pub async fn cancel_translation_task(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let cancelled = cancel_task_control(&state.controls, &task_id)?;
    if !cancelled {
        return Err(AppError::conflict("任务当前不可取消"));
    }
    mark_task_cancelled(&state, &task_id)
}

#[tauri::command]
pub async fn pause_translation_task(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let paused = pause_task_control(&state.controls, &task_id)?;
    if !paused {
        return Err(AppError::conflict("任务当前不可暂停"));
    }
    mark_task_paused(&state, &task_id)
}

#[tauri::command]
pub async fn get_task_status(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<TranslationTask, AppError> {
    task_snapshot(&state, &task_id)
}

#[tauri::command]
pub async fn list_tasks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TranslationTask>, AppError> {
    let tasks = lock_mutex(&state.tasks, "任务")?;
    let mut values: Vec<_> = tasks.values().cloned().collect();
    values.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(values)
}

#[tauri::command]
pub async fn delete_task(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    prepare_task_delete(&state, &task_id)?;
    if let Some(task) = delete_task_state_and_persist(&state, &task_id)? {
        delete_task_files(&task, &state.import_root_dir, &state.artifact_root_dir);
    }
    if let Err(error) = abort_task_control(&state.controls, &task_id) {
        report_task_nonfatal_error(&state, "abort_task_control_after_delete", &task_id, &error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ReaderState;
    use crate::commands::RuntimeConfig;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tokio::runtime::Runtime;
    use tokio::sync::Semaphore;

    fn sample_task(task_id: &str) -> TranslationTask {
        TranslationTask {
            id: task_id.to_string(),
            filename: "demo.epub".to_string(),
            pdf_path: "D:/demo.epub".to_string(),
            status: TaskStatus::Failed,
            phase: TaskPhase::Failed,
            progress: 100,
            message: "done".to_string(),
            article_type: "fiction".to_string(),
            concurrent: true,
            max_workers: 4,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            output_path: None,
            html_output_path: None,
            cover_path: None,
            artifact_paths: TaskArtifactPaths::default(),
            total_chunks: 5,
            translated_chunks: 5,
            retry_count: 0,
            source_hash: Some("hash-1".to_string()),
            last_error: None,
        }
    }

    fn sample_reader_state(book_id: &str) -> ReaderState {
        ReaderState {
            book_id: book_id.to_string(),
            progress: 50,
            chapter: "第 3 章".to_string(),
            href: None,
            offset: 0,
            bookmarks: Vec::new(),
            notes: Vec::new(),
            activities: Vec::new(),
            word_marks: HashMap::new(),
            vocab_cards: HashMap::new(),
            review_log: Vec::new(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn sample_state(
        root: &std::path::Path,
        task_store_path: std::path::PathBuf,
        reader_state_store_path: std::path::PathBuf,
    ) -> AppState {
        AppState {
            tasks: Arc::new(Mutex::new(HashMap::from([(
                "task-1".to_string(),
                sample_task("task-1"),
            )]))),
            config: Arc::new(Mutex::new(RuntimeConfig::default())),
            reader_states: Arc::new(Mutex::new(HashMap::from([(
                "task-1".to_string(),
                sample_reader_state("task-1"),
            )]))),
            task_store_path,
            reader_state_store_path,
            book_profile_store_path: root.join("book_profiles.json"),
            runtime_config_store_path: root.join("runtime_config.json"),
            import_root_dir: root.join("imports"),
            artifact_root_dir: root.join("artifacts"),
            skill_library_root_dir: root.join("skills"),
            wordlists_dir: root.join("wordlists"),
            llm_limiter: Arc::new(Semaphore::new(1)),
            nonfatal_notices: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            book_profiles: Arc::new(Mutex::new(HashMap::new())),
            controls: Arc::new(Mutex::new(HashMap::new())),
            task_event_sink: None,
        }
    }

    #[test]
    fn start_translation_does_not_persist_task_when_prompt_loading_fails() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-start-prompt-failure-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let reader_state_store_path = root.join("reader_states.json");
        let state = sample_state(&root, task_store_path.clone(), reader_state_store_path);
        let epub_path = root.join("demo.epub");
        fs::write(&epub_path, b"dummy epub").expect("epub fixture should exist");
        fs::write(&state.skill_library_root_dir, b"not a directory")
            .expect("skill library path should become a file");

        let runtime = Runtime::new().expect("tokio runtime should build");
        let error = runtime
            .block_on(async {
                start_translation_inner(&state, &epub_path.to_string_lossy(), "fiction", true, 4, None)
            })
            .expect_err("prompt loading failure should bubble up");

        assert_eq!(error.code, "INTERNAL");
        let tasks = lock_mutex(&state.tasks, "任务").expect("task lock");
        assert_eq!(tasks.len(), 1);
        assert!(tasks.contains_key("task-1"));
        drop(tasks);

        let persisted = fs::read_to_string(&task_store_path).unwrap_or_default();
        assert!(!persisted.contains("demo.epub"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn clamp_chunk_size_bounds_to_safe_range() {
        assert_eq!(clamp_chunk_size(120), MIN_CHUNK_SIZE);
        assert_eq!(clamp_chunk_size(0), MIN_CHUNK_SIZE);
        assert_eq!(clamp_chunk_size(120_000), MAX_CHUNK_SIZE);
        assert_eq!(clamp_chunk_size(3000), 3000);
    }

    #[test]
    fn resolve_task_chunk_size_falls_back_and_clamps() {
        let mut config = RuntimeConfig::default();
        // 未设置：回落默认 3000。
        config.chunk_size = None;
        assert_eq!(resolve_task_chunk_size(&config), DEFAULT_CHUNK_SIZE);
        // 合法值：原样。
        config.chunk_size = Some(6000);
        assert_eq!(resolve_task_chunk_size(&config), 6000);
        // 越界值：钳制到边界。
        config.chunk_size = Some(1);
        assert_eq!(resolve_task_chunk_size(&config), MIN_CHUNK_SIZE);
        config.chunk_size = Some(999_999);
        assert_eq!(resolve_task_chunk_size(&config), MAX_CHUNK_SIZE);
    }
}
