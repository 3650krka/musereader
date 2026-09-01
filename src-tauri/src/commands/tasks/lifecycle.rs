use super::events::emit_task_progress;
use super::runner_state::update_task_entry_with_events;
use super::{
    lock_mutex, persist_json, state, AppError, AppState, TaskArtifactPaths, TaskPhase, TaskStatus,
    TranslationTask,
};
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(super) fn repair_loaded_tasks(
    mut tasks: HashMap<String, TranslationTask>,
) -> HashMap<String, TranslationTask> {
    for task in tasks.values_mut() {
        state::repair_interrupted_task(task, Utc::now().to_rfc3339());
    }
    tasks
}

pub(super) fn validate_article_type(
    value: &str,
    supported_article_types: &[&str],
) -> Result<String, AppError> {
    if supported_article_types.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(AppError::invalid_input("不支持的文章类型"))
    }
}

pub(super) fn validate_max_workers(
    value: u8,
    min_workers: u8,
    max_workers: u8,
) -> Result<u8, AppError> {
    if (min_workers..=max_workers).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::invalid_input(format!(
            "线程数必须在 {min_workers} 到 {max_workers} 之间"
        )))
    }
}

pub(super) fn persist_tasks(
    state: &AppState,
    tasks: &HashMap<String, TranslationTask>,
) -> Result<(), AppError> {
    persist_json(&state.task_store_path, tasks)
}

pub(super) fn persist_reader_states(
    state: &AppState,
    reader_states: &HashMap<String, super::super::ReaderState>,
) -> Result<(), AppError> {
    persist_json(&state.reader_state_store_path, reader_states)
}

pub(super) fn build_pending_task(
    task_id: &str,
    path: &Path,
    article_type: &str,
    concurrent: bool,
    max_workers: u8,
    artifact_dir: &Path,
) -> TranslationTask {
    let now = Utc::now().to_rfc3339();
    TranslationTask {
        id: task_id.to_string(),
        filename: filename_or_unknown(path),
        pdf_path: path.to_string_lossy().to_string(),
        status: TaskStatus::Pending,
        phase: TaskPhase::Imported,
        progress: 0,
        message: "任务已创建".to_string(),
        article_type: article_type.to_string(),
        concurrent,
        max_workers,
        created_at: now.clone(),
        updated_at: now,
        output_path: None,
        html_output_path: None,
        cover_path: None,
        artifact_paths: TaskArtifactPaths {
            artifact_dir: Some(artifact_dir.to_string_lossy().to_string()),
            ..TaskArtifactPaths::default()
        },
        total_chunks: 0,
        translated_chunks: 0,
        retry_count: 0,
        source_hash: None,
        last_error: None,
    }
}

pub(super) fn persist_new_task(state: &AppState, task: TranslationTask) -> Result<(), AppError> {
    let mut tasks = lock_mutex(&state.tasks, "任务")?;
    let task_id = task.id.clone();
    let previous = tasks.insert(task_id.clone(), task);
    if let Err(error) = persist_tasks(state, &tasks) {
        if let Some(previous_task) = previous {
            tasks.insert(task_id, previous_task);
        } else {
            tasks.remove(&task_id);
        }
        return Err(error);
    }
    // 任务创建事件：仅在持久化成功后推送（前端用其替代初始轮询刷新）。
    if let Some(task) = tasks.get(&task_id) {
        emit_task_progress(&state.task_event_sink(), task);
    }
    Ok(())
}

pub(super) fn next_manual_epub_task_id() -> String {
    format!("manual-epub-chapter-{}", Uuid::new_v4())
}

pub(super) fn task_snapshot(state: &AppState, task_id: &str) -> Result<TranslationTask, AppError> {
    let tasks = lock_mutex(&state.tasks, "任务")?;
    tasks
        .get(task_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("任务不存在"))
}

pub(super) fn ensure_retryable(task: &TranslationTask) -> Result<(), AppError> {
    if matches!(task.status, TaskStatus::Processing | TaskStatus::Pending) {
        Err(AppError::conflict("任务仍在运行，无法重试"))
    } else {
        Ok(())
    }
}

pub(super) fn ensure_resumable(task: &TranslationTask) -> Result<(), AppError> {
    if task.status != TaskStatus::Paused {
        Err(AppError::conflict("任务当前不可继续"))
    } else {
        Ok(())
    }
}

pub(super) fn artifact_dir_for_retry(task: &TranslationTask) -> Result<PathBuf, AppError> {
    task.artifact_paths
        .artifact_dir
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::internal("缺少任务工件目录"))
}

pub(super) fn mark_task_requeued(state: &AppState, task_id: &str) -> Result<(), AppError> {
    update_task_entry_with_events(
        &state.tasks,
        &state.task_store_path,
        &state.task_event_sink(),
        task_id,
        |task| {
            state::apply_task_requeued(task);
        },
    )
}

pub(super) fn mark_task_cancelled(state: &AppState, task_id: &str) -> Result<(), AppError> {
    update_task_entry_with_events(
        &state.tasks,
        &state.task_store_path,
        &state.task_event_sink(),
        task_id,
        |task| {
            state::apply_task_cancelled(task);
        },
    )
}

pub(super) fn mark_task_paused(state: &AppState, task_id: &str) -> Result<(), AppError> {
    update_task_entry_with_events(
        &state.tasks,
        &state.task_store_path,
        &state.task_event_sink(),
        task_id,
        |task| {
            state::apply_task_paused(task);
        },
    )
}

pub(super) fn mark_task_start_failed(
    state: &AppState,
    task_id: &str,
    error: &AppError,
    retry_count: u8,
) -> Result<(), AppError> {
    update_task_entry_with_events(
        &state.tasks,
        &state.task_store_path,
        &state.task_event_sink(),
        task_id,
        |task| {
            state::apply_task_failure(task, error, retry_count);
        },
    )
}

fn filename_or_unknown(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown.pdf")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::RuntimeConfig;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Semaphore;

    fn sample_task(task_id: &str) -> TranslationTask {
        TranslationTask {
            id: task_id.to_string(),
            filename: "demo.epub".to_string(),
            pdf_path: "D:/demo.epub".to_string(),
            status: TaskStatus::Failed,
            phase: TaskPhase::Failed,
            progress: 87,
            message: "provider timeout".to_string(),
            article_type: "fiction".to_string(),
            concurrent: true,
            max_workers: 4,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            output_path: None,
            html_output_path: None,
            cover_path: None,
            artifact_paths: TaskArtifactPaths {
                artifact_dir: Some("D:/artifacts/task-1".to_string()),
                ..TaskArtifactPaths::default()
            },
            total_chunks: 5,
            translated_chunks: 3,
            retry_count: 1,
            source_hash: Some("hash-1".to_string()),
            last_error: Some(crate::commands::tasks::TaskError {
                code: "LLM_TIMEOUT".to_string(),
                message: "provider timeout".to_string(),
                retryable: true,
                retry_after_ms: Some(1500),
            }),
        }
    }

    fn sample_state(root: &Path, task: TranslationTask) -> AppState {
        AppState {
            tasks: Arc::new(Mutex::new(HashMap::from([(task.id.clone(), task)]))),
            config: Arc::new(Mutex::new(RuntimeConfig::default())),
            reader_states: Arc::new(Mutex::new(HashMap::new())),
            book_profiles: Arc::new(Mutex::new(HashMap::new())),
            task_store_path: root.join("tasks.json"),
            reader_state_store_path: root.join("reader_states.json"),
            book_profile_store_path: root.join("book_profiles.json"),
            runtime_config_store_path: root.join("runtime_config.json"),
            import_root_dir: root.join("imports"),
            artifact_root_dir: root.join("artifacts"),
            skill_library_root_dir: root.join("skills"),
            wordlists_dir: root.join("wordlists"),
            llm_limiter: Arc::new(Semaphore::new(1)),
            nonfatal_notices: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            controls: Arc::new(Mutex::new(HashMap::new())),
            task_event_sink: None,
        }
    }

    #[test]
    fn ensure_retryable_rejects_running_tasks_and_allows_terminal_tasks() {
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Pending;
        let pending_error = ensure_retryable(&task).expect_err("pending task should not retry");
        assert_eq!(pending_error.code, "CONFLICT");

        task.status = TaskStatus::Processing;
        let running_error = ensure_retryable(&task).expect_err("processing task should not retry");
        assert_eq!(running_error.code, "CONFLICT");

        task.status = TaskStatus::Failed;
        ensure_retryable(&task).expect("failed task should retry");

        task.status = TaskStatus::Completed;
        ensure_retryable(&task).expect("completed task can be rerun manually");
    }

    #[test]
    fn ensure_resumable_accepts_only_paused_tasks() {
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Paused;
        ensure_resumable(&task).expect("paused task should resume");

        task.status = TaskStatus::Failed;
        let error = ensure_resumable(&task).expect_err("failed task should not resume directly");
        assert_eq!(error.code, "CONFLICT");
    }

    #[test]
    fn artifact_dir_for_retry_requires_existing_artifact_dir() {
        let task = sample_task("task-1");
        let artifact_dir = artifact_dir_for_retry(&task).expect("artifact dir should exist");
        assert_eq!(artifact_dir, PathBuf::from("D:/artifacts/task-1"));

        let mut missing_dir_task = sample_task("task-2");
        missing_dir_task.artifact_paths.artifact_dir = None;
        let error = artifact_dir_for_retry(&missing_dir_task)
            .expect_err("missing artifact dir should fail");
        assert_eq!(error.code, "INTERNAL");
    }

    #[test]
    fn mark_task_requeued_resets_runtime_state_and_preserves_artifact_dir() {
        let unique = format!(
            "musetranslate-lifecycle-retry-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let state = sample_state(&root, sample_task("task-1"));

        mark_task_requeued(&state, "task-1").expect("task should requeue");

        let tasks = state
            .tasks
            .lock()
            .expect("task mutex should remain healthy");
        let task = tasks.get("task-1").expect("task should still exist");
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.phase, TaskPhase::Imported);
        assert_eq!(task.progress, 0);
        assert_eq!(task.message, "任务重新排队");
        assert_eq!(task.retry_count, 2);
        assert!(task.last_error.is_none());
        assert_eq!(
            task.artifact_paths.artifact_dir.as_deref(),
            Some("D:/artifacts/task-1")
        );

        let persisted =
            fs::read_to_string(&state.task_store_path).expect("task store should be updated");
        assert!(persisted.contains("\"status\": \"pending\""));
        assert!(persisted.contains("\"phase\": \"imported\""));
        assert!(persisted.contains("\"retryCount\": 2"));
        assert!(persisted.contains("\"lastError\": null"));
        assert!(persisted.contains("\"artifactDir\": \"D:/artifacts/task-1\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mark_task_cancelled_sets_retryable_cancelled_error_contract() {
        let unique = format!(
            "musetranslate-lifecycle-cancel-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Processing;
        task.phase = TaskPhase::Translating;
        let state = sample_state(&root, task);

        mark_task_cancelled(&state, "task-1").expect("task should cancel");

        let tasks = state
            .tasks
            .lock()
            .expect("task mutex should remain healthy");
        let task = tasks.get("task-1").expect("task should still exist");
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.phase, TaskPhase::Cancelled);
        assert_eq!(
            task.artifact_paths.artifact_dir.as_deref(),
            Some("D:/artifacts/task-1")
        );
        assert_eq!(task.message, "任务已取消");
        let error = task
            .last_error
            .as_ref()
            .expect("cancelled task should carry last error");
        assert_eq!(error.code, "CANCELLED");
        assert_eq!(error.message, "任务已取消");
        assert!(error.retryable);
        assert_eq!(error.retry_after_ms, None);

        let persisted =
            fs::read_to_string(&state.task_store_path).expect("task store should be updated");
        assert!(persisted.contains("\"phase\": \"cancelled\""));
        assert!(persisted.contains("\"code\": \"CANCELLED\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mark_task_paused_sets_resumeable_paused_error_contract() {
        let unique = format!(
            "musetranslate-lifecycle-pause-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Processing;
        task.phase = TaskPhase::Translating;
        let state = sample_state(&root, task);

        mark_task_paused(&state, "task-1").expect("task should pause");

        let tasks = state
            .tasks
            .lock()
            .expect("task mutex should remain healthy");
        let task = tasks.get("task-1").expect("task should still exist");
        assert_eq!(task.status, TaskStatus::Paused);
        assert_eq!(task.phase, TaskPhase::Paused);
        assert_eq!(task.message, "任务已暂停，可继续");
        let error = task
            .last_error
            .as_ref()
            .expect("paused task should carry last error");
        assert_eq!(error.code, "PAUSED");
        assert!(error.retryable);

        let persisted =
            fs::read_to_string(&state.task_store_path).expect("task store should be updated");
        assert!(persisted.contains("\"status\": \"paused\""));
        assert!(persisted.contains("\"phase\": \"paused\""));
        assert!(persisted.contains("\"code\": \"PAUSED\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mark_task_start_failed_replaces_pending_state_with_structured_error() {
        let unique = format!(
            "musetranslate-lifecycle-start-failed-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let mut task = sample_task("task-1");
        task.status = TaskStatus::Pending;
        task.phase = TaskPhase::Imported;
        task.progress = 0;
        task.message = "任务重新排队".to_string();
        task.last_error = None;
        let state = sample_state(&root, task);
        let error = AppError::internal("runtime config unavailable");

        mark_task_start_failed(&state, "task-1", &error, 2).expect("task should fail to start");

        let tasks = state
            .tasks
            .lock()
            .expect("task mutex should remain healthy");
        let task = tasks.get("task-1").expect("task should still exist");
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.phase, TaskPhase::Failed);
        assert_eq!(task.retry_count, 2);
        assert_eq!(
            task.artifact_paths.artifact_dir.as_deref(),
            Some("D:/artifacts/task-1")
        );
        let last_error = task
            .last_error
            .as_ref()
            .expect("failed task should expose error");
        assert_eq!(last_error.code, "INTERNAL");
        assert_eq!(last_error.message, "runtime config unavailable");
        assert!(last_error.retryable);

        let persisted =
            fs::read_to_string(&state.task_store_path).expect("task store should be updated");
        assert!(persisted.contains("\"status\": \"failed\""));
        assert!(persisted.contains("\"code\": \"INTERNAL\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persist_new_task_rolls_back_in_memory_insert_when_store_write_fails() {
        let unique = format!(
            "musetranslate-lifecycle-persist-new-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let state = sample_state(&root, sample_task("task-1"));
        fs::create_dir_all(&state.task_store_path)
            .expect("task store path should become a directory");

        let new_task = sample_task("task-2");
        let error =
            persist_new_task(&state, new_task).expect_err("persist failure should roll back");
        assert_eq!(error.code, "INTERNAL");

        let tasks = state
            .tasks
            .lock()
            .expect("task mutex should remain healthy");
        assert!(tasks.contains_key("task-1"));
        assert!(!tasks.contains_key("task-2"));

        let _ = fs::remove_dir_all(root);
    }
}
