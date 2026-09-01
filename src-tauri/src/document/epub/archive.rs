use crate::error::AppError;
use base64::Engine as _;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

pub(super) fn open_epub_archive(path: &Path) -> Result<ZipArchive<File>, AppError> {
    let file = File::open(path)
        .map_err(|error| AppError::internal(format!("open EPUB failed: {error}")))?;
    ZipArchive::new(file)
        .map_err(|error| AppError::internal(format!("read EPUB archive failed: {error}")))
}

pub(super) fn extract_epub_cover(
    archive: &mut ZipArchive<File>,
    base_dir: &str,
    cover_href: Option<String>,
) -> Option<String> {
    let cover_href = cover_href?;
    let joined_path = crate::document::epub::path::join_epub_path(base_dir, &cover_href);
    read_cover_data_url(archive, &joined_path).ok()
}

pub(super) fn extract_epub_images(
    archive: &mut ZipArchive<File>,
    base_dir: &str,
    image_hrefs: Vec<String>,
) -> HashMap<String, String> {
    let mut images = HashMap::new();
    for href in image_hrefs {
        let archive_path = crate::document::epub::path::join_epub_path(base_dir, &href);
        if let Ok(data_url) = read_cover_data_url(archive, &archive_path) {
            images.insert(crate::document::epub::artifact_image_path(&href), data_url);
        }
    }
    images
}

pub(super) fn read_cover_data_url(
    archive: &mut ZipArchive<File>,
    path: &str,
) -> Result<String, AppError> {
    let mut entry = archive
        .by_name(path)
        .map_err(|error| AppError::internal(format!("read EPUB cover failed: {error}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::internal(format!("read EPUB cover bytes failed: {error}")))?;
    // MIME 用文件头 magic bytes 探测（不是扩展名），避免 .jpeg 内容是 PNG 时标错。
    let mime = detect_image_mime(&bytes).unwrap_or_else(|| {
        match Path::new(path).extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("png") => "image/png",
            Some(ext) if ext.eq_ignore_ascii_case("webp") => "image/webp",
            Some(ext) if ext.eq_ignore_ascii_case("gif") => "image/gif",
            Some(ext) if ext.eq_ignore_ascii_case("svg") => "image/svg+xml",
            _ => "image/jpeg",
        }
    });
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// 用文件头 magic bytes 探测图片 MIME 类型。
fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"<?xml") || bytes.starts_with(b"<svg") {
        return Some("image/svg+xml");
    }
    None
}

pub(super) fn read_zip_entry_to_string(
    archive: &mut ZipArchive<File>,
    path: &str,
) -> Result<String, AppError> {
    let mut entry = archive
        .by_name(path)
        .map_err(|error| AppError::internal(format!("read EPUB entry failed ({path}): {error}")))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|error| {
        AppError::internal(format!("read EPUB content failed ({path}): {error}"))
    })?;
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        return Ok(text);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
