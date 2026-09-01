use super::super::NonfatalNoticeBuffer;
use super::events::{emit_task_progress, TaskEventSink};
use super::types::TaskStopReason;
use super::{lock_mutex, persist_json, report_nonfatal_error, state, AppError, TranslationTask};
use crate::pipeline::{PipelineProgressUpdate, PipelineResult};
use chrono::Utc;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) fn report_task_runner_nonfatal_error(
    nonfatal_notices: &NonfatalNoticeBuffer,
    operation: &str,
    task_id: &str,
    error: &AppError,
) {
    report_nonfatal_error(
        Some(nonfatal_notices),
        "task-runner",
        &format!("task_id={task_id} operation={operation}"),
        error,
    );
}

#[cfg(test)]
pub(super) fn update_task_entry<F>(
    tasks: &Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: &Path,
    task_id: &str,
    mutate: F,
) -> Result<(), AppError>
where
    F: FnOnce(&mut TranslationTask),
{
    update_task_entry_with_events(tasks, task_store_path, &None, task_id, mutate)
}

/// 带事件推送的任务更新：仅在持久化成功后 emit（回滚路径不 emit）。
pub(super) fn update_task_entry_with_events<F>(
    tasks: &Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: &Path,
    task_events: &Option<TaskEventSink>,
    task_id: &str,
    mutate: F,
) -> Result<(), AppError>
where
    F: FnOnce(&mut TranslationTask),
{
    let mut guard = lock_mutex(tasks, "任务")?;
    let original_task = {
        let task = guard
            .get_mut(task_id)
            .ok_or_else(|| AppError::not_found("任务不存在"))?;
        let original_task = task.clone();
        mutate(task);
        task.updated_at = Utc::now().to_rfc3339();
        original_task
    };
    if let Err(error) = persist_json(task_store_path, &*guard) {
        let task = guard
            .get_mut(task_id)
            .ok_or_else(|| AppError::not_found("任务不存在"))?;
        *task = original_task;
        return Err(error);
    }
    if task_events.is_some() {
        if let Some(task) = guard.get(task_id) {
            emit_task_progress(task_events, task);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn update_running_task(
    tasks: &Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: &Path,
    nonfatal_notices: &NonfatalNoticeBuffer,
    task_id: &str,
    update: &PipelineProgressUpdate,
    retry_count: u8,
) {
    update_running_task_with_events(
        tasks,
        task_store_path,
        nonfatal_notices,
        &None,
        task_id,
        update,
        retry_count,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_running_task_with_events(
    tasks: &Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: &Path,
    nonfatal_notices: &NonfatalNoticeBuffer,
    task_events: &Option<TaskEventSink>,
    task_id: &str,
    update: &PipelineProgressUpdate,
    retry_count: u8,
) {
    if let Err(error) =
        update_task_entry_with_events(tasks, task_store_path, task_events, task_id, |task| {
            state::apply_running_progress(task, update, retry_count);
        })
    {
        report_task_runner_nonfatal_error(nonfatal_notices, "update_running_task", task_id, &error);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_job_result_with_events(
    tasks: &Arc<Mutex<HashMap<String, TranslationTask>>>,
    task_store_path: &Path,
    nonfatal_notices: &NonfatalNoticeBuffer,
    task_events: &Option<TaskEventSink>,
    task_id: &str,
    artifact_dir: &Path,
    result: Result<PipelineResult, AppError>,
    retry_count: u8,
    stop_reason: Option<TaskStopReason>,
) {
    match result {
        Ok(pipeline_result) => {
            // 块级失败隔离：任务照常完成，但存在重试耗尽的块时进非致命通知，
            // 便于前端提示「部分块保留原文，重跑可补译」。
            if pipeline_result.failed_chunks > 0 {
                report_nonfatal_error(
                    Some(nonfatal_notices),
                    "task-runner",
                    &format!(
                        "task_id={task_id} failed_chunks={} failed_chunk_indexes={:?}",
                        pipeline_result.failed_chunks, pipeline_result.failed_chunk_indexes
                    ),
                    &AppError::new(
                        "PARTIAL_TRANSLATION_FAILURE",
                        format!(
                            "任务已完成，但 {} 个正文块重试耗尽仍失败（产物保留原文；重跑任务可补译这些块）",
                            pipeline_result.failed_chunks
                        ),
                        false,
                    ),
                );
            }
            if let Err(error) = update_task_entry_with_events(
                tasks,
                task_store_path,
                task_events,
                task_id,
                |task| {
                    state::apply_task_success(task, &pipeline_result, artifact_dir);
                },
            ) {
                report_task_runner_nonfatal_error(
                    nonfatal_notices,
                    "apply_task_success",
                    task_id,
                    &error,
                );
            }
        }
        Err(error) => {
            if let Err(writeback_error) = update_task_entry_with_events(
                tasks,
                task_store_path,
                task_events,
                task_id,
                |task| {
                    state::apply_task_failure_with_reason(task, &error, retry_count, stop_reason);
                },
            ) {
                report_task_runner_nonfatal_error(
                    nonfatal_notices,
                    "apply_task_failure",
                    task_id,
                    &writeback_error,
                );
            }
        }
    }
}
