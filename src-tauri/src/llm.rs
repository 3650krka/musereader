use crate::error::{ProviderError, ProviderErrorKind};
pub(crate) use policy::ProviderRoutingKey;
use reqwest::header::{ACCEPT, RETRY_AFTER};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod policy;
mod prompt;
mod response;
mod structured;
mod types;

use policy::{
    fallback_model_for, resolve_llm_connect_timeout_seconds, resolve_llm_timeout_seconds,
    ProviderRequestPolicy, ProviderRequestPurpose,
};
use prompt::build_translation_user_prompt;
pub use response::{map_http_status, parse_retry_after};
use response::{map_reqwest_error, parse_llm_response_bytes};
use structured::parse_structured_translation;
pub use types::{
    ChatTemplateKwargs, ConfirmedTerm, LlmExtraBody, LlmRequest, LlmResponse, LlmUsageTotals,
    Message, ResponseFormat, StructuredTranslation,
};

// 号池调度画像提供者：由 pipeline 注入，避免 llm 反向依赖 pipeline。
// 输入 routing_key，输出该 route 的稳定吞吐容量与速度（用于 water-filling 分配）。
pub type RouteCapacityFn = Arc<dyn Fn(&ProviderRoutingKey) -> RouteCapacity + Send + Sync>;

// 单路由的稳定吞吐容量与速度（water-filling 分配的输入）。
#[derive(Debug, Clone, Copy)]
pub struct RouteCapacity {
    pub speed_rpm: f64,            // s_i = 60/p95_latency（单并发速度，req/min）
    pub capacity_rpm: f64,         // c_i = min(rpm, 实测吞吐) × health（稳定吞吐上限，req/min）
    pub has_learning: bool,        // 是否有学习数据（无则冷启动）
}

#[derive(Clone)]
pub struct SensenovaClient {
    api_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
    fallback_routes: Arc<Vec<LlmFallbackRoute>>,
    next_credential: Arc<AtomicUsize>,
    active_model: Arc<Mutex<String>>,
    quota_failures_before_fallback: Arc<Mutex<u8>>,
    usage_totals: Arc<Mutex<LlmUsageTotals>>,
    // 号池（L1）：非空时按 water-filling 最优分配在异构 route 间并行调度；空时走单 provider。
    pool_routes: Arc<Vec<PoolRoute>>,
    // 号池最优分配画像提供者：由 pipeline 注入（稳定吞吐容量 + 速度）。
    capacity_profile: Option<RouteCapacityFn>,
    // 冷启动回退容量（无学习数据的路由）：池内中位速度，用于新路由试探。
    cold_probe_capacity: f64,
    // 每条 route 本周期已分配的额度（water-filling 用满前优先取）。
    allocated: Arc<Mutex<Vec<f64>>>,
    // 冷启动探测游标：无画像路由按轮转获得保底探测流量，跑出学习数据后转入正常分配。
    cold_probe_cursor: Arc<AtomicUsize>,
    // 默认请求协议（用户在供应商面板选择；Auto=按 URL 启发式），见 RequestProtocol。
    default_protocol: RequestProtocol,
}

// 号池内一条路由（已展开多 key 为凭证，含权重）。
#[derive(Clone)]
struct PoolRoute {
    api_url: String,
    model: String,
    credentials: Vec<ProviderCredential>,
}

#[derive(Clone)]
struct ProviderCredential {
    api_url: String,
    api_key: String,
}

#[derive(Clone)]
pub struct LlmFallbackRoute {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

/// LLM 请求协议：Auto=按 URL 启发式；其余为用户在供应商面板显式选择。
/// openai / openai-compatible 同走 chat/completions；openai-responses 走 /responses；
/// anthropic-messages 走 /messages（x-api-key 头）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestProtocol {
    Auto,
    OpenaiCompatible,
    OpenaiResponses,
    AnthropicMessages,
}

impl RequestProtocol {
    /// 用户协议标签（CustomProvider.protocol）→ 枚举；未知标签回落 Auto（URL 启发式）。
    pub fn from_label(label: &str) -> Self {
        match label.trim() {
            "openai" | "openai-compatible" => Self::OpenaiCompatible,
            "openai-responses" => Self::OpenaiResponses,
            "anthropic-messages" => Self::AnthropicMessages,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone)]
pub struct ProviderRoute {
    api_url: String,
    api_key: String,
    routing_key: ProviderRoutingKey,
    protocol: RequestProtocol,
}

impl ProviderRoute {
    pub(crate) fn routing_key(&self) -> &ProviderRoutingKey {
        &self.routing_key
    }
}

impl SensenovaClient {
    #[cfg(test)]
    pub fn new(api_url: String, api_key: String, model: String) -> Self {
        Self::new_with_fallback_routes(api_url, api_key, model, Vec::new())
    }

    pub fn new_with_fallback_routes(
        api_url: String,
        api_key: String,
        model: String,
        fallback_routes: Vec<LlmFallbackRoute>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(resolve_llm_timeout_seconds()))
            .connect_timeout(Duration::from_secs(resolve_llm_connect_timeout_seconds()))
            // 部分 OpenAI compatible 端点（如 opencode.ai/zen）用 Cloudflare 拦截默认 UA（403 error code 1010），
            // 需要浏览器 UA 才能通过。这是端点侧防护，不影响其他供应商。
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let active_model = Arc::new(Mutex::new(model.clone()));
        let quota_failures_before_fallback = Arc::new(Mutex::new(0));
        let usage_totals = Arc::new(Mutex::new(LlmUsageTotals::default()));
        Self {
            api_url,
            api_key,
            model,
            client,
            fallback_routes: Arc::new(fallback_routes),
            next_credential: Arc::new(AtomicUsize::new(0)),
            active_model,
            quota_failures_before_fallback,
            usage_totals,
            pool_routes: Arc::new(Vec::new()),
            capacity_profile: None,
            cold_probe_capacity: 0.0,
            allocated: Arc::new(Mutex::new(Vec::new())),
            cold_probe_cursor: Arc::new(AtomicUsize::new(0)),
            default_protocol: RequestProtocol::Auto,
        }
    }

    /// 设置默认请求协议（链式）：由调用方从 RuntimeConfig 注入。
    pub fn with_protocol(mut self, protocol: RequestProtocol) -> Self {
        self.default_protocol = protocol;
        self
    }

    // 号池构造（L1）：由 RoutePoolConfig 构建跨供应商并行调度客户端。
    // 主 api_url/api_key/model 仍填（用于 fallback 语义与 trace），但调度走 pool_routes。
    pub fn new_with_route_pool(
        api_url: String,
        api_key: String,
        model: String,
        pool: &crate::commands::RoutePoolConfig,
    ) -> Self {
        let mut client = Self::new_with_fallback_routes(api_url, api_key, model, Vec::new());
        let pool_routes = pool
            .routes
            .iter()
            .filter(|route| route.is_usable())
            .map(|route| {
                let base = crate::commands::normalize_llm_api_url(route.resolved_api_url());
                let credentials = route
                    .resolved_api_keys()
                    .into_iter()
                    .map(|key| ProviderCredential {
                        api_url: base.clone(),
                        api_key: key,
                    })
                    .collect::<Vec<_>>();
                PoolRoute {
                    api_url: base,
                    model: route.model.trim().to_string(),
                    credentials,
                }
            })
            .collect::<Vec<_>>();
        client.pool_routes = Arc::new(pool_routes);
        client
    }

    // 注入号池最优分配画像提供者：由 pipeline 在启动时注入（稳定吞吐容量 + 速度）。
    pub fn with_capacity_profile(mut self, capacity_fn: RouteCapacityFn) -> Self {
        self.capacity_profile = Some(capacity_fn);
        self
    }

    // 注入冷启动回退容量：无学习数据的路由用池内中位速度试探。
    pub fn with_cold_probe_capacity(mut self, capacity: f64) -> Self {
        self.cold_probe_capacity = capacity;
        self
    }

    fn has_route_pool(&self) -> bool {
        !self.pool_routes.is_empty()
    }

    pub fn active_model(&self) -> String {
        self.active_model
            .lock()
            .map(|model| model.clone())
            .unwrap_or_else(|_| self.model.clone())
    }

    pub fn usage_totals(&self) -> LlmUsageTotals {
        self.usage_totals
            .lock()
            .map(|usage| *usage)
            .unwrap_or_default()
    }

    /// 从响应 JSON 提取并累计 usage（三协议统一入口）：
    /// 无 usage / 全 0 静默跳过，不虚增 response_count。
    fn record_usage_from_json(&self, value: &serde_json::Value) {
        if let Some(usage) = types::usage_from_json(value) {
            if let Ok(mut totals) = self.usage_totals.lock() {
                totals.add_usage(usage);
            }
        }
    }

    pub(crate) fn runtime_trace(&self) -> String {
        let active_model = self.active_model();
        let translation_policy =
            ProviderRequestPolicy::for_model(&active_model, ProviderRequestPurpose::Translation);
        let instruction_policy =
            ProviderRequestPolicy::for_model(&active_model, ProviderRequestPurpose::Instruction);
        let route_config = self.route_config_for_model(&active_model);
        let credentials = provider_credentials(&route_config.api_url, &route_config.api_key);
        let routes = credentials
            .iter()
            .map(|credential| {
                ProviderRoutingKey::new(&credential.api_url, &active_model, &credential.api_key)
            })
            .collect::<Vec<_>>();
        let provider = routes
            .first()
            .map(|route| route.provider.as_str())
            .unwrap_or("llm");
        let key_fingerprints = routes
            .iter()
            .map(|route| route.key_fingerprint.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let fallback_models = self
            .fallback_routes
            .iter()
            .map(|route| route.model.as_str())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "provider={provider}; model={}; configured_model={}; route_count={}; key_fingerprints={key_fingerprints}; fallback_models={fallback_models}; translation_json={}; instruction_json={}; translation_max_tokens={}; instruction_max_tokens={}; translation_temperature_override={}; instruction_temperature_override={}; translation_top_p={}; instruction_top_p={}; translation_stream={}; instruction_stream={}; translation_reasoning={}; instruction_reasoning={}; translation_extra_body={}; instruction_extra_body={}; timeout_seconds={}; connect_timeout_seconds={}",
            active_model,
            self.model,
            routes.len(),
            translation_policy.response_format.is_some(),
            instruction_policy.response_format.is_some(),
            optional_u32_text(translation_policy.max_tokens),
            optional_u32_text(instruction_policy.max_tokens),
            optional_f32_text(translation_policy.temperature),
            optional_f32_text(instruction_policy.temperature),
            optional_f32_text(translation_policy.top_p),
            optional_f32_text(instruction_policy.top_p),
            optional_bool_text(translation_policy.stream),
            optional_bool_text(instruction_policy.stream),
            optional_text(translation_policy.reasoning_effort.as_deref()),
            optional_text(instruction_policy.reasoning_effort.as_deref()),
            extra_body_summary(&translation_policy.extra_body),
            extra_body_summary(&instruction_policy.extra_body),
            resolve_llm_timeout_seconds(),
            resolve_llm_connect_timeout_seconds(),
        )
    }

    pub fn reserve_route(&self) -> ProviderRoute {
        // 号池调度（L1）：按权重 + 路由画像容量在异构 route 间选择；同 route 内多 key 轮转。
        // 效果无损：只改并发来源，不改术语回补时机（wave_flush_size 锁 configured）。
        if self.has_route_pool() {
            return self.reserve_pool_route();
        }
        let active_model = self.active_model();
        let route_config = self.route_config_for_model(&active_model);
        let credentials = provider_credentials(&route_config.api_url, &route_config.api_key);
        let index = self.next_credential.fetch_add(1, Ordering::Relaxed);
        let credential = credentials
            .get(index % credentials.len().max(1))
            .cloned()
            .unwrap_or_else(|| ProviderCredential {
                api_url: route_config.api_url.clone(),
                api_key: route_config.api_key.clone(),
            });
        let routing_key =
            ProviderRoutingKey::new(&credential.api_url, &active_model, &credential.api_key);
        ProviderRoute {
            api_url: credential.api_url,
            api_key: credential.api_key,
            routing_key,
            protocol: self.default_protocol,
        }
    }

    fn reserve_pool_route(&self) -> ProviderRoute {
        let pool = self.pool_routes.clone();
        let index = self.next_credential.fetch_add(1, Ordering::Relaxed);
        // 冷启动探测流量（A 方案）：无画像路由按轮转获得保底探测份额，
        // 跑出学习数据后转入正常 water-filling。这根治"无画像→不分流→更无画像"。
        if let Some(route) = self.try_cold_probe_route(&pool, index) {
            return route;
        }
        // 汇总每条 route 的容量与速度（water-filling 输入）。
        let entries = self.collect_route_entries(&pool);
        // 选择路由（water-filling 主逻辑）。
        let selected = self.select_water_filling_route(&pool, &entries, index);
        let selected_route = &pool[selected];
        let credential = selected_route
            .credentials
            .get(index % selected_route.credentials.len().max(1))
            .cloned()
            .unwrap_or_else(|| ProviderCredential {
                api_url: selected_route.api_url.clone(),
                api_key: String::new(),
            });
        let routing_key = ProviderRoutingKey::new(
            &credential.api_url,
            &selected_route.model,
            &credential.api_key,
        );
        ProviderRoute {
            api_url: credential.api_url,
            api_key: credential.api_key,
            routing_key,
            protocol: self.default_protocol,
        }
    }

    /// 冷启动探测：若池内有未探明路由，按轮转返回探测路由；否则返回 None。
    fn try_cold_probe_route(&self, pool: &[PoolRoute], index: usize) -> Option<ProviderRoute> {
        let has_unlearned = self
            .capacity_profile
            .as_ref()
            .map(|capacity_fn| {
                pool.iter().any(|route| {
                    !route.credentials.iter().any(|credential| {
                        capacity_fn(&ProviderRoutingKey::new(
                            &credential.api_url,
                            &route.model,
                            &credential.api_key,
                        ))
                        .has_learning
                    })
                })
            })
            .unwrap_or(false);
        if !has_unlearned {
            return None;
        }
        let unlearned: Vec<usize> = pool
            .iter()
            .enumerate()
            .filter_map(|(idx, route)| {
                let learned = self.capacity_profile.as_ref().map(|capacity_fn| {
                    route.credentials.iter().any(|credential| {
                        capacity_fn(&ProviderRoutingKey::new(
                            &credential.api_url,
                            &route.model,
                            &credential.api_key,
                        ))
                        .has_learning
                    })
                });
                (!learned.unwrap_or(false)).then_some(idx)
            })
            .collect();
        if unlearned.is_empty() {
            return None;
        }
        let probe_pos = self.cold_probe_cursor.fetch_add(1, Ordering::Relaxed);
        let probe_every = (pool.len() * 4).max(4);
        if probe_pos % probe_every != 0 {
            return None;
        }
        let idx = unlearned[probe_pos % unlearned.len()];
        let selected = &pool[idx];
        let credential = selected
            .credentials
            .get(index % selected.credentials.len().max(1))
            .cloned()
            .unwrap_or_else(|| ProviderCredential {
                api_url: selected.api_url.clone(),
                api_key: String::new(),
            });
        let routing_key =
            ProviderRoutingKey::new(&credential.api_url, &selected.model, &credential.api_key);
        Some(ProviderRoute {
            api_url: credential.api_url,
            api_key: credential.api_key,
            routing_key,
            protocol: self.default_protocol,
        })
    }

    /// 汇总每条 route 的容量与速度（water-filling 输入）。
    fn collect_route_entries(&self, pool: &[PoolRoute]) -> Vec<(f64, f64)> {
        pool.iter()
            .map(|route| {
                let profile = self.capacity_profile.as_ref().and_then(|capacity_fn| {
                    route.credentials.iter().find_map(|credential| {
                        let c = capacity_fn(&ProviderRoutingKey::new(
                            &credential.api_url,
                            &route.model,
                            &credential.api_key,
                        ));
                        c.has_learning.then_some(c)
                    })
                });
                match profile {
                    Some(c) => (c.speed_rpm, c.capacity_rpm),
                    _ => (self.cold_probe_capacity, self.cold_probe_capacity),
                }
            })
            .collect()
    }

    /// water-filling 选择路由：按速度降序，优先从尚未用满 c_i 的最快路由取，
    /// 用满后才溢出到次快路由（慢得离谱的跳过）。全部用满时回退到分配最少者。
    fn select_water_filling_route(
        &self,
        pool: &[PoolRoute],
        entries: &[(f64, f64)],
        index: usize,
    ) -> usize {
        let fastest_speed = entries
            .iter()
            .map(|(speed, _)| *speed)
            .fold(0.0_f64, f64::max);
        let mut allocated = self.allocated.lock().unwrap_or_else(|_| {
            #[allow(clippy::unwrap_used)]
            std::sync::PoisonError::into_inner(self.allocated.lock().unwrap_err())
        });
        if allocated.len() != pool.len() {
            *allocated = vec![0.0; pool.len()];
        }
        let mut order: Vec<usize> = (0..pool.len()).collect();
        order.sort_by(|&a, &b| {
            entries[b]
                .0
                .partial_cmp(&entries[a].0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        const SLOW_ROUTE_MIN_SPEED_RATIO: f64 = 0.2;
        let mut selected: Option<usize> = None;
        for &idx in &order {
            let (speed, capacity) = entries[idx];
            if speed <= 0.0 || capacity <= 0.0 {
                continue;
            }
            if speed < fastest_speed * SLOW_ROUTE_MIN_SPEED_RATIO {
                continue;
            }
            if allocated[idx] < capacity {
                selected = Some(idx);
                break;
            }
        }
        let selected = selected.unwrap_or_else(|| {
            let not_excluded = |idx: usize| {
                entries[idx].0 > 0.0
                    && entries[idx].1 > 0.0
                    && entries[idx].0 >= fastest_speed * SLOW_ROUTE_MIN_SPEED_RATIO
            };
            order
                .iter()
                .copied()
                .filter(|&idx| not_excluded(idx))
                .min_by(|&a, &b| {
                    allocated[a]
                        .partial_cmp(&allocated[b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(index % pool.len().max(1))
        });
        allocated[selected] += 1.0;
        selected
    }

    pub(crate) fn active_routing_keys(&self) -> Vec<ProviderRoutingKey> {
        if self.has_route_pool() {
            return self
                .pool_routes
                .iter()
                .flat_map(|route| {
                    route.credentials.iter().map(|credential| {
                        ProviderRoutingKey::new(
                            &credential.api_url,
                            &route.model,
                            &credential.api_key,
                        )
                    })
                })
                .collect();
        }
        let active_model = self.active_model();
        let route_config = self.route_config_for_model(&active_model);
        provider_credentials(&route_config.api_url, &route_config.api_key)
            .iter()
            .map(|credential| {
                ProviderRoutingKey::new(&credential.api_url, &active_model, &credential.api_key)
            })
            .collect()
    }

    pub(crate) fn maybe_activate_fallback_after_app_error(
        &self,
        current_model: &str,
        error: &crate::error::AppError,
    ) -> Option<String> {
        if !is_hard_quota_app_error(error) || !self.quota_failure_threshold_reached() {
            return None;
        }
        if !self.active_model().eq_ignore_ascii_case(current_model) {
            return None;
        }
        let fallback_model = self.fallback_model_for(current_model)?;
        if let Ok(mut active_model) = self.active_model.lock() {
            *active_model = fallback_model.clone();
        }
        Some(fallback_model)
    }

    #[cfg(test)]
    pub async fn translate_markdown_with_context(
        &self,
        source_text: &str,
        article_type: &str,
        system_prompt: &str,
        translation_context: Option<&str>,
    ) -> Result<StructuredTranslation, ProviderError> {
        let route = self.reserve_route();
        self.translate_markdown_with_context_on_route(
            &route,
            source_text,
            article_type,
            system_prompt,
            translation_context,
        )
        .await
    }

    pub async fn translate_markdown_with_context_on_route(
        &self,
        route: &ProviderRoute,
        source_text: &str,
        article_type: &str,
        system_prompt: &str,
        translation_context: Option<&str>,
    ) -> Result<StructuredTranslation, ProviderError> {
        if self.api_key.trim().is_empty() {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::Authentication,
                "LLM API key is empty",
            ));
        }

        let prompt = build_translation_user_prompt(source_text, article_type, translation_context);
        let model = route.routing_key().model.clone();
        let policy = ProviderRequestPolicy::for_model(&model, ProviderRequestPurpose::Translation);
        let request = LlmRequest {
            model,
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt,
                },
            ],
            temperature: policy.temperature.unwrap_or(0.3),
            top_p: policy.top_p,
            stream: policy.stream,
            max_tokens: policy.max_tokens,
            reasoning_effort: policy.reasoning_effort,
            response_format: policy.response_format,
            extra_body: policy.extra_body,
        };
        let content = self.post_chat_completion(route, request).await?;
        parse_structured_translation(&content)
    }

    pub async fn review_translation_issue(
        &self,
        review_prompt: &str,
    ) -> Result<String, ProviderError> {
        let route = self.reserve_route();
        self.review_translation_issue_on_route(&route, review_prompt)
            .await
    }

    pub async fn review_translation_issue_on_route(
        &self,
        route: &ProviderRoute,
        review_prompt: &str,
    ) -> Result<String, ProviderError> {
        self.complete_json_instruction_on_route(route, review_prompt, 0.0)
            .await
    }

    /// SSE 流式补全（AI 阅读辅助弹卡逐字上屏）：逐 chunk 回调 on_chunk，
    /// 返回完整文本。流式失败（网关不支持 SSE）时由调用方回落整段请求。
    pub async fn complete_instruction_streaming(
        &self,
        prompt: &str,
        mut on_chunk: impl FnMut(&str) + Send,
    ) -> Result<String, ProviderError> {
        if self.api_key.trim().is_empty() {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::Authentication,
                "LLM API key is empty",
            ));
        }
        let route = self.reserve_route();
        let model = route.routing_key().model.clone();
        let policy = ProviderRequestPolicy::for_model(&model, ProviderRequestPurpose::Instruction);
        let request = LlmRequest {
            model,
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: policy.temperature.unwrap_or(0.0),
            top_p: policy.top_p,
            stream: Some(true),
            max_tokens: policy.max_tokens,
            reasoning_effort: policy.reasoning_effort,
            response_format: None, // SSE 流式不强制 json_schema
            extra_body: policy.extra_body,
        };

        let response = self
            .client
            .post(&route.api_url)
            .header("Authorization", format!("Bearer {}", route.api_key))
            .header(ACCEPT, "text/event-stream")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            let body = response.text().await.unwrap_or_default();
            return Err(map_http_status("llm", status, retry_after, body));
        }

        // SSE 解析：data: {...} 行；[DONE] 结束。delta.content 为增量文本。
        let mut full = String::new();
        let mut response = response;
        // usage 全协议统计：部分网关在末帧携带 usage（OpenAI 兼容命名），取最后一个有效帧。
        let mut stream_usage: Option<serde_json::Value> = None;
        // 字节缓冲：网络 chunk 可能拆散多字节 UTF-8 汉字，按行切出完整帧后再解码。
        let mut buffer: Vec<u8> = Vec::new();
        let mut process_line = |line: &[u8], full: &mut String| {
            let Ok(line) = std::str::from_utf8(line) else { return };
            let line = line.trim();
            if !line.starts_with("data:") {
                return;
            }
            let payload = line[5..].trim();
            if payload == "[DONE]" {
                return;
            }
            let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) else {
                return;
            };
            if json.get("usage").is_some_and(|usage| !usage.is_null()) {
                stream_usage = Some(json.clone());
            }
            if let Some(delta) = json
                .pointer("/choices/0/delta/content")
                .and_then(|v| v.as_str())
            {
                if !delta.is_empty() {
                    full.push_str(delta);
                    on_chunk(delta);
                }
            }
        };
        while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let mut line = buffer.drain(..=pos).collect::<Vec<u8>>();
                if line.last() == Some(&b'\n') { line.pop(); }
                if line.last() == Some(&b'\r') { line.pop(); }
                process_line(&line, &mut full);
            }
        }
        // 流尾残留：部分网关最后一帧无尾换行，补一次解析。
        if !buffer.is_empty() {
            process_line(&buffer, &mut full);
        }
        if let Some(usage_json) = &stream_usage {
            self.record_usage_from_json(usage_json);
        }
        if full.trim().is_empty() {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::EmptyResponse,
                "streaming response produced no content",
            ));
        }
        Ok(full)
    }


    async fn complete_json_instruction_on_route(
        &self,
        route: &ProviderRoute,
        prompt: &str,
        temperature: f32,
    ) -> Result<String, ProviderError> {
        if self.api_key.trim().is_empty() {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::Authentication,
                "LLM API key is empty",
            ));
        }

        let model = route.routing_key().model.clone();
        let policy = ProviderRequestPolicy::for_model(&model, ProviderRequestPurpose::Instruction);
        let request = LlmRequest {
            model,
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: policy.temperature.unwrap_or(temperature),
            top_p: policy.top_p,
            stream: policy.stream,
            max_tokens: policy.max_tokens,
            reasoning_effort: policy.reasoning_effort,
            response_format: policy.response_format,
            extra_body: policy.extra_body,
        };
        self.post_chat_completion(route, request).await
    }

    async fn post_chat_completion(
        &self,
        route: &ProviderRoute,
        request: LlmRequest,
    ) -> Result<String, ProviderError> {
        // 协议分发：用户显式选择优先；Auto 按 URL 启发式回落（旧配置兼容）。
        let url_hint_anthropic = route.api_url.contains("anthropic")
            || route.api_url.ends_with("/messages");
        let url_hint_responses = route.api_url.ends_with("/responses");
        match route.protocol {
            RequestProtocol::AnthropicMessages => {
                return self.post_anthropic_messages(route, request).await;
            }
            RequestProtocol::OpenaiResponses => {
                return self.post_openai_responses(route, request).await;
            }
            RequestProtocol::OpenaiCompatible => {
                return self.post_openai_chat(route, request).await;
            }
            RequestProtocol::Auto if url_hint_anthropic => {
                return self.post_anthropic_messages(route, request).await;
            }
            RequestProtocol::Auto if url_hint_responses => {
                return self.post_openai_responses(route, request).await;
            }
            RequestProtocol::Auto => {
                return self.post_openai_chat(route, request).await;
            }
        }
    }

    /// OpenAI chat/completions（openai / openai-compatible / Auto 默认）。
    async fn post_openai_chat(
        &self,
        route: &ProviderRoute,
        request: LlmRequest,
    ) -> Result<String, ProviderError> {

        let response = self
            .client
            .post(&route.api_url)
            .header("Authorization", format!("Bearer {}", route.api_key))
            .header(ACCEPT, "application/json")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            let body = response.text().await.unwrap_or_default();
            return Err(map_http_status("llm", status, retry_after, body));
        }

        let body = response.bytes().await.map_err(map_reqwest_error)?;
        let llm_response = parse_llm_response_bytes(&body)?;
        if let Some(usage) = llm_response.usage {
            if let Ok(mut totals) = self.usage_totals.lock() {
                totals.add_usage(usage);
            }
        }

        let Some(choice) = llm_response.choices.into_iter().next() else {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::EmptyResponse,
                "LLM returned no choices",
            ));
        };

        Ok(choice.message.content)
    }

    /// Anthropic Messages 协议：
    /// 请求 POST {base}/v1/messages，header x-api-key + anthropic-version，
    /// 响应 content[0].text 即译文；usage 字段 input/output_tokens 映射到统一用量。
    async fn post_anthropic_messages(
        &self,
        route: &ProviderRoute,
        request: LlmRequest,
    ) -> Result<String, ProviderError> {
        // Anthropic 端点形如 https://api.anthropic.com/v1/messages
        let url = if route.api_url.ends_with("/messages") {
            route.api_url.clone()
        } else if route.api_url.ends_with("/chat/completions") {
            route.api_url.replace("/chat/completions", "/messages")
        } else {
            format!("{}/v1/messages", route.api_url.trim_end_matches('/'))
        };

        // Anthropic Messages 协议要求 system prompt 单独作为顶层字段，
        // messages 数组内仅能包含 role="user" 或 role="assistant"
        let mut system_text = String::new();
        let mut anthropic_messages = Vec::new();
        for m in &request.messages {
            if m.role == "system" {
                if !system_text.is_empty() {
                    system_text.push_str("

");
                }
                system_text.push_str(&m.content);
            } else {
                anthropic_messages.push(serde_json::json!({
                    "role": if m.role == "assistant" { "assistant" } else { "user" },
                    "content": m.content,
                }));
            }
        }

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": anthropic_messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
            "temperature": request.temperature,
        });

        if !system_text.is_empty() {
            body["system"] = serde_json::Value::String(system_text);
        }

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &route.api_key)
            .header("anthropic-version", "2023-06-01")
            .header(ACCEPT, "application/json")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            let error_body = response.text().await.unwrap_or_default();
            return Err(map_http_status("llm", status, retry_after, error_body));
        }

        let raw = response.bytes().await.map_err(map_reqwest_error)?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            ProviderError::new(
                "llm",
                ProviderErrorKind::Decode,
                &format!("Anthropic response decode failed: {error}"),
            )
        })?;
        // usage 全协议统计：Anthropic 的 input/output_tokens 计入统一用量。
        self.record_usage_from_json(&parsed);

        let text = parsed
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|block| block.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::EmptyResponse,
                "Anthropic returned empty content",
            ));
        }
        Ok(text)
    }


    /// OpenAI Responses 协议：POST {base}/responses，Bearer 头。
    /// 响应 output[].content[].text 拼接；usage input/output_tokens 映射统一用量。
    async fn post_openai_responses(
        &self,
        route: &ProviderRoute,
        request: LlmRequest,
    ) -> Result<String, ProviderError> {
        let url = if route.api_url.ends_with("/responses") {
            route.api_url.clone()
        } else if route.api_url.ends_with("/chat/completions") {
            route.api_url.replace("/chat/completions", "/responses")
        } else if route.api_url.ends_with("/completions") {
            route.api_url.replace("/completions", "/responses")
        } else {
            format!("{}/responses", route.api_url.trim_end_matches('/'))
        };

        // Responses 协议：instructions 承载 system，input 为消息数组。
        let mut instructions = String::new();
        let mut input_messages = Vec::new();
        for m in &request.messages {
            if m.role == "system" {
                if !instructions.is_empty() {
                    instructions.push_str("

");
                }
                instructions.push_str(&m.content);
            } else {
                input_messages.push(serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                }));
            }
        }

        let mut body = serde_json::json!({
            "model": request.model,
            "input": input_messages,
        });
        if !instructions.is_empty() {
            body["instructions"] = serde_json::Value::String(instructions);
        }
        if request.temperature >= 0.0 {
            body["temperature"] = serde_json::json!(request.temperature);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_output_tokens"] = serde_json::json!(max_tokens);
        }
        if let Some(effort) = &request.reasoning_effort {
            body["reasoning"] = serde_json::json!({ "effort": effort });
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", route.api_key))
            .header(ACCEPT, "application/json")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            let error_body = response.text().await.unwrap_or_default();
            return Err(map_http_status("llm", status, retry_after, error_body));
        }

        let raw = response.bytes().await.map_err(map_reqwest_error)?;
        let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|error| {
            ProviderError::new(
                "llm",
                ProviderErrorKind::Decode,
                &format!("Responses API decode failed: {error}"),
            )
        })?;
        // usage 全协议统计：Responses 的 input/output_tokens 计入统一用量。
        self.record_usage_from_json(&parsed);

        // output 数组内 content 块的 text 全拼接（多段推理/正文）
        let mut text = String::new();
        if let Some(output) = parsed.get("output").and_then(|o| o.as_array()) {
            for item in output {
                if let Some(blocks) = item.get("content").and_then(|c| c.as_array()) {
                    for block in blocks {
                        if let Some(piece) = block.get("text").and_then(|t| t.as_str()) {
                            text.push_str(piece);
                        }
                    }
                }
            }
        }

        if text.trim().is_empty() {
            return Err(ProviderError::new(
                "llm",
                ProviderErrorKind::EmptyResponse,
                "Responses API returned empty output",
            ));
        }
        Ok(text)
    }
    #[cfg(test)]
    async fn with_quota_fallback<T, F, Fut>(&self, mut operation: F) -> Result<T, ProviderError>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, ProviderError>>,
    {
        let current_model = self.active_model();
        match operation(current_model.clone()).await {
            Ok(value) => {
                if current_model.eq_ignore_ascii_case(&self.model) {
                    self.reset_quota_failures_before_fallback();
                }
                Ok(value)
            }
            Err(error) if policy::should_fallback_for_quota(&error) => {
                if !self.quota_failure_threshold_reached() {
                    return Err(error);
                }
                let Some(fallback_model) = self.fallback_model_for(&current_model) else {
                    return Err(error);
                };
                if let Ok(mut active_model) = self.active_model.lock() {
                    *active_model = fallback_model.clone();
                }
                operation(fallback_model.clone())
                    .await
                    .map_err(|fallback_error| {
                        policy::merge_fallback_error(
                            &current_model,
                            &error,
                            &fallback_model,
                            fallback_error,
                        )
                    })
            }
            Err(error) => Err(error),
        }
    }

    fn quota_failure_threshold_reached(&self) -> bool {
        let threshold = policy::sensenova_fallback_quota_failure_threshold();
        self.quota_failures_before_fallback
            .lock()
            .map(|mut failures| {
                *failures = failures.saturating_add(1);
                *failures >= threshold
            })
            .unwrap_or(false)
    }

    #[cfg(test)]
    fn reset_quota_failures_before_fallback(&self) {
        if let Ok(mut failures) = self.quota_failures_before_fallback.lock() {
            *failures = 0;
        }
    }

    fn fallback_model_for(&self, current_model: &str) -> Option<String> {
        self.fallback_routes
            .iter()
            .map(|route| route.model.trim())
            .find(|model| !model.is_empty() && !model.eq_ignore_ascii_case(current_model))
            .map(ToString::to_string)
            .or_else(|| fallback_model_for(&self.api_url, current_model))
    }

    fn route_config_for_model(&self, model: &str) -> LlmFallbackRoute {
        self.fallback_routes
            .iter()
            .find(|route| route.model.eq_ignore_ascii_case(model))
            .cloned()
            .unwrap_or_else(|| LlmFallbackRoute {
                api_url: self.api_url.clone(),
                api_key: self.api_key.clone(),
                model: self.model.clone(),
            })
    }
}

fn is_hard_quota_app_error(error: &crate::error::AppError) -> bool {
    if !matches!(
        error.code,
        "LLM_QUOTA_EXHAUSTED" | "PROVIDER_QUOTA_EXHAUSTED"
    ) {
        return false;
    }
    let message = error.message.to_ascii_lowercase();
    ![
        "rpm",
        "qps",
        "tpm",
        "rate limit",
        "rate_limit",
        "requests per minute",
        "request per minute",
        "tokens per minute",
        "frequency limit",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn optional_text(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

fn optional_u32_text(value: Option<u32>) -> String {
    value
        .map(|item| item.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn optional_f32_text(value: Option<f32>) -> String {
    value
        .map(|item| item.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn optional_bool_text(value: Option<bool>) -> String {
    value
        .map(|item| item.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn extra_body_summary(extra_body: &LlmExtraBody) -> String {
    let thinking = extra_body
        .chat_template_kwargs
        .as_ref()
        .map(|kwargs| kwargs.enable_thinking)
        .unwrap_or(false);
    let budget = extra_body
        .reasoning_budget
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());
    format!("thinking:{thinking},reasoning_budget:{budget}")
}

fn provider_credentials(api_url: &str, api_key: &str) -> Vec<ProviderCredential> {
    let keys = split_provider_api_keys(api_key);
    if keys.is_empty() {
        return vec![ProviderCredential {
            api_url: api_url.to_string(),
            api_key: String::new(),
        }];
    }
    keys.into_iter()
        .map(|key| ProviderCredential {
            api_url: api_url.to_string(),
            api_key: key,
        })
        .collect()
}

fn split_provider_api_keys(api_key: &str) -> Vec<String> {
    api_key
        .split(|ch| matches!(ch, ',' | ';' | '\n' | '\r'))
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod request_config_tests {
    use super::*;
    use crate::llm::policy::{
        instruction_reasoning_effort, instruction_response_format, translation_reasoning_effort,
        translation_response_format,
    };
    use std::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        crate::llm::policy::env_test_lock()
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
    fn non_deepseek_translation_request_keeps_provider_defaults() {
        let _guard = env_lock().lock().expect("env test lock");
        let _reasoning = EnvRestore::remove("MUSETRANSLATE_REASONING_EFFORT");
        let _instruction = EnvRestore::remove("MUSETRANSLATE_INSTRUCTION_REASONING_EFFORT");

        assert_eq!(translation_reasoning_effort("MiniMax-M2.7"), None);
        assert_eq!(
            translation_response_format("MiniMax-M2.7")
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(
            translation_response_format("command-a-plus-05-2026")
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(
            translation_response_format("stepfun-ai/step-3.7-flash")
                .as_ref()
                .map(|value| value.kind.as_str()),
            Some("json_object")
        );
        assert_eq!(instruction_reasoning_effort("command-a-plus-05-2026"), None);
    }

    #[test]
    fn max_tokens_is_omitted_when_unbounded() {
        let request = LlmRequest {
            model: "deepseek-v4-flash".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Translate as JSON.".to_string(),
            }],
            temperature: 0.3,
            top_p: None,
            stream: None,
            max_tokens: None,
            reasoning_effort: None,
            response_format: Some(ResponseFormat {
                kind: "json_object".to_string(),
            }),
            extra_body: LlmExtraBody::default(),
        };

        let json = serde_json::to_value(&request).expect("request should serialize");
        assert!(json.get("max_tokens").is_none());
        assert_eq!(
            json.pointer("/response_format/type")
                .and_then(|value| value.as_str()),
            Some("json_object")
        );
    }

    #[test]
    fn diffusiongemma_request_serializes_explicit_max_tokens() {
        let _guard = env_lock().lock().expect("env test lock");
        let _max_tokens = EnvRestore::remove("MUSETRANSLATE_DIFFUSIONGEMMA_MAX_TOKENS");
        let policy = ProviderRequestPolicy::for_model(
            "google/diffusiongemma-26b-a4b-it",
            ProviderRequestPurpose::Translation,
        );
        let request = LlmRequest {
            model: "google/diffusiongemma-26b-a4b-it".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Translate as JSON.".to_string(),
            }],
            temperature: policy.temperature.unwrap_or(0.3),
            top_p: policy.top_p,
            stream: policy.stream,
            max_tokens: policy.max_tokens,
            reasoning_effort: policy.reasoning_effort,
            response_format: policy.response_format,
            extra_body: policy.extra_body,
        };

        let json = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            json.get("max_tokens").and_then(|value| value.as_u64()),
            Some(200_000)
        );
        assert_eq!(
            json.pointer("/response_format/type")
                .and_then(|value| value.as_str()),
            None
        );
        assert_eq!(
            json.pointer("/chat_template_kwargs/enable_thinking")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            json.get("temperature").and_then(|value| value.as_f64()),
            Some(1.0)
        );
        assert!(json
            .get("top_p")
            .and_then(|value| value.as_f64())
            .is_some_and(|value| (value - 0.95).abs() < 0.000_001));
        assert_eq!(
            json.get("stream").and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn diffusiongemma_policy_does_not_apply_to_other_models() {
        let policy = ProviderRequestPolicy::for_model(
            "stepfun-ai/step-3.7-flash",
            ProviderRequestPurpose::Translation,
        );

        assert_eq!(policy.temperature, None);
        assert_eq!(policy.top_p, None);
        assert_eq!(policy.stream, None);
    }

    #[test]
    fn nvidia_nemotron_policy_adds_thinking_body_without_max_tokens() {
        let policy = ProviderRequestPolicy::for_model(
            "nvidia/nemotron-3-ultra-550b-a55b",
            ProviderRequestPurpose::Translation,
        );
        let request = LlmRequest {
            model: "nvidia/nemotron-3-ultra-550b-a55b".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Translate as JSON.".to_string(),
            }],
            temperature: 0.3,
            top_p: policy.top_p,
            stream: policy.stream,
            max_tokens: policy.max_tokens,
            reasoning_effort: policy.reasoning_effort,
            response_format: policy.response_format,
            extra_body: policy.extra_body,
        };

        let json = serde_json::to_value(&request).expect("request should serialize");
        assert!(json.get("max_tokens").is_none());
        assert_eq!(
            json.pointer("/chat_template_kwargs/enable_thinking")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            json.pointer("/reasoning_budget")
                .and_then(|value| value.as_u64()),
            Some(16_384)
        );
    }

    #[test]
    fn comma_separated_provider_keys_rotate_routing_fingerprints() {
        let client = SensenovaClient::new(
            "https://integrate.api.nvidia.com/v1/chat/completions".to_string(),
            "nvidia-key-a,nvidia-key-b".to_string(),
            "nvidia/nemotron-3-ultra-550b-a55b".to_string(),
        );

        let first = client.reserve_route();
        let second = client.reserve_route();
        let third = client.reserve_route();

        assert_eq!(first.routing_key().provider, "nvidia");
        assert_ne!(
            first.routing_key().key_fingerprint,
            second.routing_key().key_fingerprint
        );
        assert_eq!(
            first.routing_key().key_fingerprint,
            third.routing_key().key_fingerprint
        );
    }

    #[test]
    fn hard_quota_switches_next_route_to_configured_nvidia_fallback() {
        let _guard = env_lock().lock().expect("env test lock");
        let _threshold = EnvRestore::remove("MUSETRANSLATE_LLM_FALLBACK_AFTER_QUOTA_FAILURES");
        std::env::set_var("MUSETRANSLATE_LLM_FALLBACK_AFTER_QUOTA_FAILURES", "1");
        let client = SensenovaClient::new_with_fallback_routes(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sensenova-key".to_string(),
            "deepseek-v4-flash".to_string(),
            vec![LlmFallbackRoute {
                api_url: "https://integrate.api.nvidia.com/v1/chat/completions".to_string(),
                api_key: "nvidia-key-a,nvidia-key-b".to_string(),
                model: "stepfun-ai/step-3.7-flash".to_string(),
            }],
        );

        let primary = client.reserve_route();
        assert_eq!(primary.routing_key().provider, "sensenova");

        let error = crate::error::AppError::new("LLM_QUOTA_EXHAUSTED", "insufficient quota", true);
        let fallback = client.maybe_activate_fallback_after_app_error("deepseek-v4-flash", &error);
        assert_eq!(fallback.as_deref(), Some("stepfun-ai/step-3.7-flash"));

        let fallback_route = client.reserve_route();
        assert_eq!(fallback_route.routing_key().provider, "nvidia");
        assert_eq!(
            fallback_route.routing_key().model,
            "stepfun-ai/step-3.7-flash"
        );
    }

    #[test]
    fn explicit_reasoning_env_overrides_default() {
        let _guard = env_lock().lock().expect("env test lock");
        let _reasoning = EnvRestore::remove("MUSETRANSLATE_REASONING_EFFORT");
        let _instruction = EnvRestore::remove("MUSETRANSLATE_INSTRUCTION_REASONING_EFFORT");
        std::env::set_var("MUSETRANSLATE_REASONING_EFFORT", "high");
        std::env::set_var("MUSETRANSLATE_INSTRUCTION_REASONING_EFFORT", "low");

        assert_eq!(
            translation_reasoning_effort("deepseek-v4-flash").as_deref(),
            Some("high")
        );
        assert_eq!(
            instruction_reasoning_effort("deepseek-v4-flash").as_deref(),
            Some("low")
        );
    }

    #[test]
    fn sensenova_fallback_is_disabled_by_default() {
        let _guard = env_lock().lock().expect("env test lock");
        let _fallbacks = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS");
        let _fallback = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODEL");
        let client = SensenovaClient::new(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "key".to_string(),
            "deepseek-v4-flash".to_string(),
        );

        assert_eq!(
            client.fallback_model_for("deepseek-v4-flash").as_deref(),
            None
        );
    }

    #[test]
    fn sensenova_fallback_does_not_loop_to_current_model() {
        let _guard = env_lock().lock().expect("env test lock");
        let _fallbacks = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS");
        let _fallback = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODEL");
        let client = SensenovaClient::new(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "key".to_string(),
            "sensenova-6.7-flash-lite".to_string(),
        );

        assert!(client
            .fallback_model_for("sensenova-6.7-flash-lite")
            .is_none());
    }

    #[test]
    fn sensenova_fallback_can_be_configured_as_list() {
        let _guard = env_lock().lock().expect("env test lock");
        let _fallbacks = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS");
        let _fallback = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODEL");
        std::env::set_var(
            "MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS",
            "deepseek-v4-flash, sensenova-6.7-flash-lite",
        );
        let client = SensenovaClient::new(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "key".to_string(),
            "deepseek-v4-flash".to_string(),
        );

        assert_eq!(
            client.fallback_model_for("deepseek-v4-flash").as_deref(),
            Some("sensenova-6.7-flash-lite")
        );
    }

    #[tokio::test]
    async fn quota_fallback_waits_for_configured_failure_threshold() {
        let _guard = env_lock().lock().expect("env test lock");
        let _fallbacks = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS");
        let _fallback = EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_MODEL");
        let _threshold =
            EnvRestore::remove("MUSETRANSLATE_SENSENOVA_FALLBACK_AFTER_QUOTA_FAILURES");
        std::env::set_var(
            "MUSETRANSLATE_SENSENOVA_FALLBACK_MODELS",
            "sensenova-6.7-flash-lite",
        );
        std::env::set_var("MUSETRANSLATE_SENSENOVA_FALLBACK_AFTER_QUOTA_FAILURES", "3");
        let client = SensenovaClient::new(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "key".to_string(),
            "deepseek-v4-flash".to_string(),
        );

        for _ in 0..2 {
            let error = client
                .with_quota_fallback(|_| async {
                    Err::<String, ProviderError>(quota_exhausted_error())
                })
                .await
                .expect_err("quota errors before threshold should surface");
            assert!(matches!(error.kind, ProviderErrorKind::QuotaExhausted));
            assert_eq!(client.active_model(), "deepseek-v4-flash");
        }

        let model = client
            .with_quota_fallback(|model| async move {
                if model == "deepseek-v4-flash" {
                    Err(quota_exhausted_error())
                } else {
                    Ok(model)
                }
            })
            .await
            .expect("fallback should run after threshold");

        assert_eq!(model, "sensenova-6.7-flash-lite");
        assert_eq!(client.active_model(), "sensenova-6.7-flash-lite");
    }

    fn quota_exhausted_error() -> ProviderError {
        ProviderError::new(
            "llm",
            ProviderErrorKind::QuotaExhausted,
            "insufficient quota",
        )
    }

    fn sample_pool() -> crate::commands::RoutePoolConfig {
        crate::commands::RoutePoolConfig {
            name: "default".to_string(),
            routes: vec![
                crate::commands::PoolRouteConfig {
                    provider: "sensenova".to_string(),
                    api_url: Some("https://token.sensenova.cn/v1/chat/completions".to_string()),
                    invoke_url: None,
                    base_url: None,
                    model: "deepseek-v4-flash".to_string(),
                    api_keys: vec!["sk-a".to_string(), "sk-b".to_string()],
                    api_key: None,
                    weight: 1.0,
                },
                crate::commands::PoolRouteConfig {
                    provider: "nvidia".to_string(),
                    api_url: None,
                    invoke_url: Some(
                        "https://integrate.api.nvidia.com/v1/chat/completions".to_string(),
                    ),
                    base_url: None,
                    model: "thinkingmachines/inkling".to_string(),
                    api_keys: vec!["nk-a".to_string()],
                    api_key: None,
                    weight: 1.0,
                },
            ],
        }
    }

    #[test]
    fn route_pool_client_exposes_all_pool_routing_keys() {
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        );
        let mut providers = client
            .active_routing_keys()
            .into_iter()
            .map(|key| (key.provider, key.model))
            .collect::<Vec<_>>();
        providers.sort();
        // 两条 route × 各自 key 数全部暴露（sensenova 2 key + nvidia 1 key = 3）
        assert_eq!(providers.len(), 3);
        assert!(providers
            .iter()
            .any(|(_, model)| model == "deepseek-v4-flash"));
        assert!(providers
            .iter()
            .any(|(_, model)| model == "thinkingmachines/inkling"));
    }

    #[test]
    fn route_pool_reserve_route_uses_pool_models_not_active_model() {
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        );
        // 号池模式下 reserve_route 应返回池内 route 的 model，而非 active_model
        let models = (0..8)
            .map(|_| client.reserve_route().routing_key().model.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(models.contains("deepseek-v4-flash"));
        assert!(models.contains("thinkingmachines/inkling"));
    }

    #[test]
    fn route_pool_reserve_route_rotates_keys_within_route() {
        let single_route_pool = crate::commands::RoutePoolConfig {
            name: "nv".to_string(),
            routes: vec![crate::commands::PoolRouteConfig {
                provider: "nvidia".to_string(),
                api_url: Some("https://integrate.api.nvidia.com/v1/chat/completions".to_string()),
                invoke_url: None,
                base_url: None,
                model: "thinkingmachines/inkling".to_string(),
                api_keys: vec!["k1".to_string(), "k2".to_string()],
                api_key: None,
                weight: 1.0,
            }],
        };
        let client = SensenovaClient::new_with_route_pool(
            "https://x".to_string(),
            "k1".to_string(),
            "thinkingmachines/inkling".to_string(),
            &single_route_pool,
        );
        let fingerprints = (0..4)
            .map(|_| client.reserve_route().routing_key().key_fingerprint.clone())
            .collect::<std::collections::BTreeSet<_>>();
        // 同 route 内多 key 应轮转（≥2 个不同指纹）
        assert!(fingerprints.len() >= 2);
    }

    #[test]
    fn route_pool_dynamic_weight_favors_faster_route() {
        // 双 route：deepseek 与 inkling。
        // 注入容量画像：deepseek 快（speed=6.0/cap=6.0）、inkling 慢（speed=1.0/cap=1.0）。
        // water-filling：deepseek 快且容量够 → 全分给 deepseek（最快先用满，慢的不加入）。
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        )
        .with_capacity_profile(std::sync::Arc::new(|key| RouteCapacity {
            speed_rpm: if key.model == "deepseek-v4-flash" {
                6.0
            } else {
                1.0
            },
            capacity_rpm: if key.model == "deepseek-v4-flash" {
                6.0
            } else {
                1.0
            },
            has_learning: true,
        }));
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for _ in 0..70 {
            *counts
                .entry(client.reserve_route().routing_key().model.clone())
                .or_insert(0) += 1;
        }
        let deepseek = counts.get("deepseek-v4-flash").copied().unwrap_or(0);
        let inkling = counts.get("thinkingmachines/inkling").copied().unwrap_or(0);
        // water-filling：deepseek 快且 cap=6，70 次需求 → deepseek 用满 6 后，
        // inkling 虽慢但 speed=1 ≥ 6*0.2=1.2？否，1<1.2 → inkling 被 ρ 排除，
        // deepseek 继续扛（回退 min-allocated 仍是 deepseek）。
        assert!(
            deepseek > inkling * 3,
            "water-filling should strongly favor fastest route, got deepseek={deepseek} inkling={inkling}"
        );
    }

    #[test]
    fn route_pool_cold_start_falls_back_to_config_weight() {
        // 无学习数据（has_learning=false）→ 冷启动回退：capacity_profile 返回 has_learning=false
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        )
        .with_capacity_profile(std::sync::Arc::new(|_| RouteCapacity {
            speed_rpm: 0.0,
            capacity_rpm: 0.0,
            has_learning: false,
        }))
        .with_cold_probe_capacity(2.0);
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for _ in 0..40 {
            *counts
                .entry(client.reserve_route().routing_key().model.clone())
                .or_insert(0) += 1;
        }
        let deepseek = counts.get("deepseek-v4-flash").copied().unwrap_or(0);
        let inkling = counts.get("thinkingmachines/inkling").copied().unwrap_or(0);
        // 冷启动中位速度 2.0，两个模型都应被选中（不饿死）
        assert!(deepseek > 0 && inkling > 0);
    }

    #[test]
    fn route_pool_cold_probe_guarantees_unlearned_route_traffic() {
        // A 方案：deepseek 有画像（快），inkling 无画像（冷启动被压制水位）。
        // 探测流量应保证 inkling 也拿到请求（破除"无画像→不分流→更无画像"死锁）。
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        )
        .with_capacity_profile(std::sync::Arc::new(|key| {
            if key.model == "deepseek-v4-flash" {
                RouteCapacity {
                    speed_rpm: 5.0,
                    capacity_rpm: 100.0,
                    has_learning: true,
                }
            } else {
                RouteCapacity {
                    speed_rpm: 0.0,
                    capacity_rpm: 0.0,
                    has_learning: false,
                }
            }
        }))
        .with_cold_probe_capacity(0.5); // 冷启动水位被压得很低（模拟被旧画像压制）
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for _ in 0..64 {
            *counts
                .entry(client.reserve_route().routing_key().model.clone())
                .or_insert(0) += 1;
        }
        let inkling = counts.get("thinkingmachines/inkling").copied().unwrap_or(0);
        let deepseek = counts.get("deepseek-v4-flash").copied().unwrap_or(0);
        // 无探测时 inkling 会因冷启动水位过低而近乎零流量；有探测则保证 > 0，
        // 且 deepseek（有画像、快）仍扛绝大多数流量。
        assert!(
            inkling > 0,
            "cold-probe should guarantee unlearned route some traffic, got inkling=0"
        );
        assert!(
            deepseek > inkling,
            "learned fast route should still dominate, deepseek={deepseek} inkling={inkling}"
        );
    }

    #[test]
    fn route_pool_unstable_route_down_weighted_but_not_killed() {
        // inkling 容量低（c=0.5 慢/不稳）但不慢得离谱（speed=1.0 与 deepseek 同级）→ 降权但仍有流量
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        )
        .with_capacity_profile(std::sync::Arc::new(|key| RouteCapacity {
            // deepseek 快(5.0)；inkling 慢(1.0)但速度比 1/5=0.2 ≥ 0.2 → 不被排除
            speed_rpm: if key.model == "thinkingmachines/inkling" {
                1.0
            } else {
                5.0
            },
            capacity_rpm: if key.model == "thinkingmachines/inkling" {
                2.0
            } else {
                10.0
            },
            has_learning: true,
        }));
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for _ in 0..100 {
            *counts
                .entry(client.reserve_route().routing_key().model.clone())
                .or_insert(0) += 1;
        }
        let inkling = counts.get("thinkingmachines/inkling").copied().unwrap_or(0);
        let deepseek = counts.get("deepseek-v4-flash").copied().unwrap_or(0);
        // water-filling：deepseek 快先用满（cap=10），100 次需求 → deepseek 用满 10 后
        // inkling 加入（speed=1 ≥ 5*0.2=1.0 → 不被排除）。最终 deepseek≈10+回退倾斜、
        // inkling 拿到大量溢出。验证：两者都 > 0，且 inkling 拿到可观溢出（非零）。
        assert!(deepseek > 0, "faster route should be used");
        assert!(
            inkling > 0,
            "slower-but-not-too-slow route should still get overflow traffic"
        );
    }

    #[test]
    fn route_pool_cold_probe_weight_gives_new_route_traffic() {
        // 新路由无学习数据（has_learning=false），注入冷启动中位速度 10.0，
        // 另一路由 speed=1.0 → 新路由应分到大部分流量（不被压制）。
        let client = SensenovaClient::new_with_route_pool(
            "https://token.sensenova.cn/v1/chat/completions".to_string(),
            "sk-a".to_string(),
            "deepseek-v4-flash".to_string(),
            &sample_pool(),
        )
        .with_capacity_profile(std::sync::Arc::new(|key| RouteCapacity {
            speed_rpm: if key.model == "deepseek-v4-flash" {
                1.0
            } else {
                0.0
            },
            capacity_rpm: if key.model == "deepseek-v4-flash" {
                1.0
            } else {
                0.0
            },
            has_learning: key.model == "deepseek-v4-flash",
        }))
        .with_cold_probe_capacity(10.0);
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for _ in 0..40 {
            *counts
                .entry(client.reserve_route().routing_key().model.clone())
                .or_insert(0) += 1;
        }
        let inkling = counts.get("thinkingmachines/inkling").copied().unwrap_or(0);
        // 无试探权重时 inkling 会回退配置 1.0 与 deepseek 平分；有试探 10.0 时应显著多
        assert!(
            inkling > 20,
            "cold probe weight should give new route most traffic, inkling={inkling}"
        );
    }
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
    #[test]
    fn request_protocol_labels_parse_with_auto_fallback() {
        assert_eq!(RequestProtocol::from_label("openai"), RequestProtocol::OpenaiCompatible);
        assert_eq!(RequestProtocol::from_label("openai-compatible"), RequestProtocol::OpenaiCompatible);
        assert_eq!(RequestProtocol::from_label(" openai-responses "), RequestProtocol::OpenaiResponses);
        assert_eq!(RequestProtocol::from_label("anthropic-messages"), RequestProtocol::AnthropicMessages);
        // 未知/空标签回落 Auto（旧配置兼容）
        assert_eq!(RequestProtocol::from_label(""), RequestProtocol::Auto);
        assert_eq!(RequestProtocol::from_label("whatever"), RequestProtocol::Auto);
    }

    #[test]
    fn provider_route_carries_protocol_from_builder() {
        let client = SensenovaClient::new(
            "https://api.example.com/v1/chat/completions".to_string(),
            "k".to_string(),
            "m".to_string(),
        )
        .with_protocol(RequestProtocol::AnthropicMessages);
        let route = client.reserve_route();
        assert_eq!(route.protocol, RequestProtocol::AnthropicMessages);
        // 未设置时为 Auto
        let plain = SensenovaClient::new(
            "https://api.example.com/v1/chat/completions".to_string(),
            "k".to_string(),
            "m".to_string(),
        );
        assert_eq!(plain.reserve_route().protocol, RequestProtocol::Auto);
    }

