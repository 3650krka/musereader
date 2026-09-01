use super::*;
use crate::commands::tasks::{TaskArtifactPaths, TaskPhase, TaskStatus};
use crate::commands::TranslationTask;
use std::fs;

fn test_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("musetranslate-{label}-{}", uuid::Uuid::new_v4()))
}

fn sample_task(task_id: &str, artifact_dir: &Path, html_path: &Path) -> TranslationTask {
    TranslationTask {
        id: task_id.to_string(),
        filename: "paper.pdf".to_string(),
        pdf_path: "paper.pdf".to_string(),
        status: TaskStatus::Completed,
        phase: TaskPhase::Completed,
        progress: 100,
        message: "done".to_string(),
        article_type: "academic".to_string(),
        concurrent: true,
        max_workers: 4,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        output_path: None,
        html_output_path: Some(html_path.to_string_lossy().to_string()),
        cover_path: None,
        artifact_paths: TaskArtifactPaths {
            artifact_dir: Some(artifact_dir.to_string_lossy().to_string()),
            output_html_path: Some(html_path.to_string_lossy().to_string()),
            ..TaskArtifactPaths::default()
        },
        total_chunks: 1,
        translated_chunks: 1,
        retry_count: 0,
        source_hash: None,
        last_error: None,
    }
}

#[test]
fn prepares_html_and_image_resources_only() {
    let root = test_root("document-reader-cache");
    let artifact_root = root.join("artifacts");
    let artifact_dir = artifact_root.join("task-1");
    let image_dir = artifact_dir.join("imgs");
    let cache_root = root.join("cache");
    let html_path = artifact_dir.join("translated.html");
    fs::create_dir_all(&image_dir).expect("image directory should exist");
    fs::write(&html_path, "<img src=\"imgs/figure.png\">").expect("html artifact should exist");
    fs::write(image_dir.join("figure.png"), b"image").expect("image artifact should exist");
    fs::write(artifact_dir.join("checkpoint.json"), b"metadata")
        .expect("metadata artifact should exist");
    let task = sample_task("task-1", &artifact_dir, &html_path);

    let source = resolve_task_document_source(&task, &artifact_root)
        .expect("managed document source should resolve");
    let package = prepare_document_reader_cache(&source, &cache_root)
        .expect("document cache should be prepared");
    let package_root = PathBuf::from(&package.root_path);

    assert!(Path::new(&package.html_path).is_file());
    assert!(package_root.join("imgs/figure.png").is_file());
    assert!(!package_root.join("checkpoint.json").exists());
    fs::remove_dir_all(root).expect("test root should be removable");
}

#[test]
fn resource_change_produces_a_new_cache_key() {
    let root = test_root("document-reader-version");
    let artifact_root = root.join("artifacts");
    let artifact_dir = artifact_root.join("task-1");
    let image_dir = artifact_dir.join("imgs");
    let cache_root = root.join("cache");
    let html_path = artifact_dir.join("translated.html");
    fs::create_dir_all(&image_dir).expect("image directory should exist");
    fs::write(&html_path, "<img src=\"imgs/figure.png\">").expect("html artifact should exist");
    fs::write(image_dir.join("figure.png"), b"first").expect("image artifact should exist");
    let task = sample_task("task-1", &artifact_dir, &html_path);
    let source = resolve_task_document_source(&task, &artifact_root)
        .expect("managed document source should resolve");

    let first = prepare_document_reader_cache(&source, &cache_root)
        .expect("first document cache should be prepared");
    fs::write(image_dir.join("figure.png"), b"second").expect("image artifact should update");
    let second = prepare_document_reader_cache(&source, &cache_root)
        .expect("second document cache should be prepared");

    assert_ne!(first.cache_key, second.cache_key);
    assert_ne!(first.root_path, second.root_path);
    fs::remove_dir_all(root).expect("test root should be removable");
}

#[test]
fn rejects_html_outside_the_task_artifact_directory() {
    let root = test_root("document-reader-boundary");
    let artifact_root = root.join("artifacts");
    let artifact_dir = artifact_root.join("task-1");
    let outside_html = root.join("outside.html");
    fs::create_dir_all(&artifact_dir).expect("artifact directory should exist");
    fs::write(&outside_html, "<p>outside</p>").expect("outside html should exist");
    fs::write(artifact_dir.join("translated.html"), "<p>inside</p>")
        .expect("fallback html should exist");
    let task = sample_task("task-1", &artifact_dir, &outside_html);

    let error = resolve_task_document_source(&task, &artifact_root)
        .expect_err("outside HTML must be rejected");

    assert!(error.message.contains("不属于当前任务工件目录"));
    fs::remove_dir_all(root).expect("test root should be removable");
}

#[test]
fn rejects_task_ids_that_are_paths() {
    let root = test_root("document-reader-task-id");
    let artifact_root = root.join("artifacts");
    let artifact_dir = artifact_root.join("task-1");
    let html_path = artifact_dir.join("translated.html");
    fs::create_dir_all(&artifact_dir).expect("artifact directory should exist");
    fs::write(&html_path, "<p>inside</p>").expect("html artifact should exist");
    let task = sample_task("../task-1", &artifact_dir, &html_path);

    let error = resolve_task_document_source(&task, &artifact_root)
        .expect_err("path-shaped task IDs must be rejected");

    assert!(error.message.contains("任务标识不合法"));
    fs::remove_dir_all(root).expect("test root should be removable");
}

#[test]
#[ignore = "requires MUSETRANSLATE_DOCUMENT_ARTIFACT to reference a real artifact directory"]
fn prepares_real_document_artifact() {
    let artifact_dir = std::env::var_os("MUSETRANSLATE_DOCUMENT_ARTIFACT")
        .map(PathBuf::from)
        .expect("MUSETRANSLATE_DOCUMENT_ARTIFACT should be set");
    let artifact_dir = fs::canonicalize(artifact_dir).expect("artifact directory should resolve");
    let artifact_root = artifact_dir.parent().expect("artifact root should exist");
    let task_id = artifact_dir
        .file_name()
        .and_then(|value| value.to_str())
        .expect("artifact directory should have a UTF-8 task id");
    let html_path = artifact_dir.join("translated.html");
    let cache_root = test_root("document-reader-real-artifact");
    let task = sample_task(task_id, &artifact_dir, &html_path);

    let source = resolve_task_document_source(&task, artifact_root)
        .expect("real document source should resolve");
    let package = prepare_document_reader_cache(&source, &cache_root)
        .expect("real document cache should be prepared");
    let package_root = PathBuf::from(&package.root_path);
    let source_images = collect_relative_files(&artifact_dir.join("imgs"));
    let cached_images = collect_relative_files(&package_root.join("imgs"));

    assert!(!source_images.is_empty());
    assert_eq!(cached_images, source_images);
    for relative_path in source_images {
        let source_size = file_size(&artifact_dir.join("imgs").join(&relative_path));
        let cached_size = file_size(&package_root.join("imgs").join(relative_path));
        assert!(source_size > 0);
        assert_eq!(cached_size, source_size);
    }
    assert!(Path::new(&package.html_path).is_file());
    assert!(!package_root.join("checkpoint.json").exists());
    fs::remove_dir_all(cache_root).expect("real artifact cache should be removable");
}

fn collect_relative_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("image directory should be readable") {
            let path = entry.expect("image entry should be readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.is_file() {
                files.push(
                    path.strip_prefix(root)
                        .expect("image should remain below its root")
                        .to_path_buf(),
                );
            }
        }
    }
    files.sort();
    files
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .expect("cached image metadata should be readable")
        .len()
}
