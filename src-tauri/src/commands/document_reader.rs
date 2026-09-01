mod cache;
mod source;

use self::cache::prepare_document_reader_cache;
use self::source::resolve_document_source;
#[cfg(test)]
use self::source::resolve_task_document_source;
use super::{AppError, AppState};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

const DOCUMENT_CACHE_NAMESPACE: &str = "reader/document";

/// Prepared HTML document and its local resources exposed through Tauri's asset protocol.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentReaderPackage {
    pub cache_key: String,
    pub root_path: String,
    pub html_path: String,
}

#[derive(Debug, Clone)]
struct DocumentArtifactSource {
    task_id: String,
    artifact_dir: PathBuf,
    html_path: PathBuf,
}

/// Prepare a non-EPUB reading package without accepting arbitrary frontend file paths.
#[tauri::command]
pub async fn prepare_document_reader(
    task_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DocumentReaderPackage, AppError> {
    let source = resolve_document_source(&state, &task_id)?;
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| AppError::internal(format!("定位应用缓存目录失败: {error}")))?
        .join(DOCUMENT_CACHE_NAMESPACE);
    let package =
        tokio::task::spawn_blocking(move || prepare_document_reader_cache(&source, &cache_root))
            .await
            .map_err(|error| AppError::internal(format!("准备文档阅读包任务失败: {error}")))??;

    app.asset_protocol_scope()
        .allow_directory(Path::new(&package.root_path), true)
        .map_err(|error| AppError::internal(format!("授权文档阅读资源失败: {error}")))?;
    Ok(package)
}

#[cfg(test)]
#[path = "document_reader_tests.rs"]
mod tests;
