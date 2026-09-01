use super::*;

fn test_limiter(initial_rpm: f64, min_rpm: f64, max_rpm: f64) -> AdaptiveLlmRateLimiter {
    AdaptiveLlmRateLimiter {
        state: Arc::new(Mutex::new(AdaptiveLlmRateState {
            current_rpm: initial_rpm,
            min_rpm,
            max_rpm,
            next_slot_at: Instant::now(),
            cooldown_until: None,
            success_since_probe: 0,
            rate_limit_streak: 0,
            active_requests: 0,
            recommended_parallel: DEFAULT_ROUTE_PARALLEL,
            max_parallel: DEFAULT_MAX_ROUTE_PARALLEL,
            success_count: 0,
            rate_limit_count: 0,
            retryable_error_count: 0,
            concurrency_limit_count: 0,
            latency_samples_ms: Vec::new(),
            throughput_ewma_rpm: 0.0,
            last_outcome_at: None,
            updated_at: Utc::now().to_rfc3339(),
            first_latency_recorded: false,
        })),
    }
}

fn current_rpm(limiter: &AdaptiveLlmRateLimiter) -> f64 {
    limiter
        .state
        .lock()
        .map(|state| state.current_rpm)
        .unwrap_or_default()
}

fn recommended_parallel(limiter: &AdaptiveLlmRateLimiter) -> usize {
    limiter
        .state
        .lock()
        .map(|state| state.recommended_parallel)
        .unwrap_or_default()
}

fn routing_key(provider: &str, model: &str, key_fingerprint: &str) -> ProviderRoutingKey {
    ProviderRoutingKey {
        provider: provider.to_string(),
        model: model.to_string(),
        key_fingerprint: key_fingerprint.to_string(),
    }
}

fn profile_store_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("profile store test lock")
}

#[test]
fn water_filling_fills_fastest_first_slow_only_when_needed() {
    let _guard = profile_store_test_guard();
    // 三条 route：agnes 快、deepseek 中、inkling 慢。
    let agnes = routing_key("agnes", "agnes-2.5-flash", "wf-agnes");
    let deepseek = routing_key("sensenova", "deepseek-v4-flash", "wf-deepseek");
    let inkling = routing_key("nvidia", "thinkingmachines/inkling", "wf-inkling");
    // agnes 快（2s × 高并发），deepseek 中（30s），inkling 慢（150s）
    for _ in 0..40 {
        record_llm_success(&agnes, 2_000);
    }
    for _ in 0..40 {
        record_llm_success(&deepseek, 30_000);
    }
    for _ in 0..40 {
        record_llm_success(&inkling, 150_000);
    }
    let routes = vec![
        (vec![agnes.clone()], 0.0),
        (vec![deepseek.clone()], 0.0),
        (vec![inkling.clone()], 0.0),
    ];
    // 需求小：agnes 单独即可满足 → 其他全 0
    let alloc = water_filling_allocation(&routes, 5.0);
    let agnes_take = alloc[0];
    let deepseek_take = alloc[1];
    let inkling_take = alloc[2];
    assert!(agnes_take > 0.0, "fastest route should be used first");
    assert_eq!(
        deepseek_take, 0.0,
        "slow route not needed when fastest suffices"
    );
    assert_eq!(inkling_take, 0.0, "slowest route not needed");
}

#[test]
fn water_filling_adds_slow_route_only_when_fast_capacity_insufficient() {
    let _guard = profile_store_test_guard();
    let fast = routing_key("agnes", "agnes-2.5-flash", "wf-fast2");
    let slow = routing_key("nvidia", "thinkingmachines/inkling", "wf-slow2");
    // fast 容量有限（少 key/低 rpm），slow 容量大但慢
    for _ in 0..2 {
        record_llm_success(&fast, 2_000);
    }
    for _ in 0..60 {
        record_llm_success(&slow, 60_000);
    }
    let routes = vec![(vec![fast.clone()], 0.0), (vec![slow.clone()], 0.0)];
    // 需求远超 fast 容量 → slow 才被加入（且 slow 速度比 0.4/2.5=0.16 < 0.2 应被 ρ 约束跳过？
    // 这里 slow 60s vs fast 2s，速度比 = (60/60)/(60/2)=1/30≈0.033 < 0.2 → 不加入主分配）
    let alloc = water_filling_allocation(&routes, 1000.0);
    let fast_take = alloc[0];
    let slow_take = alloc[1];
    assert!(fast_take > 0.0);
    // 慢得离谱（速度比<0.2）→ 不加入主翻译分配
    assert_eq!(
        slow_take, 0.0,
        "very slow route should be excluded by speed ratio"
    );
}

#[test]
fn scheduling_weight_returns_none_when_cold_start() {
    let _guard = profile_store_test_guard();
    // 无学习数据（success=0 且 rate_limit=0）的新路由 → None（冷启动，回退试探权重）
    let key = routing_key("agnes", "agnes-2.5-flash", "cold-fp-xyz");
    assert!(route_profile_scheduling_profile(&key).is_none());
}

#[test]
fn scheduling_weight_is_speed_inverse_of_latency() {
    let _guard = profile_store_test_guard();
    let fast = routing_key("agnes", "agnes-2.5-flash", "fast-fp");
    let slow = routing_key("nvidia", "thinkingmachines/inkling", "slow-fp");
    // fast: 40 个 2s 请求；slow: 40 个 80s 请求
    for _ in 0..40 {
        record_llm_success(&fast, 2_000);
        record_llm_success(&slow, 80_000);
    }
    let fast_weight = route_profile_scheduling_profile(&fast)
        .expect("fast should have weight")
        .weight;
    let slow_weight = route_profile_scheduling_profile(&slow)
        .expect("slow should have weight")
        .weight;
    // 速度权重 = 1/latency，快路由应显著大于慢路由
    assert!(
        fast_weight > slow_weight * 10.0,
        "faster route should dominate: fast={fast_weight} slow={slow_weight}"
    );
}

#[test]
fn scheduling_weight_drops_but_does_not_kill_unstable_route() {
    let _guard = profile_store_test_guard();
    let warm = routing_key("sensenova", "deepseek-v4-flash", "rl-fp-1");
    let cold = routing_key("agnes", "agnes-2.5-flash", "rl-fp-2");
    for _ in 0..40 {
        record_llm_success(&warm, 2_000);
    }
    let before = route_profile_scheduling_profile(&warm)
        .map(|p| p.weight)
        .unwrap_or(0.0);
    // 连续 429：健康度降权（不摘除，保留恢复期）
    let error = AppError::new("LLM_RATE_LIMITED", "429 rpm", true);
    for _ in 0..3 {
        record_llm_retryable_error(&warm, &error, 300);
    }
    let after = route_profile_scheduling_profile(&warm)
        .map(|p| p.weight)
        .unwrap_or(0.0);
    // 权重应下降但不为零（不摘除，时段性波动可恢复）
    assert!(
        after < before && after > 0.0,
        "unstable route should be down-weighted but not killed: before={before} after={after}"
    );
    // 未学习的 cold 仍返回 None（冷启动）
    assert!(route_profile_scheduling_profile(&cold).is_none());
}

#[test]
fn rate_limit_feedback_reduces_current_rpm_and_sets_cooldown() {
    let limiter = test_limiter(30.0, 6.0, 60.0);

    limiter.record_rate_limit(1_000, false);

    assert_eq!(current_rpm(&limiter), 19.5);
    assert!(limiter
        .state
        .lock()
        .expect("limiter state")
        .cooldown_until
        .is_some());
}

#[test]
fn repeated_rate_limits_do_not_drop_below_minimum_rpm() {
    let limiter = test_limiter(7.0, 6.0, 60.0);

    limiter.record_rate_limit(1_000, false);
    limiter.record_rate_limit(1_000, false);

    assert_eq!(current_rpm(&limiter), 6.0);
}

#[test]
fn rpm_quota_message_is_treated_as_rate_limit_feedback() {
    let error = AppError::new(
        "LLM_QUOTA_EXHAUSTED",
        "provider returned rpm exhausted",
        true,
    );

    assert!(is_rate_limit_error(&error));
}

#[test]
fn hard_quota_message_is_not_treated_as_rate_limit_feedback() {
    let error = AppError::new(
        "LLM_QUOTA_EXHAUSTED",
        "provider returned insufficient quota, please recharge",
        true,
    );

    assert!(!is_rate_limit_error(&error));
}

#[test]
fn successful_requests_probe_up_after_stable_window() {
    let limiter = test_limiter(24.0, 6.0, 60.0);

    let interval = success_probe_interval(24.0, 60.0);
    for _ in 0..interval {
        limiter.record_success(10_000);
    }

    assert!(current_rpm(&limiter) > 24.0);
}

#[test]
fn cooldown_blocks_success_probe_until_expired() {
    let limiter = test_limiter(24.0, 6.0, 60.0);
    {
        let mut state = limiter.state.lock().expect("limiter state");
        state.cooldown_until = Some(Instant::now() + Duration::from_secs(60));
    }

    for _ in 0..success_probe_interval(24.0, 60.0) {
        limiter.record_success(10_000);
    }

    assert_eq!(current_rpm(&limiter), 24.0);
}

#[test]
fn reserve_slot_during_cooldown_returns_retry_after_until_expiry() {
    let limiter = test_limiter(24.0, 6.0, 60.0);
    {
        let mut state = limiter.state.lock().expect("limiter state");
        state.cooldown_until = Some(Instant::now() + Duration::from_millis(120));
    }

    match limiter.reserve_slot() {
        RouteReserve::RetryAfter(wait) => {
            assert!(wait > Duration::ZERO, "cooldown wait should be positive");
            assert!(wait <= Duration::from_millis(120));
        }
        RouteReserve::Reserved(_) => panic!("reserve during cooldown must RetryAfter"),
    }
}

#[test]
fn single_probe_semantics_after_cooldown_expiry() {
    // 冷却刚结束：第一个 reserve 放行（探测），在探测释放前第二个 reserve
    // 必须继续 RetryAfter——防止等待者一并涌入再次被 429。
    let limiter = test_limiter(24.0, 6.0, 60.0);
    {
        let mut state = limiter.state.lock().expect("limiter state");
        state.cooldown_until = Some(Instant::now() - Duration::from_millis(1));
    }

    let first = limiter.reserve_slot();
    assert!(
        matches!(first, RouteReserve::Reserved(_)),
        "first reserve after cooldown should probe"
    );
    let second = limiter.reserve_slot();
    assert!(
        matches!(second, RouteReserve::RetryAfter(_)),
        "second reserve while probe in flight must RetryAfter"
    );
    limiter.release();
    let third = limiter.reserve_slot();
    assert!(
        matches!(third, RouteReserve::Reserved(_)),
        "after probe release, reserve should proceed"
    );
}

#[test]
fn retry_after_hint_from_server_overrides_formula_within_bounds() {
    let limiter = test_limiter(24.0, 6.0, 60.0);
    // 服务端 Retry-After: 90s → 冷却应取 90s（介于 BASE=60/MAX=180 之间）。
    limiter.record_rate_limit_with_hint(1_000, false, Some(Duration::from_secs(90)));
    let state = limiter.state.lock().expect("limiter state");
    let remaining = state
        .cooldown_until
        .expect("cooldown should be set")
        .saturating_duration_since(Instant::now());
    assert!(
        remaining > Duration::from_secs(80) && remaining <= Duration::from_secs(90),
        "cooldown should follow server hint ~90s, got {remaining:?}"
    );
    drop(state);

    // 荒谬值（10 小时）被夹到 MAX=180s。
    let limiter2 = test_limiter(24.0, 6.0, 60.0);
    limiter2.record_rate_limit_with_hint(1_000, false, Some(Duration::from_secs(36_000)));
    let state2 = limiter2.state.lock().expect("limiter state");
    let remaining2 = state2
        .cooldown_until
        .expect("cooldown should be set")
        .saturating_duration_since(Instant::now());
    assert!(
        remaining2 <= Duration::from_secs(180),
        "absurd hint should clamp to MAX, got {remaining2:?}"
    );
}

#[test]
fn successful_probe_uses_larger_steps_when_far_from_max() {
    let low = success_probe_step(12.0, 60.0);
    let high = success_probe_step(54.0, 60.0);

    assert!(low > high);
    assert!(low <= 8.0);
    assert!(high >= 1.0);
}

#[test]
fn registry_scopes_rate_state_by_provider_model_and_key() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    let primary_key = routing_key("sensenova", &format!("model-a-{suffix}"), "key-a");
    let other_model_key = routing_key("sensenova", &format!("model-b-{suffix}"), "key-a");
    let other_api_key = routing_key("sensenova", &format!("model-a-{suffix}"), "key-b");

    let primary = adaptive_llm_rate_limiter(&primary_key);
    let other_model = adaptive_llm_rate_limiter(&other_model_key);
    let other_api = adaptive_llm_rate_limiter(&other_api_key);
    let primary_before = current_rpm(&primary);
    let other_model_before = current_rpm(&other_model);
    let other_api_before = current_rpm(&other_api);

    primary.record_rate_limit(1_000, false);

    assert!(current_rpm(&primary) < primary_before);
    assert_eq!(current_rpm(&other_model), other_model_before);
    assert_eq!(current_rpm(&other_api), other_api_before);
}

#[test]
fn latency_feedback_updates_recommended_parallel() {
    let limiter = test_limiter(60.0, 6.0, 120.0);

    limiter.record_success(20_000);

    assert!(recommended_parallel(&limiter) >= 20);
}

#[test]
fn concurrency_limit_feedback_reduces_parallel_hint() {
    let limiter = test_limiter(60.0, 6.0, 120.0);
    limiter.record_success(20_000);
    let before = recommended_parallel(&limiter);

    limiter.record_rate_limit(1_000, true);

    assert!(recommended_parallel(&limiter) < before);
    assert!(recommended_parallel(&limiter) >= 1);
}

#[test]
fn route_profiles_persist_to_configured_store() {
    let _guard = profile_store_test_guard();
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("musetranslate-route-profile-{suffix}.json"));
    let key = routing_key(
        "nvidia",
        &format!("stepfun-ai/step-3.7-flash-{suffix}"),
        "key-a",
    );

    configure_llm_route_profile_store(&path).expect("profile store should configure");
    record_llm_success(&key, 20_000);

    let content = std::fs::read_to_string(&path).expect("profile should be written");
    assert!(content.contains("stepfun-ai/step-3.7-flash"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn route_profiles_persist_to_all_configured_stores() {
    let _guard = profile_store_test_guard();
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("musetranslate-route-profile-all-{suffix}"));
    let global_path = root.join("global.json");
    let artifact_path = root.join("artifact.json");
    let key = routing_key(
        "nvidia",
        &format!("stepfun-ai/step-3.7-flash-all-{suffix}"),
        "key-a",
    );

    configure_llm_route_profile_stores(&[global_path.clone(), artifact_path.clone()])
        .expect("profile stores should configure");
    record_llm_success(&key, 20_000);

    let global_content =
        std::fs::read_to_string(&global_path).expect("global profile should be written");
    let artifact_content =
        std::fs::read_to_string(&artifact_path).expect("artifact profile should be written");
    assert!(global_content.contains("stepfun-ai/step-3.7-flash"));
    assert!(artifact_content.contains("stepfun-ai/step-3.7-flash"));

    let _ = std::fs::remove_dir_all(root);
}

// ---- 号池分配算法全面模拟测试（sim_sN_*）----
// 纯离线确定性测试：仅通过 record_llm_success / record_llm_retryable_error
// 注入延迟/吞吐/限流画像，断言 water_filling_allocation / route_profile_capacity /
// route_profile_scheduling_profile 的分配行为。不触网络、不耗真实额度。

// 唯一指纹后缀：避免测试间 profile 相互污染。
fn sim_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos()
}

// 用瞬时分配结果换算"每秒平均完成请求数"（req/s）。
// 每次调度视为取 1 个请求：route i 分到 take_i 个请求时，按完成时间
// T = max(take_i / cap_i) 推算整体吞吐 rate = D / T。
fn sim_avg_req_per_sec(allocations: &[f64], capacities: &[f64], demand: f64) -> f64 {
    let mut max_time = 0.0_f64;
    for (take, capacity) in allocations.iter().zip(capacities.iter()) {
        if *take > 0.0 && *capacity > 0.0 {
            let time = *take / *capacity; // 分钟
            if time > max_time {
                max_time = time;
            }
        }
    }
    if max_time <= 0.0 || demand <= 0.0 {
        return 0.0;
    }
    demand / max_time / 60.0
}

fn sim_rate_limit_error() -> AppError {
    AppError::new("LLM_RATE_LIMITED", "429 rpm exceeded", true)
}

// S1 快+慢混合：agnes(2s) + inkling(150s)，需求中等 → 快的先用满，
// 慢得离谱的（速度比 1/75 << 0.2）即使不够也不加入主分配。
#[test]
fn sim_s1_fast_slow_mixed_fast_saturated_slow_only_when_needed() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let agnes = routing_key(
        "agnes",
        &format!("agnes-s1-{suffix}"),
        &format!("fp-s1-a-{suffix}"),
    );
    let inkling = routing_key(
        "nvidia",
        &format!("inkling-s1-{suffix}"),
        &format!("fp-s1-i-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&agnes, 2_000);
    }
    for _ in 0..40 {
        record_llm_success(&inkling, 150_000);
    }
    let routes = vec![(vec![agnes.clone()], 0.0), (vec![inkling.clone()], 0.0)];
    // 需求小（5 < agnes 容量）：agnes 全扛，inkling 0
    let alloc = water_filling_allocation(&routes, 5.0);
    assert!(alloc[0] > 0.0, "fast route should be used first");
    assert_eq!(alloc[1], 0.0, "slow route not needed when fast suffices");
    // 需求中等（8）：可能仍全部落在 agnes（其容量约 12），inkling 只在不够时加；
    // 且 inkling 速度比 = 0.4/30 ≈ 0.013 < 0.2 → 即使不够也被 ρ 约束排除。
    let alloc2 = water_filling_allocation(&routes, 8.0);
    assert!(
        alloc2[0] >= 5.0,
        "fast route should take at least the small demand: {}",
        alloc2[0]
    );
    assert_eq!(
        alloc2[1], 0.0,
        "very slow route (speed ratio << 0.2) must be excluded"
    );
}

// S2 速度快并发不足：agnes 快但 effective_parallel 应反映"不排队能扛多个"
//（effective_parallel = 实测吞吐 / 单并发速度 ≥ 1，且受限于已探明的 RPM 上限）。
#[test]
fn sim_s2_fast_route_effective_parallel_reflects_throughput() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let agnes = routing_key(
        "agnes",
        &format!("agnes-s2-{suffix}"),
        &format!("fp-s2-{suffix}"),
    );
    // 背靠背多次成功：吞吐 EWMA 打到天文数字，effective_parallel = ceil(min(rpm, ewma)/speed)。
    for _ in 0..40 {
        record_llm_success(&agnes, 2_000);
    }
    let capacity = route_profile_capacity(&agnes);
    assert!(capacity.has_learning, "route should have learning data");
    assert!(
        capacity.effective_parallel >= 1,
        "effective_parallel must be >= 1: {}",
        capacity.effective_parallel
    );
    // speed = 60s/2s = 30 req/min/槽；current_rpm 探明到 ~20 → effective_parallel = 1。
    // 关键断言：effective_parallel 由实测吞吐/速度推算，而非拍脑袋的并发配置。
    let speed = capacity.speed_rpm;
    assert!(
        (speed - 30.0).abs() < 1.0,
        "speed should be ~30 req/min for 2s latency: {speed}"
    );
    assert!(
        capacity.capacity_rpm > 0.0,
        "capacity should be positive: {}",
        capacity.capacity_rpm
    );
}

// S3 并发多速度不足：inkling 慢 → 其 capacity 远小于 fast，water_filling 分配少（0）。
#[test]
fn sim_s3_slow_route_capacity_far_below_fast() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let fast = routing_key(
        "agnes",
        &format!("agnes-s3-{suffix}"),
        &format!("fp-s3-f-{suffix}"),
    );
    let slow = routing_key(
        "nvidia",
        &format!("inkling-s3-{suffix}"),
        &format!("fp-s3-s-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&fast, 2_000);
        record_llm_success(&slow, 150_000);
    }
    let fast_capacity = route_profile_capacity(&fast);
    let slow_capacity = route_profile_capacity(&slow);
    assert!(
        slow_capacity.speed_rpm < fast_capacity.speed_rpm / 10.0,
        "slow route speed should be far below fast: fast={} slow={}",
        fast_capacity.speed_rpm,
        slow_capacity.speed_rpm
    );
    let routes = vec![(vec![fast.clone()], 0.0), (vec![slow.clone()], 0.0)];
    let alloc = water_filling_allocation(&routes, 6.0);
    assert!(alloc[0] > 0.0);
    assert_eq!(alloc[1], 0.0, "slow route gets nothing when fast suffices");
}

// S4 中途 429：先积累成功，再连续 429 → weight 下降但 > 0（不摘除）。
#[test]
fn sim_s4_rate_limit_drops_weight_but_not_killed() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let key = routing_key(
        "agnes",
        &format!("agnes-s4-{suffix}"),
        &format!("fp-s4-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&key, 2_000);
    }
    let before = route_profile_scheduling_profile(&key)
        .expect("should have weight after successes")
        .weight;
    let error = sim_rate_limit_error();
    for _ in 0..3 {
        record_llm_retryable_error(&key, &error, 300);
    }
    let after_profile = route_profile_scheduling_profile(&key).expect("must not be killed");
    let after = after_profile.weight;
    assert!(
        after < before,
        "weight should drop after 429s: before={before} after={after}"
    );
    assert!(after > 0.0, "weight must stay positive (not killed)");
    assert!(
        after_profile.in_cooldown,
        "route should be in cooldown after rate limits"
    );
}

// S5 中途 404：连续 404（LLM_PROVIDER_ERROR 走重试链路但 is_rate_limit_error=false
// 不反馈号池）→ 权重不降为 0；与 429 对比，429 降权更重。
#[test]
fn sim_s5_provider_error_404_lighter_than_429() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let key_404 = routing_key(
        "agnes",
        &format!("agnes-s5a-{suffix}"),
        &format!("fp-s5a-{suffix}"),
    );
    let key_429 = routing_key(
        "agnes",
        &format!("agnes-s5b-{suffix}"),
        &format!("fp-s5b-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&key_404, 2_000);
        record_llm_success(&key_429, 2_000);
    }
    let before_404 = route_profile_scheduling_profile(&key_404)
        .expect("weight")
        .weight;
    let before_429 = route_profile_scheduling_profile(&key_429)
        .expect("weight")
        .weight;
    // 404：LLM_PROVIDER_ERROR 不是限流错误 → record_llm_retryable_error 不反馈（无操作）
    let error_404 = AppError::new("LLM_PROVIDER_ERROR", "404 model not found", true);
    for _ in 0..3 {
        record_llm_retryable_error(&key_404, &error_404, 300);
    }
    // 429：健康度 ×3 降权
    let error_429 = sim_rate_limit_error();
    for _ in 0..3 {
        record_llm_retryable_error(&key_429, &error_429, 300);
    }
    let after_404 = route_profile_scheduling_profile(&key_404)
        .expect("404 must not kill route")
        .weight;
    let after_429 = route_profile_scheduling_profile(&key_429)
        .expect("429 must not kill route")
        .weight;
    assert_eq!(
        after_404, before_404,
        "404 (non-rate-limit retryable) must not alter weight"
    );
    assert!(
        after_429 < before_429,
        "429 should drop weight: before={before_429} after={after_429}"
    );
    assert!(
        after_429 < after_404,
        "429 down-weight should be heavier than 404: 404={after_404} 429={after_429}"
    );
}

// S6 时段性波动：稳 → 不稳（429）→ 恢复（成功）→ weight 降后能回升。
#[test]
fn sim_s6_weight_recovers_after_fluctuation() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let key = routing_key(
        "agnes",
        &format!("agnes-s6-{suffix}"),
        &format!("fp-s6-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&key, 2_000);
    }
    let stable = route_profile_scheduling_profile(&key)
        .expect("weight")
        .weight;
    let error = sim_rate_limit_error();
    for _ in 0..3 {
        record_llm_retryable_error(&key, &error, 300);
    }
    let degraded = route_profile_scheduling_profile(&key)
        .expect("still alive")
        .weight;
    assert!(
        degraded < stable,
        "weight should drop during fluctuation: stable={stable} degraded={degraded}"
    );
    // 恢复：继续成功（冷却期内 success 不涨 rpm，但健康度随 success_count 增加而回升）
    for _ in 0..30 {
        record_llm_success(&key, 2_000);
    }
    let recovered = route_profile_scheduling_profile(&key)
        .expect("still alive")
        .weight;
    assert!(
        recovered > degraded,
        "weight should recover after stability returns: degraded={degraded} recovered={recovered}"
    );
}

// S7 冷启动新路由：无 record → route_profile_capacity.has_learning=false，
// water_filling 用 cold_start 回退容量。
#[test]
fn sim_s7_cold_start_uses_fallback_capacity() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let warm = routing_key(
        "agnes",
        &format!("agnes-s7a-{suffix}"),
        &format!("fp-s7a-{suffix}"),
    );
    let cold = routing_key(
        "agnes",
        &format!("agnes-s7b-{suffix}"),
        &format!("fp-s7b-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&warm, 2_000);
    }
    let cold_capacity = route_profile_capacity(&cold);
    assert!(
        !cold_capacity.has_learning,
        "cold route must have has_learning=false"
    );
    assert_eq!(cold_capacity.capacity_rpm, 0.0);
    // water_filling：冷启动 route 用调用方给的 cold_start 回退容量（这里给 4.0）。
    // 冷启动条目 speed=capacity=4.0；速度比 4/30≈0.13 < 0.2 → ρ 约束下不加入主分配，
    // 快路由即使不够也独占（慢得离谱的冷启动路由不被试探性拉进主翻译）。
    let routes = vec![(vec![warm.clone()], 4.0), (vec![cold.clone()], 4.0)];
    let warm_cap = route_profile_capacity(&warm).capacity_rpm;
    let alloc = water_filling_allocation(&routes, warm_cap + 2.0);
    assert!(alloc[0] > 0.0, "warm route takes demand up to its capacity");
    assert_eq!(
        alloc[1], 0.0,
        "cold route (speed ratio 4/30 < 0.2) excluded by rho constraint"
    );
    // 回退容量接近快路由速度时（speed ratio ≥ 0.2）冷启动路由才能分到溢出。
    let routes2 = vec![
        (vec![warm.clone()], warm_cap),
        (vec![cold.clone()], warm_cap),
    ];
    let alloc2 = water_filling_allocation(&routes2, warm_cap + 2.0);
    assert!(alloc2[0] > 0.0);
    assert!(
        alloc2[1] > 0.0,
        "cold route with comparable fallback speed should receive spillover: {}",
        alloc2[1]
    );
    assert!(
        alloc2[1] <= warm_cap,
        "cold route capped by fallback capacity"
    );
}

// S8 多 key 加速：同模型 2 个 key 都高吞吐 → water_filling 的 capacity 是多 key 之和。
#[test]
fn sim_s8_multi_key_capacity_sums() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let model = format!("agnes-s8-{suffix}");
    let key_a = routing_key("agnes", &model, &format!("fp-s8-a-{suffix}"));
    let key_b = routing_key("agnes", &model, &format!("fp-s8-b-{suffix}"));
    for _ in 0..40 {
        record_llm_success(&key_a, 2_000);
        record_llm_success(&key_b, 2_000);
    }
    let cap_a = route_profile_capacity(&key_a).capacity_rpm;
    let cap_b = route_profile_capacity(&key_b).capacity_rpm;
    let routes = vec![(vec![key_a.clone(), key_b.clone()], 0.0)];
    // 需求超过单 key 容量：分配应能用到两 key 之和（> 单 key）
    let demand = cap_a + cap_b;
    let alloc = water_filling_allocation(&routes, demand);
    assert!(
        alloc[0] > cap_a.max(cap_b),
        "multi-key route capacity should exceed any single key: alloc={} cap_a={cap_a} cap_b={cap_b}",
        alloc[0]
    );
    assert!(
        (alloc[0] - (cap_a + cap_b)).abs() < 1.0,
        "multi-key route capacity should be ~sum of keys: alloc={} sum={}",
        alloc[0],
        cap_a + cap_b
    );
}

// S9 单 key 限流多 key 顶替：key A 429、key B 健康 → A 权重 < B，但都仍有学习。
#[test]
fn sim_s9_limited_key_downweighted_healthy_key_takes_over() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let model = format!("agnes-s9-{suffix}");
    let key_a = routing_key("agnes", &model, &format!("fp-s9-a-{suffix}"));
    let key_b = routing_key("agnes", &model, &format!("fp-s9-b-{suffix}"));
    for _ in 0..40 {
        record_llm_success(&key_a, 2_000);
        record_llm_success(&key_b, 2_000);
    }
    let error = sim_rate_limit_error();
    for _ in 0..4 {
        record_llm_retryable_error(&key_a, &error, 300);
    }
    let weight_a = route_profile_scheduling_profile(&key_a)
        .expect("key A still has learning")
        .weight;
    let weight_b = route_profile_scheduling_profile(&key_b)
        .expect("key B still has learning")
        .weight;
    assert!(
        weight_a < weight_b,
        "rate-limited key should be down-weighted below healthy key: a={weight_a} b={weight_b}"
    );
    assert!(weight_a > 0.0, "rate-limited key must not be killed");
    // 整模型不摘除：route 级 capacity 仍 > 0（B 兜底）
    let cap_a = route_profile_capacity(&key_a).capacity_rpm;
    let cap_b = route_profile_capacity(&key_b).capacity_rpm;
    assert!(cap_b > 0.0);
    assert!(
        cap_a < cap_b,
        "rate-limited key capacity should drop below healthy: a={cap_a} b={cap_b}"
    );
}

// S10 全部路由被限流：所有路由连续 429 → water_filling 仍能返回（不死锁），
// 各 capacity 降但不归零。
#[test]
fn sim_s10_all_routes_limited_no_deadlock() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let key_a = routing_key(
        "agnes",
        &format!("agnes-s10a-{suffix}"),
        &format!("fp-s10a-{suffix}"),
    );
    let key_b = routing_key(
        "sensenova",
        &format!("deepseek-s10b-{suffix}"),
        &format!("fp-s10b-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&key_a, 2_000);
        record_llm_success(&key_b, 10_000);
    }
    let error = sim_rate_limit_error();
    for _ in 0..6 {
        record_llm_retryable_error(&key_a, &error, 300);
        record_llm_retryable_error(&key_b, &error, 300);
    }
    let cap_a = route_profile_capacity(&key_a).capacity_rpm;
    let cap_b = route_profile_capacity(&key_b).capacity_rpm;
    assert!(
        cap_a > 0.0,
        "capacity must not hit zero (health floor 0.05)"
    );
    assert!(
        cap_b > 0.0,
        "capacity must not hit zero (health floor 0.05)"
    );
    let routes = vec![(vec![key_a.clone()], 0.0), (vec![key_b.clone()], 0.0)];
    let alloc = water_filling_allocation(&routes, 5.0);
    let total: f64 = alloc.iter().sum();
    assert!(
        total > 0.0,
        "water_filling must still return allocation under full rate-limit (no deadlock)"
    );
}

// S11 慢模型也想要一些（不阻塞）：快+慢都有余量，需求大；慢模型 speed ≥ 0.2×fastest
// 时不被排除 → 分到少量。
#[test]
fn sim_s11_moderately_slow_route_gets_some_when_demand_high() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let fast = routing_key(
        "agnes",
        &format!("agnes-s11-{suffix}"),
        &format!("fp-s11-f-{suffix}"),
    );
    let mid = routing_key(
        "sensenova",
        &format!("deepseek-s11-{suffix}"),
        &format!("fp-s11-m-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&fast, 2_000);
        record_llm_success(&mid, 8_000); // 速度比 = 2/8 = 0.25 ≥ 0.2 → 不排除
    }
    let fast_cap = route_profile_capacity(&fast).capacity_rpm;
    let routes = vec![(vec![fast.clone()], 0.0), (vec![mid.clone()], 0.0)];
    // 需求超过 fast 容量 → mid 分到剩余
    let demand = fast_cap + 3.0;
    let alloc = water_filling_allocation(&routes, demand);
    assert!(alloc[0] > 0.0);
    assert!(
        alloc[1] > 0.0,
        "moderately slow route (speed ratio 0.25 >= 0.2) should receive spillover: {}",
        alloc[1]
    );
    assert!(
        alloc[1] < alloc[0],
        "slow route should take less than fast: fast={} mid={}",
        alloc[0],
        alloc[1]
    );
}

// S12 突发高延迟毛刺：先 2s 多次，再连续 200s 高延迟 → p95 上升，
// speed 立即下降（p95 窗口响应）。
#[test]
fn sim_s12_latency_spike_drops_capacity() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let key = routing_key(
        "agnes",
        &format!("agnes-s12-{suffix}"),
        &format!("fp-s12-{suffix}"),
    );
    for _ in 0..30 {
        record_llm_success(&key, 2_000);
    }
    let before = route_profile_capacity(&key);
    // 突发持续高延迟：p95 样本窗口被 200s 样本占据 → speed 大幅下降
    for _ in 0..30 {
        record_llm_success(&key, 200_000);
    }
    let after = route_profile_capacity(&key);
    assert!(
        after.speed_rpm < before.speed_rpm / 10.0,
        "speed should drop sharply after latency spike: before={} after={}",
        before.speed_rpm,
        after.speed_rpm
    );
    // 吞吐 EWMA 由测试内背靠背调用决定（时间语义），不作数值断言；
    // 关键行为是 p95 延迟画像对毛刺的即时响应（speed 降 → 调度权重同步降）。
    let weight = route_profile_scheduling_profile(&key)
        .expect("still alive")
        .weight;
    assert!(weight > 0.0, "route must not be killed by latency spike");
}

// S13 并发槽被慢模型占满：3 个慢路由 record_success(150s)，需求小 → water_filling
// 只用最快，其他 0（不浪费）。
#[test]
fn sim_s13_small_demand_uses_only_fastest_slow_get_zero() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let slow_a = routing_key(
        "nvidia",
        &format!("ink-s13a-{suffix}"),
        &format!("fp-s13a-{suffix}"),
    );
    let slow_b = routing_key(
        "nvidia",
        &format!("ink-s13b-{suffix}"),
        &format!("fp-s13b-{suffix}"),
    );
    let slow_c = routing_key(
        "nvidia",
        &format!("ink-s13c-{suffix}"),
        &format!("fp-s13c-{suffix}"),
    );
    // 三个"慢"路由（150s）+ 其中一个略快（100s）→ 需求小只用最快那个
    for _ in 0..40 {
        record_llm_success(&slow_a, 150_000);
        record_llm_success(&slow_b, 150_000);
        record_llm_success(&slow_c, 100_000);
    }
    let routes = vec![
        (vec![slow_a.clone()], 0.0),
        (vec![slow_b.clone()], 0.0),
        (vec![slow_c.clone()], 0.0),
    ];
    let alloc = water_filling_allocation(&routes, 0.3);
    let total: f64 = alloc.iter().sum();
    assert!(
        (total - 0.3).abs() < 1e-9,
        "demand should be fully allocated: total={total}"
    );
    // 最快（slow_c，100s）速度比 = (60/100)/(60/150)=1.5 ≥ 0.2？三个互比：
    // fastest=0.6(100s)，0.4/0.6≈0.67 ≥ 0.2 不排除；但需求 0.3 < c_c → c 独扛
    assert_eq!(alloc[0], 0.0, "slow_a not needed");
    assert_eq!(alloc[1], 0.0, "slow_b not needed");
    assert!(alloc[2] > 0.0, "fastest of the slow routes takes all");
}

// S14 冷启动全池无画像：所有路由都无 record → water_filling 全用 cold_start
// 回退容量（不偏向，速度相同按容量降序=相等 → 按顺序填）。
#[test]
fn sim_s14_all_cold_start_uses_fallback_without_bias() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let key_a = routing_key(
        "agnes",
        &format!("agnes-s14a-{suffix}"),
        &format!("fp-s14a-{suffix}"),
    );
    let key_b = routing_key(
        "sensenova",
        &format!("deepseek-s14b-{suffix}"),
        &format!("fp-s14b-{suffix}"),
    );
    let key_c = routing_key(
        "nvidia",
        &format!("ink-s14c-{suffix}"),
        &format!("fp-s14c-{suffix}"),
    );
    let routes = vec![
        (vec![key_a.clone()], 6.0),
        (vec![key_b.clone()], 6.0),
        (vec![key_c.clone()], 6.0),
    ];
    // 需求 8 > 单 route 回退容量 6 → 第一条填满 6，第二条分 2，第三条 0
    let alloc = water_filling_allocation(&routes, 8.0);
    let total: f64 = alloc.iter().sum();
    assert!((total - 8.0).abs() < 1e-9, "total should meet demand");
    assert!(
        (alloc[0] - 6.0).abs() < 1e-9,
        "first route fills to its fallback capacity: {}",
        alloc[0]
    );
    assert!(
        (alloc[1] - 2.0).abs() < 1e-9,
        "second route takes the remainder: {}",
        alloc[1]
    );
    assert_eq!(alloc[2], 0.0, "third route not needed");
}

// S15 需求小于最快单路由容量：D < c_fast → 只用最快路由，其他全 0。
#[test]
fn sim_s15_demand_below_fastest_capacity_uses_only_fastest() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let fast = routing_key(
        "agnes",
        &format!("agnes-s15-{suffix}"),
        &format!("fp-s15-f-{suffix}"),
    );
    let mid = routing_key(
        "sensenova",
        &format!("deepseek-s15-{suffix}"),
        &format!("fp-s15-m-{suffix}"),
    );
    for _ in 0..40 {
        record_llm_success(&fast, 2_000);
        record_llm_success(&mid, 10_000);
    }
    let fast_cap = route_profile_capacity(&fast).capacity_rpm;
    let demand = fast_cap / 2.0; // 严格小于最快容量
    let routes = vec![(vec![fast.clone()], 0.0), (vec![mid.clone()], 0.0)];
    let alloc = water_filling_allocation(&routes, demand);
    assert!(
        (alloc[0] - demand).abs() < 1e-9,
        "fastest route takes the entire demand: {}",
        alloc[0]
    );
    assert_eq!(alloc[1], 0.0, "slower route gets exactly zero");
    // 换算吞吐速率（模拟每次调度取 1 请求）：D/T/60
    let caps = [
        route_profile_capacity(&fast).capacity_rpm,
        route_profile_capacity(&mid).capacity_rpm,
    ];
    let rate = sim_avg_req_per_sec(&alloc, &caps, demand);
    assert!(rate > 0.0, "effective rate should be positive: {rate}");
}

// reserve_slot 并发上限：有学习数据时优先用实测吞吐反推的 effective_parallel
//（"不排队能同时扛几个"），而不是 recommended_parallel（latency 正推）。
// 核心反差：慢思考型路由的 latency 高，recommended_parallel = rpm×p95/60 会被推高
// （"越慢越给"恶性循环）；effective_parallel = 吞吐/单并发速度 在吞吐 < rpm 时
// 把慢路由压得更小——修复"慢路由占着大量并发槽干等，快路由反而拿不到并发"。
// 用直接构造的 limiter（不经全局注册表）精确控制 throughput_ewma 与 rpm 的差。
#[test]
fn reserve_slot_uses_effective_parallel_when_learned() {
    let _guard = profile_store_test_guard();
    // 慢思考型路由：120s p95 延迟，learned_rpm=12，但实测吞吐只有 3 req/min
    // （真实场景：慢路由因重试/超时/被限流，实测完成速率低于 learned_rpm）。
    // recommended_parallel = 12×120000/60000 = 24（latency 正推，被推高）。
    // effective_parallel = 3 / (60000/120000) = 3/0.5 = 6 << 24（吞吐反推，压小）。
    let slow_limiter = test_limiter(12.0, 6.0, 60.0);
    {
        let mut state = slow_limiter.state.lock().expect("slow state");
        state.success_count = 5;
        state.latency_samples_ms = vec![120_000; 5];
        state.throughput_ewma_rpm = 3.0; // 实测吞吐 < learned_rpm
        refresh_recommended_parallel(&mut state);
    }
    let slow_recommended = slow_limiter.recommended_parallel();
    let slow_effective = {
        let state = slow_limiter.state.lock().expect("slow state");
        learned_effective_parallel(&state).expect("slow should have learning")
    };
    // recommended_parallel（latency 正推）把慢路由推高——正是要修复的反差。
    assert!(
        slow_recommended >= 20,
        "slow recommended_parallel should be pushed high by latency: {}",
        slow_recommended
    );
    // effective_parallel（吞吐反推）显著小于 recommended_parallel。
    assert!(
        slow_effective < slow_recommended,
        "slow effective_parallel ({}) should be < recommended_parallel ({})",
        slow_effective,
        slow_recommended
    );

    // reserve_slot 用 effective_parallel 作为上限：占满后 RetryAfter，
    // 而不是被 recommended_parallel 推到 24 个槽。
    let mut held = 0usize;
    for _ in 0..slow_recommended {
        match slow_limiter.reserve_slot() {
            RouteReserve::Reserved(_) => held += 1,
            RouteReserve::RetryAfter(_) => break,
        }
    }
    assert_eq!(
        held, slow_effective,
        "reserve_slot should cap at effective_parallel ({}), not recommended_parallel ({})",
        slow_effective, slow_recommended
    );
    let overflow = slow_limiter.reserve_slot();
    assert!(
        matches!(overflow, RouteReserve::RetryAfter(_)),
        "beyond effective_parallel should RetryAfter"
    );
    for _ in 0..held {
        slow_limiter.release();
    }

    // 快路由（2s 延迟 + 高吞吐）：effective_parallel 大，能拿到多个并发槽。
    let fast_limiter = test_limiter(30.0, 6.0, 60.0);
    {
        let mut state = fast_limiter.state.lock().expect("fast state");
        state.success_count = 20;
        state.latency_samples_ms = vec![2_000; 20];
        state.throughput_ewma_rpm = 28.0; // 高吞吐
        refresh_recommended_parallel(&mut state);
    }
    let fast_effective = {
        let state = fast_limiter.state.lock().expect("fast state");
        learned_effective_parallel(&state).expect("fast should have learning")
    };
    assert!(
        fast_effective >= 1,
        "fast effective_parallel should be >= 1: {}",
        fast_effective
    );
    let first = fast_limiter.reserve_slot();
    assert!(
        matches!(first, RouteReserve::Reserved(_)),
        "fast route should reserve within its effective_parallel"
    );
    fast_limiter.release();
}

// reserve_slot 并发上限：无学习数据（冷启动）时回退 recommended_parallel，
// 不能用 0 并发（否则冷启动死锁）。
#[test]
fn reserve_slot_falls_back_to_recommended_parallel_when_cold() {
    let _guard = profile_store_test_guard();
    let suffix = sim_suffix();
    let cold = routing_key(
        "agnes",
        &format!("rs-cold-{suffix}"),
        &format!("fp-rs-cold-{suffix}"),
    );
    let cap = route_profile_capacity(&cold);
    assert!(!cap.has_learning, "cold route should have no learning");
    assert_eq!(
        cap.effective_parallel, 0,
        "cold route effective_parallel is 0"
    );

    // 冷启动：recommended_parallel 默认 1 → 第 1 个 reserve 成功，第 2 个 RetryAfter。
    let limiter = adaptive_llm_rate_limiter(&cold);
    let first = limiter.reserve_slot();
    assert!(
        matches!(first, RouteReserve::Reserved(_)),
        "cold route should reserve with recommended_parallel fallback"
    );
    let second = limiter.reserve_slot();
    assert!(
        matches!(second, RouteReserve::RetryAfter(_)),
        "cold route at recommended_parallel=1 should RetryAfter on second reserve"
    );
    limiter.release();
}
