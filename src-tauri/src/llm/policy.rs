use super::{ChatTemplateKwargs, LlmExtraBody, ResponseFormat};
use crate::commands::ModelPolicyConfig;
#[cfg(test)]
use crate::error::{ProviderError, ProviderErrorKind};

/// 测试用全局环境变量锁：所有改动 `MUSETRANSLATE_*` 环境变量的测试模块必须共用此锁，
/// 否则并行的测试线程会互相污染（例如 llm::request_config_tests 与 llm::policy::tests
/// 曾各持一把私有锁，导致 reasoning_effort 默认值断言间歇性失败）。
#[cfg(test)]
pub(crate) fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

const DEFAULT_SENSENOVA_FALLBACK_QUOTA_FAILURES: u8 = 5;
const DEFAULT_LLM_TIMEOUT_SECONDS: u64 = 900;
const MIN_LLM_TIMEOUT_SECONDS: u64 = 60;
const MAX_LLM_TIMEOUT_SECONDS: u64 = 900;
const DEFAULT_LLM_CONNECT_TIMEOUT_SECONDS: u64 = 30;
const MIN_LLM_CONNECT_TIMEOUT_SECONDS: u64 = 5;
const MAX_LLM_CONNECT_TIMEOUT_SECONDS: u64 = 120;
const GOOGLE_DIFFUSIONGEMMA_MODEL: &str = "google/diffusiongemma-26b-a4b-it";
const DEFAULT_DIFFUSIONGEMMA_MAX_TOKENS: u32 = 200_000;
const DIFFUSIONGEMMA_TEMPERATURE: f32 = 1.0;
const DIFFUSIONGEMMA_TOP_P: f32 = 0.95;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRoutingKey {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) key_fingerprint: String,
}

impl ProviderRoutingKey {
    pub(crate) fn new(api_url: &str, model: &str, api_key: &str) -> Self {
        Self {
            provider: provider_name_from_url(api_url),
            model: model.trim().to_string(),
            key_fingerprint: fingerprint_api_key(api_key),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderRequestPurpose {
    Translation,
    Instruction,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProviderRequestPolicy {
    pub(crate) temperature: Option<f32>,
    pub(crate) top_p: Option<f32>,
    pub(crate) stream: Option<bool>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) response_format: Option<ResponseFormat>,
    pub(crate) max_tokens: Option<u32>,
    pub(crate) extra_body: LlmExtraBody,
}

impl ProviderRequestPolicy {
    pub(crate) fn for_model(model: &str, purpose: ProviderRequestPurpose) -> Self {
        // 外置策略优先（models.local.json 按模型名精确匹配）；缺省字段回落通用默认。
        let policy_override = model_policy_override(model);
        let reasoning_effort = match purpose {
            ProviderRequestPurpose::Translation => translation_reasoning_effort(model),
            ProviderRequestPurpose::Instruction => instruction_reasoning_effort(model),
        };
        let response_format = match purpose {
            ProviderRequestPurpose::Translation => translation_response_format(model),
            ProviderRequestPurpose::Instruction => instruction_response_format(model),
        };
        Self {
            temperature: temperature_for_model(model),
            top_p: top_p_for_model(model),
            stream: stream_for_model(model),
            reasoning_effort,
            response_format,
            max_tokens: max_tokens_for_model(model),
            extra_body: extra_body_for_model(model, purpose),
        }
        .apply_model_override(policy_override.as_ref())
    }

    fn apply_model_override(mut self, override_policy: Option<&ModelPolicyConfig>) -> Self {
        let Some(policy) = override_policy else {
            return self;
        };
        if let Some(value) = policy.temperature {
            self.temperature = Some(value);
        }
        if let Some(value) = policy.top_p {
            self.top_p = Some(value);
        }
        if let Some(value) = policy.stream {
            self.stream = Some(value);
        }
        if let Some(value) = policy.max_tokens {
            self.max_tokens = Some(value);
        }
        if let Some(value) = policy.json_response_format {
            self.response_format = value.then(|| ResponseFormat {
                kind: "json_object".to_string(),
            });
        }
        if let Some(effort) = policy.translation_reasoning_effort.as_deref() {
            if matches!(
                self.reasoning_effort.as_deref(),
                Some("none" | "low" | "medium" | "high") | None
            ) {
                self.reasoning_effort = Some(effort.to_string());
            }
        }
        if let Some(budget) = policy.reasoning_budget {
            self.extra_body.reasoning_budget = Some(budget);
        }
        if let Some(enable) = policy.enable_thinking {
            self.extra_body.chat_template_kwargs = Some(ChatTemplateKwargs {
                enable_thinking: enable,
            });
        }
        self
    }
}

// 外置模型策略查找（models.local.json）。按模型名精确匹配，第一个命中生效。
fn model_policy_override(model: &str) -> Option<ModelPolicyConfig> {
    crate::commands::load_model_policies()
        .into_iter()
        .find(|policy| policy.model.trim().eq_ignore_ascii_case(model.trim()))
}

pub(crate) fn translation_reasoning_effort(model: &str) -> Option<String> {
    let explicit = std::env::var("MUSETRANSLATE_REASONING_EFFORT")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "none" | "low" | "medium" | "high"));
    if explicit.is_some() {
        return explicit;
    }
    is_deepseek_v4_flash(model).then(|| "medium".to_string())
}

pub(crate) fn instruction_reasoning_effort(model: &str) -> Option<String> {
    let explicit = std::env::var("MUSETRANSLATE_INSTRUCTION_REASONING_EFFORT")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "none" | "low" | "medium" | "high"));
    if explicit.is_some() {
        return explicit;
    }
    is_deepseek_v4_flash(model).then(|| "none".to_string())
}

pub(crate) fn translation_response_format(model: &str) -> Option<ResponseFormat> {
    supports_json_response_format(model).then(|| ResponseFormat {
        kind: "json_object".to_string(),
    })
}

pub(crate) fn instruction_response_format(model: &str) -> Option<ResponseFormat> {
    translation_response_format(model)
}

pub(crate) fn resolve_llm_timeout_seconds() -> u64 {
    std::env::var("MUSETRANSLATE_LLM_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_LLM_TIMEOUT_SECONDS)
        .clamp(MIN_LLM_TIMEOUT_SECONDS, MAX_LLM_TIMEOUT_SECONDS)
}

pub(crate) fn resolve_llm_connect_timeout_seconds() -> u64 {
    std::env::var("MUSETRANSLATE_LLM_CONNECT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_LLM_CONNECT_TIMEOUT_SECONDS)
        .clamp(
            MIN_LLM_CONNECT_TIMEOUT_SECONDS,
            MAX_LLM_CONNECT_TIMEOUT_SECONDS,
        )
}

#[cfg(test)]
pub(crate) fn should_fallback_for_quota(error: &ProviderError) -> bool {
    matches!(error.kind, ProviderErrorKind::QuotaExhausted)
}

#[cfg(test)]
pub(crate) fn merge_fallback_error(
    primary_model: &str,
    primary_error: &ProviderError,
    fallback_model: &str,
    fallback_error: ProviderError,
) -> ProviderError {
    ProviderError::new(
        fallback_error.provider,
        fallback_error.kind,
        format!(
            "primary model {primary_model} failed with quota exhaustion: {}; fallback model {fallback_model} failed: {}",
            primary_error.message, fallback_error.message
        ),
    )
}

pub(crate) fn fallback_model_for(api_url: &str, current_model: &str) -> Option<String> {
    if let Some(model) = configured_generic_fallback_models()
        .into_iter()
        .find(|model| !model.eq_ignore_ascii_case(current_model))
    {
        return Some(model);
    }
    if !is_sensenova_endpoint(api_url) {
        return None;
    }
    configured_sensenova_fallback_models()
        .into_iter()
        .find(|model| !model.eq_ignore_ascii_case(current_model))
}

pub(crate) fn sensenova_fallback_quota_failure_threshold() -> u8 {
    std::env::var("MUSETRANSLATE_LLM_FALLBACK_AFTER_QUOTA_FAILURES")
        .or_else(|_| std::env::var("MUSETRANSLATE_SENSENOVA_FALLBACK_AFTER_QUOTA_FAILURES"))
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SENSENOVA_FALLBACK_QUOTA_FAILURES)
}

fn configured_generic_fallback_models() -> Vec<String> {
    std::env::var("MUSETRANSLATE_LLM_FALLBACK_MODELS")
        .or_else(|_| std::env::var("MUSETRANSLATE_LLM_FALLBACK_MODEL"))
        .ok()
        .map(|value| split_model_list(&value))
        .filter(|models| !models.is_empty())
        .unwrap_or_default()
}

fn configured_sensenova_fallback_models() -> Vec<String> {
    std::env::var("MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS")
        .or_else(|_| std::env::var("MUSETRANSLATE_SENSENOVA_FALLBACK_MODEL"))
        .ok()
        .map(|value| split_model_list(&value))
        .filter(|models| !models.is_empty())
        .unwrap_or_default()
}

fn split_model_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn is_sensenova_endpoint(api_url: &str) -> bool {
    api_url.to_ascii_lowercase().contains("sensenova.cn")
}

fn is_deepseek_v4_flash(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("deepseek-v4-flash")
}

fn is_nvidia_nemotron_ultra(model: &str) -> bool {
    model
        .trim()
        .eq_ignore_ascii_case("nvidia/nemotron-3-ultra-550b-a55b")
}

fn is_google_diffusiongemma(model: &str) -> bool {
    model
        .trim()
        .eq_ignore_ascii_case(GOOGLE_DIFFUSIONGEMMA_MODEL)
}

fn max_tokens_for_model(model: &str) -> Option<u32> {
    is_google_diffusiongemma(model).then(resolve_diffusiongemma_max_tokens)
}

fn temperature_for_model(model: &str) -> Option<f32> {
    is_google_diffusiongemma(model).then_some(DIFFUSIONGEMMA_TEMPERATURE)
}

fn top_p_for_model(model: &str) -> Option<f32> {
    is_google_diffusiongemma(model).then_some(DIFFUSIONGEMMA_TOP_P)
}

fn stream_for_model(model: &str) -> Option<bool> {
    is_google_diffusiongemma(model).then_some(false)
}

fn extra_body_for_model(model: &str, purpose: ProviderRequestPurpose) -> LlmExtraBody {
    if is_google_diffusiongemma(model) {
        return LlmExtraBody {
            chat_template_kwargs: Some(ChatTemplateKwargs {
                enable_thinking: true,
            }),
            reasoning_budget: None,
        };
    }
    if !is_nvidia_nemotron_ultra(model) {
        return LlmExtraBody::default();
    }
    let enable_thinking = explicit_bool_env("MUSETRANSLATE_NVIDIA_ENABLE_THINKING")
        .unwrap_or(matches!(purpose, ProviderRequestPurpose::Translation));
    if !enable_thinking {
        return LlmExtraBody::default();
    }
    LlmExtraBody {
        chat_template_kwargs: Some(ChatTemplateKwargs {
            enable_thinking: true,
        }),
        reasoning_budget: Some(resolve_nvidia_reasoning_budget()),
    }
}

fn supports_json_response_format(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    is_deepseek_v4_flash(model)
        || is_nvidia_nemotron_ultra(model)
        || normalized.starts_with("minimax-")
        || normalized.starts_with("command-")
        || normalized.starts_with("moonshotai/")
        || normalized.starts_with("nvidia/")
        || normalized.contains("/step-")
        || normalized.starts_with("step-")
        || normalized.starts_with("agnes-")
}

fn resolve_diffusiongemma_max_tokens() -> u32 {
    std::env::var("MUSETRANSLATE_DIFFUSIONGEMMA_MAX_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_DIFFUSIONGEMMA_MAX_TOKENS)
}

fn resolve_nvidia_reasoning_budget() -> u32 {
    std::env::var("MUSETRANSLATE_NVIDIA_REASONING_BUDGET")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(16_384)
}

fn explicit_bool_env(key: &str) -> Option<bool> {
    let value = std::env::var(key).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn provider_name_from_url(api_url: &str) -> String {
    let normalized = api_url.to_ascii_lowercase();
    if normalized.contains("sensenova.cn") {
        "sensenova".to_string()
    } else if normalized.contains("minimaxi.com") {
        "minimax".to_string()
    } else if normalized.contains("cohere.ai") {
        "cohere".to_string()
    } else if normalized.contains("integrate.api.nvidia.com") {
        "nvidia".to_string()
    } else if normalized.contains("agnes-ai.com") {
        "agnes".to_string()
    } else if normalized.contains("opencode.ai") {
        "opencode".to_string()
    } else {
        "llm".to_string()
    }
}

fn fingerprint_api_key(api_key: &str) -> String {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return "empty".to_string();
    }
    let visible = trimmed.chars().rev().take(6).collect::<Vec<_>>();
    let suffix = visible.into_iter().rev().collect::<String>();
    format!("len{}:{suffix}", trimmed.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        super::env_test_lock()
    }

    struct EnvRestore {
        key: &'static str,
        value: Option<String>,
    }

    impl EnvRestore {
        fn remove(key: &'static str) -> Self {
            let value = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, value }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            if let Some(value) = &self.value {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn deepseek_translation_defaults_to_json_mode_and_medium_reasoning() {
        let _guard = env_lock().lock().expect("env test lock");
        let _reasoning = EnvRestore::remove("MUSETRANSLATE_REASONING_EFFORT");
        let _instruction = EnvRestore::remove("MUSETRANSLATE_INSTRUCTION_REASONING_EFFORT");
        let reasoning = translation_reasoning_effort("deepseek-v4-flash");
        let response_format = translation_response_format("deepseek-v4-flash");

        assert_eq!(reasoning.as_deref(), Some("medium"));
        assert_eq!(
            response_format.as_ref().map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(
            instruction_response_format("deepseek-v4-flash")
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
    }

    #[test]
    fn provider_request_policy_preserves_translation_and_instruction_defaults() {
        let _guard = env_lock().lock().expect("env test lock");
        let _reasoning = EnvRestore::remove("MUSETRANSLATE_REASONING_EFFORT");
        let _instruction = EnvRestore::remove("MUSETRANSLATE_INSTRUCTION_REASONING_EFFORT");

        let translation = ProviderRequestPolicy::for_model(
            "deepseek-v4-flash",
            ProviderRequestPurpose::Translation,
        );
        let instruction = ProviderRequestPolicy::for_model(
            "deepseek-v4-flash",
            ProviderRequestPurpose::Instruction,
        );

        assert_eq!(translation.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(instruction.reasoning_effort.as_deref(), Some("none"));
        assert_eq!(
            translation
                .response_format
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(
            instruction
                .response_format
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(translation.max_tokens, None);
        assert_eq!(instruction.max_tokens, None);
    }

    #[test]
    fn diffusiongemma_policy_sets_explicit_configurable_max_tokens() {
        let _guard = env_lock().lock().expect("env test lock");
        let _max_tokens = EnvRestore::remove("MUSETRANSLATE_DIFFUSIONGEMMA_MAX_TOKENS");

        let translation = ProviderRequestPolicy::for_model(
            GOOGLE_DIFFUSIONGEMMA_MODEL,
            ProviderRequestPurpose::Translation,
        );
        let instruction = ProviderRequestPolicy::for_model(
            GOOGLE_DIFFUSIONGEMMA_MODEL,
            ProviderRequestPurpose::Instruction,
        );

        assert_eq!(translation.max_tokens, Some(200_000));
        assert_eq!(instruction.max_tokens, Some(200_000));
        assert_eq!(translation.temperature, Some(1.0));
        assert_eq!(instruction.temperature, Some(1.0));
        assert_eq!(translation.top_p, Some(0.95));
        assert_eq!(instruction.top_p, Some(0.95));
        assert_eq!(translation.stream, Some(false));
        assert_eq!(instruction.stream, Some(false));
        assert_eq!(
            translation
                .response_format
                .as_ref()
                .map(|value| value.kind.as_str()),
            None
        );
        assert_eq!(
            translation
                .extra_body
                .chat_template_kwargs
                .as_ref()
                .map(|kwargs| kwargs.enable_thinking),
            Some(true)
        );

        std::env::set_var("MUSETRANSLATE_DIFFUSIONGEMMA_MAX_TOKENS", "180000");
        let overridden = ProviderRequestPolicy::for_model(
            GOOGLE_DIFFUSIONGEMMA_MODEL,
            ProviderRequestPurpose::Translation,
        );
        assert_eq!(overridden.max_tokens, Some(180_000));
    }

    #[test]
    fn model_policy_override_from_models_local_applies_fields() {
        let _guard = env_lock().lock().expect("env test lock");
        // 注入 models.local.json 风格的外置策略：自定义 OpenAI compatible 模型
        let policy = crate::commands::ModelPolicyConfig {
            model: "custom-vendor/my-model".to_string(),
            json_response_format: Some(true),
            enable_thinking: Some(true),
            reasoning_budget: Some(4096),
            max_tokens: Some(8192),
            temperature: Some(0.7),
            top_p: Some(0.9),
            stream: Some(false),
            translation_reasoning_effort: Some("low".to_string()),
            instruction_reasoning_effort: Some("none".to_string()),
        };
        let base = ProviderRequestPolicy::for_model(
            "custom-vendor/my-model",
            ProviderRequestPurpose::Translation,
        );
        let applied = base.apply_model_override(Some(&policy));
        assert_eq!(applied.temperature, Some(0.7));
        assert_eq!(applied.top_p, Some(0.9));
        assert_eq!(applied.max_tokens, Some(8192));
        assert_eq!(
            applied
                .response_format
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(applied.extra_body.reasoning_budget, Some(4096));
        assert_eq!(
            applied
                .extra_body
                .chat_template_kwargs
                .as_ref()
                .map(|kwargs| kwargs.enable_thinking),
            Some(true)
        );
    }

    #[test]
    fn model_policy_override_absent_keeps_defaults() {
        let _guard = env_lock().lock().expect("env test lock");
        // 无外置策略时（None），默认行为不变
        let base = ProviderRequestPolicy::for_model(
            "deepseek-v4-flash",
            ProviderRequestPurpose::Translation,
        );
        let applied = base.clone().apply_model_override(None);
        assert_eq!(applied.max_tokens, base.max_tokens);
        assert_eq!(
            applied
                .response_format
                .as_ref()
                .map(|value| value.kind.as_str()),
            base.response_format
                .as_ref()
                .map(|value| value.kind.as_str())
        );
    }

    #[test]
    fn minimax_and_cohere_and_step_support_json_mode() {
        for model in [
            "MiniMax-M2.7",
            "command-a-plus-05-2026",
            "stepfun-ai/step-3.7-flash",
        ] {
            assert_eq!(
                translation_response_format(model)
                    .as_ref()
                    .map(|value| value.kind.as_str()),
                Some("json_object")
            );
        }
    }

    #[test]
    fn fallback_is_disabled_by_default_for_sensenova() {
        let _guard = env_lock().lock().expect("env test lock");
        let _fallbacks = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS");
        let _fallback = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODEL");
        assert_eq!(
            fallback_model_for(
                "https://token.sensenova.cn/v1/chat/completions",
                "deepseek-v4-flash"
            )
            .as_deref(),
            None
        );
    }

    #[test]
    fn routing_key_fingerprints_provider_model_and_key() {
        let key = ProviderRoutingKey::new(
            "https://api.cohere.ai/compatibility/v1",
            "command-a-plus-05-2026",
            "cohere-test-key-placeholder-0000000000000",
        );
        assert_eq!(key.provider, "cohere");
        assert_eq!(key.model, "command-a-plus-05-2026");
        assert!(key.key_fingerprint.starts_with("len"));
    }
}
