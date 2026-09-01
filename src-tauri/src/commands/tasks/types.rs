use super::AppError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Pending,
    Processing,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskPhase {
    Imported,
    OcrRunning,
    OcrReady,
    Chunking,
    Translating,
    Rendering,
    WritingArtifacts,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskArtifactPaths {
    pub artifact_dir: Option<String>,
    pub source_markdown_path: Option<String>,
    pub source_toc_path: Option<String>,
    pub translated_toc_path: Option<String>,
    pub output_markdown_path: Option<String>,
    pub output_html_path: Option<String>,
    pub translated_epub_path: Option<String>,
    pub bilingual_epub_path: Option<String>,
    pub checkpoint_path: Option<String>,
    pub manifest_path: Option<String>,
    pub event_log_path: Option<String>,
    pub validation_report_path: Option<String>,
    pub glossary_path: Option<String>,
    #[serde(default)]
    pub metrics_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
}

impl From<&AppError> for TaskError {
    fn from(error: &AppError) -> Self {
        Self {
            code: error.code.to_string(),
            message: error.message.clone(),
            retryable: error.retryable,
            retry_after_ms: error.retry_after_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationTask {
    pub id: String,
    pub filename: String,
    pub pdf_path: String,
    pub status: TaskStatus,
    pub phase: TaskPhase,
    pub progress: u8,
    pub message: String,
    pub article_type: String,
    pub concurrent: bool,
    pub max_workers: u8,
    pub created_at: String,
    pub updated_at: String,
    pub output_path: Option<String>,
    pub html_output_path: Option<String>,
    pub cover_path: Option<String>,
    pub artifact_paths: TaskArtifactPaths,
    pub total_chunks: usize,
    pub translated_chunks: usize,
    pub retry_count: u8,
    pub source_hash: Option<String>,
    #[serde(default)]
    pub last_error: Option<TaskError>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterTranslationRunResult {
    pub task_id: String,
    pub source_markdown_path: String,
    pub translated_markdown_path: String,
    pub translated_html_path: String,
    pub manifest_path: String,
    pub validation_report_path: String,
    pub translated_chunks: usize,
    pub total_chunks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands) enum TaskStopReason {
    Cancel,
    Pause,
}

pub(in crate::commands) struct TaskControl {
    pub(in crate::commands) cancellation: CancellationToken,
    pub(in crate::commands) stop_reason: Arc<Mutex<Option<TaskStopReason>>>,
    pub(in crate::commands) _handle: JoinHandle<()>,
}

#[cfg(test)]
mod tests {
    use super::TaskArtifactPaths;

    #[test]
    fn legacy_artifact_paths_default_new_epub_fields() {
        let paths: TaskArtifactPaths = serde_json::from_str(
            r#"{
                "artifactDir": "D:/artifacts/task-1",
                "outputHtmlPath": "D:/artifacts/task-1/translated.html"
            }"#,
        )
        .expect("legacy artifact paths should deserialize");

        assert_eq!(paths.artifact_dir.as_deref(), Some("D:/artifacts/task-1"));
        assert_eq!(
            paths.output_html_path.as_deref(),
            Some("D:/artifacts/task-1/translated.html")
        );
        assert!(paths.translated_epub_path.is_none());
        assert!(paths.bilingual_epub_path.is_none());
    }
}
