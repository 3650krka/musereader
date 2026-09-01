use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct EnvDefaults {
    pub(super) ocr_token: String,
    pub(super) llm_provider: String,
    pub(super) sensenova_api_key: String,
    pub(super) sensenova_base_url: String,
    pub(super) sensenova_model: String,
    pub(super) minimax_api_key: String,
    pub(super) minimax_base_url: String,
    pub(super) minimax_model: String,
    pub(super) cohere_api_key: String,
    pub(super) cohere_base_url: String,
    pub(super) cohere_model: String,
    pub(super) nvidia_api_key: String,
    pub(super) nvidia_base_url: String,
    pub(super) nvidia_model: String,
}

pub fn resolve_env_defaults(env_sources: &[HashMap<String, String>]) -> EnvDefaults {
    EnvDefaults {
        ocr_token: resolve_value(&["OCR_TOKEN", "BAIDU_OCR_TOKEN"], env_sources),
        llm_provider: resolve_value(&["LLM_PROVIDER", "MUSETRANSLATE_LLM_PROVIDER"], env_sources),
        sensenova_api_key: resolve_value(
            &["LLM_API_KEY", "SENSENOVA_API_KEY", "API_KEY", "api_key"],
            env_sources,
        ),
        sensenova_base_url: resolve_value(
            &["LLM_API_URL", "SENSENOVA_BASE_URL", "base_url"],
            env_sources,
        ),
        sensenova_model: resolve_value(&["LLM_MODEL", "SENSENOVA_MODEL", "MODEL"], env_sources),
        minimax_api_key: resolve_value(&["MINIMAX_API_KEY", "OPENAI_API_KEY"], env_sources),
        minimax_base_url: resolve_value(&["MINIMAX_BASE_URL", "OPENAI_BASE_URL"], env_sources),
        minimax_model: resolve_value(&["MINIMAX_MODEL"], env_sources),
        cohere_api_key: resolve_value(&["COHERE_API_KEY"], env_sources),
        cohere_base_url: resolve_value(&["COHERE_BASE_URL"], env_sources),
        cohere_model: resolve_value(&["COHERE_MODEL"], env_sources),
        nvidia_api_key: resolve_value(&["NVIDIA_API_KEY"], env_sources),
        nvidia_base_url: resolve_value(&["NVIDIA_BASE_URL"], env_sources),
        nvidia_model: resolve_value(&["NVIDIA_MODEL"], env_sources),
    }
}

pub fn load_runtime_env_sources() -> Vec<HashMap<String, String>> {
    let mut sources = runtime_env_candidates()
        .into_iter()
        .map(|path| load_envish_file(&path))
        .collect::<Vec<_>>();
    if let Some(provider_defaults) = load_provider_local_defaults() {
        sources.push(provider_defaults);
    }
    sources
}

fn runtime_env_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join(".env"));
    if let Some(parent) = manifest_dir.parent() {
        candidates.push(parent.join(".env"));
    }
    if let Some(grandparent) = manifest_dir.parent().and_then(Path::parent) {
        candidates.push(grandparent.join(".env"));
    }
    candidates
}

fn providers_local_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("providers.local.json"));
    if let Some(parent) = manifest_dir.parent() {
        candidates.push(parent.join("providers.local.json"));
    }
    if let Some(grandparent) = manifest_dir.parent().and_then(Path::parent) {
        candidates.push(grandparent.join("providers.local.json"));
    }
    candidates
}

fn pools_local_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("pools.local.json"));
    if let Some(parent) = manifest_dir.parent() {
        candidates.push(parent.join("pools.local.json"));
    }
    if let Some(grandparent) = manifest_dir.parent().and_then(Path::parent) {
        candidates.push(grandparent.join("pools.local.json"));
    }
    candidates
}

fn resolve_value(keys: &[&str], source_maps: &[HashMap<String, String>]) -> String {
    env_value(keys).unwrap_or_else(|| file_value(keys, source_maps).unwrap_or_default())
}

fn env_value(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn file_value(keys: &[&str], source_maps: &[HashMap<String, String>]) -> Option<String> {
    source_maps.iter().find_map(|mapping| {
        keys.iter().find_map(|key| {
            mapping
                .get(*key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    })
}

fn load_envish_file(path: &Path) -> HashMap<String, String> {
    if !path.exists() {
        return HashMap::new();
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    content.lines().filter_map(parse_assignment_line).collect()
}

fn load_provider_local_defaults() -> Option<HashMap<String, String>> {
    providers_local_candidates()
        .into_iter()
        .find_map(|path| load_provider_local_file(&path))
        .filter(|defaults| !defaults.is_empty())
}

fn load_provider_local_file(path: &Path) -> Option<HashMap<String, String>> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    provider_local_defaults_from_str(&content)
}

fn provider_local_defaults_from_str(content: &str) -> Option<HashMap<String, String>> {
    let file: ProvidersLocalFile = serde_json::from_str(content).ok()?;
    let mut defaults = HashMap::<String, String>::new();
    if let Some(provider) = file.active_provider() {
        defaults.insert("MUSETRANSLATE_LLM_PROVIDER".to_string(), provider);
    }
    for group in collect_provider_groups(file.providers) {
        apply_provider_group_defaults(&mut defaults, &group);
    }
    Some(defaults)
}

#[derive(Debug, Deserialize)]
struct ProvidersLocalFile {
    #[serde(default)]
    providers: Vec<ProvidersLocalEntry>,
    #[serde(default)]
    active_provider: Option<String>,
    #[serde(default)]
    llm_provider: Option<String>,
}

impl ProvidersLocalFile {
    fn active_provider(&self) -> Option<String> {
        self.llm_provider
            .as_deref()
            .or(self.active_provider.as_deref())
            .map(normalize_provider_type)
            .filter(|provider| !provider.is_empty())
    }
}

// ---- 模型策略配置：models.local.json（外置 per-model 策略，OpenAI compatible 通用默认）----

/// 单个模型的请求策略。所有字段可选，缺省时回落 OpenAI compatible 通用默认。
/// 这是把原先硬编码在 llm/policy.rs 的模型特例外置，支持自定义扩展与号池构建。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelPolicyConfig {
    /// 模型名（精确匹配，如 "google/diffusiongemma-26b-a4b-it"）。
    pub model: String,
    /// 是否发送 response_format: json_object。缺省 true（大多数 OpenAI compatible 支持）。
    #[serde(default)]
    pub json_response_format: Option<bool>,
    /// 是否启用 thinking（chat_template_kwargs.enable_thinking）。缺省 false。
    #[serde(default)]
    pub enable_thinking: Option<bool>,
    /// thinking 预算（reasoning_budget）。缺省 None。
    #[serde(default)]
    pub reasoning_budget: Option<u32>,
    /// 显式 max_tokens。缺省 None（不发送）。
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// temperature。缺省 None（不发送，用端点默认）。
    #[serde(default)]
    pub temperature: Option<f32>,
    /// top_p。缺省 None。
    #[serde(default)]
    pub top_p: Option<f32>,
    /// stream。缺省 None（false，非流式）。
    #[serde(default)]
    pub stream: Option<bool>,
    /// 主翻译 reasoning_effort（none|low|medium|high）。缺省 None。
    #[serde(default)]
    pub translation_reasoning_effort: Option<String>,
    /// instruction/review reasoning_effort。缺省 None。
    #[serde(default)]
    pub instruction_reasoning_effort: Option<String>,
}

/// models.local.json 顶层结构。
#[derive(Debug, Default, serde::Deserialize)]
struct ModelsLocalFile {
    #[serde(default)]
    models: Vec<ModelPolicyConfig>,
}

fn models_local_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("models.local.json"));
    if let Some(parent) = manifest_dir.parent() {
        candidates.push(parent.join("models.local.json"));
    }
    if let Some(grandparent) = manifest_dir.parent().and_then(Path::parent) {
        candidates.push(grandparent.join("models.local.json"));
    }
    candidates
}

/// 从磁盘加载模型策略配置（src-tauri → app 根 → 仓库根）。无文件返回空表（全部走通用默认）。
pub fn load_model_policies() -> Vec<ModelPolicyConfig> {
    models_local_candidates()
        .into_iter()
        .find_map(|path| load_model_policies_from_path(&path))
        .unwrap_or_default()
}

fn load_model_policies_from_path(path: &Path) -> Option<Vec<ModelPolicyConfig>> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let file: ModelsLocalFile = serde_json::from_str(&content).ok()?;
    Some(
        file.models
            .into_iter()
            .filter(|policy| !policy.model.trim().is_empty())
            .collect(),
    )
}

// ---- 号池（Route Pool）配置：pools.local.json（L2）----

/// 号池内一条供应商路由（含多 key）。key 只存于本地 pools.local.json，不入库、不打印。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolRouteConfig {
    pub provider: String,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub invoke_url: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_pool_route_weight")]
    pub weight: f64,
}

fn default_pool_route_weight() -> f64 {
    1.0
}

impl PoolRouteConfig {
    /// 归一化 base URL（兼容 invoke_url/base_url/api_url 三种写法），不含 /chat/completions。
    pub fn resolved_api_url(&self) -> String {
        self.api_url
            .as_deref()
            .or(self.invoke_url.as_deref())
            .or(self.base_url.as_deref())
            .unwrap_or("")
            .trim()
            .to_string()
    }

    /// 归一化 key 列表：api_keys 优先，单 api_key 次之；空字符串剔除。
    pub fn resolved_api_keys(&self) -> Vec<String> {
        let mut keys = self
            .api_keys
            .iter()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect::<Vec<_>>();
        if keys.is_empty() {
            if let Some(key) = self.api_key.as_deref() {
                let key = key.trim();
                if !key.is_empty() {
                    keys.push(key.to_string());
                }
            }
        }
        keys
    }

    pub fn is_usable(&self) -> bool {
        !self.provider.trim().is_empty()
            && !self.model.trim().is_empty()
            && !self.resolved_api_url().is_empty()
            && !self.resolved_api_keys().is_empty()
    }
}

/// 一套可保存的号池组合。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoutePoolConfig {
    pub name: String,
    #[serde(default)]
    pub routes: Vec<PoolRouteConfig>,
}

/// pools.local.json 顶层结构。
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct PoolsLocalFile {
    #[serde(default)]
    pub active_pool: Option<String>,
    #[serde(default)]
    pub pools: Vec<RoutePoolConfig>,
}

/// 供命令层使用的 pools.local.json 磁盘位置（第一个候选位）。
pub fn pools_local_primary_path() -> PathBuf {
    pools_local_candidates()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("pools.local.json"))
}

/// 从磁盘加载号池组合（src-tauri → app 根 → 仓库根）。无文件或无有效池时返回 None，
/// 调用方回落到现有单 provider 行为（完全向后兼容）。
pub fn load_route_pools() -> Option<(String, Vec<RoutePoolConfig>)> {
    pools_local_candidates()
        .into_iter()
        .find_map(|path| load_route_pools_from_path(&path))
}

fn load_route_pools_from_path(path: &Path) -> Option<(String, Vec<RoutePoolConfig>)> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    parse_route_pools_from_str(&content)
}

fn parse_route_pools_from_str(content: &str) -> Option<(String, Vec<RoutePoolConfig>)> {
    let file: PoolsLocalFile = serde_json::from_str(content).ok()?;
    let pools = file
        .pools
        .into_iter()
        .map(|mut pool| {
            pool.routes.retain(|route| route.is_usable());
            pool
        })
        .filter(|pool| !pool.name.trim().is_empty() && !pool.routes.is_empty())
        .collect::<Vec<_>>();
    if pools.is_empty() {
        return None;
    }
    let active = file
        .active_pool
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| pools.iter().any(|pool| pool.name == *name))
        .unwrap_or(&pools[0].name)
        .to_string();
    Some((active, pools))
}

#[derive(Debug, Deserialize)]
struct ProvidersLocalEntry {
    #[serde(rename = "type", default)]
    provider_type: String,
    #[serde(default)]
    invoke_url: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Default)]
struct ProviderDefaultsGroup {
    provider_type: String,
    api_keys: Vec<String>,
    api_url: Option<String>,
    model: Option<String>,
}

fn collect_provider_groups(entries: Vec<ProvidersLocalEntry>) -> Vec<ProviderDefaultsGroup> {
    let mut groups = Vec::<ProviderDefaultsGroup>::new();
    for entry in entries {
        let provider_type = normalize_provider_type(&entry.provider_type);
        if provider_type.is_empty() {
            continue;
        }
        let group = match groups
            .iter_mut()
            .find(|group| group.provider_type == provider_type)
        {
            Some(group) => group,
            None => {
                groups.push(ProviderDefaultsGroup {
                    provider_type,
                    ..ProviderDefaultsGroup::default()
                });
                groups.last_mut().expect("group was just inserted")
            }
        };
        push_non_empty(&mut group.api_keys, entry.api_key.as_deref());
        fill_first_non_empty(&mut group.api_url, entry.invoke_url.as_deref());
        fill_first_non_empty(&mut group.api_url, entry.base_url.as_deref());
        fill_first_non_empty(&mut group.model, entry.model.as_deref());
    }
    groups
}

fn apply_provider_group_defaults(
    defaults: &mut HashMap<String, String>,
    group: &ProviderDefaultsGroup,
) {
    let Some(keys) = provider_env_keys(&group.provider_type) else {
        return;
    };
    if !group.api_keys.is_empty() {
        defaults.insert(keys.api_key.to_string(), group.api_keys.join(","));
    }
    if let Some(api_url) = group.api_url.as_ref() {
        defaults.insert(keys.base_url.to_string(), api_url.clone());
    }
    if let Some(model) = group.model.as_ref() {
        defaults.insert(keys.model.to_string(), model.clone());
    }
}

struct ProviderEnvKeys {
    api_key: &'static str,
    base_url: &'static str,
    model: &'static str,
}

fn provider_env_keys(provider_type: &str) -> Option<ProviderEnvKeys> {
    match provider_type {
        "sensenova" => Some(ProviderEnvKeys {
            api_key: "SENSENOVA_API_KEY",
            base_url: "SENSENOVA_BASE_URL",
            model: "SENSENOVA_MODEL",
        }),
        "minimax" => Some(ProviderEnvKeys {
            api_key: "MINIMAX_API_KEY",
            base_url: "MINIMAX_BASE_URL",
            model: "MINIMAX_MODEL",
        }),
        "cohere" => Some(ProviderEnvKeys {
            api_key: "COHERE_API_KEY",
            base_url: "COHERE_BASE_URL",
            model: "COHERE_MODEL",
        }),
        "nvidia" => Some(ProviderEnvKeys {
            api_key: "NVIDIA_API_KEY",
            base_url: "NVIDIA_BASE_URL",
            model: "NVIDIA_MODEL",
        }),
        _ => None,
    }
}

fn normalize_provider_type(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn push_non_empty(values: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    values.push(value.to_string());
}

fn fill_first_non_empty(slot: &mut Option<String>, value: Option<&str>) {
    if slot.is_some() {
        return;
    }
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    *slot = Some(value.to_string());
}

fn parse_assignment_line(raw_line: &str) -> Option<(String, String)> {
    let line = raw_line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let delimiter = first_assignment_delimiter(line)?;
    let (key, value) = line.split_at(delimiter);
    let normalized_key = key.trim();
    if normalized_key.is_empty() {
        return None;
    }
    Some((
        normalized_key.to_string(),
        normalize_assignment_value(value.get(1..).unwrap_or_default()),
    ))
}

fn first_assignment_delimiter(line: &str) -> Option<usize> {
    match (line.find('='), line.find(':')) {
        (Some(eq), Some(colon)) => Some(eq.min(colon)),
        (Some(eq), None) => Some(eq),
        (None, Some(colon)) => Some(colon),
        (None, None) => None,
    }
}

fn normalize_assignment_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_local_defaults_merge_multiple_nvidia_keys() {
        let content = r#"{
            "llm_provider": "nvidia",
            "providers": [
                {
                    "type": "nvidia",
                    "invoke_url": "https://integrate.api.nvidia.com/v1/chat/completions",
                    "api_key": "key-a",
                    "model": "stepfun-ai/step-3.7-flash"
                },
                {
                    "type": "nvidia",
                    "invoke_url": "https://integrate.api.nvidia.com/v1/chat/completions",
                    "api_key": "key-b",
                    "model": "stepfun-ai/step-3.7-flash"
                }
            ]
        }"#;

        let defaults = provider_local_defaults_from_str(content)
            .expect("provider local defaults should parse");

        assert_eq!(
            defaults
                .get("MUSETRANSLATE_LLM_PROVIDER")
                .map(String::as_str),
            Some("nvidia")
        );
        assert_eq!(
            defaults.get("NVIDIA_API_KEY").map(String::as_str),
            Some("key-a,key-b")
        );
        assert_eq!(
            defaults.get("NVIDIA_BASE_URL").map(String::as_str),
            Some("https://integrate.api.nvidia.com/v1/chat/completions")
        );
        assert_eq!(
            defaults.get("NVIDIA_MODEL").map(String::as_str),
            Some("stepfun-ai/step-3.7-flash")
        );
    }

    #[test]
    fn provider_local_defaults_do_not_infer_active_provider() {
        let content = r#"{
            "providers": [
                { "type": "nvidia", "api_key": "key-a", "model": "model-a" }
            ]
        }"#;

        let defaults = provider_local_defaults_from_str(content)
            .expect("provider local defaults should parse");

        assert!(defaults.get("MUSETRANSLATE_LLM_PROVIDER").is_none());
        assert_eq!(
            defaults.get("NVIDIA_API_KEY").map(String::as_str),
            Some("key-a")
        );
        assert_eq!(
            defaults.get("NVIDIA_MODEL").map(String::as_str),
            Some("model-a")
        );
    }

    #[test]
    fn route_pools_parse_multi_provider_with_multi_keys() {
        let content = r#"{
            "active_pool": "default",
            "pools": [
                {
                    "name": "default",
                    "routes": [
                        {"provider":"sensenova","api_url":"https://token.sensenova.cn/v1/chat/completions","model":"deepseek-v4-flash","api_keys":["k1","k2"]},
                        {"provider":"nvidia","invoke_url":"https://integrate.api.nvidia.com/v1/chat/completions","model":"thinkingmachines/inkling","api_keys":["k3","k4"],"weight":1.0}
                    ]
                },
                {"name":"nvidia-only","routes":[{"provider":"nvidia","base_url":"https://integrate.api.nvidia.com/v1","model":"thinkingmachines/inkling","api_key":"k3"}]}
            ]
        }"#;

        let (active, pools) =
            parse_route_pools_from_str(content).expect("route pools should parse");

        assert_eq!(active, "default");
        assert_eq!(pools.len(), 2);
        let default_pool = &pools[0];
        assert_eq!(default_pool.name, "default");
        assert_eq!(default_pool.routes.len(), 2);
        assert_eq!(
            default_pool.routes[0].resolved_api_keys(),
            vec!["k1".to_string(), "k2".to_string()]
        );
        assert_eq!(
            default_pool.routes[1].resolved_api_url(),
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
        // 单 api_key 写法也应归一化为 key 列表
        assert_eq!(
            pools[1].routes[0].resolved_api_keys(),
            vec!["k3".to_string()]
        );
    }

    #[test]
    fn route_pools_falls_back_to_first_pool_when_active_missing() {
        let content = r#"{
            "pools": [
                {"name":"alpha","routes":[{"provider":"nvidia","api_url":"https://x","model":"m","api_key":"k"}]},
                {"name":"beta","routes":[{"provider":"sensenova","api_url":"https://y","model":"n","api_key":"k"}]}
            ]
        }"#;

        let (active, pools) =
            parse_route_pools_from_str(content).expect("route pools should parse");
        assert_eq!(active, "alpha");
        assert_eq!(pools.len(), 2);
    }

    #[test]
    fn route_pools_drops_unusable_routes_and_empty_pools() {
        let content = r#"{
            "pools": [
                {"name":"broken","routes":[{"provider":"nvidia","api_url":"","model":"","api_keys":[]}]},
                {"name":"ok","routes":[{"provider":"nvidia","api_url":"https://x","model":"m","api_key":"k"}]}
            ]
        }"#;

        let (active, pools) =
            parse_route_pools_from_str(content).expect("route pools should parse");
        // broken 池无可用路由被剔除，active 落到 ok
        assert_eq!(pools.len(), 1);
        assert_eq!(active, "ok");
    }

    #[test]
    fn route_pools_returns_none_when_no_valid_pool() {
        let content = r#"{"pools": []}"#;
        assert!(parse_route_pools_from_str(content).is_none());
        let invalid = r#"{"pools": [{"name":"x","routes":[]}]}"#;
        assert!(parse_route_pools_from_str(invalid).is_none());
    }
}
