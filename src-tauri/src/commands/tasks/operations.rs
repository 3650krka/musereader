use super::lifecycle::{
    artifact_dir_for_retry, ensure_resumable, ensure_retryable, mark_task_requeued,
    next_manual_epub_task_id, persist_reader_states, persist_tasks,
};
use super::runner::{build_translation_pipeline, spawn_translation_job};
use super::runner_controls::register_spawned_task_control;
use super::{
    build_pending_task, cancel_task_control, create_artifact_dir, ensure_document_path,
    ensure_epub_path, handle_task_start_failure, load_translation_prompt_bundle_with_core, lock_mutex,
    persist_new_task, report_task_nonfatal_error, run_with_artifact_cleanup,
    supported_article_types, task_snapshot, validate_article_type, validate_max_workers, AppError,
    AppState, ChapterTranslationRunResult, TaskStatus, TranslationPromptBundle, TranslationTask,
};
use crate::pipeline::{PipelineCacheConfig, PipelineRunConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MIN_WORKERS: u8 = 1;

/// 按运行配置的 inject_core_skill 加载翻译 prompt bundle（全景翻译 skill 可选注入）。
fn translation_bundle_for(state: &AppState, article_type: &str) -> Result<TranslationPromptBundle, AppError> {
    let inject_core_skill = lock_mutex(&state.config, "运行配置")?.inject_core_skill;
    load_translation_prompt_bundle_with_core(
        &state.skill_library_root_dir,
        article_type,
        inject_core_skill,
    )
}
const MAX_WORKERS: u8 = 32;
const EPUB_CHAPTER_BENCHMARK_WORKERS: usize = 10;
#[allow(clippy::too_many_arguments)]
fn handoff_persisted_task_to_worker(
    state: &AppState,
    task_id: String,
    pdf_path: PathBuf,
    article_type: String,
    concurrent: bool,
    max_workers: u8,
    prompt_bundle: TranslationPromptBundle,
    artifact_dir: PathBuf,
    retry_count: u8,
    glossary_deck_ids: Option<Vec<String>>,
) -> Result<(), AppError> {
    match start_job(
        state,
        task_id.clone(),
        pdf_path,
        article_type,
        concurrent,
        max_workers,
        prompt_bundle,
        artifact_dir,
        retry_count,
        glossary_deck_ids,
    ) {
        Ok(()) => Ok(()),
        Err(error) => handle_task_start_failure(
            state,
            &task_id,
            error,
            retry_count,
            "mark_task_start_failed_after_handoff",
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_task_with_prompt_and_start_worker(
    state: &AppState,
    task_id: String,
    pdf_path: PathBuf,
    article_type: String,
    concurrent: bool,
    max_workers: u8,
    artifact_dir: PathBuf,
    prompt_bundle: TranslationPromptBundle,
    retry_count: u8,
    glossary_deck_ids: Option<Vec<String>>,
) -> Result<(), AppError> {
    persist_new_task(
        state,
        build_pending_task(
            &task_id,
            &pdf_path,
            &article_type,
            concurrent,
            max_workers,
            &artifact_dir,
        ),
    )?;
    handoff_persisted_task_to_worker(
        state,
        task_id,
        pdf_path,
        article_type,
        concurrent,
        max_workers,
        prompt_bundle,
        artifact_dir,
        retry_count,
        glossary_deck_ids,
    )
}

#[allow(clippy::too_many_arguments)]
fn start_job(
    state: &AppState,
    task_id: String,
    path: PathBuf,
    article_type: String,
    concurrent: bool,
    max_workers: u8,
    prompt_bundle: TranslationPromptBundle,
    artifact_dir: PathBuf,
    retry_count: u8,
    glossary_deck_ids: Option<Vec<String>>,
) -> Result<(), AppError> {
    let cancellation = CancellationToken::new();
    let stop_reason = Arc::new(Mutex::new(None));
    let runtime_config = lock_mutex(&state.config, "运行配置")?.clone();
    // 术语注入：用户词库优先（deck_ids=None 全部启用词表 / Some 仅选中启用词表）
    let static_glossary_entries = crate::commands::resolve_task_static_entries(
        state,
        glossary_deck_ids,
    )?
    .into_iter()
    .map(|entry| crate::pipeline::StaticGlossaryEntry {
        source: entry.source,
        target: entry.target,
        scope: entry.scope,
        enforcement: entry.enforcement,
        notes: entry.notes,
    })
    .collect::<Vec<_>>();
    let handle = spawn_translation_job(
        task_id.clone(),
        path,
        article_type,
        concurrent,
        max_workers,
        runtime_config,
        static_glossary_entries,
        prompt_bundle,
        artifact_dir,
        Arc::clone(&state.tasks),
        state.task_store_path.clone(),
        Arc::clone(&state.llm_limiter),
        Arc::clone(&state.nonfatal_notices),
        Arc::clone(&state.controls),
        state.task_event_sink(),
        cancellation.clone(),
        Arc::clone(&stop_reason),
        retry_count,
    );
    register_spawned_task_control(&state.controls, &task_id, cancellation, stop_reason, handle)
}

pub(super) fn start_translation_inner(
    state: &AppState,
    pdf_path: &str,
    article_type: &str,
    concurrent: bool,
    max_workers: u8,
    glossary_deck_ids: Option<Vec<String>>,
) -> Result<String, AppError> {
    let validated_pdf_path = ensure_document_path(pdf_path)?;
    let normalized_article_type = validate_article_type(article_type, supported_article_types())?;
    let normalized_workers = validate_max_workers(max_workers, MIN_WORKERS, MAX_WORKERS)?;
    let task_id = Uuid::new_v4().to_string();
    let artifact_dir = create_artifact_dir(&state.artifact_root_dir, &task_id)?;
    let prompt_bundle = run_with_artifact_cleanup(&state.artifact_root_dir, &artifact_dir, || {
        translation_bundle_for(state, &normalized_article_type)
    })?;
    run_with_artifact_cleanup(&state.artifact_root_dir, &artifact_dir, || {
        persist_task_with_prompt_and_start_worker(
            state,
            task_id.clone(),
            validated_pdf_path.clone(),
            normalized_article_type.clone(),
            concurrent,
            normalized_workers,
            artifact_dir.clone(),
            prompt_bundle.clone(),
            0,
            glossary_deck_ids,
        )
    })?;
    Ok(task_id)
}

pub(super) async fn translate_epub_chapter_once_inner(
    state: &AppState,
    epub_path: &str,
    chapter_title: &str,
    article_type: &str,
) -> Result<ChapterTranslationRunResult, AppError> {
    let validated_epub_path = ensure_epub_path(epub_path)?;
    let normalized_article_type = validate_article_type(article_type, supported_article_types())?;
    let task_id = next_manual_epub_task_id();
    let artifact_dir = create_artifact_dir(&state.artifact_root_dir, &task_id)?;
    let runtime_config =
        run_with_artifact_cleanup(&state.artifact_root_dir, &artifact_dir, || {
            lock_mutex(&state.config, "运行配置").map(|config| config.clone())
        })?;
    let prompt_bundle = run_with_artifact_cleanup(&state.artifact_root_dir, &artifact_dir, || {
        translation_bundle_for(state, &normalized_article_type)
    })?;
    // 与主任务一致：号池优先，回落单 provider + fallback 路由
    let pipeline = build_translation_pipeline(&runtime_config);

    let result = pipeline
        .process_document(
            PipelineRunConfig {
                task_id: task_id.clone(),
                pdf_path: validated_epub_path,
                article_type: normalized_article_type,
                system_prompt: prompt_bundle.system_prompt,
                system_prompt_hash: prompt_bundle.system_prompt_hash,
                skill_ids: prompt_bundle.skill_ids,
                epub_chapter_title: Some(chapter_title.to_string()),
                chunk_size: super::resolve_task_chunk_size(&runtime_config),
                max_parallel: EPUB_CHAPTER_BENCHMARK_WORKERS,
                artifact_dir: artifact_dir.clone(),
                cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir)
                    .with_ocr_cache(runtime_config.use_ocr_cache)
                    .with_front_matter_cache(runtime_config.use_front_matter_cache),
                // 与主任务一致：用户词库优先（全部启用词表），空词库回落老全局词表
                static_glossary_entries: crate::commands::resolve_task_static_entries(state, None)?
                    .into_iter()
                    .map(|entry| crate::pipeline::StaticGlossaryEntry {
                        source: entry.source,
                        target: entry.target,
                        scope: entry.scope,
                        enforcement: entry.enforcement,
                        notes: entry.notes,
                    })
                    .collect(),
                global_llm_limiter: Arc::clone(&state.llm_limiter),
                cancellation_token: CancellationToken::new(),
            },
            |_| {},
        )
        .await?;

    Ok(ChapterTranslationRunResult {
        task_id,
        source_markdown_path: result.source_markdown_path,
        translated_markdown_path: result.output_markdown_path,
        translated_html_path: result.output_html_path,
        manifest_path: result.artifact_manifest_path,
        validation_report_path: result.validation_report_path,
        translated_chunks: result.translated_chunks,
        total_chunks: result.total_chunks,
    })
}

pub(super) fn retry_translation_task_inner(
    state: &AppState,
    task_id: &str,
) -> Result<(), AppError> {
    let task_snapshot = task_snapshot(state, task_id)?;
    ensure_retryable(&task_snapshot)?;
    let pdf_path = ensure_document_path(&task_snapshot.pdf_path)?;
    let artifact_dir = artifact_dir_for_retry(&task_snapshot)?;
    let retry_count = task_snapshot.retry_count.saturating_add(1);

    mark_task_requeued(state, task_id)?;
    match translation_bundle_for(state, &task_snapshot.article_type)
    {
        Ok(prompt_bundle) => handoff_persisted_task_to_worker(
            state,
            task_id.to_string(),
            pdf_path,
            task_snapshot.article_type,
            task_snapshot.concurrent,
            task_snapshot.max_workers,
            prompt_bundle,
            artifact_dir,
            retry_count,
            None,
        ),
        Err(error) => handle_task_start_failure(
            state,
            task_id,
            error,
            retry_count,
            "mark_task_start_failed_after_prompt_load",
        ),
    }
}

pub(super) fn resume_translation_task_inner(
    state: &AppState,
    task_id: &str,
) -> Result<(), AppError> {
    let task_snapshot = task_snapshot(state, task_id)?;
    ensure_resumable(&task_snapshot)?;
    let pdf_path = ensure_document_path(&task_snapshot.pdf_path)?;
    let artifact_dir = artifact_dir_for_retry(&task_snapshot)?;
    let retry_count = task_snapshot.retry_count;

    mark_task_requeued(state, task_id)?;
    match translation_bundle_for(state, &task_snapshot.article_type)
    {
        Ok(prompt_bundle) => handoff_persisted_task_to_worker(
            state,
            task_id.to_string(),
            pdf_path,
            task_snapshot.article_type,
            task_snapshot.concurrent,
            task_snapshot.max_workers,
            prompt_bundle,
            artifact_dir,
            retry_count,
            None,
        ),
        Err(error) => handle_task_start_failure(
            state,
            task_id,
            error,
            retry_count,
            "mark_task_start_failed_after_resume_prompt_load",
        ),
    }
}

pub(super) fn prepare_task_delete(state: &AppState, task_id: &str) -> Result<(), AppError> {
    let task = task_snapshot(state, task_id)?;
    let cancelled = cancel_task_control(&state.controls, task_id)?;
    if matches!(task.status, TaskStatus::Pending | TaskStatus::Processing) && !cancelled {
        return Err(AppError::conflict("任务仍在运行，无法删除"));
    }
    Ok(())
}

pub(super) fn delete_task_state_and_persist(
    state: &AppState,
    task_id: &str,
) -> Result<Option<TranslationTask>, AppError> {
    let mut tasks = lock_mutex(&state.tasks, "任务")?;
    let mut reader_states = lock_mutex(&state.reader_states, "阅读状态")?;
    let original_tasks = tasks.clone();
    let original_reader_states = reader_states.clone();

    let removed_task = tasks.remove(task_id);
    if let Some(task) = removed_task.as_ref() {
        reader_states.remove(&task.id);
    }

    if let Err(error) = persist_tasks(state, &tasks) {
        *tasks = original_tasks;
        *reader_states = original_reader_states;
        return Err(error);
    }
    if let Err(error) = persist_reader_states(state, &reader_states) {
        *tasks = original_tasks.clone();
        *reader_states = original_reader_states.clone();
        if let Err(restore_error) = persist_tasks(state, &original_tasks) {
            report_task_nonfatal_error(
                state,
                "restore_tasks_after_delete_failure",
                task_id,
                &restore_error,
            );
        }
        if let Err(restore_error) = persist_reader_states(state, &original_reader_states) {
            report_task_nonfatal_error(
                state,
                "restore_reader_states_after_delete_failure",
                task_id,
                &restore_error,
            );
        }
        return Err(error);
    }

    // 删除事件：任务本体已移除，仅推送 id（前端据其移除本地条目）。
    if removed_task.is_some() {
        super::events::emit_task_deleted(&state.task_event_sink(), task_id);
    }
    Ok(removed_task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tasks::{TaskControl, TaskError, TaskPhase};
    use crate::commands::{ReaderState, RuntimeConfig};
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tokio::runtime::Runtime;
    use tokio::sync::Semaphore;
    use tokio::time::{sleep, Duration};

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
            artifact_paths: super::super::TaskArtifactPaths::default(),
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
    fn delete_task_state_and_persist_restores_memory_when_reader_state_write_fails() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-delete-task-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let reader_state_store_path = root.join("reader_states.json");
        let state = sample_state(
            &root,
            task_store_path.clone(),
            reader_state_store_path.clone(),
        );

        persist_tasks(
            &state,
            &lock_mutex(&state.tasks, "任务").expect("task lock").clone(),
        )
        .expect("task store should persist");
        persist_reader_states(
            &state,
            &lock_mutex(&state.reader_states, "阅读状态")
                .expect("reader lock")
                .clone(),
        )
        .expect("reader store should persist");
        fs::remove_file(&reader_state_store_path).expect("reader state file should be removable");
        fs::create_dir_all(&reader_state_store_path).expect("reader state path should become dir");

        let error = delete_task_state_and_persist(&state, "task-1")
            .expect_err("reader state persist failure should roll back");

        assert_eq!(error.code, "INTERNAL");
        let tasks = lock_mutex(&state.tasks, "任务").expect("task lock");
        assert!(tasks.contains_key("task-1"));
        drop(tasks);
        let reader_states = lock_mutex(&state.reader_states, "阅读状态").expect("reader lock");
        assert!(reader_states.contains_key("task-1"));
        drop(reader_states);

        let persisted_tasks =
            fs::read_to_string(&task_store_path).expect("task store should be restored");
        assert!(persisted_tasks.contains("\"task-1\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_task_delete_rejects_running_task_without_control() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-delete-prepare-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let reader_state_store_path = root.join("reader_states.json");
        let state = sample_state(&root, task_store_path, reader_state_store_path);
        {
            let mut tasks = lock_mutex(&state.tasks, "任务").expect("task lock");
            let task = tasks.get_mut("task-1").expect("task should exist");
            task.status = TaskStatus::Processing;
            task.phase = TaskPhase::Translating;
        }

        let error = prepare_task_delete(&state, "task-1")
            .expect_err("running task without control should not delete");

        assert_eq!(error.code, "CONFLICT");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_task_delete_allows_terminal_task_without_control() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-delete-prepare-terminal-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let reader_state_store_path = root.join("reader_states.json");
        let state = sample_state(&root, task_store_path, reader_state_store_path);

        prepare_task_delete(&state, "task-1")
            .expect("failed task without control should be deletable");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_task_delete_allows_running_task_when_control_exists() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-delete-prepare-running-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let reader_state_store_path = root.join("reader_states.json");
        let state = sample_state(&root, task_store_path, reader_state_store_path);
        {
            let mut tasks = lock_mutex(&state.tasks, "任务").expect("task lock");
            let task = tasks.get_mut("task-1").expect("task should exist");
            task.status = TaskStatus::Processing;
            task.phase = TaskPhase::Translating;
        }

        let runtime = Runtime::new().expect("tokio runtime should build");
        let cancellation = CancellationToken::new();
        let handle = runtime.spawn(async {
            sleep(Duration::from_millis(10)).await;
        });
        {
            let mut controls = lock_mutex(&state.controls, "任务控制").expect("controls lock");
            controls.insert(
                "task-1".to_string(),
                TaskControl {
                    cancellation: cancellation.clone(),
                    stop_reason: Arc::new(Mutex::new(None)),
                    _handle: handle,
                },
            );
        }

        prepare_task_delete(&state, "task-1")
            .expect("running task with control should enter delete preparation");
        assert!(cancellation.is_cancelled());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn retry_translation_task_restores_failed_state_when_prompt_loading_fails() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-retry-requeue-failure-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let reader_state_store_path = root.join("reader_states.json");
        let state = sample_state(&root, task_store_path, reader_state_store_path);
        let epub_path = root.join("demo.epub");
        let artifact_dir = root.join("artifacts").join("task-1");
        fs::create_dir_all(&artifact_dir).expect("artifact dir should exist");
        fs::write(&epub_path, b"dummy epub").expect("epub fixture should exist");
        fs::write(&state.skill_library_root_dir, b"not a directory")
            .expect("skill library path should become a file");
        {
            let mut tasks = lock_mutex(&state.tasks, "任务").expect("task lock");
            let task = tasks.get_mut("task-1").expect("task should exist");
            task.pdf_path = epub_path.to_string_lossy().to_string();
            task.artifact_paths.artifact_dir = Some(artifact_dir.to_string_lossy().to_string());
            task.retry_count = 0;
            task.last_error = Some(TaskError {
                code: "PREVIOUS".to_string(),
                message: "stale failure".to_string(),
                retryable: true,
                retry_after_ms: None,
            });
        }

        let error = retry_translation_task_inner(&state, "task-1")
            .expect_err("prompt loading failure should bubble up");

        assert_eq!(error.code, "INTERNAL");
        let tasks = lock_mutex(&state.tasks, "任务").expect("task lock");
        let task = tasks.get("task-1").expect("task should still exist");
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.phase, TaskPhase::Failed);
        assert_eq!(task.retry_count, 1);
        assert_eq!(
            task.artifact_paths.artifact_dir.as_deref(),
            Some(artifact_dir.to_string_lossy().as_ref())
        );
        assert!(task.message.starts_with("创建 skill 目录失败:"));
        let last_error = task
            .last_error
            .as_ref()
            .expect("retry failure should be persisted");
        assert_eq!(last_error.code, "INTERNAL");
        assert!(last_error.message.starts_with("创建 skill 目录失败:"));
        assert!(last_error.retryable);

        let _ = fs::remove_dir_all(root);
    }
}
