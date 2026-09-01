use super::io::{read_optional_json, remove_dir_if_exists, remove_file_if_exists};
use super::*;
use crate::document::{build_source_blocks_from_markdown, EpubBlock, SourceBlock, TocArtifact};
use crate::ocr::{OcrDocument, OcrPageLayout};
use base64::Engine as _;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

pub(crate) async fn load_source_document(
    paths: &ArtifactPaths,
) -> Result<Option<OcrDocument>, AppError> {
    if !paths.source_markdown_path.exists() {
        return Ok(None);
    }

    let markdown = tokio::fs::read_to_string(&paths.source_markdown_path)
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "read source markdown failed({}): {error}",
                paths.source_markdown_path.display()
            ))
        })?;
    let page_layouts =
        read_optional_json::<Vec<OcrPageLayout>>(&paths.source_layouts_path)?.unwrap_or_default();
    let toc = read_optional_json::<TocArtifact>(&paths.source_toc_path)?;
    let source_blocks =
        read_optional_json::<Vec<SourceBlock>>(&paths.source_blocks_path)?.unwrap_or_default();
    let markdown_images =
        read_optional_json::<HashMap<String, String>>(&paths.source_images_map_path)?
            .unwrap_or_default();

    if !markdown_images.is_empty() {
        materialize_missing_markdown_images(paths, &markdown_images).await?;
    }

    let has_typed_blocks = !source_blocks.is_empty();
    let computed_source_blocks = if has_typed_blocks {
        source_blocks
    } else {
        build_source_blocks_from_markdown(&markdown)
    };
    let legacy_epub_blocks = if has_typed_blocks {
        Vec::new()
    } else {
        read_optional_json::<Vec<EpubBlock>>(&paths.source_blocks_path)?.unwrap_or_default()
    };

    Ok(Some(OcrDocument {
        markdown,
        cover_data_url: None,
        page_layouts,
        markdown_images,
        toc,
        epub_blocks: legacy_epub_blocks,
        source_blocks: computed_source_blocks,
    }))
}

pub(crate) async fn persist_source_document(
    paths: &ArtifactPaths,
    source_document: &OcrDocument,
) -> Result<(), AppError> {
    super::write_utf8(&paths.source_markdown_path, &source_document.markdown).await?;

    if source_document.page_layouts.is_empty() {
        remove_file_if_exists(&paths.source_layouts_path)?;
    } else {
        super::write_json(&paths.source_layouts_path, &source_document.page_layouts).await?;
    }

    if let Some(toc) = source_document.toc.as_ref() {
        super::write_json(&paths.source_toc_path, toc).await?;
    } else {
        remove_file_if_exists(&paths.source_toc_path)?;
    }

    if source_document.source_blocks.is_empty() {
        remove_file_if_exists(&paths.source_blocks_path)?;
    } else {
        super::write_json(&paths.source_blocks_path, &source_document.source_blocks).await?;
    }

    if source_document.markdown_images.is_empty() {
        remove_file_if_exists(&paths.source_images_map_path)?;
        remove_dir_if_exists(&paths.source_images_dir)?;
    } else {
        super::write_json(
            &paths.source_images_map_path,
            &source_document.markdown_images,
        )
        .await?;
        persist_markdown_images(paths, &source_document.markdown_images).await?;
    }

    Ok(())
}

pub(crate) async fn persist_markdown_images_only(
    paths: &ArtifactPaths,
    markdown_images: &HashMap<String, String>,
) -> Result<(), AppError> {
    if markdown_images.is_empty() {
        return Ok(());
    }
    super::write_json(&paths.source_images_map_path, markdown_images).await?;
    persist_markdown_images(paths, markdown_images).await
}

async fn materialize_missing_markdown_images(
    paths: &ArtifactPaths,
    markdown_images: &HashMap<String, String>,
) -> Result<(), AppError> {
    let missing_images = markdown_images
        .iter()
        .filter(|(relative_path, _)| {
            !resolve_artifact_image_path(artifact_root(paths), relative_path).exists()
        })
        .map(|(relative_path, value)| (relative_path.clone(), value.clone()))
        .collect::<Vec<_>>();

    if missing_images.is_empty() {
        return Ok(());
    }

    let missing_map = missing_images.into_iter().collect::<HashMap<_, _>>();
    persist_markdown_images(paths, &missing_map).await
}

async fn persist_markdown_images(
    paths: &ArtifactPaths,
    markdown_images: &HashMap<String, String>,
) -> Result<(), AppError> {
    let client = reqwest::Client::new();
    for (relative_path, value) in markdown_images {
        persist_single_markdown_image(&client, artifact_root(paths), relative_path, value).await?;
    }
    Ok(())
}

async fn persist_single_markdown_image(
    client: &reqwest::Client,
    artifact_root: &Path,
    relative_path: &str,
    raw_value: &str,
) -> Result<(), AppError> {
    let output_path = resolve_artifact_image_path(artifact_root, relative_path);
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::internal(format!(
                "create image directory failed({}): {error}",
                parent.display()
            ))
        })?;
    }

    let bytes = match classify_image_payload(raw_value)? {
        ImagePayload::Inline(bytes) => bytes,
        ImagePayload::Remote(url) => fetch_remote_image_bytes(client, url).await?,
    };
    tokio::fs::write(&output_path, &bytes)
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "write OCR image failed({}): {error}",
                output_path.display()
            ))
        })
}

fn artifact_root(paths: &ArtifactPaths) -> &Path {
    paths
        .source_markdown_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
}

fn resolve_artifact_image_path(artifact_root: &Path, relative_path: &str) -> PathBuf {
    artifact_root.join(sanitize_relative_path(relative_path))
}

fn sanitize_relative_path(raw: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(value) => path.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    path
}

enum ImagePayload<'a> {
    Inline(Vec<u8>),
    Remote(&'a str),
}

fn classify_image_payload(raw_value: &str) -> Result<ImagePayload<'_>, AppError> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return Err(AppError::internal("OCR image payload is empty"));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(ImagePayload::Remote(trimmed));
    }
    if let Some((header, data)) = trimmed.split_once(',') {
        if header.starts_with("data:image") {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|error| {
                    AppError::internal(format!("decode OCR data-url image failed: {error}"))
                })?;
            return Ok(ImagePayload::Inline(bytes));
        }
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .map_err(|error| AppError::internal(format!("decode OCR image base64 failed: {error}")))?;
    Ok(ImagePayload::Inline(bytes))
}

async fn fetch_remote_image_bytes(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, AppError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| AppError::internal(format!("download OCR image failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::internal(format!(
            "download OCR image returned HTTP {} for {url}",
            response.status()
        )));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| AppError::internal(format!("read OCR image response failed: {error}")))
}
