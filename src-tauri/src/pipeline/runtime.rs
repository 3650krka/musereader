use crate::error::AppError;
use crate::llm::ProviderRoutingKey;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
#[cfg(not(test))]
use std::time::Instant as StdInstant; // 落盘节流（见 persist_profiles_to_configured_store）
use tokio::time::sleep;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) const OPERATION_CANCELLED_MESSAGE: &str = "operation cancelled before completion";
const RATE_LIMIT_FALLBACK_DELAY: Duration = Duration::from_secs(60);
const DEFAULT_INITIAL_LLM_RPM: f64 = 12.0;
const DEFAULT_MIN_LLM_RPM: f64 = 6.0;
const DEFAULT_MAX_LLM_RPM: f64 = 60.0;
const RATE_LIMIT_BASE_COOLDOWN: Duration = Duration::from_secs(60);
const RATE_LIMIT_MAX_COOLDOWN: Duration = Duration::from_secs(180);
const RATE_LIMIT_BACKOFF_FACTOR: f64 = 0.65;
const SUCCESS_PROBE_MIN_INTERVAL: u32 = 8;
const SUCCESS_PROBE_MAX_INTERVAL: u32 = 28;
const SUCCESS_PROBE_DISTANCE_FACTOR: f64 = 0.18;
const ROUTE_CAPACITY_POLL_DELAY: Duration = Duration::from_millis(250);
const PROFILE_LATENCY_SAMPLE_LIMIT: usize = 64;
const DEFAULT_ROUTE_PARALLEL: usize = 1;
const DEFAULT_MAX_ROUTE_PARALLEL: usize = 64;
const CONCURRENCY_BACKOFF_FACTOR: f64 = 0.65;

pub(super) async fn retry_async_with_observer<T, F, Fut, C, O>(
    retry_limit: u8,
    base_delay: Duration,
    is_cancelled: C,
    mut operation: F,
    mut on_retryable_error: O,
) -> Result<T, AppError>
where
    F: FnMut(u8) -> Fut,
    Fut: std::future::Future<Output = Result<T, AppError>>,
    C: Fn() -> bool,
    O: FnMut(u8, &AppError, Duration),
{
    let mut attempt = 1u8;
    loop {
        if is_cancelled() {
            return Err(AppError::cancelled(OPERATION_CANCELLED_MESSAGE));
        }
        match operation(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) if attempt < retry_limit && error.retryable => {
                let delay = compute_retry_delay(&error, base_delay, attempt);
                on_retryable_error(attempt, &error, delay);
                sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn compute_retry_delay(error: &AppError, base_delay: Duration, attempt: u8) -> Duration {
    let _ = base_delay;
    let stepped = Duration::from_secs(u64::from(attempt).saturating_mul(5));
    if is_rate_limit_error(error) {
        let retry_after = error.retry_after_ms.map(Duration::from_millis);
        return retry_after
            .unwrap_or(RATE_LIMIT_FALLBACK_DELAY)
            .max(RATE_LIMIT_FALLBACK_DELAY)
            .max(stepped);
    }
    stepped
}

fn is_rate_limit_error(error: &AppError) -> bool {
    matches!(
        error.code,
        "LLM_RATE_LIMITED" | "OCR_RATE_LIMITED" | "PROVIDER_RATE_LIMITED"
    ) || (matches!(
        error.code,
        "LLM_QUOTA_EXHAUSTED" | "PROVIDER_QUOTA_EXHAUSTED"
    ) && message_mentions_soft_rate_limit(&error.message))
}

fn message_mentions_soft_rate_limit(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "rpm",
        "qps",
        "tpm",
        "rate limit",
        "rate_limit",
        "too many request",
        "requests per minute",
        "request per minute",
        "tokens per minute",
        "request rate",
        "frequency limit",
        "concurrency limit",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn is_concurrency_limit_error(error: &AppError) -> bool {
    let message = error.message.to_ascii_lowercase();
    message.contains("concurrency limit")
        || message.contains("concurrent request")
        || message.contains("parallel request")
        || message.contains("too many concurrent")
}

#[cfg(test)]
pub(super) fn configure_llm_route_profile_store(path: &Path) -> Result<usize, AppError> {
    configure_llm_route_profile_stores(&[path.to_path_buf()])
}

pub(super) fn configure_llm_route_profile_stores(paths: &[PathBuf]) -> Result<usize, AppError> {
    let unique_paths = unique_profile_paths(paths);
    set_profile_store_paths(unique_paths.clone());
    let mut loaded = 0usize;
    for path in &unique_paths {
        loaded = loaded.saturating_add(load_profiles_from_store(path)?);
    }
    Ok(loaded)
}

// B 方案：新会话的调度决策只依据**本次运行实测**，不继承历史画像。
// 历史画像仍加载（供 RPM 初值/断点续跑），但 has_learning / p95 / throughput / health
// 在调度侧一律按"无学习数据"处理，让所有路由从同一起点冷启动；
// 这样已移除/已漂移的旧模型画像不会压制新路由，也消除跨任务的全局脏数据耦合。
pub(super) fn reset_route_profiles_for_fresh_run() {
    if let Ok(registry) = limiter_registry().lock() {
        for limiter in registry.values() {
            if let Ok(mut state) = limiter.state.lock() {
                state.latency_samples_ms.clear();
                state.throughput_ewma_rpm = 0.0;
                state.success_count = 0;
                state.retryable_error_count = 0;
                state.rate_limit_count = 0;
                state.concurrency_limit_count = 0;
                state.rate_limit_streak = 0;
                state.cooldown_until = None;
                state.first_latency_recorded = false;
            }
        }
    }
}

pub(super) async fn wait_for_llm_rate_slot(
    routing_key: &ProviderRoutingKey,
    token: &CancellationToken,
) -> Result<LlmRoutePermit, AppError> {
    loop {
        if token.is_cancelled() {
            return Err(AppError::cancelled(OPERATION_CANCELLED_MESSAGE));
        }
        let limiter = adaptive_llm_rate_limiter(routing_key);
        match limiter.reserve_slot() {
            RouteReserve::Reserved(wait) => {
                let permit = LlmRoutePermit {
                    limiter: Some(limiter),
                };
                if wait.is_zero() {
                    return Ok(permit);
                }
                tokio::select! {
                    _ = sleep(wait) => return Ok(permit),
                    _ = token.cancelled() => {
                        drop(permit);
                        return Err(AppError::cancelled(OPERATION_CANCELLED_MESSAGE));
                    }
                }
            }
            RouteReserve::RetryAfter(wait) => {
                tokio::select! {
                    _ = sleep(wait) => {}
                    _ = token.cancelled() => {
                        return Err(AppError::cancelled(OPERATION_CANCELLED_MESSAGE));
                    }
                }
            }
        }
    }
}

pub(super) fn record_llm_success(routing_key: &ProviderRoutingKey, request_elapsed_ms: u128) {
    adaptive_llm_rate_limiter(routing_key).record_success(request_elapsed_ms);
    persist_profiles_to_configured_store();
}

pub(super) fn record_llm_retryable_error(
    routing_key: &ProviderRoutingKey,
    error: &AppError,
    request_elapsed_ms: u128,
) {
    if is_rate_limit_error(error) {
        // Retry-After 透传：服务端提示的等待时间优先于内部冷却公式。
        let retry_after_hint = error.retry_after_ms.map(Duration::from_millis);
        adaptive_llm_rate_limiter(routing_key).record_rate_limit_with_hint(
            request_elapsed_ms,
            is_concurrency_limit_error(error),
            retry_after_hint,
        );
        persist_profiles_to_configured_store();
    } else if is_content_quality_error(error) {
        adaptive_llm_rate_limiter(routing_key).record_content_quality_error(request_elapsed_ms);
        persist_profiles_to_configured_store();
    }
}

// 内容质量硬错误（译文乱码/替换符）：译出的文本本身不可信，需给路由健康度负反馈
// 让重试换模型，但不触发限流退避（该路由的速度/配额没坏，坏的是产出文本）。
fn is_content_quality_error(error: &AppError) -> bool {
    error.code == "TRANSLATION_ENCODING_POLLUTION"
}

pub(super) fn route_profile_recommended_parallel(
    routing_key: &ProviderRoutingKey,
    configured_parallel: usize,
) -> usize {
    adaptive_llm_rate_limiter(routing_key)
        .recommended_parallel()
        .min(configured_parallel.max(1))
        .max(1)
}

// ---- 号池自适应调度（供应商/模型无关）----
// 设计原则：把"分配权重（谁更优谁多扛）"与"限流配额（谁被限谁让位）"彻底分开。
// - 分配权重 = 健康度 × 速度：健康度（近期成功率，连续值）反映某时段稳定性，
//   速度（1/p95_latency）反映快慢；两者相乘，让"又快又稳"的路由多扛，
//   "慢或不稳定"的少扛——但不摘除，保留恢复期（时段性波动会自然回升）。
// - 429/404/5xx/超时都纳入自适应计算：成功刷新健康度、失败按类型降健康度
//   （429 降最多、5xx/超时次之、404/硬错误最少——硬错误可能只是个别请求不兼容）。
// - 冷启动：无 latency 数据时给"池内中位速度"作为初始权重，让新路由能分到
//   流量去实测 latency，跑几个请求后即切换到实测速度——自动收敛，无需暖机。
// - 重试链路：某 route 失败时先按退避重试，仍失败则故障转移到池内次优 route，
//   而不是让任务死掉（per agnespool-001 的 inkling 404 实证）。

// 单路由的调度画像（健康度 × 速度）。
#[derive(Debug, Clone, Copy)]
pub(super) struct RouteSchedulingProfile {
    pub weight: f64,
    /// 冷却态标记（测试断言用；生产降权已折入 weight）。
    #[cfg(test)]
    pub in_cooldown: bool,
}

// 近期健康度（0..1）：success/(success+失败)，失败按类型加权（429 权重最高）。
// 用 latency 样本窗口近似"近期"；冷启动返回 1.0（乐观，让新路由有机会）。
pub(super) fn route_profile_scheduling_profile(
    routing_key: &ProviderRoutingKey,
) -> Option<RouteSchedulingProfile> {
    let limiter = adaptive_llm_rate_limiter(routing_key);
    let (p95_ms, health, in_cooldown, has_learning) = limiter
        .state
        .lock()
        .map(|state| {
            let health = compute_route_health(&state);
            (
                percentile_latency(&state.latency_samples_ms, 0.95),
                health,
                state
                    .cooldown_until
                    .is_some_and(|until| Instant::now() < until),
                state.success_count > 0 || state.retryable_error_count > 0,
            )
        })
        .unwrap_or((0, 1.0, false, false));
    if !has_learning || p95_ms == 0 {
        return None;
    }
    // 分配权重 = 健康度 × 速度。健康度低（时段性不稳定）的路由权重降，
    // 但不归零——保留恢复期；冷却期内进一步降权但不摘除（重试链路接管）。
    let speed = 1_000_000.0 / p95_ms as f64;
    let cooldown_factor = if in_cooldown { 0.25 } else { 1.0 };
    Some(RouteSchedulingProfile {
        weight: (health * speed * cooldown_factor).max(f64::MIN_POSITIVE),
        #[cfg(test)]
        in_cooldown,
    })
}

// 计算路由健康度（0.05..1.0）：近期成功率的连续度量。
// 429/限流错误权重最高（×3），5xx/超时次之（×2），404/硬错误最少（×1）。
fn compute_route_health(state: &AdaptiveLlmRateState) -> f64 {
    let success = state.success_count as f64;
    if success <= 0.0 {
        return 1.0; // 冷启动乐观
    }
    let hard_errors = (state.retryable_error_count - state.rate_limit_count).max(0) as f64;
    let rate_limit_errors = state.rate_limit_count as f64;
    let weighted_errors = rate_limit_errors * 3.0 + hard_errors * 1.0;
    // 健康度 = success / (success + 加权失败)，下限 0.05（保留恢复通道）
    let health = success / (success + weighted_errors.max(0.0));
    health.clamp(0.05, 1.0)
}

// ---- 号池最优分配（边际吞吐最优 / water-filling over speed-ordered routes）----
// 用户拍板的原则：快的优先用满；慢模型只在快模型的并发或限流不够、且加入
// 能有效提速时才加入。这是该原则的精确数学实现（详见
// working/10-active-blueprints/pool-optimal-allocation-algorithm-20260729.md）。

// 每条路由：c_i = min(learned_rpm, recommended_parallel × 60/p95_latency) × health
//   —— 稳定吞吐上限（req/min），受 RPM 配额、并发×速度、健康度三重约束。
// 按速度 s_i = 60/p95_latency 降序，water-filling：x_i = min(c_i, max(0, D-Σ_{j<i} c_j))。
// 慢路由仅在 Σ_{j<k} c_j < D（快路由稳定吞吐总和不够）时才有非零分配。

// 单路由的稳定吞吐上限（req/min）与速度（req/min/并发槽）。
// 关键修正：capacity 用**实测吞吐速率**（近窗口完成速率），不是理论并发推算
// （recommended_parallel 由 latency 推算，慢的被推高导致"越慢越给"恶性循环）。
#[derive(Debug, Clone, Copy)]
pub(super) struct RouteCapacity {
    pub speed_rpm: f64,            // s_i = 60/p95_latency（单并发速度）
    pub capacity_rpm: f64,         // c_i = min(rpm, 实测吞吐速率) × health（稳定吞吐上限）
    /// 有效并发槽（测试基准用；生产调度只消费 speed/capacity）。
    #[cfg(test)]
    pub effective_parallel: usize, // p_i = 实测吞吐 / 单并发速度（不排队能扛几个）
    pub has_learning: bool,        // 是否有学习数据（无则冷启动）
}

pub(super) fn route_profile_capacity(routing_key: &ProviderRoutingKey) -> RouteCapacity {
    let limiter = adaptive_llm_rate_limiter(routing_key);
    // B 方案补充：has_learning 只要有**任何真实延迟样本**即成立（探测流量首跑即得），
    // 不再要求 success/error 计数——探测请求一旦返回，立刻转入实测容量/速度估算，
    // 而非继续按冷启动水位被压制。
    let (p95_ms, rpm, throughput, health, has_learning) = limiter
        .state
        .lock()
        .map(|state| {
            let has_sample = !state.latency_samples_ms.is_empty()
                || state.success_count > 0
                || state.retryable_error_count > 0;
            (
                percentile_latency(&state.latency_samples_ms, 0.95),
                state.current_rpm,
                state.throughput_ewma_rpm,
                compute_route_health(&state),
                has_sample,
            )
        })
        .unwrap_or((0, 0.0, 0.0, 1.0, false));
    if !has_learning || p95_ms == 0 {
        return RouteCapacity {
            speed_rpm: 0.0,
            capacity_rpm: 0.0,
            #[cfg(test)]
            effective_parallel: 0,
            has_learning: false,
        };
    }
    let speed = 60_000.0 / p95_ms as f64; // req/min/并发槽
                                          // 实测吞吐：EWMA 近窗口速率；冷启动/无样本时回退到 rpm（learned_rpm）
    let measured_throughput = if throughput > 0.0 {
        throughput
    } else {
        rpm.max(0.0)
    };
    // 稳定吞吐上限 = min(RPM 配额, 实测吞吐) × 健康度
    let capacity = (rpm.max(0.0).min(measured_throughput)) * health;
    // 有效并发槽 = 实测吞吐 / 单并发速度（不排队能同时扛几个），下限 1
    #[cfg(test)]
    let effective_parallel = (measured_throughput / speed.max(f64::MIN_POSITIVE))
        .ceil()
        .max(1.0) as usize;
    RouteCapacity {
        speed_rpm: speed,
        capacity_rpm: capacity.max(f64::MIN_POSITIVE),
        #[cfg(test)]
        effective_parallel,
        has_learning: true,
    }
}

// 慢路由加入的"有效提速"约束：其速度不得低于已在池最快路由速度的 ρ 倍。
// 慢得离谱的路由（如 inkling 比 agnes 慢 6 倍）即使"不够"也不加入主翻译。
// 注意：此实现当前仅被 sim 测试使用（生产路径在 llm.rs::select_water_filling_route），
// 保留作为「需求额度分配」的参考实现与测试基准。
#[cfg(test)]
const SLOW_ROUTE_MIN_SPEED_RATIO: f64 = 0.2;

// water-filling 最优分配（供应商/模型无关）。
// 输入：每条 route 的 (routing_keys, 冷启动回退容量)，总需求 demand_rpm。
// 输出：每条 route 应分配的 req/min 额度；慢路由仅在快路由不够且满足速度比时才有非零值。
// 注意：此实现当前仅被 sim 测试使用（生产路径在 llm.rs::select_water_filling_route），
// 保留作为「需求额度分配」的参考实现与测试基准。
#[cfg(test)]
pub(super) fn water_filling_allocation(
    routes: &[(Vec<ProviderRoutingKey>, f64)], // (routing_keys, cold_start_capacity_rpm)
    demand_rpm: f64,
) -> Vec<f64> {
    // 1. 求每条 route 的容量与速度（跨其所有 key 聚合）。
    let mut entries: Vec<(usize, f64, f64)> = routes
        .iter()
        .enumerate()
        .map(|(idx, (keys, cold_capacity))| {
            let caps: Vec<RouteCapacity> =
                keys.iter().map(|key| route_profile_capacity(key)).collect();
            let has_learning = caps.iter().any(|c| c.has_learning);
            let (capacity, speed) = if has_learning {
                (
                    caps.iter().map(|c| c.capacity_rpm).sum::<f64>(),
                    // 聚合速度：多 key 取最快（容量相加，速度取让该 route 整体最快的度量）
                    caps.iter()
                        .map(|c| c.speed_rpm)
                        .fold(0.0_f64, |a, b| a.max(b)),
                )
            } else {
                (*cold_capacity, *cold_capacity)
            };
            (idx, capacity.max(0.0), speed.max(0.0))
        })
        .collect();
    // 2. 按速度降序（快在前）；速度相同按容量降序。
    entries.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let fastest_speed = entries.first().map(|e| e.2).unwrap_or(0.0);
    // 3. water-filling：依次填满快路由，剩余需求才给慢路由；慢得离谱的不加入。
    let mut allocations = vec![0.0_f64; routes.len()];
    let mut remaining = demand_rpm.max(0.0);
    for (idx, capacity, speed) in &entries {
        if remaining <= 0.0 {
            break;
        }
        // 慢路由"有效提速"约束：非最快路由且速度比低于阈值则跳过（不加入主分配）。
        if *speed < fastest_speed * SLOW_ROUTE_MIN_SPEED_RATIO && *idx != entries[0].0 {
            continue;
        }
        let take = capacity.min(remaining);
        if take > 0.0 {
            allocations[*idx] = take;
            remaining -= take;
        }
    }
    allocations
}

// 池内中位速度权重（冷启动试探水位）：用有学习数据的路由的速度权重中位数，
// 让新路由从"池平均水平"起步，而非被有画像的路由压制。
pub(super) fn route_pool_median_speed_weight(routing_keys: &[ProviderRoutingKey]) -> Option<f64> {
    let mut weights = routing_keys
        .iter()
        .filter_map(|key| route_profile_scheduling_profile(key).map(|p| p.weight))
        .filter(|weight| *weight > 0.0)
        .collect::<Vec<_>>();
    if weights.is_empty() {
        return None;
    }
    weights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(weights[weights.len() / 2])
}

fn adaptive_llm_rate_limiter(routing_key: &ProviderRoutingKey) -> AdaptiveLlmRateLimiter {
    let key = profile_key(routing_key);
    limiter_registry()
        .lock()
        .ok()
        .and_then(|mut state| {
            let entry = state
                .entry(key)
                .or_insert_with(AdaptiveLlmRateLimiter::from_env);
            Some(entry.clone())
        })
        .unwrap_or_else(AdaptiveLlmRateLimiter::from_env)
}

fn limiter_registry() -> &'static Mutex<BTreeMap<String, AdaptiveLlmRateLimiter>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, AdaptiveLlmRateLimiter>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn profile_store_paths() -> &'static Mutex<Vec<PathBuf>> {
    static STORE_PATHS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    STORE_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

fn set_profile_store_paths(paths: Vec<PathBuf>) {
    if let Ok(mut store_paths) = profile_store_paths().lock() {
        *store_paths = paths;
    }
}

fn load_profiles_from_store(path: &Path) -> Result<usize, AppError> {
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(path).map_err(|error| {
        AppError::internal(format!(
            "read LLM route profile file failed({}): {error}",
            path.display()
        ))
    })?;
    let document: LlmRouteProfileDocument = serde_json::from_str(&content).map_err(|error| {
        AppError::internal(format!(
            "parse LLM route profile file failed({}): {error}",
            path.display()
        ))
    })?;
    let mut loaded = 0usize;
    if let Ok(mut registry) = limiter_registry().lock() {
        for profile in document.routes {
            let Some(routing_key) = profile.to_routing_key() else {
                continue;
            };
            registry.insert(
                profile_key(&routing_key),
                AdaptiveLlmRateLimiter::from_profile(profile),
            );
            loaded += 1;
        }
    }
    Ok(loaded)
}

fn persist_profiles_to_configured_store() {
    // 节流：距上次 persist ≥5s 且 dirty 才落盘，避免每请求 2 次全量文件写。
    // 崩溃时最多丢 5s 的学习数据（profile 是启发式初值，影响极小）。
    // 测试环境（cfg(test)）不节流，保证 route_profiles_persist_to_all_configured_stores
    // 等测试能立即验证落盘行为。
    #[cfg(not(test))]
    {
        static LAST_PERSIST: OnceLock<Mutex<Option<StdInstant>>> = OnceLock::new();
        let last_persist = LAST_PERSIST.get_or_init(|| Mutex::new(None));
        {
            let mut guard = match last_persist.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let now = StdInstant::now();
            if let Some(last) = *guard {
                if now.duration_since(last) < Duration::from_secs(5) {
                    return;
                }
            }
            *guard = Some(now);
        }
    }
    let paths = profile_store_paths()
        .lock()
        .ok()
        .map(|paths| paths.clone())
        .unwrap_or_default();
    for path in paths {
        let _ = persist_profiles_to_store(&path);
    }
}

fn unique_profile_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if path.as_os_str().is_empty() || unique.iter().any(|item| item == path) {
            continue;
        }
        unique.push(path.clone());
    }
    unique
}

fn persist_profiles_to_store(path: &Path) -> Result<(), AppError> {
    let routes = limiter_registry()
        .lock()
        .ok()
        .map(|registry| {
            registry
                .iter()
                .filter_map(|(key, limiter)| limiter.snapshot(key))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let document = LlmRouteProfileDocument {
        version: 1,
        updated_at: Utc::now().to_rfc3339(),
        routes,
    };
    let payload = serde_json::to_string_pretty(&document).map_err(|error| {
        AppError::internal(format!("serialize LLM route profiles failed: {error}"))
    })?;
    atomic_write_profile(path, &payload).map_err(|error| {
        AppError::internal(format!(
            "write LLM route profiles failed({}): {error}",
            path.display()
        ))
    })
}

fn atomic_write_profile(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("llm_route_profiles.json");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.tmp",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&temp_path, content.as_bytes())?;
    let rename_result = std::fs::rename(&temp_path, path);
    if rename_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    rename_result
}

fn profile_key(routing_key: &ProviderRoutingKey) -> String {
    format!(
        "{}|{}|{}",
        routing_key.provider, routing_key.model, routing_key.key_fingerprint
    )
}

fn split_profile_key(key: &str) -> Option<ProviderRoutingKey> {
    let mut parts = key.split('|');
    let provider = parts.next()?.to_string();
    let model = parts.next()?.to_string();
    let key_fingerprint = parts.next()?.to_string();
    if parts.next().is_some() {
        return None;
    }
    Some(ProviderRoutingKey {
        provider,
        model,
        key_fingerprint,
    })
}

pub(super) struct LlmRoutePermit {
    limiter: Option<AdaptiveLlmRateLimiter>,
}

impl Drop for LlmRoutePermit {
    fn drop(&mut self) {
        if let Some(limiter) = self.limiter.take() {
            limiter.release();
        }
    }
}

enum RouteReserve {
    Reserved(Duration),
    RetryAfter(Duration),
}

#[derive(Clone)]
struct AdaptiveLlmRateLimiter {
    state: Arc<Mutex<AdaptiveLlmRateState>>,
}

struct AdaptiveLlmRateState {
    current_rpm: f64,
    min_rpm: f64,
    max_rpm: f64,
    next_slot_at: Instant,
    cooldown_until: Option<Instant>,
    success_since_probe: u32,
    rate_limit_streak: u32,
    active_requests: usize,
    recommended_parallel: usize,
    max_parallel: usize,
    success_count: u64,
    rate_limit_count: u64,
    retryable_error_count: u64,
    concurrency_limit_count: u64,
    latency_samples_ms: Vec<u128>,
    // 实测吞吐速率（req/min，近窗口）：用于替代理论并发推算的分配容量。
    // 每次成功/失败都推进 EWMA；初值 0（冷启动）。
    throughput_ewma_rpm: f64,
    last_outcome_at: Option<Instant>,
    updated_at: String,
    // 首次真实延迟样本时刻：B 方案冷启动探测路由一旦拿到首个 p95 样本，
    // 调度即可用它做精确容量/速度估算，替代冷启动回退水位。
    first_latency_recorded: bool,
}

impl AdaptiveLlmRateLimiter {
    fn from_env() -> Self {
        let min_rpm = read_rpm_env("MUSETRANSLATE_LLM_MIN_RPM", DEFAULT_MIN_LLM_RPM);
        let max_rpm = read_rpm_env("MUSETRANSLATE_LLM_MAX_RPM", DEFAULT_MAX_LLM_RPM).max(min_rpm);
        let initial_rpm = read_rpm_env("MUSETRANSLATE_LLM_INITIAL_RPM", DEFAULT_INITIAL_LLM_RPM)
            .clamp(min_rpm, max_rpm);
        Self::from_state(AdaptiveLlmRateState {
            current_rpm: initial_rpm,
            min_rpm,
            max_rpm,
            next_slot_at: Instant::now(),
            cooldown_until: None,
            success_since_probe: 0,
            rate_limit_streak: 0,
            active_requests: 0,
            recommended_parallel: DEFAULT_ROUTE_PARALLEL,
            max_parallel: read_parallel_env(),
            success_count: 0,
            rate_limit_count: 0,
            retryable_error_count: 0,
            concurrency_limit_count: 0,
            latency_samples_ms: Vec::new(),
            throughput_ewma_rpm: 0.0,
            last_outcome_at: None,
            updated_at: Utc::now().to_rfc3339(),
            first_latency_recorded: false,
        })
    }

    fn from_profile(profile: LlmRouteRuntimeProfile) -> Self {
        let min_rpm = read_rpm_env("MUSETRANSLATE_LLM_MIN_RPM", DEFAULT_MIN_LLM_RPM);
        let max_rpm = read_rpm_env("MUSETRANSLATE_LLM_MAX_RPM", DEFAULT_MAX_LLM_RPM).max(min_rpm);
        let mut latency_samples_ms = Vec::new();
        if profile.observed_p95_latency_ms > 0 {
            latency_samples_ms.push(profile.observed_p95_latency_ms);
        }
        Self::from_state(AdaptiveLlmRateState {
            current_rpm: profile.learned_rpm.clamp(min_rpm, max_rpm),
            min_rpm,
            max_rpm,
            next_slot_at: Instant::now(),
            cooldown_until: None,
            success_since_probe: 0,
            rate_limit_streak: 0,
            active_requests: 0,
            recommended_parallel: profile.recommended_parallel.max(1),
            max_parallel: read_parallel_env(),
            success_count: profile.success_count,
            rate_limit_count: profile.rate_limit_count,
            retryable_error_count: profile.retryable_error_count,
            concurrency_limit_count: profile.concurrency_limit_count,
            latency_samples_ms,
            throughput_ewma_rpm: 0.0,
            last_outcome_at: None,
            updated_at: profile.updated_at,
            first_latency_recorded: false,
        })
    }

    fn from_state(state: AdaptiveLlmRateState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn reserve_slot(&self) -> RouteReserve {
        let Ok(mut state) = self.state.lock() else {
            return RouteReserve::Reserved(Duration::ZERO);
        };
        // 冷却期恢复探测（pause-and-retry 单探测语义）：冷却结束后第一个请求
        // 独享路由（active_requests=0 时才放行），探测成功前不允许并发涌入，
        // 避免"冷却一结束 N 个等待者同时打满又被 429"的振荡。
        let now = Instant::now();
        if let Some(until) = state.cooldown_until {
            if now < until {
                return RouteReserve::RetryAfter(until.saturating_duration_since(now));
            }
            // 冷却刚结束：只允许一个探测请求进入（其余继续等）。
            if state.active_requests > 0 {
                return RouteReserve::RetryAfter(ROUTE_CAPACITY_POLL_DELAY);
            }
            state.cooldown_until = None;
        }
        // 并发上限优先用实测吞吐反推的 effective_parallel（"不排队能同时扛几个"），
        // 避免 recommended_parallel（latency 正推）让慢思考型路由被推高、占着大量
        // 并发槽干等，而快路由反而拿不到并发。冷启动（无学习数据）回退 recommended_parallel。
        let learned_parallel = learned_effective_parallel(&state);
        let parallel_cap = learned_parallel.unwrap_or_else(|| state.recommended_parallel.max(1));
        if state.active_requests >= parallel_cap {
            return RouteReserve::RetryAfter(ROUTE_CAPACITY_POLL_DELAY);
        }
        let spacing = rpm_spacing(state.current_rpm);
        let slot_at = state.next_slot_at.max(now);
        state.next_slot_at = slot_at + spacing;
        state.active_requests = state.active_requests.saturating_add(1);
        RouteReserve::Reserved(slot_at.saturating_duration_since(now))
    }

    fn release(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.active_requests = state.active_requests.saturating_sub(1);
        state.updated_at = Utc::now().to_rfc3339();
    }

    fn recommended_parallel(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.recommended_parallel.max(1))
            .unwrap_or(DEFAULT_ROUTE_PARALLEL)
    }

    fn record_success(&self, request_elapsed_ms: u128) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.success_count = state.success_count.saturating_add(1);
        push_latency_sample(&mut state.latency_samples_ms, request_elapsed_ms);
        state.first_latency_recorded = true;
        record_outcome_throughput(&mut state, true);
        refresh_recommended_parallel(&mut state);
        let now = Instant::now();
        if state.cooldown_until.is_some_and(|until| now < until) {
            state.updated_at = Utc::now().to_rfc3339();
            return;
        }
        state.cooldown_until = None;
        state.success_since_probe = state.success_since_probe.saturating_add(1);
        let probe_interval = success_probe_interval(state.current_rpm, state.max_rpm);
        if state.success_since_probe < probe_interval {
            state.updated_at = Utc::now().to_rfc3339();
            return;
        }
        state.success_since_probe = 0;
        state.rate_limit_streak = 0;
        let probe_step = success_probe_step(state.current_rpm, state.max_rpm);
        state.current_rpm = (state.current_rpm + probe_step).min(state.max_rpm);
        refresh_recommended_parallel(&mut state);
        state.updated_at = Utc::now().to_rfc3339();
    }

    /// 限流负反馈：限流命中一律记录（供自适应限速器消费）；
    /// `retry_after_hint` 携带服务端的 Retry-After 时附加冷却语义。
    #[cfg(test)]
    fn record_rate_limit(&self, request_elapsed_ms: u128, concurrency_limited: bool) {
        self.record_rate_limit_with_hint(request_elapsed_ms, concurrency_limited, None);
    }

    /// 限流负反馈：retry_after_hint 来自服务端 Retry-After 头（优先于内部
    /// 指数冷却公式——服务端最清楚何时放行）。冷却期内路由整体暂停
    /// （pause），恢复时由 reserve_slot 的单探测语义控制重进速率。
    fn record_rate_limit_with_hint(
        &self,
        request_elapsed_ms: u128,
        concurrency_limited: bool,
        retry_after_hint: Option<Duration>,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.retryable_error_count = state.retryable_error_count.saturating_add(1);
        state.rate_limit_count = state.rate_limit_count.saturating_add(1);
        push_latency_sample(&mut state.latency_samples_ms, request_elapsed_ms);
        state.first_latency_recorded = true;
        record_outcome_throughput(&mut state, false);
        let now = Instant::now();
        state.rate_limit_streak = state.rate_limit_streak.saturating_add(1);
        let backoff = RATE_LIMIT_BACKOFF_FACTOR.powi(state.rate_limit_streak.min(4) as i32);
        state.current_rpm = (state.current_rpm * backoff).max(state.min_rpm);
        if concurrency_limited {
            state.concurrency_limit_count = state.concurrency_limit_count.saturating_add(1);
            state.recommended_parallel =
                ((state.recommended_parallel as f64) * CONCURRENCY_BACKOFF_FACTOR).floor() as usize;
            state.recommended_parallel = state.recommended_parallel.max(1);
        } else {
            refresh_recommended_parallel(&mut state);
        }
        // Retry-After 优先：服务端给的等待时间比我们的指数公式更准。
        // 仍夹在 [BASE, MAX] 区间内防服务端给出荒谬值。
        let cooldown = retry_after_hint
            .unwrap_or_else(|| rate_limit_cooldown(state.rate_limit_streak))
            .clamp(RATE_LIMIT_BASE_COOLDOWN, RATE_LIMIT_MAX_COOLDOWN);
        state.cooldown_until = Some(now + cooldown);
        state.success_since_probe = 0;
        state.next_slot_at = state.next_slot_at.max(now + cooldown);
        state.updated_at = Utc::now().to_rfc3339();
    }

    // 内容质量失败的负反馈：译文乱码（含 U+FFFD 替换符）等**内容型硬错误**。

    // 为什么单独开一条而不复用 record_rate_limit：内容失败与限流是两种病理。
    // 限流需要「降 RPM + 冷却退避」；内容失败（如某模型稀疏丢字）该路由的
    // 速度与配额其实没问题，问题在**它产出的文本不可信**——所以只需把它在
    // 健康度里的权重打低，让 water-filling 在重试时自然漂移到其他模型，
    // 而不该误伤它的 RPM/并发配额（否则会造成「因为丢了一个字符就把整条
    // 路由限流到爬行」的过度反应）。

    // 效果：retryable_error_count +1 → compute_route_health 的 hard_errors 加权 →
    // 健康度下降 → 分配权重下降 → 后续重试/新块被分配到其他模型（换模型重翻）。
    // 不触发冷却（cooldown_until 不动），保留该路由的恢复期与后续配额。
    fn record_content_quality_error(&self, request_elapsed_ms: u128) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.retryable_error_count = state.retryable_error_count.saturating_add(1);
        push_latency_sample(&mut state.latency_samples_ms, request_elapsed_ms);
        state.first_latency_recorded = true;
        record_outcome_throughput(&mut state, false);
        // 不动 rate_limit_streak / current_rpm / cooldown_until / next_slot_at：
        // 只贡献健康度负反馈，不触发限流式退避。
        state.updated_at = Utc::now().to_rfc3339();
    }

    fn snapshot(&self, key: &str) -> Option<LlmRouteRuntimeProfile> {
        let routing_key = split_profile_key(key)?;
        let state = self.state.lock().ok()?;
        Some(LlmRouteRuntimeProfile {
            provider: routing_key.provider,
            model: routing_key.model,
            key_fingerprint: routing_key.key_fingerprint,
            learned_rpm: round_profile_float(state.current_rpm),
            recommended_parallel: state.recommended_parallel.max(1),
            active_requests: state.active_requests,
            observed_p95_latency_ms: percentile_latency(&state.latency_samples_ms, 0.95),
            success_count: state.success_count,
            rate_limit_count: state.rate_limit_count,
            retryable_error_count: state.retryable_error_count,
            concurrency_limit_count: state.concurrency_limit_count,
            throughput_ewma_rpm: state.throughput_ewma_rpm,
            updated_at: state.updated_at.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmRouteProfileDocument {
    version: u8,
    updated_at: String,
    routes: Vec<LlmRouteRuntimeProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmRouteRuntimeProfile {
    provider: String,
    model: String,
    key_fingerprint: String,
    learned_rpm: f64,
    recommended_parallel: usize,
    #[serde(default)]
    active_requests: usize,
    #[serde(default)]
    observed_p95_latency_ms: u128,
    #[serde(default)]
    success_count: u64,
    #[serde(default)]
    rate_limit_count: u64,
    #[serde(default)]
    retryable_error_count: u64,
    #[serde(default)]
    concurrency_limit_count: u64,
    // 实测吞吐速率（req/min，EWMA）。持久化以便跨任务 warmup 后直接可用，
    // 避免每次启动都从零 warmup（冷启动期只能靠 learned_rpm 回退）。
    #[serde(default)]
    throughput_ewma_rpm: f64,
    updated_at: String,
}

impl LlmRouteRuntimeProfile {
    fn to_routing_key(&self) -> Option<ProviderRoutingKey> {
        if self.provider.trim().is_empty()
            || self.model.trim().is_empty()
            || self.key_fingerprint.trim().is_empty()
        {
            return None;
        }
        Some(ProviderRoutingKey {
            provider: self.provider.clone(),
            model: self.model.clone(),
            key_fingerprint: self.key_fingerprint.clone(),
        })
    }
}

fn read_parallel_env() -> usize {
    std::env::var("MUSETRANSLATE_LLM_MAX_ROUTE_PARALLEL")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_ROUTE_PARALLEL)
}

fn push_latency_sample(samples: &mut Vec<u128>, elapsed_ms: u128) {
    if elapsed_ms == 0 {
        return;
    }
    samples.push(elapsed_ms);
    if samples.len() > PROFILE_LATENCY_SAMPLE_LIMIT {
        let overflow = samples.len().saturating_sub(PROFILE_LATENCY_SAMPLE_LIMIT);
        samples.drain(0..overflow);
    }
}

// 实测吞吐速率推进（req/min，EWMA）：每次成功/失败都更新。
// 用相邻两次 outcome 的时间间隔估算瞬时速率，再 EWMA 平滑（α=0.3 快速响应变化）。
fn record_outcome_throughput(state: &mut AdaptiveLlmRateState, _success: bool) {
    let now = Instant::now();
    if let Some(last) = state.last_outcome_at {
        let interval_s = now.saturating_duration_since(last).as_secs_f64();
        if interval_s > 0.001 {
            let instant_rpm = 60.0 / interval_s;
            // EWMA：α=0.3（新值权重），快速响应时段性变化，又不过度抖动
            state.throughput_ewma_rpm = if state.throughput_ewma_rpm <= 0.0 {
                instant_rpm
            } else {
                0.7 * state.throughput_ewma_rpm + 0.3 * instant_rpm
            };
        }
    }
    state.last_outcome_at = Some(now);
}

fn refresh_recommended_parallel(state: &mut AdaptiveLlmRateState) {
    let p95_latency_ms = percentile_latency(&state.latency_samples_ms, 0.95);
    if p95_latency_ms == 0 {
        state.recommended_parallel = state.recommended_parallel.max(1);
        return;
    }
    let derived = (state.current_rpm * (p95_latency_ms as f64) / 60_000.0).ceil() as usize;
    state.recommended_parallel = derived.clamp(1, state.max_parallel.max(1));
}

// 实测吞吐反推的并发上限（"不排队能同时扛几个"）：与 route_profile_capacity 的
// effective_parallel 同公式（实测吞吐 / 单并发速度），但从已持锁的 state 直接计算，
// 供 reserve_slot 在持锁期间使用。无学习数据返回 None（调用方回退 recommended_parallel）。
fn learned_effective_parallel(state: &AdaptiveLlmRateState) -> Option<usize> {
    let has_learning = state.success_count > 0 || state.retryable_error_count > 0;
    let p95_ms = percentile_latency(&state.latency_samples_ms, 0.95);
    if !has_learning || p95_ms == 0 {
        return None;
    }
    let speed = 60_000.0 / p95_ms as f64; // req/min/并发槽
    if speed <= 0.0 {
        return None;
    }
    let throughput = if state.throughput_ewma_rpm > 0.0 {
        state.throughput_ewma_rpm
    } else {
        state.current_rpm.max(0.0)
    };
    let derived = (throughput / speed).ceil().max(1.0) as usize;
    Some(derived.min(state.max_parallel.max(1)))
}

fn percentile_latency(samples: &[u128], percentile: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    let mut values = samples.to_vec();
    values.sort_unstable();
    let index = ((values.len() as f64) * percentile.clamp(0.0, 1.0)).ceil() as usize;
    values[index.saturating_sub(1).min(values.len() - 1)]
}

fn round_profile_float(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn read_rpm_env(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

fn rpm_spacing(rpm: f64) -> Duration {
    Duration::from_secs_f64(60.0 / rpm.max(1.0))
}

fn success_probe_interval(current_rpm: f64, max_rpm: f64) -> u32 {
    let saturation = (current_rpm / max_rpm.max(1.0)).clamp(0.0, 1.0);
    let span = SUCCESS_PROBE_MAX_INTERVAL - SUCCESS_PROBE_MIN_INTERVAL;
    SUCCESS_PROBE_MIN_INTERVAL + (f64::from(span) * saturation).round() as u32
}

fn success_probe_step(current_rpm: f64, max_rpm: f64) -> f64 {
    let remaining = (max_rpm - current_rpm).max(0.0);
    (remaining * SUCCESS_PROBE_DISTANCE_FACTOR).clamp(1.0, 8.0)
}

fn rate_limit_cooldown(streak: u32) -> Duration {
    let extra = Duration::from_secs(u64::from(streak.saturating_sub(1)).saturating_mul(30));
    (RATE_LIMIT_BASE_COOLDOWN + extra).min(RATE_LIMIT_MAX_COOLDOWN)
}

pub(super) fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn ensure_not_cancelled(token: &CancellationToken) -> Result<(), AppError> {
    if token.is_cancelled() {
        Err(AppError::cancelled(OPERATION_CANCELLED_MESSAGE))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
