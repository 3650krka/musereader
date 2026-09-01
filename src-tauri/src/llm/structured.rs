use super::{ConfirmedTerm, StructuredTranslation};
use crate::error::{ProviderError, ProviderErrorKind};
use serde_json::Value;

pub(super) fn parse_structured_translation(
    content: &str,
) -> Result<StructuredTranslation, ProviderError> {
    let Some(json) = extract_json_object(content) else {
        // 截断修复：模型输出被 max_tokens/供应商默认上限截断（confirmedTerms 人名爆炸时
        // JSON 残缺、找不到平衡闭括号）。此时 translatedText 通常已完整出现在 JSON 前部，
        // 尝试「截断修复」——提取已完整的 translatedText、丢弃残缺的 confirmedTerms，
        // 而非让整个 chunk 失败重试（重试仍会因同样原因截断，死循环）。
        if let Some(text) = salvage_truncated_translated_text(content) {
            return Ok(StructuredTranslation {
                translated_text: text,
                confirmed_terms: Vec::new(),
            });
        }
        return Err(ProviderError::new(
            "llm",
            ProviderErrorKind::Decode,
            format!(
                "structured translation missing JSON object; preview={}",
                content.trim()
            ),
        ));
    };
    let parsed = serde_json::from_str::<StructuredTranslation>(json).map_err(|error| {
        ProviderError::new(
            "llm",
            ProviderErrorKind::Decode,
            format!(
                "parse structured translation failed: {error}; preview={}",
                content.trim()
            ),
        )
    })?;
    let fallback_text = extract_translation_text_alias(json);
    let translated_text = if parsed.translated_text.trim().is_empty() {
        fallback_text.unwrap_or_default()
    } else {
        parsed.translated_text.trim().to_string()
    };
    if translated_text.trim().is_empty() {
        return Err(ProviderError::new(
            "llm",
            ProviderErrorKind::EmptyResponse,
            "structured translation missing translatedText",
        ));
    }
    Ok(StructuredTranslation {
        translated_text,
        confirmed_terms: sanitize_confirmed_terms(parsed.confirmed_terms),
    })
}

fn extract_translation_text_alias(json: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(json).ok()?;
    let object = value.as_object()?;
    const CANDIDATE_KEYS: &[&str] = &[
        "translatedText",
        "translated_text",
        "translation",
        "translated",
        "text",
        "content",
        "output",
    ];
    for key in CANDIDATE_KEYS {
        if let Some(text) = object
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .and_then(|(_, value)| value.as_str())
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_json_object(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if trimmed.starts_with('{')
        && trimmed.ends_with('}')
        && looks_like_structured_translation_json(trimmed)
    {
        return Some(trimmed);
    }
    let bytes = trimmed.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'{' {
            continue;
        }
        if let Some(end) = find_balanced_json_object_end(bytes, index) {
            let candidate = &trimmed[index..=end];
            if looks_like_structured_translation_json(candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn find_balanced_json_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(start + index);
                }
            }
            _ => {}
        }
    }

    None
}

fn looks_like_structured_translation_json(candidate: &str) -> bool {
    candidate.contains("\"translatedText\"") || candidate.contains("\"translated_text\"")
}

/// 截断修复：从「被 max_tokens 截断的不完整 JSON」里抢救已完整的 translatedText。
///
/// 场景：confirmedTerms 人名爆炸（如致谢章 30+ 人名）时，模型输出超出供应商默认
/// 上限被截断，JSON 缺闭合括号、extract_json_object 找不到平衡对象。但 translatedText
/// 是 JSON 的第一个字段、通常已完整，直接放弃整个 chunk 去重试会因同样原因再截断
/// （死循环）。这里只提取 translatedText 的字符串值（按 JSON 字符串规则找其结束引号），
/// confirmedTerms 记空——宁可损失术语登记，不让整章翻译失败。
///
/// 返回 None 表示连 translatedText 都不完整（无法抢救），走正常 Decode 错误。
fn salvage_truncated_translated_text(content: &str) -> Option<String> {
    // 找 "translatedText" 键（也兼容 "translated_text"）。
    let key = if content.contains("\"translatedText\"") {
        "\"translatedText\""
    } else if content.contains("\"translated_text\"") {
        "\"translated_text\""
    } else {
        return None;
    };
    let key_pos = content.find(key)?;
    let after_key = &content[key_pos + key.len()..];
    // 跳过冒号与空白
    let value_start = after_key.find(|c: char| c == '"')?;
    let value_bytes = after_key[value_start..].as_bytes();
    // value_bytes[0] 是开引号；从 1 开始按 JSON 字符串规则找未转义闭引号
    let mut escaped = false;
    let mut end = None;
    for (i, byte) in value_bytes.iter().enumerate().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'"' => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    let raw_value = &after_key[value_start + 1..value_start + end];
    // 按 JSON 字符串反转义（\" \\ \/ \n \t \r \uXXXX）
    let decoded = serde_json::from_str::<String>(&format!("\"{raw_value}\"")).ok()?;
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn sanitize_confirmed_terms(terms: Vec<ConfirmedTerm>) -> Vec<ConfirmedTerm> {
    let mut deduped = Vec::new();
    for term in terms {
        let source = term.source.trim();
        let translation = term.translation.trim();
        if source.is_empty() || translation.is_empty() {
            continue;
        }
        let normalized = ConfirmedTerm {
            source: source.to_string(),
            translation: translation.to_string(),
            term_type: term.term_type.trim().to_string(),
            confidence: term.confidence.min(100),
            usage_role: term.usage_role.trim().to_string(),
        };
        if normalized.has_untranslated_source_token() {
            continue;
        }
        if deduped.iter().any(|existing: &ConfirmedTerm| {
            existing.source.eq_ignore_ascii_case(source) && existing.translation == translation
        }) {
            continue;
        }
        deduped.push(normalized);
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::extract_json_object;
    use crate::error::ProviderErrorKind;

    #[test]
    fn extracts_first_complete_json_object_after_think_block() {
        let raw = concat!(
            "<think>The model is reasoning about {noise} first.</think>\n\n",
            "{\"translatedText\":\"ok\",\"confirmedTerms\":[]}"
        );

        assert_eq!(
            extract_json_object(raw),
            Some("{\"translatedText\":\"ok\",\"confirmedTerms\":[]}")
        );
    }

    #[test]
    fn ignores_braces_inside_json_strings_when_extracting_object() {
        let raw =
            "prefix {\"translatedText\":\"value with { braces }\",\"confirmedTerms\":[]} suffix";

        assert_eq!(
            extract_json_object(raw),
            Some("{\"translatedText\":\"value with { braces }\",\"confirmedTerms\":[]}")
        );
    }

    #[test]
    fn falls_back_to_alias_field_when_translated_text_is_empty() {
        let raw = "{\"translatedText\":\"\",\"text\":\"别名正文\",\"confirmedTerms\":[]}";

        let parsed = super::parse_structured_translation(raw).expect("structured translation");

        assert_eq!(parsed.translated_text, "别名正文");
    }
    #[test]
    fn rejects_non_json_plaintext_response() {
        let raw = "This is not JSON.";

        let error =
            super::parse_structured_translation(raw).expect_err("plain text must be rejected");

        match error.kind {
            ProviderErrorKind::Decode => {}
            _ => panic!("expected decode error"),
        }
    }

    #[test]
    fn drops_confirmed_terms_with_untranslated_source_tokens() {
        let raw = concat!(
            "{\"translatedText\":\"一种 deliberative 工具。\",",
            "\"confirmedTerms\":[",
            "{\"source\":\"deliberative instrument\",",
            "\"translation\":\"deliberative 工具\",",
            "\"termType\":\"technical\",",
            "\"confidence\":95,",
            "\"usageRole\":\"technical_term\"}",
            "]}"
        );

        let parsed = super::parse_structured_translation(raw).expect("structured translation");

        assert!(parsed.confirmed_terms.is_empty());
    }

    #[test]
    fn salvages_translated_text_from_truncated_json() {
        // 模型输出被 max_tokens 截断：confirmedTerms 数组不完整、无闭合括号。
        // translatedText 已完整，应抢救出译文、confirmedTerms 记空，而非整个 chunk 失败。
        let raw = concat!(
            "{\"translatedText\":\"[[B000]]\\n我特别要感谢那些医生们。\\n\\n[[B001]]\\n《其他母亲》的灵感。\",",
            "\"confirmedTerms\":[",
            "{\"source\":\"The Other Mothers\",\"translation\":\"《其他母亲》\",\"termType\":\"title\",",
            "\"confidence\":95,\"usageRole\":\"coinage_modifier\"},",
            "{\"source\":\"Ben Crooks\",\"translation\":\"本·克鲁克斯\",\"termType\":\"person\",\"confiden"
        );
        let parsed = super::parse_structured_translation(raw)
            .expect("truncated json should salvage translatedText");
        assert!(parsed.translated_text.contains("我特别要感谢那些医生们"));
        assert!(parsed.translated_text.contains("《其他母亲》的灵感"));
        assert!(
            parsed.confirmed_terms.is_empty(),
            "confirmedTerms dropped on salvage"
        );
    }

    #[test]
    fn salvage_returns_none_when_translated_text_also_truncated() {
        // translatedText 本身也截断（字符串没闭合）→ 无法抢救 → Decode 错误
        let raw = "{\"translatedText\":\"[[B000]]\\n我特别要感谢";
        let result = super::parse_structured_translation(raw);
        assert!(result.is_err(), "unrecoverable truncation must error");
    }
}
