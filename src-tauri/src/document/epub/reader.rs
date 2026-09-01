use crate::error::AppError;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

mod cache;
mod package;
mod sanitize;
use super::path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubReaderPackage {
    pub cache_key: String,
    pub root_path: String,
    pub sections: Vec<EpubReaderSection>,
    pub has_bilingual_markup: bool,
    pub fixed_layout: bool,
    pub page_progression_direction: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubReaderSection {
    pub index: usize,
    pub href: String,
    pub file_path: String,
    pub title: String,
    /// 正文文本字符数，前端按此加权计算全书进度。
    pub text_length: usize,
    /// 是否被书籍目录（NCX/nav）收录：前端目录树仅展示收录条目，
    /// 未收录的连续子文件归并到前一目录条目之下。
    pub toc_covered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubSearchResult {
    pub href: String,
    pub title: String,
    pub snippet: String,
    /// 关键词高亮版 snippet（<mark class="mt-hit"> 包裹），前端直接渲染。
    pub snippet_html: String,
    pub occurrences: usize,
}

/// 带目录覆盖层的阅读包准备：overrides 为 `(归一化 href, 用户修订标题)` 列表，
/// 来自 toc_overrides.json（按原条目 href 索引）。叠加后阅读器目录（TocDrawer）与
/// 校对面板（目录管理）显示一致——此前阅读器只看 packed 成书的文件级标题解析
/// （NCX→nav→h1-h3→head-title→spine 兜底），用户在目录管理里的改名/合并对读者
/// 完全不可见（实测：改名后阅读器目录仍是旧标题，含英文兜底标题）。
pub fn prepare_epub_reader_package_with_overrides(
    epub_path: &Path,
    cache_root: &Path,
    overrides: &[(String, String)],
) -> Result<EpubReaderPackage, AppError> {
    let descriptor = package::read_package_descriptor(epub_path)?;
    let cache_entry = cache::ensure_reader_cache(epub_path, cache_root)?;
    let override_map: std::collections::HashMap<String, String> = overrides
        .iter()
        .map(|(href, title)| (path::normalize_href_key(href), title.clone()))
        .collect();
    let sections: Vec<package::PackageSection> = descriptor
        .sections
        .into_iter()
        .map(|mut section| {
            if let Some(title) = override_map.get(&path::normalize_href_key(&section.href)) {
                section.title = title.clone();
            }
            section
        })
        .collect();
    let sections = build_reader_sections(&cache_entry.root, sections)?;

    Ok(EpubReaderPackage {
        cache_key: cache_entry.key,
        root_path: cache_entry.root.to_string_lossy().to_string(),
        sections,
        has_bilingual_markup: descriptor.has_bilingual_markup,
        fixed_layout: descriptor.fixed_layout,
        page_progression_direction: descriptor.page_progression_direction,
    })
}

pub fn search_epub_reader(
    epub_path: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<EpubSearchResult>, AppError> {
    package::search_package(epub_path, query, limit)
}

fn build_reader_sections(
    cache_root: &Path,
    sections: Vec<package::PackageSection>,
) -> Result<Vec<EpubReaderSection>, AppError> {
    let mut output = Vec::with_capacity(sections.len());
    for section in sections {
        let file_path = cache_root.join(path_from_epub_href(&section.href)?);
        if !file_path.is_file() {
            return Err(AppError::not_found(format!(
                "EPUB 阅读章节不存在: {}",
                section.href
            )));
        }
        output.push(EpubReaderSection {
            index: section.index,
            href: section.href,
            file_path: file_path.to_string_lossy().to_string(),
            title: section.title,
            text_length: section.text_length,
            toc_covered: section.toc_covered,
        });
    }
    Ok(output)
}

fn path_from_epub_href(href: &str) -> Result<PathBuf, AppError> {
    let path_only = href.split('#').next().unwrap_or(href);
    let decoded = urlencoding::decode(path_only)
        .map_err(|error| AppError::invalid_input(format!("EPUB 章节路径编码无效: {error}")))?;
    let normalized = decoded.replace('\\', "/");
    let mut path = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir => {
                return Err(AppError::invalid_input("EPUB 章节路径越界"));
            }
            #[cfg(windows)]
            Component::Prefix(_) => {
                return Err(AppError::invalid_input("EPUB 章节路径包含盘符"));
            }
        }
    }
    if path.as_os_str().is_empty() {
        return Err(AppError::invalid_input("EPUB 章节路径为空"));
    }
    Ok(path)
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
