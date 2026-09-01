use super::{AppError, TranslationTask};
use std::path::{Path, PathBuf};

pub(super) fn ensure_document_path(document_path: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(document_path);
    if !path.exists() {
        return Err(AppError::not_found("文件不存在"));
    }
    if !path.is_file() {
        return Err(AppError::invalid_input("选择的路径不是文件"));
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext)
            if ext.eq_ignore_ascii_case("pdf")
                || ext.eq_ignore_ascii_case("epub")
                || ext.eq_ignore_ascii_case("docx")
                || ext.eq_ignore_ascii_case("md")
                || ext.eq_ignore_ascii_case("markdown")
                || ext.eq_ignore_ascii_case("txt") =>
        {
            Ok(path)
        }
        _ => Err(AppError::invalid_input(
            "仅支持 PDF、EPUB、DOCX、Markdown 或 TXT 文件",
        )),
    }
}

pub(super) fn ensure_epub_path(document_path: &str) -> Result<PathBuf, AppError> {
    let path = ensure_document_path(document_path)?;
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"))
    {
        Ok(path)
    } else {
        Err(AppError::invalid_input("请选择 EPUB 文件"))
    }
}

pub(super) fn create_artifact_dir(root: &Path, task_id: &str) -> Result<PathBuf, AppError> {
    let path = root.join(task_id);
    std::fs::create_dir_all(&path)
        .map_err(|error| AppError::internal(format!("创建工件目录失败: {error}")))?;
    Ok(path)
}

pub(super) fn cleanup_artifact_dir(root: &Path, artifact_dir: &Path) {
    try_remove_managed_dir(&artifact_dir.to_string_lossy(), root);
}

pub(super) fn delete_task_files(
    task: &TranslationTask,
    import_root_dir: &Path,
    artifact_root_dir: &Path,
) {
    try_remove_managed_file(&task.pdf_path, import_root_dir);
    if let Some(artifact_dir) = task.artifact_paths.artifact_dir.as_deref() {
        try_remove_managed_dir(artifact_dir, artifact_root_dir);
    }
}

fn try_remove_managed_file(path: &str, managed_root: &Path) {
    let Ok(candidate) = normalized_path(Path::new(path)) else {
        report_cleanup_error("remove_managed_file_normalize", path, managed_root, None);
        return;
    };
    if !candidate.is_file() || !is_within_root(&candidate, managed_root) {
        return;
    }
    if let Err(error) = std::fs::remove_file(&candidate) {
        report_cleanup_error(
            "remove_managed_file",
            &candidate.to_string_lossy(),
            managed_root,
            Some(&error.to_string()),
        );
    }
}

fn try_remove_managed_dir(path: &str, managed_root: &Path) {
    let Ok(candidate) = normalized_path(Path::new(path)) else {
        report_cleanup_error("remove_managed_dir_normalize", path, managed_root, None);
        return;
    };
    if !candidate.is_dir() || !is_within_root(&candidate, managed_root) {
        return;
    }
    if let Err(error) = std::fs::remove_dir_all(&candidate) {
        report_cleanup_error(
            "remove_managed_dir",
            &candidate.to_string_lossy(),
            managed_root,
            Some(&error.to_string()),
        );
    }
}

fn report_cleanup_error(operation: &str, path: &str, managed_root: &Path, detail: Option<&str>) {
    match detail {
        Some(detail) => eprintln!(
            "[task-cleanup] operation={operation} path={path} managed_root={} detail={detail}",
            managed_root.to_string_lossy()
        ),
        None => eprintln!(
            "[task-cleanup] operation={operation} path={path} managed_root={}",
            managed_root.to_string_lossy()
        ),
    }
}

fn is_within_root(candidate: &Path, managed_root: &Path) -> bool {
    match (normalized_path(candidate), normalized_path(managed_root)) {
        (Ok(candidate), Ok(root)) => candidate.starts_with(root),
        _ => false,
    }
}

fn normalized_path(path: &Path) -> Result<PathBuf, AppError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| AppError::internal(format!("解析当前目录失败: {error}")))?
            .join(path)
    };

    if absolute.exists() {
        std::fs::canonicalize(&absolute)
            .map_err(|error| AppError::internal(format!("规范化路径失败: {error}")))
    } else {
        Ok(absolute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sample_task(pdf_path: &std::path::Path, artifact_dir: &std::path::Path) -> TranslationTask {
        TranslationTask {
            id: "task-1".to_string(),
            filename: "demo.epub".to_string(),
            pdf_path: pdf_path.to_string_lossy().to_string(),
            status: super::super::TaskStatus::Failed,
            phase: super::super::TaskPhase::Failed,
            progress: 100,
            message: "done".to_string(),
            article_type: "fiction".to_string(),
            concurrent: true,
            max_workers: 4,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            output_path: None,
            html_output_path: None,
            cover_path: None,
            artifact_paths: super::super::TaskArtifactPaths {
                artifact_dir: Some(artifact_dir.to_string_lossy().to_string()),
                ..super::super::TaskArtifactPaths::default()
            },
            total_chunks: 0,
            translated_chunks: 0,
            retry_count: 0,
            source_hash: None,
            last_error: None,
        }
    }

    #[test]
    fn cleanup_artifact_dir_removes_managed_directory_only() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-artifact-cleanup-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let managed = root.join("task-1");
        let outside = std::env::temp_dir().join(format!(
            "musetranslate-artifact-outside-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&managed).expect("managed dir should exist");
        fs::create_dir_all(&outside).expect("outside dir should exist");

        cleanup_artifact_dir(&root, &managed);
        cleanup_artifact_dir(&root, &outside);

        assert!(!managed.exists());
        assert!(outside.exists());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn delete_task_files_removes_managed_import_and_artifact_paths() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-task-files-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let import_root = root.join("imports");
        let artifact_root = root.join("artifacts");
        let pdf_path = import_root.join("demo.epub");
        let artifact_dir = artifact_root.join("task-1");
        fs::create_dir_all(&import_root).expect("import root should exist");
        fs::create_dir_all(&artifact_dir).expect("artifact dir should exist");
        fs::write(&pdf_path, b"demo").expect("managed import file should exist");
        let task = sample_task(&pdf_path, &artifact_dir);

        delete_task_files(&task, &import_root, &artifact_root);

        assert!(!pdf_path.exists());
        assert!(!artifact_dir.exists());

        let _ = fs::remove_dir_all(root);
    }
}
