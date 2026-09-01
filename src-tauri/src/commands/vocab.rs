//! 词汇透析背诵卡片：SM-2 间隔重复调度 + Anki 文本导出。
//!
//! 数据模型：
//! - 每本书一套卡片集合（ReaderState.vocab_cards，word 小写为键）
//! - 卡片调度字段（SM-2）：repetitions / interval_days / ease_factor / due_at
//! - 牌面内容（音标/释义/档级/例句）查询时由 word_levels 全量表现取——
//!   词表升级后牌面自动升级，存储层不含冗余文本。
//!
//! SM-2 参考 SuperMemo 2 经典算法（quality 0-5 → <3 重学, ≥3 间隔增长）。
//! 我们的前端评分是 忘记(0)/困难(3)/记住(5) 三档，映射到 SM-2 quality。

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MIN_EASE_FACTOR: f32 = 1.3;
pub const DEFAULT_EASE_FACTOR: f32 = 2.5;
const MAX_VOCAB_CARDS: usize = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabCard {
    pub word: String,
    /// 首次成卡时的语境句（例句兜底；词表释义缺失时也用它）。
    #[serde(default)]
    pub context: String,
    /// 例句译文（双语书成卡时由前端从配对块提取）。
    #[serde(default)]
    pub context_zh: String,
    /// 成卡来源章节（分章背诵过滤用）。
    #[serde(default)]
    pub chapter: String,
    pub created_at: String,
    /// AI 语境释义覆盖（成卡后保存的语境义，优先于词表默认释义）。
    #[serde(default)]
    pub custom_definition: String,
    /// 用户单词笔记。
    #[serde(default)]
    pub note: String,
    /// SM-2 调度字段
    #[serde(default)]
    pub repetitions: u32,
    #[serde(default)]
    pub interval_days: f32,
    #[serde(default = "default_ease")]
    pub ease_factor: f32,
    /// ISO8601；None = 新卡（立即可学）。
    #[serde(default)]
    pub due_at: Option<String>,
}

fn default_ease() -> f32 {
    DEFAULT_EASE_FACTOR
}

impl VocabCard {
    pub fn new(word: &str, context: String, chapter: String) -> Self {
        Self {
            word: word.trim().to_ascii_lowercase(),
            context,
            context_zh: String::new(),
            chapter,
            created_at: Utc::now().to_rfc3339(),
            custom_definition: String::new(),
            note: String::new(),
            repetitions: 0,
            interval_days: 0.0,
            ease_factor: DEFAULT_EASE_FACTOR,
            due_at: None,
        }
    }

    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        match &self.due_at {
            None => true,
            Some(due) => DateTime::parse_from_rfc3339(due)
                .map(|due| due.with_timezone(&Utc) <= now)
                .unwrap_or(true), // 坏时间戳按到期处理，避免卡片永久消失
        }
    }
}

/// SM-2 复习评分：quality ∈ [0,5]；<3 重学（rep 归零、间隔 10 分钟用 0.007 天近似），
/// ≥3 按 1天→6天→间隔×EF 增长，EF 随表现浮动、下限 1.3。
pub fn sm2_review(card: &mut VocabCard, quality: u8, now: DateTime<Utc>) {
    let quality = quality.min(5);
    if quality < 3 {
        card.repetitions = 0;
        card.interval_days = 0.007; // ~10 分钟后再见
    } else {
        card.repetitions += 1;
        card.interval_days = match card.repetitions {
            1 => 1.0,
            2 => 6.0,
            _ => (card.interval_days * card.ease_factor).max(1.0),
        };
    }
    let delta = 0.1 - (5.0 - quality as f32) * (0.08 + (5.0 - quality as f32) * 0.02);
    card.ease_factor = (card.ease_factor + delta).max(MIN_EASE_FACTOR);
    let due = now + Duration::milliseconds((card.interval_days * 86_400_000.0) as i64);
    card.due_at = Some(due.to_rfc3339());
}

/// 成卡：已存在则刷新语境/章节（不重置调度）；超出容量时返回 Err。
pub fn upsert_card(
    cards: &mut HashMap<String, VocabCard>,
    word: &str,
    context: String,
    chapter: String,
) -> Result<(), &'static str> {
    let key = word.trim().to_ascii_lowercase();
    if key.is_empty() {
        return Err("empty word");
    }
    match cards.get_mut(&key) {
        Some(existing) => {
            if !context.is_empty() {
                existing.context = context;
            }
            if !chapter.is_empty() {
                existing.chapter = chapter;
            }
        }
        None => {
            if cards.len() >= MAX_VOCAB_CARDS {
                return Err("vocab card capacity reached");
            }
            cards.insert(key.clone(), VocabCard::new(&key, context, chapter));
        }
    }
    Ok(())
}

pub fn remove_card(cards: &mut HashMap<String, VocabCard>, word: &str) -> bool {
    cards.remove(&word.trim().to_ascii_lowercase()).is_some()
}

/// 到期卡片（分章过滤可选）：新卡优先（按创建序），其次按到期时间升序。
pub fn due_cards<'a>(
    cards: &'a HashMap<String, VocabCard>,
    chapter: Option<&str>,
    now: DateTime<Utc>,
) -> Vec<&'a VocabCard> {
    let mut due: Vec<&VocabCard> = cards
        .values()
        .filter(|card| chapter.map_or(true, |ch| card.chapter == ch))
        .filter(|card| card.is_due(now))
        .collect();
    due.sort_by(|left, right| match (&left.due_at, &right.due_at) {
        (None, None) => left.created_at.cmp(&right.created_at),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => a.cmp(b),
    });
    due
}

/// Anki 文本导入格式（TSV：Front<TAB>Back<TAB>Tags），Anki 桌面「文件→导入」直接可用。
/// 牌面 HTML 极简：词+音标正面，释义+例句背面。
pub fn export_anki_tsv(entries: &[AnkiCardPayload], deck_tag: &str) -> String {
    entries
        .iter()
        .map(|entry| {
            let front = anki_escape(&format!(
                "{}{}",
                entry.word,
                if entry.phonetic.is_empty() {
                    String::new()
                } else {
                    format!(" <span class=\"phonetic\">{}</span>", entry.phonetic)
                }
            ));
            let mut back = anki_escape(&entry.definition);
            if !entry.context.is_empty() {
                back.push_str(&format!("<br><i>{}</i>", anki_escape(&entry.context)));
            }
            let tag = anki_escape(&format!("{} {}", deck_tag, entry.level).trim().to_string());
            format!("{front}\t{back}\t{tag}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct AnkiCardPayload {
    pub word: String,
    pub phonetic: String,
    pub definition: String,
    pub context: String,
    pub level: String,
}

/// TSV 安全：换行折叠为空格（Anki 一行一卡），制表符转义。
fn anki_escape(text: &str) -> String {
    text.replace('\t', " ")
        .replace('\r', " ")
        .replace('\n', "<br>")
        .trim()
        .to_string()
}

// ---- Tauri 命令：卡片 CRUD + 复习调度 + Anki 导出 ----

use super::{AppError, AppState};
use crate::commands::reader::storage::mutate_reader_state;
use crate::word_levels::active_index;

/// 牌面（查询时由全量词表填充）：正面词+音标，背面释义+例句。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabCardFace {
    pub word: String,
    pub phonetic: String,
    pub definition: String,
    pub level: String,
    pub context: String,
    /// 例句译文（双语书成卡时提取）。
    pub context_zh: String,
    pub chapter: String,
    pub repetitions: u32,
    pub interval_days: f32,
    /// 前端四档评分的下次间隔预览需要 EF（本地重放 SM-2）。
    pub ease_factor: f32,
    pub due_at: Option<String>,
    // —— 以下取自词库 entry，供背诵卡「背面」分层展示（与 WordWise 卡同源）——
    /// 完整中文翻译（词性分段，通常比 definition 更全）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub translation: String,
    /// 考试标签（zk/gk/cet4/…，空格分隔，前端解码成 chips）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    /// 词形变换编码（d/p/i/3/s/r/t，前端解码）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub exchange: String,
    /// 词根助记（最多两条，分号分隔）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    /// 柯林斯星级 1–5（0=无）。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub collins: u8,
    /// 牛津3000（0/1）。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub oxford: u8,
    /// BNC 词频排名（0=无）。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub bnc: u32,
    /// COCA 词频排名（0=无）。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub frq: u32,
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// 成卡（词透析：从生词标记/提取结果一键加入背诵）。已存在则刷新语境不重置调度。
/// customDefinition（AI 语境义）随卡保存，牌面展示优先于词表默认释义。
/// contextZh：双语书例句译文（由前端从配对块提取），随语境一并刷新。
#[tauri::command]
pub async fn add_vocab_card(
    book_id: String,
    word: String,
    context: Option<String>,
    chapter: Option<String>,
    custom_definition: Option<String>,
    context_zh: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<super::ReaderState, AppError> {
    let word_key = word.trim().to_ascii_lowercase();
    if word_key.is_empty() {
        return Err(AppError::invalid_input("卡片单词为空"));
    }
    mutate_reader_state(&state, &book_id, |reader_state| {
        if upsert_card(
            &mut reader_state.vocab_cards,
            &word_key,
            context.unwrap_or_default(),
            chapter.unwrap_or_default(),
        )
        .is_ok()
        {
            if let Some(card) = reader_state.vocab_cards.get_mut(&word_key) {
                if let Some(definition) = custom_definition.filter(|d| !d.trim().is_empty()) {
                    card.custom_definition = definition.trim().to_string();
                }
                if let Some(zh) = context_zh.filter(|d| !d.trim().is_empty()) {
                    card.context_zh = zh.trim().to_string();
                }
            }
            /* 互斥：入本即撤"已掌握"标记——白名单在后端过滤序上优先于
               learning 强制标注，mastered 不清则该词的 WordWise 永不出现
               （"加入生词本后不标注"的根因）。learning 标记与生词本语义
               同向（都要学），保留不动。 */
            if reader_state
                .word_marks
                .get(&word_key)
                .is_some_and(|mark| mark.status == super::reader::WordMarkStatus::Mastered)
            {
                crate::commands::reader::storage::remove_word_mark(reader_state, &word_key);
            }
        }
        reader_state.updated_at = chrono::Utc::now().to_rfc3339();
    })
}

/// WordWise 自动成卡种子：章节标注命中的超档生词 + 首现语境句 + 成卡章节。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordWiseSeed {
    pub word: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub chapter: String,
}

/// 播种核心：只新增词库中不存在的词（蒙哥式透析语义——WordWise 识别到的
/// 超档生词自动成为该书生词库成员）；已有卡（手动添加/已复习）的语境、
/// 译文与 SM-2 调度一律保留不动。返回实际新增数。
fn seed_cards(cards: &mut HashMap<String, VocabCard>, seeds: &[WordWiseSeed]) -> usize {
    let mut added = 0;
    for seed in seeds {
        let key = seed.word.trim().to_ascii_lowercase();
        if key.is_empty() || cards.contains_key(&key) {
            continue;
        }
        if cards.len() >= MAX_VOCAB_CARDS {
            break;
        }
        cards.insert(key.clone(), VocabCard::new(&key, seed.context.clone(), seed.chapter.clone()));
        added += 1;
    }
    added
}

/// WordWise 自动入生词库：阅读器章节标注完成后批量播种超档词。
/// 幂等：重复同步同一批词不会改动任何已有卡。
#[tauri::command]
pub async fn sync_wordwise_vocab(
    book_id: String,
    seeds: Vec<WordWiseSeed>,
    state: tauri::State<'_, AppState>,
) -> Result<super::ReaderState, AppError> {
    mutate_reader_state(&state, &book_id, |reader_state| {
        seed_cards(&mut reader_state.vocab_cards, &seeds);
        reader_state.updated_at = chrono::Utc::now().to_rfc3339();
    })
}

/// 整本语境句截取：在块文本里找词（独立单词边界，大小写不敏感），
/// 取前 60/后 180 字窗口作播种例句；找不到退化为块首截断。
fn context_sentence_around(text: &str, word: &str) -> String {
    if text.trim().is_empty() || word.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_ascii_lowercase()).collect();
    let needle: Vec<char> = word.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut hit: Option<usize> = None;
    if needle.len() <= lower.len() {
        for start in 0..=(lower.len() - needle.len()) {
            let before_ok = start == 0 || !lower[start - 1].is_ascii_alphabetic();
            let end = start + needle.len();
            let after_ok = end >= lower.len() || !lower[end].is_ascii_alphabetic();
            if before_ok && after_ok && lower[start..end] == needle[..] {
                hit = Some(start);
                break;
            }
        }
    }
    match hit {
        Some(index) => {
            let from = index.saturating_sub(60);
            let to = (index + needle.len() + 180).min(chars.len());
            chars[from..to].iter().collect::<String>().trim().to_string()
        }
        None => chars.iter().take(240).collect::<String>(),
    }
}

/// 打开书面整本 WordWise 种子：扫描成书全部英文文本，一次性把超档词
/// 补入生词库（幂等：已有卡/已标记词不动）。旧版逐页播种需用户翻完全书
/// 才能集齐词库，本命令让背诵队列/生词统计开局即覆盖整本。
#[tauri::command]
pub async fn seed_book_vocab_all(
    book_id: String,
    user_level: String,
    state: tauri::State<'_, AppState>,
) -> Result<super::ReaderState, AppError> {
    super::words::parse_user_level(&user_level)
        .ok_or_else(|| AppError::invalid_input(format!("不支持的用户难度: {user_level}")))?;
    let (skip_words, epub_path) = {
        let skip = {
            let states = super::lock_mutex(&state.reader_states, "阅读状态")?;
            match states.get(&book_id) {
                Some(reader_state) => reader_state
                    .vocab_cards
                    .keys()
                    .chain(reader_state.word_marks.keys())
                    .cloned()
                    .collect::<Vec<String>>(),
                None => Vec::new(),
            }
        };
        let epub = super::files::resolve_reader_epub_path(&state, &book_id)?;
        (skip, epub)
    };
    let seeds = tokio::task::spawn_blocking(move || -> Vec<WordWiseSeed> {
        let index = crate::word_levels::active_index();
        let Ok(document) = crate::document::extract_epub_document(&epub_path) else {
            return Vec::new();
        };
        let mut seen: std::collections::HashSet<String> = skip_words
            .into_iter()
            .map(|word| word.to_ascii_lowercase())
            .collect();
        let mut seeds: Vec<WordWiseSeed> = Vec::new();
        for block in &document.epub_blocks {
            let text = if block.text.trim().is_empty() {
                block.source_markdown.as_str()
            } else {
                block.text.as_str()
            };
            if text.trim().is_empty() {
                continue;
            }
            let Ok(infos) =
                super::words::difficult_words_in_text(&index, text, &user_level, None, None)
            else {
                continue;
            };
            for info in infos {
                let key = info.word.to_ascii_lowercase();
                if !seen.insert(key) {
                    continue;
                }
                let context = context_sentence_around(text, &info.word);
                let chapter = block.chapter_title.clone();
                seeds.push(WordWiseSeed {
                    word: info.word,
                    context,
                    chapter,
                });
            }
        }
        seeds
    })
    .await
    .map_err(|error| AppError::internal(format!("整本种子任务失败: {error}")))?;
    mutate_reader_state(&state, &book_id, |reader_state| {
        seed_cards(&mut reader_state.vocab_cards, &seeds);
        reader_state.updated_at = Utc::now().to_rfc3339();
    })
}

#[tauri::command]
pub async fn remove_vocab_card(
    book_id: String,
    word: String,
    state: tauri::State<'_, AppState>,
) -> Result<super::ReaderState, AppError> {
    mutate_reader_state(&state, &book_id, |reader_state| {
        remove_card(&mut reader_state.vocab_cards, &word);
    })
}

/// 到期卡片查询（分章过滤可选）：牌面由全量词表填充。
#[tauri::command]
pub async fn list_due_vocab_cards(
    book_id: String,
    chapter: Option<String>,
    include_future: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VocabCardFace>, AppError> {
    let snapshot = {
        let states = super::lock_mutex(&state.reader_states, "阅读状态")?;
        states.get(&book_id).cloned()
    };
    let Some(reader_state) = snapshot else {
        return Ok(Vec::new());
    };
    let now = Utc::now();
    let index = active_index();
    let include_future = include_future.unwrap_or(false);
    let cards: Vec<&VocabCard> = if include_future {
        let mut all: Vec<&VocabCard> = reader_state
            .vocab_cards
            .values()
            .filter(|card| chapter.as_deref().map_or(true, |ch| card.chapter == ch))
            .collect();
        all.sort_by(|left, right| left.word.cmp(&right.word));
        all
    } else {
        due_cards(&reader_state.vocab_cards, chapter.as_deref(), now)
    };
    Ok(cards
        .into_iter()
        .map(|card| card_face(card, &index))
        .collect())
}

/// 复习评分：quality 0=忘记 / 3=困难 / 4=记住 / 5=简单（SM-2），并记一条复习日志。
#[tauri::command]
pub async fn review_vocab_card(
    book_id: String,
    word: String,
    quality: u8,
    state: tauri::State<'_, AppState>,
) -> Result<super::ReaderState, AppError> {
    if quality > 5 {
        return Err(AppError::invalid_input("评分需在 0-5 之间"));
    }
    mutate_reader_state(&state, &book_id, |reader_state| {
        let word_key = word.trim().to_ascii_lowercase();
        if let Some(card) = reader_state.vocab_cards.get_mut(&word_key) {
            sm2_review(card, quality, Utc::now());
            reader_state.review_log.push(crate::commands::reader::ReviewLogEntry {
                book_id: book_id.clone(),
                word: word_key.clone(),
                quality,
                created_at: Utc::now().to_rfc3339(),
            });
            reader_state.updated_at = Utc::now().to_rfc3339();
        }
    })
}

/// 跨书到期卡（全局复习队列入口）：聚合所有书的到期卡，附 bookId；
/// 排序与单书 due_cards 一致（新卡优先，其次按到期时间升序）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalDueCard {
    pub book_id: String,
    #[serde(flatten)]
    pub face: VocabCardFace,
}

#[tauri::command]
pub async fn list_due_vocab_cards_all(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GlobalDueCard>, AppError> {
    let snapshot = {
        let states = super::lock_mutex(&state.reader_states, "阅读状态")?;
        states.values().cloned().collect::<Vec<_>>()
    };
    let now = Utc::now();
    let index = active_index();
    let mut all: Vec<GlobalDueCard> = Vec::new();
    for reader_state in &snapshot {
        for card in due_cards(&reader_state.vocab_cards, None, now) {
            all.push(GlobalDueCard {
                book_id: reader_state.book_id.clone(),
                face: card_face(card, &index),
            });
        }
    }
    // 与单书队列一致：新卡（无 due_at）在前，其余按到期时间升序；跨书稳定按书名次序并列
    all.sort_by(|left, right| match (&left.face.due_at, &right.face.due_at) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => a.cmp(b),
    });
    Ok(all)
}

/// Anki 导出：写文件到用户选择的路径，返回写入路径与卡片数。
/// （TSV 一行一卡：Front<TAB>Back<TAB>Tags，Anki「文件→导入」直接可用。）
#[tauri::command]
pub async fn export_anki_cards(
    book_id: String,
    chapter: Option<String>,
    output_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<AnkiExportResult, AppError> {
    let output_path = output_path.trim().to_string();
    if output_path.is_empty() {
        return Err(AppError::invalid_input("导出路径为空"));
    }
    let snapshot = {
        let states = super::lock_mutex(&state.reader_states, "阅读状态")?;
        states.get(&book_id).cloned()
    };
    let Some(reader_state) = snapshot else {
        return Ok(AnkiExportResult {
            path: output_path,
            card_count: 0,
        });
    };
    let index = active_index();
    let mut cards: Vec<&VocabCard> = reader_state
        .vocab_cards
        .values()
        .filter(|card| chapter.as_deref().map_or(true, |ch| card.chapter == ch))
        .collect();
    cards.sort_by(|left, right| left.word.cmp(&right.word));
    let payloads: Vec<AnkiCardPayload> = cards
        .into_iter()
        .map(|card| {
            let face = card_face(card, &index);
            AnkiCardPayload {
                word: face.word,
                phonetic: face.phonetic,
                definition: face.definition,
                context: face.context,
                level: face.level,
            }
        })
        .collect();
    let card_count = payloads.len();
    let tsv = export_anki_tsv(&payloads, "MuseReader");
    std::fs::write(&output_path, tsv)
        .map_err(|error| AppError::internal(format!("Anki 导出写入失败: {error}")))?;
    Ok(AnkiExportResult {
        path: output_path,
        card_count,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnkiExportResult {
    pub path: String,
    pub card_count: usize,
}

fn card_face(card: &VocabCard, index: &crate::word_levels::WordLevelIndex) -> VocabCardFace {
    let entry = index.entry_of(&card.word);
    // AI 语境释义（custom_definition）优先于词表默认释义
    let custom = card.custom_definition.trim();
    let definition = if !custom.is_empty() {
        custom.to_string()
    } else {
        entry.map(|e| e.definition.clone()).unwrap_or_default()
    };
    VocabCardFace {
        word: card.word.clone(),
        phonetic: entry.map(|e| e.phonetic.clone()).unwrap_or_default(),
        definition,
        level: entry
            .map(|e| e.level.label().to_string())
            .unwrap_or_default(),
        context: card.context.clone(),
        context_zh: card.context_zh.clone(),
        chapter: card.chapter.clone(),
        repetitions: card.repetitions,
        interval_days: card.interval_days,
        ease_factor: card.ease_factor,
        due_at: card.due_at.clone(),
        translation: entry.map(|e| e.translation.clone()).unwrap_or_default(),
        tag: entry.map(|e| e.tag.clone()).unwrap_or_default(),
        exchange: entry.map(|e| e.exchange.clone()).unwrap_or_default(),
        root: entry.map(|e| e.root.clone()).unwrap_or_default(),
        collins: entry.map(|e| e.collins).unwrap_or_default(),
        oxford: entry.map(|e| e.oxford).unwrap_or_default(),
        bnc: entry.map(|e| e.bnc).unwrap_or_default(),
        frq: entry.map(|e| e.frq).unwrap_or_default(),
    }
}

/// 单词笔记保存。
#[tauri::command]
pub async fn save_vocab_card_note(
    book_id: String,
    word: String,
    note: String,
    state: tauri::State<'_, AppState>,
) -> Result<super::ReaderState, AppError> {
    let trimmed = note.trim().to_string();
    mutate_reader_state(&state, &book_id, |reader_state| {
        if let Some(card) = reader_state.vocab_cards.get_mut(&word.trim().to_ascii_lowercase()) {
            card.note = trimmed;
            reader_state.updated_at = chrono::Utc::now().to_rfc3339();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn seed_cards_inserts_only_missing_words_and_is_idempotent() {
        let mut cards = HashMap::new();
        // 已有卡：手动添加过并复习过一轮（调度字段非初始值）
        upsert_card(&mut cards, "abandon", "旧语境句".to_string(), "第 1 章".to_string())
            .unwrap();
        if let Some(card) = cards.get_mut("abandon") {
            sm2_review(card, 5, now());
        }
        let legacy = cards.get("abandon").cloned().unwrap();

        let seeds = vec![
            WordWiseSeed {
                word: "Abandon".to_string(),
                context: "新语境句（重复词，不得覆盖）".to_string(),
                chapter: "第 2 章".to_string(),
            },
            WordWiseSeed {
                word: "serendipity".to_string(),
                context: "A happy accident.".to_string(),
                chapter: "第 2 章".to_string(),
            },
            WordWiseSeed {
                word: "  ".to_string(),
                context: String::new(),
                chapter: String::new(),
            },
        ];
        assert_eq!(seed_cards(&mut cards, &seeds), 1);
        // 幂等：再同步一遍不再新增
        assert_eq!(seed_cards(&mut cards, &seeds), 0);
        // 已有卡的语境/章节/调度保留不动
        let kept = cards.get("abandon").unwrap();
        assert_eq!(kept.context, legacy.context);
        assert_eq!(kept.chapter, legacy.chapter);
        assert_eq!(kept.repetitions, legacy.repetitions);
        // 新卡按播种语境落库、立即可学
        let seeded = cards.get("serendipity").unwrap();
        assert_eq!(seeded.context, "A happy accident.");
        assert!(seeded.is_due(now()));
    }

    #[test]
    fn new_card_is_due_immediately() {
        let card = VocabCard::new("Ephemeral ", String::new(), "第 1 章".to_string());
        assert_eq!(card.word, "ephemeral");
        assert!(card.is_due(now()));
    }

    #[test]
    fn sm2_forget_resets_schedule_with_short_relearn() {
        let mut card = VocabCard::new("abandon", String::new(), String::new());
        sm2_review(&mut card, 5, now());
        sm2_review(&mut card, 5, now());
        assert_eq!(card.repetitions, 2);
        assert_eq!(card.interval_days, 6.0);

        sm2_review(&mut card, 0, now());
        assert_eq!(card.repetitions, 0);
        assert!(card.interval_days < 0.01);
        assert!(card.is_due(now() + Duration::minutes(11)));
        assert!(!card.is_due(now() + Duration::minutes(5)));
    }

    #[test]
    fn sm2_remember_grows_interval_and_ease_floor_holds() {
        let mut card = VocabCard::new("paradigm", String::new(), String::new());
        sm2_review(&mut card, 5, now());
        assert_eq!(card.interval_days, 1.0);
        sm2_review(&mut card, 5, now());
        assert_eq!(card.interval_days, 6.0);
        sm2_review(&mut card, 5, now());
        assert!(card.interval_days > 6.0);

        // 连续差评 EF 不低于 1.3
        for _ in 0..20 {
            sm2_review(&mut card, 0, now());
        }
        assert!(card.ease_factor >= MIN_EASE_FACTOR);
    }

    #[test]
    fn sm2_hard_grows_slowly() {
        let mut easy = VocabCard::new("a", String::new(), String::new());
        let mut hard = VocabCard::new("b", String::new(), String::new());
        for _ in 0..3 {
            sm2_review(&mut easy, 5, now());
            sm2_review(&mut hard, 3, now());
        }
        assert!(hard.interval_days < easy.interval_days);
        assert!(hard.ease_factor < easy.ease_factor);
    }

    #[test]
    fn upsert_refreshes_context_without_resetting_schedule() {
        let mut cards = HashMap::new();
        upsert_card(
            &mut cards,
            "word",
            "ctx-1".to_string(),
            "第 1 章".to_string(),
        )
        .unwrap();
        let card = cards.get_mut("word").unwrap();
        sm2_review(card, 5, now());
        let reps = card.repetitions;

        upsert_card(
            &mut cards,
            "Word",
            "ctx-2".to_string(),
            "第 2 章".to_string(),
        )
        .unwrap();
        let card = cards.get("word").unwrap();
        assert_eq!(card.repetitions, reps); // 调度不被覆盖
        assert_eq!(card.context, "ctx-2");
        assert_eq!(card.chapter, "第 2 章");
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn due_cards_chapter_filter_and_new_first_ordering() {
        let mut cards = HashMap::new();
        upsert_card(&mut cards, "new-card", String::new(), "第 1 章".to_string()).unwrap();
        upsert_card(&mut cards, "old-card", String::new(), "第 1 章".to_string()).unwrap();
        upsert_card(
            &mut cards,
            "other-chapter",
            String::new(),
            "第 9 章".to_string(),
        )
        .unwrap();
        // old-card 学过一轮 → 1 天后到期（现在不到期）
        let card = cards.get_mut("old-card").unwrap();
        sm2_review(card, 5, now());

        let due = due_cards(&cards, Some("第 1 章"), now());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].word, "new-card"); // 新卡立即可学，old-card 未到期

        let all = due_cards(&cards, None, now());
        assert_eq!(all.len(), 2); // new-card + other-chapter
    }

    #[test]
    fn remove_card_is_case_insensitive() {
        let mut cards = HashMap::new();
        upsert_card(&mut cards, "Word", String::new(), String::new()).unwrap();
        assert!(remove_card(&mut cards, "WORD"));
        assert!(!remove_card(&mut cards, "word"));
    }

    #[test]
    fn anki_tsv_one_line_per_card_with_tabs_and_tags() {
        let payload = vec![
            AnkiCardPayload {
                word: "ephemeral".to_string(),
                phonetic: "/ɪˈfemərəl/".to_string(),
                definition: "adj. 短暂的\nn. 朝生暮死之物".to_string(),
                context: "an ephemeral bloom".to_string(),
                level: "专八".to_string(),
            },
            AnkiCardPayload {
                word: "serendipity".to_string(),
                phonetic: String::new(),
                definition: "n. 意外发现珍宝的运气".to_string(),
                context: String::new(),
                level: "GRE".to_string(),
            },
        ];
        let tsv = export_anki_tsv(&payload, "MuseReader");
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].split('\t').count(), 3);
        assert!(lines[0].contains("ephemeral"));
        assert!(lines[0].contains("<br>")); // 释义换行折叠为 HTML
        assert!(lines[0].contains("MuseReader 专八"));
        assert!(lines[1].contains("GRE"));
    }

    #[test]
    fn corrupt_due_timestamp_treated_as_due() {
        let mut card = VocabCard::new("glitch", String::new(), String::new());
        card.due_at = Some("not-a-date".to_string());
        assert!(card.is_due(now()));
    }
}
