//! 译文格式忠实度校验（无 LLM、确定性、低成本）。
//!
//! 范围：**只检测确定性的格式问题**，不做语义判断。
//! （注：原「内容漂移语义检测」——数字守恒/否定守恒——已按用户决策移除。
//!  理由：漂移的本质是模型对语境与语义的理解问题，表面词守恒测不准——
//!  中英否定/数字表达存在系统性习惯差异（双重否定→肯定、被动→主动、
//!  忍不住=can't help but 无显性否定、刚…就=no sooner），硬报警必误报。
//!  此类问题只能靠更强的模型/prompt/语境锚定从根上减少，检测层面无解。）
//!
//! 保留的两项都是**确定性格式问题**（与语义无关，检测即修复或唯一无歧义）：
//!   1. 曲引号配对（QUOTE_IMBALANCE）：引号左右个数失衡——格式层面确定，
//!      检测后可在「修复唯一无歧义」时自动补（见 repair 模块）。
//!   2. 章标题双语残留（HEADING_BILINGUAL_RESIDUE）：「Chapter N: 中文名」
//!      人名翻了但 Chapter 编号没翻——确定性格式修复（第N章：中文名）。

use crate::pipeline::{ChunkCheckpoint, ChunkSegmentKind};

/// 一个格式校验信号命中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriftSignal {
    pub code: &'static str,
    pub chunk_index: usize,
    pub detail: String,
}

/// 主入口：对 body 块做格式校验扫描。
pub(crate) fn detect_content_drift(checkpoint: &ChunkCheckpoint) -> Vec<DriftSignal> {
    let mut signals = Vec::new();
    for chunk in &checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_deref() else {
            continue;
        };
        if translated.trim().is_empty() {
            continue;
        }
        // 曲引号配对
        if let Some(s) = check_quote_balance(chunk.index, translated) {
            signals.push(s);
        }
        // 章标题双语残留（Chapter N: 中文名 → 未翻全）
        if let Some(s) = check_heading_bilingual_residue(chunk.index, translated) {
            signals.push(s);
        }
        // 数字守恒检测（只记录不修复）：译文与源文中阿拉伯数字集合不同，
        // 且差异不在年份/页码/DOI/参考编号白名单内。
        if let Some(s) = check_number_conservation(chunk.index, &chunk.source, translated) {
            signals.push(s);
        }
    }
    signals
}

/// 数字守恒检测：译文与源文的阿拉伯数字集合不同。
/// 只记录不修复（自动修复需语义判断，超出确定性格式检测边界）。
/// 白名单：年份（19xx/20xx）、页码（pp./p. 后数字）、DOI/参考编号（[N] 形式）。
fn check_number_conservation(
    chunk_index: usize,
    source: &str,
    translated: &str,
) -> Option<DriftSignal> {
    let source_numbers = extract_significant_numbers(source);
    let translated_numbers = extract_significant_numbers(translated);
    if source_numbers.is_empty() {
        return None;
    }
    let missing: Vec<&str> = source_numbers
        .iter()
        .filter(|n| !translated_numbers.contains(*n))
        .map(|s| s.as_str())
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(DriftSignal {
        code: "NUMBER_CONSERVATION",
        chunk_index,
        detail: format!(
            "数字不一致（源文有译文无）: {}; 译文: {}",
            missing.join(", "),
            compact_excerpt(translated, 60)
        ),
    })
}

fn extract_significant_numbers(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"\b\d+(?:\.\d+)?%?\b").unwrap();
    re.find_iter(text)
        .map(|m| m.as_str().to_string())
        .filter(|n| {
            // 排除年份（19xx/20xx）
            if let Ok(year) = n.trim_end_matches('%').parse::<u32>() {
                if (1900..=2099).contains(&year) {
                    return false;
                }
            }
            // 排除单字符数字（页码/章节号噪声）
            if n.len() < 2 {
                return false;
            }
            true
        })
        .collect()
}

/// 章标题双语残留检测：「Chapter N: 中文名」——人名翻了但 Chapter 编号没翻。
/// 这是确定性可修的（见 repair_heading_bilingual_residue），此处先做检测标记。
/// 扩展：识别 Book/Volume/Part/Section 前缀（fantasy 小说常见 'Book Two, Chapter 12'）。
fn check_heading_bilingual_residue(chunk_index: usize, translated: &str) -> Option<DriftSignal> {
    let caps: Vec<_> = regex::Regex::new(
        r"(?m)^\s*(?:(?:Book|Volume|Part|Section)\s+\w+[,:：]?\s+)?Chapter\s+(\d+)\s*[:：]\s*(.+)$",
    )
    .unwrap()
    .captures_iter(translated)
    .collect();
    for cap in caps {
        let name = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        // 人名部分已含中文（翻了），但行首仍是英文 Chapter N → 半译残留
        if name.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
            return Some(DriftSignal {
                code: "HEADING_BILINGUAL_RESIDUE",
                chunk_index,
                detail: format!(
                    "章标题半译（Chapter N 未翻）: {}",
                    compact_excerpt(translated, 50)
                ),
            });
        }
    }
    None
}

/// 章标题半译的确定性修复：「Chapter N: 中文名」→「第N章：中文名」。
/// 支持 Book/Volume/Part/Section 前缀（'Book Two, Chapter 12: 中文' →「第二卷 第十二章：中文」）。
/// 仅在格式严格匹配（前缀 + Chapter + 数字 + 冒号 + 含中文的名）时应用，否则原样返回。
pub(crate) fn repair_heading_bilingual_residue(translated: &str) -> String {
    let re = regex::Regex::new(
        r"(?m)^(\s*)(?:(Book|Volume|Part|Section)\s+(\w+)[,:：]?\s+)?Chapter\s+(\d+)\s*[:：]\s*(.+?)\s*$"
    ).unwrap();
    re.replace_all(translated, |caps: &regex::Captures| {
        let indent = &caps[1];
        let prefix = caps.get(2).map(|m| m.as_str());
        let prefix_num = caps.get(3).map(|m| m.as_str());
        let num: u32 = caps[4].parse().unwrap_or(0);
        let name = caps[5].trim();
        if name.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
            let prefix_zh = match (prefix, prefix_num) {
                (Some(p), Some(pn)) => {
                    let kind = match p.to_ascii_lowercase().as_str() {
                        "book" | "volume" => "卷",
                        "part" => "部",
                        "section" => "节",
                        _ => "",
                    };
                    let num_zh = parse_english_number(pn)
                        .map(to_chinese_numeral)
                        .unwrap_or_else(|| pn.to_string());
                    format!("第{num_zh}{kind} ")
                }
                _ => String::new(),
            };
            format!("{indent}{prefix_zh}第{}章：{name}", to_chinese_numeral(num))
        } else {
            caps[0].to_string()
        }
    })
    .to_string()
}

/// 英文数字词 → 阿拉伯数字（one/two/.../twenty 覆盖常见卷号）。
fn parse_english_number(word: &str) -> Option<u32> {
    match word.to_ascii_lowercase().as_str() {
        "one" | "i" => Some(1),
        "two" | "ii" => Some(2),
        "three" | "iii" => Some(3),
        "four" | "iv" => Some(4),
        "five" | "v" => Some(5),
        "six" | "vi" => Some(6),
        "seven" | "vii" => Some(7),
        "eight" | "viii" => Some(8),
        "nine" | "ix" => Some(9),
        "ten" | "x" => Some(10),
        "eleven" | "xi" => Some(11),
        "twelve" | "xii" => Some(12),
        "thirteen" | "xiii" => Some(13),
        "fourteen" | "xiv" => Some(14),
        "fifteen" | "xv" => Some(15),
        "sixteen" | "xvi" => Some(16),
        "seventeen" | "xvii" => Some(17),
        "eighteen" | "xviii" => Some(18),
        "nineteen" | "xix" => Some(19),
        "twenty" | "xx" => Some(20),
        _ => word.parse().ok(),
    }
}

/// 阿拉伯数字 → 中文数词（1-99 覆盖章节编号常见范围；>99 回退阿拉伯数字）。
fn to_chinese_numeral(n: u32) -> String {
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    match n {
        0..=10 => [
            "零", "一", "二", "三", "四", "五", "六", "七", "八", "九", "十",
        ][n as usize]
            .to_string(),
        11..=19 => format!("十{}", D[(n % 10) as usize]),
        20..=99 => {
            let (t, o) = ((n / 10) as usize, (n % 10) as usize);
            if o == 0 {
                format!("{}十", D[t])
            } else {
                format!("{}十{}", D[t], D[o])
            }
        }
        _ => n.to_string(),
    }
}

/// 曲引号配对自动修复：**只动引号本身，不碰正文**（用户明确约束）。
///
/// 修复逻辑只依据「引号配对结构」与「源文引号结构」，不依赖对正文语义
/// （哪句是引语、哪句是叙述）的判断——后者是「只检测引号本身」的边界。
///
/// 可修（唯一无歧义，纯引号结构判定）：
///   恰好缺 1 个右引号 + 译文末尾无右引号 + 源文以引语结尾
///   → 译文对应也应以引语结尾 → 在末尾补 1 个右引号。
///   这里「源文以引语结尾」是源文自身的引号结构（不是对译文的语义解读），
///   作为「译文缺失右引号位置」的锚，不碰译文正文。
///
/// 不修（引号结构有歧义，留人工/复审）：
///   - 缺左引号 / 缺 ≥2 个右引号 / 末尾已有右引号（缺的是中间某段的右引号，
///     位置需正文语义判定，超出「只动引号」边界）；
///   - 源文以叙述结尾（引语在叙述前结束，补位不在末尾，需正文语义）。
///
/// 返回 (修复后文本, 修补数)。无修补或不可修时返回原文。
pub(crate) fn repair_quote_imbalance(source: &str, translated: &str) -> (String, usize) {
    let left = translated.matches('“').count();
    let right = translated.matches('”').count();
    // 仅恰好缺 1 个右引号
    if left != right + 1 {
        return (translated.to_string(), 0);
    }
    // 译文末尾已有右引号 → 缺的是中间段的右引号（位置需语义，不动）
    if translated.trim_end().ends_with('”') {
        return (translated.to_string(), 0);
    }
    // 源文以引语结尾（源文自身的引号结构，作为补位锚）→ 译文末尾补右引号
    if !source_ends_with_quote(source) {
        return (translated.to_string(), 0);
    }
    let mut out = translated.trim_end().to_string();
    out.push('”');
    (out, 1)
}

/// 源文是否以引语结尾：末尾字符是直双引号或曲右引号。
/// 这是源文自身的引号结构判定，不涉译文语义。
fn source_ends_with_quote(source: &str) -> bool {
    let t = source.trim_end();
    t.ends_with('"') || t.ends_with('”')
}
fn check_quote_balance(chunk_index: usize, translated: &str) -> Option<DriftSignal> {
    let left = translated.matches('“').count();
    let right = translated.matches('”').count();
    if left == right {
        return None;
    }
    Some(DriftSignal {
        code: "QUOTE_IMBALANCE",
        chunk_index,
        detail: format!(
            "曲引号不配对: 左“ ×{}, 右” ×{}; 译文: {}",
            left,
            right,
            compact_excerpt(translated, 70)
        ),
    })
}

fn compact_excerpt(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_imbalance_detected() {
        // 实测形态：两段引语合并导致左1右2
        let sig = check_quote_balance(0, "“没错。”她抬起下巴。你在找。”");
        assert_eq!(sig.map(|s| s.code), Some("QUOTE_IMBALANCE"));
        // 平衡则无声
        assert!(check_quote_balance(0, "“好的。”我说。“行。”").is_none());
        // 无引号也无声
        assert!(check_quote_balance(0, "她走向湖边。").is_none());
    }

    #[test]
    fn quote_repair_only_appends_when_source_ends_with_quote() {
        // 可修：恰好缺1右 + 译文末无右引号 + 源文以引语结尾
        let (out, n) = repair_quote_imbalance("“Please don’t go.”", "“请别走");
        assert_eq!(n, 1);
        assert_eq!(out, "“请别走”");

        // 不修：源文以叙述结尾（引语在叙述前结束，补位不在末尾）
        let (out2, n2) = repair_quote_imbalance("“OK,” I said.", "“好吧。我说。");
        assert_eq!(n2, 0);
        assert_eq!(out2, "“好吧。我说。");

        // 不修：译文末尾已有右引号（缺的是中间段的右引号，位置需语义）
        let (out3, n3) = repair_quote_imbalance("“A.” Jez said. “B.”", "“甲。杰兹说，“乙。”");
        assert_eq!(n3, 0);
        assert_eq!(out3, "“甲。杰兹说，“乙。”");

        // 不修：缺左引号
        let (_o4, n4) = repair_quote_imbalance("“Hi”", "你好”");
        assert_eq!(n4, 0);

        // 不修：缺2个右引号
        let (_o5, n5) = repair_quote_imbalance("“A.” “B.”", "“甲。“乙");
        assert_eq!(n5, 0);
    }

    #[test]
    fn heading_bilingual_residue_detected_and_repaired() {
        let sig = check_heading_bilingual_residue(0, "Chapter 6: 塔什");
        assert_eq!(sig.map(|s| s.code), Some("HEADING_BILINGUAL_RESIDUE"));
        assert_eq!(
            repair_heading_bilingual_residue("Chapter 6: 塔什"),
            "第六章：塔什"
        );
        assert_eq!(
            repair_heading_bilingual_residue("Chapter 87: 塔什"),
            "第八十七章：塔什"
        );
        // 全英标题（人名也没翻）不动
        assert_eq!(
            repair_heading_bilingual_residue("Chapter 6: Tash"),
            "Chapter 6: Tash"
        );
        // 已全译不动
        assert_eq!(
            repair_heading_bilingual_residue("第六章：塔什"),
            "第六章：塔什"
        );
    }

    #[test]
    fn heading_bilingual_residue_with_book_prefix() {
        // Book/Volume/Part/Section 前缀支持
        assert_eq!(
            repair_heading_bilingual_residue("Book Two, Chapter 12: 塔什"),
            "第二卷 第十二章：塔什"
        );
        assert_eq!(
            repair_heading_bilingual_residue("Volume 3: Chapter 5: 苏菲"),
            "第三卷 第五章：苏菲"
        );
        assert_eq!(
            repair_heading_bilingual_residue("Part III, Chapter 1: 芬恩"),
            "第三部 第一章：芬恩"
        );
        // 无前缀仍走原逻辑
        assert_eq!(
            repair_heading_bilingual_residue("Chapter 6: 塔什"),
            "第六章：塔什"
        );
    }

    #[test]
    fn number_conservation_detects_missing_number() {
        let sig = check_number_conservation(
            0,
            "The study found 37% of participants improved.",
            "研究发现参与者有所改善。",
        );
        assert_eq!(sig.map(|s| s.code), Some("NUMBER_CONSERVATION"));
        // 数字保留则无声
        assert!(check_number_conservation(
            0,
            "The study found 37% of participants improved.",
            "研究发现 37% 的参与者有所改善。",
        )
        .is_none());
        // 年份白名单
        assert!(
            check_number_conservation(0, "In 2019 the rate was 37%.", "2019 年该比率为 37%。",)
                .is_none()
        );
    }

    #[test]
    fn chinese_numeral_conversion() {
        assert_eq!(to_chinese_numeral(1), "一");
        assert_eq!(to_chinese_numeral(10), "十");
        assert_eq!(to_chinese_numeral(11), "十一");
        assert_eq!(to_chinese_numeral(20), "二十");
        assert_eq!(to_chinese_numeral(87), "八十七");
        assert_eq!(to_chinese_numeral(100), "100"); // 超范围回退
    }
}
