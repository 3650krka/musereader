//! 用户自建生词本：可建 n 个命名生词本，收纳生词透析的词；
//! 词条复用 SM-2 调度字段，支持在学习中心按本背诵。
//!
//! 持久化：`.runtime/vocab_notebooks.json`（原子写），独立于书籍档案。

use super::{lock_mutex, persist_json, AppError, AppState};
use crate::commands::vocab::sm2_review;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_NOTEBOOKS: usize = 200;
const MAX_ENTRIES_PER_NOTEBOOK: usize = 10_000;
const MAX_NOTEBOOK_NAME_LEN: usize = 48;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabNotebookEntry {
    pub word: String,
    #[serde(default)]
    pub definition: String,
    /// 原文所在例句（生词透析 1–3 句语境）。
    #[serde(default)]
    pub context: String,
    /// 来源书目（可为空=手动添加）。
    #[serde(default)]
    pub book_id: String,
    /// 用户单词笔记。
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub chapter: String,
    pub created_at: String,
    /* SM-2 调度字段 */
    #[serde(default)]
    pub repetitions: u32,
    #[serde(default)]
    pub interval_days: f32,
    #[serde(default = "default_ease")]
    pub ease_factor: f32,
    #[serde(default)]
    pub due_at: Option<String>,
}

fn default_ease() -> f32 {
    2.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabNotebook {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<VocabNotebookEntry>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NotebooksFile {
    #[serde(default)]
    notebooks: Vec<VocabNotebook>,
}

fn store_path(state: &AppState) -> std::path::PathBuf {
    state
        .reader_state_store_path
        .parent()
        .map(|dir| dir.join("vocab_notebooks.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("vocab_notebooks.json"))
}

fn load_store(state: &AppState) -> NotebooksFile {
    super::storage::load_single_json(&store_path(state)).unwrap_or_default()
}

fn save_store(state: &AppState, file: &NotebooksFile) -> Result<(), AppError> {
    persist_json(&store_path(state), file)
}

fn normalize_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("生词本名称为空"));
    }
    if trimmed.chars().count() > MAX_NOTEBOOK_NAME_LEN {
        return Err(AppError::invalid_input(format!(
            "生词本名称超过 {MAX_NOTEBOOK_NAME_LEN} 字"
        )));
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookEntryInput {
    pub word: String,
    #[serde(default)]
    pub definition: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub book_id: String,
    #[serde(default)]
    pub chapter: String,
}

#[tauri::command]
pub async fn list_vocab_notebooks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    Ok(load_store(&state).notebooks)
}

#[tauri::command]
pub async fn create_vocab_notebook(
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    let name = normalize_name(&name)?;
    let mut file = load_store(&state);
    if file.notebooks.len() >= MAX_NOTEBOOKS {
        return Err(AppError::invalid_input(format!(
            "生词本数量超出上限 {MAX_NOTEBOOKS} 个"
        )));
    }
    if file.notebooks.iter().any(|nb| nb.name == name) {
        return Err(AppError::conflict("同名生词本已存在"));
    }
    let now = Utc::now().to_rfc3339();
    file.notebooks.push(VocabNotebook {
        id: Uuid::new_v4().to_string(),
        name,
        created_at: now.clone(),
        updated_at: now,
        entries: Vec::new(),
    });
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

#[tauri::command]
pub async fn delete_vocab_notebook(
    notebook_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    let mut file = load_store(&state);
    let before = file.notebooks.len();
    file.notebooks.retain(|nb| nb.id != notebook_id);
    if file.notebooks.len() == before {
        return Err(AppError::not_found("生词本不存在"));
    }
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

#[tauri::command]
pub async fn rename_vocab_notebook(
    notebook_id: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    let name = normalize_name(&name)?;
    let mut file = load_store(&state);
    if file
        .notebooks
        .iter()
        .any(|nb| nb.id != notebook_id && nb.name == name)
    {
        return Err(AppError::conflict("同名生词本已存在"));
    }
    let notebook = file
        .notebooks
        .iter_mut()
        .find(|nb| nb.id == notebook_id)
        .ok_or_else(|| AppError::not_found("生词本不存在"))?;
    notebook.name = name;
    notebook.updated_at = Utc::now().to_rfc3339();
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

/// 批量添加词条（按 word 去重：已存在则跳过，不覆盖既有释义/调度）。
#[tauri::command]
pub async fn add_vocab_notebook_entries(
    notebook_id: String,
    entries: Vec<NotebookEntryInput>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    let mut file = load_store(&state);
    let notebook = file
        .notebooks
        .iter_mut()
        .find(|nb| nb.id == notebook_id)
        .ok_or_else(|| AppError::not_found("生词本不存在"))?;
    let mut seen: std::collections::HashSet<String> =
        notebook.entries.iter().map(|e| e.word.clone()).collect();
    let now = Utc::now().to_rfc3339();
    for input in entries {
        let word = input.word.trim().to_ascii_lowercase();
        if word.is_empty() || !seen.insert(word.clone()) {
            continue;
        }
        if notebook.entries.len() >= MAX_ENTRIES_PER_NOTEBOOK {
            break;
        }
        notebook.entries.push(VocabNotebookEntry {
            word,
            definition: input.definition.trim().to_string(),
            context: input.context.trim().to_string(),
            book_id: input.book_id,
            chapter: input.chapter,
            note: String::new(),
            created_at: now.clone(),
            repetitions: 0,
            interval_days: 0.0,
            ease_factor: default_ease(),
            due_at: None,
        });
    }
    notebook.updated_at = now;
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

#[tauri::command]
pub async fn remove_vocab_notebook_entry(
    notebook_id: String,
    word: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    let word_key = word.trim().to_ascii_lowercase();
    let mut file = load_store(&state);
    let notebook = file
        .notebooks
        .iter_mut()
        .find(|nb| nb.id == notebook_id)
        .ok_or_else(|| AppError::not_found("生词本不存在"))?;
    let before = notebook.entries.len();
    notebook.entries.retain(|entry| entry.word != word_key);
    if notebook.entries.len() != before {
        notebook.updated_at = Utc::now().to_rfc3339();
    }
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

/// 生词本背诵评分（SM-2 复用，与书籍卡组同参数语义）。
#[tauri::command]
pub async fn review_vocab_notebook_entry(
    notebook_id: String,
    word: String,
    quality: u8,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let _lock = lock_mutex(&state.reader_states, "阅读状态")?;
    let word_key = word.trim().to_ascii_lowercase();
    if word_key.is_empty() {
        return Err(AppError::invalid_input("卡片单词为空"));
    }
    let mut file = load_store(&state);
    let notebook = file
        .notebooks
        .iter_mut()
        .find(|nb| nb.id == notebook_id)
        .ok_or_else(|| AppError::not_found("生词本不存在"))?;
    let entry = notebook
        .entries
        .iter_mut()
        .find(|entry| entry.word == word_key)
        .ok_or_else(|| AppError::not_found("词条不存在"))?;
    // 复用 vocab 模块的 SM-2 实现（内部构造 VocabCard 调度字段）
    let mut card = crate::commands::vocab::VocabCard {
        word: entry.word.clone(),
        context: entry.context.clone(),
        context_zh: String::new(),
        chapter: entry.chapter.clone(),
        created_at: entry.created_at.clone(),
        custom_definition: entry.definition.clone(),
        note: String::new(),
        repetitions: entry.repetitions,
        interval_days: entry.interval_days,
        ease_factor: entry.ease_factor,
        due_at: entry.due_at.clone(),
    };
    sm2_review(&mut card, quality, Utc::now());
    entry.repetitions = card.repetitions;
    entry.interval_days = card.interval_days;
    entry.ease_factor = card.ease_factor;
    entry.due_at = card.due_at;
    notebook.updated_at = Utc::now().to_rfc3339();
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

/// 生词本词条笔记保存。
#[tauri::command]
pub async fn save_vocab_notebook_entry_note(
    notebook_id: String,
    word: String,
    note: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabNotebook>, AppError> {
    let trimmed = note.trim().to_string();
    let mut file = load_store(&state);
    let notebook = file
        .notebooks
        .iter_mut()
        .find(|nb| nb.id == notebook_id)
        .ok_or_else(|| AppError::not_found("生词本不存在"))?;
    if let Some(entry) = notebook
        .entries
        .iter_mut()
        .find(|e| e.word == word.trim().to_ascii_lowercase())
    {
        entry.note = trimmed;
        notebook.updated_at = Utc::now().to_rfc3339();
    }
    save_store(&state, &file)?;
    Ok(file.notebooks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_name_rejects_empty_and_overlong() {
        assert!(normalize_name("  ").is_err());
        assert!(normalize_name(&"词".repeat(MAX_NOTEBOOK_NAME_LEN + 1)).is_err());
        assert_eq!(normalize_name(" 考研高频 ").unwrap(), "考研高频");
    }
}
