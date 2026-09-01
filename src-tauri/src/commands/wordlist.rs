//! 用户词库管理：导入词包（多格式）、持久化、合并为运行期活跃索引。
//!
//! 词库数据**不随应用分发**：应用只带格式与解析器。用户从设置页导入：
//! - 本应用词包 JSON（`entries` 数组，兼容裸 WordEntry 数组）
//! - ECDICT `ecdict.csv`（列含 word/phonetic/definition/translation/tag/…）
//! - 通用 TXT/CSV/TSV 词表（每行 `word[,level[,phonetic[,definition]]]`）
//!
//! 多个词包共存于 `{wordlists_dir}/{id}.json`，按导入时间先后合并
//! （一词多档取最低档、空字段互补，与 `WordLevelIndex::from_entries` 同语义）。

use super::words::parse_user_level;
use crate::error::AppError;
use crate::word_levels::{install_index, WordEntry, WordLevel, WordLevelIndex};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_ENTRIES_PER_PACK: usize = 800_000;
const MAX_DEF_LEN: usize = 600;

/// 磁盘词包（含元数据；entries 为归一后的 WordEntry 列表）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPack {
    id: String,
    name: String,
    /// 来源格式（展示用）：pack-json | ecdict-csv | word-list
    format: String,
    created_at: String,
    entries: Vec<WordEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordlistPackInfo {
    id: String,
    name: String,
    format: String,
    created_at: String,
    word_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordlistStatus {
    /// 活跃索引词条总数（多包合并去重后）。
    active_words: usize,
    /// 是否处于无词库状态（前端空态引导依据）。
    empty: bool,
    packs: Vec<WordlistPackInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordlistImportSummary {
    id: String,
    name: String,
    format: String,
    /// 本次导入有效词条数（去重前）。
    imported: usize,
    /// 被跳过的行数（不合词形规则/无档级且无默认档等）。
    skipped: usize,
    /// 合并后活跃索引总词数。
    active_words: usize,
}

/// 从磁盘词包重建活跃索引（启动装配与导入/删除后调用）。
/// 返回合并去重后的词条总数；目录不存在视为空词库（正常态）。
pub(crate) fn rebuild_active_index(dir: &Path) -> usize {
    let mut packs = read_packs(dir);
    // created_at 为 RFC3339 文本，字典序即时间序。
    packs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let merged = packs
        .into_iter()
        .flat_map(|pack| pack.entries.into_iter())
        .collect::<Vec<_>>();
    let index = WordLevelIndex::from_entries(merged);
    let total = index.len();
    install_index(index);
    total
}

/// 导入词包：path 为用户选择的本地文件；default_level 为无档级词的兜底档。
#[tauri::command]
pub async fn import_wordlist(
    state: tauri::State<'_, crate::commands::AppState>,
    path: String,
    default_level: Option<String>,
    name: Option<String>,
) -> Result<WordlistImportSummary, AppError> {
    let source = PathBuf::from(path.trim());
    if !source.is_file() {
        return Err(AppError::invalid_input("词库文件不存在"));
    }
    let default_level = match default_level.as_deref() {
        Some(label) => Some(parse_user_level(label).ok_or_else(|| {
            AppError::invalid_input(format!("不支持的难度档: {label}"))
        })?),
        None => None,
    };
    let content = std::fs::read_to_string(&source)
        .map_err(|error| AppError::internal(format!("读取词库文件失败: {error}")))?;
    let (format, entries, skipped) = sniff_and_parse(&content, default_level)?;
    if entries.is_empty() {
        return Err(AppError::invalid_input(if skipped > 0 {
            "未解析到有效词条（检查列格式与默认难度档）"
        } else {
            "未解析到有效词条"
        }));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let pack_name = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            source
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| format!("wordlist-{id}"));
    let imported = entries.len();
    let pack = StoredPack {
        id: id.clone(),
        name: pack_name.clone(),
        format: format.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        entries,
    };
    let dir = state.wordlists_dir.clone();
    std::fs::create_dir_all(&dir)
        .map_err(|error| AppError::internal(format!("创建词库目录失败: {error}")))?;
    super::persist_json(&dir.join(format!("{id}.json")), &pack)
        .map_err(|error| AppError::internal(format!("词包写入失败: {error:?}")))?;
    let active_words = rebuild_active_index(&dir);
    Ok(WordlistImportSummary {
        id,
        name: pack_name,
        format,
        imported,
        skipped,
        active_words,
    })
}

/// 已导入词包与活跃规模。
#[tauri::command]
pub async fn wordlist_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<WordlistStatus, AppError> {
    let packs = read_packs(&state.wordlists_dir)
        .into_iter()
        .map(|pack| WordlistPackInfo {
            id: pack.id,
            name: pack.name,
            format: pack.format,
            created_at: pack.created_at,
            word_count: pack.entries.len(),
        })
        .collect();
    let active_words = crate::word_levels::active_index().len();
    Ok(WordlistStatus {
        active_words,
        empty: active_words == 0,
        packs,
    })
}

/// 删除词包并重建活跃索引。
#[tauri::command]
pub async fn delete_wordlist(
    state: tauri::State<'_, crate::commands::AppState>,
    id: String,
) -> Result<WordlistStatus, AppError> {
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(AppError::invalid_input("词包 id 非法"));
    }
    let path = state.wordlists_dir.join(format!("{id}.json"));
    if path.is_file() {
        std::fs::remove_file(&path)
            .map_err(|error| AppError::internal(format!("删除词包失败: {error}")))?;
    }
    let _ = rebuild_active_index(&state.wordlists_dir);
    wordlist_status(state).await
}

fn read_packs(dir: &Path) -> Vec<StoredPack> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<StoredPack>(&raw) {
            Ok(pack) if !pack.entries.is_empty() => packs.push(pack),
            _ => {}
        }
    }
    packs
}

// ===================== 多格式解析 =====================

fn sniff_and_parse(
    content: &str,
    default_level: Option<WordLevel>,
) -> Result<(String, Vec<WordEntry>, usize), AppError> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        let (entries, skipped) = parse_pack_json(trimmed, default_level)?;
        return Ok(("pack-json".to_string(), entries, skipped));
    }
    // CSV 判定：首行含逗号且含 word 字段名 → ECDICT；否则按通用词表解析。
    let first_line = content.lines().next().unwrap_or_default();
    if first_line.contains(',') && first_line.to_ascii_lowercase().starts_with("word") {
        let (entries, skipped) = parse_ecdict_csv(content, default_level)?;
        return Ok(("ecdict-csv".to_string(), entries, skipped));
    }
    let (entries, skipped) = parse_word_list(content, default_level)?;
    Ok(("word-list".to_string(), entries, skipped))
}

/// 词包 JSON：`{"entries":[...]}` 或裸数组 `[WordEntry...]`（与基础词包兼容）。
fn parse_pack_json(
    content: &str,
    default_level: Option<WordLevel>,
) -> Result<(Vec<WordEntry>, usize), AppError> {
    #[derive(Deserialize)]
    struct Envelope {
        entries: Vec<WordEntry>,
    }
    let raw_entries = if let Ok(envelope) = serde_json::from_str::<Envelope>(content) {
        envelope.entries
    } else {
        serde_json::from_str::<Vec<WordEntry>>(content)
            .map_err(|error| AppError::invalid_input(format!("词包 JSON 解析失败: {error}")))?
    };
    let mut skipped = 0usize;
    let mut entries = Vec::with_capacity(raw_entries.len().min(MAX_ENTRIES_PER_PACK));
    for entry in raw_entries {
        if entries.len() >= MAX_ENTRIES_PER_PACK {
            skipped += 1;
            continue;
        }
        if !valid_word(&entry.word) {
            skipped += 1;
            continue;
        }
        let mut entry = entry;
        entry.word = entry.word.to_ascii_lowercase();
        if entry.level.label().is_empty() {
            entry.level = default_level.unwrap_or(WordLevel::ZhongKao);
        }
        entries.push(entry);
    }
    Ok((entries, skipped))
}

/// ECDICT csv：tag（空格分隔，如 "zk gk cet4"）取最低档；无 tag 用默认档。
fn parse_ecdict_csv(
    content: &str,
    default_level: Option<WordLevel>,
) -> Result<(Vec<WordEntry>, usize), AppError> {
    let mut rows = parse_csv_records(content);
    if rows.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let header = rows
        .remove(0)
        .into_iter()
        .map(|cell| cell.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let column = |name: &str| header.iter().position(|cell| cell == name);
    let (Some(i_word), Some(i_def)) = (column("word"), column("definition").or(column("translation")))
    else {
        return Err(AppError::invalid_input(
            "ECDICT CSV 需要至少 word 与 definition/translation 列",
        ));
    };
    let i_phonetic = column("phonetic");
    let i_translation = column("translation");
    let i_tag = column("tag");
    let i_exchange = column("exchange");
    let i_collins = column("collins");
    let i_oxford = column("oxford");
    let i_bnc = column("bnc");
    let i_frq = column("frq");
    let cell = |row: &Vec<String>, idx: Option<usize>| -> String {
        idx.and_then(|idx| row.get(idx))
            .map(|value| value.trim().to_string())
            .unwrap_or_default()
    };
    let mut entries = Vec::new();
    let mut skipped = 0usize;
    for row in rows {
        if entries.len() >= MAX_ENTRIES_PER_PACK {
            skipped += 1;
            continue;
        }
        let word = cell(&row, Some(i_word));
        if !valid_word(&word) {
            skipped += 1;
            continue;
        }
        let tag = cell(&row, i_tag);
        let level = level_from_tag(&tag).or(default_level);
        let Some(level) = level else {
            // 无档级且无默认：收录为最高档之前（考研后按 ielts）不如跳过省内存——
            // 折中：作为 CET6 以上收录会污染透析，直接跳过。
            skipped += 1;
            continue;
        };
        entries.push(WordEntry {
            word: word.to_ascii_lowercase(),
            level,
            phonetic: cell(&row, i_phonetic),
            definition: truncate_def(cell(&row, Some(i_def))),
            translation: truncate_def(cell(&row, i_translation)),
            tag,
            exchange: cell(&row, i_exchange),
            root: String::new(),
            collins: cell(&row, i_collins).parse().unwrap_or_default(),
            oxford: cell(&row, i_oxford).parse().unwrap_or_default(),
            bnc: cell(&row, i_bnc).parse().unwrap_or_default(),
            frq: cell(&row, i_frq).parse().unwrap_or_default(),
        });
    }
    Ok((entries, skipped))
}

/// ECDICT tag → 最低考试档（与 from_entries 的取低语义一致）。
fn level_from_tag(tag: &str) -> Option<WordLevel> {
    let mut best: Option<WordLevel> = None;
    for token in tag.split_whitespace() {
        let Some(level) = (match token {
            "zk" => Some(WordLevel::ZhongKao),
            "gk" => Some(WordLevel::GaoKao),
            "cet4" => Some(WordLevel::Cet4),
            "cet6" => Some(WordLevel::Cet6),
            "ky" => Some(WordLevel::KaoYan),
            "ielts" => Some(WordLevel::Ielts),
            "toefl" => Some(WordLevel::Toefl),
            "tem4" => Some(WordLevel::Tem4),
            "tem8" => Some(WordLevel::Tem8),
            "gre" | "sat" | "gmat" => Some(WordLevel::Gre),
            _ => None,
        }) else {
            continue;
        };
        best = Some(best.map_or(level, |current| current.min(level)));
    }
    best
}

/// 词表首行表头启发：第一格为常见表头词（word/english/term/单词…）。
/// 真实词表首词恰为 "word" 的概率极低，误判损失一行，可接受。
fn looks_like_wordlist_header(first_cell: &str) -> bool {
    matches!(
        first_cell.trim().to_ascii_lowercase().as_str(),
        "word" | "words" | "english" | "term" | "vocabulary" | "单词" | "词汇" | "词表"
    )
}

/// 通用词表：每行 word[,level[,phonetic[,definition]]]（自动识别 , ; \t 分隔），
/// 或每行一个单词 + 调用方 default_level。含表头自动跳过。
fn parse_word_list(
    content: &str,
    default_level: Option<WordLevel>,
) -> Result<(Vec<WordEntry>, usize), AppError> {
    let delimiter = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| {
            if line.contains('\t') {
                '\t'
            } else if line.contains(';') {
                ';'
            } else {
                ','
            }
        })
        .unwrap_or(',');
    let mut entries = Vec::new();
    let mut skipped = 0usize;
    for (index, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let first_cell = line.split(delimiter).next().unwrap_or("").trim();
        if index == 0 && looks_like_wordlist_header(first_cell) {
            continue;
        }
        if entries.len() >= MAX_ENTRIES_PER_PACK {
            skipped += 1;
            continue;
        }
        if !valid_word(first_cell) {
            skipped += 1;
            continue;
        }
        let mut parts = line.split(delimiter).map(|part| part.trim());
        let word = parts.next().unwrap_or_default().to_ascii_lowercase();
        let level = parts
            .next()
            .filter(|value| !value.is_empty())
            .and_then(parse_user_level)
            .or(default_level);
        let Some(level) = level else {
            skipped += 1;
            continue;
        };
        let phonetic = parts.next().unwrap_or_default().to_string();
        let definition = truncate_def(parts.next().unwrap_or_default().to_string());
        entries.push(WordEntry {
            word,
            level,
            phonetic,
            definition,
            ..Default::default()
        });
    }
    Ok((entries, skipped))
}

/// 单词形态校验：纯字母（可含内部 ' -），长度 1–32，不含空格。
fn valid_word(word: &str) -> bool {
    let word = word.trim();
    (1..=32).contains(&word.len())
        && !word.contains(' ')
        && word.chars().all(|ch| ch.is_ascii_alphabetic() || ch == '\'' || ch == '-')
        && word.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn truncate_def(text: String) -> String {
    if text.chars().count() > MAX_DEF_LEN {
        text.chars().take(MAX_DEF_LEN).collect()
    } else {
        text
    }
}

/// 最小 RFC4180 CSV：双引号包裹、"" 转义、字段内换行。
fn parse_csv_records(content: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                row.push(std::mem::take(&mut field));
            }
            '\n' | '\r' if !in_quotes => {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                if row.iter().any(|cell| !cell.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            other => field.push(other),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|cell| !cell.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn entry_words(entries: &[WordEntry]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|entry| (entry.word.clone(), entry.level.label().to_string()))
            .collect()
    }

    #[test]
    fn plain_word_list_parses_with_levels_and_defaults() {
        let content =
            "word,level,phonetic,definition\nabandon,高考,/əˈbændən/,v. 放弃\nserendipity,,,\nhappy,GRE,,\n";
        let (entries, skipped) = parse_word_list(content, Some(WordLevel::Cet4)).expect("list");
        assert_eq!(skipped, 0);
        let words = entry_words(&entries);
        assert_eq!(words.get("abandon").map(String::as_str), Some("高考"));
        // 无档级列 → 默认档
        assert_eq!(words.get("serendipity").map(String::as_str), Some("四级"));
        // 内联档级第三列写法（GRE）
        assert_eq!(words.get("happy").map(String::as_str), Some("GRE"));
        let abandon = entries.iter().find(|e| e.word == "abandon").expect("abandon");
        assert_eq!(abandon.phonetic, "/əˈbændən/");
        assert_eq!(abandon.definition, "v. 放弃");
    }

    #[test]
    fn one_word_per_line_uses_default_level() {
        let (entries, skipped) = parse_word_list("apple\nzeal\nrun", Some(WordLevel::Ielts))
            .expect("lines");
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|entry| entry.level == WordLevel::Ielts));
    }

    #[test]
    fn invalid_word_forms_are_skipped() {
        // 空、含空格、数字开头、纯符号都不得进词库（避免污染行内标注）。
        let (entries, skipped) = parse_word_list(
            "good\nbad word\n3d\n--\nokay-ish",
            Some(WordLevel::Cet4),
        )
        .expect("mixed");
        assert_eq!(skipped, 3, "bad word / 3d / -- 应跳过");
        assert!(entries.iter().any(|e| e.word == "good"));
        assert!(entries.iter().any(|e| e.word == "okay-ish"));
    }

    #[test]
    fn ecdict_csv_maps_tags_and_handles_quoted_commas() {
        let header = "word,phonetic,definition,translation,pos,collins,oxford,tag,bnc,frq,exchange,detail,audio";
        // 列序：word,phonetic,definition,translation,pos,collins,oxford,tag,bnc,frq,exchange,detail,audio（共 13 列）
        // abandon：tag "gk cet4" 取最低档；引号内含逗号+换行的 translation 不得拆列。
        let abandon = "abandon,/,v. 放弃,\"vt. 放弃, 抛弃\\n[计] 放弃\",,3,,gk cet4,2057,2182,d:abandoned,,";
        // knife：translation 含逗号，若解析器错误拆列则 tag/collins 会全部错位。
        let knife = "knife,/naɪf/,n. 刀,\"a quoted, comma\",,4,,gk,100,200,d:knives,,";
        // zep：无 tag 且未给默认档 → 跳过。
        let zep = "zep,/,,n. 无档,,,,,,,,";
        let content = format!("{header}\n{abandon}\n{knife}\n{zep}\n");
        let (entries, skipped) = parse_ecdict_csv(&content, None).expect("ecdict");
        assert_eq!(skipped, 1, "只有 zep 应被跳过");
        let words = entry_words(&entries);
        assert_eq!(words.get("abandon").map(String::as_str), Some("高考"));
        let abandon_entry = entries.iter().find(|e| e.word == "abandon").expect("abandon");
        assert_eq!(abandon_entry.collins, 3);
        assert_eq!(abandon_entry.exchange, "d:abandoned");
        let knife_entry = entries.iter().find(|e| e.word == "knife").expect("knife");
        assert_eq!(knife_entry.translation, "a quoted, comma", "引号内逗号不拆列");
        assert_eq!(knife_entry.collins, 4);
        assert_eq!(knife_entry.level.label(), "高考");
    }

    #[test]
    fn pack_json_accepts_envelope_and_bare_array() {
        let envelope = r#"{"entries":[{"word":"Abandon","level":"cet4","definition":"v. 放弃"}]}"#;
        let (entries, skipped) = parse_pack_json(envelope, None).expect("envelope");
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].word, "abandon");
        let bare = r#"[{"word":"zeal","level":"gre"}]"#;
        let (entries, _) = parse_pack_json(bare, None).expect("bare");
        assert_eq!(entries[0].word, "zeal");
    }

    #[test]
    fn sniff_routes_by_content_shape() {
        let (format, _, _) = sniff_and_parse(r#"[{"word":"a","level":"cet4"}]"#, None).expect("json");
        assert_eq!(format, "pack-json");
        let (format, _, _) =
            sniff_and_parse("word,phonetic,definition\nabandon,/,v.\n", None).expect("csv");
        assert_eq!(format, "ecdict-csv");
        let (format, _, _) = sniff_and_parse("abandon\nserendipity\n", None).expect("list");
        assert_eq!(format, "word-list");
    }

    #[test]
    fn valid_word_rules() {
        assert!(valid_word("run"));
        assert!(valid_word("well-known"));
        assert!(valid_word("it's"));
        assert!(!valid_word("a b"));
        assert!(!valid_word(""));
        assert!(!valid_word("3d"));
        assert!(!valid_word(&"x".repeat(33)));
    }

    #[test]
    fn rebuild_merges_packs_lowest_level_and_reinstalls() {
        let dir = std::env::temp_dir().join(format!(
            "musetranslate-wordlist-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let pack = |id: &str, created: &str, entries: Vec<WordEntry>| StoredPack {
            id: id.to_string(),
            name: format!("pack-{id}"),
            format: "pack-json".to_string(),
            created_at: created.to_string(),
            entries,
        };
        let entry = |word: &str, level: WordLevel, definition: &str| WordEntry {
            word: word.to_string(),
            level,
            definition: definition.to_string(),
            ..Default::default()
        };
        let first = pack(
            "a",
            "2026-01-01T00:00:00Z",
            vec![entry("abandon", WordLevel::Gre, "GRE 释义")],
        );
        let second = pack(
            "b",
            "2026-01-02T00:00:00Z",
            vec![entry("abandon", WordLevel::GaoKao, ""), entry("zeal", WordLevel::Ielts, "n. 热情")],
        );
        super::super::persist_json(&dir.join("a.json"), &first).expect("a");
        super::super::persist_json(&dir.join("b.json"), &second).expect("b");

        let total = rebuild_active_index(&dir);
        assert_eq!(total, 2, "去重合并后 abandon+zeal");
        let index = crate::word_levels::active_index();
        let abandon = index.entry_of("abandon").expect("abandon");
        assert_eq!(abandon.level, WordLevel::GaoKao, "一词多包取最低档");
        assert_eq!(abandon.definition, "GRE 释义", "空字段不覆盖已有释义");
        assert!(index.entry_of("zeal").is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
