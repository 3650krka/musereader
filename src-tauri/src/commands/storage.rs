use super::{
    repair_loaded_tasks, repair_reader_states, report_nonfatal_error, AppError, ReaderState,
    TranslationTask,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub(super) fn load_and_repair_tasks(path: &Path) -> HashMap<String, TranslationTask> {
    let loaded_tasks: HashMap<String, TranslationTask> = load_json(path);
    let repaired_tasks = repair_loaded_tasks(loaded_tasks.clone());
    let loaded_snapshot = serde_json::to_string(&loaded_tasks).ok();
    let repaired_snapshot = serde_json::to_string(&repaired_tasks).ok();
    if loaded_snapshot != repaired_snapshot {
        if let Err(error) = persist_json(path, &repaired_tasks) {
            report_nonfatal_error(None, "commands", "operation=persist_repaired_tasks", &error);
        }
    }
    repaired_tasks
}

pub(super) fn load_and_repair_reader_states(path: &Path) -> HashMap<String, ReaderState> {
    let loaded_reader_states: HashMap<String, ReaderState> = load_json(path);
    let repaired_reader_states = repair_reader_states(loaded_reader_states.clone());
    let loaded_snapshot = serde_json::to_string(&loaded_reader_states).ok();
    let repaired_snapshot = serde_json::to_string(&repaired_reader_states).ok();
    if loaded_snapshot != repaired_snapshot {
        if let Err(error) = persist_json(path, &repaired_reader_states) {
            report_nonfatal_error(
                None,
                "commands",
                "operation=persist_repaired_reader_states",
                &error,
            );
        }
    }
    repaired_reader_states
}

pub(super) fn load_json<T>(path: &Path) -> HashMap<String, T>
where
    T: for<'de> Deserialize<'de>,
{
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    match serde_json::from_str(&content) {
        Ok(map) => map,
        Err(error) => {
            // 半损坏文件不能被空表静默覆盖：先把原文件改名留档，再上报运行时通告。
            // （load_and_repair_* 随后对空表的"修复回写"只会写出与空表一致的内容，
            //   原始数据保留在 .corrupt-<时间戳> 中可人工找回。）
            let backup = path.with_extension(format!(
                "json.corrupt-{}",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default()
            ));
            let quarantine = std::fs::rename(path, &backup).is_ok();
            report_nonfatal_error(
                None,
                "storage",
                &format!(
                    "file={} parse_error={error} quarantined_to={}",
                    backup.display(),
                    if quarantine { "yes" } else { "no" }
                ),
                &AppError::internal(format!("运行数据文件解析失败，已隔离原文件: {}", path.display())),
            );
            HashMap::new()
        }
    }
}

pub(super) fn load_single_json<T>(path: &Path) -> Option<T>
where
    T: for<'de> Deserialize<'de>,
{
    let Ok(content) = std::fs::read_to_string(path) else {
        return None;
    };
    serde_json::from_str(&content).ok()
}

pub(super) fn persist_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let payload = serde_json::to_string_pretty(value)
        .map_err(|error| AppError::internal(format!("序列化运行数据失败: {error}")))?;
    // 原子写：崩溃/断电不会留下半文件（任务与阅读状态均为崩溃恢复数据源）。
    write_json_atomic(path, payload.as_bytes())
}

/// 原子写：先写临时文件再 rename，避免崩溃留下半文件（校对工作台修订层用）。
pub(super) fn write_json_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::internal(format!("创建运行目录失败: {error}")))?;
    }
    let temp_path = path.with_extension("json.tmp");
    std::fs::write(&temp_path, bytes)
        .map_err(|error| AppError::internal(format!("写入临时文件失败: {error}")))?;
    std::fs::rename(&temp_path, path)
        .map_err(|error| AppError::internal(format!("替换运行数据失败: {error}")))
}
