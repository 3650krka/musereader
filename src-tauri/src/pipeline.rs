use crate::error::AppError;
#[cfg(test)]
use crate::llm::ConfirmedTerm;
use crate::llm::{LlmFallbackRoute, SensenovaClient};
use crate::ocr::BaiduOcrClient;
use chrono::Utc;
use std::path::Path;
use std::time::Instant;

mod artifact;
mod cache;
mod checkpoint;
mod chunking;
mod consistency;
mod context;
mod drift;
#[cfg(test)]
mod evaluation;
mod layout;
mod ner_glossary;
mod orchestration;
mod paths;
mod render;
mod reporting;
mod review;
mod runtime;
mod runtime_types;
mod source_prep;
mod structure;
mod style_profile;
mod translation;
mod types;
use checkpoint::{load_or_initialize_checkpoint as load_checkpoint_state, CheckpointLoadStatus};
use orchestration::{
    apply_term_consistency_with_events, build_runtime_prompt, emit_prompt_ready,
    load_checkpoint_with_events,
};
use paths::ArtifactPaths;
use reporting::finalize_translation_outputs;
use review::review_residual_english_candidates;
use review::{frozen_chunk_patch_index_is_current, rebuild_frozen_chunk_patch_terms};
use runtime::{
    configure_llm_route_profile_stores, ensure_not_cancelled, record_llm_retryable_error,
    record_llm_success, retry_async_with_observer, route_profile_recommended_parallel,
    wait_for_llm_rate_slot,
};
use runtime_types::{
    collect_pending_indexes, compute_translate_progress, failed_body_chunk_indexes,
    mark_checkpoint_merge_ready, merge_translated_chunks, needs_merge_ready_advance,
    needs_residual_review, needs_term_consistency, repair_body_chunks_with_final_context,
};
use source_prep::prepare_source_document;
use translation::translate_checkpoint_chunks;
use types::*;
pub(crate) use types::{env_flag_disabled, env_flag_enabled};
pub use types::{
    ArtifactFile, ArtifactManifest, GlossaryArtifact, GlossaryEntry, PipelineCacheConfig,
    PipelineProgressStage, PipelineProgressUpdate, PipelineResult, PipelineRunConfig,
    ProperNounEnforcement, ProperNounHint, StaticGlossaryEntry, TranslationContext,
    ValidationSummary,
};

#[cfg(test)]
fn apply_reference_chunk_policy(checkpoint: &mut ChunkCheckpoint) {
    checkpoint::apply_reference_chunk_policy(checkpoint);
}

#[cfg(test)]
fn split_reference_aware_markdown(
    text: &str,
    chunk_size: usize,
    article_type: &str,
) -> Vec<String> {
    chunking::split_reference_aware_markdown(text, chunk_size, article_type)
}

#[cfg(test)]
fn format_translation_context_for_chunk(
    context: &TranslationContext,
    article_type: &str,
    chunk_index: usize,
    source: &str,
) -> String {
    context::format_translation_context_for_chunk(context, article_type, chunk_index, source)
}

#[cfg(test)]
fn source_mentions_hint(source: &str, hint: &str) -> bool {
    context::source_mentions_hint(source, hint)
}

#[cfg(test)]
fn parse_repaired_translation(review_response: &str) -> Option<String> {
    review::parse_repaired_translation(review_response)
}

#[cfg(test)]
fn synthesize_missing_strict_hit_terms(
    strict_hits: &[StrictChunkHit],
    translated_text: &str,
    confirmed_terms: &mut Vec<crate::llm::ConfirmedTerm>,
) {
    context::synthesize_missing_strict_hit_terms(strict_hits, translated_text, confirmed_terms)
}

#[cfg(test)]
fn repair_strict_hit_aliases(
    translated_text: &str,
    confirmed_terms: &[crate::llm::ConfirmedTerm],
    strict_hits: &[StrictChunkHit],
) -> String {
    context::repair_strict_hit_aliases(translated_text, confirmed_terms, strict_hits)
}

#[cfg(test)]
fn compute_retry_delay(
    error: &AppError,
    base_delay: std::time::Duration,
    attempt: u8,
) -> std::time::Duration {
    runtime::compute_retry_delay(error, base_delay, attempt)
}

const OCR_RETRY_LIMIT: u8 = 5;
const LLM_RETRY_LIMIT: u8 = 5;

fn llm_retry_limit() -> u8 {
    std::env::var("MUSETRANSLATE_LLM_RETRY_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(LLM_RETRY_LIMIT)
        .clamp(1, LLM_RETRY_LIMIT)
}

const PROPER_NOUN_NOISE_WORDS: &[&str] = &[
    "The",
    "A",
    "An",
    "And",
    "But",
    "For",
    "If",
    "In",
    "Into",
    "Of",
    "On",
    "Or",
    "That",
    "Their",
    "Then",
    "There",
    "They",
    "This",
    "Those",
    "To",
    "We",
    "When",
    "Where",
    "With",
    "Now",
    "Indeed",
    "Even",
    "One",
    "You",
    "People",
    "Story",
    "Chance",
    "Anon",
    "End",
    "Abstract",
    "Introduction",
    "Background",
    "Methods",
    "Results",
    "Discussion",
    "Conclusion",
    "Conclusions",
    "References",
    "Appendix",
    "Table",
    "Figure",
    "Language",
    "Specifically",
    "However",
    "Although",
    "Apparently",
    "Before",
    "Besides",
    "Consequently",
    "Doubtless",
    "Meanwhile",
    "Moreover",
    "Nevertheless",
    "Nothing",
    "Perhaps",
    "Presently",
    "Probably",
    "Somehow",
    "Something",
    "Sometimes",
    "Suddenly",
    "Though",
    "Toward",
    "Turning",
    "Unlike",
    "Whatever",
    "Whenever",
    "Wherever",
    "Whether",
    "Future",
    "Lastly",
    "Department",
    "Social",
    "Participants",
    "Study",
    "Research",
    "Data",
    "First",
    "Second",
    "Third",
];

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

pub struct TranslationPipeline {
    ocr_client: BaiduOcrClient,
    llm_client: SensenovaClient,
}

impl TranslationPipeline {
    /// 测试便捷构造（单 provider、无号池/回退路由）。
    #[cfg(test)]
    pub fn new(
        ocr_api_url: String,
        ocr_token: String,
        llm_api_url: String,
        llm_api_key: String,
        llm_model: String,
    ) -> Self {
        Self::new_with_llm_fallback_routes(
            ocr_api_url,
            ocr_token,
            llm_api_url,
            llm_api_key,
            llm_model,
            Vec::new(),
        )
    }

    pub(crate) fn new_with_llm_fallback_routes(
        ocr_api_url: String,
        ocr_token: String,
        llm_api_url: String,
        llm_api_key: String,
        llm_model: String,
        llm_fallback_routes: Vec<LlmFallbackRoute>,
    ) -> Self {
        Self {
            ocr_client: BaiduOcrClient::new(ocr_api_url, ocr_token),
            llm_client: SensenovaClient::new_with_fallback_routes(
                llm_api_url,
                llm_api_key,
                llm_model,
                llm_fallback_routes,
            ),
        }
    }

    /// 注入默认请求协议：号池/单 provider 构造后统一补充。
    pub(crate) fn with_llm_protocol(
        mut self,
        protocol: crate::llm::RequestProtocol,
    ) -> Self {
        self.llm_client = self.llm_client.with_protocol(protocol);
        self
    }

    // 号池构造（L1）：跨供应商并行调度，并注入 route profile 动态加权（A）。
    pub(crate) fn new_with_route_pool(
        ocr_api_url: String,
        ocr_token: String,
        llm_api_url: String,
        llm_api_key: String,
        llm_model: String,
        pool: &crate::commands::RoutePoolConfig,
    ) -> Self {
        Self {
            ocr_client: BaiduOcrClient::new(ocr_api_url, ocr_token),
            llm_client: SensenovaClient::new_with_route_pool(
                llm_api_url,
                llm_api_key,
                llm_model,
                pool,
            )
            .with_capacity_profile(std::sync::Arc::new(
                |key: &crate::llm::ProviderRoutingKey| {
                    let capacity = runtime::route_profile_capacity(key);
                    crate::llm::RouteCapacity {
                        speed_rpm: capacity.speed_rpm,
                        capacity_rpm: capacity.capacity_rpm,
                        has_learning: capacity.has_learning,
                    }
                },
            ))
            .with_cold_probe_capacity({
                // 冷启动回退容量 = 池内有学习数据的路由的中位速度。
                let keys = pool
                    .routes
                    .iter()
                    .flat_map(|route| {
                        route.resolved_api_keys().into_iter().map(|key| {
                            crate::llm::ProviderRoutingKey::new(
                                &crate::commands::normalize_llm_api_url(route.resolved_api_url()),
                                &route.model,
                                &key,
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                runtime::route_pool_median_speed_weight(&keys).unwrap_or(0.0)
            }),
        }
    }

    pub async fn process_document(
        &self,
        config: PipelineRunConfig,
        progress_callback: impl Fn(PipelineProgressUpdate),
    ) -> Result<PipelineResult, AppError> {
        let emit_progress = |stage: PipelineProgressStage, message: &str, progress: u8| {
            progress_callback(PipelineProgressUpdate::new(stage, message, progress));
        };

        ensure_not_cancelled(&config.cancellation_token)?;
        artifact::ensure_dir(&config.artifact_dir)?;
        let paths = ArtifactPaths::new(&config.artifact_dir);
        let global_route_profile_path = config.cache_config.llm_route_profile_path();
        let loaded_route_profiles = configure_llm_route_profile_stores(&[
            paths.llm_route_profile_path.clone(),
            global_route_profile_path.clone(),
        ])?;
        // B 方案：调度决策只依据本次运行实测，不继承历史画像的延迟/吞吐/健康度，
        // 避免旧模型（如已移除的 agnes）画像压制新接入路由（学习死锁）。
        crate::pipeline::runtime::reset_route_profiles_for_fresh_run();

        self.record_event(&paths.event_log_path, "queued", "task execution started", 5)?;
        self.record_event(
            &paths.event_log_path,
            "llm_route_profiles_loaded",
            &format!(
                "loaded_route_profiles={loaded_route_profiles}; global_profile_path={}; artifact_profile_path={}",
                global_route_profile_path.display(),
                paths.llm_route_profile_path.display()
            ),
            6,
        )?;
        self.record_event(
            &paths.event_log_path,
            "llm_provider_trace",
            &self.llm_client.runtime_trace(),
            6,
        )?;
        emit_progress(
            PipelineProgressStage::Imported,
            "preparing artifact directory",
            5,
        );

        let source_prep_started = Instant::now();
        let (mut document, source_hash) = prepare_source_document(
            &self.ocr_client,
            &config,
            &paths,
            &emit_progress,
            |stage, message, progress| {
                self.record_event(&paths.event_log_path, stage, message, progress)
            },
        )
        .await?;
        self.record_stage_timing(
            &paths,
            "source_prep",
            source_prep_started,
            27,
            &format!(
                "chars={}; source_blocks={}; epub_blocks={}; toc_entries={}",
                document.markdown.chars().count(),
                document.source_blocks.len(),
                document.epub_blocks.len(),
                document
                    .toc
                    .as_ref()
                    .map(|toc| toc.entries.len())
                    .unwrap_or(0)
            ),
        )?;
        let front_matter_started = Instant::now();
        let source_front_matter = self
            .extract_source_front_matter(&config, &mut document, &paths, &emit_progress)
            .await?;
        self.record_stage_timing(
            &paths,
            "source_front_matter",
            front_matter_started,
            28,
            &format!("author_count={}", source_front_matter.authors.len()),
        )?;
        ensure_not_cancelled(&config.cancellation_token)?;
        let runtime_prompt = build_runtime_prompt(&config);
        // 风格画像：对全书采样段落做一次性 LLM 分析，产出抽象风格规则
        // 追加到系统提示词（约束全书语域/声音统一）。按 source_hash 缓存，
        // 失败静默降级（绝不阻塞翻译主链路）。
        // 默认关（MUSETRANSLATE_STYLE_PROFILE=1 开启）：风格规则属于质量增强，
        // 对短文档/技术文档收益不明显，且多一次 LLM 调用；需要时显式启用。
        let runtime_prompt = if env_flag_enabled("MUSETRANSLATE_STYLE_PROFILE") {
            match self
                .load_or_extract_style_profile(&paths, &source_hash, &document.markdown)
                .await
            {
                Some(profile) => {
                    let block = profile.to_prompt_block();
                    self.record_event(
                        &paths.event_log_path,
                        "style_profile_applied",
                        &format!(
                            "rules={}; setting={}",
                            profile.rules.len(),
                            if profile.setting.is_empty() {
                                "none"
                            } else {
                                &profile.setting
                            }
                        ),
                        28,
                    )?;
                    RuntimePromptConfig {
                        system_prompt: format!("{}\n\n{}", runtime_prompt.system_prompt, block),
                        ..runtime_prompt
                    }
                }
                None => runtime_prompt,
            }
        } else {
            runtime_prompt
        };
        emit_prompt_ready(self, &paths, &runtime_prompt, &emit_progress)?;

        // NER 预提取（MUSETRANSLATE_PRE_NER=1 开启，默认关）：翻译前对全书采样
        // 做一次性实体提取（角色/地点/组织/物品，evidence-only gender），预填
        // properNouns.target（预锁定）。与 first-seen lock 互补：NER 覆盖首批
        // chunk（词表在翻译前已就位），first-seen 覆盖采样外的长尾实体。
        // 按 source_hash 缓存，失败静默降级。
        let ner_hints = if env_flag_enabled("MUSETRANSLATE_PRE_NER") {
            self.load_or_extract_ner_glossary(&paths, &source_hash, &document.markdown)
                .await
                .map(|glossary| ner_glossary::ner_entries_to_hints(&glossary.entries))
        } else {
            None
        };

        let checkpoint_load_started = Instant::now();
        let mut checkpoint = load_checkpoint_with_events(
            self,
            &config,
            &runtime_prompt,
            &paths,
            &document.markdown,
            &source_hash,
            &emit_progress,
        )?;
        // 把 NER 预提取的 hint 合并进翻译上下文（target 预填 = 预锁定，
        // first_seen lock 不覆盖已有 target，天然兼容）。仅注入源文确实提及的项。
        if let Some(hints) = ner_hints {
            if !hints.is_empty() {
                let markdown = &document.markdown;
                let context =
                    checkpoint
                        .translation_context
                        .get_or_insert_with(|| TranslationContext {
                            proper_nouns: Vec::new(),
                            static_terms: Vec::new(),
                        });
                let mut injected = 0usize;
                for hint in hints {
                    if context
                        .proper_nouns
                        .iter()
                        .any(|existing| existing.source.eq_ignore_ascii_case(&hint.source))
                    {
                        continue;
                    }
                    if !markdown.contains(&hint.source) {
                        continue;
                    }
                    context.proper_nouns.push(hint);
                    injected += 1;
                }
                if injected > 0 {
                    self.record_event(
                        &paths.event_log_path,
                        "ner_glossary_applied",
                        &format!("pre_locked_terms={injected}"),
                        33,
                    )?;
                }
            }
        }
        self.record_stage_timing(
            &paths,
            "checkpoint_load",
            checkpoint_load_started,
            32,
            &format!("chunks={}", checkpoint.chunks.len()),
        )?;

        // 方案 B：主翻译窗口内并行预取 front_matter 译文，避免 finalize 串行尾段干等。
        // 预取结果落盘到 front_matter_translated_zh.json，断点续跑时若已有有效译文则跳过。
        let front_matter_prefetch = self.spawn_front_matter_prefetch(&config, &source_front_matter);

        self.record_event(
            &paths.event_log_path,
            "translating",
            "starting translation",
            40,
        )?;
        emit_progress(PipelineProgressStage::Translating, "translating", 40);
        let translation_started = Instant::now();
        translate_checkpoint_chunks(
            self,
            &config,
            &runtime_prompt,
            &paths,
            &mut checkpoint,
            &emit_progress,
        )
        .await?;
        self.record_stage_timing(
            &paths,
            "translation",
            translation_started,
            90,
            &format!("chunks={}", checkpoint.chunks.len()),
        )?;

        if needs_term_consistency(&checkpoint) {
            let term_consistency_started = Instant::now();
            apply_term_consistency_with_events(self, &paths, &mut checkpoint, &emit_progress)?;
            self.record_stage_timing(
                &paths,
                "term_consistency",
                term_consistency_started,
                92,
                &format!("glossary_seq={}", checkpoint.delta_sync.glossary_seq),
            )?;
        }

        ensure_not_cancelled(&config.cancellation_token)?;
        if !frozen_chunk_patch_index_is_current(&checkpoint) {
            let frozen_index_started = Instant::now();
            let indexed_chunks = rebuild_frozen_chunk_patch_terms(&mut checkpoint);
            checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint)?;
            self.record_event(
                &paths.event_log_path,
                "frozen_glossary_index",
                &format!(
                    "frozen chunk patch index at glossary_seq={}; indexed_chunks={indexed_chunks}; version={}",
                    checkpoint.delta_sync.glossary_seq,
                    checkpoint.delta_sync.frozen_patch_index_version.unwrap_or_default()
                ),
                91,
            )?;
            self.record_stage_timing(
                &paths,
                "frozen_glossary_index",
                frozen_index_started,
                91,
                &format!(
                    "indexed_chunks={indexed_chunks}; glossary_seq={}",
                    checkpoint.delta_sync.glossary_seq
                ),
            )?;
        }

        ensure_not_cancelled(&config.cancellation_token)?;
        if needs_residual_review(&checkpoint) {
            let residual_review_started = Instant::now();
            self.review_residual_english_candidates(
                &config,
                &runtime_prompt,
                &paths,
                &mut checkpoint,
                &emit_progress,
            )
            .await?;
            self.record_stage_timing(
                &paths,
                "residual_review",
                residual_review_started,
                93,
                &format!("chunks={}", checkpoint.chunks.len()),
            )?;
        }

        let merge_started = Instant::now();
        if needs_merge_ready_advance(&checkpoint)
            && mark_checkpoint_merge_ready(&mut checkpoint) > 0
        {
            checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint)?;
        }
        checkpoint::flush_checkpoint(&paths, &mut checkpoint)?;
        let translated_markdown = merge_translated_chunks(&checkpoint)?;
        self.record_stage_timing(
            &paths,
            "merge_translated_chunks",
            merge_started,
            93,
            &format!("chars={}", translated_markdown.chars().count()),
        )?;
        let finalize_started = Instant::now();
        finalize_translation_outputs(
            self,
            &config,
            &runtime_prompt,
            &paths,
            &checkpoint,
            &source_hash,
            &translated_markdown,
            &source_front_matter,
            front_matter_prefetch,
            document.cover_data_url,
            |stage, message, progress| {
                self.record_event(&paths.event_log_path, stage, message, progress)
            },
            &emit_progress,
        )
        .await
        .inspect(|_| {
            let _ = self.record_stage_timing(
                &paths,
                "finalize_outputs",
                finalize_started,
                100,
                "status=ok",
            );
        })
    }

    // 主翻译窗口内并行预取 front_matter 译文。返回 None 表示无需预取（无学术 front matter）。
    // 预取结果在 finalize 时 join，并落盘 front_matter_translated_zh.json 供断点续跑复用。
    fn spawn_front_matter_prefetch(
        &self,
        config: &PipelineRunConfig,
        source_front_matter: &render::authors::AcademicFrontMatter,
    ) -> Option<reporting::FrontMatterPrefetch> {
        if !render::authors::has_academic_front_matter(source_front_matter) {
            return None;
        }
        let client = self.llm_client.clone();
        let front_matter = source_front_matter.clone();
        let cancel = config.cancellation_token.clone();
        let handle = tokio::spawn(async move {
            if cancel.is_cancelled() {
                return Err(AppError::cancelled(
                    runtime::OPERATION_CANCELLED_MESSAGE.to_string(),
                ));
            }
            structure::translate_front_matter_with_llm(&client, &front_matter).await
        });
        Some(handle)
    }

    async fn extract_source_front_matter(
        &self,
        config: &PipelineRunConfig,
        source_document: &mut crate::ocr::OcrDocument,
        paths: &ArtifactPaths,
        progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
    ) -> Result<render::authors::AcademicFrontMatter, AppError> {
        if let Ok(front_matter) = artifact::read_json::<render::authors::AcademicFrontMatter>(
            &paths.source_front_matter_path,
        ) {
            if render::authors::has_academic_front_matter(&front_matter) {
                let rewritten = structure::rewrite_source_markdown_with_front_matter(
                    &source_document.markdown,
                    &front_matter,
                );
                let rewritten = normalize_rewritten_pdf_source(&rewritten, source_document, paths);
                if rewritten != source_document.markdown {
                    source_document.markdown = rewritten;
                    source_document.source_blocks =
                        crate::document::build_source_blocks_from_markdown(
                            &source_document.markdown,
                        );
                    if source_document.toc.as_ref().is_some_and(|toc| {
                        matches!(
                            toc.source_kind,
                            crate::document::TocSourceKind::HeadingInferred
                                | crate::document::TocSourceKind::LayoutInferred
                        )
                    }) {
                        source_document.toc = crate::pipeline::source_prep::infer_source_toc(
                            &source_document.markdown,
                        );
                    }
                    artifact::persist_source_document(paths, source_document).await?;
                    self.record_event(
                        &paths.event_log_path,
                        "source_front_matter_rewritten",
                        "rewrote source markdown front matter from cached structured extraction",
                        24,
                    )?;
                }
                self.record_event(
                    &paths.event_log_path,
                    "author_metadata_front_matter_reused",
                    &format!(
                        "reused {} source author metadata entries from cached front matter",
                        front_matter.authors.len()
                    ),
                    23,
                )?;
                progress_callback(
                    PipelineProgressStage::Imported,
                    "reused source author metadata from front matter cache",
                    23,
                );
                return Ok(front_matter);
            }
        }
        let candidate = structure::front_matter_candidate_from_layout(
            &source_document.markdown,
            &source_document.page_layouts,
        );
        if candidate.blocks.is_empty() || !structure::candidate_has_author_signal(&candidate) {
            self.record_event(
                &paths.event_log_path,
                "author_metadata_front_matter_missing",
                "source front matter unavailable; skipping structured author rendering",
                23,
            )?;
            progress_callback(
                PipelineProgressStage::Imported,
                "source front matter unavailable",
                23,
            );
            return Ok(Default::default());
        }

        if let Some(front_matter) = self
            .load_cross_artifact_front_matter(config, paths, &candidate)
            .await?
        {
            let front_matter = self
                .apply_source_front_matter(
                    source_document,
                    paths,
                    front_matter,
                    "cross-artifact cache",
                    "source_front_matter_rewritten_from_cache",
                )
                .await?;
            self.record_event(
                &paths.event_log_path,
                "author_metadata_front_matter_cross_cache_reused",
                &format!(
                    "reused {} source author metadata entries from cross-artifact front matter cache",
                    front_matter.authors.len()
                ),
                23,
            )?;
            progress_callback(
                PipelineProgressStage::Imported,
                "reused source author metadata from cross-artifact front matter cache",
                23,
            );
            return Ok(front_matter);
        }

        match self
            .extract_source_front_matter_with_llm_candidate(&candidate)
            .await
        {
            Ok(front_matter) if render::authors::has_academic_front_matter(&front_matter) => {
                artifact::write_json(&paths.source_front_matter_path, &front_matter).await?;
                self.store_cross_artifact_front_matter(config, paths, &candidate, &front_matter)
                    .await?;
                let front_matter = self
                    .apply_source_front_matter(
                        source_document,
                        paths,
                        front_matter,
                        "structured extraction",
                        "source_front_matter_rewritten",
                    )
                    .await?;
                self.record_event(
                    &paths.event_log_path,
                    "author_metadata_front_matter",
                    &format!(
                        "extracted {} source author metadata entries from front matter",
                        front_matter.authors.len()
                    ),
                    23,
                )?;
                progress_callback(
                    PipelineProgressStage::Imported,
                    "extracted source author metadata from front matter",
                    23,
                );
                return Ok(front_matter);
            }
            Err(error) => {
                self.record_event(
                    &paths.event_log_path,
                    "author_metadata_front_matter_skipped",
                    &format!("front-matter author extraction skipped: {}", error.message),
                    23,
                )?;
            }
            Ok(_) => {}
        }
        self.record_event(
            &paths.event_log_path,
            "author_metadata_front_matter_missing",
            "source front matter unavailable; skipping structured author rendering",
            23,
        )?;
        progress_callback(
            PipelineProgressStage::Imported,
            "source front matter unavailable",
            23,
        );
        Ok(Default::default())
    }

    async fn load_cross_artifact_front_matter(
        &self,
        config: &PipelineRunConfig,
        paths: &ArtifactPaths,
        candidate: &structure::HeaderBlockCandidate,
    ) -> Result<Option<render::authors::AcademicFrontMatter>, AppError> {
        if !config.cache_config.effective_front_matter_cache_enabled() {
            self.record_event(
                &paths.event_log_path,
                "front_matter_cache_disabled",
                "front matter cache disabled for this run",
                23,
            )?;
            return Ok(None);
        }
        let cache_key =
            cache::build_front_matter_cache_key(candidate, &self.llm_client.active_model());
        match cache::load_front_matter_cache(&config.cache_config.root_dir, &cache_key) {
            Ok(Some(front_matter)) if render::authors::has_academic_front_matter(&front_matter) => {
                artifact::write_json(&paths.source_front_matter_path, &front_matter).await?;
                self.record_event(
                    &paths.event_log_path,
                    "front_matter_cache_hit",
                    &format!("front matter cache hit; cache_key={}", cache_key.key),
                    23,
                )?;
                Ok(Some(front_matter))
            }
            Ok(Some(_)) | Ok(None) => {
                self.record_event(
                    &paths.event_log_path,
                    "front_matter_cache_miss",
                    &format!("front matter cache miss; cache_key={}", cache_key.key),
                    23,
                )?;
                Ok(None)
            }
            Err(error) => {
                self.record_event(
                    &paths.event_log_path,
                    "front_matter_cache_invalid",
                    &format!(
                        "front matter cache invalid; cache_key={}; message={}",
                        cache_key.key, error.message
                    ),
                    23,
                )?;
                Ok(None)
            }
        }
    }

    async fn store_cross_artifact_front_matter(
        &self,
        config: &PipelineRunConfig,
        paths: &ArtifactPaths,
        candidate: &structure::HeaderBlockCandidate,
        front_matter: &render::authors::AcademicFrontMatter,
    ) -> Result<(), AppError> {
        if !config.cache_config.effective_front_matter_cache_enabled() {
            return Ok(());
        }
        let cache_key =
            cache::build_front_matter_cache_key(candidate, &self.llm_client.active_model());
        match cache::store_front_matter_cache(
            &config.cache_config.root_dir,
            &cache_key,
            front_matter,
        )
        .await
        {
            Ok(()) => self.record_event(
                &paths.event_log_path,
                "front_matter_cache_stored",
                &format!("stored front matter cache; cache_key={}", cache_key.key),
                23,
            ),
            Err(error) => self.record_event(
                &paths.event_log_path,
                "front_matter_cache_store_failed",
                &format!(
                    "front matter cache store failed; cache_key={}; message={}",
                    cache_key.key, error.message
                ),
                23,
            ),
        }
    }

    async fn apply_source_front_matter(
        &self,
        source_document: &mut crate::ocr::OcrDocument,
        paths: &ArtifactPaths,
        front_matter: render::authors::AcademicFrontMatter,
        source_label: &str,
        rewrite_stage: &str,
    ) -> Result<render::authors::AcademicFrontMatter, AppError> {
        let rewritten = structure::rewrite_source_markdown_with_front_matter(
            &source_document.markdown,
            &front_matter,
        );
        let rewritten = normalize_rewritten_pdf_source(&rewritten, source_document, paths);
        if rewritten != source_document.markdown {
            source_document.markdown = rewritten;
            source_document.source_blocks =
                crate::document::build_source_blocks_from_markdown(&source_document.markdown);
            if source_document.toc.as_ref().is_some_and(|toc| {
                matches!(
                    toc.source_kind,
                    crate::document::TocSourceKind::HeadingInferred
                        | crate::document::TocSourceKind::LayoutInferred
                )
            }) {
                source_document.toc =
                    crate::pipeline::source_prep::infer_source_toc(&source_document.markdown);
            }
            artifact::persist_source_document(paths, source_document).await?;
            self.record_event(
                &paths.event_log_path,
                rewrite_stage,
                &format!("rewrote source markdown front matter from {source_label}"),
                24,
            )?;
        }
        Ok(front_matter)
    }

    async fn extract_source_front_matter_with_llm_candidate(
        &self,
        candidate: &structure::HeaderBlockCandidate,
    ) -> Result<render::authors::AcademicFrontMatter, AppError> {
        let front_matter =
            structure::extract_front_matter_with_llm(&self.llm_client, candidate).await?;
        Ok(structure::front_matter_to_academic_front_matter(
            front_matter,
        ))
    }

    #[cfg(test)]
    fn load_or_initialize_checkpoint(
        &self,
        config: &PipelineRunConfig,
        runtime_prompt: &RuntimePromptConfig,
        paths: &ArtifactPaths,
        source_markdown: &str,
        source_hash: &str,
        progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
    ) -> Result<ChunkCheckpoint, AppError> {
        load_checkpoint_with_events(
            self,
            config,
            runtime_prompt,
            paths,
            source_markdown,
            source_hash,
            progress_callback,
        )
    }

    async fn review_residual_english_candidates(
        &self,
        config: &PipelineRunConfig,
        runtime_prompt: &RuntimePromptConfig,
        paths: &ArtifactPaths,
        checkpoint: &mut ChunkCheckpoint,
        progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
    ) -> Result<(), AppError> {
        review_residual_english_candidates(
            &self.llm_client,
            config,
            runtime_prompt,
            paths,
            checkpoint,
            progress_callback,
            |stage, message, progress| {
                self.record_event(&paths.event_log_path, stage, message, progress)
            },
        )
        .await
    }

    fn event_level_for_stage(stage: &str) -> &'static str {
        if matches!(
            stage,
            "llm_attempt_started"
                | "residual_llm_attempt_started"
                | "ocr_attempt"
                | "checkpoint_write_started"
                | "checkpoint_flush_started"
        ) {
            "debug"
        } else if stage.contains("skipped") || stage.contains("failed") {
            "warn"
        } else {
            "info"
        }
    }

    fn record_event(
        &self,
        event_log_path: &Path,
        stage: &str,
        message: &str,
        progress: u8,
    ) -> Result<(), AppError> {
        let event = TaskEvent {
            timestamp: Utc::now().to_rfc3339(),
            stage: stage.to_string(),
            message: message.to_string(),
            progress,
            level: Self::event_level_for_stage(stage).to_string(),
        };
        artifact::append_json_line(event_log_path, &event)
    }

    fn record_stage_timing(
        &self,
        paths: &ArtifactPaths,
        stage_name: &str,
        started: Instant,
        progress: u8,
        details: &str,
    ) -> Result<(), AppError> {
        self.record_event(
            &paths.event_log_path,
            "pipeline_stage_timing",
            &format!(
                "stage={stage_name}; elapsed_ms={}; {details}",
                started.elapsed().as_millis()
            ),
            progress,
        )
    }

    /// 风格画像加载/提取：优先按 source_hash 命中缓存；未命中则一次性
    /// LLM 分析全书样本段落并落盘。任何失败静默降级为 None。
    async fn load_or_extract_style_profile(
        &self,
        paths: &ArtifactPaths,
        source_hash: &str,
        source_markdown: &str,
    ) -> Option<style_profile::StyleProfile> {
        if let Some(profile) =
            style_profile::load_cached_style_profile(&paths.style_profile_path, source_hash)
        {
            return Some(profile);
        }
        let started = Instant::now();
        let profile =
            style_profile::extract_style_profile(&self.llm_client, source_hash, source_markdown)
                .await?;
        if let Err(error) =
            style_profile::persist_style_profile(&paths.style_profile_path, &profile)
        {
            let _ = self.record_event(
                &paths.event_log_path,
                "style_profile_persist_failed",
                &format!("{error:?}"),
                28,
            );
        }
        let _ = self.record_event(
            &paths.event_log_path,
            "style_profile_extracted",
            &format!(
                "elapsed_ms={}; rules={}",
                started.elapsed().as_millis(),
                profile.rules.len()
            ),
            28,
        );
        Some(profile)
    }

    /// NER 词表加载/提取：优先按 source_hash 命中缓存；未命中则一次性
    /// LLM 提取并落盘。任何失败静默降级为 None。
    async fn load_or_extract_ner_glossary(
        &self,
        paths: &ArtifactPaths,
        source_hash: &str,
        source_markdown: &str,
    ) -> Option<ner_glossary::NerGlossary> {
        if let Some(glossary) =
            ner_glossary::load_cached_ner_glossary(&paths.ner_glossary_path, source_hash)
        {
            return Some(glossary);
        }
        let started = Instant::now();
        let glossary =
            ner_glossary::extract_ner_glossary(&self.llm_client, source_hash, source_markdown)
                .await?;
        if let Err(error) = ner_glossary::persist_ner_glossary(&paths.ner_glossary_path, &glossary)
        {
            let _ = self.record_event(
                &paths.event_log_path,
                "ner_glossary_persist_failed",
                &format!("{error:?}"),
                33,
            );
        }
        let _ = self.record_event(
            &paths.event_log_path,
            "ner_glossary_extracted",
            &format!(
                "elapsed_ms={}; entities={}",
                started.elapsed().as_millis(),
                glossary.entries.len()
            ),
            33,
        );
        Some(glossary)
    }
}

fn normalize_rewritten_pdf_source(
    markdown: &str,
    source_document: &crate::ocr::OcrDocument,
    paths: &ArtifactPaths,
) -> String {
    let artifact_root = paths
        .source_markdown_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let normalized = layout::normalize_pdf_markdown_after_front_matter_rewrite(markdown);
    let with_layout_tables = if source_document
        .markdown
        .contains(layout::PDF_TABLE_START_MARKER)
        || normalized.contains(layout::PDF_TABLE_START_MARKER)
    {
        normalized
    } else {
        layout::replace_layout_tables_with_stable_snapshots(
            &normalized,
            &source_document.page_layouts,
            artifact_root,
        )
    };
    let with_table_images = layout::replace_missing_pdf_table_snapshots(
        &with_layout_tables,
        &source_document.page_layouts,
        artifact_root,
    );
    let with_captions =
        layout::inject_layout_captions(&with_table_images, &source_document.page_layouts);
    layout::collapse_adjacent_pdf_table_blocks(&with_captions)
}
