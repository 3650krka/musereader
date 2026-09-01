use super::context::{prune_confirmed_algorithmic_hints, repair_canonical_proper_nouns};
use super::review::{
    append_wave_term_key_index, collect_wave_patch_terms_for_chunk, collect_wave_term_delta,
    fill_translation_workers,
};
use super::runtime::{ensure_not_cancelled, route_profile_recommended_parallel};
use super::{
    collect_pending_indexes, compute_translate_progress, ChunkCheckpoint, ChunkSegmentKind,
    PipelineProgressStage, PipelineRunConfig, RuntimePromptConfig,
};
use super::{ArtifactPaths, TranslationPipeline};
use crate::error::AppError;
use chrono::Utc;
use tokio::task::JoinSet;

// 主翻译运行并发的全局上限兜底（与 commands.rs 的 MUSETRANSLATE_GLOBAL_LLM_LIMIT 语义一致）。
const DEFAULT_TRANSLATION_MAX_PARALLEL: usize = 10;
const MAX_TRANSLATION_MAX_PARALLEL: usize = 64;

fn resolve_translation_parallel_ceiling() -> usize {
    std::env::var("MUSETRANSLATE_GLOBAL_LLM_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_TRANSLATION_MAX_PARALLEL)
        .clamp(1, MAX_TRANSLATION_MAX_PARALLEL)
}

pub(super) async fn translate_checkpoint_chunks(
    pipeline: &TranslationPipeline,
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    checkpoint: &mut ChunkCheckpoint,
    progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
) -> Result<(), AppError> {
    let total = checkpoint.chunks.len();
    if total == 0 {
        return Err(AppError::internal("OCR produced zero chunks"));
    }

    let pending_indexes = collect_pending_indexes(checkpoint);
    // 预建 index→pos 映射：每块完成时 O(1) 查找，替代 O(n) 线性 find。
    let chunk_pos: std::collections::HashMap<usize, usize> = checkpoint
        .chunks
        .iter()
        .enumerate()
        .map(|(pos, chunk)| (chunk.index, pos))
        .collect();
    let mut completed_count = total.saturating_sub(pending_indexes.len());
    if completed_count > 0 {
        let progress = compute_translate_progress(completed_count, total);
        progress_callback(
            PipelineProgressStage::Translating,
            &format!("prepared non-body/protected chunks ({completed_count}/{total})"),
            progress,
        );
    }

    let configured_parallel = config.max_parallel.max(1);
    // 主翻译并发接入 route profile 学习（方案A+选项1）：
    //   effective_parallel = min(各路由 recommended_parallel 之和, 全局上限 GLOBAL_LLM_LIMIT)
    // 把空等的本地并发槽用起来（empirical route_wait 135s 的根因是并发写死=configured=3，
    // 而路由画像 learned 建议 15）。上限用全局上限而非路由全量，给后续阶段留余量。
    let global_ceiling = resolve_translation_parallel_ceiling();
    let learned_sum: usize = pipeline
        .llm_client
        .active_routing_keys()
        .iter()
        .map(|key| route_profile_recommended_parallel(key, configured_parallel))
        .sum();
    let max_parallel = learned_sum.clamp(1, global_ceiling);
    // 全局信号量与 effective_parallel 同步：worker 先 acquire 全局信号量再 reserve 路由，
    // 若全局信号量（global_llm_limiter）小于 effective_parallel，10 个 worker 抢少数
    // 全局许可 → 大量 wait_ms 等待（实证 47% 时间空等）。此处把全局信号量补齐到
    // ≥ effective_parallel（不缩小已有较大值），消除分配与全局并发脱节的阻塞。
    sync_global_llm_limiter_to_at_least(&config.global_llm_limiter, max_parallel);
    // 解耦运行并发与 wave 术语 flush 粒度：运行并发（max_parallel）可调大提速，
    // 而 wave flush 粒度锁为 configured_parallel（原始并发），保证动态术语进入后续
    // chunk prompt 的时机与现状完全一致（效果等价，遗留#12 的解耦前提）。

    // 修正（Sophie 漂移根因之一）：wave_flush_size 若锁为 configured_parallel（3），
    // 而运行并发 max_parallel 已学到 15+，则术语进入后续 chunk prompt 的延迟
    // 被人为拉长——15 个 worker 在飞，但每 3 个完成才 flush 一次术语，
    // 后续 chunk 看到的 confirmedTerms 严重滞后。改为与运行并发对齐。
    let wave_flush_size = resolve_wave_flush_size(max_parallel);
    pipeline.record_event(
        &paths.event_log_path,
        "translation_parallel_runtime",
        &format!(
            "configured_parallel={configured_parallel}; learned_route_parallel_sum={learned_sum}; global_ceiling={global_ceiling}; effective_parallel={max_parallel}; wave_flush_size={wave_flush_size}"
        ),
        40,
    )?;
    let system_prompt = runtime_prompt.system_prompt.clone();
    let mut next_pending = 0usize;
    let mut join_set = JoinSet::new();
    let mut wave_completed_indexes = Vec::new();
    fill_translation_workers(
        &mut join_set,
        &mut next_pending,
        &pending_indexes,
        max_parallel,
        checkpoint,
        config,
        &pipeline.llm_client,
        &system_prompt,
        &paths.event_log_path,
    )?;

    while completed_count < total {
        ensure_not_cancelled(&config.cancellation_token)?;

        if join_set.is_empty() {
            break;
        }

        let result = join_set
            .join_next()
            .await
            .ok_or_else(|| AppError::internal("translation worker set ended unexpectedly"))?;
        let outcome = result.map_err(|error| {
            AppError::internal(format!("translation worker join failed: {error}"))
        })??;
        /* 块级失败隔离：重试耗尽的块标记 TranslationFailed 并继续推进任务——
           产物中该块保留原文，失败清单随 PipelineResult 上报；
           重跑任务时加载层会把它重置为 Pending 补译。 */
        let outcome = match outcome {
            super::TranslationWorkerResult::Exhausted {
                index,
                attempts,
                failures,
            } => {
                let pos = chunk_pos.get(&index).copied().ok_or_else(|| {
                    AppError::internal(format!(
                        "translation exhaustion referenced missing chunk index={index}"
                    ))
                })?;
                let chunk = &mut checkpoint.chunks[pos];
                chunk.attempts = attempts;
                chunk.updated_at = Utc::now().to_rfc3339();
                chunk.processing_stage = Some(super::ChunkProcessingStage::TranslationFailed);
                let last_failure = failures.last().map(|failure| {
                    format!(
                        "code={}; attempt={}; message={}",
                        failure.code, failure.attempt, failure.message_preview
                    )
                });
                let progress = compute_translate_progress(completed_count + 1, total);
                pipeline.record_event(
                    &paths.event_log_path,
                    "chunk_translation_exhausted",
                    &format!(
                        "chunk_index={index}; attempts={attempts}; failure_count={}; last_failure={}",
                        failures.len(),
                        last_failure.unwrap_or_default()
                    ),
                    progress,
                )?;
                super::checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
                completed_count += 1;
                progress_callback(
                    PipelineProgressStage::Translating,
                    &format!("translating ({completed_count}/{total})"),
                    compute_translate_progress(completed_count, total),
                );
                continue;
            }
            super::TranslationWorkerResult::Done(outcome) => outcome,
        };
        for failure in &outcome.attempt_failures {
            pipeline.record_event(
                &paths.event_log_path,
                "llm_attempt_failed",
                &format!(
                    "chunk_index={}; attempt={}; code={}; request_elapsed_ms={}; delay_ms={}; message={}",
                    outcome.index,
                    failure.attempt,
                    failure.code,
                    failure.request_elapsed_ms,
                    failure.delay_ms,
                    failure.message_preview
                ),
                compute_translate_progress(completed_count, total),
            )?;
        }
        let pos = chunk_pos.get(&outcome.index).copied().ok_or_else(|| {
            AppError::internal(format!(
                "translation outcome referenced missing chunk index={}",
                outcome.index
            ))
        })?;
        let chunk = &mut checkpoint.chunks[pos];
        let repaired = repair_canonical_proper_nouns(
            &outcome.translated,
            checkpoint.translation_context.as_ref(),
        );
        let structure_stats = collect_chunk_structure_stats(&chunk.source, &repaired);
        chunk.translated = Some(repaired);
        chunk.confirmed_terms = outcome.confirmed_terms;
        let pruned_algorithmic_hints = checkpoint
            .translation_context
            .as_mut()
            .map(|context| {
                prune_confirmed_algorithmic_hints(context, outcome.index, &chunk.confirmed_terms)
            })
            .unwrap_or_default();
        checkpoint
            .delta_sync
            .chunk_started_glossary_seq
            .insert(outcome.index, outcome.started_glossary_seq);
        checkpoint
            .delta_sync
            .chunk_patched_through_glossary_seq
            .insert(outcome.index, outcome.started_glossary_seq);
        chunk.attempts = chunk.attempts.saturating_add(1);
        chunk.updated_at = Utc::now().to_rfc3339();
        chunk.processing_stage = Some(super::ChunkProcessingStage::Translated);
        // 译名首见锁定：该 chunk 译完后立即把 confirmedTerms 里「首见译名」写回
        // properNouns.target（Sophie 苏菲/索菲 不一致的根治——把 first-seen anchor 从
        // 「翻译后统一跑」提前到「每个 chunk 译完后立即」，后续 chunk 的 prompt 即可
        // 把该专名升级为 strict 硬约束 MUST USE EXACTLY）。
        // 注意：需在 chunk 借用结束后调用（checkpoint 二次可变借用）。
        let first_seen_locked = crate::pipeline::consistency::lock_chunk_first_seen_translations(
            checkpoint,
            outcome.index,
        );
        let _ = first_seen_locked;
        super::checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
        wave_completed_indexes.push(outcome.index);

        completed_count += 1;
        let progress = compute_translate_progress(completed_count, total);
        pipeline.record_event(
            &paths.event_log_path,
            "chunk_translated",
            &format!(
                "translated chunk {}/{}; index={}; elapsed_ms={}; wait_ms={}; route_wait_ms={}; llm_elapsed_ms={}; llm_attempts={}",
                completed_count,
                total,
                outcome.index,
                outcome.elapsed_ms,
                outcome.wait_ms,
                outcome.route_wait_ms,
                outcome.llm_elapsed_ms,
                outcome.llm_attempts
            ),
            progress,
        )?;
        pipeline.record_event(
            &paths.event_log_path,
            "chunk_structure_check",
            &format!(
                "chunk_index={}; source_blocks={}; translated_blocks={}; block_delta={}; source_headings={}; translated_headings={}; heading_mismatch={}; mismatch={}",
                outcome.index,
                structure_stats.source_blocks,
                structure_stats.translated_blocks,
                structure_stats.block_delta_abs,
                structure_stats.source_headings,
                structure_stats.translated_headings,
                structure_stats.heading_mismatch,
                structure_stats.mismatch
            ),
            progress,
        )?;
        if pruned_algorithmic_hints > 0 {
            pipeline.record_event(
                &paths.event_log_path,
                "algorithmic_hints_pruned",
                &format!(
                    "chunk_index={}; pruned_count={pruned_algorithmic_hints}",
                    outcome.index
                ),
                progress,
            )?;
        }
        progress_callback(
            PipelineProgressStage::Translating,
            &format!("translating ({completed_count}/{total})"),
            progress,
        );

        if wave_completed_indexes.len() >= wave_flush_size || join_set.is_empty() {
            let mut wave_delta = collect_wave_term_delta(checkpoint, &wave_completed_indexes);
            for term in &mut wave_delta {
                checkpoint.delta_sync.glossary_seq =
                    checkpoint.delta_sync.glossary_seq.saturating_add(1);
                term.seq = checkpoint.delta_sync.glossary_seq;
            }
            append_wave_term_key_index(checkpoint, &wave_delta);
            checkpoint
                .delta_sync
                .wave_delta_log
                .extend(wave_delta.iter().cloned());
            if !wave_delta.is_empty() {
                // 并发化 wave repair：每个 chunk 的 repair 作为 JoinSet 任务并行执行，
                // 主循环只 await 聚合结果。复用 global_llm_limiter 与 wait_for_llm_rate_slot
                // 背压，与主翻译 worker 共享路由并发槽。
                let mut repair_join_set = JoinSet::new();
                let mut repair_stats_total = super::WaveTermRepairStats::default();
                let mut selection_matched_terms = 0usize;
                let mut selection_algorithmic_match_count = 0usize;
                let mut selection_source_scan_match_count = 0usize;
                let mut selection_deduped_terms = 0usize;
                let mut patched_chunks = 0usize;
                let article_type = checkpoint.article_type.clone();
                let static_glossary_entries = checkpoint
                    .translation_context
                    .as_ref()
                    .map(|context| {
                        context
                            .static_terms
                            .iter()
                            .filter(|entry| {
                                !entry.source.trim().is_empty() && !entry.target.trim().is_empty()
                            })
                            .map(|entry| (entry.source.clone(), entry.target.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                for completed_index in wave_completed_indexes.iter().copied() {
                    let selection = collect_wave_patch_terms_for_chunk(
                        checkpoint,
                        completed_index,
                        &wave_delta,
                    );
                    selection_deduped_terms += selection.deduped_count;
                    selection_algorithmic_match_count += selection.algorithmic_match_count;
                    selection_source_scan_match_count += selection.source_scan_match_count;
                    if selection.terms.is_empty() {
                        continue;
                    }
                    selection_matched_terms += selection.terms.len();
                    let Some(pos) = chunk_pos.get(&completed_index).copied() else {
                        continue;
                    };
                    let Some(translated) = checkpoint.chunks[pos].translated.clone() else {
                        continue;
                    };
                    let client = pipeline.llm_client.clone();
                    let source = checkpoint.chunks[pos].source.clone();
                    let article_type = article_type.clone();
                    let static_entries = static_glossary_entries.clone();
                    let terms = selection.terms;
                    repair_join_set.spawn(async move {
                        crate::pipeline::review::apply_wave_term_repairs_for_chunk(
                            &client,
                            source,
                            translated,
                            completed_index,
                            terms,
                            static_entries,
                            article_type,
                        )
                        .await
                    });
                }

                // 聚合并发结果并写回 checkpoint。
                while let Some(result) = repair_join_set.join_next().await {
                    let outcome = result.map_err(|error| {
                        AppError::internal(format!("wave repair worker join failed: {error}"))
                    })??;
                    let stats = outcome.stats;
                    repair_stats_total.attempted_terms += stats.attempted_terms;
                    repair_stats_total.applied_terms += stats.applied_terms;
                    repair_stats_total.already_correct_count += stats.already_correct_count;
                    repair_stats_total.llm_patch_count += stats.llm_patch_count;
                    repair_stats_total.llm_correct_count += stats.llm_correct_count;
                    repair_stats_total.fallback_patch_count += stats.fallback_patch_count;
                    if stats.changed {
                        patched_chunks += 1;
                        // 写回修补后的文本与 patched_seq。
                        if let Some(pos) = chunk_pos.get(&outcome.chunk_index).copied() {
                            checkpoint.chunks[pos].translated = Some(outcome.repaired);
                            let started_seq = checkpoint
                                .delta_sync
                                .chunk_started_glossary_seq
                                .get(&outcome.chunk_index)
                                .copied()
                                .unwrap_or_default();
                            checkpoint
                                .delta_sync
                                .chunk_patched_through_glossary_seq
                                .insert(outcome.chunk_index, outcome.max_seq.max(started_seq));
                        }
                    }
                }

                if patched_chunks > 0 {
                    super::checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
                }
                pipeline.record_event(
                    &paths.event_log_path,
                    "wave_term_delta",
                    &format!(
                        "wave completed; new_term_count={}; chunk_count={}; matched_terms={}; algorithmic_match_count={}; source_scan_match_count={}; deduped_terms={}; applied_terms={}; already_correct_terms={}; llm_patch_count={}; llm_correct_count={}; fallback_patch_count={}; patched_chunks={}",
                        wave_delta.len(),
                        wave_completed_indexes.len(),
                        selection_matched_terms,
                        selection_algorithmic_match_count,
                        selection_source_scan_match_count,
                        selection_deduped_terms,
                        repair_stats_total.applied_terms,
                        repair_stats_total.already_correct_count,
                        repair_stats_total.llm_patch_count,
                        repair_stats_total.llm_correct_count,
                        repair_stats_total.fallback_patch_count,
                        patched_chunks,
                    ),
                    progress,
                )?;
            }
            wave_completed_indexes.clear();
        }

        fill_translation_workers(
            &mut join_set,
            &mut next_pending,
            &pending_indexes,
            max_parallel,
            checkpoint,
            config,
            &pipeline.llm_client,
            &system_prompt,
            &paths.event_log_path,
        )?;
    }

    if checkpoint.chunks.iter().any(|chunk| {
        chunk.segment_kind == ChunkSegmentKind::Body
            && super::checkpoint::current_chunk_processing_stage(chunk)
                == super::ChunkProcessingStage::Pending
    }) {
        let remaining = checkpoint
            .chunks
            .iter()
            .filter(|chunk| {
                chunk.segment_kind == ChunkSegmentKind::Body
                    && super::checkpoint::current_chunk_processing_stage(chunk)
                        == super::ChunkProcessingStage::Pending
            })
            .count();
        return Err(AppError::internal(format!(
            "translation completed with {remaining} untranslated body chunks remaining"
        )));
    }

    // 全失败保护：正文块全部翻译失败时不交付「整篇原文」的产物，
    // 按整任务失败上报（此时大概率是配置/凭证问题，块级隔离无意义）。
    let failed_count = super::failed_body_chunk_indexes(checkpoint).len();
    let body_count = checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.segment_kind == ChunkSegmentKind::Body)
        .count();
    if failed_count > 0 && failed_count >= body_count {
        return Err(AppError::internal(format!(
            "all {failed_count} body chunks failed translation"
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkStructureStats {
    source_blocks: usize,
    translated_blocks: usize,
    block_delta_abs: usize,
    source_headings: usize,
    translated_headings: usize,
    heading_mismatch: bool,
    mismatch: bool,
}

fn collect_chunk_structure_stats(source: &str, translated: &str) -> ChunkStructureStats {
    // 轻量计数：split("\n\n") + 首行 # 判定，与 build_source_blocks_from_markdown
    // 的 block 切分/heading 判定语义对齐，但只做计数不做完整 block 构建。
    let (source_blocks, source_headings) = count_blocks_and_headings(source);
    let (translated_blocks, translated_headings) = count_blocks_and_headings(translated);
    let block_delta_abs = source_blocks.abs_diff(translated_blocks);
    let heading_mismatch = source_headings != translated_headings;

    ChunkStructureStats {
        source_blocks,
        translated_blocks,
        block_delta_abs,
        source_headings,
        translated_headings,
        heading_mismatch,
        mismatch: block_delta_abs > 0 || heading_mismatch,
    }
}

fn count_blocks_and_headings(markdown: &str) -> (usize, usize) {
    let mut blocks = 0usize;
    let mut headings = 0usize;
    for block in markdown.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }
        blocks += 1;
        if trimmed.starts_with('#') {
            headings += 1;
        }
    }
    (blocks, headings)
}

// wave 术语 flush 粒度：默认与运行并发一致（保持既有行为），可用
// MUSETRANSLATE_WAVE_FLUSH_SIZE 显式固定为较小值，使运行并发调大时动态术语
// 进入后续 chunk prompt 的时机不随并发漂移（解耦 wave flush 与运行并发）。
fn resolve_wave_flush_size(max_parallel: usize) -> usize {
    let max_parallel = max_parallel.max(1);
    resolve_wave_flush_size_with_env(max_parallel, |key| std::env::var(key).ok())
}

// 全局信号量与 effective_parallel 同步：若全局信号量小于 effective_parallel，
// 10 个 worker 抢少数全局许可 → 大量 wait_ms 等待（实证 47% 时间空等）。
// 通过 add_permits 把全局信号量补齐到 ≥ effective_parallel（不缩小已有较大值）。
// 注意：tokio Semaphore 无"当前许可数"读取，available_permits() 是瞬时值（含在飞），
// 所以按"缺口 = effective - available"补，可能略超（在飞请求占用的额度会随后释放，
// 不会影响正确性——只是短暂多给一点，worker 仍受路由级 RPM/并发约束）。
fn sync_global_llm_limiter_to_at_least(
    limiter: &std::sync::Arc<tokio::sync::Semaphore>,
    effective_parallel: usize,
) {
    let available = limiter.available_permits();
    if available < effective_parallel {
        limiter.add_permits(effective_parallel - available);
    }
}

fn resolve_wave_flush_size_with_env(
    max_parallel: usize,
    read_env: impl Fn(&str) -> Option<String>,
) -> usize {
    read_env("MUSETRANSLATE_WAVE_FLUSH_SIZE")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(max_parallel))
        .unwrap_or(max_parallel)
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_translation_parallel_ceiling, resolve_wave_flush_size_with_env,
        sync_global_llm_limiter_to_at_least,
    };

    fn env_with(value: Option<&str>) -> impl Fn(&str) -> Option<String> {
        let value = value.map(str::to_string);
        move |key| {
            if key == "MUSETRANSLATE_WAVE_FLUSH_SIZE" {
                value.clone()
            } else {
                None
            }
        }
    }

    // 主翻译有效并发的纯函数核心（与 translate_checkpoint_chunks 中的取法一致）：
    // effective = clamp(learned_sum, 1, global_ceiling)。
    fn effective_translation_parallel(learned_sum: usize, global_ceiling: usize) -> usize {
        learned_sum.clamp(1, global_ceiling.max(1))
    }

    #[test]
    fn translation_parallel_uses_learned_sum_capped_by_global_ceiling() {
        // route profile 学到 15，全局上限 10 → 取 10（选项1：不独占全局）
        assert_eq!(effective_translation_parallel(15, 10), 10);
        // route profile 学到 4，全局上限 10 → 取 4
        assert_eq!(effective_translation_parallel(4, 10), 4);
        // 冷启动 learned=0/1 → 下限 1（安全保守）
        assert_eq!(effective_translation_parallel(0, 10), 1);
    }

    #[test]
    fn global_llm_limiter_is_topped_up_to_effective_parallel() {
        // 全局信号量只有 3，effective_parallel=10 → 补齐到 ≥10
        let limiter = std::sync::Arc::new(tokio::sync::Semaphore::new(3));
        sync_global_llm_limiter_to_at_least(&limiter, 10);
        assert!(limiter.available_permits() >= 10);
        // 已较大（20）时不缩小
        let big = std::sync::Arc::new(tokio::sync::Semaphore::new(20));
        sync_global_llm_limiter_to_at_least(&big, 10);
        assert!(big.available_permits() >= 20);
    }

    #[test]
    fn wave_flush_size_defaults_to_max_parallel() {
        assert_eq!(resolve_wave_flush_size_with_env(3, env_with(None)), 3);
        assert_eq!(resolve_wave_flush_size_with_env(10, env_with(None)), 10);
    }

    #[test]
    fn wave_flush_size_is_capped_by_max_parallel() {
        // 运行并发调大时，flush 粒度保持固定（解耦核心）
        assert_eq!(resolve_wave_flush_size_with_env(15, env_with(Some("3"))), 3);
        // 运行并发小于设定值时，flush 粒度收敛到并发（不超过并发）
        assert_eq!(resolve_wave_flush_size_with_env(2, env_with(Some("3"))), 2);
    }

    #[test]
    fn wave_flush_size_ignores_invalid_env() {
        assert_eq!(resolve_wave_flush_size_with_env(5, env_with(Some("0"))), 5);
        assert_eq!(
            resolve_wave_flush_size_with_env(5, env_with(Some("abc"))),
            5
        );
    }

    #[test]
    fn translation_parallel_ceiling_defaults_to_global_llm_limit() {
        // 无 env 时应回退到全局默认 10（与 commands.rs 语义一致）。
        // 该测试依赖 MUSETRANSLATE_GLOBAL_LLM_LIMIT 未被外部环境设置。
        if std::env::var("MUSETRANSLATE_GLOBAL_LLM_LIMIT").is_err() {
            assert_eq!(resolve_translation_parallel_ceiling(), 10);
        }
    }
}
