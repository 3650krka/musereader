//! 用户术语词库命令层：词表 CRUD + 条目整表保存 + 启用开关。
//! 全部操作读改写 user_glossaries.json（原子持久化），文件即事实来源；
//! 首次访问时把老版本全局静态词表（runtime_config.static_glossary_entries）迁入默认词表。

use super::{lock_mutex, AppError, AppState, StaticGlossaryEntry};
use crate::glossary_store::{
    collect_entries_for_deck_ids, find_deck_mut, list_views, load_store, migrate_legacy_entries,
    normalize_deck_name, normalize_entries, save_store, store_path, GlossaryDeck,
    GlossaryDeckInput, GlossaryDeckView, StoreFile, MAX_DECKS,
};
use std::collections::HashSet;
use uuid::Uuid;

fn ensure_migration(state: &AppState) -> Result<StoreFile, AppError> {
    let path = store_path();
    let mut store = load_store(&path);
    let legacy = lock_mutex(&state.config, "运行配置")?
        .static_glossary_entries
        .clone();
    if !migrate_legacy_entries(&mut store, &legacy) {
        return Ok(store);
    }
    save_store(&path, &store)?;
    // 迁移成功后清空老字段，避免双写歧义；持久化失败回滚，下次访问重试迁移
    let mut config = lock_mutex(&state.config, "运行配置")?.clone();
    let previous = config.clone();
    config.static_glossary_entries.clear();
    if let Err(error) = super::persist_json(&state.runtime_config_store_path, &config) {
        *lock_mutex(&state.config, "运行配置")? = previous;
        return Err(error);
    }
    *lock_mutex(&state.config, "运行配置")? = config;
    Ok(store)
}

#[tauri::command]
pub async fn list_glossary_decks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlossaryDeckView>, AppError> {
    let store = ensure_migration(&state)?;
    Ok(list_views(&store))
}

#[tauri::command]
pub async fn create_glossary_deck(
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlossaryDeckView>, AppError> {
    let mut store = ensure_migration(&state)?;
    if store.decks.len() >= MAX_DECKS {
        return Err(AppError::invalid_input(format!(
            "词表数量超出上限 {MAX_DECKS} 个"
        )));
    }
    let normalized = normalize_deck_name(&name)?;
    if store.decks.iter().any(|deck| deck.name == normalized) {
        return Err(AppError::conflict("同名词表已存在"));
    }
    let now = chrono::Utc::now().to_rfc3339();
    store.decks.push(GlossaryDeck {
        id: Uuid::new_v4().to_string(),
        name: normalized,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
        entries: Vec::new(),
    });
    save_store(&store_path(), &store)?;
    Ok(list_views(&store))
}

#[tauri::command]
pub async fn rename_glossary_deck(
    deck_id: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlossaryDeckView>, AppError> {
    let mut store = ensure_migration(&state)?;
    let normalized = normalize_deck_name(&name)?;
    if store
        .decks
        .iter()
        .any(|deck| deck.id != deck_id && deck.name == normalized)
    {
        return Err(AppError::conflict("同名词表已存在"));
    }
    {
        let deck = find_deck_mut(&mut store, &deck_id)?;
        deck.name = normalized;
        deck.updated_at = chrono::Utc::now().to_rfc3339();
    }
    save_store(&store_path(), &store)?;
    Ok(list_views(&store))
}

#[tauri::command]
pub async fn delete_glossary_deck(
    deck_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlossaryDeckView>, AppError> {
    let mut store = ensure_migration(&state)?;
    let before = store.decks.len();
    store.decks.retain(|deck| deck.id != deck_id);
    if store.decks.len() == before {
        return Err(AppError::not_found(format!("词表不存在: {deck_id}")));
    }
    save_store(&store_path(), &store)?;
    Ok(list_views(&store))
}

#[tauri::command]
pub async fn toggle_glossary_deck(
    deck_id: String,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlossaryDeckView>, AppError> {
    let mut store = ensure_migration(&state)?;
    {
        let deck = find_deck_mut(&mut store, &deck_id)?;
        deck.enabled = enabled;
        deck.updated_at = chrono::Utc::now().to_rfc3339();
    }
    save_store(&store_path(), &store)?;
    Ok(list_views(&store))
}

#[tauri::command]
pub async fn save_glossary_deck_entries(
    deck_id: String,
    entries: Vec<GlossaryDeckInput>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlossaryDeckView>, AppError> {
    let mut store = ensure_migration(&state)?;
    let normalized = normalize_entries(entries)?;
    {
        let deck = find_deck_mut(&mut store, &deck_id)?;
        deck.entries = normalized;
        deck.updated_at = chrono::Utc::now().to_rfc3339();
    }
    save_store(&store_path(), &store)?;
    Ok(list_views(&store))
}

/// 翻译启动/重试时解析本次注入管线的静态术语：
/// deck_ids=None → 全部启用词表；Some → 仅「启用且在集合内」的词表。
/// 词库为空时回落到老全局词表（兼容未完成迁移的极端时序）。
pub fn resolve_task_static_entries(
    state: &AppState,
    deck_ids: Option<Vec<String>>,
) -> Result<Vec<StaticGlossaryEntry>, AppError> {
    let store = ensure_migration(state)?;
    let selected: Option<HashSet<String>> = deck_ids.map(|ids| ids.into_iter().collect());
    let entries = collect_entries_for_deck_ids(&store, selected.as_ref());
    if entries.is_empty() && store.decks.is_empty() {
        return Ok(lock_mutex(&state.config, "运行配置")?
            .static_glossary_entries
            .clone());
    }
    Ok(entries)
}
