use crate::llm::ConfirmedTerm;
use crate::translation_validation::ResidualEnglishTerm;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineResult {
    pub translated_markdown: String,
    pub translated_html: String,
    pub cover_data_url: Option<String>,
    pub source_markdown_path: String,
    pub output_markdown_path: String,
    pub output_html_path: String,
    #[serde(default)]
    pub translated_epub_path: Option<String>,
    #[serde(default)]
    pub bilingual_epub_path: Option<String>,
    pub artifact_manifest_path: String,
    pub checkpoint_path: String,
    pub event_log_path: String,
    pub validation_report_path: String,
    pub glossary_path: String,
    pub metrics_path: String,
    pub source_hash: String,
    pub translated_chunks: usize,
    pub total_chunks: usize,
    /// 重试耗尽后仍失败的正文块数（任务照常完成；失败块保留原文，可重跑任务补译）。
    #[serde(default)]
    pub failed_chunks: usize,
    /// 失败块的 chunk index 清单（归因与补译定位用）。
    #[serde(default)]
    pub failed_chunk_indexes: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PipelineProgressStage {
    Imported,
    OcrRunning,
    Chunking,
    Translating,
    TermConsistency,
    ResidualReview,
    Rendering,
    WritingArtifacts,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineProgressUpdate {
    pub stage: PipelineProgressStage,
    pub message: String,
    pub progress: u8,
}

impl PipelineProgressUpdate {
    pub fn new(stage: PipelineProgressStage, message: impl Into<String>, progress: u8) -> Self {
        Self {
            stage,
            message: message.into(),
            progress,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PipelineRunConfig {
    pub task_id: String,
    pub pdf_path: PathBuf,
    pub article_type: String,
    pub system_prompt: String,
    pub system_prompt_hash: String,
    pub skill_ids: Vec<String>,
    pub epub_chapter_title: Option<String>,
    pub chunk_size: usize,
    pub max_parallel: usize,
    pub artifact_dir: PathBuf,
    pub cache_config: PipelineCacheConfig,
    pub static_glossary_entries: Vec<StaticGlossaryEntry>,
    pub global_llm_limiter: Arc<Semaphore>,
    pub cancellation_token: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct PipelineCacheConfig {
    pub root_dir: PathBuf,
    pub use_ocr_cache: bool,
    pub use_front_matter_cache: bool,
}

impl PipelineCacheConfig {
    pub fn from_artifact_dir(artifact_dir: &Path) -> Self {
        Self {
            root_dir: default_cache_root_for_artifact_dir(artifact_dir),
            use_ocr_cache: true,
            use_front_matter_cache: true,
        }
    }

    pub fn with_ocr_cache(mut self, enabled: bool) -> Self {
        self.use_ocr_cache = enabled;
        self
    }

    pub fn with_front_matter_cache(mut self, enabled: bool) -> Self {
        self.use_front_matter_cache = enabled;
        self
    }

    pub(super) fn effective_ocr_cache_enabled(&self) -> bool {
        self.use_ocr_cache
            && !env_flag_enabled("MUSETRANSLATE_DISABLE_PIPELINE_CACHE")
            && !env_flag_enabled("MUSETRANSLATE_DISABLE_OCR_CACHE")
    }

    pub(super) fn effective_front_matter_cache_enabled(&self) -> bool {
        self.use_front_matter_cache
            && !env_flag_enabled("MUSETRANSLATE_DISABLE_PIPELINE_CACHE")
            && !env_flag_enabled("MUSETRANSLATE_DISABLE_FRONT_MATTER_CACHE")
    }

    pub(super) fn llm_route_profile_path(&self) -> PathBuf {
        self.root_dir.join("llm_route_profiles.json")
    }
}

fn default_cache_root_for_artifact_dir(artifact_dir: &Path) -> PathBuf {
    let Some(artifact_root) = artifact_dir.parent() else {
        return artifact_dir.join(".cache");
    };
    if artifact_root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("artifacts"))
    {
        return artifact_root
            .parent()
            .map(|runtime_root| runtime_root.join("cache"))
            .unwrap_or_else(|| artifact_dir.join(".cache"));
    }
    artifact_dir.join(".cache")
}

pub(crate) fn env_flag_enabled(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// 显式关闭判定（"0"/"false"/"no"/"off"）：用于默认开启功能的退出开关。
pub(crate) fn env_flag_disabled(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(false)
}

/// 命令层（commands/*）可用的功能开关判定——与 pipeline 内部同一套语义，
/// 经 pipeline.rs 的 `pub(crate) use` 再导出。
/// 显式关闭判定（"0"/"false"/"no"/"off"）：用于默认开启功能的退出开关。

#[derive(Debug, Clone)]
pub(super) struct RuntimePromptConfig {
    pub(super) system_prompt: String,
    pub(super) system_prompt_hash: String,
    pub(super) article_type: String,
    pub(super) skill_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactFile {
    pub kind: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifest {
    pub version: u8,
    pub task_id: String,
    pub source_pdf_path: String,
    pub article_type: String,
    pub system_prompt_hash: String,
    pub skill_ids: Vec<String>,
    pub translation_context: Option<TranslationContext>,
    pub glossary: Option<GlossaryArtifact>,
    pub source_hash: String,
    pub created_at: String,
    pub updated_at: String,
    pub total_chunks: usize,
    pub translated_chunks: usize,
    pub validation_summary: ValidationSummary,
    pub files: Vec<ArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedChunkArtifact {
    pub chunk_index: usize,
    pub segment_kind: String,
    pub processing_stage: String,
    pub source: String,
    pub translated: String,
    #[serde(default)]
    pub marker_blocks: Vec<TranslatedChunkMarkerBlock>,
    pub confirmed_terms: Vec<ConfirmedTerm>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedChunkMarkerBlock {
    pub marker_id: String,
    pub source_markdown: String,
    pub translated_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSummary {
    pub issue_count: usize,
    pub untranslated_chunk_count: usize,
    pub empty_chunk_count: usize,
    pub markdown_heading_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ValidationReport {
    pub(super) version: u8,
    pub(super) task_id: String,
    pub(super) created_at: String,
    pub(super) summary: ValidationSummary,
    pub(super) issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryArtifact {
    pub(super) article_type: String,
    pub(super) entries: Vec<GlossaryEntry>,
    #[serde(default)]
    pub(super) grouped_hits: Vec<GlossaryHitGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub(super) source: String,
    pub(super) translation: String,
    pub(super) category: String,
    pub(super) occurrences: usize,
    pub(super) first_chunk_index: usize,
    #[serde(default)]
    pub(super) matched_chunk_count: usize,
    #[serde(default)]
    pub(super) heading_groups: Vec<String>,
    #[serde(default)]
    pub(super) origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryHitGroup {
    pub(super) heading: String,
    pub(super) entries: Vec<GlossaryHitStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryHitStat {
    pub(super) source: String,
    pub(super) translation: String,
    pub(super) hit_count: usize,
    pub(super) matched_chunk_count: usize,
    pub(super) first_chunk_index: usize,
    pub(super) category: String,
    pub(super) origin: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ValidationIssue {
    pub(super) code: String,
    pub(super) level: String,
    pub(super) chunk_index: Option<usize>,
    pub(super) message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChunkCheckpoint {
    pub(super) version: u8,
    pub(super) task_id: String,
    pub(super) source_pdf_path: String,
    pub(super) article_type: String,
    #[serde(default)]
    pub(super) system_prompt_hash: String,
    #[serde(default)]
    pub(super) skill_ids: Vec<String>,
    #[serde(default)]
    pub(super) translation_context: Option<TranslationContext>,
    pub(super) source_hash: String,
    pub(super) chunk_size: usize,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) source_markdown_path: String,
    pub(super) chunks: Vec<ChunkState>,
    #[serde(default)]
    pub(super) write_metrics: CheckpointWriteMetrics,
    #[serde(default)]
    pub(super) delta_sync: DeltaSyncState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct CheckpointWriteMetrics {
    pub(super) write_count: usize,
    pub(super) total_write_ms: u128,
    #[serde(default)]
    pub(super) pending_write_count: usize,
    #[serde(default)]
    pub(super) last_flush_write_count: usize,
    #[serde(default)]
    pub(super) last_flush_timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChunkState {
    pub(super) index: usize,
    pub(super) source: String,
    pub(super) translated: Option<String>,
    #[serde(default)]
    pub(super) confirmed_terms: Vec<ConfirmedTerm>,
    pub(super) attempts: u8,
    pub(super) updated_at: String,
    #[serde(default)]
    pub(super) segment_kind: ChunkSegmentKind,
    #[serde(default)]
    pub(super) processing_stage: Option<ChunkProcessingStage>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub(super) enum ChunkProcessingStage {
    Pending,
    Translated,
    TermConsistencyApplied,
    ResidualReviewed,
    /// 重试耗尽仍失败：译文缺失，产物保留原文；任务照常完成，
    /// 失败清单随 PipelineResult 上报，重跑任务时重置为 Pending 补译。
    TranslationFailed,
    MergeReady,
    ProtectedPassthrough,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(super) enum ChunkSegmentKind {
    #[default]
    Body,
    Reference,
    ProtectedMetadata,
    PdfTable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationContext {
    pub(super) proper_nouns: Vec<ProperNounHint>,
    #[serde(default)]
    pub(super) static_terms: Vec<StaticGlossaryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProperNounHint {
    pub(super) source: String,
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) enforcement: ProperNounEnforcement,
    #[serde(default)]
    pub(super) occurrences: usize,
    #[serde(default)]
    pub(super) first_chunk_index: usize,
    #[serde(default)]
    pub(super) chunk_indexes: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProperNounEnforcement {
    #[default]
    Strict,
    Contextual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskEvent {
    pub(super) timestamp: String,
    pub(super) stage: String,
    pub(super) message: String,
    pub(super) progress: u8,
    #[serde(default = "default_task_event_level")]
    pub(super) level: String,
}

fn default_task_event_level() -> String {
    "info".to_string()
}

#[derive(Debug)]
pub(super) struct TranslationOutcome {
    pub(super) index: usize,
    pub(super) translated: String,
    pub(super) confirmed_terms: Vec<ConfirmedTerm>,
    pub(super) started_glossary_seq: u64,
    pub(super) elapsed_ms: u128,
    pub(super) wait_ms: u128,
    pub(super) route_wait_ms: u128,
    pub(super) llm_elapsed_ms: u128,
    pub(super) llm_attempts: u8,
    pub(super) attempt_failures: Vec<LlmAttemptFailure>,
}

#[derive(Debug, Clone)]
pub(super) struct LlmAttemptFailure {
    pub(super) attempt: u8,
    pub(super) code: &'static str,
    pub(super) message_preview: String,
    pub(super) request_elapsed_ms: u128,
    pub(super) delay_ms: u128,
}

/// 翻译 worker 结果（块级失败隔离）：
/// - Done：该块翻译成功（原有语义）；
/// - Exhausted：重试耗尽，块标记 TranslationFailed，任务继续。
/// JoinSet 外层 Result 仍承载真正的基建错误（取消/信号量/线程异常）。
#[derive(Debug)]
pub(super) enum TranslationWorkerResult {
    Done(TranslationOutcome),
    Exhausted {
        index: usize,
        attempts: u8,
        failures: Vec<LlmAttemptFailure>,
    },
}

#[derive(Debug, Clone)]
pub(super) struct StrictChunkHit {
    pub(super) source: String,
    pub(super) target: String,
    pub(super) usage_role: String,
    pub(super) term_type: String,
}

#[derive(Debug, Clone)]
pub(super) struct ResidualReviewChunk {
    pub(super) chunk_index: usize,
    pub(super) source: String,
    pub(super) translated: String,
    pub(super) terms: Vec<ResidualEnglishTerm>,
    pub(super) patch_terms: Vec<ResidualPatchTerm>,
}

#[derive(Debug, Clone)]
pub(super) struct ResidualPatchTerm {
    pub(super) source: String,
    pub(super) target: String,
}

#[derive(Debug)]
pub(super) struct ResidualReviewOutcome {
    pub(super) chunk_index: usize,
    pub(super) repaired: Option<String>,
    pub(super) confirmed_terms: Vec<ConfirmedTerm>,
    pub(super) skipped_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct WaveTermDelta {
    pub(super) seq: u64,
    pub(super) source: String,
    pub(super) target: String,
    pub(super) chunk_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrozenChunkPatchTerm {
    pub(super) seq: u64,
    pub(super) source: String,
    pub(super) target: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeltaSyncState {
    #[serde(default)]
    pub(super) glossary_seq: u64,
    #[serde(default)]
    pub(super) wave_delta_log: Vec<WaveTermDelta>,
    #[serde(default)]
    pub(super) wave_term_key_index: BTreeMap<String, Vec<usize>>,
    #[serde(default)]
    pub(super) chunk_started_glossary_seq: BTreeMap<usize, u64>,
    #[serde(default)]
    pub(super) chunk_patched_through_glossary_seq: BTreeMap<usize, u64>,
    #[serde(default)]
    pub(super) frozen_at_glossary_seq: Option<u64>,
    #[serde(default)]
    pub(super) frozen_patch_index_version: Option<u32>,
    #[serde(default)]
    pub(super) frozen_chunk_patch_terms: BTreeMap<usize, Vec<FrozenChunkPatchTerm>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WavePatchSelection {
    pub(super) terms: Vec<WaveTermDelta>,
    pub(super) deduped_count: usize,
    pub(super) algorithmic_match_count: usize,
    pub(super) source_scan_match_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WaveTermRepairStats {
    pub(super) changed: bool,
    pub(super) attempted_terms: usize,
    pub(super) applied_terms: usize,
    pub(super) already_correct_count: usize,
    pub(super) llm_patch_count: usize,
    pub(super) llm_correct_count: usize,
    pub(super) fallback_patch_count: usize,
}
