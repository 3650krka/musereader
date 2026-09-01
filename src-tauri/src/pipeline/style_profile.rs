//! 风格画像（style profile）：翻译前对全书样本段落做一次性 LLM 分析，
//! 产出抽象风格规则注入系统提示词，约束全书译文的语域与声音统一。
//!
//! 设计（借鉴 style-extractor 思路并按我们的法则收紧）：
//! - 规则必须**抽象**：禁止引用具体词表/例句（否则模型把它当词库反复用，
//!   译文出现"风格词堆砌"）。lint 会剥离含引号示例/词表形态的规则。
//! - 附带**反词表化 guard**（规则是倾向不是词表）与**反时代错置 guard**
//!   （不用晚于故事年代的词汇），与规则一并注入。
//! - 结果按 source_hash 缓存到 artifact（style_profile.json），断点续跑
//!   不重复调用；提取失败静默降级为无画像（绝不阻塞翻译主链路）。

use crate::error::AppError;
use crate::llm::SensenovaClient;
use serde::{Deserialize, Serialize};
use std::path::Path;

const STYLE_PROFILE_VERSION: u8 = 1;
const SAMPLE_BLOCK_COUNT: usize = 6;
const SAMPLE_MAX_CHARS: usize = 4000;
const STYLE_RULE_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct StyleProfile {
    pub version: u8,
    pub source_hash: String,
    pub setting: String,
    pub rules: Vec<String>,
    /// 全书摘要：每 chunk 恒定的全局语境，
    /// 补充术语锁之外的叙事连续性。一次 LLM 调用、随画像一同缓存。
    #[serde(default)]
    pub summary: String,
}

impl StyleProfile {
    /// 注入系统提示词的规则块（含摘要与两条 guard）。
    pub(super) fn to_prompt_block(&self) -> String {
        let mut lines = vec!["Style profile for this book:".to_string()];
        if !self.summary.trim().is_empty() {
            lines.push(format!("- Book summary: {}", self.summary.trim()));
        }
        if !self.setting.trim().is_empty() {
            lines.push(format!("- Setting: {}", self.setting.trim()));
            lines.push(
                "- Do not use words that belong to a later era or a different technological level than this setting, even when they are the most direct equivalent."
                    .to_string(),
            );
        }
        for rule in &self.rules {
            lines.push(format!("- {rule}"));
        }
        lines.push(
            "- Treat these rules as tendencies of the writing, not as a vocabulary. Never reuse a fixed set of words to satisfy them; whenever a rule and the natural phrasing of a passage conflict, favour the natural phrasing."
                .to_string(),
        );
        lines.join("\n")
    }
}

/// 从全书 markdown 采样代表性段落（首/中/尾各取若干非标题块）。
pub(super) fn sample_passages_for_style(markdown: &str) -> String {
    let blocks: Vec<&str> = markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| {
            !block.is_empty()
                && !block.starts_with('#')
                && !block.starts_with("<!--")
                && block.chars().count() >= 80
        })
        .collect();
    if blocks.is_empty() {
        return String::new();
    }
    let mut picked = Vec::new();
    let step = (blocks.len() / SAMPLE_BLOCK_COUNT).max(1);
    let mut index = 0usize;
    while picked.len() < SAMPLE_BLOCK_COUNT && index < blocks.len() {
        picked.push(blocks[index]);
        index += step;
    }
    let mut sample = picked.join("\n\n");
    if sample.chars().count() > SAMPLE_MAX_CHARS {
        sample = sample.chars().take(SAMPLE_MAX_CHARS).collect();
    }
    sample
}

/// 调用 LLM 提取抽象风格规则；失败返回 None（降级，不阻塞）。
pub(super) async fn extract_style_profile(
    llm_client: &SensenovaClient,
    source_hash: &str,
    source_markdown: &str,
) -> Option<StyleProfile> {
    let sample = sample_passages_for_style(source_markdown);
    if sample.trim().is_empty() {
        return None;
    }
    let prompt = build_style_extraction_prompt(&sample);
    let route = llm_client.reserve_route();
    let response = llm_client
        .review_translation_issue_on_route(&route, &prompt)
        .await
        .ok()?;
    parse_style_profile_response(source_hash, &response)
}

fn build_style_extraction_prompt(sample: &str) -> String {
    format!(
        "You are a literary style analyst. Read the following excerpt from a book and produce a compact style profile to guide a translator.\n\
         \n\
         Output JSON only, with this exact shape:\n\
         {{\"summary\": \"...\", \"setting\": \"...\", \"rules\": [\"...\", \"...\"]}}\n\
         \n\
         Requirements:\n\
         - \"summary\": 3-5 sentences summarizing the book's premise, main characters, and tone — written for a translator who will see only one chunk at a time and needs the global arc. Write it in English.\n\
         - \"setting\": one line describing era, technological level, and social frame inferred from the text (e.g. \"contemporary urban\", \"medieval European fantasy\"). If unclear, write \"unclear\".\n\
         - \"rules\": 4-8 ABSTRACT style rules covering: register (formal/casual), narrative voice, sentence rhythm, dialogue style, imagery density, humor/tone.\n\
         - Rules MUST be abstract tendencies. Do NOT quote specific words or sentences from the excerpt. Do NOT include word lists, examples, or \"such as\" phrases.\n\
         - Write rules in English (the translator LLM consumes them as-is).\n\
         \n\
         Excerpt:\n\
         ---\n\
         {sample}\n\
         ---"
    )
}

fn parse_style_profile_response(source_hash: &str, response: &str) -> Option<StyleProfile> {
    let json_text = extract_json_object(response)?;
    let parsed: serde_json::Value = serde_json::from_str(&json_text).ok()?;
    let setting = parsed
        .get("setting")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let setting = if setting.eq_ignore_ascii_case("unclear") {
        String::new()
    } else {
        setting
    };
    let summary = parsed
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let rules = parsed
        .get("rules")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
        .filter(|rule| lint_abstract_rule(rule))
        .take(STYLE_RULE_LIMIT)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return None;
    }
    Some(StyleProfile {
        version: STYLE_PROFILE_VERSION,
        source_hash: source_hash.to_string(),
        setting,
        rules,
        summary,
    })
}

/// 反词表化 lint：含引号示例、"such as"/"e.g."/"例如"、或冒号后词表形态的规则
/// 被判为具体化，剔除（宁缺毋滥——抽象不足的风格规则弊大于利）。
fn lint_abstract_rule(rule: &str) -> bool {
    let lower = rule.to_ascii_lowercase();
    if rule.contains('"') || rule.contains('"') || rule.contains('"') || rule.contains('\'') {
        return false;
    }
    for marker in ["such as", "e.g.", "for example", "like \"", "words like"] {
        if lower.contains(marker) {
            return false;
        }
    }
    // 词表形态："use words: a, b, c" / "词汇: 甲、乙"
    if lower.contains("use words") || rule.contains("词汇") {
        return false;
    }
    true
}

fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(text[start..start + offset + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn load_cached_style_profile(path: &Path, source_hash: &str) -> Option<StyleProfile> {
    let raw = std::fs::read_to_string(path).ok()?;
    let profile: StyleProfile = serde_json::from_str(&raw).ok()?;
    if profile.version == STYLE_PROFILE_VERSION && profile.source_hash == source_hash {
        Some(profile)
    } else {
        None
    }
}

pub(super) fn persist_style_profile(path: &Path, profile: &StyleProfile) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(profile)
        .map_err(|error| AppError::internal(format!("serialize style profile failed: {error}")))?;
    std::fs::write(path, json)
        .map_err(|error| AppError::internal(format!("write style profile failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_passages_skips_headings_and_short_blocks() {
        let markdown = "# Title\n\nShort.\n\nThis is a sufficiently long paragraph of narrative prose that describes the morning light over the harbour and the gulls circling above the water.\n\n## Chapter 2\n\nAnother long paragraph follows here, continuing the story with dialogue and description that easily exceeds eighty characters in total length.";
        let sample = sample_passages_for_style(markdown);
        assert!(sample.contains("morning light"), "sample: {sample}");
        assert!(
            sample.contains("Another long paragraph"),
            "sample: {sample}"
        );
        assert!(!sample.contains("# Title"), "sample: {sample}");
        assert!(!sample.contains("Short."), "sample: {sample}");
    }

    #[test]
    fn parses_style_response_and_lints_concrete_rules() {
        let response = r#"Here is the profile:
        {"setting": "medieval European fantasy", "rules": [
            "Maintain a formal, measured register with long periodic sentences",
            "Use words like \"thee\" and \"thou\" for archaic flavor",
            "Keep dialogue terse and laden with subtext",
            "Prefer concrete sensory imagery over abstract commentary, such as describing cold stone",
            "Allow dry understatement for humor"
        ]}"#;
        let profile = parse_style_profile_response("hash-1", response).expect("should parse");
        assert_eq!(profile.setting, "medieval European fantasy");
        assert_eq!(profile.rules.len(), 3, "rules: {:?}", profile.rules);
        assert!(profile.rules.iter().all(|rule| !rule.contains('"')));
        assert!(profile
            .rules
            .iter()
            .all(|rule| !rule.to_ascii_lowercase().contains("such as")));
    }

    #[test]
    fn unclear_setting_is_dropped() {
        let response =
            r#"{"setting": "unclear", "rules": ["Keep a neutral, transparent register"]}"#;
        let profile = parse_style_profile_response("hash-1", response).expect("should parse");
        assert!(profile.setting.is_empty());
    }

    #[test]
    fn empty_or_all_linted_rules_yield_none() {
        let response = r#"{"setting": "modern", "rules": ["Use words like \"big\" and \"bad\""]}"#;
        assert!(parse_style_profile_response("hash-1", response).is_none());
    }

    #[test]
    fn prompt_block_contains_guards() {
        let profile = StyleProfile {
            version: STYLE_PROFILE_VERSION,
            source_hash: "hash".to_string(),
            setting: "contemporary urban".to_string(),
            rules: vec!["Keep dialogue naturalistic".to_string()],
            summary: "A courier uncovers a conspiracy in a coastal city.".to_string(),
        };
        let block = profile.to_prompt_block();
        assert!(block.contains("Setting: contemporary urban"));
        assert!(block.contains("later era or a different technological level"));
        assert!(block.contains("tendencies of the writing, not as a vocabulary"));
        assert!(block.contains("- Keep dialogue naturalistic"));
        assert!(block.contains("Book summary: A courier uncovers a conspiracy"));
    }

    #[test]
    fn parses_summary_from_response() {
        let response = r#"{"summary": "An apprentice mage flees a fallen order.", "setting": "unclear", "rules": ["Keep a brisk, wry narrative voice"]}"#;
        let profile = parse_style_profile_response("hash-1", response).expect("should parse");
        assert_eq!(profile.summary, "An apprentice mage flees a fallen order.");
    }

    #[test]
    fn cached_profile_requires_matching_source_hash() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-style-profile-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("style_profile.json");
        let profile = StyleProfile {
            version: STYLE_PROFILE_VERSION,
            source_hash: "hash-a".to_string(),
            setting: String::new(),
            rules: vec!["Rule".to_string()],
            summary: String::new(),
        };
        persist_style_profile(&path, &profile).expect("persist");
        assert!(load_cached_style_profile(&path, "hash-a").is_some());
        assert!(load_cached_style_profile(&path, "hash-b").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
