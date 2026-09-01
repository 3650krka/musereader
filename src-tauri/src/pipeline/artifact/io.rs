use super::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const CHECKPOINT_WRITE_RETRY_LIMIT: u8 = 8;
const CHECKPOINT_WRITE_RETRY_BASE_MS: u64 = 50;

pub(crate) fn ensure_dir(path: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(path)
        .map_err(|error| AppError::internal(format!("create artifact directory failed: {error}")))
}

pub(crate) fn read_json<T>(path: &Path) -> Result<T, AppError>
where
    T: for<'de> Deserialize<'de>,
{
    let content = std::fs::read_to_string(path)
        .map_err(|error| AppError::internal(format!("read JSON file failed: {error}")))?;
    serde_json::from_str(&content)
        .map_err(|error| AppError::internal(format!("parse JSON file failed: {error}")))
}

pub(crate) fn read_optional_json<T>(path: &Path) -> Result<Option<T>, AppError>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(None);
    }
    read_json(path).map(Some)
}

pub(crate) async fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let payload = serde_json::to_string_pretty(value)
        .map_err(|error| AppError::internal(format!("serialize JSON failed: {error}")))?;
    write_utf8(path, &payload).await
}

pub(crate) async fn write_utf8(path: &Path, content: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::internal(format!(
                "create parent directory failed({}): {error}",
                parent.display()
            ))
        })?;
    }
    tokio::fs::write(path, content.as_bytes())
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "write utf-8 file failed({}): {error}",
                path.display()
            ))
        })
}

pub(crate) fn persist_checkpoint(
    path: &Path,
    checkpoint: &ChunkCheckpoint,
) -> Result<(), AppError> {
    let payload = serde_json::to_string_pretty(checkpoint)
        .map_err(|error| AppError::internal(format!("serialize checkpoint failed: {error}")))?;
    atomic_write_utf8(path, &payload).map_err(|error| {
        AppError::internal(format!(
            "write checkpoint failed({}): {error}",
            path.display()
        ))
    })
}

fn atomic_write_utf8(path: &Path, content: &str) -> std::io::Result<()> {
    let mut last_error = None;
    for attempt in 1..=CHECKPOINT_WRITE_RETRY_LIMIT {
        match atomic_write_utf8_once(path, content) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < CHECKPOINT_WRITE_RETRY_LIMIT
                    && is_retryable_checkpoint_write_error(&error) =>
            {
                last_error = Some(error);
                std::thread::sleep(checkpoint_write_retry_delay(attempt));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("checkpoint write retry exhausted")))
}

fn atomic_write_utf8_once(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("checkpoint.json");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.tmp",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));

    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    let rename_result = std::fs::rename(&temp_path, path);
    if rename_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    rename_result
}

fn is_retryable_checkpoint_write_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
    )
}

fn checkpoint_write_retry_delay(attempt: u8) -> Duration {
    Duration::from_millis(CHECKPOINT_WRITE_RETRY_BASE_MS.saturating_mul(u64::from(attempt)))
}

pub(crate) fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let mut line = serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("serialize event log failed: {error}")))?;
    line.push('\n');
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| AppError::internal(format!("create event directory failed: {error}")))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| AppError::internal(format!("open event log failed: {error}")))?;
    file.write_all(line.as_bytes())
        .map_err(|error| AppError::internal(format!("write event log failed: {error}")))
}

pub(crate) fn checkpoint_flush_interval(write_count: usize) -> usize {
    if write_count < 3 {
        1
    } else if write_count < 20 {
        2
    } else {
        4
    }
}

/// 稳态下的 checkpoint 全量 flush 时间窗（秒）：在 write_count ≥ 20 后，
/// 除按块数间隔外，还要求距上次 flush 至少经过该秒数。
/// 避免长书翻译中后期每 4 块就全量序列化落盘（O(n²) 磁盘放大）。
pub(crate) const CHECKPOINT_FLUSH_MIN_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckpointWalEntry {
    pub event: String,
    pub timestamp: String,
    pub write_count: usize,
    pub chunk_count: usize,
    pub translated_chunk_count: usize,
    pub updated_at: String,
    #[serde(default)]
    pub checkpoint: Option<ChunkCheckpoint>,
}

pub(crate) fn append_checkpoint_wal(
    path: &Path,
    event: &str,
    checkpoint: &ChunkCheckpoint,
) -> Result<(), AppError> {
    let translated_chunk_count = checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.translated.is_some())
        .count();
    append_json_line(
        path,
        &CheckpointWalEntry {
            event: event.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            write_count: checkpoint.write_metrics.write_count,
            chunk_count: checkpoint.chunks.len(),
            translated_chunk_count,
            updated_at: checkpoint.updated_at.clone(),
            checkpoint: matches!(
                event,
                "checkpoint_write_committed" | "checkpoint_flush_committed"
            )
            .then_some(checkpoint.clone()),
        },
    )
}

pub(crate) fn read_json_lines<T>(path: &Path) -> Result<Vec<T>, AppError>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| AppError::internal(format!("read event log failed: {error}")))?;
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| {
                AppError::internal(format!(
                    "parse event log line {} failed: {error}",
                    index + 1
                ))
            })
        })
        .collect()
}

pub(crate) fn remove_file_if_exists(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|error| {
        AppError::internal(format!(
            "remove stale file failed({}): {error}",
            path.display()
        ))
    })
}

pub(crate) fn remove_dir_if_exists(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(path).map_err(|error| {
        AppError::internal(format!(
            "remove stale directory failed({}): {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_write_retries_only_transient_file_access_errors() {
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::Interrupted,
        ] {
            let error = std::io::Error::new(kind, "transient");
            assert!(is_retryable_checkpoint_write_error(&error));
        }

        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        assert!(!is_retryable_checkpoint_write_error(&error));
    }

    #[test]
    fn checkpoint_write_retry_delay_increases_linearly() {
        assert_eq!(checkpoint_write_retry_delay(1), Duration::from_millis(50));
        assert_eq!(checkpoint_write_retry_delay(3), Duration::from_millis(150));
    }

    #[test]
    fn checkpoint_flush_interval_is_conservative_and_bounded() {
        assert_eq!(checkpoint_flush_interval(0), 1);
        assert_eq!(checkpoint_flush_interval(2), 1);
        assert_eq!(checkpoint_flush_interval(3), 2);
        assert_eq!(checkpoint_flush_interval(19), 2);
        assert_eq!(checkpoint_flush_interval(20), 4);
    }
}
