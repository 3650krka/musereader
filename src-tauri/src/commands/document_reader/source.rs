use super::DocumentArtifactSource;
use crate::commands::{lock_mutex, AppError, AppState, TranslationTask};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) fn resolve_document_source(
    state: &AppState,
    task_id: &str,
) -> Result<DocumentArtifactSource, AppError> {
    let task = {
        let tasks = lock_mutex(&state.tasks, "翻译任务")?;
        tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("未找到对应文档任务"))?
    };
    resolve_task_document_source(&task, &state.artifact_root_dir)
}

pub(super) fn resolve_task_document_source(
    task: &TranslationTask,
    artifact_root: &Path,
) -> Result<DocumentArtifactSource, AppError> {
    validate_task_id(&task.id)?;
    let managed_root = canonical_directory(artifact_root, "工件根目录")?;
    let expected_dir =
        canonical_managed_task_directory(&managed_root.join(&task.id), &managed_root)?;
    validate_declared_artifact_dir(task, &expected_dir)?;
    let html_path = resolve_task_html_path(task, &expected_dir)?;

    Ok(DocumentArtifactSource {
        task_id: task.id.clone(),
        artifact_dir: expected_dir,
        html_path,
    })
}

fn validate_task_id(task_id: &str) -> Result<(), AppError> {
    let mut components = Path::new(task_id).components();
    let is_single_component =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if task_id.trim().is_empty() || !is_single_component {
        return Err(AppError::invalid_input("任务标识不合法"));
    }
    Ok(())
}

fn validate_declared_artifact_dir(
    task: &TranslationTask,
    expected_dir: &Path,
) -> Result<(), AppError> {
    let Some(raw_path) = task.artifact_paths.artifact_dir.as_deref() else {
        return Ok(());
    };
    let declared = canonical_directory(&absolute_path(raw_path)?, "任务工件目录")?;
    if declared != expected_dir {
        return Err(AppError::invalid_input("任务工件目录不属于当前任务"));
    }
    Ok(())
}

fn resolve_task_html_path(
    task: &TranslationTask,
    artifact_dir: &Path,
) -> Result<PathBuf, AppError> {
    let mut candidates = Vec::new();
    if let Some(path) = task.artifact_paths.output_html_path.as_deref() {
        candidates.push(resolve_artifact_path(path, artifact_dir));
    }
    if let Some(path) = task.html_output_path.as_deref() {
        candidates.push(resolve_artifact_path(path, artifact_dir));
    }
    candidates.push(artifact_dir.join("translated.html"));

    for candidate in candidates {
        if candidate.exists() {
            return validate_html_path(&candidate, artifact_dir);
        }
    }
    Err(AppError::not_found("该文档没有可阅读的 HTML 成书"))
}

fn resolve_artifact_path(raw_path: &str, artifact_dir: &Path) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        artifact_dir.join(path)
    }
}

fn validate_html_path(path: &Path, artifact_dir: &Path) -> Result<PathBuf, AppError> {
    let canonical = canonical_file(path, "HTML 成书")?;
    let is_html = canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        });
    if !is_html || canonical.parent() != Some(artifact_dir) {
        return Err(AppError::invalid_input("HTML 成书不属于当前任务工件目录"));
    }
    Ok(canonical)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, AppError> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| AppError::not_found(format!("{label}不存在: {error}")))?;
    if !canonical.is_dir() {
        return Err(AppError::invalid_input(format!("{label}不是目录")));
    }
    Ok(canonical)
}

fn canonical_managed_task_directory(path: &Path, managed_root: &Path) -> Result<PathBuf, AppError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| AppError::not_found(format!("任务工件目录不存在: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::invalid_input("任务工件目录类型不合法"));
    }

    let canonical = fs::canonicalize(path)
        .map_err(|error| AppError::not_found(format!("任务工件目录不存在: {error}")))?;
    if canonical.parent() != Some(managed_root) {
        return Err(AppError::invalid_input("任务工件目录越过工件根目录"));
    }
    Ok(canonical)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, AppError> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| AppError::not_found(format!("{label}不存在: {error}")))?;
    if !canonical.is_file() {
        return Err(AppError::invalid_input(format!("{label}不是文件")));
    }
    Ok(canonical)
}

fn absolute_path(raw_path: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| AppError::internal(format!("解析当前目录失败: {error}")))
}
