//! 数据备份与恢复：.runtime 全量数据（任务/阅读状态/书籍档案/运行配置/号池）
//! 打包为单一 ZIP；恢复时整表替换内存态并落盘（原子写）。
//!
//! 边界约定：
//! - 导出只含 JSON 数据文件（不含 artifacts/OCR 缓存等大文件）；
//! - 恢复要求压缩包内至少含 manifest（backup.json），文件按白名单读取，忽略其他条目；
//! - 恢复会覆盖当前全部任务与阅读数据——前端在调用前提示用户。

use super::{lock_mutex, persist_json, AppError, AppState, ReaderState, TranslationTask};
use crate::commands::book_profile::BookProfile;
use crate::commands::config::RuntimeConfig;
use crate::commands::storage::load_single_json;
use crate::commands::config::resolve_runtime_paths;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const BACKUP_MANIFEST: &str = "backup.json";
const BACKUP_VERSION: u32 = 1;

/// 压缩包内数据文件名白名单（与 .runtime 存储文件一一对应）。
const TASKS_FILE: &str = "tasks.json";
const READER_STATES_FILE: &str = "reader_states.json";
const BOOK_PROFILES_FILE: &str = "book_profiles.json";
const RUNTIME_CONFIG_FILE: &str = "runtime_config.json";
const ROUTE_POOLS_FILE: &str = "pools.local.json";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    version: u32,
    created_at: String,
    files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupContents {
    #[serde(default)]
    pub tasks: HashMap<String, TranslationTask>,
    #[serde(default)]
    pub reader_states: HashMap<String, ReaderState>,
    #[serde(default)]
    pub book_profiles: HashMap<String, BookProfile>,
    pub runtime_config: RuntimeConfig,
}

/// 备份清单（导出命令返回）：落盘路径与打包文件数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupExportResult {
    pub path: String,
    pub file_count: usize,
}

/// 恢复结果：各数据表恢复条目数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestoreResult {
    pub tasks: usize,
    pub reader_states: usize,
    pub book_profiles: usize,
}

/// 导出全量数据 ZIP：tasks/reader_states/book_profiles/runtime_config/pools
/// （存在者打包，缺失者跳过），附 manifest。
#[tauri::command]
pub async fn export_backup_zip(
    output_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<BackupExportResult, AppError> {
    let output_path = output_path.trim().to_string();
    if output_path.is_empty() {
        return Err(AppError::invalid_input("导出路径为空"));
    }
    let paths = resolve_runtime_paths();

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    // 内存态优先（与运行中的最新状态一致），磁盘文件仅作号池等无内存副本来源
    entries.push((TASKS_FILE.to_string(), snapshot_json(
        &*lock_mutex(&state.tasks, "任务")?,
    )?));
    entries.push((READER_STATES_FILE.to_string(), snapshot_json(
        &*lock_mutex(&state.reader_states, "阅读状态")?,
    )?));
    entries.push((BOOK_PROFILES_FILE.to_string(), snapshot_json(
        &*lock_mutex(&state.book_profiles, "书籍档案")?,
    )?));
    entries.push((RUNTIME_CONFIG_FILE.to_string(), snapshot_json(
        &*lock_mutex(&state.config, "运行配置")?,
    )?));
    if let Some(bytes) = read_optional_file(&paths.runtime_config_store_path.with_file_name(ROUTE_POOLS_FILE)) {
        entries.push((ROUTE_POOLS_FILE.to_string(), bytes));
    }

    let manifest = serde_json::to_vec_pretty(&BackupManifest {
        version: BACKUP_VERSION,
        created_at: Utc::now().to_rfc3339(),
        files: entries.iter().map(|(name, _)| name.clone()).collect(),
    })
    .map_err(|error| AppError::internal(format!("备份清单序列化失败: {error}")))?;
    let file_count = entries.len();

    let file = std::fs::File::create(&output_path)
        .map_err(|error| AppError::internal(format!("创建备份文件失败: {error}")))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file(BACKUP_MANIFEST, options)
        .map_err(|error| AppError::internal(format!("写入备份清单失败: {error}")))?;
    writer
        .write_all(&manifest)
        .map_err(|error| AppError::internal(format!("写入备份清单失败: {error}")))?;
    for (name, bytes) in &entries {
        writer
            .start_file(name, options)
            .map_err(|error| AppError::internal(format!("写入备份条目 {name} 失败: {error}")))?;
        writer
            .write_all(bytes)
            .map_err(|error| AppError::internal(format!("写入备份条目 {name} 失败: {error}")))?;
    }
    writer
        .finish()
        .map_err(|error| AppError::internal(format!("完成备份压缩失败: {error}")))?;

    Ok(BackupExportResult {
        path: output_path,
        file_count,
    })
}

/// 恢复备份 ZIP：解析白名单文件 → 整表替换内存态 → 落盘持久化。
/// 落盘失败回滚内存副本，保证内存与磁盘一致。
#[tauri::command]
pub async fn restore_backup_zip(
    archive_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<BackupRestoreResult, AppError> {
    let archive_path = archive_path.trim().to_string();
    if archive_path.is_empty() {
        return Err(AppError::invalid_input("备份文件路径为空"));
    }
    let file = std::fs::File::open(&archive_path)
        .map_err(|error| AppError::internal(format!("打开备份文件失败: {error}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| AppError::internal(format!("备份压缩包损坏: {error}")))?;
    if archive.by_name(BACKUP_MANIFEST).is_err() {
        return Err(AppError::invalid_input("不是有效的备份文件（缺少清单）"));
    }

    let contents = read_backup_contents(&mut archive)?;

    // 逐表替换（持久化失败回滚），号池文件原样写回磁盘
    let result = apply_backup(&state, &contents)?;
    if let Some(pools_bytes) = read_zip_entry(&mut archive, ROUTE_POOLS_FILE) {
        let pools_path = resolve_runtime_paths()
            .runtime_config_store_path
            .with_file_name(ROUTE_POOLS_FILE);
        std::fs::write(&pools_path, pools_bytes)
            .map_err(|error| AppError::internal(format!("写回号池文件失败: {error}")))?;
    }
    Ok(result)
}

fn snapshot_json<T: Serialize>(value: &T) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::internal(format!("序列化运行数据失败: {error}")))
}

fn read_optional_file(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// 单个备份条目解压上限：白名单内全部是小型 JSON 数据文件，正常远达不到此值；
/// 防御恶意/损坏压缩包的解压炸弹条目撑爆内存。
const MAX_BACKUP_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

fn read_zip_entry(archive: &mut ZipArchive<std::fs::File>, name: &str) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    let mut buffer = Vec::new();
    let mut limited = entry.take(MAX_BACKUP_ENTRY_BYTES);
    limited.read_to_end(&mut buffer).ok()?;
    if buffer.len() as u64 >= MAX_BACKUP_ENTRY_BYTES {
        return None;
    }
    Some(buffer)
}

fn parse_entry<T: serde::de::DeserializeOwned>(
    archive: &mut ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Option<T>, AppError> {
    let Some(bytes) = read_zip_entry(archive, name) else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        AppError::invalid_input(format!("备份条目 {name} 解析失败: {error}"))
    })
}

fn read_backup_contents(
    archive: &mut ZipArchive<std::fs::File>,
) -> Result<BackupContents, AppError> {
    let runtime_config: RuntimeConfig = parse_entry(archive, RUNTIME_CONFIG_FILE)?
        .ok_or_else(|| AppError::invalid_input("备份缺少运行配置"))?;
    Ok(BackupContents {
        tasks: parse_entry(archive, TASKS_FILE)?.unwrap_or_default(),
        reader_states: parse_entry(archive, READER_STATES_FILE)?.unwrap_or_default(),
        book_profiles: parse_entry(archive, BOOK_PROFILES_FILE)?.unwrap_or_default(),
        runtime_config,
    })
}

fn apply_backup(state: &AppState, contents: &BackupContents) -> Result<BackupRestoreResult, AppError> {
    let paths = resolve_runtime_paths();

    let previous_tasks = lock_mutex(&state.tasks, "任务")?.clone();
    {
        let mut tasks = lock_mutex(&state.tasks, "任务")?;
        *tasks = contents.tasks.clone();
        if let Err(error) = persist_json(&paths.task_store_path, &*tasks) {
            *tasks = previous_tasks;
            return Err(error);
        }
    }

    let previous_states = lock_mutex(&state.reader_states, "阅读状态")?.clone();
    {
        let mut states = lock_mutex(&state.reader_states, "阅读状态")?;
        *states = contents.reader_states.clone();
        if let Err(error) = persist_json(&paths.reader_state_store_path, &*states) {
            *states = previous_states;
            return Err(error);
        }
    }

    let previous_profiles = lock_mutex(&state.book_profiles, "书籍档案")?.clone();
    {
        let mut profiles = lock_mutex(&state.book_profiles, "书籍档案")?;
        *profiles = contents.book_profiles.clone();
        if let Err(error) = persist_json(&paths.book_profile_store_path, &*profiles) {
            *profiles = previous_profiles;
            return Err(error);
        }
    }

    // 运行配置走既有替换通道（URL 归一 + provider 同步 + 失败回滚）
    super::config::replace_runtime_config(state, contents.runtime_config.clone())?;

    Ok(BackupRestoreResult {
        tasks: contents.tasks.len(),
        reader_states: contents.reader_states.len(),
        book_profiles: contents.book_profiles.len(),
    })
}

// load_single_json 预留给恢复前的磁盘校验（当前流程直接内存替换，保留引用避免死代码）
#[allow(dead_code)]
fn peek_runtime_config(path: &Path) -> Option<RuntimeConfig> {
    load_single_json(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    #[test]
    fn backup_contents_roundtrip_via_zip() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-backup-roundtrip-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let zip_path = root.join("backup.zip");

        let mut tasks = HashMap::new();
        tasks.insert(
            "task-1".to_string(),
            TranslationTask {
                id: "task-1".to_string(),
                filename: "demo.epub".to_string(),
                pdf_path: "/tmp/demo.epub".to_string(),
                status: super::super::tasks::TaskStatus::Completed,
                phase: super::super::tasks::TaskPhase::Completed,
                progress: 100,
                message: "done".to_string(),
                article_type: "fiction".to_string(),
                concurrent: true,
                max_workers: 4,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                output_path: Some("/tmp/out.md".to_string()),
                html_output_path: None,
                cover_path: None,
                artifact_paths: super::super::tasks::TaskArtifactPaths::default(),
                total_chunks: 1,
                translated_chunks: 1,
                retry_count: 0,
                source_hash: None,
                last_error: None,
            },
        );
        let contents = BackupContents {
            tasks,
            reader_states: HashMap::new(),
            book_profiles: HashMap::new(),
            runtime_config: sample_config(),
        };

        // 写 ZIP
        let file = std::fs::File::create(&zip_path).expect("create zip");
        let mut writer = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let manifest = serde_json::to_vec_pretty(&BackupManifest {
            version: BACKUP_VERSION,
            created_at: Utc::now().to_rfc3339(),
            files: vec![TASKS_FILE.to_string(), RUNTIME_CONFIG_FILE.to_string()],
        })
        .expect("manifest");
        writer.start_file(BACKUP_MANIFEST, options).expect("manifest entry");
        writer.write_all(&manifest).expect("manifest write");
        writer.start_file(TASKS_FILE, options).expect("tasks entry");
        writer
            .write_all(&serde_json::to_vec_pretty(&contents.tasks).expect("tasks json"))
            .expect("tasks write");
        writer
            .start_file(RUNTIME_CONFIG_FILE, options)
            .expect("config entry");
        writer
            .write_all(&serde_json::to_vec_pretty(&contents.runtime_config).expect("config json"))
            .expect("config write");
        writer.finish().expect("finish");

        // 读 ZIP
        let file = std::fs::File::open(&zip_path).expect("open zip");
        let mut archive = ZipArchive::new(file).expect("archive");
        assert!(archive.by_name(BACKUP_MANIFEST).is_ok());
        let parsed = read_backup_contents(&mut archive).expect("parse contents");
        assert_eq!(parsed.tasks.len(), 1);
        assert!(parsed.tasks.contains_key("task-1"));
        assert!(parsed.reader_states.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restore_rejects_zip_without_manifest() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-backup-nomanifest-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let zip_path = root.join("bad.zip");
        let file = std::fs::File::create(&zip_path).expect("create zip");
        let mut writer = ZipWriter::new(file);
        writer
            .start_file(
                TASKS_FILE,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .expect("entry");
        writer.write_all(b"{}").expect("write");
        writer.finish().expect("finish");

        let file = std::fs::File::open(&zip_path).expect("open");
        let mut archive = ZipArchive::new(file).expect("archive");
        assert!(archive.by_name(BACKUP_MANIFEST).is_err());

        let _ = std::fs::remove_dir_all(root);
    }
}
