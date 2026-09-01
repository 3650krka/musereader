use super::super::super::consistency::sanitize_dynamic_confirmed_term;
use super::super::super::context::repair_canonical_proper_nouns;
#[cfg(test)]
use super::collect::collect_chunk_residual_terms;
use super::collect::{
    collect_chunk_residual_terms_with_patch_terms, collect_pending_residual_review_stats,
    collect_residual_review_chunks,
};
use super::parse::parse_residual_repair;
use crate::llm::SensenovaClient;
use crate::pipeline::{
    artifact, checkpoint, ensure_not_cancelled, record_llm_retryable_error, record_llm_success,
    retry_async_with_observer, route_profile_recommended_parallel, wait_for_llm_rate_slot,
    AppError, ArtifactPaths, ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind,
    PipelineProgressStage, PipelineRunConfig, ResidualReviewChunk, ResidualReviewOutcome,
    RuntimePromptConfig, TaskEvent,
};
use chrono::Utc;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant};

pub(crate) async fn review_residual_english_candidates(
    llm_client: &SensenovaClient,
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    checkpoint: &mut ChunkCheckpoint,
    progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
    mut record_event: impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let review_chunks = collect_residual_review_chunks(checkpoint);
    if review_chunks.is_empty() {
        let advanced_chunks = mark_residual_review_complete(checkpoint);
        if advanced_chunks > 0 {
            checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
        }
        record_second_residual_check(checkpoint, &mut record_event)?;
        return Ok(());
    }

    record_event(
        "validation_review",
        &format!(
            "reviewing {} residual candidates across {} chunks",
            review_chunks
                .iter()
                .map(|chunk| chunk.terms.len())
                .sum::<usize>(),
            review_chunks.len()
        ),
        92,
    )?;
    progress_callback(
        PipelineProgressStage::ResidualReview,
        "reviewing residual English candidates",
        92,
    );

    let mut completed = 0usize;
    let mut next_pending = 0usize;
    let max_parallel = residual_review_parallel_limit(config, llm_client);
    record_event(
        "validation_review_runtime",
        &format!(
            "configured_parallel={}; effective_parallel={max_parallel}",
            config.max_parallel.max(1)
        ),
        92,
    )?;
    let mut join_set = JoinSet::new();
    fill_residual_review_workers(
        &mut join_set,
        &mut next_pending,
        &review_chunks,
        max_parallel,
        config,
        llm_client,
        &runtime_prompt.article_type,
        &paths.event_log_path,
    )?;

    while completed < review_chunks.len() {
        ensure_not_cancelled(&config.cancellation_token)?;
        if join_set.is_empty() {
            break;
        }
        let result = join_set
            .join_next()
            .await
            .ok_or_else(|| AppError::internal("review worker set ended unexpectedly"))?;
        let outcome = result
            .map_err(|error| AppError::internal(format!("review worker join failed: {error}")))??;
        completed += 1;

        if let Some(message) = outcome.skipped_message {
            record_event("validation_review_skipped", &message, 93)?;
        }

        if let Some(repaired) = outcome.repaired {
            if let Some(chunk) = checkpoint.chunks.get_mut(outcome.chunk_index) {
                chunk.translated = Some(repair_canonical_proper_nouns(
                    &repaired,
                    checkpoint.translation_context.as_ref(),
                ));
                merge_residual_confirmed_terms(
                    chunk,
                    &runtime_prompt.article_type,
                    &outcome.confirmed_terms,
                );
                checkpoint::advance_chunk_processing_stage(
                    chunk,
                    ChunkProcessingStage::ResidualReviewed,
                );
                chunk.updated_at = Utc::now().to_rfc3339();
                checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
                record_event(
                    "validation_repaired",
                    &format!("applied residual repair to chunk={}", outcome.chunk_index),
                    93,
                )?;
            }
        }

        fill_residual_review_workers(
            &mut join_set,
            &mut next_pending,
            &review_chunks,
            max_parallel,
            config,
            llm_client,
            &runtime_prompt.article_type,
            &paths.event_log_path,
        )?;
    }

    let advanced_chunks = mark_residual_review_complete(checkpoint);
    if advanced_chunks > 0 {
        checkpoint::persist_dirty_checkpoint(paths, checkpoint)?;
    }
    record_second_residual_check(checkpoint, &mut record_event)?;

    Ok(())
}

fn record_second_residual_check(
    checkpoint: &ChunkCheckpoint,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let stats = collect_pending_residual_review_stats(checkpoint);
    record_event(
        "validation_review_second_check",
        &format!(
            "remaining_candidate_chunks={}; remaining_candidate_terms={}; candidate_limit={}; hit_candidate_limit={}",
            stats.candidate_chunk_count,
            stats.candidate_term_count,
            stats.candidate_limit,
            stats.hit_candidate_limit
        ),
        93,
    )
}

fn mark_residual_review_complete(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut advanced = 0usize;
    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body || chunk.translated.is_none() {
            continue;
        }
        if checkpoint::advance_chunk_processing_stage(chunk, ChunkProcessingStage::ResidualReviewed)
        {
            advanced += 1;
        }
    }
    advanced
}

fn residual_review_parallel_limit(
    config: &PipelineRunConfig,
    llm_client: &SensenovaClient,
) -> usize {
    let configured = config.max_parallel.max(1);
    let summed_route_parallel = llm_client
        .active_routing_keys()
        .iter()
        .map(|routing_key| route_profile_recommended_parallel(routing_key, configured))
        .sum::<usize>();
    // 解除按 config.max_parallel 的钳制：residual 阶段无 wave 术语回补语义，
    // 可安全使用路由画像学到的容量。上限由全局信号量（global_llm_limiter）
    // 与路由级 RPM/并发许可（wait_for_llm_rate_slot）共同保证，超发不会超配额。
    summed_route_parallel.max(1)
}

fn fill_residual_review_workers(
    join_set: &mut JoinSet<Result<ResidualReviewOutcome, AppError>>,
    next_pending: &mut usize,
    review_chunks: &[ResidualReviewChunk],
    max_parallel: usize,
    config: &PipelineRunConfig,
    llm_client: &SensenovaClient,
    article_type: &str,
    event_log_path: &Path,
) -> Result<(), AppError> {
    while join_set.len() < max_parallel && *next_pending < review_chunks.len() {
        ensure_not_cancelled(&config.cancellation_token)?;
        let review_chunk = review_chunks[*next_pending].clone();
        *next_pending += 1;

        let article_type = article_type.to_string();
        let sentence_plans = build_sentence_patch_plans(
            &review_chunk.source,
            &review_chunk.translated,
            &review_chunk.terms,
            &review_chunk.patch_terms,
        );
        let limiter = Arc::clone(&config.global_llm_limiter);
        let client = llm_client.clone();
        let cancel = config.cancellation_token.clone();
        let event_log_path = event_log_path.to_path_buf();

        join_set.spawn(async move {
            if cancel.is_cancelled() {
                return Err(AppError::cancelled(
                    super::super::super::runtime::OPERATION_CANCELLED_MESSAGE,
                ));
            }
            let _permit = limiter.acquire_owned().await.map_err(|error| {
                AppError::internal(format!("failed to acquire residual review permit: {error}"))
            })?;
            let (repaired, confirmed_terms) = match apply_sentence_first_residual_review(
                &client,
                &cancel,
                &article_type,
                &review_chunk,
                &sentence_plans,
                &event_log_path,
            )
            .await
            {
                Ok(result) => result,
                Err(app_error) => {
                    return Ok(ResidualReviewOutcome {
                        chunk_index: review_chunk.chunk_index,
                        repaired: None,
                        confirmed_terms: Vec::new(),
                        skipped_message: Some(format!(
                            "chunk={} residual review skipped: {}",
                            review_chunk.chunk_index, app_error.message
                        )),
                    });
                }
            };

            Ok(ResidualReviewOutcome {
                chunk_index: review_chunk.chunk_index,
                repaired,
                confirmed_terms,
                skipped_message: None,
            })
        });
    }

    Ok(())
}

#[derive(Clone)]
struct ResidualSentencePlan {
    target: crate::translation_validation::SentencePatchTarget,
    patch_terms: Vec<(String, String)>,
}

#[derive(Clone)]
struct ResidualExcerptPlan {
    source_excerpt: String,
    translated_excerpt: String,
    patch_terms: Vec<(String, String)>,
}

enum ResidualRepairScope {
    Sentence,
    Paragraph(ResidualExcerptPlan),
    AdjacentParagraphs(ResidualExcerptPlan),
    Chunk,
}

fn build_sentence_patch_plans(
    source_text: &str,
    translated_text: &str,
    terms: &[crate::translation_validation::ResidualEnglishTerm],
    patch_terms: &[crate::pipeline::ResidualPatchTerm],
) -> Vec<ResidualSentencePlan> {
    let mut grouped = BTreeMap::<String, ResidualSentencePlan>::new();
    for term in terms {
        let Some(target) =
            crate::pipeline::review::locate_patch_target(source_text, translated_text, term)
        else {
            continue;
        };
        let key = target.translated_sentence.clone();
        let entry = grouped.entry(key).or_insert_with(|| ResidualSentencePlan {
            target,
            patch_terms: Vec::new(),
        });
        for patch_term in patch_terms_for_residual_term(term, patch_terms) {
            if !entry.patch_terms.iter().any(|item| item == &patch_term) {
                entry.patch_terms.push(patch_term);
            }
        }
    }
    grouped.into_values().collect()
}

fn choose_residual_repair_scope(
    review_chunk: &ResidualReviewChunk,
    sentence_plans: &[ResidualSentencePlan],
) -> ResidualRepairScope {
    if sentence_plans.is_empty() {
        return ResidualRepairScope::Chunk;
    }
    if review_chunk.terms.len() >= 6 {
        return ResidualRepairScope::Chunk;
    }
    if sentence_plans.len() == 1 {
        return ResidualRepairScope::Sentence;
    }

    let paragraph_plans = build_excerpt_patch_plans(
        &review_chunk.source,
        &review_chunk.translated,
        sentence_plans,
    );
    if paragraph_plans.len() == 1 {
        return ResidualRepairScope::Paragraph(paragraph_plans[0].plan.clone());
    }
    if paragraph_plans.len() == 2
        && paragraph_plans[0].translated_index + 1 == paragraph_plans[1].translated_index
    {
        return ResidualRepairScope::AdjacentParagraphs(merge_adjacent_excerpt_plans(
            &paragraph_plans[0],
            &paragraph_plans[1],
        ));
    }
    ResidualRepairScope::Chunk
}

#[derive(Clone)]
struct IndexedExcerptPlan {
    translated_index: usize,
    source_index: Option<usize>,
    plan: ResidualExcerptPlan,
}

fn build_excerpt_patch_plans(
    source_text: &str,
    translated_text: &str,
    sentence_plans: &[ResidualSentencePlan],
) -> Vec<IndexedExcerptPlan> {
    let source_blocks = split_paragraph_blocks(source_text);
    let translated_blocks = split_paragraph_blocks(translated_text);
    let mut grouped = BTreeMap::<usize, IndexedExcerptPlan>::new();

    for sentence_plan in sentence_plans {
        let Some(translated_index) = find_block_containing(
            &translated_blocks,
            &sentence_plan.target.translated_sentence,
        ) else {
            continue;
        };
        let source_index =
            find_block_containing(&source_blocks, &sentence_plan.target.source_sentence);
        let translated_excerpt = translated_blocks[translated_index].text.clone();
        let source_excerpt = source_index
            .and_then(|index| source_blocks.get(index))
            .map(|span| span.text.clone())
            .unwrap_or_else(|| sentence_plan.target.source_context.clone());

        let entry = grouped
            .entry(translated_index)
            .or_insert_with(|| IndexedExcerptPlan {
                translated_index,
                source_index,
                plan: ResidualExcerptPlan {
                    source_excerpt,
                    translated_excerpt,
                    patch_terms: Vec::new(),
                },
            });
        append_unique_patch_terms(&mut entry.plan.patch_terms, &sentence_plan.patch_terms);
    }

    grouped.into_values().collect()
}

fn merge_adjacent_excerpt_plans(
    first: &IndexedExcerptPlan,
    second: &IndexedExcerptPlan,
) -> ResidualExcerptPlan {
    let source_excerpt = match (first.source_index, second.source_index) {
        (Some(left), Some(right)) if left + 1 == right => {
            format!(
                "{}\n\n{}",
                first.plan.source_excerpt, second.plan.source_excerpt
            )
        }
        _ => format!(
            "{}\n\n{}",
            first.plan.source_excerpt, second.plan.source_excerpt
        ),
    };
    let translated_excerpt = format!(
        "{}\n\n{}",
        first.plan.translated_excerpt, second.plan.translated_excerpt
    );
    let mut patch_terms = first.plan.patch_terms.clone();
    append_unique_patch_terms(&mut patch_terms, &second.plan.patch_terms);
    ResidualExcerptPlan {
        source_excerpt,
        translated_excerpt,
        patch_terms,
    }
}

fn append_unique_patch_terms(target: &mut Vec<(String, String)>, incoming: &[(String, String)]) {
    for term in incoming {
        if !target.iter().any(|existing| existing == term) {
            target.push(term.clone());
        }
    }
}

struct TextBlock {
    text: String,
}

fn split_paragraph_blocks(text: &str) -> Vec<TextBlock> {
    let marker_blocks = split_paragraph_blocks_by_markers(text);
    if !marker_blocks.is_empty() {
        return marker_blocks
            .into_iter()
            .map(|block| TextBlock { text: block })
            .collect();
    }
    text.split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(|block| TextBlock {
            text: block.to_string(),
        })
        .collect()
}

fn split_paragraph_blocks_by_markers(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    let mut current_block_start = None::<usize>;

    while let Some((marker_start, marker_end)) = find_next_block_marker(text, cursor) {
        if let Some(block_start) = current_block_start {
            let block = text[block_start..marker_start].trim();
            if !block.is_empty() {
                blocks.push(block.to_string());
            }
        }
        current_block_start = Some(marker_end);
        cursor = marker_end;
    }

    if let Some(block_start) = current_block_start {
        let block = text[block_start..].trim();
        if !block.is_empty() {
            blocks.push(block.to_string());
        }
    }
    blocks
}

fn find_next_block_marker(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let mut search_start = cursor;
    while search_start < text.len() {
        let relative_start = text[search_start..].find("[[B")?;
        let marker_start = search_start + relative_start;
        let digits_start = marker_start + 3;
        let relative_end = text[digits_start..].find("]]")?;
        let digits_end = digits_start + relative_end;
        let digits = &text[digits_start..digits_end];
        let marker_end = digits_end + 2;
        if !digits.is_empty() && digits.len() <= 6 && digits.chars().all(|ch| ch.is_ascii_digit()) {
            return Some((marker_start, marker_end));
        }
        search_start = marker_start + 3;
    }
    None
}

fn find_block_containing(blocks: &[TextBlock], needle: &str) -> Option<usize> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    blocks.iter().position(|block| block.text.contains(needle))
}

fn patch_terms_for_residual_term(
    term: &crate::translation_validation::ResidualEnglishTerm,
    patch_terms: &[crate::pipeline::ResidualPatchTerm],
) -> Vec<(String, String)> {
    let matched = patch_terms
        .iter()
        .filter(|patch_term| residual_term_matches_patch_term(&term.term, patch_term))
        .take(8)
        .map(|patch_term| (patch_term.source.clone(), patch_term.target.clone()))
        .collect::<Vec<_>>();
    if matched.is_empty() {
        vec![(term.term.clone(), String::new())]
    } else {
        matched
    }
}

fn residual_term_matches_patch_term(
    residual_term: &str,
    patch_term: &crate::pipeline::ResidualPatchTerm,
) -> bool {
    crate::pipeline::context::source_mentions_hint(&patch_term.source, residual_term)
        || crate::pipeline::context::source_mentions_hint(residual_term, &patch_term.source)
}

async fn apply_sentence_first_residual_review(
    client: &SensenovaClient,
    cancel: &tokio_util::sync::CancellationToken,
    article_type: &str,
    review_chunk: &ResidualReviewChunk,
    sentence_plans: &[ResidualSentencePlan],
    event_log_path: &Path,
) -> Result<(Option<String>, Vec<crate::llm::ConfirmedTerm>), AppError> {
    let mut translated = review_chunk.translated.clone();
    let mut changed = false;
    let mut confirmed_terms = Vec::new();

    match choose_residual_repair_scope(review_chunk, sentence_plans) {
        ResidualRepairScope::Chunk => {
            return apply_chunk_first_residual_review(
                client,
                cancel,
                article_type,
                review_chunk,
                event_log_path,
            )
            .await;
        }
        ResidualRepairScope::Sentence => {
            for plan in sentence_plans {
                let (did_change, new_terms) = apply_sentence_patch(
                    client,
                    cancel,
                    event_log_path,
                    review_chunk.chunk_index,
                    article_type,
                    plan,
                    &mut translated,
                )
                .await?;
                confirmed_terms.extend(new_terms);
                if did_change {
                    changed = true;
                }
            }
        }
        ResidualRepairScope::Paragraph(plan) => {
            let (did_change, new_terms) = apply_excerpt_patch(
                client,
                cancel,
                event_log_path,
                review_chunk.chunk_index,
                article_type,
                "paragraph",
                &plan,
                &mut translated,
            )
            .await?;
            confirmed_terms.extend(new_terms);
            if did_change {
                changed = true;
            }
        }
        ResidualRepairScope::AdjacentParagraphs(plan) => {
            let (did_change, new_terms) = apply_excerpt_patch(
                client,
                cancel,
                event_log_path,
                review_chunk.chunk_index,
                article_type,
                "adjacent_paragraphs",
                &plan,
                &mut translated,
            )
            .await?;
            confirmed_terms.extend(new_terms);
            if did_change {
                changed = true;
            }
        }
    }

    let remaining_terms = collect_chunk_residual_terms_with_patch_terms(
        article_type,
        &review_chunk.source,
        &translated,
        &review_chunk.patch_terms,
        usize::MAX,
    );
    if remaining_terms.is_empty() {
        return Ok((changed.then_some(translated), confirmed_terms));
    }

    let Some((repaired, new_terms)) = request_chunk_residual_repair(
        client,
        cancel,
        event_log_path,
        review_chunk.chunk_index,
        article_type,
        &review_chunk.source,
        &translated,
        &remaining_terms,
        "chunk",
    )
    .await?
    else {
        return Ok((changed.then_some(translated), confirmed_terms));
    };
    confirmed_terms.extend(new_terms);

    let second_remaining = collect_chunk_residual_terms_with_patch_terms(
        article_type,
        &review_chunk.source,
        &repaired,
        &review_chunk.patch_terms,
        usize::MAX,
    );
    if second_remaining.is_empty() {
        return Ok((Some(repaired), confirmed_terms));
    }

    let second_repaired = request_chunk_residual_repair(
        client,
        cancel,
        event_log_path,
        review_chunk.chunk_index,
        article_type,
        &review_chunk.source,
        &repaired,
        &second_remaining,
        "chunk_second",
    )
    .await?;

    Ok((
        second_repaired
            .map(|(text, extra_terms)| {
                confirmed_terms.extend(extra_terms);
                text
            })
            .or(Some(repaired)),
        confirmed_terms,
    ))
}

async fn apply_sentence_patch(
    client: &SensenovaClient,
    cancel: &tokio_util::sync::CancellationToken,
    event_log_path: &Path,
    chunk_index: usize,
    article_type: &str,
    plan: &ResidualSentencePlan,
    translated: &mut String,
) -> Result<(bool, Vec<crate::llm::ConfirmedTerm>), AppError> {
    if apply_deterministic_sentence_patch(plan, translated) {
        return Ok((true, Vec::new()));
    }

    let patch_terms = plan
        .patch_terms
        .iter()
        .map(|(source, target)| (source.as_str(), target.as_str()))
        .collect::<Vec<_>>();
    let prompt = crate::translation_validation::build_llm_sentence_patch_prompt(
        article_type,
        &plan.target,
        &patch_terms,
    );
    let response = review_issue_with_observed_retry(
        client,
        cancel,
        event_log_path,
        chunk_index,
        "sentence",
        &prompt,
    )
    .await?;
    let parsed = parse_residual_repair(&response);
    let Some(patched_sentence) = parsed.repaired_text.clone() else {
        return Ok((false, Vec::new()));
    };
    let Some(updated) = crate::pipeline::review::replace_sentence_once(
        translated,
        &plan.target.translated_sentence,
        &patched_sentence,
    ) else {
        return Ok((false, Vec::new()));
    };
    if updated == *translated {
        return Ok((false, Vec::new()));
    }
    let confirmed_terms =
        filter_sentence_level_residual_confirmed_terms(&parsed, article_type, plan, &updated);
    *translated = updated;
    Ok((true, confirmed_terms))
}

fn apply_deterministic_sentence_patch(
    plan: &ResidualSentencePlan,
    translated: &mut String,
) -> bool {
    if plan.patch_terms.is_empty() {
        return false;
    }
    let mut patched_sentence = plan.target.translated_sentence.clone();
    for (source, target) in &plan.patch_terms {
        let next =
            crate::pipeline::review::replace_source_like_phrase(&patched_sentence, source, target);
        if next != patched_sentence {
            patched_sentence = next;
        }
    }
    if patched_sentence == plan.target.translated_sentence {
        return false;
    }
    let Some(updated) = crate::pipeline::review::replace_sentence_once(
        translated,
        &plan.target.translated_sentence,
        &patched_sentence,
    ) else {
        return false;
    };
    if updated == *translated {
        return false;
    }
    *translated = updated;
    true
}

async fn apply_excerpt_patch(
    client: &SensenovaClient,
    cancel: &tokio_util::sync::CancellationToken,
    event_log_path: &Path,
    chunk_index: usize,
    article_type: &str,
    phase: &str,
    plan: &ResidualExcerptPlan,
    translated: &mut String,
) -> Result<(bool, Vec<crate::llm::ConfirmedTerm>), AppError> {
    let patch_terms = plan
        .patch_terms
        .iter()
        .map(|(source, target)| (source.as_str(), target.as_str()))
        .collect::<Vec<_>>();
    let prompt = crate::translation_validation::build_llm_paragraph_patch_prompt(
        article_type,
        &plan.source_excerpt,
        &plan.translated_excerpt,
        &patch_terms,
        phase,
    );
    let response = review_issue_with_observed_retry(
        client,
        cancel,
        event_log_path,
        chunk_index,
        phase,
        &prompt,
    )
    .await?;
    let parsed = parse_residual_repair(&response);
    let Some(patched_excerpt) = parsed.repaired_text.clone() else {
        return Ok((false, Vec::new()));
    };
    let Some(updated) = replace_text_once(translated, &plan.translated_excerpt, &patched_excerpt)
    else {
        return Ok((false, Vec::new()));
    };
    if updated == *translated {
        return Ok((false, Vec::new()));
    }
    let confirmed_terms =
        filter_excerpt_level_residual_confirmed_terms(&parsed, article_type, plan, &updated);
    *translated = updated;
    Ok((true, confirmed_terms))
}

fn replace_text_once(text: &str, original: &str, replacement: &str) -> Option<String> {
    let index = text.find(original)?;
    let mut updated = String::with_capacity(text.len() - original.len() + replacement.len());
    updated.push_str(&text[..index]);
    updated.push_str(replacement);
    updated.push_str(&text[index + original.len()..]);
    Some(updated)
}

fn filter_sentence_level_residual_confirmed_terms(
    parsed: &super::parse::ParsedResidualRepair,
    article_type: &str,
    plan: &ResidualSentencePlan,
    repaired_sentence: &str,
) -> Vec<crate::llm::ConfirmedTerm> {
    let unmapped_sources = plan
        .patch_terms
        .iter()
        .filter(|(_, target)| target.trim().is_empty())
        .map(|(source, _)| source.as_str())
        .collect::<Vec<_>>();
    if unmapped_sources.is_empty() {
        return Vec::new();
    }

    parsed
        .confirmed_terms
        .iter()
        .filter(|term| {
            unmapped_sources.iter().any(|source| {
                crate::pipeline::context::source_terms_are_similar(source, &term.source)
            })
        })
        .filter_map(|term| sanitize_dynamic_confirmed_term(term, repaired_sentence, article_type))
        .collect()
}

fn filter_excerpt_level_residual_confirmed_terms(
    parsed: &super::parse::ParsedResidualRepair,
    article_type: &str,
    plan: &ResidualExcerptPlan,
    repaired_excerpt: &str,
) -> Vec<crate::llm::ConfirmedTerm> {
    let unmapped_sources = plan
        .patch_terms
        .iter()
        .filter(|(_, target)| target.trim().is_empty())
        .map(|(source, _)| source.as_str())
        .collect::<Vec<_>>();
    if unmapped_sources.is_empty() {
        return Vec::new();
    }

    parsed
        .confirmed_terms
        .iter()
        .filter(|term| {
            unmapped_sources.iter().any(|source| {
                crate::pipeline::context::source_terms_are_similar(source, &term.source)
            })
        })
        .filter_map(|term| sanitize_dynamic_confirmed_term(term, repaired_excerpt, article_type))
        .collect()
}

fn filter_chunk_level_residual_confirmed_terms(
    parsed: &super::parse::ParsedResidualRepair,
    article_type: &str,
    translated_before_patch: &str,
    repaired_chunk: &str,
    residual_terms: &[crate::translation_validation::ResidualEnglishTerm],
) -> Vec<crate::llm::ConfirmedTerm> {
    let unmapped_sources = residual_terms
        .iter()
        .map(|term| term.term.as_str())
        .collect::<Vec<_>>();
    if unmapped_sources.is_empty() {
        return Vec::new();
    }

    parsed
        .confirmed_terms
        .iter()
        .filter(|term| {
            unmapped_sources.iter().any(|source| {
                crate::pipeline::context::source_terms_are_similar(source, &term.source)
            })
        })
        .filter_map(|term| sanitize_dynamic_confirmed_term(term, repaired_chunk, article_type))
        .filter(|term| !translated_before_patch.contains(&term.translation))
        .collect()
}

fn merge_residual_confirmed_terms(
    chunk: &mut crate::pipeline::ChunkState,
    article_type: &str,
    incoming: &[crate::llm::ConfirmedTerm],
) {
    let translated_text = chunk.translated.as_deref().unwrap_or_default();
    for term in incoming {
        let Some(term) = sanitize_dynamic_confirmed_term(term, translated_text, article_type)
        else {
            continue;
        };
        if chunk.confirmed_terms.iter().any(|existing| {
            crate::pipeline::context::source_terms_are_similar(&existing.source, &term.source)
        }) {
            continue;
        }
        chunk.confirmed_terms.push(term);
    }
}

async fn apply_chunk_first_residual_review(
    client: &SensenovaClient,
    cancel: &tokio_util::sync::CancellationToken,
    article_type: &str,
    review_chunk: &ResidualReviewChunk,
    event_log_path: &Path,
) -> Result<(Option<String>, Vec<crate::llm::ConfirmedTerm>), AppError> {
    let Some((repaired, confirmed_terms)) = request_chunk_residual_repair(
        client,
        cancel,
        event_log_path,
        review_chunk.chunk_index,
        article_type,
        &review_chunk.source,
        &review_chunk.translated,
        &review_chunk.terms,
        "chunk_first",
    )
    .await?
    else {
        return Ok((None, Vec::new()));
    };

    let remaining_terms = collect_chunk_residual_terms_with_patch_terms(
        article_type,
        &review_chunk.source,
        &repaired,
        &review_chunk.patch_terms,
        usize::MAX,
    );
    if remaining_terms.is_empty() {
        return Ok((Some(repaired), confirmed_terms));
    }

    request_chunk_residual_repair(
        client,
        cancel,
        event_log_path,
        review_chunk.chunk_index,
        article_type,
        &review_chunk.source,
        &repaired,
        &remaining_terms,
        "chunk_first_second",
    )
    .await
    .map(|second| {
        second
            .map(|(text, extra_terms)| {
                let mut merged = confirmed_terms.clone();
                merged.extend(extra_terms);
                (Some(text), merged)
            })
            .unwrap_or((Some(repaired), confirmed_terms))
    })
}

async fn request_chunk_residual_repair(
    client: &SensenovaClient,
    cancel: &tokio_util::sync::CancellationToken,
    event_log_path: &Path,
    chunk_index: usize,
    article_type: &str,
    source: &str,
    translated: &str,
    residual_terms: &[crate::translation_validation::ResidualEnglishTerm],
    phase: &str,
) -> Result<Option<(String, Vec<crate::llm::ConfirmedTerm>)>, AppError> {
    let prompt = crate::translation_validation::build_llm_validation_chunk_review_prompt(
        article_type,
        source,
        translated,
        residual_terms,
    );
    let response = review_issue_with_observed_retry(
        client,
        cancel,
        event_log_path,
        chunk_index,
        phase,
        &prompt,
    )
    .await?;
    let parsed = parse_residual_repair(&response);
    let repaired_text = parsed.repaired_text.clone();
    Ok(repaired_text.map(|text| {
        (
            text.clone(),
            filter_chunk_level_residual_confirmed_terms(
                &parsed,
                article_type,
                translated,
                &text,
                residual_terms,
            ),
        )
    }))
}

async fn review_issue_with_observed_retry(
    client: &SensenovaClient,
    cancel: &tokio_util::sync::CancellationToken,
    event_log_path: &Path,
    chunk_index: usize,
    phase: &str,
    prompt: &str,
) -> Result<String, AppError> {
    let attempts = Arc::new(Mutex::new(0u8));
    let last_request_elapsed_ms = Arc::new(Mutex::new(0u128));
    let event_log_path = event_log_path.to_path_buf();
    let result = retry_async_with_observer(
        residual_llm_retry_limit(),
        Duration::from_secs(2),
        || cancel.is_cancelled(),
        |attempt| {
            let client = client.clone();
            let prompt = prompt.to_string();
            let cancel = cancel.clone();
            let attempts = Arc::clone(&attempts);
            let last_request_elapsed_ms = Arc::clone(&last_request_elapsed_ms);
            let event_log_path = event_log_path.clone();
            let phase = phase.to_string();
            async move {
                if let Ok(mut attempts) = attempts.lock() {
                    *attempts = (*attempts).max(attempt);
                }
                let route = client.reserve_route();
                let routing_key = route.routing_key().clone();
                let _route_permit = wait_for_llm_rate_slot(&routing_key, &cancel).await?;
                record_residual_llm_attempt_started(
                    &event_log_path,
                    chunk_index,
                    &phase,
                    attempt,
                    &routing_key.model,
                    prompt.chars().count(),
                );
                let started = Instant::now();
                match client
                    .review_translation_issue_on_route(&route, &prompt)
                    .await
                {
                    Ok(response) => {
                        if let Ok(mut elapsed) = last_request_elapsed_ms.lock() {
                            *elapsed = started.elapsed().as_millis();
                        }
                        let request_elapsed_ms = started.elapsed().as_millis();
                        record_llm_success(&routing_key, request_elapsed_ms);
                        record_residual_llm_attempt_completed(
                            &event_log_path,
                            chunk_index,
                            &phase,
                            attempt,
                            &routing_key.model,
                            request_elapsed_ms,
                        );
                        Ok(response)
                    }
                    Err(error) => {
                        let request_elapsed_ms = started.elapsed().as_millis();
                        if let Ok(mut elapsed) = last_request_elapsed_ms.lock() {
                            *elapsed = request_elapsed_ms;
                        }
                        let error = AppError::from_provider_error(error);
                        client.maybe_activate_fallback_after_app_error(&routing_key.model, &error);
                        record_llm_retryable_error(&routing_key, &error, request_elapsed_ms);
                        Err(error)
                    }
                }
            }
        },
        |attempt, error, delay| {
            let request_elapsed_ms = last_request_elapsed_ms
                .lock()
                .map(|value| *value)
                .unwrap_or_default();
            record_residual_llm_attempt_failed(
                &event_log_path,
                chunk_index,
                phase,
                attempt,
                error,
                request_elapsed_ms,
                delay.as_millis(),
                false,
            );
        },
    )
    .await;

    if let Err(error) = &result {
        let attempt = attempts.lock().map(|value| *value).unwrap_or_default();
        let request_elapsed_ms = last_request_elapsed_ms
            .lock()
            .map(|value| *value)
            .unwrap_or_default();
        record_residual_llm_attempt_failed(
            &event_log_path,
            chunk_index,
            phase,
            attempt,
            error,
            request_elapsed_ms,
            0,
            true,
        );
    }

    result
}

fn residual_llm_retry_limit() -> u8 {
    std::env::var("MUSETRANSLATE_RESIDUAL_LLM_RETRY_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(2)
        .clamp(1, 2)
}

fn record_residual_llm_attempt_started(
    event_log_path: &Path,
    chunk_index: usize,
    phase: &str,
    attempt: u8,
    model: &str,
    prompt_chars: usize,
) {
    append_residual_event(
        event_log_path,
        "residual_llm_attempt_started",
        format!(
            "chunk_index={chunk_index}; phase={phase}; attempt={attempt}; model={model}; prompt_chars={prompt_chars}"
        ),
    );
}

fn record_residual_llm_attempt_completed(
    event_log_path: &Path,
    chunk_index: usize,
    phase: &str,
    attempt: u8,
    model: &str,
    request_elapsed_ms: u128,
) {
    append_residual_event(
        event_log_path,
        "residual_llm_attempt_completed",
        format!(
            "chunk_index={chunk_index}; phase={phase}; attempt={attempt}; model={model}; request_elapsed_ms={request_elapsed_ms}"
        ),
    );
}

fn record_residual_llm_attempt_failed(
    event_log_path: &Path,
    chunk_index: usize,
    phase: &str,
    attempt: u8,
    error: &AppError,
    request_elapsed_ms: u128,
    delay_ms: u128,
    final_failure: bool,
) {
    append_residual_event(
        event_log_path,
        "residual_llm_attempt_failed",
        format!(
            "chunk_index={chunk_index}; phase={phase}; attempt={attempt}; code={}; request_elapsed_ms={request_elapsed_ms}; delay_ms={delay_ms}; final={final_failure}; message={}",
            error.code,
            preview_error_message(&error.message)
        ),
    );
}

fn append_residual_event(event_log_path: &Path, stage: &str, message: String) {
    let event = TaskEvent {
        timestamp: Utc::now().to_rfc3339(),
        stage: stage.to_string(),
        message,
        progress: 92,
        level: if stage.contains("failed") {
            "warn".to_string()
        } else {
            "debug".to_string()
        },
    };
    let _ = artifact::append_json_line(event_log_path, &event);
}

fn preview_error_message(message: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 180;
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_PREVIEW_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ConfirmedTerm;
    use crate::pipeline::review::parse_repaired_translation;
    use crate::pipeline::review::residual::parse::ParsedResidualRepair;
    use crate::pipeline::{
        ChunkState, ProperNounEnforcement, ProperNounHint, StaticGlossaryEntry, TranslationContext,
    };

    fn static_term(source: &str, target: &str) -> StaticGlossaryEntry {
        StaticGlossaryEntry {
            source: source.to_string(),
            target: target.to_string(),
            scope: "global".to_string(),
            enforcement: "strict".to_string(),
            notes: String::new(),
        }
    }

    #[test]
    fn residual_sentence_plan_for_deliberation_uses_static_patch_term() {
        let checkpoint = residual_checkpoint(
            "This mechanism provides additional knowledge that induces deliberation.",
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{63D0}\u{4F9B}\u{4E86}\u{5F15}\u{53D1} deliberation \u{7684}\u{989D}\u{5916}\u{77E5}\u{8BC6}\u{3002}",
            vec![static_term("deliberation", "\u{5BA1}\u{8BAE}")],
        );

        let review_chunks = collect_residual_review_chunks(&checkpoint);
        assert_eq!(review_chunks.len(), 1);
        assert_eq!(review_chunks[0].terms.len(), 1);
        assert_eq!(review_chunks[0].terms[0].term, "deliberation");

        let plans = build_sentence_patch_plans(
            &review_chunks[0].source,
            &review_chunks[0].translated,
            &review_chunks[0].terms,
            &review_chunks[0].patch_terms,
        );

        assert_eq!(plans.len(), 1);
        assert!(plans[0].target.translated_sentence.contains("deliberation"));
        assert!(plans[0]
            .patch_terms
            .iter()
            .any(|(source, target)| source == "deliberation" && target == "\u{5BA1}\u{8BAE}"));
    }

    #[test]
    fn deterministic_sentence_patch_repairs_mixed_confirmed_term() {
        let mut translated = "卡法克斯 Admiral 与三位医生在身旁。".to_string();
        let plan = ResidualSentencePlan {
            target: crate::translation_validation::SentencePatchTarget {
                source_context: "Admiral Carfax stood beside me.".to_string(),
                translated_context: translated.clone(),
                source_sentence: "Admiral Carfax stood beside me.".to_string(),
                translated_sentence: translated.clone(),
            },
            patch_terms: vec![("Admiral Carfax".to_string(), "卡法克斯海军上将".to_string())],
        };

        assert!(apply_deterministic_sentence_patch(&plan, &mut translated));
        assert_eq!(translated, "卡法克斯海军上将与三位医生在身旁。");
    }

    #[test]
    fn residual_sentence_patch_chain_replaces_deliberation_and_clears_detection() {
        let source = "This mechanism provides additional knowledge that induces deliberation.";
        let translated =
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{63D0}\u{4F9B}\u{4E86}\u{5F15}\u{53D1} deliberation \u{7684}\u{989D}\u{5916}\u{77E5}\u{8BC6}\u{3002}";
        let checkpoint = residual_checkpoint(
            source,
            translated,
            vec![static_term("deliberation", "\u{5BA1}\u{8BAE}")],
        );
        let review_chunk = collect_residual_review_chunks(&checkpoint)
            .into_iter()
            .next()
            .expect("review chunk");
        let plan = build_sentence_patch_plans(
            &review_chunk.source,
            &review_chunk.translated,
            &review_chunk.terms,
            &review_chunk.patch_terms,
        )
        .into_iter()
        .next()
        .expect("sentence plan");

        let patch_response = r#"{"decision":"patch","patchedSentence":"\u8FD9\u4E00\u673A\u5236\u63D0\u4F9B\u4E86\u5F15\u53D1\u5BA1\u8BAE\u7684\u989D\u5916\u77E5\u8BC6\u3002"}"#;
        let patched_sentence =
            parse_repaired_translation(patch_response).expect("patched sentence");
        let patched_translation = crate::pipeline::review::replace_sentence_once(
            &review_chunk.translated,
            &plan.target.translated_sentence,
            &patched_sentence,
        )
        .expect("patched translation");

        let remaining = collect_chunk_residual_terms(
            "academic",
            &review_chunk.source,
            &patched_translation,
            usize::MAX,
        );
        assert!(remaining.is_empty());
    }

    #[test]
    fn residual_sentence_patch_chain_preserves_translation_on_correct_signal() {
        let source = "This mechanism provides additional knowledge that induces deliberation.";
        let translated =
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{63D0}\u{4F9B}\u{4E86}\u{5F15}\u{53D1}\u{5BA1}\u{8BAE}\u{7684}\u{989D}\u{5916}\u{77E5}\u{8BC6}\u{3002}";
        let checkpoint = residual_checkpoint(
            source,
            translated,
            vec![static_term("deliberation", "\u{5BA1}\u{8BAE}")],
        );
        let review_chunks = collect_residual_review_chunks(&checkpoint);
        assert!(review_chunks.is_empty());

        let response = r#"{"decision":"correct"}"#;
        assert!(parse_repaired_translation(response).is_none());
        assert_eq!(
            translated,
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{63D0}\u{4F9B}\u{4E86}\u{5F15}\u{53D1}\u{5BA1}\u{8BAE}\u{7684}\u{989D}\u{5916}\u{77E5}\u{8BC6}\u{3002}"
        );
    }

    #[test]
    fn residual_review_parallel_limit_reuses_active_route_profile_capacity() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let client = SensenovaClient::new(
            "https://api.deepseek.com/chat/completions".to_string(),
            format!("test-key-{suffix}"),
            format!("deepseek-v4-flash-{suffix}"),
        );
        let routing_key = client
            .active_routing_keys()
            .into_iter()
            .next()
            .expect("client should expose an active route");
        record_llm_success(&routing_key, 20_000);
        let config = PipelineRunConfig {
            task_id: "task".to_string(),
            pdf_path: std::path::PathBuf::from("sample.epub"),
            article_type: "fiction".to_string(),
            system_prompt: "prompt".to_string(),
            system_prompt_hash: "prompt-hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            epub_chapter_title: None,
            chunk_size: 1000,
            max_parallel: 16,
            artifact_dir: std::env::temp_dir(),
            cache_config: crate::pipeline::PipelineCacheConfig::from_artifact_dir(
                &std::env::temp_dir(),
            ),
            static_glossary_entries: Vec::new(),
            global_llm_limiter: Arc::new(tokio::sync::Semaphore::new(16)),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
        };

        let effective = residual_review_parallel_limit(&config, &client);

        assert!(
            effective > 1,
            "residual review should inherit learned route capacity"
        );
        // 不再钳制到 config.max_parallel：residual 阶段可使用超过主翻译并发的
        // 路由画像容量；上限由全局信号量与路由许可兜底。
        let _ = config.max_parallel;
    }

    #[test]
    fn residual_review_uses_paragraph_scope_for_same_paragraph_candidates() {
        let review_chunk = ResidualReviewChunk {
            chunk_index: 0,
            source: "The orpods approached. The gorthak opened.".to_string(),
            translated: "orpods\u{903C}\u{8FD1}\u{3002}gorthak\u{6253}\u{5F00}\u{4E86}\u{3002}"
                .to_string(),
            terms: vec![
                crate::translation_validation::ResidualEnglishTerm {
                    term: "orpods".to_string(),
                    count: 1,
                    source_excerpt: "orpods approached".to_string(),
                    translated_excerpt: "orpods".to_string(),
                },
                crate::translation_validation::ResidualEnglishTerm {
                    term: "gorthak".to_string(),
                    count: 1,
                    source_excerpt: "gorthak opened".to_string(),
                    translated_excerpt: "gorthak".to_string(),
                },
            ],
            patch_terms: Vec::new(),
        };
        let plans = vec![
            ResidualSentencePlan {
                target: crate::translation_validation::SentencePatchTarget {
                    source_context: "The orpods approached.".to_string(),
                    translated_context: "orpods\u{903C}\u{8FD1}\u{3002}".to_string(),
                    source_sentence: "The orpods approached.".to_string(),
                    translated_sentence: "orpods\u{903C}\u{8FD1}\u{3002}".to_string(),
                },
                patch_terms: Vec::new(),
            },
            ResidualSentencePlan {
                target: crate::translation_validation::SentencePatchTarget {
                    source_context: "The gorthak opened.".to_string(),
                    translated_context: "gorthak\u{6253}\u{5F00}\u{4E86}\u{3002}".to_string(),
                    source_sentence: "The gorthak opened.".to_string(),
                    translated_sentence: "gorthak\u{6253}\u{5F00}\u{4E86}\u{3002}".to_string(),
                },
                patch_terms: Vec::new(),
            },
        ];

        assert!(matches!(
            choose_residual_repair_scope(&review_chunk, &plans),
            ResidualRepairScope::Paragraph(_)
        ));
    }

    #[test]
    fn residual_review_keeps_sentence_first_for_single_candidate() {
        let checkpoint = residual_checkpoint(
            "The mechanism induces deliberation.",
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{5F15}\u{53D1} deliberation\u{3002}",
            vec![static_term("deliberation", "\u{5BA1}\u{8BAE}")],
        );
        let review_chunk = collect_residual_review_chunks(&checkpoint)
            .into_iter()
            .next()
            .expect("review chunk");
        let plans = build_sentence_patch_plans(
            &review_chunk.source,
            &review_chunk.translated,
            &review_chunk.terms,
            &review_chunk.patch_terms,
        );

        assert_eq!(review_chunk.terms.len(), 1);
        assert!(matches!(
            choose_residual_repair_scope(&review_chunk, &plans),
            ResidualRepairScope::Sentence
        ));
    }

    #[test]
    fn residual_review_uses_adjacent_paragraph_scope_for_neighbor_hits() {
        let review_chunk = ResidualReviewChunk {
            chunk_index: 0,
            source: "The orpods approached.\n\nThe gorthak opened.".to_string(),
            translated: "orpods\u{903C}\u{8FD1}\u{3002}\n\ngorthak\u{6253}\u{5F00}\u{4E86}\u{3002}"
                .to_string(),
            terms: vec![
                crate::translation_validation::ResidualEnglishTerm {
                    term: "orpods".to_string(),
                    count: 1,
                    source_excerpt: "orpods approached".to_string(),
                    translated_excerpt: "orpods".to_string(),
                },
                crate::translation_validation::ResidualEnglishTerm {
                    term: "gorthak".to_string(),
                    count: 1,
                    source_excerpt: "gorthak opened".to_string(),
                    translated_excerpt: "gorthak".to_string(),
                },
            ],
            patch_terms: Vec::new(),
        };
        let plans = build_sentence_patch_plans(
            &review_chunk.source,
            &review_chunk.translated,
            &review_chunk.terms,
            &review_chunk.patch_terms,
        );

        assert_eq!(plans.len(), 2);
        assert!(matches!(
            choose_residual_repair_scope(&review_chunk, &plans),
            ResidualRepairScope::AdjacentParagraphs(_)
        ));
    }

    #[test]
    fn residual_review_uses_marker_boundaries_for_adjacent_paragraph_scope() {
        let review_chunk = ResidualReviewChunk {
            chunk_index: 0,
            source: "[[B000]]\nThe orpods approached.\n[[B001]]\nThe gorthak opened.".to_string(),
            translated:
                "[[B000]]\norpods\u{903C}\u{8FD1}\u{3002}\n[[B001]]\ngorthak\u{6253}\u{5F00}\u{4E86}\u{3002}"
                    .to_string(),
            terms: vec![
                crate::translation_validation::ResidualEnglishTerm {
                    term: "orpods".to_string(),
                    count: 1,
                    source_excerpt: "orpods approached".to_string(),
                    translated_excerpt: "orpods".to_string(),
                },
                crate::translation_validation::ResidualEnglishTerm {
                    term: "gorthak".to_string(),
                    count: 1,
                    source_excerpt: "gorthak opened".to_string(),
                    translated_excerpt: "gorthak".to_string(),
                },
            ],
            patch_terms: Vec::new(),
        };
        let plans = build_sentence_patch_plans(
            &review_chunk.source,
            &review_chunk.translated,
            &review_chunk.terms,
            &review_chunk.patch_terms,
        );

        assert_eq!(plans.len(), 2);
        assert!(matches!(
            choose_residual_repair_scope(&review_chunk, &plans),
            ResidualRepairScope::AdjacentParagraphs(_)
        ));
    }

    #[test]
    fn residual_review_uses_chunk_scope_for_distributed_hits() {
        let review_chunk = ResidualReviewChunk {
            chunk_index: 0,
            source: "The orpods approached.\n\nMiddle translated paragraph.\n\nThe gorthak opened."
                .to_string(),
            translated:
                "orpods\u{903C}\u{8FD1}\u{3002}\n\n\u{4E2D}\u{95F4}\u{6BB5}\u{843D}\u{3002}\n\ngorthak\u{6253}\u{5F00}\u{4E86}\u{3002}"
                    .to_string(),
            terms: vec![
                crate::translation_validation::ResidualEnglishTerm {
                    term: "orpods".to_string(),
                    count: 1,
                    source_excerpt: "orpods approached".to_string(),
                    translated_excerpt: "orpods".to_string(),
                },
                crate::translation_validation::ResidualEnglishTerm {
                    term: "gorthak".to_string(),
                    count: 1,
                    source_excerpt: "gorthak opened".to_string(),
                    translated_excerpt: "gorthak".to_string(),
                },
            ],
            patch_terms: Vec::new(),
        };
        let plans = build_sentence_patch_plans(
            &review_chunk.source,
            &review_chunk.translated,
            &review_chunk.terms,
            &review_chunk.patch_terms,
        );

        assert_eq!(plans.len(), 2);
        assert!(matches!(
            choose_residual_repair_scope(&review_chunk, &plans),
            ResidualRepairScope::Chunk
        ));
    }

    #[test]
    fn residual_confirmed_terms_only_accept_current_unmapped_patch_terms() {
        let plan = ResidualSentencePlan {
            target: crate::translation_validation::SentencePatchTarget {
                source_context: "The orpods moved.".to_string(),
                translated_context: "orpods 移动。".to_string(),
                source_sentence: "The orpods moved.".to_string(),
                translated_sentence: "orpods 移动。".to_string(),
            },
            patch_terms: vec![
                ("orpods".to_string(), String::new()),
                ("deliberation".to_string(), "审议".to_string()),
            ],
        };
        let parsed = ParsedResidualRepair {
            repaired_text: Some("奥尔波兹移动。".to_string()),
            confirmed_terms: vec![
                ConfirmedTerm {
                    source: "orpods".to_string(),
                    translation: "奥尔波兹".to_string(),
                    term_type: "coinage".to_string(),
                    confidence: 95,
                    usage_role: "coinage_noun".to_string(),
                },
                ConfirmedTerm {
                    source: "deliberation".to_string(),
                    translation: "审议".to_string(),
                    term_type: "technical".to_string(),
                    confidence: 95,
                    usage_role: "technical_term".to_string(),
                },
            ],
        };

        let terms = filter_sentence_level_residual_confirmed_terms(
            &parsed,
            "fiction",
            &plan,
            "奥尔波兹移动。",
        );

        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].source, "orpods");
    }

    #[test]
    fn excerpt_level_confirmed_terms_only_accept_current_unmapped_patch_terms() {
        let plan = ResidualExcerptPlan {
            source_excerpt: "The orpods moved. The deliberation deepened.".to_string(),
            translated_excerpt: "orpods 移动。审议加深。".to_string(),
            patch_terms: vec![
                ("orpods".to_string(), String::new()),
                ("deliberation".to_string(), "审议".to_string()),
            ],
        };
        let parsed = ParsedResidualRepair {
            repaired_text: Some("奥尔波兹移动。审议加深。".to_string()),
            confirmed_terms: vec![
                ConfirmedTerm {
                    source: "orpods".to_string(),
                    translation: "奥尔波兹".to_string(),
                    term_type: "coinage".to_string(),
                    confidence: 95,
                    usage_role: "coinage_noun".to_string(),
                },
                ConfirmedTerm {
                    source: "deliberation".to_string(),
                    translation: "审议".to_string(),
                    term_type: "technical".to_string(),
                    confidence: 95,
                    usage_role: "technical_term".to_string(),
                },
            ],
        };

        let terms = filter_excerpt_level_residual_confirmed_terms(
            &parsed,
            "fiction",
            &plan,
            "奥尔波兹移动。审议加深。",
        );

        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].source, "orpods");
    }

    #[test]
    fn chunk_level_confirmed_terms_only_accept_current_candidate_residual_terms() {
        let parsed = ParsedResidualRepair {
            repaired_text: Some("奥尔波兹逼近，审议开始。".to_string()),
            confirmed_terms: vec![
                ConfirmedTerm {
                    source: "orpods".to_string(),
                    translation: "奥尔波兹".to_string(),
                    term_type: "coinage".to_string(),
                    confidence: 95,
                    usage_role: "coinage_noun".to_string(),
                },
                ConfirmedTerm {
                    source: "outside-term".to_string(),
                    translation: "外部术语".to_string(),
                    term_type: "technical".to_string(),
                    confidence: 95,
                    usage_role: "technical_term".to_string(),
                },
            ],
        };
        let residual_terms = vec![crate::translation_validation::ResidualEnglishTerm {
            term: "orpods".to_string(),
            count: 1,
            source_excerpt: "The orpods approached".to_string(),
            translated_excerpt: "orpods".to_string(),
        }];

        let terms = filter_chunk_level_residual_confirmed_terms(
            &parsed,
            "fiction",
            "orpods 逼近，审议开始。",
            "奥尔波兹逼近，审议开始。",
            &residual_terms,
        );

        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].source, "orpods");
    }

    fn residual_checkpoint(
        source: &str,
        translated: &str,
        static_terms: Vec<StaticGlossaryEntry>,
    ) -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec![],
            translation_context: Some(TranslationContext {
                static_terms,
                proper_nouns: vec![ProperNounHint {
                    source: "deliberation".to_string(),
                    target: None,
                    enforcement: ProperNounEnforcement::Contextual,
                    occurrences: 1,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                }],
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: source.to_string(),
                translated: Some(translated.to_string()),
                confirmed_terms: Vec::<ConfirmedTerm>::new(),
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::TermConsistencyApplied),
            }],
        }
    }
}
