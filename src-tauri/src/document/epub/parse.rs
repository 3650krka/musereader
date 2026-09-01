use crate::document::epub::archive::read_zip_entry_to_string;
use crate::document::epub::path::{join_epub_path, normalize_href_key, parent_dir};
use crate::document::epub::EpubChapter;
use crate::document::html::{extract_html_head_title, extract_html_title, extract_text_from_html};
use crate::document::EpubBlock;
use crate::document::TocSourceKind;
use crate::error::AppError;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::process::{Command, Stdio};
use zip::ZipArchive;

pub(super) fn extract_spine_chapters(
    archive: &mut ZipArchive<File>,
    base_dir: &str,
    content_paths: Vec<String>,
    title_map: &HashMap<String, String>,
    nav_title_map: &HashMap<String, String>,
) -> Result<Vec<EpubChapter>, AppError> {
    let mut chapters = Vec::new();
    for (order, relative_path) in content_paths.into_iter().enumerate() {
        let archive_path = join_epub_path(base_dir, &relative_path);
        if let Some(chapter) = extract_spine_chapter(
            archive,
            &relative_path,
            &archive_path,
            order,
            title_map,
            nav_title_map,
        )? {
            chapters.push(chapter);
        }
    }
    Ok(chapters)
}

pub(super) fn extract_spine_chapter(
    archive: &mut ZipArchive<File>,
    relative_path: &str,
    archive_path: &str,
    order: usize,
    title_map: &HashMap<String, String>,
    nav_title_map: &HashMap<String, String>,
) -> Result<Option<EpubChapter>, AppError> {
    let raw = read_zip_entry_to_string(archive, archive_path)?;
    let text = convert_epub_html_to_markdown(&raw, relative_path)?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let (title, title_source) = resolve_chapter_title(
        title_map,
        nav_title_map,
        archive_path,
        relative_path,
        &raw,
        order,
    );
    let blocks = extract_epub_blocks(&raw, &text, order, &title, Some(relative_path.to_string()))?;
    Ok(Some(EpubChapter {
        title,
        markdown: text,
        order,
        href: Some(relative_path.to_string()),
        title_source,
        blocks,
    }))
}

pub(super) fn resolve_chapter_title(
    title_map: &HashMap<String, String>,
    nav_title_map: &HashMap<String, String>,
    archive_path: &str,
    relative_path: &str,
    raw_html: &str,
    order: usize,
) -> (String, TocSourceKind) {
    if let Some(title) = title_map
        .get(&normalize_href_key(relative_path))
        .or_else(|| title_map.get(&normalize_href_key(archive_path)))
        .cloned()
    {
        return (title, TocSourceKind::EpubNcx);
    }
    if let Some(title) = nav_title_map
        .get(&normalize_href_key(archive_path))
        .or_else(|| nav_title_map.get(&normalize_href_key(relative_path)))
        .cloned()
    {
        return (title, TocSourceKind::EpubNav);
    }
    if let Some(title) = extract_html_title(raw_html) {
        return (title, TocSourceKind::EpubHtmlTitle);
    }
    // 无 h1-h3 的章节（插页/新闻剪报等）：退回 <head><title>（如 "1. Tash"），
    // 仍优于 spine 序号兜底 "Chapter N"（会造成同书重复的假标题）。
    if let Some(title) = extract_html_head_title(raw_html) {
        return (title, TocSourceKind::EpubHtmlTitle);
    }
    (
        format!("Chapter {}", order + 1),
        TocSourceKind::EpubSpineFallback,
    )
}

fn convert_epub_html_to_markdown(raw_html: &str, relative_path: &str) -> Result<String, AppError> {
    // Boilerplate 剥离（借鉴 boilerplate-filter）：网页抓取的 EPUB 常含分享栏/
    // 相关文章/页脚导航等结构性残渣，翻译它们是纯 token 浪费且污染译文。
    // 结构性标记（class/id token + 语义标签 + hidden 属性）判定，不看可见文本；
    // epub:type=toc/landmarks/page-list 的 nav 保护不动。MUSETRANSLATE_STRIP_BOILERPLATE=0 关。
    let stripped_html;
    let raw_html = if crate::pipeline::env_flag_disabled("MUSETRANSLATE_STRIP_BOILERPLATE") {
        raw_html
    } else {
        stripped_html = strip_web_boilerplate(raw_html);
        stripped_html.as_str()
    };
    if should_use_internal_epub_converter() {
        return extract_text_from_html(raw_html);
    }
    match pandoc_html_to_markdown(raw_html) {
        Ok(markdown) if !markdown.trim().is_empty() => Ok(rewrite_epub_markdown_image_paths(
            &normalize_pandoc_markdown(&markdown),
            relative_path,
        )),
        _ => extract_text_from_html(raw_html),
    }
}

fn should_use_internal_epub_converter() -> bool {
    std::env::var("MUSETRANSLATE_EPUB_CONVERTER")
        .ok()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("internal"))
}

fn pandoc_html_to_markdown(raw_html: &str) -> Result<String, AppError> {
    let pandoc = std::env::var("MUSETRANSLATE_PANDOC_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "pandoc".to_string());
    let normalized_html = normalize_epub_html_for_pandoc(raw_html);
    let mut child = Command::new(pandoc)
        .args([
            "--from=html",
            "--to=gfm-raw_html+footnotes",
            "--wrap=none",
            "--markdown-headings=atx",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| AppError::internal(format!("start pandoc failed: {error}")))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(normalized_html.as_bytes())
            .map_err(|error| AppError::internal(format!("write pandoc stdin failed: {error}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| AppError::internal(format!("wait pandoc failed: {error}")))?;
    if !output.status.success() {
        return Err(AppError::internal(format!(
            "pandoc exited with status {}; stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| AppError::internal(format!("pandoc stdout was not UTF-8: {error}")))
}

fn normalize_epub_html_for_pandoc(raw_html: &str) -> String {
    unwrap_svg_image_wrappers(raw_html)
}

/// 网页残渣剥离（结构性标记驱动，不看可见文本——保守不删正文）。
/// 返回清理后的 HTML。判定三类：
/// 1. 语义标签 footer/nav（nav 若有 epub:type=toc/landmarks/page-list 或 role=doc-toc 则保护）；
/// 2. class/id 含 boilerplate token（分享/相关文章/页脚导航/评论/屏幕阅读器）；
/// 3. hidden / aria-hidden="true" / display:none / visibility:hidden。
pub(super) fn strip_web_boilerplate_pub(html: &str) -> String {
    strip_web_boilerplate(html)
}

fn strip_web_boilerplate(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    // 逐标签扫描，命中起始标签时按深度跳过到对应闭标签。
    while let Some(start) = find_next_tag(rest) {
        out.push_str(&rest[..start]);
        let tag = &rest[start..];
        let Some((name, attrs, tag_end, self_closing)) = parse_tag(tag) else {
            out.push_str(&rest[start..start + 1]);
            rest = &rest[start + 1..];
            continue;
        };
        if is_boilerplate_element(&name, &attrs) {
            if self_closing {
                rest = &tag[tag_end..];
                continue;
            }
            // 跳过到匹配闭标签（考虑同名嵌套深度）。
            let after_open = &tag[tag_end..];
            let skip = skip_to_close(after_open, &name);
            rest = &after_open[skip..];
            continue;
        }
        out.push_str(&tag[..tag_end]);
        rest = &tag[tag_end..];
    }
    out.push_str(rest);
    out
}

fn find_next_tag(html: &str) -> Option<usize> {
    html.find('<')
}

/// 解析一个标签：返回 (小写标签名, 属性字符串, 标签结束偏移, 是否自闭合)。
/// 无法解析（</、<!--、<!DOCTYPE、<?）返回 None。
fn parse_tag(tag: &str) -> Option<(String, String, usize, bool)> {
    let bytes = tag.as_bytes();
    if bytes.len() < 3 || bytes[1] == b'/' || bytes[1] == b'!' || bytes[1] == b'?' {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric()) {
        end += 1;
    }
    let name = tag[1..end].to_ascii_lowercase();
    // 找 >（忽略引号内的 >）
    let mut in_quote: Option<u8> = None;
    let mut i = end;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            in_quote = Some(b);
        } else if b == b'>' {
            let inner = &tag[end..i];
            let self_closing = inner.trim_end().ends_with('/');
            return Some((name, inner.to_string(), i + 1, self_closing));
        }
        i += 1;
    }
    None
}

/// 从开标签后跳到匹配闭标签之后，返回跳过字节数。同名嵌套用深度计数。
fn skip_to_close(html: &str, name: &str) -> usize {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut depth = 1usize;
    let mut cursor = 0usize;
    while cursor < html.len() {
        let Some(pos) = html[cursor..].find('<') else {
            return html.len();
        };
        let abs = cursor + pos;
        let slice = &html[abs..];
        if slice.starts_with(&close) {
            // 闭标签到 > 为止
            if let Some(end) = slice.find('>') {
                depth -= 1;
                if depth == 0 {
                    return abs + end + 1;
                }
                cursor = abs + end + 1;
                continue;
            }
            return html.len();
        }
        if slice.starts_with(&open) {
            // 开标签到 > 为止（自闭合不计深度）
            if let Some((_, _, tag_end, self_closing)) = parse_tag(slice) {
                if !self_closing {
                    depth += 1;
                }
                cursor = abs + tag_end;
                continue;
            }
        }
        cursor = abs + 1;
    }
    html.len()
}

/// 语义标签 + class/id token + hidden 属性三类判定。
fn is_boilerplate_element(name: &str, attrs: &str) -> bool {
    match name {
        "footer" => return true,
        "nav" => {
            // 保护 toc/landmarks/page-list/aria doc-toc
            let protected = attrs.contains("epub:type=\"toc")
                || attrs.contains("epub:type='toc")
                || attrs.contains("landmarks")
                || attrs.contains("page-list")
                || attrs.contains("doc-toc");
            return !protected;
        }
        "script" | "style" => return true,
        _ => {}
    }
    let lower = attrs.to_ascii_lowercase();
    if lower.contains("hidden")
        || lower.contains("aria-hidden=\"true\"")
        || lower.contains("display:none")
        || lower.contains("display: none")
        || lower.contains("visibility:hidden")
        || lower.contains("visibility: hidden")
    {
        return true;
    }
    // class/id token 匹配（锚定 token 边界，防 "shared" 误伤 "sharedaddy" 之类）
    const TOKENS: &[&str] = &[
        "sharedaddy",
        "sd-sharing",
        "sd-social",
        "sd-block",
        "jp-sharing",
        "jp-relatedposts",
        "jp-post-flair",
        "share-buttons",
        "share-button",
        "sharebox",
        "share-bar",
        "social-share",
        "social-links",
        "social-media",
        "social-icons",
        "addtoany",
        "addthis",
        "sharethis",
        "related-posts",
        "related-post",
        "related-articles",
        "yarpp",
        "post-navigation",
        "post-nav",
        "nav-links",
        "nav-previous",
        "nav-next",
        "pagination",
        "page-links",
        "prev-next",
        "post-pagination",
        "comment-respond",
        "comments-area",
        "comment-area",
        "comments-list",
        "comment-list",
        "screen-reader-text",
        "sr-only",
        "visually-hidden",
        "a11y-hidden",
    ];
    for attr_name in ["class=\"", "class='", "id=\"", id_prefix_single()] {
        let mut search_from = 0usize;
        while let Some(pos) = lower[search_from..].find(attr_name) {
            let abs = search_from + pos + attr_name.len();
            let quote = attr_name.chars().last().unwrap_or('"');
            if let Some(end) = lower[abs..].find(quote) {
                let value = &lower[abs..abs + end];
                for token in value.split(|c: char| c.is_whitespace() || c == ';') {
                    if TOKENS.contains(&token) {
                        return true;
                    }
                }
                search_from = abs + end;
            } else {
                break;
            }
        }
    }
    false
}

fn id_prefix_single() -> &'static str {
    "id='"
}

fn unwrap_svg_image_wrappers(raw_html: &str) -> String {
    let Ok(svg_re) = regex::Regex::new(
        r#"(?is)<svg\b[^>]*>\s*<image\b(?P<attrs>[^>]*)\s*/>\s*</svg>|<svg\b[^>]*>\s*<image\b(?P<attrs2>[^>]*)>\s*</image>\s*</svg>"#,
    ) else {
        return raw_html.to_string();
    };
    svg_re
        .replace_all(raw_html, |captures: &regex::Captures| {
            let attrs = captures
                .name("attrs")
                .or_else(|| captures.name("attrs2"))
                .map(|item| item.as_str())
                .unwrap_or("");
            image_href_from_attrs(attrs)
                .map(|href| format!(r#"<img src="{href}" />"#))
                .unwrap_or_else(|| captures[0].to_string())
        })
        .to_string()
}

fn image_href_from_attrs(attrs: &str) -> Option<String> {
    let href_re = regex::Regex::new(r#"(?i)(?:xlink:href|href)\s*=\s*["']([^"']+)["']"#).ok()?;
    href_re
        .captures(attrs)
        .and_then(|captures| captures.get(1))
        .map(|href| href.as_str().to_string())
}

fn normalize_pandoc_markdown(markdown: &str) -> String {
    let mut normalized = Vec::new();
    let mut previous_blank = false;
    for line in markdown.lines().map(str::trim_end) {
        let is_blank = line.trim().is_empty();
        if is_blank && previous_blank {
            continue;
        }
        normalized.push(line);
        previous_blank = is_blank;
    }
    normalized.join("\n").trim().to_string()
}

fn rewrite_epub_markdown_image_paths(markdown: &str, chapter_relative_path: &str) -> String {
    let Ok(image_re) = regex::Regex::new(r#"!\[(?P<alt>[^\]]*)\]\((?P<url>[^)]+)\)"#) else {
        return markdown.to_string();
    };
    let chapter_dir = parent_dir(chapter_relative_path);
    image_re
        .replace_all(markdown, |captures: &regex::Captures| {
            let alt = captures.name("alt").map(|item| item.as_str()).unwrap_or("");
            let url = captures.name("url").map(|item| item.as_str()).unwrap_or("");
            if should_preserve_markdown_image_url(url) {
                return captures[0].to_string();
            }
            let resolved = join_epub_path(&chapter_dir, url);
            let artifact_path = crate::document::epub::artifact_image_path(&resolved);
            format!("![{alt}]({artifact_path})")
        })
        .to_string()
}

fn should_preserve_markdown_image_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("data:")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("imgs/")
}

fn extract_epub_blocks(
    raw_html: &str,
    markdown: &str,
    chapter_order: usize,
    chapter_title: &str,
    href: Option<String>,
) -> Result<Vec<EpubBlock>, AppError> {
    let markdown_blocks =
        extract_epub_blocks_from_markdown(markdown, chapter_order, chapter_title, href.clone());
    if !markdown_blocks.is_empty() {
        return Ok(markdown_blocks);
    }
    extract_epub_blocks_from_html(raw_html, chapter_order, chapter_title, href)
}

fn extract_epub_blocks_from_markdown(
    markdown: &str,
    chapter_order: usize,
    chapter_title: &str,
    href: Option<String>,
) -> Vec<EpubBlock> {
    let mut blocks = markdown_block_sources(markdown)
        .into_iter()
        .enumerate()
        .filter_map(|(order, source_markdown)| {
            let text = markdown_block_text(&source_markdown);
            let kind = markdown_block_kind(&source_markdown);
            if text.trim().is_empty() && kind != "image" {
                return None;
            }
            Some(EpubBlock {
                id: format!("ch-{chapter_order:03}-b-{order:04}"),
                chapter_order,
                chapter_title: chapter_title.to_string(),
                href: href.clone(),
                order,
                kind: kind.to_string(),
                source_html: String::new(),
                source_markdown,
                text: text.trim().to_string(),
                translate: should_translate_markdown_block(kind, &text),
            })
        })
        .collect::<Vec<_>>();
    if should_inject_chapter_heading(markdown, chapter_title) {
        blocks.insert(
            0,
            EpubBlock {
                id: String::new(),
                chapter_order,
                chapter_title: chapter_title.to_string(),
                href: href.clone(),
                order: 0,
                kind: "heading".to_string(),
                source_html: String::new(),
                source_markdown: format!("# {}", chapter_title.trim()),
                text: chapter_title.trim().to_string(),
                translate: true,
            },
        );
        reindex_epub_blocks(&mut blocks, chapter_order);
    } else {
        promote_first_plain_title_block(&mut blocks, chapter_title);
    }
    blocks
}

fn should_inject_chapter_heading(markdown: &str, chapter_title: &str) -> bool {
    let title = chapter_title.trim();
    if title.is_empty() {
        return false;
    }
    let Some(first_line) = markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return false;
    };
    let normalized_line = first_line.trim_start_matches('#').trim();
    !normalized_line.eq_ignore_ascii_case(title)
}

fn reindex_epub_blocks(blocks: &mut [EpubBlock], chapter_order: usize) {
    for (order, block) in blocks.iter_mut().enumerate() {
        block.order = order;
        block.id = format!("ch-{chapter_order:03}-b-{order:04}");
    }
}

fn promote_first_plain_title_block(blocks: &mut [EpubBlock], chapter_title: &str) {
    let title = chapter_title.trim();
    if title.is_empty() {
        return;
    }
    let Some(first) = blocks.first_mut() else {
        return;
    };
    if first.kind == "paragraph" && first.text.trim().eq_ignore_ascii_case(title) {
        first.kind = "heading".to_string();
        first.source_markdown = format!("# {title}");
    }
}

fn markdown_block_sources(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in markdown.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n").trim().to_string());
                current.clear();
            }
            continue;
        }
        current.push(line.to_string());
    }
    if !current.is_empty() {
        blocks.push(current.join("\n").trim().to_string());
    }
    blocks
}

fn markdown_block_kind(source: &str) -> &'static str {
    let first_line = source.lines().next().unwrap_or("").trim_start();
    if first_line.starts_with('#') {
        return "heading";
    }
    if first_line.starts_with("![") {
        return "image";
    }
    if first_line.starts_with('>') {
        return "blockquote";
    }
    if first_line.starts_with("- ")
        || first_line.starts_with("* ")
        || ordered_list_start(first_line)
    {
        return "listItem";
    }
    if markdown_table_like(source) {
        return "table";
    }
    "paragraph"
}

fn ordered_list_start(line: &str) -> bool {
    let Some((number, rest)) = line.split_once('.') else {
        return false;
    };
    !number.is_empty()
        && number.chars().all(|character| character.is_ascii_digit())
        && rest.starts_with(' ')
}

fn markdown_table_like(source: &str) -> bool {
    let mut lines = source.lines();
    let Some(header) = lines.next() else {
        return false;
    };
    let Some(separator) = lines.next() else {
        return false;
    };
    header.contains('|')
        && separator.contains('|')
        && separator
            .chars()
            .all(|character| matches!(character, '|' | '-' | ':' | ' '))
}

fn markdown_block_text(source: &str) -> String {
    if markdown_block_kind(source) == "image" {
        return markdown_image_alt_text(source);
    }
    source
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches('#')
                .trim_start_matches('>')
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown_image_alt_text(source: &str) -> String {
    let Some(start) = source.find("![") else {
        return String::new();
    };
    let Some(end) = source[start + 2..].find(']') else {
        return String::new();
    };
    source[start + 2..start + 2 + end].trim().to_string()
}

fn should_translate_markdown_block(kind: &str, text: &str) -> bool {
    !matches!(kind, "image" | "table") && !text.trim().is_empty()
}

fn extract_epub_blocks_from_html(
    raw_html: &str,
    chapter_order: usize,
    chapter_title: &str,
    href: Option<String>,
) -> Result<Vec<EpubBlock>, AppError> {
    let block_re = regex::Regex::new(
        r#"(?is)<(h[1-6]|p|blockquote|li|pre|figcaption|caption|dt|dd|td|th|table|figure)\b[^>]*>.*?</(?:h[1-6]|p|blockquote|li|pre|figcaption|caption|dt|dd|td|th|table|figure)>"#,
    )
    .map_err(|error| AppError::internal(format!("compile EPUB block regex failed: {error}")))?;
    let mut blocks = Vec::new();
    for (order, captures) in block_re.captures_iter(raw_html).enumerate() {
        let Some(source_html) = captures.get(0).map(|item| item.as_str().trim().to_string()) else {
            continue;
        };
        let tag = captures
            .get(1)
            .map(|item| item.as_str().to_ascii_lowercase())
            .unwrap_or_else(|| "block".to_string());
        let text = extract_text_from_html(&source_html)?;
        if text.trim().is_empty() && !source_html.to_ascii_lowercase().contains("<img") {
            continue;
        }
        blocks.push(EpubBlock {
            id: format!("ch-{chapter_order:03}-b-{order:04}"),
            chapter_order,
            chapter_title: chapter_title.to_string(),
            href: href.clone(),
            order,
            kind: epub_block_kind(&tag, &source_html).to_string(),
            source_html,
            source_markdown: text.clone(),
            text: text.trim().to_string(),
            translate: should_translate_epub_block(&tag, &text),
        });
    }
    if blocks.is_empty() {
        let text = extract_text_from_html(raw_html)?;
        if !text.trim().is_empty() {
            blocks.push(EpubBlock {
                id: format!("ch-{chapter_order:03}-b-0000"),
                chapter_order,
                chapter_title: chapter_title.to_string(),
                href,
                order: 0,
                kind: "chapterText".to_string(),
                source_html: raw_html.to_string(),
                source_markdown: text.clone(),
                text: text.trim().to_string(),
                translate: true,
            });
        }
    }
    Ok(blocks)
}

fn epub_block_kind<'a>(tag: &'a str, source_html: &str) -> &'a str {
    if tag.starts_with('h') && tag.len() == 2 {
        return "heading";
    }
    match tag {
        "blockquote" => "blockquote",
        "li" => "listItem",
        "pre" => "preformatted",
        "figcaption" | "caption" => "caption",
        "table" | "td" | "th" => "table",
        "figure" => "figure",
        _ if source_html.to_ascii_lowercase().contains("<img") => "image",
        _ => "paragraph",
    }
}

fn should_translate_epub_block(tag: &str, text: &str) -> bool {
    !matches!(tag, "table" | "figure") && !text.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_boilerplate_removes_footer_and_share_widgets() {
        let html = r#"<html><body><p>Real content.</p><footer>Copyright footer</footer><div class="sharedaddy">Share this</div><p>More content.</p></body></html>"#;
        let out = strip_web_boilerplate(html);
        assert!(out.contains("Real content."));
        assert!(out.contains("More content."));
        assert!(!out.contains("Copyright footer"));
        assert!(!out.contains("Share this"));
    }

    #[test]
    fn strip_boilerplate_protects_toc_nav() {
        let html = r#"<html><body><nav epub:type="toc"><ol><li>Ch1</li></ol></nav><nav><a>Next</a></nav></body></html>"#;
        let out = strip_web_boilerplate(html);
        assert!(out.contains("Ch1"), "toc nav protected: {out}");
        assert!(!out.contains("Next"), "plain nav stripped: {out}");
    }

    #[test]
    fn strip_boilerplate_removes_hidden_and_related_posts() {
        let html = r#"<body><p>Keep.</p><div id="related-posts"><p>Related</p></div><span style="display:none">Hidden</span></body>"#;
        let out = strip_web_boilerplate(html);
        assert!(out.contains("Keep."));
        assert!(!out.contains("Related"));
        assert!(!out.contains("Hidden"));
    }

    #[test]
    fn strip_boilerplate_keeps_normal_content_with_partial_token_match() {
        // "shared" 不是 token "sharedaddy"；保守不删
        let html = r#"<body><div class="shared-resource">Keep this.</div></body>"#;
        let out = strip_web_boilerplate(html);
        assert!(out.contains("Keep this."));
    }

    #[test]
    fn strip_boilerplate_handles_nested_same_tag() {
        let html = r#"<body><div class="sharethis"><div>nested <div>deep</div></div></div><p>After.</p></body>"#;
        let out = strip_web_boilerplate(html);
        assert!(!out.contains("nested"));
        assert!(out.contains("After."));
    }

    #[test]
    fn chapter_title_prefers_ncx_over_nav() {
        let title_map = HashMap::from([("chapter.xhtml".to_string(), "NCX Title".to_string())]);
        let nav_title_map = HashMap::from([("chapter.xhtml".to_string(), "Nav Title".to_string())]);

        let (title, source) = resolve_chapter_title(
            &title_map,
            &nav_title_map,
            "OEBPS/chapter.xhtml",
            "chapter.xhtml",
            "<h1>HTML Title</h1>",
            0,
        );

        assert_eq!(title, "NCX Title");
        assert_eq!(source, TocSourceKind::EpubNcx);
    }

    #[test]
    fn chapter_title_uses_nav_before_html_heading() {
        let title_map = HashMap::new();
        let nav_title_map =
            HashMap::from([("OEBPS/chapter.xhtml".to_string(), "Nav Title".to_string())]);

        let (title, source) = resolve_chapter_title(
            &title_map,
            &nav_title_map,
            "OEBPS/chapter.xhtml",
            "chapter.xhtml#top",
            "<h1>HTML Title</h1>",
            0,
        );

        assert_eq!(title, "Nav Title");
        assert_eq!(source, TocSourceKind::EpubNav);
    }

    #[test]
    fn chapter_title_matches_legacy_relative_ncx_key() {
        let title_map = HashMap::from([("chapter.xhtml".to_string(), "NCX Title".to_string())]);
        let nav_title_map = HashMap::new();

        let (title, source) = resolve_chapter_title(
            &title_map,
            &nav_title_map,
            "OEBPS/chapter.xhtml",
            "chapter.xhtml",
            "<h1>HTML Title</h1>",
            0,
        );

        assert_eq!(title, "NCX Title");
        assert_eq!(source, TocSourceKind::EpubNcx);
    }

    #[test]
    fn chapter_title_falls_back_to_head_title_without_headings() {
        // 出版书插页/新闻剪报等无 h1-h3 的章节：<head><title> 优于 spine 兜底
        let (title, source) = resolve_chapter_title(
            &HashMap::new(),
            &HashMap::new(),
            "OEBPS/chapter5.xhtml",
            "chapter5.xhtml",
            r#"<html><head><title>1. Tash</title></head><body><p>Text</p></body></html>"#,
            4,
        );

        assert_eq!(title, "1. Tash");
        assert_eq!(source, TocSourceKind::EpubHtmlTitle);
    }

    #[test]
    fn chapter_title_prefers_heading_over_head_title() {
        let (title, source) = resolve_chapter_title(
            &HashMap::new(),
            &HashMap::new(),
            "OEBPS/chapter.xhtml",
            "chapter.xhtml",
            r#"<html><head><title>Editor Title</title></head><body><h2>Body Heading</h2></body></html>"#,
            0,
        );

        assert_eq!(title, "Body Heading");
        assert_eq!(source, TocSourceKind::EpubHtmlTitle);
    }

    #[test]
    fn chapter_title_spine_fallback_when_no_title_sources() {
        let (title, source) = resolve_chapter_title(
            &HashMap::new(),
            &HashMap::new(),
            "OEBPS/blank.xhtml",
            "blank.xhtml",
            r#"<html><head><title></title></head><body><p>Plain</p></body></html>"#,
            6,
        );

        assert_eq!(title, "Chapter 7");
        assert_eq!(source, TocSourceKind::EpubSpineFallback);
    }

    #[test]
    fn head_title_decodes_entities_and_collapses_whitespace() {
        // head title 里的实体与多余空白需归一，否则目录条目会出现 "&amp;" 或双空格
        let (title, _) = resolve_chapter_title(
            &HashMap::new(),
            &HashMap::new(),
            "OEBPS/c.xhtml",
            "c.xhtml",
            r#"<html><head><title>  Crime &amp;   Punishment </title></head><body><p>x</p></body></html>"#,
            0,
        );

        assert_eq!(title, "Crime & Punishment");
    }

    #[test]
    fn normalizes_repeated_blank_lines_from_pandoc() {
        let markdown = "Title\n\n\n\nParagraph\n\n\nNext";

        assert_eq!(
            normalize_pandoc_markdown(markdown),
            "Title\n\nParagraph\n\nNext"
        );
    }

    #[test]
    fn unwraps_svg_image_wrapper_before_pandoc() {
        let html = r#"<svg viewBox="0 0 1 1"><image width="1" xlink:href="../Images/cover.jpeg"></image></svg>"#;

        assert_eq!(
            normalize_epub_html_for_pandoc(html),
            r#"<img src="../Images/cover.jpeg" />"#
        );
    }

    #[test]
    fn unwraps_self_closing_svg_image_wrapper_before_pandoc() {
        let html =
            r#"<svg viewBox="0 0 1 1"><image width="1" xlink:href="../Images/cover.jpeg"/></svg>"#;

        assert_eq!(
            normalize_epub_html_for_pandoc(html),
            r#"<img src="../Images/cover.jpeg" />"#
        );
    }

    #[test]
    fn rewrites_epub_markdown_image_paths_to_artifact_paths() {
        let markdown = "![Cover](../Images/cover.jpeg)\n\nText";

        assert_eq!(
            rewrite_epub_markdown_image_paths(markdown, "Text/titlepage.xhtml"),
            "![Cover](imgs/epub/Images/cover.jpeg)\n\nText"
        );
    }

    #[test]
    fn extracts_structured_blocks_from_markdown() {
        let blocks = extract_epub_blocks(
            "<html></html>",
            "# Chapter\n\n![Cover](cover.jpg)\n\nA paragraph.\n\n> Quoted text",
            2,
            "Chapter",
            Some("chapter.xhtml".to_string()),
        )
        .expect("extract EPUB blocks");

        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].kind, "heading");
        assert_eq!(blocks[1].kind, "image");
        assert!(!blocks[1].translate);
        assert_eq!(blocks[2].kind, "paragraph");
        assert!(blocks[2].translate);
        assert_eq!(blocks[3].kind, "blockquote");
    }

    #[test]
    fn injects_chapter_heading_into_blocks_when_missing_from_markdown() {
        let blocks = extract_epub_blocks(
            "<html></html>",
            "Opening paragraph.",
            4,
            "Chapter Title",
            Some("chapter.xhtml".to_string()),
        )
        .expect("extract EPUB blocks");

        assert_eq!(blocks[0].id, "ch-004-b-0000");
        assert_eq!(blocks[0].kind, "heading");
        assert_eq!(blocks[0].text, "Chapter Title");
        assert_eq!(blocks[1].id, "ch-004-b-0001");
        assert_eq!(blocks[1].kind, "paragraph");
    }

    #[test]
    fn promotes_plain_opening_title_block_to_heading() {
        let blocks = extract_epub_blocks(
            "<html></html>",
            "Chapter Title\n\nOpening paragraph.",
            4,
            "Chapter Title",
            Some("chapter.xhtml".to_string()),
        )
        .expect("extract EPUB blocks");

        assert_eq!(blocks[0].id, "ch-004-b-0000");
        assert_eq!(blocks[0].kind, "heading");
        assert_eq!(blocks[0].source_markdown, "# Chapter Title");
        assert_eq!(blocks[1].kind, "paragraph");
    }
}
