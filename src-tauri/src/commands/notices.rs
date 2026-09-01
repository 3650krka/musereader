use super::{
    lock_mutex, AppError, AppState, NonfatalNoticeBuffer, RuntimeNotice, MAX_NONFATAL_NOTICES,
};
use chrono::Utc;

fn extract_task_id(detail: &str) -> Option<String> {
    detail
        .split_whitespace()
        .find_map(|segment| segment.strip_prefix("task_id=").map(ToString::to_string))
}

fn push_nonfatal_notice(
    notices: &NonfatalNoticeBuffer,
    scope: &str,
    detail: &str,
    error: &AppError,
) {
    if let Ok(mut guard) = notices.lock() {
        if guard.len() >= MAX_NONFATAL_NOTICES {
            guard.pop_front();
        }
        guard.push_back(RuntimeNotice {
            timestamp: Utc::now().to_rfc3339(),
            scope: scope.to_string(),
            task_id: extract_task_id(detail),
            detail: detail.to_string(),
            code: error.code.to_string(),
            message: error.message.clone(),
        });
    }
}

pub(super) fn report_nonfatal_error(
    notices: Option<&NonfatalNoticeBuffer>,
    scope: &str,
    detail: &str,
    error: &AppError,
) {
    if let Some(notices) = notices {
        push_nonfatal_notice(notices, scope, detail, error);
    }
    eprintln!(
        "[{scope}] {detail} code={} message={}",
        error.code, error.message
    );
}

pub(super) fn notice_matches_scope(notice: &RuntimeNotice, scope: Option<&str>) -> bool {
    scope.is_none_or(|expected| notice.scope == expected)
}

pub(super) fn notice_matches_task_id(notice: &RuntimeNotice, task_id: Option<&str>) -> bool {
    task_id.is_none_or(|expected| notice.task_id.as_deref() == Some(expected))
}

#[tauri::command]
pub async fn get_runtime_notices(
    limit: Option<usize>,
    scope: Option<String>,
    task_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RuntimeNotice>, AppError> {
    let notices = lock_mutex(&state.nonfatal_notices, "运行告警")?;
    let take = limit
        .unwrap_or(MAX_NONFATAL_NOTICES)
        .min(MAX_NONFATAL_NOTICES);
    Ok(notices
        .iter()
        .rev()
        .filter(|notice| notice_matches_scope(notice, scope.as_deref()))
        .filter(|notice| notice_matches_task_id(notice, task_id.as_deref()))
        .take(take)
        .cloned()
        .collect())
}

#[tauri::command]
pub async fn clear_runtime_notices(
    scope: Option<String>,
    task_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<usize, AppError> {
    let mut notices = lock_mutex(&state.nonfatal_notices, "运行告警")?;
    let before = notices.len();
    notices.retain(|notice| {
        !(notice_matches_scope(notice, scope.as_deref())
            && notice_matches_task_id(notice, task_id.as_deref()))
    });
    Ok(before.saturating_sub(notices.len()))
}
