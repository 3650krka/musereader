use super::EpubSearchResult;
use crate::document::epub::archive::{open_epub_archive, read_zip_entry_to_string};
use crate::document::epub::nav::parse_nav_title_map;
use crate::document::epub::ncx::parse_ncx_title_map;
use crate::document::epub::opf::{locate_opf_path, parse_opf_manifest_and_spine};
use crate::document::epub::parse::resolve_chapter_title;
use crate::document::epub::path::{join_epub_path, parent_dir};
use crate::document::html::extract_text_from_html;
use crate::document::TocSourceKind;
use crate::error::AppError;
use regex::RegexBuilder;
use std::path::Path;

pub(super) struct PackageDescriptor {
    pub sections: Vec<PackageSection>,
    pub has_bilingual_markup: bool,
    pub fixed_layout: bool,
    pub page_progression_direction: Option<String>,
}

pub(super) struct PackageSection {
    pub index: usize,
    pub href: String,
    pub title: String,
    /// 正文文本字符数：全书进度按章节权重计算的权重依据（封面/广告页权重≈0）。
    pub text_length: usize,
    /// 该 section 是否被书籍目录（NCX/nav）收录。未收录的连续子文件
    /// （如 ch01_sub01，head-title 兜底标题）由前端归并到前一目录条目之下，
    /// 避免与章节并列成独立假目录项。 */
    pub toc_covered: bool,
}

pub(super) fn read_package_descriptor(epub_path: &Path) -> Result<PackageDescriptor, AppError> {
    let mut archive = open_epub_archive(epub_path)?;
    let opf_path = locate_opf_path(&mut archive)?;
    let opf_content = read_zip_entry_to_string(&mut archive, &opf_path)?;
    let (content_paths, _, nav_href, _) = parse_opf_manifest_and_spine(&opf_content)?;
    let title_map = parse_ncx_title_map(&mut archive, &opf_path).unwrap_or_default();
    let base_dir = parent_dir(&opf_path);
    let nav_path = nav_href.map(|href| join_epub_path(&base_dir, &href));
    let nav_title_map = parse_nav_title_map(&mut archive, nav_path.as_deref()).unwrap_or_default();
    let mut has_bilingual_markup = false;
    let mut sections = Vec::with_capacity(content_paths.len());

    for (index, relative_path) in content_paths.into_iter().enumerate() {
        let href = join_epub_path(&base_dir, &relative_path);
        let raw = read_zip_entry_to_string(&mut archive, &href)?;
        has_bilingual_markup |= contains_bilingual_markup(&raw);
        let (title, title_source) = resolve_chapter_title(
            &title_map,
            &nav_title_map,
            &href,
            &relative_path,
            &raw,
            index,
        );
        /* 目录收录判定：标题来自 NCX/nav 即视为目录条目；
           html-title/spine 兜底的连续子文件归并为所属章节的一部分。 */
        let toc_covered = matches!(title_source, TocSourceKind::EpubNcx | TocSourceKind::EpubNav);
        let text_length = extract_text_from_html(&raw)
            .map(|text| text.chars().count())
            .unwrap_or(raw.len());
        sections.push(PackageSection {
            index,
            href,
            title,
            text_length,
            toc_covered,
        });
    }
    if sections.is_empty() {
        return Err(AppError::invalid_input("EPUB 没有可阅读的 spine 章节"));
    }

    Ok(PackageDescriptor {
        sections,
        has_bilingual_markup,
        fixed_layout: opf_declares_fixed_layout(&opf_content),
        page_progression_direction: parse_page_progression_direction(&opf_content),
    })
}

pub(super) fn search_package(
    epub_path: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<EpubSearchResult>, AppError> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Ok(Vec::new());
    }
    let descriptor = read_package_descriptor(epub_path)?;
    let matcher = RegexBuilder::new(&regex::escape(trimmed_query))
        .case_insensitive(true)
        .unicode(true)
        .build()
        .map_err(|error| AppError::internal(format!("创建 EPUB 搜索表达式失败: {error}")))?;
    let mut archive = open_epub_archive(epub_path)?;
    let mut results = Vec::new();
    let result_limit = limit.clamp(1, 100);

    for section in descriptor.sections {
        let raw = read_zip_entry_to_string(&mut archive, &section.href)?;
        let text = extract_text_from_html(&raw)?;
        let Some(first_match) = matcher.find(&text) else {
            continue;
        };
        results.push(EpubSearchResult {
            href: section.href,
            title: section.title,
            snippet: search_snippet(&text, first_match.start(), first_match.end()),
            snippet_html: search_snippet_html(&text, first_match.start(), first_match.end()),
            occurrences: matcher.find_iter(&text).count(),
        });
        if results.len() >= result_limit {
            break;
        }
    }
    Ok(results)
}

fn contains_bilingual_markup(html: &str) -> bool {
    html.contains("mt-zh") && html.contains("mt-en")
}

fn opf_declares_fixed_layout(opf: &str) -> bool {
    let compact = opf.to_ascii_lowercase().replace(char::is_whitespace, "");
    compact.contains("property=\"rendition:layout\">pre-paginated")
        || compact.contains("property='rendition:layout'>pre-paginated")
}

fn parse_page_progression_direction(opf: &str) -> Option<String> {
    let matcher = RegexBuilder::new(
        r#"page-progression-direction\s*=\s*[\"'](?P<direction>ltr|rtl|default)[\"']"#,
    )
    .case_insensitive(true)
    .build()
    .ok()?;
    matcher
        .captures(opf)
        .and_then(|captures| captures.name("direction"))
        .map(|value| value.as_str().to_ascii_lowercase())
}

fn search_snippet(text: &str, start: usize, end: usize) -> String {
    let snippet_start = previous_char_boundary(text, start, 80);
    let snippet_end = next_char_boundary(text, end, 120);
    let mut snippet = text[snippet_start..snippet_end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if snippet_start > 0 {
        snippet.insert(0, '…');
    }
    if snippet_end < text.len() {
        snippet.push('…');
    }
    snippet
}

/// 高亮版 snippet：命中片段包 <mark class="mt-hit">（HTML 转义其余文本）。
fn search_snippet_html(text: &str, start: usize, end: usize) -> String {
    let snippet_start = previous_char_boundary(text, start, 80);
    let snippet_end = next_char_boundary(text, end, 120);
    let before = html_escape(&text[snippet_start..start]);
    let hit = html_escape(&text[start..end]);
    let after = html_escape(&text[end..snippet_end]);
    let mut out = String::new();
    if snippet_start > 0 {
        out.push('…');
    }
    out.push_str(&before);
    out.push_str("<mark class=\"mt-hit\">");
    out.push_str(&hit);
    out.push_str("</mark>");
    out.push_str(&after);
    if snippet_end < text.len() {
        out.push('…');
    }
    out
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn previous_char_boundary(text: &str, byte_index: usize, max_chars: usize) -> usize {
    text[..byte_index]
        .char_indices()
        .rev()
        .nth(max_chars)
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, byte_index: usize, max_chars: usize) -> usize {
    text[byte_index..]
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| byte_index + index)
        .unwrap_or(text.len())
}
