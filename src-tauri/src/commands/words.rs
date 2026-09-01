use super::AppError;
use crate::word_levels::{active_index, WordEntry, WordLevel, WordLevelIndex};
use serde::Serialize;

/// 词难度查询：返回单词的难度档 + 音标 + 释义 + 词库增量字段（翻译/标签/星级/词形/词频/词根）。
/// 数据源：用户导入词包合并后的运行期活跃索引（未导入时恒为 None）。
#[tauri::command]
pub async fn lookup_word_level(word: String) -> Result<Option<WordLevelInfo>, AppError> {
    Ok(lookup_word_level_with(&active_index(), &word))
}

fn lookup_word_level_with(index: &WordLevelIndex, word: &str) -> Option<WordLevelInfo> {
    index.entry_of(word).map(info_from_entry)
}

/// 从文本提取高于用户难度的生词（词透析基础）。
/// mastered 中的词（白名单）直接排除；learning 中的词（黑名单）不受难度过滤保留。
#[tauri::command]
pub async fn extract_difficult_words(
    text: String,
    user_level: String,
    mastered: Option<Vec<String>>,
    learning: Option<Vec<String>>,
) -> Result<Vec<WordLevelInfo>, AppError> {
    extract_difficult_words_with(
        &active_index(),
        &text,
        &user_level,
        mastered,
        learning,
    )
}

/// 整本种子命令的跨模块入口：与 extract_difficult_words 完全同一过滤语义。
pub(crate) fn difficult_words_in_text(
    index: &WordLevelIndex,
    text: &str,
    user_level: &str,
    mastered: Option<Vec<String>>,
    learning: Option<Vec<String>>,
) -> Result<Vec<WordLevelInfo>, AppError> {
    extract_difficult_words_with(index, text, user_level, mastered, learning)
}

fn extract_difficult_words_with(
    index: &WordLevelIndex,
    text: &str,
    user_level: &str,
    mastered: Option<Vec<String>>,
    learning: Option<Vec<String>>,
) -> Result<Vec<WordLevelInfo>, AppError> {
    let level = parse_user_level(user_level)
        .ok_or_else(|| AppError::invalid_input(format!("不支持的用户难度: {user_level}")))?;
    let mastered: std::collections::HashSet<String> = mastered
        .unwrap_or_default()
        .into_iter()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let learning: std::collections::HashSet<String> = learning
        .unwrap_or_default()
        .into_iter()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let words = tokenize_english_words(text);
    let mut seen = std::collections::HashSet::new();
    /* 与批量查询同语义：标记以原形为准——用户标记 abandon，文中的
       abandoned/abandons 同样排除（mastered）或强制保留（learning）。 */
    let unique: Vec<String> = words
        .into_iter()
        .map(|word| word.to_ascii_lowercase())
        .filter(|word| seen.insert(word.clone()))
        .filter_map(|word| {
            let entry = index.entry_of(&word);
            let lemma = entry
                .map(|entry| entry.word.as_str())
                .unwrap_or(word.as_str());
            if mastered.contains(&word) || mastered.contains(lemma) {
                return None;
            }
            if learning.contains(&word) || learning.contains(lemma) {
                /* learning 词不受难度档过滤；归回原形记录，各屈折形共用一条 */
                return Some(entry.map(|entry| entry.word.clone()).unwrap_or(word));
            }
            entry
                .filter(|entry| entry.level > level)
                .map(|entry| entry.word.clone())
        })
        .collect();
    Ok(unique
                .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|word| {
            index.entry_of(&word).map(info_from_entry).or_else(|| {
                /* 词表未收录的 learning 词：与批量查询同语义返回占位条目，
                   用户主动标记的词在透析里不因词库缺载而消失。 */
                learning.contains(&word).then(|| WordLevelInfo {
                    word: word.clone(),
                    level: "生词".to_string(),
                    phonetic: String::new(),
                    definition: String::new(),
                    translation: String::new(),
                    tag: String::new(),
                    exchange: String::new(),
                    root: String::new(),
                    collins: 0,
                    oxford: 0,
                    bnc: 0,
                    frq: 0,
                    surface: String::new(),
                })
            })
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordLevelInfo {
    pub word: String,
    pub level: String,
    pub phonetic: String,
    pub definition: String,
    /// 完整中文翻译（词性分段，通常比词表 definition 更全）
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub translation: String,
    /// 考试标签（zk/gk/cet4/cet6/ky/toefl/gre…，空格分隔）
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub tag: String,
    /// 词形变换（exchange 编码，前端解码展示）
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub exchange: String,
    /// 词根助记（最多两条，分号分隔）
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub root: String,
    /// 柯林斯星级 1–5（0=无数据）
    #[serde(skip_serializing_if = "is_zero_u8", default)]
    pub collins: u8,
    /// 牛津3000（0/1）
    #[serde(skip_serializing_if = "is_zero_u8", default)]
    pub oxford: u8,
    /// BNC 词频排名（越小越常用，0=无数据）
    #[serde(skip_serializing_if = "is_zero_u32", default)]
    pub bnc: u32,
    /// COCA 词频排名
    #[serde(skip_serializing_if = "is_zero_u32", default)]
    pub frq: u32,
    /// 批量查询回显：实际查询的词形（可能是屈折形）；与 word（原形）相同或为空时省略。
    /// 前端据此把返回行映射回文中出现的词形，屈折形（abandoned→abandon）也能命中。
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub surface: String,
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// WordEntry → WordLevelInfo（全字段映射；档级取 label 文本）。
pub(crate) fn info_from_entry(entry: &WordEntry) -> WordLevelInfo {
    WordLevelInfo {
        word: entry.word.clone(),
        level: entry.level.label().to_string(),
        phonetic: entry.phonetic.clone(),
        definition: entry.definition.clone(),
        translation: entry.translation.clone(),
        tag: entry.tag.clone(),
        exchange: entry.exchange.clone(),
        root: entry.root.clone(),
        collins: entry.collins,
        oxford: entry.oxford,
        bnc: entry.bnc,
        frq: entry.frq,
        surface: String::new(),
    }
}

/// 批量生词标注查询（WordWise 行内标注专用）：一次调用返回整页需标注词条，
/// 避免前端逐词 invoke。mastered 排除；learning 强制标注；其余仅高于用户难度档的词。
#[tauri::command]
pub async fn lookup_words_batch(
    words: Vec<String>,
    user_level: String,
    mastered: Option<Vec<String>>,
    learning: Option<Vec<String>>,
) -> Result<Vec<WordLevelInfo>, AppError> {
    lookup_words_batch_with(&active_index(), words, &user_level, mastered, learning)
}

fn lookup_words_batch_with(
    index: &WordLevelIndex,
    words: Vec<String>,
    user_level: &str,
    mastered: Option<Vec<String>>,
    learning: Option<Vec<String>>,
) -> Result<Vec<WordLevelInfo>, AppError> {
    let level = parse_user_level(user_level)
        .ok_or_else(|| AppError::invalid_input(format!("不支持的用户难度: {user_level}")))?;
    let mastered: std::collections::HashSet<String> = mastered
        .unwrap_or_default()
        .into_iter()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let learning: std::collections::HashSet<String> = learning
        .unwrap_or_default()
        .into_iter()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    Ok(words
        .into_iter()
        .map(|word| word.to_ascii_lowercase())
        .filter(|word| !word.is_empty())
        .filter_map(|word| {
            /* 先还原原形再判定 mastered/learning：用户标记的是 abandon，
               文中的 abandoned/abandons 必须同样被压制或强制标注；
               用户也可能标记屈折形本身，故原形与本形都查。 */
            let entry = index.entry_of(&word);
            let lemma = entry
                .map(|entry| entry.word.clone())
                .unwrap_or_else(|| word.clone());
            let marked = |w: &str| mastered.contains(w) || mastered.contains(lemma.as_str());
            if marked(&word) {
                return None;
            }
            if learning.contains(&word) || learning.contains(lemma.as_str()) {
                /* learning 强制标注：词表未收录也返回占位条目（档级「生词」），
                   保证用户主动标记学习的词必然获得行内提词；上标退显档级文案，
                   词卡内可经 AI 语境义补全释义。 */
                return entry
                    .map(|entry| {
                        let mut info = info_from_entry(entry);
                        /* 屈折形（abandoned→abandon）：回显送查词形，前端据此映射命中 */
                        if entry.word != word {
                            info.surface = word.clone();
                        }
                        info
                    })
                    .or(Some(WordLevelInfo {
                        word: lemma.clone(),
                        level: "生词".to_string(),
                        phonetic: String::new(),
                        definition: String::new(),
                        translation: String::new(),
                        tag: String::new(),
                        exchange: String::new(),
                        root: String::new(),
                        collins: 0,
                        oxford: 0,
                        bnc: 0,
                        frq: 0,
                        surface: if word != lemma { word.clone() } else { String::new() },
                    }));
            }
            entry.filter(|entry| entry.level > level).map(|entry| {
                let mut info = info_from_entry(entry);
                if entry.word != word {
                    info.surface = word.clone();
                }
                info
            })
        })
        .collect())
}

/// 批量词级信息查询（不做难度过滤，学习中心专用）：难度分布统计与词表增强。
/// 与 lookup_words_batch 的区别：不应用 mastered/learning/难度档语义，纯信息查询；
/// 大小写归一并去重，未收录词静默跳过。
#[tauri::command]
pub async fn lookup_words_info_batch(words: Vec<String>) -> Result<Vec<WordLevelInfo>, AppError> {
    Ok(lookup_words_info_batch_with(&active_index(), words))
}

fn lookup_words_info_batch_with(index: &WordLevelIndex, words: Vec<String>) -> Vec<WordLevelInfo> {
    let mut seen = std::collections::HashSet::new();
    words
        .into_iter()
        .map(|word| word.trim().to_ascii_lowercase())
        .filter(|word| !word.is_empty() && seen.insert(word.clone()))
        .filter_map(|word| index.entry_of(&word))
        .map(info_from_entry)
        .collect()
}

/// 用户难度档文案 → 枚举（设置/词库导入的默认档参数共用此解析）。
pub(crate) fn parse_user_level(label: &str) -> Option<WordLevel> {
    match label.trim() {
        "中考" | "初中" | "zhongKao" | "zk" => Some(WordLevel::ZhongKao),
        "高考" | "高中" | "gaoKao" | "gk" => Some(WordLevel::GaoKao),
        "四级" | "cet4" | "CET4" => Some(WordLevel::Cet4),
        "六级" | "cet6" | "CET6" => Some(WordLevel::Cet6),
        "考研" | "kaoYan" | "ky" => Some(WordLevel::KaoYan),
        "雅思" | "ielts" | "IELTS" => Some(WordLevel::Ielts),
        "托福" | "toefl" | "TOEFL" => Some(WordLevel::Toefl),
        "专四" | "tem4" | "TEM4" => Some(WordLevel::Tem4),
        "专八" | "tem8" | "TEM8" => Some(WordLevel::Tem8),
        "GRE" | "gre" | "SAT" | "sat" | "GMAT" => Some(WordLevel::Gre),
        _ => None,
    }
}

fn tokenize_english_words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'' && ch != '-')
        .filter(|word| word.len() >= 3)
        .map(|word| {
            word.trim_matches(|ch: char| ch == '\'' || ch == '-')
                .to_string()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// 测试共用的小词库 fixture（词库外置后不再有内嵌数据可读）。
#[cfg(test)]
pub(crate) fn fixture_index() -> WordLevelIndex {
    WordLevelIndex::from_entries(vec![
        WordEntry {
            word: "the".to_string(),
            level: WordLevel::ZhongKao,
            definition: "art. 这；那".to_string(),
            ..Default::default()
        },
        WordEntry {
            word: "abandon".to_string(),
            level: WordLevel::GaoKao,
            phonetic: "/əˈbændən/".to_string(),
            definition: "v. 放弃；抛弃".to_string(),
            ..Default::default()
        },
        WordEntry {
            word: "thing".to_string(),
            level: WordLevel::ZhongKao,
            ..Default::default()
        },
        WordEntry {
            word: "serendipity".to_string(),
            level: WordLevel::Gre,
            definition: "n. 意外发现珍宝的运气".to_string(),
            ..Default::default()
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_index_serves_level_and_entry_queries() {
        let index = fixture_index();
        assert_eq!(index.level_of("the"), Some(WordLevel::ZhongKao));
        assert_eq!(index.level_of("abandon"), Some(WordLevel::GaoKao));
        assert_eq!(index.level_of("serendipity"), Some(WordLevel::Gre));
        // 释义与音标随词包带入
        let entry = index.entry_of("abandon").expect("abandon entry");
        assert!(!entry.definition.is_empty());
        assert!(!entry.phonetic.is_empty());
    }

    #[test]
    fn empty_index_returns_nothing_without_error() {
        // 未导入词包：活跃查询返回 None，不 panic（前端空态据此提示）。
        let empty = WordLevelIndex::default();
        assert!(lookup_word_level_with(&empty, "abandon").is_none());
        let rows = lookup_words_info_batch_with(&empty, vec!["abandon".into()]);
        assert!(rows.is_empty());
    }

    #[test]
    fn tokenize_strips_punctuation_and_keeps_contractions() {
        let words = tokenize_english_words("It's a well-known fact, isn't it?");
        assert!(words.contains(&"It's".to_string()));
        assert!(words.contains(&"well-known".to_string()));
        assert!(!words.contains(&"a".to_string())); // 短词过滤
    }

    #[tokio::test]
    async fn batch_lookup_filters_by_level_and_marks() {
        let index = fixture_index();
        // abandon=高考 serendipity=GRE；用户四级 → 仅 GRE 超档
        let rows = lookup_words_batch_with(
            &index,
            vec!["abandon".into(), "serendipity".into(), "the".into()],
            "四级",
            None,
            None,
        )
        .expect("batch lookup");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].word, "serendipity");

        // mastered 排除；learning 即使不超档也强制保留
        let rows = lookup_words_batch_with(
            &index,
            vec!["abandon".into(), "serendipity".into()],
            "GRE",
            Some(vec!["serendipity".into()]),
            Some(vec!["abandon".into()]),
        )
        .expect("batch lookup with marks");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].word, "abandon");

        // 标记原形、文中屈折形：mastered 压制到变体，learning 强制到变体；
        // 屈折形行回显送查词形（surface），词目仍为原形。
        let rows = lookup_words_batch_with(
            &index,
            vec!["abandoned".into(), "things".into()],
            "GRE",
            Some(vec!["abandon".into()]),
            None,
        )
        .expect("batch lookup mastered lemma");
        assert!(rows.iter().all(|row| row.word != "abandon"));

        let rows = lookup_words_batch_with(
            &index,
            vec!["abandoned".into()],
            "GRE",
            None,
            Some(vec!["abandon".into()]),
        )
        .expect("batch lookup learning lemma");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].word, "abandon");
        assert_eq!(rows[0].surface.as_str(), "abandoned");

        // 词表未收录的 learning 词返回占位条目（档级「生词」）
        let rows = lookup_words_batch_with(
            &index,
            vec!["zzzqqq".into()],
            "中考",
            None,
            Some(vec!["zzzqqq".into()]),
        )
        .expect("batch lookup placeholder");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].level, "生词");
    }

    #[tokio::test]
    async fn info_batch_returns_unfiltered_levels() {
        let index = fixture_index();
        let rows = lookup_words_info_batch_with(
            &index,
            vec![
            "The".into(),
            "abandon".into(),
            "ABANDON".into(),
            "  ".into(),
            "zzzznotaword".into(),
            ],
        );
        // 大小写归一 + 去重 + 未收录词跳过；不做难度过滤（中考档 the 也返回）
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].word, "the");
        assert_eq!(rows[0].level, "中考");
        assert_eq!(rows[1].word, "abandon");
    }
}
