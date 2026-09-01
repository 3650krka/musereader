use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::commands::vocab::VocabCard;

const DEFAULT_CHAPTER: &str = "第 1 章";
const MAX_ACTIVITY_RECORDS: usize = 180;
/// 复习日志保留 90 天窗口（趋势图足够，避免无限膨胀）。
const MAX_REVIEW_LOG_DAYS: i64 = 90;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderBookmark {
    pub id: String,
    pub book_id: String,
    pub chapter: String,
    pub quote: String,
    pub progress: u8,
    /// EPUB 章节定位（href#anchor），用于从书签跳回原文位置。
    #[serde(default)]
    pub href: Option<String>,
    /// 划线/高亮样式：highlight/underline/wave/bookmark（空=老数据纯书签）。
    #[serde(default)]
    pub style: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderNote {
    pub id: String,
    pub book_id: String,
    pub chapter: String,
    pub quote: String,
    pub note: String,
    pub progress: u8,
    /// EPUB 章节定位（href#anchor），用于从笔记跳回原文位置。
    #[serde(default)]
    pub href: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderActivity {
    pub id: String,
    pub book_id: String,
    pub minutes: f32,
    pub progress: u8,
    pub chapter: String,
    pub created_at: String,
}

/// 词汇掌握状态（黑白名单）：mastered=已掌握（白名单，不再提示），
/// learning=未掌握（黑名单，优先进入背诵卡片）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WordMarkStatus {
    Mastered,
    Learning,
}

/// 单次复习日志（Insights 复习趋势数据源）：quality 为 SM-2 0-5 评分。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLogEntry {
    pub book_id: String,
    pub word: String,
    pub quality: u8,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordMark {
    pub word: String,
    pub status: WordMarkStatus,
    /// 标记时的语境句子（AI 语境纠正/背诵卡片例句来源）。
    #[serde(default)]
    pub context: String,
    /// AI 语境释义覆盖：用户点击「AI 释义」后保存的语境义（优先于词表默认释义展示）。
    #[serde(default)]
    pub custom_definition: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderState {
    pub book_id: String,
    pub progress: u8,
    pub chapter: String,
    /// 最近阅读位置的 EPUB 锚点（章节 href#anchor），用于重开书籍时精确恢复。
    #[serde(default)]
    pub href: Option<String>,
    /// 锚点之后的页内偏移（配合 href 恢复分页位置）。
    #[serde(default)]
    pub offset: u32,
    pub bookmarks: Vec<ReaderBookmark>,
    pub notes: Vec<ReaderNote>,
    #[serde(default)]
    pub activities: Vec<ReaderActivity>,
    /// 词汇黑白名单（word 小写为键）。
    #[serde(default)]
    pub word_marks: HashMap<String, WordMark>,
    /// 词汇透析背诵卡片（word 小写为键，SM-2 调度）。
    #[serde(default)]
    pub vocab_cards: HashMap<String, VocabCard>,
    /// 复习日志（Insights 复习趋势，90 天窗口自动修剪）。
    #[serde(default)]
    pub review_log: Vec<ReviewLogEntry>,
    pub updated_at: String,
}

pub fn repair_reader_states(
    mut reader_states: HashMap<String, ReaderState>,
) -> HashMap<String, ReaderState> {
    for state in reader_states.values_mut() {
        normalize_reader_state(state);
    }
    reader_states
}

pub fn empty_reader_state(book_id: &str) -> ReaderState {
    ReaderState {
        book_id: book_id.to_string(),
        progress: 0,
        chapter: DEFAULT_CHAPTER.to_string(),
        href: None,
        offset: 0,
        bookmarks: Vec::new(),
        notes: Vec::new(),
        activities: Vec::new(),
        word_marks: HashMap::new(),
        vocab_cards: HashMap::new(),
        review_log: Vec::new(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

pub fn normalized_chapter(chapter: String) -> String {
    if chapter.trim().is_empty() {
        DEFAULT_CHAPTER.to_string()
    } else {
        chapter
    }
}

fn normalize_reader_state(state: &mut ReaderState) {
    if state.chapter.trim().is_empty() {
        state.chapter = DEFAULT_CHAPTER.to_string();
    }
    if state.activities.len() > MAX_ACTIVITY_RECORDS {
        state.activities.truncate(MAX_ACTIVITY_RECORDS);
    }
    trim_review_log(state);
}

/// 复习日志修剪：丢弃 90 天窗口之外的旧记录（按 created_at 升序存储，旧在前）。
fn trim_review_log(state: &mut ReaderState) {
    let cutoff = Utc::now() - chrono::Duration::days(MAX_REVIEW_LOG_DAYS);
    let keep_from = state
        .review_log
        .iter()
        .position(|entry| {
            chrono::DateTime::parse_from_rfc3339(&entry.created_at)
                .map(|ts| ts.with_timezone(&Utc) >= cutoff)
                .unwrap_or(false) // 坏时间戳视为过期，随下次修剪移除
        })
        .unwrap_or(state.review_log.len());
    if keep_from > 0 {
        state.review_log.drain(..keep_from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_review_log_drops_stale_entries_and_keeps_recent() {
        let mut state = empty_reader_state("book-1");
        let old = (Utc::now() - chrono::Duration::days(120)).to_rfc3339();
        let broken = "not-a-date".to_string();
        let recent = (Utc::now() - chrono::Duration::days(3)).to_rfc3339();
        state.review_log = vec![
            ReviewLogEntry {
                book_id: "book-1".to_string(),
                word: "old".to_string(),
                quality: 5,
                created_at: old,
            },
            ReviewLogEntry {
                book_id: "book-1".to_string(),
                word: "broken".to_string(),
                quality: 3,
                created_at: broken,
            },
            ReviewLogEntry {
                book_id: "book-1".to_string(),
                word: "recent".to_string(),
                quality: 4,
                created_at: recent.clone(),
            },
        ];

        normalize_reader_state(&mut state);

        assert_eq!(state.review_log.len(), 1);
        assert_eq!(state.review_log[0].word, "recent");
        assert_eq!(state.review_log[0].created_at, recent);
    }
}
