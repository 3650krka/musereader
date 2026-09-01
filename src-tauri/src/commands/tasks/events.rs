//! 任务状态变更事件推送。
//!
//! 事件通道（`Arc<dyn Fn>`）而非直接持有 `AppHandle`：
//! - 保持 AppState 与 lib 测试对 tauri runtime 零依赖（测试注入 sink 即可断言）。
//! - 生产环境在 lib.rs setup 中注入 `app_handle.emit` 闭包。
//!
//! 事件契约（前端 listen）：
//! - `task-progress`：payload 为完整 TranslationTask（创建/进度/成功/失败/取消/暂停/重排队）。
//! - `task-deleted`：payload 为 `{ "taskId": "..." }`（删除后任务本体已不存在）。

use super::TranslationTask;
use serde::Serialize;
use std::sync::Arc;

pub const TASK_PROGRESS_EVENT: &str = "task-progress";
pub const TASK_DELETED_EVENT: &str = "task-deleted";

pub(crate) type TaskEventSink = Arc<dyn Fn(&str, String) + Send + Sync>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDeletedPayload {
    pub task_id: String,
}

pub(crate) fn emit_json(sink: &Option<TaskEventSink>, event: &str, payload: &impl Serialize) {
    let Some(sink) = sink else {
        return;
    };
    match serde_json::to_string(payload) {
        Ok(json) => sink(event, json),
        Err(error) => {
            eprintln!("task event serialize failed: event={event} error={error}");
        }
    }
}

pub(crate) fn emit_task_progress(sink: &Option<TaskEventSink>, task: &TranslationTask) {
    emit_json(sink, TASK_PROGRESS_EVENT, task);
}

pub(crate) fn emit_task_deleted(sink: &Option<TaskEventSink>, task_id: &str) {
    emit_json(
        sink,
        TASK_DELETED_EVENT,
        &TaskDeletedPayload {
            task_id: task_id.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn collect_sink() -> (Option<TaskEventSink>, Arc<Mutex<Vec<(String, String)>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let sink: TaskEventSink = Arc::new(move |event: &str, payload: String| {
            captured
                .lock()
                .expect("event capture lock")
                .push((event.to_string(), payload));
        });
        (Some(sink), events)
    }

    #[test]
    fn emit_task_deleted_serializes_camel_case_payload() {
        let (sink, events) = collect_sink();
        emit_task_deleted(&sink, "task-1");
        let events = events.lock().expect("event capture lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, TASK_DELETED_EVENT);
        assert_eq!(events[0].1, r#"{"taskId":"task-1"}"#);
    }

    #[test]
    fn emit_json_is_noop_without_sink() {
        // 无 sink（未 setup 的测试环境）时不得 panic。
        emit_json(
            &None,
            TASK_PROGRESS_EVENT,
            &TaskDeletedPayload {
                task_id: "task-1".to_string(),
            },
        );
    }
}
