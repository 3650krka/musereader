use super::{load_single_json, lock_mutex, persist_json, AppError, AppState};
use crate::skill_library::ensure_seeded as ensure_skill_library_seeded;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod env;

pub use env::PoolRouteConfig;
pub use env::{
    load_model_policies, load_route_pools, load_runtime_env_sources, resolve_env_defaults,
    EnvDefaults, ModelPolicyConfig, RoutePoolConfig,
};

const DEFAULT_SENSENOVA_BASE_URL: &str = "https://token.sensenova.cn/v1";
const DEFAULT_MINIMAX_BASE_URL: &str = "https://api.minimaxi.com/v1";
const DEFAULT_COHERE_BASE_URL: &str = "https://api.cohere.ai/compatibility/v1";
const DEFAULT_NVIDIA_BASE_URL: &str = "https://integrate.api.nvidia.com/v1";
const DEFAULT_NVIDIA_MODEL: &str = "stepfun-ai/step-3.7-flash";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub ocr_api_url: String,
    pub ocr_token: String,
    pub llm_provider: String,
    pub llm_api_url: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub sensenova_api_url: String,
    pub sensenova_api_key: String,
    pub sensenova_model: String,
    pub minimax_api_url: String,
    pub minimax_api_key: String,
    pub minimax_model: String,
    pub cohere_api_url: String,
    pub cohere_api_key: String,
    pub cohere_model: String,
    pub nvidia_api_url: String,
    pub nvidia_api_key: String,
    pub nvidia_model: String,
    #[serde(default = "default_true")]
    pub use_ocr_cache: bool,
    #[serde(default = "default_true")]
    pub use_front_matter_cache: bool,
    #[serde(default)]
    pub static_glossary_entries: Vec<StaticGlossaryEntry>,
    /// 翻译 prompt 注入策略：是否注入核心翻译 skill（全景翻译 skill）
    #[serde(default = "default_true")]
    pub inject_core_skill: bool,
    /// 翻译分块尺寸（字符数）：None 用默认 3000；任务启动时钳制到 [500, 20000]。
    #[serde(default)]
    pub chunk_size: Option<usize>,
    /// 号池组合（L2）。来自 pools.local.json；为空时回落单 provider 行为（向后兼容）。
    #[serde(default)]
    pub route_pools: Vec<RoutePoolConfig>,
    /// 当前活跃号池名；空时取 route_pools 第一个或回落单 provider。
    #[serde(default)]
    pub active_pool: String,
    /// 用户自添加的 OpenAI 兼容供应商（CherryStudio 式管理，左侧列表右侧详情）
    #[serde(default)]
    pub custom_providers: Vec<CustomProvider>,
    /// 内置供应商扩展。
    /// 以 provider id 为键（sensenova/minimax/...），仅存 models/api_keys；
    /// 单值字段（如 sensenova_model）由前端在编辑时同步回写，后端解析逻辑零改动。
    #[serde(default)]
    pub provider_extras: Vec<CustomProvider>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            ocr_api_url: String::new(),
            ocr_token: String::new(),
            llm_provider: String::new(),
            llm_api_url: String::new(),
            llm_api_key: String::new(),
            llm_model: String::new(),
            sensenova_api_url: String::new(),
            sensenova_api_key: String::new(),
            sensenova_model: String::new(),
            minimax_api_url: String::new(),
            minimax_api_key: String::new(),
            minimax_model: String::new(),
            cohere_api_url: String::new(),
            cohere_api_key: String::new(),
            cohere_model: String::new(),
            nvidia_api_url: String::new(),
            nvidia_api_key: String::new(),
            nvidia_model: String::new(),
            use_ocr_cache: true,
            use_front_matter_cache: true,
            static_glossary_entries: Vec::new(),
            inject_core_skill: true,
            chunk_size: None,
            route_pools: Vec::new(),
            active_pool: String::new(),
            custom_providers: Vec::new(),
            provider_extras: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StaticGlossaryEntry {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub enforcement: String,
    #[serde(default)]
    pub notes: String,
}

/// 用户自定义供应商：URL+Key 清单（每 key 可独立启停）+模型清单+协议。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub api_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<String>,
    /// API 协议：openai | openai-compatible | openai-responses | anthropic-messages

    #[serde(default = "default_protocol")]
    pub protocol: String,
    /// 多 key：逐个启停；与 api_key 并存，解析时合并（api_keys 优先）。
    #[serde(default)]
    pub api_keys: Vec<ProviderApiKey>,
}

/// 供应商 API key 条目（enabled=false 的 key 轮换时跳过）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderApiKey {
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_protocol() -> String {
    "openai-compatible".to_string()
}

pub struct RuntimePaths {
    pub task_store_path: PathBuf,
    pub reader_state_store_path: PathBuf,
    pub book_profile_store_path: PathBuf,
    pub runtime_config_store_path: PathBuf,
    pub import_root_dir: PathBuf,
    pub artifact_root_dir: PathBuf,
    pub skill_library_root_dir: PathBuf,
    /// 用户导入词包目录（词库与应用分发解耦，数据不内置）。
    pub wordlists_dir: PathBuf,
}

pub fn resolve_runtime_paths() -> RuntimePaths {
    let paths = RuntimePaths {
        task_store_path: resolve_runtime_file_path("tasks.json"),
        reader_state_store_path: resolve_runtime_file_path("reader_states.json"),
        book_profile_store_path: resolve_runtime_file_path("book_profiles.json"),
        runtime_config_store_path: resolve_runtime_file_path("runtime_config.json"),
        import_root_dir: resolve_runtime_dir_path("imports"),
        artifact_root_dir: resolve_runtime_dir_path("artifacts"),
        skill_library_root_dir: resolve_runtime_dir_path("skills"),
        wordlists_dir: resolve_runtime_dir_path("wordlists"),
    };
    let _ = ensure_skill_library_seeded(&paths.skill_library_root_dir);
    let _ = std::fs::create_dir_all(&paths.wordlists_dir);
    paths
}

pub fn load_runtime_config(path: &Path, defaults: EnvDefaults) -> RuntimeConfig {
    let mut config: RuntimeConfig = load_single_json(path).unwrap_or_default();
    fill_runtime_config_defaults(&mut config, defaults);
    fill_route_pool_defaults(&mut config);
    config
}

// 号池组合填充（L2）：runtime_config.json 未存号池时，从 pools.local.json 加载。
// 无 pools.local.json 或无有效池时保持空，运行时回落单 provider 行为（向后兼容）。
fn fill_route_pool_defaults(config: &mut RuntimeConfig) {
    if !config.route_pools.is_empty() {
        if config.active_pool.trim().is_empty() {
            config.active_pool = config.route_pools[0].name.clone();
        }
        return;
    }
    if let Some((active, pools)) = load_route_pools() {
        config.active_pool = if config.active_pool.trim().is_empty() {
            active
        } else {
            config.active_pool.clone()
        };
        config.route_pools = pools;
    }
}

fn fill_runtime_config_defaults(config: &mut RuntimeConfig, defaults: EnvDefaults) {
    if config.ocr_api_url.trim().is_empty() {
        config.ocr_api_url = default_ocr_api_url();
    }
    if config.ocr_token.trim().is_empty() {
        config.ocr_token = defaults.ocr_token;
    }
    if config.llm_provider.trim().is_empty() {
        config.llm_provider = default_llm_provider(defaults.llm_provider);
    }
    if config.sensenova_api_key.trim().is_empty() {
        config.sensenova_api_key = defaults.sensenova_api_key.clone();
    }
    if config.sensenova_model.trim().is_empty() {
        config.sensenova_model = default_sensenova_model(defaults.sensenova_model.clone());
    }
    if config.sensenova_api_url.trim().is_empty() {
        config.sensenova_api_url = normalize_provider_api_url(
            defaults.sensenova_base_url.clone(),
            DEFAULT_SENSENOVA_BASE_URL,
        );
    }
    if config.minimax_api_key.trim().is_empty() {
        config.minimax_api_key = defaults.minimax_api_key;
    }
    if config.minimax_model.trim().is_empty() {
        config.minimax_model = default_minimax_model(defaults.minimax_model);
    }
    if config.minimax_api_url.trim().is_empty() {
        config.minimax_api_url =
            normalize_provider_api_url(defaults.minimax_base_url, DEFAULT_MINIMAX_BASE_URL);
    }
    if config.cohere_api_key.trim().is_empty() {
        config.cohere_api_key = defaults.cohere_api_key;
    }
    if config.cohere_model.trim().is_empty() {
        config.cohere_model = default_cohere_model(defaults.cohere_model);
    }
    if config.cohere_api_url.trim().is_empty() {
        config.cohere_api_url =
            normalize_provider_api_url(defaults.cohere_base_url, DEFAULT_COHERE_BASE_URL);
    }
    if config.nvidia_api_key.trim().is_empty() {
        config.nvidia_api_key = defaults.nvidia_api_key;
    }
    if config.nvidia_model.trim().is_empty() {
        config.nvidia_model = default_nvidia_model(defaults.nvidia_model);
    }
    if config.nvidia_api_url.trim().is_empty() {
        config.nvidia_api_url =
            normalize_provider_api_url(defaults.nvidia_base_url, DEFAULT_NVIDIA_BASE_URL);
    }
    if config.llm_api_key.trim().is_empty() {
        config.llm_api_key = config.sensenova_api_key.clone();
    }
    if config.llm_model.trim().is_empty() {
        config.llm_model = config.sensenova_model.clone();
    }
    if config.llm_api_url.trim().is_empty() {
        config.llm_api_url = config.sensenova_api_url.clone();
    }
    synchronize_active_llm_config(config);
}

fn default_true() -> bool {
    true
}

fn default_ocr_api_url() -> String {
    "https://x977a9m8t78er7u3.aistudio-app.com/layout-parsing".to_string()
}

fn default_llm_provider(env_provider: String) -> String {
    let provider = env_provider.trim();
    if provider.is_empty() {
        "sensenova".to_string()
    } else {
        provider.to_ascii_lowercase()
    }
}

fn default_sensenova_model(env_model: String) -> String {
    if env_model.is_empty() {
        "deepseek-v4-flash".to_string()
    } else {
        env_model
    }
}

fn default_minimax_model(env_model: String) -> String {
    if env_model.is_empty() {
        "MiniMax-M2.7".to_string()
    } else {
        env_model
    }
}

fn default_cohere_model(env_model: String) -> String {
    if env_model.is_empty() {
        "command-a-plus-05-2026".to_string()
    } else {
        env_model
    }
}

fn default_nvidia_model(env_model: String) -> String {
    if env_model.is_empty() {
        DEFAULT_NVIDIA_MODEL.to_string()
    } else {
        env_model
    }
}

fn normalize_provider_api_url(base_or_full: String, default_base_or_full: &str) -> String {
    if base_or_full.trim().is_empty() {
        normalize_llm_api_url(default_base_or_full.to_string())
    } else {
        normalize_llm_api_url(base_or_full)
    }
}

pub fn normalize_llm_api_url(base_or_full: String) -> String {
    if base_or_full.trim().is_empty() {
        "https://token.sensenova.cn/v1/chat/completions".to_string()
    } else if base_or_full.ends_with("/chat/completions") {
        base_or_full
    } else {
        format!("{}/chat/completions", base_or_full.trim_end_matches('/'))
    }
}

fn synchronize_active_llm_config(config: &mut RuntimeConfig) {
    match config.llm_provider.as_str() {
        "sensenova" => {
            config.llm_api_url = normalize_llm_api_url(config.sensenova_api_url.clone());
            config.llm_api_key = config.sensenova_api_key.clone();
            config.llm_model = config.sensenova_model.clone();
        }
        "minimax" => {
            config.llm_api_url = normalize_llm_api_url(config.minimax_api_url.clone());
            config.llm_api_key = config.minimax_api_key.clone();
            config.llm_model = config.minimax_model.clone();
        }
        "cohere" => {
            config.llm_api_url = normalize_llm_api_url(config.cohere_api_url.clone());
            config.llm_api_key = config.cohere_api_key.clone();
            config.llm_model = config.cohere_model.clone();
        }
        "nvidia" => {
            config.llm_api_url = normalize_llm_api_url(config.nvidia_api_url.clone());
            config.llm_api_key = config.nvidia_api_key.clone();
            config.llm_model = config.nvidia_model.clone();
        }
        _ => {
            config.llm_api_url = normalize_llm_api_url(config.llm_api_url.clone());
        }
    }
}

fn resolve_runtime_file_path(filename: &str) -> PathBuf {
    resolve_runtime_dir_path("").join(filename)
}

fn resolve_runtime_dir_path(subdir: &str) -> PathBuf {
    let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    if !subdir.is_empty() {
        base = base.join(subdir);
    }
    base
}

fn persist_runtime_config(state: &AppState, config: &RuntimeConfig) -> Result<(), AppError> {
    persist_json(&state.runtime_config_store_path, config)
}

pub(crate) fn replace_runtime_config(state: &AppState, config: RuntimeConfig) -> Result<(), AppError> {
    let mut current = lock_mutex(&state.config, "运行配置")?;
    let previous = current.clone();
    let mut next = config;
    // 分块尺寸钳制到安全区间（见 commands/tasks.rs）：非法值不报错，夹回边界即可。
    next.chunk_size = next.chunk_size.map(super::tasks::clamp_chunk_size);
    next.sensenova_api_url =
        normalize_provider_api_url(next.sensenova_api_url, DEFAULT_SENSENOVA_BASE_URL);
    next.minimax_api_url =
        normalize_provider_api_url(next.minimax_api_url, DEFAULT_MINIMAX_BASE_URL);
    next.cohere_api_url = normalize_provider_api_url(next.cohere_api_url, DEFAULT_COHERE_BASE_URL);
    next.nvidia_api_url = normalize_provider_api_url(next.nvidia_api_url, DEFAULT_NVIDIA_BASE_URL);
    if next.nvidia_model.trim().is_empty() {
        next.nvidia_model = default_nvidia_model(String::new());
    }
    next.llm_api_url = normalize_llm_api_url(next.llm_api_url);
    synchronize_active_llm_config(&mut next);
    *current = next;

    if let Err(error) = persist_runtime_config(state, &current) {
        *current = previous;
        return Err(error);
    }

    Ok(())
}

#[tauri::command]
pub async fn get_runtime_config(
    state: tauri::State<'_, AppState>,
) -> Result<RuntimeConfig, AppError> {
    Ok(lock_mutex(&state.config, "运行配置")?.clone())
}

#[tauri::command]
pub async fn update_runtime_config(
    config: RuntimeConfig,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    replace_runtime_config(&state, config)
}

/* ---- 号池持久化（pools.local.json 的读写命令） ---- */

/// 号池视图（前端展示用）：保留 key 数量与掩码，不回传明文 key。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePoolView {
    pub name: String,
    pub routes: Vec<PoolRouteView>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolRouteView {
    pub provider: String,
    pub api_url: String,
    pub model: String,
    pub weight: f64,
    pub key_count: usize,
    pub key_masks: Vec<String>,
}

fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.chars().count() <= 4 {
        return "…".to_string() + trimmed;
    }
    let tail: String = trimmed.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…{}", tail)
}

#[tauri::command]
pub async fn list_route_pools() -> Result<Vec<RoutePoolView>, AppError> {
    let Some((active, pools)) = load_route_pools() else {
        return Ok(Vec::new());
    };
    Ok(pools
        .into_iter()
        .map(|pool| RoutePoolView {
            active: pool.name == active,
            name: pool.name,
            routes: pool
                .routes
                .into_iter()
                .map(|route| {
                    let keys = route.resolved_api_keys();
                    PoolRouteView {
                        provider: route.provider.clone(),
                        api_url: route.resolved_api_url(),
                        model: route.model.clone(),
                        weight: route.weight,
                        key_count: keys.len(),
                        key_masks: keys.iter().map(|k| mask_key(k)).collect(),
                    }
                })
                .collect(),
        })
        .collect())
}

/// 号池保存入参：routes 传完整 key 列表（编辑会话内持有明文）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePoolInput {
    pub name: String,
    pub routes: Vec<PoolRouteInput>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolRouteInput {
    pub provider: String,
    #[serde(default)]
    pub api_url: String,
    pub model: String,
    #[serde(default = "default_input_weight")]
    pub weight: f64,
    #[serde(default)]
    pub api_keys: Vec<String>,
}

fn default_input_weight() -> f64 {
    1.0
}

#[tauri::command]
pub async fn save_route_pools(pools: Vec<RoutePoolInput>) -> Result<(), AppError> {
    let active = pools
        .iter()
        .find(|p| p.active)
        .map(|p| p.name.clone())
        .or_else(|| pools.first().map(|p| p.name.clone()));
    // 密钥保留：前端面板不回显密钥（apiKeys 恒空），空数组视为「未改动」，
    // 从磁盘 pools.local.json 回填原值，防止无密钥 UI 覆写清空密钥。
    let existing_keys: std::collections::HashMap<(String, String), Vec<String>> =
        env::load_route_pools()
            .map(|(_, pools)| {
                pools
                    .into_iter()
                    .flat_map(|pool| {
                        pool.routes
                            .into_iter()
                            .map(move |route| {
                                ((pool.name.clone(), route.model.clone()), route.resolved_api_keys())
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
            .unwrap_or_default();
    let file = env::PoolsLocalFile {
        active_pool: active,
        pools: pools
            .into_iter()
            .map(|pool| {
                let pool_name_for_lookup = pool.name.clone();
                RoutePoolConfig {
                name: pool.name,
                routes: pool
                    .routes
                    .into_iter()
                    .map(|route| PoolRouteConfig {
                        provider: route.provider,
                        api_url: if route.api_url.is_empty() {
                            None
                        } else {
                            Some(route.api_url)
                        },
                        invoke_url: None,
                        base_url: None,
                        model: route.model.clone(),
                        api_keys: if route.api_keys.is_empty() {
                            existing_keys
                                .get(&(pool_name_for_lookup.clone(), route.model.clone()))
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            route.api_keys
                        },
                        api_key: None,
                        weight: route.weight,
                    })
                    .collect(),
                }
            })
            .collect(),
    };
    let path = env::pools_local_primary_path();
    let payload = serde_json::to_string_pretty(&file).map_err(|error| AppError {
        code: "pools_serialize_failed".into(),
        message: format!("号池序列化失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    // 原子写：tmp + rename，避免半截文件损坏号池。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, payload).map_err(|error| AppError {
        code: "pools_write_failed".into(),
        message: format!("号池写入失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    std::fs::rename(&tmp, &path).map_err(|error| AppError {
        code: "pools_write_failed".into(),
        message: format!("号池落盘失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    Ok(())
}

/// 号池路由 key 即时编辑：前端不持有明文 key，
/// 增删走专用命令直改 pools.local.json（保存全表会因空 apiKeys 语义覆盖丢失）。
#[tauri::command]
pub async fn pool_route_add_key(
    pool_name: String,
    model: String,
    key: String,
) -> Result<Vec<RoutePoolView>, AppError> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(AppError::invalid_input("key 为空"));
    }
    mutate_pools_local(|pool| {
        if pool.name != pool_name {
            return false;
        }
        for route in pool.routes.iter_mut() {
            if route.model == model {
                if !route.resolved_api_keys().iter().any(|k| *k == key) {
                    route.api_keys.push(key.clone());
                }
                return true;
            }
        }
        false
    })
}

#[tauri::command]
pub async fn pool_route_remove_key(
    pool_name: String,
    model: String,
    key_index: usize,
) -> Result<Vec<RoutePoolView>, AppError> {
    mutate_pools_local(|pool| {
        if pool.name != pool_name {
            return false;
        }
        for route in pool.routes.iter_mut() {
            if route.model == model {
                let mut keys = route.resolved_api_keys();
                if key_index < keys.len() {
                    keys.remove(key_index);
                    route.api_keys = keys;
                    route.api_key = None;
                }
                return true;
            }
        }
        false
    })
}

/// 读-改-写 pools.local.json；未命中路由返回错误，成功后回传最新视图。
fn mutate_pools_local(
    mutate: impl Fn(&mut RoutePoolConfig) -> bool,
) -> Result<Vec<RoutePoolView>, AppError> {
    let Some((active, mut pools)) = env::load_route_pools() else {
        return Err(AppError::invalid_input("pools.local.json 不存在或无有效池"));
    };
    let mut hit = false;
    for pool in pools.iter_mut() {
        if mutate(pool) {
            hit = true;
            break;
        }
    }
    if !hit {
        return Err(AppError::invalid_input("未找到对应号池路由"));
    }
    let file = env::PoolsLocalFile {
        active_pool: pools.iter().find(|p| p.name == active).map(|p| p.name.clone()),
        pools: pools.clone(),
    };
    let payload = serde_json::to_string_pretty(&file).map_err(|error| AppError {
        code: "pools_serialize_failed".into(),
        message: format!("号池序列化失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    let path = env::pools_local_primary_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, payload).map_err(|error| AppError {
        code: "pools_write_failed".into(),
        message: format!("号池写入失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    std::fs::rename(&tmp, &path).map_err(|error| AppError {
        code: "pools_write_failed".into(),
        message: format!("号池落盘失败：{error}"),
        retryable: false,
        retry_after_ms: None,
    })?;
    Ok(pools
        .into_iter()
        .map(|pool| RoutePoolView {
            active: pool.name == active,
            name: pool.name,
            routes: pool
                .routes
                .into_iter()
                .map(|route| {
                    let keys = route.resolved_api_keys();
                    PoolRouteView {
                        provider: route.provider.clone(),
                        api_url: route.resolved_api_url(),
                        model: route.model.clone(),
                        weight: route.weight,
                        key_count: keys.len(),
                        key_masks: keys.iter().map(|k| mask_key(k)).collect(),
                    }
                })
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{ReaderState, TranslationTask};
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Semaphore;

    fn sample_runtime_config() -> RuntimeConfig {
        RuntimeConfig {
            ocr_api_url: "https://ocr.example.com".to_string(),
            ocr_token: "ocr-token".to_string(),
            llm_provider: "sensenova".to_string(),
            llm_api_url: "https://llm.example.com/v1/chat/completions".to_string(),
            llm_api_key: "llm-key".to_string(),
            llm_model: "model-a".to_string(),
            sensenova_api_url: "https://llm.example.com/v1/chat/completions".to_string(),
            sensenova_api_key: "llm-key".to_string(),
            sensenova_model: "model-a".to_string(),
            minimax_api_url: "https://api.minimaxi.com/v1/chat/completions".to_string(),
            minimax_api_key: "minimax-key".to_string(),
            minimax_model: "MiniMax-M2.7".to_string(),
            cohere_api_url: "https://api.cohere.ai/compatibility/v1/chat/completions".to_string(),
            cohere_api_key: "cohere-key".to_string(),
            cohere_model: "command-a-plus-05-2026".to_string(),
            nvidia_api_url: "https://integrate.api.nvidia.com/v1/chat/completions".to_string(),
            nvidia_api_key: "nvidia-key".to_string(),
            nvidia_model: "stepfun-ai/step-3.7-flash".to_string(),
            use_ocr_cache: true,
            use_front_matter_cache: true,
            static_glossary_entries: Vec::new(),
            inject_core_skill: true,
            chunk_size: None,
            route_pools: Vec::new(),
            active_pool: String::new(),
            custom_providers: Vec::new(),
            provider_extras: Vec::new(),
        }
    }

    fn sample_state(root: &Path, config: RuntimeConfig) -> AppState {
        AppState {
            tasks: Arc::new(Mutex::new(HashMap::<String, TranslationTask>::new())),
            config: Arc::new(Mutex::new(config)),
            reader_states: Arc::new(Mutex::new(HashMap::<String, ReaderState>::new())),
            book_profiles: Arc::new(Mutex::new(HashMap::new())),
            task_store_path: root.join("tasks.json"),
            reader_state_store_path: root.join("reader_states.json"),
            book_profile_store_path: root.join("book_profiles.json"),
            runtime_config_store_path: root.join("runtime_config.json"),
            import_root_dir: root.join("imports"),
            artifact_root_dir: root.join("artifacts"),
            skill_library_root_dir: root.join("skills"),
            wordlists_dir: root.join("wordlists"),
            llm_limiter: Arc::new(Semaphore::new(1)),
            nonfatal_notices: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            controls: Arc::new(Mutex::new(HashMap::new())),
            task_event_sink: None,
        }
    }

    #[test]
    fn replace_runtime_config_restores_previous_value_when_persist_fails() {
        let unique = format!(
            "musetranslate-config-replace-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("temp dir should exist");
        let state = sample_state(&root, sample_runtime_config());
        fs::create_dir_all(&state.runtime_config_store_path)
            .expect("runtime config store path should become a directory");

        let error = replace_runtime_config(
            &state,
            RuntimeConfig {
                ocr_api_url: "https://ocr.changed.example.com".to_string(),
                ocr_token: "changed-ocr-token".to_string(),
                llm_provider: "minimax".to_string(),
                llm_api_url: "https://llm.changed.example.com".to_string(),
                llm_api_key: "changed-llm-key".to_string(),
                llm_model: "model-b".to_string(),
                sensenova_api_url: "https://sensenova.changed.example.com".to_string(),
                sensenova_api_key: "changed-sensenova-key".to_string(),
                sensenova_model: "deepseek-v4-flash".to_string(),
                minimax_api_url: "https://api.minimaxi.com/v1".to_string(),
                minimax_api_key: "changed-minimax-key".to_string(),
                minimax_model: "MiniMax-M2.7".to_string(),
                cohere_api_url: "https://api.cohere.ai/compatibility/v1".to_string(),
                cohere_api_key: "changed-cohere-key".to_string(),
                cohere_model: "command-a-plus-05-2026".to_string(),
                nvidia_api_url: "https://integrate.api.nvidia.com/v1".to_string(),
                nvidia_api_key: "changed-nvidia-key".to_string(),
                nvidia_model: "stepfun-ai/step-3.7-flash".to_string(),
                use_ocr_cache: false,
                use_front_matter_cache: false,
                static_glossary_entries: Vec::new(),
                inject_core_skill: true,
                chunk_size: None,
                route_pools: Vec::new(),
                active_pool: String::new(),
                custom_providers: Vec::new(),
                provider_extras: Vec::new(),
            },
        )
        .expect_err("persist failure should restore previous runtime config");

        assert_eq!(error.code, "INTERNAL");
        let current = state
            .config
            .lock()
            .expect("runtime config mutex should remain healthy");
        assert_eq!(current.ocr_api_url, "https://ocr.example.com");
        assert_eq!(current.ocr_token, "ocr-token");
        assert_eq!(
            current.llm_api_url,
            "https://llm.example.com/v1/chat/completions"
        );
        assert_eq!(current.llm_api_key, "llm-key");
        assert_eq!(current.llm_model, "model-a");
        assert_eq!(current.llm_provider, "sensenova");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fill_runtime_config_defaults_syncs_active_provider_to_minimax() {
        let mut config = RuntimeConfig {
            llm_provider: "minimax".to_string(),
            ..RuntimeConfig::default()
        };
        let defaults = EnvDefaults {
            ocr_token: String::new(),
            llm_provider: String::new(),
            sensenova_api_key: "sense-key".to_string(),
            sensenova_base_url: "https://token.sensenova.cn/v1".to_string(),
            sensenova_model: "deepseek-v4-flash".to_string(),
            minimax_api_key: "mini-key".to_string(),
            minimax_base_url: "https://api.minimaxi.com/v1".to_string(),
            minimax_model: "MiniMax-M2.7".to_string(),
            cohere_api_key: "cohere-key".to_string(),
            cohere_base_url: "https://api.cohere.ai/compatibility/v1".to_string(),
            cohere_model: "command-a-plus-05-2026".to_string(),
            nvidia_api_key: "nvidia-key".to_string(),
            nvidia_base_url: "https://integrate.api.nvidia.com/v1".to_string(),
            nvidia_model: "stepfun-ai/step-3.7-flash".to_string(),
        };

        fill_runtime_config_defaults(&mut config, defaults);

        assert_eq!(
            config.llm_api_url,
            "https://api.minimaxi.com/v1/chat/completions"
        );
        assert_eq!(config.llm_api_key, "mini-key");
        assert_eq!(config.llm_model, "MiniMax-M2.7");
        assert_eq!(
            config.sensenova_api_url,
            "https://token.sensenova.cn/v1/chat/completions"
        );
    }

    #[test]
    fn fill_runtime_config_defaults_syncs_active_provider_to_cohere() {
        let mut config = RuntimeConfig {
            llm_provider: "cohere".to_string(),
            ..RuntimeConfig::default()
        };
        let defaults = EnvDefaults {
            ocr_token: String::new(),
            llm_provider: String::new(),
            sensenova_api_key: String::new(),
            sensenova_base_url: String::new(),
            sensenova_model: String::new(),
            minimax_api_key: String::new(),
            minimax_base_url: String::new(),
            minimax_model: String::new(),
            cohere_api_key: "cohere-key".to_string(),
            cohere_base_url: "https://api.cohere.ai/compatibility/v1".to_string(),
            cohere_model: "command-a-plus-05-2026".to_string(),
            nvidia_api_key: String::new(),
            nvidia_base_url: String::new(),
            nvidia_model: String::new(),
        };

        fill_runtime_config_defaults(&mut config, defaults);

        assert_eq!(
            config.llm_api_url,
            "https://api.cohere.ai/compatibility/v1/chat/completions"
        );
        assert_eq!(config.llm_api_key, "cohere-key");
        assert_eq!(config.llm_model, "command-a-plus-05-2026");
    }

    #[test]
    fn fill_runtime_config_defaults_syncs_active_provider_to_nvidia() {
        let mut config = RuntimeConfig {
            llm_provider: "nvidia".to_string(),
            ..RuntimeConfig::default()
        };
        let defaults = EnvDefaults {
            ocr_token: String::new(),
            llm_provider: String::new(),
            sensenova_api_key: String::new(),
            sensenova_base_url: String::new(),
            sensenova_model: String::new(),
            minimax_api_key: String::new(),
            minimax_base_url: String::new(),
            minimax_model: String::new(),
            cohere_api_key: String::new(),
            cohere_base_url: String::new(),
            cohere_model: String::new(),
            nvidia_api_key: "nvidia-key".to_string(),
            nvidia_base_url: "https://integrate.api.nvidia.com/v1".to_string(),
            nvidia_model: String::new(),
        };

        fill_runtime_config_defaults(&mut config, defaults);

        assert_eq!(
            config.llm_api_url,
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
        assert_eq!(config.llm_api_key, "nvidia-key");
        assert_eq!(config.llm_model, "stepfun-ai/step-3.7-flash");
    }
}
