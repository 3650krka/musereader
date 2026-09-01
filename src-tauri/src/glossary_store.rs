//! 用户术语词库（多词表）：deck 级启用/禁用 + 条目 CRUD，独立于翻译任务存在。
//!
//! 持久化：user_glossaries.json（原子写）。`collect_enabled_glossary_entries`
//! 在任务启动时把启用词表拍平为 StaticGlossaryEntry 注入管线 prompt；
//! 前端在启动翻译时可传 `glossary_deck_ids` 显式指定本次启用的词表。
//!
//! 迁移兼容：老版本单一全局词表（runtime_config.static_glossary_entries）
//! 非空时一次性迁入默认词表，之后以词库文件为准。

use crate::commands::StaticGlossaryEntry;
use crate::commands::resolve_runtime_paths;
use crate::error::AppError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

const STORE_FILE: &str = "user_glossaries.json";
const DEFAULT_DECK_ID: &str = "default";
pub const MAX_DECKS: usize = 64;
const MAX_ENTRIES_PER_DECK: usize = 2_000;
const MAX_DECK_NAME_LEN: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryDeck {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<StaticGlossaryEntry>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryDeckInput {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub enforcement: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryDeckView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub entry_count: usize,
    pub entries: Vec<StaticGlossaryEntry>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StoreFile {
    #[serde(default)]
    pub decks: Vec<GlossaryDeck>,
}

pub fn store_path() -> PathBuf {
    resolve_runtime_paths()
        .runtime_config_store_path
        .parent()
        .map(|dir| dir.join(STORE_FILE))
        .unwrap_or_else(|| PathBuf::from(STORE_FILE))
}

pub fn load_store(path: &PathBuf) -> StoreFile {
    let Ok(content) = std::fs::read_to_string(path) else {
        return StoreFile::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_store(path: &PathBuf, store: &StoreFile) -> Result<(), AppError> {
    let payload = serde_json::to_string_pretty(store)
        .map_err(|error| AppError::internal(format!("序列化术语词库失败: {error}")))?;
    // 原子写：先写临时文件再 rename，崩溃不留半文件
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::internal(format!("创建术语词库目录失败: {error}")))?;
    }
    let temp_path = path.with_extension("json.tmp");
    std::fs::write(&temp_path, payload.as_bytes())
        .map_err(|error| AppError::internal(format!("写入术语词库临时文件失败: {error}")))?;
    std::fs::rename(&temp_path, path)
        .map_err(|error| AppError::internal(format!("替换术语词库失败: {error}")))
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub fn normalize_deck_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("词表名称不能为空"));
    }
    if trimmed.chars().count() > MAX_DECK_NAME_LEN {
        return Err(AppError::invalid_input(format!(
            "词表名称超过 {MAX_DECK_NAME_LEN} 字"
        )));
    }
    Ok(trimmed.to_string())
}

pub fn find_deck_mut<'a>(
    store: &'a mut StoreFile,
    deck_id: &str,
) -> Result<&'a mut GlossaryDeck, AppError> {
    store
        .decks
        .iter_mut()
        .find(|deck| deck.id == deck_id)
        .ok_or_else(|| AppError::not_found(format!("词表不存在: {deck_id}")))
}

pub fn normalize_entries(entries: Vec<GlossaryDeckInput>) -> Result<Vec<StaticGlossaryEntry>, AppError> {
    let mut normalized: Vec<StaticGlossaryEntry> = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let source = entry.source.trim().to_string();
        let target = entry.target.trim().to_string();
        if source.is_empty() || target.is_empty() {
            continue;
        }
        if !seen.insert(source.to_ascii_lowercase()) {
            continue;
        }
        normalized.push(StaticGlossaryEntry {
            source,
            target,
            scope: entry.scope.trim().to_string(),
            enforcement: entry.enforcement.trim().to_string(),
            notes: entry.notes.trim().to_string(),
        });
        if normalized.len() > MAX_ENTRIES_PER_DECK {
            return Err(AppError::invalid_input(format!(
                "单词表条目超出上限 {MAX_ENTRIES_PER_DECK} 条"
            )));
        }
    }
    Ok(normalized)
}

pub fn list_views(store: &StoreFile) -> Vec<GlossaryDeckView> {
    store
        .decks
        .iter()
        .map(|deck| GlossaryDeckView {
            id: deck.id.clone(),
            name: deck.name.clone(),
            enabled: deck.enabled,
            created_at: deck.created_at.clone(),
            updated_at: deck.updated_at.clone(),
            entry_count: deck.entries.len(),
            entries: deck.entries.clone(),
        })
        .collect()
}

/// 迁移：老全局静态词表非空且词库尚无默认词表时，整表迁入默认词表。
/// 返回是否发生了迁移（调用方据此回写文件并清空老字段）。
pub fn migrate_legacy_entries(store: &mut StoreFile, legacy: &[StaticGlossaryEntry]) -> bool {
    if legacy.is_empty() || store.decks.iter().any(|deck| deck.id == DEFAULT_DECK_ID) {
        return false;
    }
    let now = now_rfc3339();
    store.decks.insert(
        0,
        GlossaryDeck {
            id: DEFAULT_DECK_ID.to_string(),
            name: "通用术语表".to_string(),
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
            entries: legacy.to_vec(),
        },
    );
    true
}

/// 拍平启用词表 → 管线静态术语（跨表按 source 去重，先出现的词表优先）。
#[cfg(test)]
pub fn collect_enabled_glossary_entries(store: &StoreFile) -> Vec<StaticGlossaryEntry> {
    collect_entries_for_deck_ids(store, None)
}

/// 按显式词表 id 集合拍平术语：deck_ids=None 取全部启用词表；
/// Some 时仅取「启用且在集合内」的词表（翻译启动时的用户选择）。
pub fn collect_entries_for_deck_ids(
    store: &StoreFile,
    deck_ids: Option<&HashSet<String>>,
) -> Vec<StaticGlossaryEntry> {
    let mut collected = Vec::new();
    let mut seen = HashSet::new();
    for deck in &store.decks {
        if !deck.enabled {
            continue;
        }
        if let Some(ids) = deck_ids {
            if !ids.contains(&deck.id) {
                continue;
            }
        }
        for entry in &deck.entries {
            if seen.insert(entry.source.to_ascii_lowercase()) {
                collected.push(entry.clone());
            }
        }
    }
    collected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, target: &str) -> StaticGlossaryEntry {
        StaticGlossaryEntry {
            source: source.to_string(),
            target: target.to_string(),
            scope: String::new(),
            enforcement: String::new(),
            notes: String::new(),
        }
    }

    #[test]
    fn migration_moves_legacy_entries_into_default_deck() {
        let mut store = StoreFile::default();
        let legacy = vec![entry("Tash", "塔什"), entry("Finn", "芬恩")];
        assert!(migrate_legacy_entries(&mut store, &legacy));
        assert_eq!(store.decks.len(), 1);
        assert_eq!(store.decks[0].id, DEFAULT_DECK_ID);
        assert_eq!(store.decks[0].entries.len(), 2);
        // 二次迁移不重复
        assert!(!migrate_legacy_entries(&mut store, &legacy));
        // 空 legacy 不迁移
        let mut empty_store = StoreFile::default();
        assert!(!migrate_legacy_entries(&mut empty_store, &[]));
    }

    #[test]
    fn enabled_only_flattening_dedupes_across_decks() {
        let now = now_rfc3339();
        let store = StoreFile {
            decks: vec![
                GlossaryDeck {
                    id: "a".into(),
                    name: "A".into(),
                    enabled: true,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    entries: vec![entry("Tash", "塔什")],
                },
                GlossaryDeck {
                    id: "b".into(),
                    name: "B".into(),
                    enabled: false,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    entries: vec![entry("Hidden", "隐藏"), entry("Tash", "冲突译法")],
                },
                GlossaryDeck {
                    id: "c".into(),
                    name: "C".into(),
                    enabled: true,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    entries: vec![entry("tash", "小写重复"), entry("London", "伦敦")],
                },
            ],
        };
        let entries = collect_enabled_glossary_entries(&store);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].target, "塔什"); // 先出现词表优先
        assert_eq!(entries[1].source, "London");
    }

    #[test]
    fn normalize_drops_empty_and_dedupes() {
        let inputs = vec![
            GlossaryDeckInput {
                source: "Tash ".into(),
                target: "塔什".into(),
                scope: String::new(),
                enforcement: "strict".into(),
                notes: String::new(),
            },
            GlossaryDeckInput {
                source: "".into(),
                target: "无效".into(),
                scope: String::new(),
                enforcement: String::new(),
                notes: String::new(),
            },
            GlossaryDeckInput {
                source: "tash".into(),
                target: "另一译法".into(),
                scope: String::new(),
                enforcement: String::new(),
                notes: String::new(),
            },
        ];
        let normalized = normalize_entries(inputs).expect("normalize");
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].source, "Tash");
        assert_eq!(normalized[0].target, "塔什");
    }

    #[test]
    fn deck_name_validation() {
        assert!(normalize_deck_name("   ").is_err());
        assert!(normalize_deck_name(&"长".repeat(MAX_DECK_NAME_LEN + 1)).is_err());
        assert_eq!(normalize_deck_name(" 小说 ").unwrap(), "小说");
    }

    #[test]
    fn find_deck_missing_returns_not_found() {
        let mut store = StoreFile::default();
        let error = find_deck_mut(&mut store, "nope").expect_err("missing deck");
        assert_eq!(error.code, "NOT_FOUND");
    }

    #[test]
    fn deck_id_selection_respects_enabled_and_selection() {
        let now = now_rfc3339();
        let store = StoreFile {
            decks: vec![
                GlossaryDeck {
                    id: "a".into(),
                    name: "A".into(),
                    enabled: true,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    entries: vec![entry("Tash", "塔什")],
                },
                GlossaryDeck {
                    id: "b".into(),
                    name: "B".into(),
                    enabled: false,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    entries: vec![entry("Disabled", "禁用")],
                },
                GlossaryDeck {
                    id: "c".into(),
                    name: "C".into(),
                    enabled: true,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    entries: vec![entry("London", "伦敦")],
                },
            ],
        };
        // 只选 c
        let selected: HashSet<String> = ["c".to_string()].into_iter().collect();
        let entries = collect_entries_for_deck_ids(&store, Some(&selected));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "London");
        // 选了禁用表 b → 不生效（用户关闭词表优先于单次选择）
        let selected: HashSet<String> = ["b".to_string()].into_iter().collect();
        assert!(collect_entries_for_deck_ids(&store, Some(&selected)).is_empty());
        // None → 全部启用
        assert_eq!(collect_entries_for_deck_ids(&store, None).len(), 2);
    }
}
