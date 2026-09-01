//! 词汇难度分级数据结构与运行期词库索引。
//!
//! 词库数据**不内置**：由用户导入词包（见 `commands/wordlist.rs`），
//! 支持本应用 JSON 包、ECDICT CSV、通用 TXT/CSV 词表等多种格式；
//! 多包合并后装入运行期活跃索引。
//!
//! 分级语义：一个词属于「它首次出现的最低考试档」——abandon 在中考词表
//! 即记为中考，即使它也在四级词表里。多包合并采用同一语义。
//!
//! 难度顺序（由低到高，用于「高于用户所选难度」过滤）：
//! 中考 < 高考 < CET4 < CET6 < 考研 < IELTS < TOEFL < 专四 < 专八 < GRE/SAT。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

static ACTIVE_INDEX: OnceLock<RwLock<Arc<WordLevelIndex>>> = OnceLock::new();

fn active_slot() -> &'static RwLock<Arc<WordLevelIndex>> {
    ACTIVE_INDEX.get_or_init(|| RwLock::new(Arc::new(WordLevelIndex::default())))
}

/// 当前活跃词库索引（导入词包合并结果；未导入时为空表）。
/// 返回 Arc 快照：调用方在整个查询作用域内持有，避免与并发导入撕扯。
pub fn active_index() -> Arc<WordLevelIndex> {
    active_slot().read()
        .map(|guard| Arc::clone(&guard))
        .unwrap_or_default()
}

/// 用新的合并索引替换活跃索引（导入/删除词包、启动装配后调用）。
pub fn install_index(index: WordLevelIndex) {
    if let Ok(mut guard) = active_slot().write() {
        *guard = Arc::new(index);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WordLevel {
    /// 中考（初中）
    #[default]
    ZhongKao,
    /// 高考（高中）
    GaoKao,
    /// 大学英语四级
    Cet4,
    /// 大学英语六级
    Cet6,
    /// 考研
    KaoYan,
    /// 雅思
    Ielts,
    /// 托福
    Toefl,
    /// 英语专业四级
    Tem4,
    /// 英语专业八级
    Tem8,
    /// GRE/SAT/GMAT（海外考试）
    Gre,
}

impl WordLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::ZhongKao => "中考",
            Self::GaoKao => "高考",
            Self::Cet4 => "四级",
            Self::Cet6 => "六级",
            Self::KaoYan => "考研",
            Self::Ielts => "雅思",
            Self::Toefl => "托福",
            Self::Tem4 => "专四",
            Self::Tem8 => "专八",
            Self::Gre => "GRE",
        }
    }

    /// 从目录名/文件名推断难度档（构建工具/测试用）。
    #[cfg(test)]
    pub fn from_category(category: &str) -> Option<Self> {
        if category.contains("中考") {
            Some(Self::ZhongKao)
        } else if category.contains("高考") {
            Some(Self::GaoKao)
        } else if category.contains("四级")
            || category.contains("CET4")
            || category.contains("cet4")
        {
            Some(Self::Cet4)
        } else if category.contains("六级")
            || category.contains("CET6")
            || category.contains("cet6")
        {
            Some(Self::Cet6)
        } else if category.contains("考研") {
            Some(Self::KaoYan)
        } else if category.contains("雅思")
            || category.contains("IELTS")
            || category.contains("ielts")
        {
            Some(Self::Ielts)
        } else if category.contains("托福")
            || category.contains("TOEFL")
            || category.contains("toefl")
        {
            Some(Self::Toefl)
        } else if category.contains("专四") {
            Some(Self::Tem4)
        } else if category.contains("专八") {
            Some(Self::Tem8)
        } else if category.contains("GRE")
            || category.contains("SAT")
            || category.contains("GMAT")
            || category.contains("gre")
            || category.contains("sat")
        {
            Some(Self::Gre)
        } else {
            None
        }
    }
}

/// 单词条目（含音标与释义，来自词表数据；ECDICT 增量：完整中文翻译、
/// 考试标签、柯林斯星级、牛津3000、BNC/词频排名、词形变换、词根助记）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordEntry {
    pub word: String,
    pub level: WordLevel,
    #[serde(default)]
    pub phonetic: String,
    #[serde(default)]
    pub definition: String,
    /// ECDICT 完整中文翻译（词性分段，比词表 definition 更全）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub translation: String,
    /// 考试标签（zk/gk/cet4/cet6/ky/toefl/gre…，空格分隔）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    /// 词形变换（ECDICT exchange 编码：d 过去式/p 过去分词/i 进行时/3 三单/s 复数）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub exchange: String,
    /// 词根助记（wordroot 反查：词根：含义（来源），最多两条）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    /// 柯林斯星级 1–5
    #[serde(default, skip_serializing_if = "is_zero")]
    pub collins: u8,
    /// 牛津3000 标记
    #[serde(default, skip_serializing_if = "is_zero")]
    pub oxford: u8,
    /// BNC 词频排名（越小越常用）
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub bnc: u32,
    /// COCA/美语词频排名
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub frq: u32,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// 内存索引：word（小写） → WordEntry（一词多档取最低档）。
#[derive(Debug, Default)]
pub struct WordLevelIndex {
    entries: HashMap<String, WordEntry>,
}

impl WordLevelIndex {
    pub fn from_entries(entries: Vec<WordEntry>) -> Self {
        let mut index = HashMap::with_capacity(entries.len());
        for entry in entries {
            let key = entry.word.to_ascii_lowercase();
            index
                .entry(key)
                .and_modify(|existing: &mut WordEntry| {
                    if entry.level < existing.level {
                        existing.level = entry.level;
                        if !entry.definition.is_empty() {
                            existing.definition = entry.definition.clone();
                        }
                        if !entry.phonetic.is_empty() {
                            existing.phonetic = entry.phonetic.clone();
                        }
                    }
                })
                .or_insert(entry);
        }
        Self { entries: index }
    }

    #[cfg(test)]
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let entries: Vec<WordEntry> = serde_json::from_str(json)?;
        Ok(Self::from_entries(entries))
    }

    /// 按难度档查询（生产路径用 entry_of；测试断言用）。
    #[cfg(test)]
    pub fn level_of(&self, word: &str) -> Option<WordLevel> {
        self.entries
            .get(&word.to_ascii_lowercase())
            .map(|e| e.level)
    }

    pub fn entry_of(&self, word: &str) -> Option<&WordEntry> {
        self.lookup_with_lemma(word).map(|(entry, _)| entry)
    }

    /// 带词形还原的查询：返回（词条, 命中的还原形）。
    /// 词表是原形词表，
    /// 这里按英语屈折规则逆推候选（复数/三单/ing/ed/比较级/最高级），逐个尝试。
    fn lookup_with_lemma(&self, word: &str) -> Option<(&WordEntry, String)> {
        let key = word.to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        // 直查（最常见路径：原形或词表本身收录的形）
        if let Some(entry) = self.entries.get(&key) {
            return Some((entry, key));
        }
        for candidate in lemma_candidates(&key) {
            if let Some(entry) = self.entries.get(&candidate) {
                return Some((entry, candidate));
            }
        }
        None
    }

    /// 词条总数（词库状态/空态判定使用）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 「高于用户所选难度」过滤：返回 level > user_level 的词。
    /// 用户选「四级」则返回六级及以上的词（用户需要背的正是超出当前水平的词）。
    #[cfg(test)]
    pub fn words_above(&self, text_words: &[String], user_level: WordLevel) -> Vec<&WordEntry> {
        text_words
            .iter()
            .filter_map(|word| self.entry_of(word))
            .filter(|entry| entry.level > user_level)
            .collect()
    }
}

/// 屈折词形 → 原形候选（有序：越靠前越可能是正确还原）。
/// 规则覆盖英语常见屈折：-s/-es/-ies 复数与三单、-ing、-ed/-ied/-d、
/// 双写尾辅音（running→run）、-er/-est 比较级最高级、's 所有格剥离。
/// 候选只是「查表尝试」，命中与否由词表决定——无误伤（查不到就 None）。
fn lemma_candidates(word: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(10);
    let chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    if n < 4 {
        return out;
    }
    let stem: String = chars[..n - 1].iter().collect();
    let stem2: String = chars[..n - 2].iter().collect();
    let stem3: String = chars[..n - 3].iter().collect();

    // 所有格 / 复数 / 三单
    if word.ends_with('s') {
        if word.ends_with("'s") || word.ends_with("’s") {
            out.push(chars[..n - 2].iter().collect::<String>());
        }
        if let Some(es_stem) = word.strip_suffix("es") {
            // boxes→box, goes→go, watches→watch
            out.push(es_stem.to_string());
            // issues→issue (e 结尾词 +s 复数被拼成 es) / misses→miss
            out.push(format!("{es_stem}e"));
        }
        if let Some(ies_stem) = word.strip_suffix("ies") {
            // studies→study, cities→city
            out.push(format!("{ies_stem}y"));
        }
        if let Some(vf_stem) = word.strip_suffix("ves") {
            // f/fe 交替：wives→wife, knives→knife, shelves→shelf, wolves→wolf
            out.push(format!("{vf_stem}fe"));
            out.push(format!("{vf_stem}f"));
        }
        out.push(stem.clone());
        // likes→like（e 结尾词只加 s 的常见误还原兜底）
        out.push(format!("{stem}e"));
    }
    // 进行体
    if word.ends_with("ing") {
        out.push(stem3.clone()); // running→run 的整词兜底走双写还原
        if let Some(ing_stem) = word.strip_suffix("ing") {
            out.push(ing_stem.to_string());        // reading→read
            out.push(format!("{ing_stem}e"));       // making→make
            out.push(format!("{ing_stem}ing"));     // skiing 之类整词
            // 双写尾辅音：running→run, swimming→swim
            let mut cs = ing_stem.chars().collect::<Vec<char>>();
            if cs.len() >= 3 {
                let last = cs[cs.len() - 1];
                let prev = cs[cs.len() - 2];
                if last == prev
                    && !"aeiou".contains(last)
                    && prev != 'l' // US travelling→travel 与 UK travel(l)ing 歧义，两者都试
                {
                    cs.truncate(cs.len() - 1);
                    out.push(cs.into_iter().collect::<String>());
                } else if last == 'l' && prev == 'l' {
                    cs.truncate(cs.len() - 1);
                    out.push(cs.into_iter().collect::<String>());
                    out.push(ing_stem.to_string());
                }
            }
        }
    }
    // 过去式/过去分词
    if word.ends_with("ied") {
        out.push(format!("{}y", word.strip_suffix("ied").unwrap_or("")));
    }
    if word.ends_with("ed") {
        out.push(stem2.clone());
        if let Some(ed_stem) = word.strip_suffix("ed") {
            out.push(ed_stem.to_string());         // walked→walk
            out.push(format!("{ed_stem}e"));       // loved→love
            // 双写尾辅音：stopped→stop
            let mut cs = ed_stem.chars().collect::<Vec<char>>();
            if cs.len() >= 3 {
                let last = cs[cs.len() - 1];
                let prev = cs[cs.len() - 2];
                if last == prev && !"aeiou".contains(last) {
                    cs.truncate(cs.len() - 1);
                    out.push(cs.into_iter().collect::<String>());
                }
            }
        }
    }
    // 比较级/最高级
    if word.ends_with("iest") {
        out.push(format!("{}y", word.strip_suffix("iest").unwrap_or("")));
    }
    if word.ends_with("er") || word.ends_with("est") {
        out.push(stem2.clone()); // bigger→big 走下方双写? 简化：先整词候选
        if word.ends_with("er") {
            if let Some(s) = word.strip_suffix("er") {
                out.push(s.to_string());
                out.push(format!("{s}e"));
            }
        } else if let Some(s) = word.strip_suffix("est") {
            out.push(stem3.clone());
            out.push(s.to_string());
            out.push(format!("{s}e"));
        }
        // 双写：bigger→big, biggest→big
        let base = word
            .strip_suffix("er")
            .or_else(|| word.strip_suffix("est"))
            .unwrap_or(word);
        let mut cs: Vec<char> = base.chars().collect();
        if cs.len() >= 3 {
            let last = cs[cs.len() - 1];
            let prev = cs[cs.len() - 2];
            if last == prev && !"aeiou".contains(last) {
                cs.truncate(cs.len() - 1);
                out.push(cs.into_iter().collect::<String>());
            }
        }
    }
    out.retain(|candidate| candidate.len() >= 2 && candidate != word);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(word: &str, level: WordLevel, phonetic: &str, definition: &str) -> WordEntry {
        WordEntry {
            word: word.to_string(),
            level,
            phonetic: phonetic.to_string(),
            definition: definition.to_string(),
            translation: String::new(),
            tag: String::new(),
            exchange: String::new(),
            root: String::new(),
            collins: 0,
            oxford: 0,
            bnc: 0,
            frq: 0,
        }
    }

    fn sample_index() -> WordLevelIndex {
        WordLevelIndex::from_entries(vec![
            entry("abandon", WordLevel::Cet4, "/əˈbændən/", "v. 放弃；抛弃"),
            // 更低档应覆盖
            entry("abandon", WordLevel::ZhongKao, "", ""),
            entry("ephemeral", WordLevel::Gre, "", "adj. 短暂的"),
            entry("ubiquitous", WordLevel::Toefl, "", "adj. 无处不在的"),
        ])
    }

    #[test]
    fn lowest_level_wins_for_duplicate_words() {
        let index = sample_index();
        assert_eq!(index.level_of("abandon"), Some(WordLevel::ZhongKao));
        // 低档覆盖时保留原释义（若覆盖项为空）
        let entry = index.entry_of("abandon").expect("entry");
        assert!(!entry.definition.is_empty());
    }

    #[test]
    fn case_insensitive_lookup() {
        let index = sample_index();
        assert_eq!(index.level_of("Abandon"), Some(WordLevel::ZhongKao));
        assert_eq!(index.level_of("EPHEMERAL"), Some(WordLevel::Gre));
    }

    #[test]
    fn unknown_word_returns_none() {
        let index = sample_index();
        assert_eq!(index.level_of("zzzqqq"), None);
    }

    #[test]
    fn words_above_filters_by_user_level() {
        let index = sample_index();
        let words = vec![
            "abandon".to_string(),
            "ephemeral".to_string(),
            "ubiquitous".to_string(),
            "unknown".to_string(),
        ];
        // 用户水平四级：高于四级 = 六级/考研/雅思/托福/GRE → ephemeral(GRE) + ubiquitous(TOEFL)
        let above = index.words_above(&words, WordLevel::Cet4);
        assert_eq!(above.len(), 2);
        // 用户水平托福：高于托福 = GRE → 只有 ephemeral
        let above = index.words_above(&words, WordLevel::Toefl);
        assert_eq!(above.len(), 1);
        assert_eq!(above[0].word, "ephemeral");
    }

    #[test]
    fn level_ordering() {
        assert!(WordLevel::ZhongKao < WordLevel::GaoKao);
        assert!(WordLevel::GaoKao < WordLevel::Cet4);
        assert!(WordLevel::Cet4 < WordLevel::Cet6);
        assert!(WordLevel::Cet6 < WordLevel::KaoYan);
        assert!(WordLevel::KaoYan < WordLevel::Ielts);
        assert!(WordLevel::Ielts < WordLevel::Toefl);
        assert!(WordLevel::Toefl < WordLevel::Tem4);
        assert!(WordLevel::Tem4 < WordLevel::Tem8);
        assert!(WordLevel::Tem8 < WordLevel::Gre);
    }

    #[test]
    fn category_detection() {
        assert_eq!(
            WordLevel::from_category("2.中考"),
            Some(WordLevel::ZhongKao)
        );
        assert_eq!(WordLevel::from_category("3.四级"), Some(WordLevel::Cet4));
        assert_eq!(WordLevel::from_category("7.雅思"), Some(WordLevel::Ielts));
        assert_eq!(WordLevel::from_category("7.托福"), Some(WordLevel::Toefl));
        assert_eq!(WordLevel::from_category("5.考研"), Some(WordLevel::KaoYan));
        assert_eq!(WordLevel::from_category("9.其他"), None);
    }

    #[test]
    fn inflected_forms_resolve_via_lemma() {
        // 词库外置后不依赖内嵌表：构造含原形词条的小索引验证还原逻辑。
        let seed = ["abandon", "thing", "box", "study", "run", "big", "wife"]
        .into_iter()
        .map(|word| WordEntry {
            word: word.to_string(),
            level: WordLevel::Cet4,
            ..WordEntry::default()
        })
        .collect::<Vec<_>>();
        let index = WordLevelIndex::from_entries(seed);
        // 复数/三单
        assert!(index.entry_of("abandons").is_some(), "abandons → abandon");
        assert!(index.entry_of("things").is_some(), "things → thing");
        assert!(index.entry_of("boxes").is_some(), "boxes → box");
        assert!(index.entry_of("studies").is_some(), "studies → study");
        // 进行体/过去式
        assert!(index.entry_of("abandoned").is_some(), "abandoned → abandon");
        assert!(index.entry_of("abandoning").is_some(), "abandoning → abandon");
        assert!(index.entry_of("running").is_some(), "running → run（双写还原）");
        assert!(index.entry_of("bigger").is_some(), "bigger → big（比较级）");
        assert!(index.entry_of("wives").is_some(), "wives → wife（f→v 复数）");
        // 还原形与原词条一致（难度与释义取原形）
        let a = index.entry_of("abandoned").expect("abandoned");
        let b = index.entry_of("abandon").expect("abandon");
        assert_eq!(a.word, b.word);
        // 未收录垃圾词仍然 None（无误伤）
        assert!(index.entry_of("zzzqqqing").is_none());
    }

    #[test]
    fn json_round_trip() {
        let json =
            r#"[{"word":"hello","level":"cet4","phonetic":"/həˈləʊ/","definition":"int. 你好"}]"#;
        let index = WordLevelIndex::from_json(json).expect("parse");
        assert_eq!(index.level_of("hello"), Some(WordLevel::Cet4));
        assert_eq!(index.len(), 1);
    }
}
