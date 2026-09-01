use super::super::NonfatalNoticeBuffer;
use super::events::TaskEventSink;
use super::runner_controls::remove_task_control;
use super::runner_state::{
    apply_job_result_with_events, report_task_runner_nonfatal_error,
    update_running_task_with_events,
};
use super::types::TaskStopReason;
use super::{TaskControl, TranslationPromptBundle, TranslationTask};
use crate::llm::LlmFallbackRoute;
use crate::pipeline::{PipelineCacheConfig, PipelineRunConfig, TranslationPipeline};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_translation_job(
    task_id: String,
    pdf_path: PathBuf,
    article_type: String,
    concurrent: bool,
    max_workers: u8,
    runtime_config: super::super::RuntimeConfig,
    static_glossary_entries: Vec<crate::pipeline::StaticGlossaryEntry>,
    prompt_bundle: TranslationPromptBundle,
    artifact_dir: PathBuf,
    tasks: Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: PathBuf,
    llm_limiter: Arc<Semaphore>,
    nonfatal_notices: NonfatalNoticeBuffer,
    controls: Arc<Mutex<HashMap<String, TaskControl>>>,
    task_events: Option<TaskEventSink>,
    cancellation: CancellationToken,
    stop_reason: Arc<Mutex<Option<TaskStopReason>>>,
    retry_count: u8,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let pipeline = build_translation_pipeline(&runtime_config);
        run_translation_job(
            pipeline,
            task_id,
            pdf_path,
            article_type,
            concurrent,
            max_workers,
            runtime_config,
            static_glossary_entries,
            prompt_bundle,
            artifact_dir,
            tasks,
            task_store_path,
            llm_limiter,
            nonfatal_notices,
            controls,
            task_events,
            cancellation,
            stop_reason,
            retry_count,
        )
        .await;
    })
}

/// 翻译 pipeline 构建：号池优先（有活跃池→跨供应商并行调度），
/// 否则单 provider + fallback 路由（向后兼容）。主任务与单章翻译共用。
pub(super) fn build_translation_pipeline(
    runtime_config: &super::super::RuntimeConfig,
) -> TranslationPipeline {
    // 默认协议：custom 供应商按 URL 匹配回填其显式协议；其余 Auto。
    let llm_protocol = resolve_custom_provider_protocol(runtime_config);
    let llm_api_url = runtime_config.llm_api_url.clone();
    let llm_api_key = runtime_config.llm_api_key.clone();
    let llm_model = runtime_config.llm_model.clone();
    let ocr_api_url = runtime_config.ocr_api_url.clone();
    let ocr_token = runtime_config.ocr_token.clone();
    if let Some(pool) = resolve_active_route_pool(runtime_config) {
        TranslationPipeline::new_with_route_pool(
            ocr_api_url,
            ocr_token,
            llm_api_url,
            llm_api_key,
            llm_model,
            pool,
        )
        .with_llm_protocol(llm_protocol)
    } else {
        let llm_fallback_routes = build_llm_fallback_routes(runtime_config);
        TranslationPipeline::new_with_llm_fallback_routes(
            ocr_api_url,
            ocr_token,
            llm_api_url,
            llm_api_key,
            llm_model,
            llm_fallback_routes,
        )
        .with_llm_protocol(llm_protocol)
    }
}

/// 当前 llm_api_url 命中的自定义供应商协议：
/// 前端「使用此供应商」只复制 url/key/model，protocol 留在 CustomProvider——
/// 这里按 URL 匹配回填，保证显式协议真正进入请求层。
fn resolve_custom_provider_protocol(
    runtime_config: &super::super::RuntimeConfig,
) -> crate::llm::RequestProtocol {
    let url = runtime_config.llm_api_url.trim();
    if url.is_empty() {
        return crate::llm::RequestProtocol::Auto;
    }
    runtime_config
        .custom_providers
        .iter()
        .find(|provider| provider.api_url.trim() == url)
        .map(|provider| crate::llm::RequestProtocol::from_label(&provider.protocol))
        .unwrap_or(crate::llm::RequestProtocol::Auto)
}

fn build_llm_fallback_routes(
    runtime_config: &super::super::RuntimeConfig,
) -> Vec<LlmFallbackRoute> {
    let mut routes = Vec::new();
    if runtime_config
        .llm_provider
        .trim()
        .eq_ignore_ascii_case("nvidia")
    {
        return routes;
    }
    if runtime_config.nvidia_api_key.trim().is_empty()
        || runtime_config.nvidia_model.trim().is_empty()
        || runtime_config.nvidia_api_url.trim().is_empty()
    {
        return routes;
    }
    routes.push(LlmFallbackRoute {
        api_url: runtime_config.nvidia_api_url.clone(),
        api_key: runtime_config.nvidia_api_key.clone(),
        model: runtime_config.nvidia_model.clone(),
    });
    routes
}

// 解析当前活跃号池（L2）：route_pools 非空且能匹配 active_pool（或取首池）时返回该池。
// 无可用池返回 None，调用方回落单 provider 行为。
fn resolve_active_route_pool(
    runtime_config: &super::super::RuntimeConfig,
) -> Option<&super::super::RoutePoolConfig> {
    if runtime_config.route_pools.is_empty() {
        return None;
    }
    let active = runtime_config.active_pool.trim();
    if !active.is_empty() {
        if let Some(pool) = runtime_config
            .route_pools
            .iter()
            .find(|pool| pool.name == active)
        {
            return Some(pool);
        }
    }
    runtime_config.route_pools.first()
}

#[allow(clippy::too_many_arguments)]
async fn run_translation_job(
    pipeline: TranslationPipeline,
    task_id: String,
    pdf_path: PathBuf,
    article_type: String,
    concurrent: bool,
    max_workers: u8,
    runtime_config: super::super::RuntimeConfig,
    static_glossary_entries: Vec<crate::pipeline::StaticGlossaryEntry>,
    prompt_bundle: TranslationPromptBundle,
    artifact_dir: PathBuf,
    tasks: Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: PathBuf,
    llm_limiter: Arc<Semaphore>,
    nonfatal_notices: NonfatalNoticeBuffer,
    controls: Arc<Mutex<HashMap<String, TaskControl>>>,
    task_events: Option<TaskEventSink>,
    cancellation: CancellationToken,
    stop_reason: Arc<Mutex<Option<TaskStopReason>>>,
    retry_count: u8,
) {
    let run_config = PipelineRunConfig {
        task_id: task_id.clone(),
        pdf_path,
        article_type,
        system_prompt: prompt_bundle.system_prompt,
        system_prompt_hash: prompt_bundle.system_prompt_hash,
        skill_ids: prompt_bundle.skill_ids,
        epub_chapter_title: None,
        chunk_size: super::resolve_task_chunk_size(&runtime_config),
        max_parallel: if concurrent {
            usize::from(max_workers.max(1))
        } else {
            1
        },
        artifact_dir: artifact_dir.clone(),
        cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir)
            .with_ocr_cache(runtime_config.use_ocr_cache)
            .with_front_matter_cache(runtime_config.use_front_matter_cache),
        static_glossary_entries,
        global_llm_limiter: llm_limiter,
        cancellation_token: cancellation,
    };

    let result = pipeline
        .process_document(run_config, |update| {
            update_running_task_with_events(
                &tasks,
                &task_store_path,
                &nonfatal_notices,
                &task_events,
                &task_id,
                &update,
                retry_count,
            );
        })
        .await;
    let stop_reason = super::lock_mutex(&stop_reason, "任务停止原因")
        .ok()
        .and_then(|reason| *reason);
    apply_job_result_with_events(
        &tasks,
        &task_store_path,
        &nonfatal_notices,
        &task_events,
        &task_id,
        &artifact_dir,
        result,
        retry_count,
        stop_reason,
    );
    if let Err(error) = remove_task_control(&controls, &task_id) {
        report_task_runner_nonfatal_error(
            &nonfatal_notices,
            "remove_task_control_after_job",
            &task_id,
            &error,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tasks::runner_controls::register_spawned_task_control;
    use crate::commands::tasks::runner_state::{update_running_task, update_task_entry};
    use crate::commands::tasks::{TaskArtifactPaths, TaskPhase, TaskStatus};
    use crate::error::AppError;
    use crate::pipeline::PipelineProgressStage;
    use std::collections::HashMap;
    use std::fs;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::{Arc, Mutex};
    use tokio::runtime::Runtime;
    use tokio::time::{sleep, Duration};

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
    fn phase_mapping_uses_explicit_pipeline_stage() {
        assert_eq!(
            super::super::state::phase_from_pipeline_stage(PipelineProgressStage::OcrRunning),
            TaskPhase::OcrRunning
        );
    }

    #[test]
    fn phase_mapping_tracks_expected_pipeline_stages() {
        assert_eq!(
            super::super::state::phase_from_pipeline_stage(PipelineProgressStage::Chunking),
            TaskPhase::Chunking
        );
        assert_eq!(
            super::super::state::phase_from_pipeline_stage(PipelineProgressStage::Translating),
            TaskPhase::Translating
        );
        assert_eq!(
            super::super::state::phase_from_pipeline_stage(PipelineProgressStage::WritingArtifacts),
            TaskPhase::WritingArtifacts
        );
    }

    #[test]
    fn failed_phase_distinguishes_cancelled_from_failed() {
        assert_eq!(
            super::super::state::failed_phase(
                &AppError::cancelled("stop requested"),
                Some(TaskStopReason::Cancel),
            ),
            TaskPhase::Cancelled
        );
        assert_eq!(
            super::super::state::failed_phase(&AppError::internal("boom"), None),
            TaskPhase::Failed
        );
    }

    #[test]
    fn apply_pipeline_result_populates_artifact_contract() {
        let mut task = sample_task("task-1");
        let result = crate::pipeline::PipelineResult {
            translated_markdown: "译文".to_string(),
            translated_html: "<p>译文</p>".to_string(),
            source_markdown_path: "source.md".to_string(),
            output_markdown_path: "output.md".to_string(),
            output_html_path: "output.html".to_string(),
            translated_epub_path: Some("book_zh.epub".to_string()),
            bilingual_epub_path: Some("book_bilingual.epub".to_string()),
            checkpoint_path: "checkpoint.json".to_string(),
            artifact_manifest_path: "manifest.json".to_string(),
            event_log_path: "events.jsonl".to_string(),
            validation_report_path: "validation.json".to_string(),
            glossary_path: "glossary.json".to_string(),
            metrics_path: "metrics.json".to_string(),
            cover_data_url: Some("cover".to_string()),
            translated_chunks: 5,
            total_chunks: 5,
            failed_chunks: 0,
            failed_chunk_indexes: Vec::new(),
            source_hash: "hash-1".to_string(),
        };

        super::super::state::apply_task_success(
            &mut task,
            &result,
            std::path::Path::new("D:/artifacts/task-1"),
        );

        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.phase, TaskPhase::Completed);
        assert_eq!(task.output_path.as_deref(), Some("output.md"));
        assert_eq!(task.html_output_path.as_deref(), Some("output.html"));
        assert_eq!(
            task.artifact_paths.bilingual_epub_path.as_deref(),
            Some("book_bilingual.epub")
        );
        assert_eq!(
            task.artifact_paths.checkpoint_path.as_deref(),
            Some("checkpoint.json")
        );
        assert_eq!(
            task.artifact_paths.artifact_dir.as_deref(),
            Some("D:/artifacts/task-1")
        );
    }

    #[test]
    fn update_running_task_consumes_explicit_progress_update() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-runner-progress-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let tasks = Arc::new(Mutex::new(HashMap::from([(
            "task-1".to_string(),
            sample_task("task-1"),
        )])));
        update_task_entry(&tasks, &task_store_path, "task-1", |_| {})
            .expect("initial store should persist");
        let nonfatal = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let update = crate::pipeline::PipelineProgressUpdate::new(
            PipelineProgressStage::Rendering,
            "rendering",
            88,
        );

        update_running_task(&tasks, &task_store_path, &nonfatal, "task-1", &update, 2);

        let tasks = tasks.lock().expect("task mutex should remain healthy");
        let task = tasks.get("task-1").expect("task should exist");
        assert_eq!(task.status, TaskStatus::Processing);
        assert_eq!(task.phase, TaskPhase::Rendering);
        assert_eq!(task.progress, 88);
        assert_eq!(task.message, "rendering");
        assert_eq!(task.retry_count, 2);
    }

    #[test]
    fn update_task_entry_persists_mutation_to_store() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-runner-update-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let tasks = Arc::new(Mutex::new(HashMap::from([(
            "task-1".to_string(),
            sample_task("task-1"),
        )])));

        update_task_entry(&tasks, &task_store_path, "task-1", |task| {
            task.message = "updated".to_string();
            task.progress = 42;
        })
        .expect("task update should persist");

        let persisted = fs::read_to_string(&task_store_path).expect("task store should exist");
        assert!(persisted.contains("\"message\": \"updated\""));
        assert!(persisted.contains("\"progress\": 42"));
    }

    #[test]
    fn update_task_entry_reverts_in_memory_mutation_when_persist_fails() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-runner-revert-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let tasks = Arc::new(Mutex::new(HashMap::from([(
            "task-1".to_string(),
            sample_task("task-1"),
        )])));
        fs::create_dir_all(&task_store_path).expect("task store path should become directory");

        let error = update_task_entry(&tasks, &task_store_path, "task-1", |task| {
            task.message = "updated".to_string();
        })
        .expect_err("persist failure should bubble up");

        assert_eq!(error.code, "INTERNAL");
        let tasks = tasks.lock().expect("task mutex should remain healthy");
        let task = tasks.get("task-1").expect("task should exist");
        assert_ne!(task.message, "updated");
    }

    #[test]
    fn update_task_entry_rejects_missing_task() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-runner-missing-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let tasks = Arc::new(Mutex::new(HashMap::new()));

        let error = update_task_entry(&tasks, &task_store_path, "missing", |_| {})
            .expect_err("missing task should fail");

        assert_eq!(error.code, "NOT_FOUND");
    }

    #[test]
    fn register_spawned_task_control_rejects_duplicate_task_id_without_replacing_existing_control()
    {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let controls = Arc::new(Mutex::new(HashMap::new()));

        let first_cancellation = CancellationToken::new();
        let first_handle = runtime.spawn(async {
            sleep(Duration::from_millis(20)).await;
        });
        register_spawned_task_control(
            &controls,
            "task-1",
            first_cancellation.clone(),
            Arc::new(Mutex::new(None)),
            first_handle,
        )
        .expect("first control should register");

        let duplicate_cancellation = CancellationToken::new();
        let duplicate_handle = runtime.spawn(async {
            sleep(Duration::from_millis(20)).await;
        });
        let error = register_spawned_task_control(
            &controls,
            "task-1",
            duplicate_cancellation.clone(),
            Arc::new(Mutex::new(None)),
            duplicate_handle,
        )
        .expect_err("duplicate task control should fail");

        assert_eq!(error.code, "CONFLICT");
        assert!(duplicate_cancellation.is_cancelled());
        assert!(!first_cancellation.is_cancelled());
    }

    #[test]
    fn cancel_task_control_marks_existing_token_cancelled() {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let controls = Arc::new(Mutex::new(HashMap::new()));
        let cancellation = CancellationToken::new();
        let handle = runtime.spawn(async {
            sleep(Duration::from_millis(20)).await;
        });
        register_spawned_task_control(
            &controls,
            "task-1",
            cancellation.clone(),
            Arc::new(Mutex::new(None)),
            handle,
        )
        .expect("control should register");

        let cancelled =
            super::super::cancel_task_control(&controls, "task-1").expect("cancel should succeed");

        assert!(cancelled);
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn pause_task_control_marks_existing_token_cancelled_and_sets_reason() {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let controls = Arc::new(Mutex::new(HashMap::new()));
        let cancellation = CancellationToken::new();
        let stop_reason = Arc::new(Mutex::new(None));
        let handle = runtime.spawn(async {
            sleep(Duration::from_millis(20)).await;
        });
        register_spawned_task_control(
            &controls,
            "task-1",
            cancellation.clone(),
            Arc::clone(&stop_reason),
            handle,
        )
        .expect("control should register");

        let paused =
            super::super::pause_task_control(&controls, "task-1").expect("pause should succeed");

        assert!(paused);
        assert!(cancellation.is_cancelled());
        assert_eq!(
            *stop_reason.lock().expect("stop reason lock"),
            Some(TaskStopReason::Pause)
        );
    }

    #[test]
    fn cancel_task_control_returns_false_when_task_is_missing() {
        let controls = Arc::new(Mutex::new(HashMap::new()));
        let cancelled =
            super::super::cancel_task_control(&controls, "missing").expect("cancel should succeed");
        assert!(!cancelled);
    }

    #[test]
    fn abort_task_control_cancels_and_aborts_registered_handle() {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let controls = Arc::new(Mutex::new(HashMap::new()));
        let cancellation = CancellationToken::new();
        let handle = runtime.spawn(async {
            sleep(Duration::from_secs(10)).await;
        });
        register_spawned_task_control(
            &controls,
            "task-1",
            cancellation.clone(),
            Arc::new(Mutex::new(None)),
            handle,
        )
        .expect("control should register");

        let aborted =
            super::super::abort_task_control(&controls, "task-1").expect("abort should succeed");

        assert!(aborted);
        assert!(cancellation.is_cancelled());
        let guard = controls
            .lock()
            .expect("controls lock should remain healthy");
        assert!(!guard.contains_key("task-1"));
    }

    #[test]
    fn abort_task_control_returns_false_when_task_is_missing() {
        let controls = Arc::new(Mutex::new(HashMap::new()));
        let aborted =
            super::super::abort_task_control(&controls, "missing").expect("abort should succeed");
        assert!(!aborted);
    }

    #[test]
    fn remove_task_control_clears_registered_handle() {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let controls = Arc::new(Mutex::new(HashMap::new()));
        let cancellation = CancellationToken::new();
        let handle = runtime.spawn(async {
            sleep(Duration::from_millis(10)).await;
        });
        register_spawned_task_control(
            &controls,
            "task-1",
            cancellation,
            Arc::new(Mutex::new(None)),
            handle,
        )
        .expect("control should register");

        remove_task_control(&controls, "task-1").expect("remove should succeed");

        let guard = controls
            .lock()
            .expect("controls lock should remain healthy");
        assert!(!guard.contains_key("task-1"));
    }

    #[test]
    fn register_spawned_task_control_aborts_handle_when_control_lock_is_poisoned() {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let controls = Arc::new(Mutex::new(HashMap::<String, TaskControl>::new()));
        let poisoned = Arc::clone(&controls);
        let _ = panic::catch_unwind(AssertUnwindSafe(move || {
            let _guard = poisoned.lock().expect("poison setup lock should succeed");
            panic!("poison controls lock");
        }));

        let cancellation = CancellationToken::new();
        let handle = runtime.spawn(async {
            sleep(Duration::from_secs(10)).await;
        });
        let abort_handle = handle.abort_handle();

        let error = register_spawned_task_control(
            &controls,
            "task-1",
            cancellation.clone(),
            Arc::new(Mutex::new(None)),
            handle,
        )
        .expect_err("poisoned control lock should fail");

        assert_eq!(error.code, "INTERNAL");
        assert!(cancellation.is_cancelled());
        abort_handle.abort();
    }
}
