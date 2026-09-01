use super::{lock_mutex, AppError, AppState};

mod model;
pub(crate) mod storage;
mod toc;

pub use model::{repair_reader_states, ReaderState, ReviewLogEntry, WordMarkStatus};
use storage::{
    append_bookmark, append_note, load_reader_snapshot, mutate_reader_state, prepend_activity,
    remove_word_mark, set_word_custom_definition, update_reading_location, update_reading_position,
    upsert_word_mark,
};
pub use toc::read_reader_toc;

#[tauri::command]
pub async fn get_reader_state(
    book_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    load_reader_snapshot(&state, &book_id)
}

#[tauri::command]
pub async fn list_reader_states(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ReaderState>, AppError> {
    let states = lock_mutex(&state.reader_states, "阅读状态")?;
    let mut values: Vec<_> = states.values().cloned().collect();
    values.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(values)
}

#[tauri::command]
pub async fn save_reader_progress(
    book_id: String,
    progress: u8,
    chapter: String,
    href: Option<String>,
    offset: Option<u32>,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    mutate_reader_state(&state, &book_id, |reader_state| {
        update_reading_position(reader_state, progress, chapter);
        if let Some(href) = href.filter(|value| !value.trim().is_empty()) {
            update_reading_location(reader_state, href, offset.unwrap_or(0));
        }
    })
}

#[tauri::command]
pub async fn log_reader_activity(
    book_id: String,
    minutes: f32,
    progress: u8,
    chapter: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    mutate_reader_state(&state, &book_id, |reader_state| {
        update_reading_position(reader_state, progress, chapter);
        prepend_activity(reader_state, minutes);
    })
}

#[tauri::command]
pub async fn add_bookmark(
    book_id: String,
    chapter: String,
    quote: String,
    progress: u8,
    href: Option<String>,
    style: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    let style = style.unwrap_or_default();
    mutate_reader_state(&state, &book_id, |reader_state| {
        append_bookmark(reader_state, chapter, quote, progress, href, style);
    })
}

/// 删除书签/划线（按 id；未命中为幂等空操作）。
#[tauri::command]
pub async fn remove_bookmark(
    book_id: String,
    bookmark_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    let bookmark_id = bookmark_id.trim().to_string();
    if bookmark_id.is_empty() {
        return Err(AppError::invalid_input("bookmark_id 不能为空"));
    }
    mutate_reader_state(&state, &book_id, |reader_state| {
        crate::commands::reader::storage::remove_bookmark(reader_state, &bookmark_id);
    })
}

/// 删除笔记（按 id；未命中为幂等空操作）。
#[tauri::command]
pub async fn remove_note(
    book_id: String,
    note_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    let note_id = note_id.trim().to_string();
    if note_id.is_empty() {
        return Err(AppError::invalid_input("note_id 不能为空"));
    }
    mutate_reader_state(&state, &book_id, |reader_state| {
        crate::commands::reader::storage::remove_note(reader_state, &note_id);
    })
}

#[tauri::command]
pub async fn add_note(
    book_id: String,
    chapter: String,
    quote: String,
    note: String,
    progress: u8,
    href: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    mutate_reader_state(&state, &book_id, |reader_state| {
        append_note(reader_state, chapter, quote, note, progress, href);
    })
}

/** 编辑笔记正文（按 id）。 */
#[tauri::command]
pub async fn update_note(
    book_id: String,
    note_id: String,
    note: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    if note_id.trim().is_empty() {
        return Err(AppError::invalid_input("note_id 为空"));
    }
    mutate_reader_state(&state, &book_id, |reader_state| {
        storage::update_note(reader_state, &note_id, note);
    })
}



/// 词汇黑白名单标记：status = "mastered"（已掌握/白名单）| "learning"（未掌握/黑名单）。
/// context 为标记时的语境句子（背诵卡片例句与 AI 语境释义来源），可为空。
#[tauri::command]
pub async fn mark_word_status(
    book_id: String,
    word: String,
    status: String,
    context: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    let status = match status.trim().to_ascii_lowercase().as_str() {
        "mastered" => WordMarkStatus::Mastered,
        "learning" => WordMarkStatus::Learning,
        other => {
            return Err(AppError::invalid_input(format!(
                "不支持的词汇标记状态: {other}（应为 mastered 或 learning）"
            )))
        }
    };
    mutate_reader_state(&state, &book_id, |reader_state| {
        upsert_word_mark(reader_state, &word, status, context.unwrap_or_default());
        /* 互斥：标记已掌握同时撤出生词本——同一词不能既"已掌握"（白名单，
           压制标注）又在背诵队列；用户点其一即自动取消另一个。 */
        if status == WordMarkStatus::Mastered {
            super::vocab::remove_card(&mut reader_state.vocab_cards, &word);
        }
    })
}

/// 移除词汇标记（误标撤回；未标记时幂等返回当前状态）。
#[tauri::command]
pub async fn unmark_word_status(
    book_id: String,
    word: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    mutate_reader_state(&state, &book_id, |reader_state| {
        remove_word_mark(reader_state, &word);
    })
}

/// 保存 AI 语境释义（多义词 AI 纠正结果）：未标记的词自动落入 learning（黑名单）。
#[tauri::command]
pub async fn save_word_definition(
    book_id: String,
    word: String,
    definition: String,
    context: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ReaderState, AppError> {
    if definition.trim().is_empty() {
        return Err(AppError::invalid_input("释义内容为空"));
    }
    mutate_reader_state(&state, &book_id, |reader_state| {
        set_word_custom_definition(reader_state, &word, context.unwrap_or_default(), definition);
    })
}
