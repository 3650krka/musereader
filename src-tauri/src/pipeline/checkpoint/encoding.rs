use crate::error::AppError;

#[derive(Debug, Clone, Copy)]
pub(super) struct MojibakeScore {
    pub(super) is_polluted: bool,
    pub(super) suspicious_score: usize,
    pub(super) chars: usize,
}

pub(super) fn reject_mojibake_translation(
    chunk_index: usize,
    translated: &str,
) -> Result<(), AppError> {
    let score = mojibake_score(translated);
    if score.is_polluted {
        return Err(AppError::new(
            "TRANSLATION_ENCODING_POLLUTION",
            format!(
                "chunk={chunk_index} translated text appears to contain encoding pollution; score={}/{}; preview={}",
                score.suspicious_score,
                score.chars,
                compact_text_preview(translated, 160)
            ),
            true,
        ));
    }

    Ok(())
}

pub(super) fn looks_like_mojibake(text: &str) -> bool {
    mojibake_score(text).is_polluted
}

fn mojibake_score(text: &str) -> MojibakeScore {
    let chars = text.chars().count();
    if chars < 4 {
        return MojibakeScore {
            is_polluted: false,
            suspicious_score: 0,
            chars,
        };
    }

    // 单趟扫描：21 个 CJK marker + PUA + latin1 + block + U+FFFD 一次判完，
    // 避免 25 趟全文扫描（每 chunk 含续跑校验对已译 chunk 全量跑）。
    let mut private_use = 0usize;
    let mut latin1_artifacts = 0usize;
    let mut block_artifacts = 0usize;
    let mut cjk_noise = 0usize;
    let mut replacement_count = 0usize;
    for ch in text.chars() {
        if ('\u{e000}'..='\u{f8ff}').contains(&ch) {
            private_use += 1;
        }
        if matches!(ch, '\u{20ac}' | '\u{2122}') {
            latin1_artifacts += 1;
        }
        if ch == '\u{2588}' {
            block_artifacts += 1;
        }
        if ch == '\u{fffd}' {
            replacement_count += 1;
        }
        if is_mojibake_cjk_marker(ch) {
            cjk_noise += 1;
        }
    }
    let strong_artifacts = private_use + latin1_artifacts + block_artifacts + replacement_count;
    let suspicious_score = strong_artifacts + cjk_noise;
    let strong_artifact_density =
        strong_artifacts >= 2 && suspicious_score.saturating_mul(100) / chars >= 2;
    // 替换符专项：单个 U+FFFD 即视为硬错误。
    //
    // 依据：U+FFFD 是 Unicode 解码失败后的替换符，正常译文**永远不会**合法包含它——
    // 它一旦出现，必然意味着模型输出流被截断/损坏了对应字符（实测 inkling 模型以
    // 8.1% 的块率稀疏丢失单个字符，原「≥2 个且密度 ≥2%」阈值对此完全失明）。
    // 与 PUA/块符/Latin-1 artifact 不同（这些偶见于源文本本身的特殊排版），
    // U+FFFD 没有合法来源，故无需密度阈值，一个即污染。
    let replacement_char_present = replacement_count >= 1;

    MojibakeScore {
        is_polluted: strong_artifact_density || replacement_char_present,
        suspicious_score,
        chars,
    }
}

fn is_mojibake_cjk_marker(ch: char) -> bool {
    matches!(
        ch,
        '\u{7487}'
            | '\u{5997}'
            | '\u{55d8}'
            | '\u{7066}'
            | '\u{6fe1}'
            | '\u{5099}'
            | '\u{7d8d}'
            | '\u{6fc9}'
            | '\u{6226}'
            | '\u{72b2}'
            | '\u{7ec0}'
            | '\u{7459}'
            | '\u{52ee}'
            | '\u{5bd6}'
            | '\u{9438}'
            | '\u{942d}'
            | '\u{74a7}'
            | '\u{93b4}'
            | '\u{6d7c}'
            | '\u{6fc2}'
            | '\u{9359}'
    )
}

pub(super) fn compact_text_preview(text: &str, limit: usize) -> String {
    text.chars()
        .take(limit)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::mojibake_score;

    #[test]
    fn single_replacement_char_is_polluted() {
        // inkling 稀疏丢字符的实证形态：一个 U+FFFD 夹在正常中文里。
        let text = "她的袖口湿\u{fffd}兮的，贴在手腕上。";
        assert!(
            mojibake_score(text).is_polluted,
            "single U+FFFD must be rejected"
        );
    }

    #[test]
    fn clean_chinese_is_not_polluted() {
        let text = "她的袖口湿乎乎的，贴在手腕上。我们沿着悬崖走了很久。";
        assert!(!mojibake_score(text).is_polluted);
    }

    #[test]
    fn original_density_threshold_still_applies() {
        // PUA ×2 且密度达标仍判污染（不依赖替换符通道）。
        let text = "前面\u{e000}\u{e001}后面再补一点正文让密度达标";
        assert!(mojibake_score(text).is_polluted);
    }

    #[test]
    fn short_text_under_4_chars_exempt() {
        assert!(
            !mojibake_score("好\u{fffd}").is_polluted,
            "short text stays exempt"
        );
    }
}
