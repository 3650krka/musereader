use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(flatten)]
    pub extra_body: LlmExtraBody,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LlmExtraBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<ChatTemplateKwargs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_budget: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatTemplateKwargs {
    pub enable_thinking: bool,
}

#[derive(Debug, Deserialize)]
pub struct LlmResponse {
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<LlmUsage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: Message,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmUsageTotals {
    pub prompt_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub uncached_prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub response_count: u64,
}

/// 从已解析响应 JSON 中提取 usage，统一三种协议命名（问题：Anthropic/Responses/流式路径计数为 0）：
/// - OpenAI Chat：`usage.prompt_tokens/completion_tokens[/prompt_tokens_details.cached_tokens]`
/// - Anthropic Messages / OpenAI Responses：`usage.input_tokens/output_tokens`
///   （Anthropic 缓存读记在 `cache_read_input_tokens`，Responses 在 `input_tokens_details.cached_tokens`）
/// usage 缺失或全 0 视为无用量（返回 None，不虚增 response_count）。
pub fn usage_from_json(value: &serde_json::Value) -> Option<LlmUsage> {
    let usage = value.get("usage").filter(|usage| !usage.is_null())?;
    let pick = |names: &[&str]| -> u64 {
        names
            .iter()
            .find_map(|name| usage.get(*name).and_then(|v| v.as_u64()))
            .unwrap_or_default()
    };
    let prompt_tokens = pick(&["prompt_tokens", "input_tokens"]);
    let completion_tokens = pick(&["completion_tokens", "output_tokens"]);
    if prompt_tokens == 0 && completion_tokens == 0 {
        return None;
    }
    let total_tokens =
        pick(&["total_tokens"]).max(prompt_tokens.saturating_add(completion_tokens));
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or_default()
        .min(prompt_tokens);
    Some(LlmUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: cached,
        }),
    })
}

impl LlmUsageTotals {
    pub fn add_usage(&mut self, usage: LlmUsage) {
        let cached = usage
            .prompt_tokens_details
            .map(|details| details.cached_tokens)
            .unwrap_or_default()
            .min(usage.prompt_tokens);
        self.prompt_tokens = self.prompt_tokens.saturating_add(usage.prompt_tokens);
        self.cached_prompt_tokens = self.cached_prompt_tokens.saturating_add(cached);
        self.uncached_prompt_tokens = self
            .uncached_prompt_tokens
            .saturating_add(usage.prompt_tokens.saturating_sub(cached));
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(usage.total_tokens);
        self.response_count = self.response_count.saturating_add(1);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedTerm {
    pub source: String,
    pub translation: String,
    pub term_type: String,
    #[serde(default)]
    pub confidence: u8,
    #[serde(default)]
    pub usage_role: String,
}

impl ConfirmedTerm {
    pub(crate) fn has_untranslated_source_token(&self) -> bool {
        if self.allows_source_preservation() {
            return false;
        }
        let translation_lower = self.translation.to_ascii_lowercase();
        source_ascii_tokens(&self.source)
            .into_iter()
            .any(|token| is_untranslated_source_token(&translation_lower, &token))
    }

    fn allows_source_preservation(&self) -> bool {
        matches!(self.term_type.as_str(), "person" | "organization")
            && matches!(
                self.usage_role.as_str(),
                "person_name" | "organization_name"
            )
    }
}

fn is_untranslated_source_token(translation_lower: &str, token: &str) -> bool {
    if token.len() < 4 || is_allowed_preserved_token(token) {
        return false;
    }
    contains_ascii_word(translation_lower, token)
}

fn source_ascii_tokens(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in source.chars() {
        if ch.is_ascii_alphabetic() {
            current.push(ch.to_ascii_lowercase());
            continue;
        }
        push_source_token(&mut tokens, &mut current);
    }
    push_source_token(&mut tokens, &mut current);
    tokens
}

fn push_source_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_allowed_preserved_token(token: &str) -> bool {
    matches!(
        token,
        "plus"
            | "minus"
            | "alpha"
            | "beta"
            | "gamma"
            | "delta"
            | "covid"
            | "internet"
            | "email"
            | "online"
    )
}

fn contains_ascii_word(text: &str, token: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = token.as_bytes();
    if needle.is_empty() || needle.len() > bytes.len() {
        return false;
    }
    bytes
        .windows(needle.len())
        .enumerate()
        .any(|(index, window)| {
            window == needle
                && !is_ascii_word_byte(bytes.get(index.wrapping_sub(1)).copied())
                && !is_ascii_word_byte(bytes.get(index + needle.len()).copied())
        })
}

fn is_ascii_word_byte(byte: Option<u8>) -> bool {
    byte.is_some_and(|item| item.is_ascii_alphanumeric() || item == b'_')
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn usage_totals_split_cached_and_uncached_prompt_tokens() {
        let mut totals = LlmUsageTotals::default();
        totals.add_usage(LlmUsage {
            prompt_tokens: 100,
            completion_tokens: 30,
            total_tokens: 130,
            prompt_tokens_details: Some(PromptTokensDetails { cached_tokens: 40 }),
        });

        assert_eq!(totals.prompt_tokens, 100);
        assert_eq!(totals.cached_prompt_tokens, 40);
        assert_eq!(totals.uncached_prompt_tokens, 60);
        assert_eq!(totals.completion_tokens, 30);
        assert_eq!(totals.total_tokens, 130);
        assert_eq!(totals.response_count, 1);
    }

    #[test]
    fn usage_totals_clamp_cached_tokens_to_prompt_tokens() {
        let mut totals = LlmUsageTotals::default();
        totals.add_usage(LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 3,
            total_tokens: 13,
            prompt_tokens_details: Some(PromptTokensDetails { cached_tokens: 99 }),
        });

        assert_eq!(totals.cached_prompt_tokens, 10);
        assert_eq!(totals.uncached_prompt_tokens, 0);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StructuredTranslation {
    pub translated_text: String,
    #[serde(default)]
    pub confirmed_terms: Vec<ConfirmedTerm>,
}
