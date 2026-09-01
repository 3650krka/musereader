use super::{DocumentArtifactSource, DocumentReaderPackage};
use crate::commands::AppError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

const IMAGE_DIRECTORY: &str = "imgs";

#[derive(Debug, Clone)]
struct ReaderCacheFile {
    source_path: PathBuf,
    relative_path: PathBuf,
}

pub(super) fn prepare_document_reader_cache(
    source: &DocumentArtifactSource,
    cache_root: &Path,
) -> Result<DocumentReaderPackage, AppError> {
    let files = collect_reader_cache_files(source)?;
    let cache_key = hash_reader_files(&files)?;
    let task_key = hash_text(&source.task_id);
    let package_root = cache_root.join(&task_key[..16]).join(&cache_key);
    fs::create_dir_all(&package_root)
        .map_err(|error| AppError::internal(format!("创建文档阅读缓存失败: {error}")))?;

    for file in &files {
        copy_cache_file(file, &package_root)?;
    }

    let html_name = source
        .html_path
        .file_name()
        .ok_or_else(|| AppError::internal("HTML 成书缺少文件名"))?;
    Ok(DocumentReaderPackage {
        cache_key,
        root_path: package_root.to_string_lossy().to_string(),
        html_path: package_root.join(html_name).to_string_lossy().to_string(),
    })
}

fn collect_reader_cache_files(
    source: &DocumentArtifactSource,
) -> Result<Vec<ReaderCacheFile>, AppError> {
    let html_name = source
        .html_path
        .file_name()
        .ok_or_else(|| AppError::internal("HTML 成书缺少文件名"))?;
    let mut files = vec![ReaderCacheFile {
        source_path: source.html_path.clone(),
        relative_path: PathBuf::from(html_name),
    }];
    let image_dir = source.artifact_dir.join(IMAGE_DIRECTORY);
    if image_dir.exists() {
        collect_directory_files(&image_dir, &source.artifact_dir, &mut files)?;
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn collect_directory_files(
    directory: &Path,
    managed_root: &Path,
    files: &mut Vec<ReaderCacheFile>,
) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| AppError::internal(format!("读取文档资源目录失败: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::invalid_input("文档资源目录类型不合法"));
    }

    let mut pending = vec![canonical_managed_path(directory, managed_root)?];
    while let Some(current) = pending.pop() {
        let mut entries = fs::read_dir(&current)
            .map_err(|error| AppError::internal(format!("枚举文档资源失败: {error}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::internal(format!("读取文档资源失败: {error}")))?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            collect_directory_entry(entry.path(), managed_root, &mut pending, files)?;
        }
    }
    Ok(())
}

fn collect_directory_entry(
    path: PathBuf,
    managed_root: &Path,
    pending: &mut Vec<PathBuf>,
    files: &mut Vec<ReaderCacheFile>,
) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| AppError::internal(format!("读取文档资源元数据失败: {error}")))?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::invalid_input("文档资源不能包含符号链接"));
    }
    if metadata.is_dir() {
        pending.push(canonical_managed_path(&path, managed_root)?);
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(AppError::invalid_input("文档资源包含不支持的文件类型"));
    }

    let source_path = canonical_managed_path(&path, managed_root)?;
    let relative_path = source_path
        .strip_prefix(managed_root)
        .map_err(|_| AppError::invalid_input("文档资源越过任务工件目录"))?
        .to_path_buf();
    files.push(ReaderCacheFile {
        source_path,
        relative_path,
    });
    Ok(())
}

fn canonical_managed_path(path: &Path, managed_root: &Path) -> Result<PathBuf, AppError> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| AppError::internal(format!("规范化文档资源路径失败: {error}")))?;
    if !canonical.starts_with(managed_root) {
        return Err(AppError::invalid_input("文档资源越过任务工件目录"));
    }
    Ok(canonical)
}

fn hash_reader_files(files: &[ReaderCacheFile]) -> Result<String, AppError> {
    let mut hasher = Sha256::new();
    for file in files {
        hash_framed_bytes(
            &mut hasher,
            file.relative_path.as_os_str().as_encoded_bytes(),
        )?;
        hasher.update(hash_file(&file.source_path)?);
    }
    Ok(digest_hex(hasher.finalize().as_slice()))
}

fn hash_framed_bytes(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), AppError> {
    let length =
        u64::try_from(bytes.len()).map_err(|_| AppError::internal("文档阅读资源路径过长"))?;
    hasher.update(length.to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn hash_file(path: &Path) -> Result<[u8; 32], AppError> {
    let file = fs::File::open(path)
        .map_err(|error| AppError::internal(format!("打开文档阅读资源失败: {error}")))?;
    let mut reader = BufReader::new(file);
    let mut buffer = [0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| AppError::internal(format!("读取文档阅读资源失败: {error}")))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let mut output = [0_u8; 32];
    output.copy_from_slice(&digest);
    Ok(output)
}

fn hash_text(value: &str) -> String {
    digest_hex(Sha256::digest(value.as_bytes()).as_slice())
}

fn digest_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn copy_cache_file(file: &ReaderCacheFile, package_root: &Path) -> Result<(), AppError> {
    let destination = package_root.join(&file.relative_path);
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::internal("文档阅读缓存路径缺少父目录"))?;
    fs::create_dir_all(parent)
        .map_err(|error| AppError::internal(format!("创建文档资源缓存目录失败: {error}")))?;
    fs::copy(&file.source_path, &destination)
        .map_err(|error| AppError::internal(format!("复制文档阅读资源失败: {error}")))?;
    Ok(())
}
