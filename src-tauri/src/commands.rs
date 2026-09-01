use crate::error::AppError;
mod ai_reader;
mod backup;
mod book_profile;
mod config;
mod document_reader;
mod files;
mod fonts;
mod glossary;
mod notices;
mod provider_test;
mod reader;
mod review;
mod skills;
mod storage;
mod tasks;
mod toc_edit;
mod tts;
mod user_glossaries;
mod vocab;
mod vocab_notebooks;
mod web_capture;
mod words;
pub(crate) mod wordlist;

pub use ai_reader::{ai_assist_paragraph, ai_assist_paragraph_streaming};
pub use backup::{export_backup_zip, restore_backup_zip};
pub use book_profile::{
    list_book_profiles, set_book_favorite, set_book_folder, set_book_tags, BookProfile,
};

#[cfg(test)]
pub use config::PoolRouteConfig;
pub use config::{
    get_runtime_config, list_route_pools, load_model_policies, load_runtime_config,
    load_runtime_env_sources, normalize_llm_api_url, pool_route_add_key, pool_route_remove_key,
    resolve_env_defaults, resolve_runtime_paths,
    save_route_pools, update_runtime_config, ModelPolicyConfig, RoutePoolConfig, RuntimeConfig,
    StaticGlossaryEntry,
};
pub use document_reader::prepare_document_reader;
pub use files::{
    export_book_bundle, import_pdf, import_translated_book, prepare_epub_reader, probe_epub,
    read_bilingual_pairs, read_cover_data_url, read_epub_metadata, read_translated_file,
    search_epub_reader_content,
};
pub use glossary::{
    list_glossary_entries, save_artifact_glossary_overrides, save_user_glossary,
};
pub use notices::{clear_runtime_notices, get_runtime_notices};
pub use provider_test::test_provider_connection;
pub use reader::{
    add_bookmark, add_note, get_reader_state, list_reader_states, log_reader_activity,
    mark_word_status, read_reader_toc, remove_bookmark, remove_note, repair_reader_states,
    save_reader_progress, save_word_definition, unmark_word_status, update_note, ReaderState,
};
pub use review::{export_reviewed_markdown, list_review_segments, run_review_checks, save_review_edit};
pub use skills::{
    create_skill_entry, delete_skill_entry, get_skill_library_entry, list_agents_registry,
    list_skill_library_entries, preview_agent_prompt, update_skill_library_entry,
};
use storage::{
    load_and_repair_reader_states, load_and_repair_tasks, load_json, load_single_json,
    persist_json,
};
pub use toc_edit::{
    add_toc_entry, get_page_preview, list_toc_edit_items, remove_toc_entry, rename_toc_entry,
    set_toc_entry_page,
};
pub use user_glossaries::{
    create_glossary_deck, delete_glossary_deck, list_glossary_decks, rename_glossary_deck,
    resolve_task_static_entries, save_glossary_deck_entries, toggle_glossary_deck,
};
pub use tasks::{
    cancel_translation_task, delete_task, get_task_status, list_tasks, pause_translation_task,
    resume_translation_task, retry_translation_task, start_translation,
    translate_epub_chapter_once, TranslationTask,
};
use tasks::{repair_loaded_tasks, TaskControl};
pub use tts::{list_tts_voices, synthesize_tts};
pub use vocab::{
    add_vocab_card, export_anki_cards, list_due_vocab_cards, list_due_vocab_cards_all,
    remove_vocab_card, review_vocab_card, save_vocab_card_note, seed_book_vocab_all,
    sync_wordwise_vocab,
};
pub use vocab_notebooks::{
    add_vocab_notebook_entries, create_vocab_notebook, delete_vocab_notebook,
    list_vocab_notebooks, remove_vocab_notebook_entry, rename_vocab_notebook,
    review_vocab_notebook_entry, save_vocab_notebook_entry_note,
};
pub use fonts::{delete_reader_font, import_reader_font, list_reader_fonts};
pub use web_capture::capture_web_article;
pub use wordlist::{delete_wordlist, import_wordlist, wordlist_status};
pub use words::{extract_difficult_words, lookup_word_level, lookup_words_batch, lookup_words_info_batch};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Semaphore;

const DEFAULT_GLOBAL_LLM_LIMIT: usize = 10;
const MIN_GLOBAL_LLM_LIMIT: usize = 1;
const MAX_GLOBAL_LLM_LIMIT: usize = 64;
pub(super) const MAX_NONFATAL_NOTICES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeNotice {
    pub timestamp: String,
    pub scope: String,
    pub task_id: Option<String>,
    pub detail: String,
    pub code: String,
    pub message: String,
}

pub(super) type NonfatalNoticeBuffer = Arc<Mutex<VecDeque<RuntimeNotice>>>;

pub struct AppState {
    pub tasks: Arc<Mutex<HashMap<String, TranslationTask>>>,
    pub config: Arc<Mutex<RuntimeConfig>>,
    pub reader_states: Arc<Mutex<HashMap<String, ReaderState>>>,
    pub book_profiles: Arc<Mutex<HashMap<String, BookProfile>>>,
    pub task_store_path: PathBuf,
    pub reader_state_store_path: PathBuf,
    pub book_profile_store_path: PathBuf,
    pub runtime_config_store_path: PathBuf,
    pub import_root_dir: PathBuf,
    pub artifact_root_dir: PathBuf,
    pub skill_library_root_dir: PathBuf,
    /// 用户导入词包的存放目录（词库不随应用分发）。
    pub wordlists_dir: PathBuf,
    pub llm_limiter: Arc<Semaphore>,
    nonfatal_notices: NonfatalNoticeBuffer,
    controls: Arc<Mutex<HashMap<String, TaskControl>>>,
    task_event_sink: Option<tasks::TaskEventSink>,
}

impl AppState {
    pub fn new() -> Self {
        let env_sources = load_runtime_env_sources();
        let paths = resolve_runtime_paths();
        let defaults = resolve_env_defaults(&env_sources);
        let runtime_config = load_runtime_config(&paths.runtime_config_store_path, defaults);
        let persisted_tasks = load_and_repair_tasks(&paths.task_store_path);
        let persisted_reader_states = load_and_repair_reader_states(&paths.reader_state_store_path);
        let persisted_book_profiles: HashMap<String, BookProfile> =
            load_json(&paths.book_profile_store_path);

        Self {
            tasks: Arc::new(Mutex::new(persisted_tasks)),
            config: Arc::new(Mutex::new(runtime_config)),
            reader_states: Arc::new(Mutex::new(persisted_reader_states)),
            book_profiles: Arc::new(Mutex::new(persisted_book_profiles)),
            task_store_path: paths.task_store_path,
            reader_state_store_path: paths.reader_state_store_path,
            book_profile_store_path: paths.book_profile_store_path,
            runtime_config_store_path: paths.runtime_config_store_path,
            import_root_dir: paths.import_root_dir,
            artifact_root_dir: paths.artifact_root_dir,
            skill_library_root_dir: paths.skill_library_root_dir,
            wordlists_dir: paths.wordlists_dir,
            llm_limiter: Arc::new(Semaphore::new(resolve_global_llm_limit())),
            nonfatal_notices: Arc::new(Mutex::new(VecDeque::new())),
            controls: Arc::new(Mutex::new(HashMap::new())),
            task_event_sink: None,
        }
    }

    /// 注入任务事件 sink（生产环境在 setup 中绑定 app_handle.emit）。
    /// 必须在任何任务启动前调用；未注入时事件静默丢弃（测试环境）。
    pub fn set_task_event_sink(&mut self, sink: tasks::TaskEventSink) {
        self.task_event_sink = Some(sink);
    }

    pub(crate) fn task_event_sink(&self) -> Option<tasks::TaskEventSink> {
        self.task_event_sink.clone()
    }
}

fn resolve_global_llm_limit() -> usize {
    std::env::var("MUSETRANSLATE_GLOBAL_LLM_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_GLOBAL_LLM_LIMIT)
        .clamp(MIN_GLOBAL_LLM_LIMIT, MAX_GLOBAL_LLM_LIMIT)
}

pub(super) fn report_nonfatal_error(
    notices: Option<&NonfatalNoticeBuffer>,
    scope: &str,
    detail: &str,
    error: &AppError,
) {
    notices::report_nonfatal_error(notices, scope, detail, error)
}

fn lock_mutex<'a, T>(mutex: &'a Mutex<T>, label: &str) -> Result<MutexGuard<'a, T>, AppError> {
    mutex
        .lock()
        .map_err(|_| AppError::internal(format!("{label}状态锁已损坏")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::notices::{notice_matches_scope, notice_matches_task_id};
    use crate::commands::tasks::{TaskArtifactPaths, TaskPhase, TaskStatus};
    use std::fs;

    fn sample_task(task_id: &str) -> TranslationTask {
        TranslationTask {
            id: task_id.to_string(),
            filename: "demo.epub".to_string(),
            pdf_path: "D:/demo.epub".to_string(),
            status: TaskStatus::Processing,
            phase: TaskPhase::Translating,
            progress: 67,
            message: "translating".to_string(),
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
            translated_chunks: 3,
            retry_count: 1,
            source_hash: Some("hash-1".to_string()),
            last_error: None,
        }
    }

    #[test]
    fn load_and_repair_tasks_persists_recovered_failure_state() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-load-repair-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let task_store_path = root.join("tasks.json");
        let tasks = HashMap::from([("task-1".to_string(), sample_task("task-1"))]);
        persist_json(&task_store_path, &tasks).expect("task store should persist");

        let repaired = load_and_repair_tasks(&task_store_path);

        let task = repaired.get("task-1").expect("task should exist");
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.phase, TaskPhase::Translating);
        assert_eq!(
            task.last_error.as_ref().map(|error| error.code.as_str()),
            Some("INTERRUPTED")
        );

        let persisted =
            fs::read_to_string(&task_store_path).expect("repaired task store should be written");
        assert!(persisted.contains("\"status\": \"failed\""));
        assert!(persisted.contains("\"phase\": \"translating\""));
        assert!(persisted.contains("\"code\": \"INTERRUPTED\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_and_repair_reader_states_persists_normalized_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-reader-load-repair-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).expect("temp dir should exist");
        let reader_state_store_path = root.join("reader_states.json");
        let activities: Vec<_> = (0..190)
            .map(|index| {
                serde_json::json!({
                    "id": format!("activity-{index}"),
                    "bookId": "book-1",
                    "minutes": 5,
                    "progress": 15,
                    "chapter": "第 1 章",
                    "createdAt": "2026-01-01T00:00:00Z"
                })
            })
            .collect();
        let reader_states = serde_json::json!({
            "book-1": {
                "bookId": "book-1",
                "progress": 15,
                "chapter": "",
                "bookmarks": [],
                "notes": [],
                "activities": activities,
                "updatedAt": "2026-01-01T00:00:00Z"
            }
        });
        persist_json(&reader_state_store_path, &reader_states)
            .expect("reader state store should persist");

        let repaired = load_and_repair_reader_states(&reader_state_store_path);

        let state = repaired.get("book-1").expect("reader state should exist");
        assert_eq!(state.chapter, "第 1 章");
        assert_eq!(state.activities.len(), 180);

        let persisted = fs::read_to_string(&reader_state_store_path)
            .expect("repaired reader state store should be written");
        assert!(persisted.contains("\"chapter\": \"第 1 章\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn report_nonfatal_error_buffers_recent_structured_notices() {
        let notices = Arc::new(Mutex::new(VecDeque::new()));
        let error = AppError::internal("persist failed");

        for index in 0..130 {
            report_nonfatal_error(
                Some(&notices),
                "tasks",
                &format!("task_id=task-{index} operation=writeback"),
                &error,
            );
        }

        let guard = notices.lock().expect("notice mutex should remain healthy");
        assert_eq!(guard.len(), 128);
        assert_eq!(
            guard.front().and_then(|notice| notice.task_id.as_deref()),
            Some("task-2")
        );
        assert_eq!(
            guard.back().and_then(|notice| notice.task_id.as_deref()),
            Some("task-129")
        );
        assert_eq!(
            guard.front().map(|notice| notice.detail.as_str()),
            Some("task_id=task-2 operation=writeback")
        );
        assert_eq!(
            guard.back().map(|notice| notice.detail.as_str()),
            Some("task_id=task-129 operation=writeback")
        );
        assert!(guard.iter().all(|notice| notice.scope == "tasks"));
        assert!(guard.iter().all(|notice| notice.code == "INTERNAL"));
    }

    #[test]
    fn get_runtime_notices_returns_latest_entries_in_reverse_order() {
        let notices = Arc::new(Mutex::new(VecDeque::new()));
        let error = AppError::internal("persist failed");
        for index in 0..4 {
            report_nonfatal_error(
                Some(&notices),
                "tasks",
                &format!("task_id=task-{index} operation=writeback"),
                &error,
            );
        }
        let state = AppState {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            config: Arc::new(Mutex::new(RuntimeConfig::default())),
            reader_states: Arc::new(Mutex::new(HashMap::new())),
            book_profiles: Arc::new(Mutex::new(HashMap::new())),
            task_store_path: PathBuf::from("tasks.json"),
            reader_state_store_path: PathBuf::from("reader_states.json"),
            book_profile_store_path: PathBuf::from("book_profiles.json"),
            runtime_config_store_path: PathBuf::from("runtime_config.json"),
            import_root_dir: PathBuf::from("imports"),
            artifact_root_dir: PathBuf::from("artifacts"),
            skill_library_root_dir: PathBuf::from("skills"),
            wordlists_dir: PathBuf::from("wordlists"),
            llm_limiter: Arc::new(Semaphore::new(1)),
            nonfatal_notices: Arc::clone(&notices),
            controls: Arc::new(Mutex::new(HashMap::new())),
            task_event_sink: None,
        };

        let latest_two = {
            let notices = lock_mutex(&state.nonfatal_notices, "运行告警").expect("notice lock");
            let take = 2usize;
            notices.iter().rev().take(take).cloned().collect::<Vec<_>>()
        };

        assert_eq!(latest_two.len(), 2);
        assert_eq!(latest_two[0].task_id.as_deref(), Some("task-3"));
        assert_eq!(latest_two[1].task_id.as_deref(), Some("task-2"));
        assert_eq!(latest_two[0].detail, "task_id=task-3 operation=writeback");
        assert_eq!(latest_two[1].detail, "task_id=task-2 operation=writeback");
    }

    #[test]
    fn runtime_notice_filters_match_scope_and_task_id_contract() {
        let notices = Arc::new(Mutex::new(VecDeque::new()));
        let error = AppError::internal("persist failed");
        report_nonfatal_error(
            Some(&notices),
            "tasks",
            "task_id=task-1 operation=writeback",
            &error,
        );
        report_nonfatal_error(
            Some(&notices),
            "task-runner",
            "task_id=task-2 operation=apply_task_failure",
            &error,
        );
        report_nonfatal_error(
            Some(&notices),
            "commands",
            "operation=persist_repaired_tasks",
            &error,
        );

        let guard = notices.lock().expect("notice mutex should remain healthy");
        let task_scope = guard
            .iter()
            .filter(|notice| notice_matches_scope(notice, Some("tasks")))
            .cloned()
            .collect::<Vec<_>>();
        let task_id_hits = guard
            .iter()
            .filter(|notice| notice_matches_task_id(notice, Some("task-2")))
            .cloned()
            .collect::<Vec<_>>();
        drop(guard);

        assert_eq!(task_scope.len(), 1);
        assert_eq!(task_scope[0].scope, "tasks");
        assert_eq!(task_id_hits.len(), 1);
        assert_eq!(task_id_hits[0].task_id.as_deref(), Some("task-2"));
        assert_eq!(
            task_id_hits[0].detail,
            "task_id=task-2 operation=apply_task_failure"
        );
    }

    #[test]
    fn clear_runtime_notices_removes_only_matching_entries() {
        let notices = Arc::new(Mutex::new(VecDeque::new()));
        let error = AppError::internal("persist failed");
        report_nonfatal_error(
            Some(&notices),
            "tasks",
            "task_id=task-1 operation=writeback",
            &error,
        );
        report_nonfatal_error(
            Some(&notices),
            "tasks",
            "task_id=task-2 operation=writeback",
            &error,
        );
        report_nonfatal_error(
            Some(&notices),
            "commands",
            "operation=persist_repaired_tasks",
            &error,
        );

        let mut guard = notices.lock().expect("notice mutex should remain healthy");
        let before = guard.len();
        guard.retain(|notice| {
            !(notice_matches_scope(notice, Some("tasks"))
                && notice_matches_task_id(notice, Some("task-1")))
        });
        let removed = before.saturating_sub(guard.len());

        assert_eq!(removed, 1);
        assert_eq!(guard.len(), 2);
        assert!(guard
            .iter()
            .all(|notice| notice.task_id.as_deref() != Some("task-1")));
        assert!(guard
            .iter()
            .all(|notice| notice.detail != "task_id=task-1 operation=writeback"));
        assert!(guard
            .iter()
            .any(|notice| notice.detail == "task_id=task-2 operation=writeback"));
        assert!(guard.iter().any(|notice| notice.scope == "commands"));
    }
}
