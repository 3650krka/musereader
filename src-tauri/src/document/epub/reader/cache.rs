use super::sanitize::sanitize_reader_html;
use crate::error::AppError;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::ZipArchive;

const CACHE_SCHEMA_VERSION: &str = "v2";
const READY_MARKER: &str = ".musetranslate-reader-ready";
const MAX_ENTRY_BYTES: u64 = 96 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 768 * 1024 * 1024;
const MAX_ENTRY_COUNT: usize = 20_000;

pub(super) struct ReaderCacheEntry {
    pub key: String,
    pub root: PathBuf,
}

pub(super) fn ensure_reader_cache(
    epub_path: &Path,
    cache_root: &Path,
) -> Result<ReaderCacheEntry, AppError> {
    fs::create_dir_all(cache_root)
        .map_err(|error| AppError::internal(format!("创建 EPUB 阅读缓存失败: {error}")))?;
    let key = reader_cache_key(epub_path)?;
    let target = cache_root.join(&key);
    if target.join(READY_MARKER).is_file() {
        return Ok(ReaderCacheEntry { key, root: target });
    }

    let temporary = cache_root.join(format!(".{key}-{}.tmp", Uuid::new_v4()));
    fs::create_dir_all(&temporary)
        .map_err(|error| AppError::internal(format!("创建 EPUB 临时缓存失败: {error}")))?;
    let preparation = extract_archive(epub_path, &temporary)
        .and_then(|()| write_ready_marker(&temporary))
        .and_then(|()| publish_cache(&temporary, &target));
    if let Err(error) = preparation {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    Ok(ReaderCacheEntry { key, root: target })
}

fn reader_cache_key(epub_path: &Path) -> Result<String, AppError> {
    let mut file = File::open(epub_path)
        .map_err(|error| AppError::internal(format!("打开 EPUB 计算缓存键失败: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(CACHE_SCHEMA_VERSION.as_bytes());
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| AppError::internal(format!("读取 EPUB 计算缓存键失败: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{CACHE_SCHEMA_VERSION}-{hex}"))
}

fn extract_archive(epub_path: &Path, destination: &Path) -> Result<(), AppError> {
    let file = File::open(epub_path)
        .map_err(|error| AppError::internal(format!("打开 EPUB 阅读包失败: {error}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| AppError::invalid_input(format!("EPUB 压缩包无效: {error}")))?;
    if archive.len() > MAX_ENTRY_COUNT {
        return Err(AppError::invalid_input("EPUB 文件条目过多"));
    }

    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| AppError::internal(format!("读取 EPUB 条目失败: {error}")))?;
        validate_entry_size(entry.size(), &mut total_bytes)?;
        let Some(relative_path) = entry.enclosed_name() else {
            return Err(AppError::invalid_input("EPUB 包含不安全路径"));
        };
        if is_symbolic_link(entry.unix_mode()) {
            return Err(AppError::invalid_input("EPUB 包含符号链接"));
        }
        let output_path = destination.join(relative_path);
        if entry.is_dir() {
            create_directory(&output_path)?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            create_directory(parent)?;
        }
        write_entry(&mut entry, &output_path)?;
    }
    Ok(())
}

fn validate_entry_size(size: u64, total_bytes: &mut u64) -> Result<(), AppError> {
    if size > MAX_ENTRY_BYTES {
        return Err(AppError::invalid_input("EPUB 单个资源过大"));
    }
    *total_bytes = total_bytes.saturating_add(size);
    if *total_bytes > MAX_TOTAL_BYTES {
        return Err(AppError::invalid_input("EPUB 解压后体积过大"));
    }
    Ok(())
}

fn is_symbolic_link(unix_mode: Option<u32>) -> bool {
    unix_mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn create_directory(path: &Path) -> Result<(), AppError> {
    fs::create_dir_all(path)
        .map_err(|error| AppError::internal(format!("创建 EPUB 缓存目录失败: {error}")))
}

fn write_entry<R: Read>(entry: &mut R, output_path: &Path) -> Result<(), AppError> {
    if is_html_path(output_path) {
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| AppError::internal(format!("读取 EPUB XHTML 失败: {error}")))?;
        let html = sanitize_reader_html(&String::from_utf8_lossy(&bytes))?;
        fs::write(output_path, html.as_bytes())
            .map_err(|error| AppError::internal(format!("写入 EPUB XHTML 缓存失败: {error}")))?;
        return Ok(());
    }

    let mut output = File::create(output_path)
        .map_err(|error| AppError::internal(format!("创建 EPUB 缓存文件失败: {error}")))?;
    std::io::copy(entry, &mut output)
        .map_err(|error| AppError::internal(format!("写入 EPUB 缓存资源失败: {error}")))?;
    output
        .flush()
        .map_err(|error| AppError::internal(format!("刷新 EPUB 缓存资源失败: {error}")))
}

fn is_html_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("xhtml")
                || extension.eq_ignore_ascii_case("html")
                || extension.eq_ignore_ascii_case("htm")
        })
}

fn write_ready_marker(directory: &Path) -> Result<(), AppError> {
    fs::write(directory.join(READY_MARKER), CACHE_SCHEMA_VERSION)
        .map_err(|error| AppError::internal(format!("写入 EPUB 缓存标记失败: {error}")))
}

fn publish_cache(temporary: &Path, target: &Path) -> Result<(), AppError> {
    if target.join(READY_MARKER).is_file() {
        let _ = fs::remove_dir_all(temporary);
        return Ok(());
    }
    remove_stale_cache(target)?;
    match fs::rename(temporary, target) {
        Ok(()) => Ok(()),
        Err(_error) if target.join(READY_MARKER).is_file() => {
            let _ = fs::remove_dir_all(temporary);
            Ok(())
        }
        Err(error) => Err(AppError::internal(format!(
            "发布 EPUB 阅读缓存失败: {error}"
        ))),
    }
}

fn remove_stale_cache(target: &Path) -> Result<(), AppError> {
    if target.is_dir() {
        fs::remove_dir_all(target)
            .map_err(|error| AppError::internal(format!("清理失效 EPUB 缓存失败: {error}")))?;
    } else if target.exists() {
        fs::remove_file(target)
            .map_err(|error| AppError::internal(format!("清理失效 EPUB 缓存文件失败: {error}")))?;
    }
    Ok(())
}
