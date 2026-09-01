//! 书籍档案（收藏/标签）：独立于任务重建链路的持久化元数据。
//!
//! 书籍对象每次启动由 completed tasks 重建，收藏/标签必须有独立存储：
//! `.runtime/book_profiles.json`（book_id → BookProfile），写失败回滚内存副本。

use super::{lock_mutex, persist_json, AppError, AppState};
use chrono::Utc;
use serde::{Deserialize, Serialize};

const MAX_TAGS_PER_BOOK: usize = 12;
const MAX_TAG_LEN: usize = 32;
const MAX_FOLDER_LEN: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookProfile {
    pub book_id: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 文件夹分组名（空=未分组；拖拽/右键成组，同名即同组）。
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub updated_at: String,
}

fn empty_profile(book_id: &str) -> BookProfile {
    BookProfile {
        book_id: book_id.to_string(),
        favorite: false,
        tags: Vec::new(),
        folder: String::new(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

/// 标签归一：去空白、去重（保留首见顺序）、长度与数量上限。
pub(crate) fn normalize_tags(raw_tags: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for raw in raw_tags {
        let tag = raw.trim().chars().take(MAX_TAG_LEN).collect::<String>();
        if tag.is_empty() || !seen.insert(tag.clone()) {
            continue;
        }
        normalized.push(tag);
        if normalized.len() >= MAX_TAGS_PER_BOOK {
            break;
        }
    }
    normalized
}

/// 变更 + 持久化 + 失败回滚（与 reader state mutate 模式一致）。
fn mutate_profile<F>(state: &AppState, book_id: &str, mutate: F) -> Result<BookProfile, AppError>
where
    F: FnOnce(&mut BookProfile),
{
    if book_id.trim().is_empty() {
        return Err(AppError::invalid_input("书籍 ID 为空"));
    }
    let mut profiles = lock_mutex(&state.book_profiles, "书籍档案")?;
    let previous = profiles.get(book_id).cloned();
    let profile = profiles.entry(book_id.to_string()).or_insert_with(|| empty_profile(book_id));
    mutate(profile);
    profile.updated_at = Utc::now().to_rfc3339();
    let snapshot = profile.clone();

    if let Err(error) = persist_json(&state.book_profile_store_path, &*profiles) {
        match previous {
            Some(previous) => {
                profiles.insert(book_id.to_string(), previous);
            }
            None => {
                profiles.remove(book_id);
            }
        }
        return Err(error);
    }
    Ok(snapshot)
}

// ---- Tauri 命令 ----

#[tauri::command]
pub async fn list_book_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BookProfile>, AppError> {
    Ok(lock_mutex(&state.book_profiles, "书籍档案")?
        .values()
        .cloned()
        .collect())
}

#[tauri::command]
pub async fn set_book_favorite(
    book_id: String,
    favorite: bool,
    state: tauri::State<'_, AppState>,
) -> Result<BookProfile, AppError> {
    mutate_profile(&state, &book_id, |profile| profile.favorite = favorite)
}

#[tauri::command]
pub async fn set_book_tags(
    book_id: String,
    tags: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<BookProfile, AppError> {
    let normalized = normalize_tags(tags);
    mutate_profile(&state, &book_id, |profile| profile.tags = normalized.clone())
}

/// 文件夹分组：folder 为空串=移出分组。组内书籍按 folder 名聚合展示。
#[tauri::command]
pub async fn set_book_folder(
    book_id: String,
    folder: String,
    state: tauri::State<'_, AppState>,
) -> Result<BookProfile, AppError> {
    let folder = folder.trim().chars().take(MAX_FOLDER_LEN).collect::<String>();
    mutate_profile(&state, &book_id, move |profile| profile.folder = folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tags_dedupes_trims_and_caps() {
        let mut tags = vec![" 科幻 ".to_string(), "科幻".to_string(), "".to_string(), "   ".to_string()];
        for i in 0..20 {
            tags.push(format!("tag-{i}"));
        }
        let normalized = normalize_tags(tags);
        assert_eq!(normalized[0], "科幻");
        assert_eq!(normalized.len(), MAX_TAGS_PER_BOOK);
    }

    #[test]
    fn normalize_tags_truncates_overlong_tag() {
        let long = "x".repeat(80);
        let normalized = normalize_tags(vec![long]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].chars().count(), MAX_TAG_LEN);
    }
}
