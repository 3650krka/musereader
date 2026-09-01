//! 语言对策略（LanguagePairPolicy）：把「英语→简体中文」的语种硬编码
//! 集中到单一配置点，并为其他语种路径提供符合产品哲学的适配方案。
//!
//! 当前适配矩阵（2026-08-06 实测分析）：
//!
//! | 路径 | 状态 | 关键问题 |
//! |---|---|---|
//! | EN → ZH | ✅ 完全适配 | 所有算法的设计原点 |
//! | ZH → EN | ⚠️ 部分可用 | 残留检测失效（检出的是中文残留而非英文）；prompt 目标语种硬编码；专名提取基于大写拉丁字母（中文源文无此信号） |
//! | JA → ZH | ⚠️ 部分可用 | 残留检测会把日文汉字当中文（U+4E00-9FFF 重叠）；专名提取失效（日文无大写） |
//! | 其他拉丁语 → ZH | ⚠️ 大部分可用 | 残留检测可用（拉丁字母）；专名提取可用（大写）；引号对/数字守恒可用 |
//! | * → 非 ZH | ❌ 基本不可用 | prompt/校验/引号/数字/残留检测全以中文为目标 |
//!
//! 产品哲学方案（不盲目通用化，而是「语种策略层」）：
//! - **LanguagePairPolicy** 抽象「源语种信号」与「目标语种信号」：
//!   源专名提取规则（拉丁大写/日文假名+CJK/中文无大写）；
//!   目标残留检测字符集（拉丁字母/假名/中文）；
//!   目标引号对（“”/「」/""）；
//!   目标数字格式（阿拉伯/中文数词）。
//! - EN→ZH 走全量策略（现状）；其他路径按策略矩阵降级启用，
//!   不可用的检测**显式关闭**（比误报好——全息保真原则：错误的检测不如没有检测）。
//! - 通用流水线不是"备选"，而是"按语种策略参数化的同一条流水线"——
//!   这正是本质极简：一套代码，策略注入。
//!
//! 注：本模块为多语种扩展的预留设计层——当前产品锁定英→中，尚无生产
//! 调用方；编译期以允许未使用项的形式保留完整策略矩阵。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceLanguage {
    English,
    Chinese,
    Japanese,
    /// 其他拉丁字母语言（法/德/西等，专名提取与残留检测与英文同构）。
    LatinOther,
    /// 未识别/混合（保守：全部按英文处理，与历史行为一致）。
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetLanguage {
    ChineseSimplified,
    English,
    /// 其他目标语种（当前仅声明，策略矩阵按「与英文同构」降级处理）。
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguagePairPolicy {
    pub source: SourceLanguage,
    pub target: TargetLanguage,
}

impl LanguagePairPolicy {
    /// 当前默认：英语 → 简体中文（产品主战场，所有算法的设计原点）。
    pub fn default_en_zh() -> Self {
        Self {
            source: SourceLanguage::English,
            target: TargetLanguage::ChineseSimplified,
        }
    }

    /// 残留检测是否可用：检出「目标语种中残留的源语种字符」。
    /// - EN→ZH：检测中文里的拉丁字母残留 ✅
    /// - ZH→EN：检测英文里的中文字符残留 ✅
    /// - JA→ZH：日文汉字与中文共享 U+4E00-9FFF，无法区分 → ❌（误报率极高）
    /// - 拉丁→ZH：检测中文里的拉丁字母残留 ✅
    /// - *→非ZH：检测非中文目标里的源字符 ✅（但需目标字符集，当前仅声明）
    pub fn residual_detection_available(self) -> bool {
        !matches!(
            (self.source, self.target),
            (SourceLanguage::Japanese, TargetLanguage::ChineseSimplified)
        )
    }

    /// 残留检测的「源语种字符」判定：在目标文本中检出这些字符即视为残留。
    /// 返回字符判定函数的静态分发（避免闭包分配）。
    pub fn source_char_is_residual(self, ch: char) -> bool {
        match self.source {
            SourceLanguage::English | SourceLanguage::LatinOther | SourceLanguage::Unknown => {
                ch.is_ascii_alphabetic()
            }
            SourceLanguage::Chinese => ('\u{4e00}'..='\u{9fff}').contains(&ch),
            SourceLanguage::Japanese => {
                // 假名（平/片）是日文独有信号；汉字与中文共享，不作残留信号。
                ('\u{3040}'..='\u{309f}').contains(&ch) || ('\u{30a0}'..='\u{30ff}').contains(&ch)
            }
        }
    }

    /// 源文专名提取是否可用：依赖源语种的「专名正字法信号」。
    /// - 拉丁语系：首字母大写 ✅
    /// - 日文：无大写，但可用「汉字+假名边界」或「～の/～は」助词信号（未实现，暂不可用）
    /// - 中文：无大写无词界，需 NER（我们的 algorithmic 提取不可用）
    pub fn proper_noun_extraction_available(self) -> bool {
        matches!(
            self.source,
            SourceLanguage::English | SourceLanguage::LatinOther | SourceLanguage::Unknown
        )
    }

    /// 目标语种的引号对（左, 右）：用于引号配对校验。
    pub fn quote_pair(self) -> (char, char) {
        match self.target {
            TargetLanguage::ChineseSimplified => ('\u{201c}', '\u{201d}'), // “ ”
            TargetLanguage::English => ('"', '"'),
            TargetLanguage::Other => ('\u{201c}', '\u{201d}'),
        }
    }

    /// 目标语种是否 CJK（影响排版/分词/词界）。
    pub fn target_is_cjk(self) -> bool {
        matches!(self.target, TargetLanguage::ChineseSimplified)
    }
}

/// 从源文本自动识别语种（轻量字符分布法，无外部依赖）。
/// 用于未来「用户只给文件，系统自动选策略」。
pub fn detect_source_language(text: &str) -> SourceLanguage {
    let sample: String = text.chars().take(2000).collect();
    if sample.is_empty() {
        return SourceLanguage::Unknown;
    }
    let total = sample.chars().count() as f32;
    let mut latin = 0f32;
    let mut cjk = 0f32;
    let mut kana = 0f32;
    for ch in sample.chars() {
        if ch.is_ascii_alphabetic() {
            latin += 1.0;
        } else if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            cjk += 1.0;
        } else if ('\u{3040}'..='\u{309f}').contains(&ch) || ('\u{30a0}'..='\u{30ff}').contains(&ch)
        {
            kana += 1.0;
        }
    }
    let (latin_r, cjk_r, kana_r) = (latin / total, cjk / total, kana / total);
    // 假名占比 >5% → 日文（假名是日文独有强信号）
    if kana_r > 0.05 {
        return SourceLanguage::Japanese;
    }
    // CJK 占比 >30% 且无假名 → 中文
    if cjk_r > 0.3 {
        return SourceLanguage::Chinese;
    }
    // 拉丁字母占比 >40% → 英文（拉丁其他语种无法区分，按英文策略处理——同构）
    if latin_r > 0.4 {
        return SourceLanguage::English;
    }
    SourceLanguage::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_zh_full_support() {
        let policy = LanguagePairPolicy::default_en_zh();
        assert!(policy.residual_detection_available());
        assert!(policy.proper_noun_extraction_available());
        assert!(policy.target_is_cjk());
        assert!(policy.source_char_is_residual('a'));
        assert!(!policy.source_char_is_residual('中'));
    }

    #[test]
    fn zh_en_residual_detects_cjk() {
        let policy = LanguagePairPolicy {
            source: SourceLanguage::Chinese,
            target: TargetLanguage::English,
        };
        assert!(policy.residual_detection_available());
        assert!(!policy.proper_noun_extraction_available());
        assert!(policy.source_char_is_residual('中'));
        assert!(!policy.source_char_is_residual('a'));
        assert!(!policy.target_is_cjk());
    }

    #[test]
    fn ja_zh_residual_disabled_for_shared_hanzi() {
        let policy = LanguagePairPolicy {
            source: SourceLanguage::Japanese,
            target: TargetLanguage::ChineseSimplified,
        };
        // 日文汉字与中文共享码点，残留检测会大量误报 → 显式关闭
        assert!(!policy.residual_detection_available());
        assert!(!policy.proper_noun_extraction_available());
    }

    #[test]
    fn latin_other_behaves_like_english() {
        let policy = LanguagePairPolicy {
            source: SourceLanguage::LatinOther,
            target: TargetLanguage::ChineseSimplified,
        };
        assert!(policy.residual_detection_available());
        assert!(policy.proper_noun_extraction_available());
    }

    #[test]
    fn language_detection_by_char_distribution() {
        assert_eq!(
            detect_source_language("The quick brown fox jumps over the lazy dog repeatedly."),
            SourceLanguage::English
        );
        assert_eq!(
            detect_source_language(
                "这是一个纯粹的中文段落，不包含任何其他语言的内容，用于测试中文识别。"
            ),
            SourceLanguage::Chinese
        );
        assert_eq!(
            detect_source_language("これは日本語のテストです。ひらがなとカタカナを含みます。"),
            SourceLanguage::Japanese
        );
        assert_eq!(detect_source_language(""), SourceLanguage::Unknown);
    }

    #[test]
    fn japanese_detection_prioritizes_kana_over_hanzi() {
        // 日文文本含大量汉字，但只要有假名就应判为日文
        assert_eq!(
            detect_source_language("日本語の文章には漢字とひらがなとカタカナが混在しています。"),
            SourceLanguage::Japanese
        );
    }

    #[test]
    fn quote_pairs_per_target() {
        assert_eq!(
            LanguagePairPolicy::default_en_zh().quote_pair(),
            ('\u{201c}', '\u{201d}')
        );
        let en_target = LanguagePairPolicy {
            source: SourceLanguage::Chinese,
            target: TargetLanguage::English,
        };
        assert_eq!(en_target.quote_pair(), ('"', '"'));
    }
}
