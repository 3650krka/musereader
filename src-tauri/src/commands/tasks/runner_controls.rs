use super::types::TaskStopReason;
use super::{lock_mutex, AppError, TaskControl};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) fn register_spawned_task_control(
    controls: &Arc<Mutex<HashMap<String, TaskControl>>>,
    task_id: &str,
    cancellation: CancellationToken,
    stop_reason: Arc<Mutex<Option<TaskStopReason>>>,
    handle: JoinHandle<()>,
) -> Result<(), AppError> {
    let mut controls = match lock_mutex(controls, "任务控制") {
        Ok(controls) => controls,
        Err(error) => {
            cancellation.cancel();
            handle.abort();
            return Err(error);
        }
    };
    if controls.contains_key(task_id) {
        cancellation.cancel();
        handle.abort();
        return Err(AppError::conflict("任务控制已存在，无法重复启动"));
    }
    controls.insert(
        task_id.to_string(),
        TaskControl {
            cancellation,
            stop_reason,
            _handle: handle,
        },
    );
    Ok(())
}

fn request_stop(
    controls: &Arc<Mutex<HashMap<String, TaskControl>>>,
    task_id: &str,
    reason: TaskStopReason,
) -> Result<bool, AppError> {
    let controls = lock_mutex(controls, "任务控制")?;
    if let Some(control) = controls.get(task_id) {
        *lock_mutex(&control.stop_reason, "任务停止原因")? = Some(reason);
        control.cancellation.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(super) fn cancel_task_control(
    controls: &Arc<Mutex<HashMap<String, TaskControl>>>,
    task_id: &str,
) -> Result<bool, AppError> {
    request_stop(controls, task_id, TaskStopReason::Cancel)
}

pub(super) fn pause_task_control(
    controls: &Arc<Mutex<HashMap<String, TaskControl>>>,
    task_id: &str,
) -> Result<bool, AppError> {
    request_stop(controls, task_id, TaskStopReason::Pause)
}

pub(super) fn remove_task_control(
    controls: &Arc<Mutex<HashMap<String, TaskControl>>>,
    task_id: &str,
) -> Result<(), AppError> {
    let mut guard = lock_mutex(controls, "任务控制")?;
    guard.remove(task_id);
    Ok(())
}

pub(super) fn abort_task_control(
    controls: &Arc<Mutex<HashMap<String, TaskControl>>>,
    task_id: &str,
) -> Result<bool, AppError> {
    let mut guard = lock_mutex(controls, "任务控制")?;
    if let Some(control) = guard.remove(task_id) {
        let _ = lock_mutex(&control.stop_reason, "任务停止原因").map(|mut reason| {
            *reason = Some(TaskStopReason::Cancel);
        });
        control.cancellation.cancel();
        control._handle.abort();
        Ok(true)
    } else {
        Ok(false)
    }
}
