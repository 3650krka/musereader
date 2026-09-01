//! 术语表读取命令：把任务 artifact 的 glossary.json 暴露给前端术语表视图。
//!
//! 数据来自翻译管线的首次锁定机制（first-seen lock），entries 含
//! source/translation/category/occurrences 等。本命令只做只读投影。

use super::{resolve_runtime_paths, AppError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub source: String,
    pub translation: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub occurrences: u32,
    /// 来源：static=用户词库 / dynamic=翻译中锁定 / user=用户在术语页新增
    #[serde(default)]
    pub origin: String,
}

#[derive(Debug, Deserialize)]
struct GlossaryFile {
    #[serde(default)]
    entries: Vec<GlossaryEntryWire>,
}

#[derive(Debug, Deserialize)]
struct GlossaryEntryWire {
    source: String,
    translation: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    occurrences: u32,
    #[serde(default)]
    origin: String,
}

/// 用户修订条目（术语表内调整译法 / 新增术语）：只存 source→translation，
/// 不动 artifact 的 glossary.json 本体，读取时投影叠加。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryOverrideInput {
    pub source: String,
    pub translation: String,
    /// source 重命名来源：
    /// Some(old) 表示把 artifact 中 old 行改名为 source；None 为普通译文修订。
    #[serde(default)]
    pub rename_from: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OverridesFile {
    #[serde(default)]
    overrides: Vec<GlossaryOverrideInput>,
}

#[tauri::command]
pub async fn list_glossary_entries(
    task_id: String,
    overrides: Option<Vec<GlossaryOverrideInput>>,
) -> Result<Vec<GlossaryEntry>, AppError> {
    let path = resolve_runtime_paths()
        .artifact_root_dir
        .join(&task_id)
        .join("glossary.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|error| AppError {
        code: "glossary_read_failed".into(),
        message: format!("读取术语表失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    let parsed: GlossaryFile = serde_json::from_str(&raw).map_err(|error| AppError {
        code: "glossary_parse_failed".into(),
        message: format!("术语表格式损坏：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    let merged = overrides.unwrap_or_else(|| load_artifact_overrides(&task_id));
    let mut entries: Vec<GlossaryEntry> = parsed
        .entries
        .into_iter()
        .map(|e| GlossaryEntry {
            source: e.source,
            translation: e.translation,
            category: e.category,
            occurrences: e.occurrences,
            origin: e.origin,
        })
        .collect();
    apply_overrides(&mut entries, &merged);
    /* 按原文去重：同一 source 可能被多轮抽取为不同类别/上下文（如 "Abi" 既作
       人名又作严格术语），术语表逐行展示时会出现重复行，削弱“识别精准”的观感。
       保留出现次数最高的一条（同次数保留先出现者），使每个原文只占一行。 */
    let mut deduped: Vec<GlossaryEntry> = Vec::with_capacity(entries.len());
    let mut index_by_source: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in entries {
        let key = entry.source.to_ascii_lowercase();
        match index_by_source.get(&key) {
            Some(&idx) => {
                if entry.occurrences > deduped[idx].occurrences {
                    deduped[idx] = entry;
                }
            }
            None => {
                index_by_source.insert(key, deduped.len());
                deduped.push(entry);
            }
        }
    }
    let mut entries = deduped;
    entries.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then(a.source.cmp(&b.source))
    });
    Ok(entries)
}

/// 叠加用户修订：已存在条目替换译文，未命中的修订条目作为 user 新增追加。
fn apply_overrides(entries: &mut Vec<GlossaryEntry>, overrides: &[GlossaryOverrideInput]) {
    // 重命名先行：rename_from 命中的行改 source，后续同轮正常替换译文。
    for rev in overrides {
        if let Some(old) = rev.rename_from.as_deref().filter(|old| !old.is_empty()) {
            for entry in entries.iter_mut() {
                if entry.source.eq_ignore_ascii_case(old) {
                    entry.source = rev.source.clone();
                    entry.origin = "user".to_string();
                }
            }
        }
    }
    let mut matched = std::collections::HashSet::new();
    for entry in entries.iter_mut() {
        if let Some(rev) = overrides
            .iter()
            .find(|rev| rev.source.eq_ignore_ascii_case(&entry.source))
        {
            entry.translation = rev.translation.clone();
            matched.insert(rev.source.to_ascii_lowercase());
        }
    }
    for rev in overrides {
        if matched.insert(rev.source.to_ascii_lowercase()) {
            entries.push(GlossaryEntry {
                source: rev.source.clone(),
                translation: rev.translation.clone(),
                category: String::new(),
                occurrences: 0,
                origin: "user".to_string(),
            });
        }
    }
}

fn overrides_path(task_id: &str) -> std::path::PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .join(task_id)
        .join("glossary_overrides.json")
}

fn load_artifact_overrides(task_id: &str) -> Vec<GlossaryOverrideInput> {
    let path = overrides_path(task_id);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<OverridesFile>(&raw) else {
        return Vec::new();
    };
    normalize_overrides(parsed.overrides)
}

fn normalize_overrides(overrides: Vec<GlossaryOverrideInput>) -> Vec<GlossaryOverrideInput> {
    let mut normalized = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in overrides {
        let source = entry.source.trim().to_string();
        let translation = entry.translation.trim().to_string();
        if source.is_empty() || translation.is_empty() {
            continue;
        }
        if seen.insert(source.to_ascii_lowercase()) {
            normalized.push(GlossaryOverrideInput { source, translation, rename_from: entry.rename_from });
        }
    }
    normalized
}

/// 保存术语修订（整表替换）：不修改 artifact glossary.json，只写旁路 overrides 文件。
#[tauri::command]
pub async fn save_artifact_glossary_overrides(
    task_id: String,
    overrides: Vec<GlossaryOverrideInput>,
) -> Result<usize, AppError> {
    let normalized = normalize_overrides(overrides);
    let count = normalized.len();
    let path = overrides_path(&task_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| AppError {
            code: "glossary_override_failed".into(),
            message: format!("创建术语修订目录失败：{error}"),
            retryable: false,
            retry_after_ms: None,
        })?;
    }
    let payload = serde_json::to_string_pretty(&OverridesFile {
        overrides: normalized,
    })
    .map_err(|error| AppError {
        code: "glossary_override_failed".into(),
        message: format!("序列化术语修订失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    let temp_path = path.with_extension("json.tmp");
    std::fs::write(&temp_path, payload.as_bytes()).map_err(|error| AppError {
        code: "glossary_override_failed".into(),
        message: format!("写入术语修订失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    std::fs::rename(&temp_path, &path).map_err(|error| AppError {
        code: "glossary_override_failed".into(),
        message: format!("替换术语修订失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    Ok(count)
}


// ---- 用户术语表编辑（全局静态词表，随 runtime_config.json 持久化） ----

/// 用户术语条目：source/target 为原文与固定译法；enforcement 空或 strict=强制一致，
/// contextual=仅上下文参考（与 pipeline prompt 格式化层语义一致）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserGlossaryInput {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub enforcement: String,
    #[serde(default)]
    pub notes: String,
}

const MAX_USER_GLOSSARY_ENTRIES: usize = 2_000;

/// 用户术语条目归一：去空白、丢空行、按原文去重（首次译法为准，
/// 与管线 first-seen 锁定语义一致）、容量上限保护。
pub(crate) fn normalize_user_glossary(
    entries: Vec<UserGlossaryInput>,
) -> Result<Vec<super::config::StaticGlossaryEntry>, super::AppError> {
    let mut normalized: Vec<super::config::StaticGlossaryEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let source = entry.source.trim().to_string();
        let target = entry.target.trim().to_string();
        if source.is_empty() || target.is_empty() {
            continue;
        }
        if !seen.insert(source.to_ascii_lowercase()) {
            continue;
        }
        normalized.push(super::config::StaticGlossaryEntry {
            source,
            target,
            scope: entry.scope.trim().to_string(),
            enforcement: entry.enforcement.trim().to_string(),
            notes: entry.notes.trim().to_string(),
        });
        if normalized.len() >= MAX_USER_GLOSSARY_ENTRIES {
            return Err(super::AppError::invalid_input(format!(
                "术语条目超出上限 {MAX_USER_GLOSSARY_ENTRIES} 条"
            )));
        }
    }
    Ok(normalized)
}

/// 保存用户术语表（整表替换）：条目去重归一后写入运行配置并原子持久化。
/// 新任务启动时经 static_glossary_entries 流入翻译管线 prompt。
#[tauri::command]
pub async fn save_user_glossary(
    entries: Vec<UserGlossaryInput>,
    state: tauri::State<'_, super::AppState>,
) -> Result<usize, super::AppError> {
    let normalized = normalize_user_glossary(entries)?;
    let entry_count = normalized.len();
    let mut config = super::lock_mutex(&state.config, "运行配置")?.clone();
    let previous = config.clone();
    config.static_glossary_entries = normalized;
    if let Err(error) = super::persist_json(&state.runtime_config_store_path, &config) {
        // 持久化失败回滚内存副本，配置与磁盘保持一致
        *super::lock_mutex(&state.config, "运行配置")? = previous;
        return Err(error);
    }
    *super::lock_mutex(&state.config, "运行配置")? = config;
    Ok(entry_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(source: &str, target: &str, enforcement: &str) -> UserGlossaryInput {
        UserGlossaryInput {
            source: source.to_string(),
            target: target.to_string(),
            scope: String::new(),
            enforcement: enforcement.to_string(),
            notes: String::new(),
        }
    }

    #[test]
    fn normalize_user_glossary_drops_empty_and_dedupes_by_source() {
        let entries = vec![
            input("Tash ", "塔什", "strict"),
            input("", "无效", "strict"),
            input("Tash", "另一个译法", "strict"),
            input("LONDON", "伦敦", "contextual"),
            input("London", "重复源", "strict"),
        ];
        let normalized = normalize_user_glossary(entries).expect("should normalize");
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].source, "Tash");
        assert_eq!(normalized[0].target, "塔什"); // 首次译法为准
        assert_eq!(normalized[1].enforcement, "contextual");
    }

    #[test]
    fn normalize_user_glossary_enforces_capacity_limit() {
        let entries = (0..(MAX_USER_GLOSSARY_ENTRIES + 1))
            .map(|i| input(&format!("word-{i}"), "译", "strict"))
            .collect();
        let error = normalize_user_glossary(entries).expect_err("should reject overflow");
        assert_eq!(error.code, "INVALID_INPUT");
    }


    #[test]
    fn parses_glossary_entries_and_sorts_by_occurrences() {
        let raw = r#"{
            "articleType": "fiction",
            "entries": [
                {"source":"Tash","translation":"塔什","category":"person_name","occurrences":1},
                {"source":"Finn","translation":"芬恩","category":"person_name","occurrences":5}
            ]
        }"#;
        let parsed: GlossaryFile = serde_json::from_str(raw).expect("glossary should parse");
        let mut entries: Vec<GlossaryEntry> = parsed
            .entries
            .into_iter()
            .map(|e| GlossaryEntry {
                source: e.source,
                translation: e.translation,
                category: e.category,
                occurrences: e.occurrences,
                origin: e.origin,
            })
            .collect();
        entries.sort_by(|a, b| {
            b.occurrences
                .cmp(&a.occurrences)
                .then(a.source.cmp(&b.source))
        });
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source, "Finn");
        assert_eq!(entries[1].translation, "塔什");
    }

    #[test]
    fn tolerates_missing_optional_fields() {
        let raw = r#"{"entries":[{"source":"London","translation":"伦敦"}]}"#;
        let parsed: GlossaryFile = serde_json::from_str(raw).expect("minimal entry should parse");
        assert_eq!(parsed.entries[0].category, "");
        assert_eq!(parsed.entries[0].occurrences, 0);
    }

    #[test]
    fn overrides_replace_translation_and_append_new_entries() {
        let mut entries = vec![
            GlossaryEntry {
                source: "Tash".into(),
                translation: "塔什".into(),
                category: "person_name".into(),
                occurrences: 5,
                origin: "dynamic".into(),
            },
            GlossaryEntry {
                source: "Finn".into(),
                translation: "芬恩".into(),
                category: "person_name".into(),
                occurrences: 2,
                origin: "dynamic".into(),
            },
        ];
        let overrides = vec![
            GlossaryOverrideInput { source: "tash".into(), translation: "塔什（修订）".into(), rename_from: None },
            GlossaryOverrideInput { source: "NewTerm".into(), translation: "新术语".into(), rename_from: None },
        ];
        apply_overrides(&mut entries, &overrides);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].translation, "塔什（修订）");
        assert_eq!(entries[2].source, "NewTerm");
        assert_eq!(entries[2].origin, "user");
    }

    #[test]
    fn apply_overrides_renames_source_and_marks_user() {
        let mut entries = vec![GlossaryEntry {
            source: "Old Term".into(),
            translation: "旧译".into(),
            category: String::new(),
            occurrences: 3,
            origin: "dynamic".into(),
        }];
        apply_overrides(
            &mut entries,
            &[GlossaryOverrideInput {
                source: "New Term".into(),
                translation: "旧译".into(),
                rename_from: Some("Old Term".into()),
            }],
        );
        assert_eq!(entries[0].source, "New Term");
        assert_eq!(entries[0].origin, "user");
        assert_eq!(entries[0].occurrences, 3); // 重命名保留词频
    }

    #[test]
    fn normalize_overrides_drops_empty_and_dedupes() {
        let normalized = normalize_overrides(vec![
            GlossaryOverrideInput { source: " Tash ".into(), translation: "塔什".into(), rename_from: None },
            GlossaryOverrideInput { source: "".into(), translation: "空源".into(), rename_from: None },
            GlossaryOverrideInput { source: "Tash".into(), translation: "重复".into(), rename_from: None },
            GlossaryOverrideInput { source: "Finn".into(), translation: "  ".into(), rename_from: None },
        ]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].source, "Tash");
        assert_eq!(normalized[0].translation, "塔什");
    }
}
