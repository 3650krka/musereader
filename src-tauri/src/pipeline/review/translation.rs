use super::super::checkpoint::reject_mojibake_translation;
use super::super::context::{
    collect_strict_chunk_hits, format_translation_context_for_chunk, repair_strict_hit_aliases,
    synthesize_missing_strict_hit_terms,
};
use super::super::{
    artifact, ensure_not_cancelled, llm_retry_limit, record_llm_retryable_error,
    record_llm_success, retry_async_with_observer, wait_for_llm_rate_slot, AppError,
    ChunkCheckpoint, LlmAttemptFailure, PipelineRunConfig, TaskEvent, TranslationOutcome,
    TranslationWorkerResult,
};
use crate::llm::SensenovaClient;
use chrono::Utc;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::{Mutex as StdMutex, OnceLock};
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant};

fn format_dynamic_delta_context(
    source: &str,
    delta_terms: &[super::super::WaveTermDelta],
) -> Option<String> {
    let source_cache = crate::pipeline::context::SourceCandidateCache::new(source);
    let matched = delta_terms
        .iter()
        .filter(|term| {
            crate::pipeline::context::source_mentions_hint_cached(
                source,
                &term.source,
                &source_cache,
            )
        })
        .map(|term| format!("- {} -> {}", term.source, term.target))
        .collect::<Vec<_>>();
    if matched.is_empty() {
        return None;
    }
    Some(format!(
        "Recent confirmed terms from earlier completed chunks:\n{}\nUse them when the same source term appears in this chunk.",
        matched.join("\n")
    ))
}

fn previous_paragraph_context(checkpoint: &ChunkCheckpoint, index: usize) -> Option<String> {
    if index == 0 {
        return None;
    }
    let previous_source = checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index + 1 == index)
        .map(|chunk| chunk.source.as_str())?;
    // 过滤媒体段（图片/表格截图标记/注释），选最近的非空叙述段。
    let paragraph = previous_source
        .rsplit("\n\n")
        .map(str::trim)
        .find(|paragraph| {
            !paragraph.is_empty() && !paragraph.starts_with("![") && !paragraph.starts_with("<!--")
        })?;
    Some(format!(
        "Reference-only previous paragraph for context. Do not translate, rewrite, quote, summarize, or include this paragraph in translatedText:\n<REFERENCE_ONLY_PREVIOUS_PARAGRAPH>\n{}\n</REFERENCE_ONLY_PREVIOUS_PARAGRAPH>",
        paragraph
    ))
}

fn next_paragraph_context(checkpoint: &ChunkCheckpoint, index: usize) -> Option<String> {
    let next_source = checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index == index + 1)
        .map(|chunk| chunk.source.as_str())?;
    let paragraph = next_source.split("\n\n").map(str::trim).find(|paragraph| {
        !paragraph.is_empty() && !paragraph.starts_with("![") && !paragraph.starts_with("<!--")
    })?;
    Some(format!(
        "Reference-only next paragraph for context. Do not translate, rewrite, quote, summarize, or include this paragraph in translatedText:\n<REFERENCE_ONLY_NEXT_PARAGRAPH>\n{}\n</REFERENCE_ONLY_NEXT_PARAGRAPH>",
        paragraph
    ))
}

fn merge_translation_context(parts: Vec<Option<String>>) -> Option<String> {
    let merged = parts
        .into_iter()
        .flatten()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    (!merged.is_empty()).then(|| merged.join("\n\n"))
}

fn preview_error_message(message: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 180;
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.chars().take(MAX_PREVIEW_CHARS).collect()
}

fn event_log_write_lock() -> &'static StdMutex<()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
}

fn add_block_alignment_markers(markdown: &str) -> String {
    split_markdown_blocks(markdown)
        .into_iter()
        .enumerate()
        .map(|(index, block)| format!("[[B{index:03}]]\n{block}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn strip_block_alignment_markers(markdown: &str) -> String {
    let marker_blocks = split_markdown_by_block_alignment_markers(markdown);
    let blocks = if marker_blocks.is_empty() {
        split_markdown_blocks(markdown)
            .into_iter()
            .map(|block| strip_leading_block_alignment_marker(&block).to_string())
            .collect::<Vec<_>>()
    } else {
        marker_blocks
    };
    blocks
        .into_iter()
        .filter_map(|block| {
            let stripped = block.trim().to_string();
            (!stripped.is_empty()).then_some(stripped)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn split_markdown_by_block_alignment_markers(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    let mut current_block_start = None::<usize>;

    while let Some((marker_start, marker_end)) = find_next_block_alignment_marker(markdown, cursor)
    {
        if let Some(block_start) = current_block_start {
            blocks.push(markdown[block_start..marker_start].trim().to_string());
        }
        current_block_start = Some(marker_end);
        cursor = marker_end;
    }

    if let Some(block_start) = current_block_start {
        blocks.push(markdown[block_start..].trim().to_string());
    }
    blocks
}

fn find_next_block_alignment_marker(markdown: &str, cursor: usize) -> Option<(usize, usize)> {
    let mut search_start = cursor;
    while search_start < markdown.len() {
        let relative_start = markdown[search_start..].find("[[B")?;
        let marker_start = search_start + relative_start;
        let digits_start = marker_start + 3;
        let relative_end = markdown[digits_start..].find("]]")?;
        let digits_end = digits_start + relative_end;
        let digits = &markdown[digits_start..digits_end];
        let marker_end = digits_end + 2;
        if !digits.is_empty() && digits.len() <= 6 && digits.chars().all(|ch| ch.is_ascii_digit()) {
            return Some((marker_start, marker_end));
        }
        search_start = marker_start + 3;
    }
    None
}

/// Marker 完整性校验（借鉴 placeholder-validator 的差异报告思路）：
/// 比对译文中的 [[Bxxx]] 标记与源文标记序列，返回精确差异（缺失/重复/乱序）。
/// 全部匹配返回 None；有差异返回人类可读的报告（供重试时反馈给 LLM 或记事件）。
fn block_marker_integrity_report(source_marked: &str, translated: &str) -> Option<String> {
    let source_markers = extract_block_marker_indices(source_marked);
    let translated_markers = extract_block_marker_indices(translated);
    if source_markers == translated_markers {
        return None;
    }
    let mut parts = Vec::new();
    let missing: Vec<usize> = source_markers
        .iter()
        .filter(|index| !translated_markers.contains(index))
        .copied()
        .collect();
    if !missing.is_empty() {
        parts.push(format!(
            "Missing: {}",
            missing
                .iter()
                .map(|index| format!("[[B{index:03}]]"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut seen = std::collections::HashMap::<usize, usize>::new();
    for index in &translated_markers {
        *seen.entry(*index).or_default() += 1;
    }
    let duplicates: Vec<String> = seen
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(index, count)| format!("[[B{index:03}]] appears {count} times"))
        .collect();
    if !duplicates.is_empty() {
        parts.push(format!("Duplicate: {}", duplicates.join(", ")));
    }
    let unexpected: Vec<usize> = translated_markers
        .iter()
        .filter(|index| !source_markers.contains(index))
        .copied()
        .collect();
    if !unexpected.is_empty() {
        parts.push(format!(
            "Unexpected: {}",
            unexpected
                .iter()
                .map(|index| format!("[[B{index:03}]]"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if parts.is_empty() {
        // 数量相同但顺序/集合不同（乱序）。
        parts.push("Markers out of order".to_string());
    }
    Some(parts.join("; "))
}

fn extract_block_marker_indices(markdown: &str) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut cursor = 0usize;
    while let Some((marker_start, marker_end)) = find_next_block_alignment_marker(markdown, cursor)
    {
        if let Ok(index) = markdown[marker_start + 3..marker_end - 2].parse::<usize>() {
            indices.push(index);
        }
        cursor = marker_end;
    }
    indices
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// 重试降级（借鉴 round-based size halving）：第 attempt 次尝试时（1-based，
/// attempt≥3 生效），只发送源文的一个连续 block 切片，规模逐次减半。
/// 格式类错误（STRICT_GLOSSARY_HIT_MISSING / HEADING_LOSS / decode）随输入
/// 变短而显著缓解——这是 。
/// 返回 (切片文本, 起止 block 索引)；attempt<3 或块数不足时返回全量。
fn source_slice_for_attempt(source: &str, attempt: u8) -> (String, Option<(usize, usize)>) {
    let blocks = split_markdown_blocks(source);
    if attempt < 3 || blocks.len() < 2 {
        return (source.to_string(), None);
    }
    // 减半窗口：第3次取全长的 1/2，第4次 1/4，第5次 1/8（至少 1 块）。
    let divisor = 1usize << (attempt - 2).min(3);
    let window = (blocks.len() / divisor).max(1);
    // 轮流取不同切片窗口，避免每次重试撞同一片（第3次取前半，第4次取前 1/4...）。
    let start = ((usize::from(attempt) - 3) * window) % blocks.len().max(1);
    let end = (start + window).min(blocks.len());
    let slice = blocks[start..end].join("\n\n");
    (slice, Some((start, end)))
}

/// 把切片译文合并回全量译文：切片块位置被译文替换，其余块保留原文。
/// （被替换区域的英文原文将由 wave repair / residual review 后续处理，
/// 比整 chunk 失败卡死 pipeline 好。）
fn merge_sliced_translation(
    full_source: &str,
    translated_slice: &str,
    slice_range: (usize, usize),
) -> String {
    let (start, end) = slice_range;
    let blocks = split_markdown_blocks(full_source);
    if start >= blocks.len() || end > blocks.len() || start >= end {
        return translated_slice.to_string();
    }
    let mut merged = Vec::with_capacity(blocks.len() + 1);
    merged.extend_from_slice(&blocks[..start]);
    merged.push(translated_slice.to_string());
    merged.extend_from_slice(&blocks[end..]);
    merged.join("\n\n")
}

fn strip_leading_block_alignment_marker(block: &str) -> &str {
    let trimmed = block.trim_start();
    let Some(rest) = trimmed.strip_prefix("[[B") else {
        return block;
    };
    let Some((digits, after_marker)) = rest.split_once("]]") else {
        return block;
    };
    if digits.is_empty() || digits.len() > 6 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return block;
    }
    after_marker.trim_start()
}

fn record_llm_attempt_started(
    event_log_path: &Path,
    chunk_index: usize,
    attempt: u8,
    model: &str,
    source_len: usize,
    context_len: usize,
) {
    let Ok(_guard) = event_log_write_lock().lock() else {
        return;
    };
    let event = TaskEvent {
        timestamp: Utc::now().to_rfc3339(),
        stage: "llm_attempt_started".to_string(),
        message: format!(
            "chunk_index={chunk_index}; attempt={attempt}; model={model}; source_chars={source_len}; context_chars={context_len}"
        ),
        progress: 40,
        level: "debug".to_string(),
    };
    let _ = artifact::append_json_line(event_log_path, &event);
}

pub(crate) fn fill_translation_workers(
    join_set: &mut JoinSet<Result<TranslationWorkerResult, AppError>>,
    next_pending: &mut usize,
    pending_indexes: &[usize],
    max_parallel: usize,
    checkpoint: &ChunkCheckpoint,
    config: &PipelineRunConfig,
    llm_client: &SensenovaClient,
    system_prompt: &str,
    event_log_path: &Path,
) -> Result<(), AppError> {
    while join_set.len() < max_parallel && *next_pending < pending_indexes.len() {
        ensure_not_cancelled(&config.cancellation_token)?;
        let index = pending_indexes[*next_pending];
        *next_pending += 1;

        let source = source_for_chunk_index(checkpoint, index)?;
        let started_glossary_seq = checkpoint.delta_sync.glossary_seq;
        let delta_terms = checkpoint
            .delta_sync
            .wave_delta_log
            .iter()
            .filter(|term| term.seq <= started_glossary_seq)
            .cloned()
            .collect::<Vec<_>>();
        let base_translation_context = checkpoint.translation_context.as_ref().map(|context| {
            format_translation_context_for_chunk(context, &checkpoint.article_type, index, &source)
        });
        let dynamic_delta_context = format_dynamic_delta_context(&source, &delta_terms);
        let previous_context = previous_paragraph_context(checkpoint, index);
        let next_context = next_paragraph_context(checkpoint, index);
        let translation_context = merge_translation_context(vec![
            base_translation_context,
            dynamic_delta_context,
            previous_context,
            next_context,
        ]);
        let strict_hits = checkpoint
            .translation_context
            .as_ref()
            .map(|context| collect_strict_chunk_hits(context, &source))
            .unwrap_or_default();
        let source_len = source.chars().count();
        let context_len = translation_context
            .as_deref()
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or_default();
        let article_type = config.article_type.clone();
        let system_prompt = system_prompt.to_string();
        let limiter = Arc::clone(&config.global_llm_limiter);
        let client = llm_client.clone();
        let cancel = config.cancellation_token.clone();
        let event_log_path = event_log_path.to_path_buf();

        join_set.spawn(async move {
            if cancel.is_cancelled() {
                return Err(AppError::cancelled(
                    "translation cancelled before worker start",
                ));
            }
            let started = Instant::now();
            if source.trim().is_empty() {
                return Ok(TranslationWorkerResult::Done(TranslationOutcome {
                    index,
                    translated: source,
                    confirmed_terms: Vec::new(),
                    started_glossary_seq,
                    elapsed_ms: started.elapsed().as_millis(),
                    wait_ms: 0,
                    route_wait_ms: 0,
                    llm_elapsed_ms: 0,
                    llm_attempts: 0,
                    attempt_failures: Vec::new(),
                }));
            }
            let llm_attempts = Arc::new(Mutex::new(0u8));
            let llm_elapsed_ms = Arc::new(Mutex::new(0u128));
            let route_wait_ms = Arc::new(Mutex::new(0u128));
            let last_request_elapsed_ms = Arc::new(Mutex::new(0u128));
            let attempt_failures = Arc::new(Mutex::new(Vec::<LlmAttemptFailure>::new()));
            let _permit = limiter.acquire_owned().await.map_err(|error| {
                AppError::internal(format!("failed to acquire LLM permit: {error}"))
            })?;
            let wait_ms = started.elapsed().as_millis();
            let cancel_for_retry = cancel.clone();
            let cancel_for_attempts = cancel.clone();
            let translated_result = retry_async_with_observer(
                llm_retry_limit(),
                Duration::from_secs(2),
                || cancel_for_retry.is_cancelled(),
                |attempt| {
                    let client = client.clone();
                    let source = source.clone();
                    let article_type = article_type.clone();
                    let system_prompt = system_prompt.clone();
                    let translation_context = translation_context.clone();
                    let strict_hits = strict_hits.clone();
                    let llm_attempts = Arc::clone(&llm_attempts);
                    let llm_elapsed_ms = Arc::clone(&llm_elapsed_ms);
                    let route_wait_ms = Arc::clone(&route_wait_ms);
                    let last_request_elapsed_ms = Arc::clone(&last_request_elapsed_ms);
                    let event_log_path = event_log_path.clone();
                    let cancel = cancel_for_attempts.clone();
                    // heading 漏译兜底 + 3 次失败换模型：LLM_MARKDOWN_HEADING_LOSS 是可重试错误，
                    // 每次重试 reserve_route 会重新按健康度×速度权重选路由——连续失败的路由
                    // 健康度被打低，第 3 次（及以后）重试自然切到更优路由（换模型）。
                    let previous_failures = attempt.saturating_sub(1);
                    async move {
                        if let Ok(mut attempts) = llm_attempts.lock() {
                            *attempts = (*attempts).max(attempt);
                        }
                        let route = client.reserve_route();
                        let routing_key = route.routing_key().clone();
                        let route_wait_started = Instant::now();
                        let _route_permit = wait_for_llm_rate_slot(&routing_key, &cancel).await?;
                        if let Ok(mut total_route_wait_ms) = route_wait_ms.lock() {
                            *total_route_wait_ms += route_wait_started.elapsed().as_millis();
                        }
                        if previous_failures >= 2 {
                            // 已失败 ≥2 次（这是第 3+ 次尝试）：记录换路由事件便于归因。
                            record_model_switch_after_retries(
                                &event_log_path,
                                index,
                                attempt,
                                &routing_key.model,
                            );
                        }
                        record_llm_attempt_started(
                            &event_log_path,
                            index,
                            attempt,
                            &routing_key.model,
                            source_len,
                            context_len,
                        );
                        // 重试降级：第 3+ 次尝试只发送源文切片（逐次减半），
                        // 格式类错误随输入变短而缓解；成功后译文并回全量。
                        // MUSETRANSLATE_SLICED_RETRY=0 可关（回到全量重试）。
                        let (attempt_source, slice_range) =
                            if crate::pipeline::env_flag_disabled("MUSETRANSLATE_SLICED_RETRY") {
                                (source.clone(), None)
                            } else {
                                source_slice_for_attempt(&source, attempt)
                            };
                        if slice_range.is_some() {
                            record_chunk_sliced_retry(&event_log_path, index, attempt);
                        }
                        let marked_source = add_block_alignment_markers(&attempt_source);
                        let llm_started = Instant::now();
                        let translated = client
                            .translate_markdown_with_context_on_route(
                                &route,
                                &marked_source,
                                &article_type,
                                &system_prompt,
                                translation_context.as_deref(),
                            )
                            .await;
                        let request_elapsed_ms = llm_started.elapsed().as_millis();
                        if let Ok(mut total_elapsed_ms) = llm_elapsed_ms.lock() {
                            *total_elapsed_ms += request_elapsed_ms;
                        }
                        if let Ok(mut last_elapsed_ms) = last_request_elapsed_ms.lock() {
                            *last_elapsed_ms = request_elapsed_ms;
                        }
                        let mut translated = match translated {
                            Ok(value) => {
                                record_llm_success(&routing_key, request_elapsed_ms);
                                value
                            }
                            Err(error) => {
                                let error = AppError::from_provider_error(error);
                                client.maybe_activate_fallback_after_app_error(
                                    &routing_key.model,
                                    &error,
                                );
                                record_llm_retryable_error(
                                    &routing_key,
                                    &error,
                                    request_elapsed_ms,
                                );
                                return Err(error);
                            }
                        };
                        // Marker 完整性检查（剥标记之前）：模型丢/重/乱 [[Bxxx]] 时
                        // 生成精确差异报告并以可重试错误反馈（重试时模型可见上一次
                        // 失败原因的事件链；切片合并错位也在此拦截）。
                        if let Some(report) = block_marker_integrity_report(
                            &marked_source,
                            &translated.translated_text,
                        ) {
                            record_block_marker_mismatch(&event_log_path, index, attempt, &report);
                            return Err(AppError::new(
                                "LLM_BLOCK_MARKER_MISMATCH",
                                format!("chunk {index} block marker mismatch: {report}"),
                                true,
                            ));
                        }
                        translated.translated_text =
                            strip_block_alignment_markers(&translated.translated_text);
                        // 切片译文（合并前的 LLM 直接产出）保留用于校验。
                        let slice_translated = translated.translated_text.clone();
                        if let Some(range) = slice_range {
                            translated.translated_text = merge_sliced_translation(
                                &source,
                                &translated.translated_text,
                                range,
                            );
                        }
                        translated.translated_text = repair_strict_hit_aliases(
                            &translated.translated_text,
                            &translated.confirmed_terms,
                            &strict_hits,
                        );
                        // 切片重试时校验只针对切片（合并前）译文与切片源文；
                        // 全量校验必失败（未翻译区域的 heading/strict-hit 不在译文里）。
                        let validation_translated = if slice_range.is_some() {
                            slice_translated.as_str()
                        } else {
                            translated.translated_text.as_str()
                        };
                        reject_untranslated_body_passthrough(
                            article_type.as_str(),
                            attempt_source.as_str(),
                            validation_translated,
                        )?;
                        reject_markdown_heading_loss(
                            index,
                            attempt_source.as_str(),
                            validation_translated,
                        )?;
                        enforce_strict_chunk_hits(
                            index,
                            &strict_hits,
                            attempt_source.as_str(),
                            validation_translated,
                        )?;
                        synthesize_missing_strict_hit_terms(
                            &strict_hits,
                            &translated.translated_text,
                            &mut translated.confirmed_terms,
                        );
                        reject_mojibake_translation(index, &translated.translated_text)?;
                        Ok(translated)
                    }
                },
                |attempt, error, delay| {
                    let request_elapsed_ms = last_request_elapsed_ms
                        .lock()
                        .map(|value| *value)
                        .unwrap_or_default();
                    if let Ok(mut failures) = attempt_failures.lock() {
                        failures.push(LlmAttemptFailure {
                            attempt,
                            code: error.code,
                            message_preview: preview_error_message(&error.message),
                            request_elapsed_ms,
                            delay_ms: delay.as_millis(),
                        });
                    }
                },
            )
            .await;
            let translated = match translated_result {
                Ok(value) => value,
                Err(error) => {
                    // 块级失败隔离：仅取消类错误向上传播终止任务；
                    // 其余重试耗尽错误转为 Exhausted，由主循环标记该块失败并继续。
                    if error.code == "CANCELLED" {
                        return Err(error);
                    }
                    let failures = attempt_failures
                        .lock()
                        .map(|value| value.clone())
                        .unwrap_or_default();
                    let attempts = llm_attempts.lock().map(|value| *value).unwrap_or_default();
                    return Ok(TranslationWorkerResult::Exhausted {
                        index,
                        attempts,
                        failures,
                    });
                }
            };
            let llm_attempts = llm_attempts.lock().map(|value| *value).unwrap_or_default();
            let llm_elapsed_ms = llm_elapsed_ms
                .lock()
                .map(|value| *value)
                .unwrap_or_default();
            let route_wait_ms = route_wait_ms.lock().map(|value| *value).unwrap_or_default();
            let attempt_failures = attempt_failures
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            Ok(TranslationWorkerResult::Done(TranslationOutcome {
                index,
                translated: translated.translated_text,
                confirmed_terms: translated.confirmed_terms,
                started_glossary_seq,
                elapsed_ms: started.elapsed().as_millis(),
                wait_ms,
                route_wait_ms,
                llm_elapsed_ms,
                llm_attempts,
                attempt_failures,
            }))
        });
    }

    Ok(())
}

fn source_for_chunk_index(checkpoint: &ChunkCheckpoint, index: usize) -> Result<String, AppError> {
    checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index == index)
        .map(|chunk| chunk.source.clone())
        .ok_or_else(|| AppError::internal("missing source chunk while scheduling translation"))
}

fn reject_markdown_heading_loss(
    chunk_index: usize,
    source: &str,
    translated: &str,
) -> Result<(), AppError> {
    let source_headings = markdown_heading_levels(source);
    if source_headings.is_empty() {
        return Ok(());
    }
    let translated_headings = markdown_heading_levels(translated);
    if translated_headings.len() >= source_headings.len() {
        return Ok(());
    }
    Err(AppError::new(
        "LLM_MARKDOWN_HEADING_LOSS",
        format!(
            "chunk {chunk_index} lost markdown headings: source_headings={}, translated_headings={}",
            source_headings.len(),
            translated_headings.len()
        ),
        true,
    ))
}

// heading 漏译兜底：第 3 次（及以后）重试时记录换路由事件。
// reserve_route 按健康度×速度权重选路由，连续失败的路由健康度被打低，
// 第 3 次重试自然切到更优路由（换模型）；此事件便于复测归因。
fn record_model_switch_after_retries(
    event_log_path: &std::path::Path,
    chunk_index: usize,
    attempt: u8,
    model: &str,
) {
    let event = crate::pipeline::TaskEvent {
        timestamp: chrono::Utc::now().to_rfc3339(),
        stage: "llm_model_switch_after_retries".to_string(),
        message: format!(
            "chunk_index={chunk_index}; attempt={attempt}; model={model}; reason=retry_after_repeated_failures"
        ),
        progress: 40,
        level: "info".to_string(),
    };
    let _ = crate::pipeline::artifact::append_json_line(event_log_path, &event);
}

// 切片重试事件：第 3+ 次尝试只发送源文的一部分（减半降级策略）。
fn record_chunk_sliced_retry(event_log_path: &std::path::Path, chunk_index: usize, attempt: u8) {
    let event = crate::pipeline::TaskEvent {
        timestamp: chrono::Utc::now().to_rfc3339(),
        stage: "llm_chunk_sliced_retry".to_string(),
        message: format!(
            "chunk_index={chunk_index}; attempt={attempt}; reason=size_halving_retry_degradation"
        ),
        progress: 40,
        level: "info".to_string(),
    };
    let _ = crate::pipeline::artifact::append_json_line(event_log_path, &event);
}

// Marker 差异事件：记录精确的 missing/duplicate/unexpected 报告（重试归因）。
fn record_block_marker_mismatch(
    event_log_path: &std::path::Path,
    chunk_index: usize,
    attempt: u8,
    report: &str,
) {
    let event = crate::pipeline::TaskEvent {
        timestamp: chrono::Utc::now().to_rfc3339(),
        stage: "llm_block_marker_mismatch".to_string(),
        message: format!("chunk_index={chunk_index}; attempt={attempt}; report={report}"),
        progress: 40,
        level: "warn".to_string(),
    };
    let _ = crate::pipeline::artifact::append_json_line(event_log_path, &event);
}

fn markdown_heading_levels(markdown: &str) -> Vec<u8> {
    markdown
        .lines()
        .filter_map(markdown_heading_level)
        .collect()
}

fn markdown_heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start();
    let marker_count = trimmed.chars().take_while(|ch| *ch == '#').count();
    if marker_count == 0 || marker_count > 6 {
        return None;
    }
    let rest = trimmed.get(marker_count..)?;
    if !rest.starts_with(' ') || rest.trim().is_empty() {
        return None;
    }
    Some(marker_count as u8)
}

fn enforce_strict_chunk_hits(
    chunk_index: usize,
    strict_hits: &[super::super::StrictChunkHit],
    source: &str,
    translated_text: &str,
) -> Result<(), AppError> {
    let missing = strict_hits
        .iter()
        // 误判护栏：strict hit 来自 collect_strict_chunk_hits 的模糊匹配
        // （shared_sequence_ratio≥0.8 等宽松阈值），会把不含该专名的源文误判为
        // 「提及」。复核：用词边界精确匹配验证源文是否真的含该专名，
        // 不含则该 hit 是误判，跳过 enforce——否则模型永远翻不出源文里没有的词，
        // 重试 5 次全失败、pipeline 中止（实测 chunk29/30 的 SimonandSchuster）。
        .filter(|hit| source_mentions_exact(source, &hit.source))
        .filter(|hit| !translated_text.contains(&hit.target))
        .map(|hit| format!("{} -> {}", hit.source, hit.target))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        // strict 命中缺失降级为「重试可解」而非「内部错误」：
        // ①模型漏翻（真问题，换模型重试可解）；②匹配器误判（假问题，上面已护栏）。
        Err(AppError::new(
            "STRICT_GLOSSARY_HIT_MISSING",
            format!(
                "strict glossary hits missing from translated chunk={chunk_index}: {}",
                missing.join("; ")
            ),
            true,
        ))
    }
}

/// 词边界精确匹配：源文是否含该专名（作为完整词/短语，不做模糊匹配）。
/// 用于复核 collect_strict_chunk_hits 的模糊匹配结果，剔除误判。
fn source_mentions_exact(source: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }
    // 非 ASCII（中文等）直接子串匹配
    if !term.is_ascii() {
        return source.contains(term);
    }
    // ASCII：词边界匹配（两侧非字母数字）
    let mut start = 0usize;
    while let Some(pos) = source[start..].find(term) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !source[..abs]
                .chars()
                .next_back()
                .map(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                .unwrap_or(false);
        let after = &source[abs + term.len()..];
        let after_ok = after
            .chars()
            .next()
            .map(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        start = abs + term.len();
    }
    false
}

fn reject_untranslated_body_passthrough(
    article_type: &str,
    source: &str,
    translated_text: &str,
) -> Result<(), AppError> {
    // 仅 academic 对"译文≈英文原文"做硬拒绝（触发重试）；
    // 其他类型允许叙事性保留原文。
    if !crate::article_policy::ArticlePolicy::from_article_type(article_type).is_academic() {
        return Ok(());
    }
    if !looks_like_english_body_prose(source) {
        return Ok(());
    }
    if !is_near_source_passthrough(source, translated_text) {
        return Ok(());
    }
    Err(AppError::from_provider_error(
        crate::error::ProviderError::new(
            "llm",
            crate::error::ProviderErrorKind::Decode,
            "academic body translation remained effectively identical to English source",
        ),
    ))
}

fn looks_like_english_body_prose(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 160 {
        return false;
    }
    let letter_count = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let cjk_count = trimmed
        .chars()
        .filter(|ch| ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    letter_count >= 120 && letter_count > cjk_count * 4
}

fn is_near_source_passthrough(source: &str, translated_text: &str) -> bool {
    let normalized_source = normalize_passthrough_compare_text(source);
    let normalized_translated = normalize_passthrough_compare_text(translated_text);
    if normalized_source.len() < 80 || normalized_translated.len() < 80 {
        return false;
    }
    if normalized_source == normalized_translated {
        return true;
    }
    // 长度带早退：Levenshtein 在 |len_diff| / max_len > 1 - 0.94 = 0.06 时不可能 ≥0.94。
    let max_len = normalized_source.len().max(normalized_translated.len());
    let len_diff = normalized_source
        .len()
        .abs_diff(normalized_translated.len());
    if len_diff as f32 / max_len as f32 > 0.06 {
        return false;
    }
    // 采样分段哈希/首 80 字符精确比较，仅相近时才跑 DP。
    let sample_len = 80
        .min(normalized_source.len())
        .min(normalized_translated.len());
    if normalized_source[..sample_len] != normalized_translated[..sample_len] {
        // 首 80 字符不同 → 可能整体相似度 <0.94，但也可能只是开头差异。
        // 再采样中段：若中段也完全不同，直接 false。
        let mid_start_source = normalized_source.len() / 2;
        let mid_start_translated = normalized_translated.len() / 2;
        let mid_len = 40
            .min(normalized_source.len() - mid_start_source)
            .min(normalized_translated.len() - mid_start_translated);
        if mid_len > 0
            && normalized_source[mid_start_source..mid_start_source + mid_len]
                != normalized_translated[mid_start_translated..mid_start_translated + mid_len]
        {
            return false;
        }
    }
    normalized_similarity(&normalized_source, &normalized_translated) >= 0.94
}

fn normalize_passthrough_compare_text(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn normalized_similarity(left: &str, right: &str) -> f32 {
    crate::pipeline::context::normalized_similarity(left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ChunkSegmentKind, ChunkState};

    #[test]
    fn source_lookup_uses_chunk_index_not_vec_position() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 1000,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![chunk(7, "目标正文"), chunk(0, "前置正文")],
        };

        assert_eq!(
            source_for_chunk_index(&checkpoint, 7).expect("source should exist"),
            "目标正文"
        );
    }

    #[test]
    fn markdown_heading_loss_is_retryable() {
        let error = reject_markdown_heading_loss(
            12,
            "## Declarations\n\n- Funding: supported by a grant.",
            "- 资助：由基金支持。",
        )
        .expect_err("missing translated heading should be rejected");

        assert_eq!(error.code, "LLM_MARKDOWN_HEADING_LOSS");
        assert!(error.retryable);
    }

    #[test]
    fn markdown_heading_loss_allows_preserved_heading_count() {
        reject_markdown_heading_loss(
            12,
            "## Declarations\n\n- Funding: supported by a grant.",
            "## 声明\n\n- 资助：由基金支持。",
        )
        .expect("preserved heading count should pass");
    }

    #[test]
    fn previous_paragraph_context_uses_previous_chunk_tail_only() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: None,
            source_hash: "hash".to_string(),
            chunk_size: 1000,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![
                chunk(0, "First paragraph.\n\nPrevious tail paragraph."),
                chunk(1, "Current paragraph."),
            ],
        };

        let context =
            previous_paragraph_context(&checkpoint, 1).expect("previous paragraph should exist");

        assert!(context.contains("Previous tail paragraph."));
        assert!(!context.contains("First paragraph."));
        assert!(context.contains("Do not translate, rewrite, quote, summarize, or include"));
    }

    #[test]
    fn block_alignment_markers_are_added_per_markdown_block() {
        let marked =
            add_block_alignment_markers("## Title\n\nFirst paragraph.\n\nSecond paragraph.");

        assert_eq!(
            marked,
            "[[B000]]\n## Title\n\n[[B001]]\nFirst paragraph.\n\n[[B002]]\nSecond paragraph."
        );
    }

    #[test]
    fn block_alignment_markers_are_stripped_before_checkpoint_storage() {
        let translated = [
            "[[B000]]",
            "## Title ZH",
            "[[B001]]",
            "First paragraph ZH.",
            "[[B002]] Second paragraph ZH.",
        ]
        .join("\n");

        assert_eq!(
            strip_block_alignment_markers(&translated),
            "## Title ZH\n\nFirst paragraph ZH.\n\nSecond paragraph ZH."
        );
    }

    fn chunk(index: usize, source: &str) -> ChunkState {
        ChunkState {
            index,
            source: source.to_string(),
            translated: None,
            confirmed_terms: Vec::new(),
            attempts: 0,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }
    }

    #[test]
    fn rejects_near_source_passthrough_for_academic_english_body() {
        let source = "The unprecedented growth of the internet and social media platforms has been accompanied both by an abundance of content and by the spread of misinformation. This long paragraph remains fully English and should not be accepted as a translation.";
        let translated = source;

        let error = reject_untranslated_body_passthrough("academic", source, translated)
            .expect_err("academic English passthrough must be rejected");

        assert_eq!(error.code, "LLM_DECODE_ERROR");
    }

    #[test]
    fn source_slice_returns_full_for_early_attempts() {
        let source = "Para one.\n\nPara two.\n\nPara three.\n\nPara four.";
        let (slice, range) = source_slice_for_attempt(source, 1);
        assert_eq!(slice, source);
        assert!(range.is_none());
        let (slice, range) = source_slice_for_attempt(source, 2);
        assert_eq!(slice, source);
        assert!(range.is_none());
    }

    #[test]
    fn source_slice_halves_window_from_third_attempt() {
        let source = "Para one.\n\nPara two.\n\nPara three.\n\nPara four.";
        // 第3次：全长 1/2（2 块）
        let (slice, range) = source_slice_for_attempt(source, 3);
        assert_eq!(range, Some((0, 2)));
        assert_eq!(slice, "Para one.\n\nPara two.");
        // 第4次：全长 1/4（1 块），窗口游标前进
        let (slice, range) = source_slice_for_attempt(source, 4);
        assert_eq!(range, Some((1, 2)));
        assert_eq!(slice, "Para two.");
        // 第5次：全长 1/8 → 至少 1 块
        let (slice, range) = source_slice_for_attempt(source, 5);
        assert_eq!(range, Some((2, 3)));
        assert_eq!(slice, "Para three.");
    }

    #[test]
    fn source_slice_keeps_single_block_sources_whole() {
        let source = "Only one paragraph.";
        let (slice, range) = source_slice_for_attempt(source, 5);
        assert_eq!(slice, source);
        assert!(range.is_none());
    }

    #[test]
    fn merge_sliced_translation_replaces_only_slice_range() {
        let full_source = "Para one.\n\nPara two.\n\nPara three.\n\nPara four.";
        let merged = merge_sliced_translation(full_source, "第二段译文。", (1, 2));
        assert_eq!(
            merged,
            "Para one.\n\n第二段译文。\n\nPara three.\n\nPara four."
        );
    }

    #[test]
    fn merge_sliced_translation_rejects_out_of_range() {
        let full_source = "Para one.\n\nPara two.";
        let merged = merge_sliced_translation(full_source, "译文", (5, 9));
        assert_eq!(merged, "译文");
    }

    #[test]
    fn marker_integrity_passes_when_sequences_match() {
        let source = "[[B000]]\nOne.\n\n[[B001]]\nTwo.";
        let translated = "[[B000]]\n一。\n\n[[B001]]\n二。";
        assert!(block_marker_integrity_report(source, translated).is_none());
    }

    #[test]
    fn marker_integrity_reports_missing_duplicate_unexpected() {
        let source = "[[B000]]\nOne.\n\n[[B001]]\nTwo.\n\n[[B002]]\nThree.";
        let translated =
            "[[B000]]\n一。\n\n[[B001]]\n二。\n\n[[B001]]\n二重复。\n\n[[B009]]\n意外。";
        let report =
            block_marker_integrity_report(source, translated).expect("mismatch should be reported");
        assert!(report.contains("Missing: [[B002]]"), "report: {report}");
        assert!(
            report.contains("Duplicate: [[B001]] appears 2 times"),
            "report: {report}"
        );
        assert!(report.contains("Unexpected: [[B009]]"), "report: {report}");
    }

    #[test]
    fn marker_integrity_reports_out_of_order() {
        let source = "[[B000]]\nOne.\n\n[[B001]]\nTwo.";
        let translated = "[[B001]]\n二。\n\n[[B000]]\n一。";
        let report = block_marker_integrity_report(source, translated)
            .expect("out of order should be reported");
        assert!(report.contains("out of order"), "report: {report}");
    }

    #[test]
    fn allows_translated_academic_body() {
        let source = "The unprecedented growth of the internet and social media platforms has been accompanied both by an abundance of content and by the spread of misinformation. This long paragraph remains fully English in the source.";
        let translated = "互联网和社交媒体平台前所未有的发展，既带来了海量内容，也伴随着错误信息的传播。这一长段正文已经被翻译成中文。";

        reject_untranslated_body_passthrough("academic", source, translated)
            .expect("translated academic body should pass");
    }
}
