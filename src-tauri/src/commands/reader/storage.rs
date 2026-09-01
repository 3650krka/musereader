use super::model::{
    empty_reader_state, normalized_chapter, ReaderActivity, ReaderBookmark, ReaderNote,
    ReaderState, WordMark, WordMarkStatus,
};
use crate::commands::{lock_mutex, persist_json, AppError, AppState};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

const MAX_ACTIVITY_MINUTES: u16 = 180;
const MAX_ACTIVITY_RECORDS: usize = 180;

pub fn ensure_reader_state<'a>(
    states: &'a mut HashMap<String, ReaderState>,
    book_id: &str,
) -> &'a mut ReaderState {
    states
        .entry(book_id.to_string())
        .or_insert_with(|| empty_reader_state(book_id))
}

pub fn persist_reader_states(
    state: &AppState,
    reader_states: &HashMap<String, ReaderState>,
) -> Result<(), AppError> {
    persist_json(&state.reader_state_store_path, reader_states)
}

pub fn mutate_reader_state<F>(
    state: &AppState,
    book_id: &str,
    mutate: F,
) -> Result<ReaderState, AppError>
where
    F: FnOnce(&mut ReaderState),
{
    let mut states = lock_mutex(&state.reader_states, "阅读状态")?;
    let previous = states.get(book_id).cloned();
    let snapshot = {
        let reader_state = ensure_reader_state(&mut states, book_id);
        mutate(reader_state);
        reader_state.clone()
    };

    if let Err(error) = persist_reader_states(state, &states) {
        if let Some(previous_state) = previous {
            states.insert(book_id.to_string(), previous_state);
        } else {
            states.remove(book_id);
        }
        return Err(error);
    }

    Ok(snapshot)
}

pub fn update_reading_position(reader_state: &mut ReaderState, progress: u8, chapter: String) {
    reader_state.progress = progress.min(100);
    reader_state.chapter = normalized_chapter(chapter);
    reader_state.updated_at = Utc::now().to_rfc3339();
}

/// 记录精确阅读位置（EPUB 锚点 + 页内偏移）；仅在前端上报有效 href 时调用。
pub fn update_reading_location(reader_state: &mut ReaderState, href: String, offset: u32) {
    reader_state.href = Some(href);
    reader_state.offset = offset;
    reader_state.updated_at = Utc::now().to_rfc3339();
}

pub fn prepend_activity(reader_state: &mut ReaderState, minutes: f32) {
    reader_state.activities.insert(
        0,
        ReaderActivity {
            id: Uuid::new_v4().to_string(),
            book_id: reader_state.book_id.clone(),
            minutes: minutes.clamp(0.1, MAX_ACTIVITY_MINUTES as f32),
            progress: reader_state.progress,
            chapter: reader_state.chapter.clone(),
            created_at: Utc::now().to_rfc3339(),
        },
    );
    reader_state.activities.truncate(MAX_ACTIVITY_RECORDS);
}

pub fn append_bookmark(
    reader_state: &mut ReaderState,
    chapter: String,
    quote: String,
    progress: u8,
    href: Option<String>,
    style: String,
) {
    reader_state.bookmarks.insert(
        0,
        ReaderBookmark {
            id: Uuid::new_v4().to_string(),
            book_id: reader_state.book_id.clone(),
            chapter: normalized_chapter(chapter),
            quote,
            progress: progress.min(100),
            href,
            style,
            created_at: Utc::now().to_rfc3339(),
        },
    );
    reader_state.updated_at = Utc::now().to_rfc3339();
}

/// 删除书签/划线（按 id；未命中为幂等空操作）。
pub fn remove_bookmark(reader_state: &mut ReaderState, bookmark_id: &str) {
    let before = reader_state.bookmarks.len();
    reader_state.bookmarks.retain(|bookmark| bookmark.id != bookmark_id);
    if reader_state.bookmarks.len() != before {
        reader_state.updated_at = Utc::now().to_rfc3339();
    }
}

/// 删除笔记（按 id；未命中为幂等空操作）。
pub fn remove_note(reader_state: &mut ReaderState, note_id: &str) {
    let before = reader_state.notes.len();
    reader_state.notes.retain(|note| note.id != note_id);
    if reader_state.notes.len() != before {
        reader_state.updated_at = Utc::now().to_rfc3339();
    }
}

/** 编辑笔记正文（按 id；未命中时查书签 quote；幂等空操作）。 */
pub fn update_note(reader_state: &mut ReaderState, note_id: &str, note: String) {
    if let Some(entry) = reader_state.notes.iter_mut().find(|n| n.id == note_id) {
        entry.note = note;
        reader_state.updated_at = Utc::now().to_rfc3339();
    } else if let Some(bmk) = reader_state.bookmarks.iter_mut().find(|b| b.id == note_id) {
        bmk.quote = note;
        reader_state.updated_at = Utc::now().to_rfc3339();
    }
}

pub fn append_note(
    reader_state: &mut ReaderState,
    chapter: String,
    quote: String,
    note: String,
    progress: u8,
    href: Option<String>,
) {
    reader_state.notes.insert(
        0,
        ReaderNote {
            id: Uuid::new_v4().to_string(),
            book_id: reader_state.book_id.clone(),
            chapter: normalized_chapter(chapter),
            quote,
            note,
            progress: progress.min(100),
            href,
            created_at: Utc::now().to_rfc3339(),
        },
    );
    reader_state.updated_at = Utc::now().to_rfc3339();
}

/// 词汇黑白名单标记（mastered=白名单 / learning=黑名单）。
/// 重复标记同一状态幂等；状态切换覆盖旧值并刷新语境与时间；已有 AI 释义保留。
pub fn upsert_word_mark(
    reader_state: &mut ReaderState,
    word: &str,
    status: WordMarkStatus,
    context: String,
) {
    let key = word.trim().to_ascii_lowercase();
    if key.is_empty() {
        return;
    }
    let custom_definition = reader_state
        .word_marks
        .get(&key)
        .map(|mark| mark.custom_definition.clone())
        .unwrap_or_default();
    reader_state.word_marks.insert(
        key.clone(),
        WordMark {
            word: key,
            status,
            context,
            custom_definition,
            updated_at: Utc::now().to_rfc3339(),
        },
    );
    reader_state.updated_at = Utc::now().to_rfc3339();
}

/// 保存 AI 语境释义（custom_definition）：未标记的词自动落入 learning（黑名单）。
pub fn set_word_custom_definition(
    reader_state: &mut ReaderState,
    word: &str,
    context: String,
    definition: String,
) {
    let key = word.trim().to_ascii_lowercase();
    if key.is_empty() || definition.trim().is_empty() {
        return;
    }
    let existing = reader_state.word_marks.get(&key);
    let status = existing
        .map(|mark| mark.status)
        .unwrap_or(WordMarkStatus::Learning);
    let context = if context.is_empty() {
        existing
            .map(|mark| mark.context.clone())
            .unwrap_or_default()
    } else {
        context
    };
    reader_state.word_marks.insert(
        key.clone(),
        WordMark {
            word: key,
            status,
            context,
            custom_definition: definition.trim().to_string(),
            updated_at: Utc::now().to_rfc3339(),
        },
    );
    reader_state.updated_at = Utc::now().to_rfc3339();
}

/// 移除词汇标记（误标撤回）。返回是否实际移除。
pub fn remove_word_mark(reader_state: &mut ReaderState, word: &str) -> bool {
    let removed = reader_state
        .word_marks
        .remove(&word.trim().to_ascii_lowercase())
        .is_some();
    if removed {
        reader_state.updated_at = Utc::now().to_rfc3339();
    }
    removed
}

pub fn load_reader_snapshot(state: &AppState, book_id: &str) -> Result<ReaderState, AppError> {
    mutate_reader_state(state, book_id, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::RuntimeConfig;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Semaphore;

    fn sample_reader_state(book_id: &str) -> ReaderState {
        ReaderState {
            book_id: book_id.to_string(),
            progress: 25,
            chapter: "第 2 章".to_string(),
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

    fn sample_state(root: &Path, reader_state: Option<ReaderState>) -> AppState {
        let states = reader_state
            .map(|state| HashMap::from([(state.book_id.clone(), state)]))
            .unwrap_or_default();
        AppState {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            config: Arc::new(Mutex::new(RuntimeConfig::default())),
            reader_states: Arc::new(Mutex::new(states)),
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
    fn mutate_reader_state_removes_new_state_when_persist_fails() {
        let unique = format!(
            "musetranslate-reader-storage-insert-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let state = sample_state(&root, None);
        fs::create_dir_all(&state.reader_state_store_path)
            .expect("reader state store path should become a directory");

        let error = mutate_reader_state(&state, "book-1", |reader_state| {
            update_reading_position(reader_state, 50, "第 5 章".to_string());
        })
        .expect_err("persist failure should roll back");

        assert_eq!(error.code, "INTERNAL");
        let states = state
            .reader_states
            .lock()
            .expect("reader state mutex should remain healthy");
        assert!(!states.contains_key("book-1"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mutate_reader_state_restores_existing_state_when_persist_fails() {
        let unique = format!(
            "musetranslate-reader-storage-restore-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let state = sample_state(&root, Some(sample_reader_state("book-1")));
        fs::create_dir_all(&state.reader_state_store_path)
            .expect("reader state store path should become a directory");

        let error = mutate_reader_state(&state, "book-1", |reader_state| {
            append_bookmark(
                reader_state,
                "第 9 章".to_string(),
                "quoted text".to_string(),
                88,
                None,
                "bookmark".to_string(),
            );
        })
        .expect_err("persist failure should restore previous state");

        assert_eq!(error.code, "INTERNAL");
        let states = state
            .reader_states
            .lock()
            .expect("reader state mutex should remain healthy");
        let reader_state = states
            .get("book-1")
            .expect("reader state should still exist");
        assert_eq!(reader_state.progress, 25);
        assert_eq!(reader_state.chapter, "第 2 章");
        assert!(reader_state.bookmarks.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn word_mark_upsert_switches_status_and_remove_is_idempotent() {
        let mut state = sample_reader_state("book-1");

        upsert_word_mark(
            &mut state,
            "Ephemeral",
            WordMarkStatus::Learning,
            "an ephemeral bloom".to_string(),
        );
        let mark = state
            .word_marks
            .get("ephemeral")
            .expect("mark should exist");
        assert_eq!(mark.status, WordMarkStatus::Learning);
        assert_eq!(mark.context, "an ephemeral bloom");

        // 重复标记同状态幂等；切换状态覆盖并刷新语境
        upsert_word_mark(
            &mut state,
            "ephemeral",
            WordMarkStatus::Learning,
            String::new(),
        );
        assert_eq!(state.word_marks.len(), 1);
        upsert_word_mark(
            &mut state,
            "ephemeral",
            WordMarkStatus::Mastered,
            String::new(),
        );
        assert_eq!(
            state.word_marks.get("ephemeral").map(|mark| mark.status),
            Some(WordMarkStatus::Mastered)
        );

        assert!(remove_word_mark(&mut state, "EPHEMERAL"));
        assert!(!remove_word_mark(&mut state, "ephemeral"));
        assert!(state.word_marks.is_empty());

        // 空词不入名单
        upsert_word_mark(&mut state, "   ", WordMarkStatus::Learning, String::new());
        assert!(state.word_marks.is_empty());
    }
}
