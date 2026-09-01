// EPUB 打包器：把翻译结果写回 EPUB，产出中文版与双语对照版两个独立文件。

// 复用读取侧的 OPF/spine 解析与图片路径约定。核心策略：
//   - 正文：对**未翻章节**（front matter、封面、版权、目录等）**原样保留**；对已翻章节用
//     对齐块重建（中文版替换段落、双语版左中右英双列），并尽量保留原 xhtml 的
//     结构骨架与排版 class（首行缩进/日期行/章节标题样式等）。
//   - 目录：nav.xhtml / toc.ncx 的标题用 translated_toc.json 的中文标题原位替换。
//   - 非正文条目（CSS/字体/图片/OPF/manifest）原样复制，仅注入一份自定义样式表。

// 数据对齐依据：translated_blocks.json 的 sourceText+translatedText 双非空对齐。

use crate::document::{TocEntry, TranslatedBlock};
use crate::error::AppError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// 打包模式：中文版（仅译文）或双语对照（左中右英）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpubPackMode {
    Chinese,
    Bilingual,
}

/// 一组待回写的源→译 block 对齐数据（按出现顺序对应原 xhtml 段落序列）。
/// （内容锚定打包按块实际位置归章，无需章节对象。）
pub struct EpubPackChapter {
    pub translated: Vec<TranslatedBlock>,
}

/// 内容锚定打包：write_epub 忽略调用方传入的 chapters 归章（可能错位），
/// 改为收集全部块后用「源文本在 spine 文本流中的实际位置」重新归章。
/// 这保证即使管线的 chapterTitle 标注与 spine 文件边界不一致（如章标题被重写成
/// 「人物名+内联图」），块也能落到正文真正所在的 xhtml，不会整章错塞。
pub fn write_epub(
    source_epub: &Path,
    output_epub: &Path,
    mode: EpubPackMode,
    chapters: &[EpubPackChapter],
    book_title: &str,
    toc: &[TocEntry],
) -> Result<PackReport, AppError> {
    let mut archive = open_archive(source_epub)?;
    let opf_path = locate_opf_path(&mut archive)?;
    let base_dir = parent_dir(&opf_path);
    let opf_content = read_entry_to_string(&mut archive, &opf_path)?;
    let (content_paths, _cover_href, nav_href, _images) =
        parse_opf_manifest_and_spine(&opf_content)?;

    // 1) 收集全部译文块（保序），并预计算归一化文本（pack 全程复用，避免重复 normalize）。
    let all_blocks: Vec<&TranslatedBlock> =
        chapters.iter().flat_map(|c| c.translated.iter()).collect();
    let block_norms: Vec<String> = all_blocks
        .iter()
        .map(|tb| normalize_stream_text(&tb.source_text))
        .collect();

    // 2) 构建 spine 归一化文本流 + 每文件字符区间，把每块锚定到实际 xhtml。
    let spine_files: Vec<String> = content_paths
        .iter()
        .map(|href| join_epub_path(&base_dir, href))
        .collect();
    let placement = place_blocks_by_content(&mut archive, &spine_files, &all_blocks, &block_norms)?;

    // 3) 按文件分组块（保序），文件级重建。
    let stylesheet_href = "musetranslate-style.css";
    let stylesheet_archive_path = join_epub_path(&base_dir, stylesheet_href);
    let mut replacements: HashMap<String, String> = HashMap::new();
    let mut aligned_blocks_total = 0usize;

    for (file_idx, file_blocks) in &placement.blocks_by_file {
        if file_blocks.is_empty() {
            continue;
        }
        let archive_path = &spine_files[*file_idx];
        let raw = read_entry_to_string(&mut archive, archive_path)?;
        let chapter_dir = parent_dir(archive_path);
        let (xhtml, aligned, _) =
            rebuild_chapter_xhtml(&raw, file_blocks, mode, stylesheet_href, &chapter_dir)?;
        aligned_blocks_total += aligned;
        replacements.insert(archive_path.clone(), xhtml);
    }

    // 目录翻译：nav.xhtml / toc.ncx 的标题原位替换为中文。
    let toc_replacements =
        build_toc_replacements(&mut archive, &base_dir, nav_href.as_deref(), toc)?;
    for (path, content) in toc_replacements {
        replacements.insert(path, content);
    }

    // OPF：注册样式表 + 更新标题。
    let opf_replacement = rewrite_opf(&opf_content, stylesheet_href, book_title, mode);
    replacements.insert(opf_path.clone(), opf_replacement);

    write_output_epub(
        &mut archive,
        output_epub,
        &replacements,
        &stylesheet_archive_path,
        stylesheet_for_mode(mode),
    )?;

    Ok(PackReport {
        chapters: placement.blocks_by_file.len(),
        spine_content_paths: content_paths.len(),
        aligned_blocks: aligned_blocks_total,
        unplaced_blocks: placement.unplaced_blocks,
    })
}

#[derive(Debug, Clone)]
pub struct PackReport {
    pub chapters: usize,
    pub spine_content_paths: usize,
    pub aligned_blocks: usize,
    /// 未放置块数（pack 阶段 find 失败，被渲染为新块）。
    /// 观测用：大量未放置块可能意味着归章错位或文本流不匹配。
    pub unplaced_blocks: usize,
}

/// EPUB OPF 中可直接用于书库展示的基础元数据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubBookMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataField {
    Title,
    Creator,
}

/// 读取 EPUB 的 OPF 标题与作者；依据 XML 结构，不依赖文件名或作者命名特例。
pub fn read_opf_metadata(source_epub: &Path) -> Result<EpubBookMetadata, AppError> {
    let mut archive = open_archive(source_epub)?;
    let opf_path = locate_opf_path(&mut archive)?;
    let opf_content = read_entry_to_string(&mut archive, &opf_path)?;
    parse_opf_metadata(&opf_content)
}

/// 读取 EPUB 的 OPF dc:title（用于打包书名，避免硬编码）。
pub fn read_opf_title(source_epub: &Path) -> Option<String> {
    read_opf_metadata(source_epub).ok()?.title
}

fn parse_opf_metadata(opf_content: &str) -> Result<EpubBookMetadata, AppError> {
    let mut reader = Reader::from_str(opf_content);
    reader.config_mut().trim_text(false);
    let mut metadata = EpubBookMetadata::default();
    let mut active_field = None;
    let mut value = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                if let Some(field) = metadata_field(event.name().as_ref()) {
                    active_field = Some(field);
                    value.clear();
                }
            }
            Ok(Event::Text(event)) if active_field.is_some() => {
                value.push_str(&event.decode().map_err(|error| {
                    AppError::internal(format!("parse EPUB metadata text failed: {error}"))
                })?);
            }
            Ok(Event::End(event)) => {
                let closing_field = metadata_field(event.name().as_ref());
                if closing_field.is_some() && closing_field == active_field {
                    commit_metadata_value(&mut metadata, active_field, &value);
                    active_field = None;
                    value.clear();
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!(
                    "parse EPUB OPF metadata failed: {error}"
                )));
            }
            _ => {}
        }
    }

    Ok(metadata)
}

fn metadata_field(name: &[u8]) -> Option<MetadataField> {
    match xml_local_name(name) {
        b"title" => Some(MetadataField::Title),
        b"creator" => Some(MetadataField::Creator),
        _ => None,
    }
}

fn xml_local_name(qualified: &[u8]) -> &[u8] {
    qualified
        .rsplit(|byte| *byte == b':')
        .next()
        .unwrap_or(qualified)
}

fn commit_metadata_value(
    metadata: &mut EpubBookMetadata,
    field: Option<MetadataField>,
    value: &str,
) {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return;
    }
    match field {
        Some(MetadataField::Title) => {
            if metadata.title.is_none() {
                metadata.title = Some(normalized);
            }
        }
        Some(MetadataField::Creator) => {
            if !metadata.authors.iter().any(|author| author == &normalized) {
                metadata.authors.push(normalized);
            }
        }
        None => {}
    }
}

// ---- 目录（nav.xhtml / toc.ncx）标题翻译 ----

/// 生成 nav.xhtml 与 toc.ncx 的翻译后内容。href->中文标题 映射来自 translated_toc.json。
fn build_toc_replacements(
    archive: &mut ZipArchive<File>,
    base_dir: &str,
    nav_href: Option<&str>,
    toc: &[TocEntry],
) -> Result<Vec<(String, String)>, AppError> {
    // href(archive 绝对路径) -> 中文标题
    let mut title_by_href: HashMap<String, String> = HashMap::new();
    for entry in toc {
        if let (Some(href), Some(zh)) = (&entry.href, &entry.translated_title) {
            if !zh.trim().is_empty() {
                title_by_href.insert(
                    normalize_href_key(&join_epub_path(base_dir, href)),
                    zh.clone(),
                );
            }
        }
    }
    let mut out = Vec::new();
    // nav.xhtml（EPUB3）
    if let Some(nav_href) = nav_href {
        let nav_path = join_epub_path(base_dir, nav_href);
        if let Ok(raw) = read_entry_to_string(archive, &nav_path) {
            let base = parent_dir(&nav_path);
            out.push((nav_path, translate_nav_titles(&raw, &base, &title_by_href)));
        }
    }
    // toc.ncx（EPUB2 向后兼容）
    if let Some(ncx_path) = locate_ncx_path(archive, base_dir)? {
        if let Ok(raw) = read_entry_to_string(archive, &ncx_path) {
            let base = parent_dir(&ncx_path);
            out.push((ncx_path, translate_ncx_titles(&raw, &base, &title_by_href)));
        }
    }
    Ok(out)
}

/// 替换 nav.xhtml 里 <a href="...">label</a> 的 label（按 href 匹配中文标题）。
/// 同时移除 page-list nav（页码目录在翻译后无意义，避免误导用户点击失效页码）。
fn translate_nav_titles(
    raw: &str,
    nav_base: &str,
    title_by_href: &HashMap<String, String>,
) -> String {
    // 移除 page-list nav（<nav epub:type="page-list" ...>...</nav> 整块）
    let page_list_re = regex::Regex::new(r#"(?is)<nav[^>]*epub:type="page-list"[^>]*>.*?</nav>"#);
    let raw = match page_list_re {
        Ok(re) => re.replace_all(raw, "").to_string(),
        Err(_) => raw.to_string(),
    };
    replace_anchor_labels(&raw, |href| {
        let key = normalize_href_key(&join_epub_path(nav_base, href));
        title_by_href.get(&key).cloned()
    })
}

static NCX_NAVPOINT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"(?is)(<navLabel>\s*<text[^>]*>)(.*?)(</text>\s*</navLabel>\s*<content[^>]*src="([^"]+)")"#,
    ).unwrap()
});
static ANCHOR_LABEL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"(?is)(<a\b[^>]*href="([^"]+)"[^>]*>)(.*?)(</a>)"#).unwrap()
});

/// 替换 toc.ncx 里 <navLabel><text>label</text></navLabel>（配对的 content src）的 label。
fn translate_ncx_titles(
    raw: &str,
    ncx_base: &str,
    title_by_href: &HashMap<String, String>,
) -> String {
    // ncx 结构：<navPoint><navLabel><text>LABEL</text></navLabel><content src="HREF"/></navPoint>
    // 简化：用正则按 navPoint 块配对 text 与其后的 content src。
    NCX_NAVPOINT_RE
        .replace_all(raw, |caps: &regex::Captures| {
            let href = caps.get(4).map(|m| m.as_str()).unwrap_or("");
            let key = normalize_href_key(&join_epub_path(ncx_base, href));
            match title_by_href.get(&key) {
                Some(zh) => format!("{}{}{}", &caps[1], escape_xml(zh), &caps[3]),
                None => caps[0].to_string(),
            }
        })
        .to_string()
}

/// 通用：替换所有 <a href="HREF" ...>LABEL</a> 的 LABEL，label 由 resolve(HREF) 决定。
fn replace_anchor_labels(raw: &str, resolve: impl Fn(&str) -> Option<String>) -> String {
    ANCHOR_LABEL_RE
        .replace_all(raw, |caps: &regex::Captures| {
            let href = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            match resolve(href) {
                Some(zh) => format!("{}{}{}", &caps[1], escape_xml(&zh), &caps[4]),
                None => caps[0].to_string(),
            }
        })
        .to_string()
}

fn escape_xml(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn locate_ncx_path(
    archive: &mut ZipArchive<File>,
    base_dir: &str,
) -> Result<Option<String>, AppError> {
    // 从 OPF manifest 找 media-type=application/x-dtbncx+xml 的 item。
    let opf_path = locate_opf_path(archive)?;
    let opf = read_entry_to_string(archive, &opf_path)?;
    let mut reader = Reader::from_str(&opf);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if e.name().as_ref() == b"item" => {
                let mut href = String::new();
                let mut media = String::new();
                for attr in e.attributes().flatten() {
                    let v = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|e| AppError::internal(format!("parse ncx item failed: {e}")))?
                        .to_string();
                    match attr.key.as_ref() {
                        b"href" => href = v,
                        b"media-type" => media = v,
                        _ => {}
                    }
                }
                if media == "application/x-dtbncx+xml" && !href.is_empty() {
                    return Ok(Some(join_epub_path(base_dir, &href)));
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(AppError::internal(format!("parse opf ncx failed: {e}"))),
            _ => {}
        }
    }
    Ok(None)
}

// ---- 内容锚定（content-anchored placement）----

// 正则静态化：normalize_stream_text 是打包全程最热函数（每块×每段调用），
// 每次调用编译 6 条正则是纯浪费。用 LazyLock 全局一次编译。
static IMG_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap());
static LINK_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());
static TAG_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());
static NUMERIC_ENTITY_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"&#(\d+);").unwrap());
static PAGEBREAK_SPAN_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"(?is)<span[^>]*role="doc-pagebreak"[^>]*/?>"#).unwrap()
});

/// 归一化文本：去 markdown 装饰/HTML 标签/空白/大小写，用于源文本与 spine 文本流比对。
/// 与读取侧（pandoc）产出的块文本保持同一套规约，确保「块文本是 spine 文本流的子序列」。
/// HTML 实体用 regex 处理 `&#数字;` 与命名实体，避免链式 replace 把 `&#39;` 误切成 `&`+`39;`。
fn normalize_stream_text(raw: &str) -> String {
    // 去 markdown 图片 ![](...)
    let s = IMG_RE.replace_all(raw, "");
    // markdown 链接 [text](url) → text
    let s = LINK_RE.replace_all(&s, "$1");
    // 去 markdown 强调符号（* 用于斜体/粗体标记；# 是标题符号但可能是 HTML 实体
    // `&#39;` 的一部分，故 # 留到实体解码之后再删）。
    let s = s.replace('*', "");
    // 去 HTML 标签
    let s = TAG_RE.replace_all(&s, "");
    // HTML 实体：先 &#数字;（含 &#39;），后命名实体。命名实体里的 &amp; 放最后，
    // 避免把 &#39; 的 & 误当 &amp; 拆成 &+39;。
    let s = NUMERIC_ENTITY_RE.replace_all(&s, |caps: &regex::Captures| {
        let code: u32 = caps[1].parse().unwrap_or(0);
        char::from_u32(code)
            .map(|c| c.to_string())
            .unwrap_or_default()
    });
    let s = s
        .replace("&rsquo;", "'")
        .replace("&lsquo;", "'")
        .replace("&ldquo;", "\"")
        .replace("&rdquo;", "\"")
        .replace("&hellip;", "…")
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // 实体解码完再删 markdown 标题符号 #
    let s = s.replace('#', "");
    // 去所有空白 + 小写
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 剥离 pagebreak span（EPUB 页码标记，读取侧 pandoc 会丢弃，打包前同样剔除以保持文本流一致）。
fn strip_pagebreak_spans(html: &str) -> String {
    PAGEBREAK_SPAN_RE.replace_all(html, "").to_string()
}

/// 内容锚定结果：每个 spine 文件（按 spine 顺序索引）分到哪些块（保序）。
struct Placement<'a> {
    /// (spine_file_index, blocks_in_order)。只含有块的文件。
    blocks_by_file: Vec<(usize, Vec<&'a TranslatedBlock>)>,
    /// 未放置块数（pack 阶段 find 失败，被附到前驱/后继文件渲染为新块）。
    unplaced_blocks: usize,
}

/// 把全部块按「源文本在 spine 归一化文本流中的位置」归章。

/// 算法：全书块序 = 阅读序，spine 文件序 = 阅读序，二者同源 → 单调指针全文 find。
/// 防虚锚：若找到的匹配位置跳过的未读区间远超「连续未放置块的文本量 + 注入标题容差」，
/// 判定为「在后方重复文本处的误锚」（如 signup 文本在书末版权页重复出现），拒绝并保指针。
fn place_blocks_by_content<'a>(
    archive: &mut ZipArchive<File>,
    spine_files: &[String],
    blocks: &[&'a TranslatedBlock],
    block_norms: &[String],
) -> Result<Placement<'a>, AppError> {
    // 1) spine 归一化文本流 + 每文件字符区间
    let mut stream = String::new();
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(spine_files.len());
    for path in spine_files {
        let raw = read_entry_to_string(archive, path).unwrap_or_default();
        let body = extract_body_inner(&raw);
        let body = strip_pagebreak_spans(&body);
        let start = stream.len();
        stream.push_str(&normalize_stream_text(&body));
        ranges.push((start, stream.len()));
    }
    let stream = stream;

    // 2) 单调放置（虚锚拒绝）
    /// 跳过的未读区间容差：注入章标题（spine 文本中不存在的 markdown 标题）的字符预算。
    /// 动态化：取「累计未放置块的归一化文本长度 × 1.5 + 500」，下限 3000。
    /// 长前言/序言（front matter 几千字符未翻译）场景下 3000 可能不够。
    const GAP_TOLERANCE_BASE: usize = 3000;
    let mut file_of_block: Vec<Option<usize>> = Vec::with_capacity(blocks.len());
    let mut pos = 0usize;
    let mut pending = 0usize; // 连续未放置块的归一化文本总长
    for st in block_norms.iter() {
        if st.is_empty() {
            file_of_block.push(None);
            continue;
        }
        let head: String = st.chars().take(40).collect();
        let gap_tolerance = GAP_TOLERANCE_BASE.max((pending as f64 * 1.5 + 500.0) as usize);
        let found = stream[pos..].find(&head).map(|i| pos + i).or_else(|| {
            let short: String = st.chars().take(25).collect();
            stream[pos..].find(&short).map(|i| pos + i)
        });
        match found {
            Some(j) if j - pos <= pending + gap_tolerance => {
                let file_idx = ranges
                    .iter()
                    .position(|(s, e)| *s <= j && j < *e)
                    .unwrap_or(ranges.len() - 1);
                file_of_block.push(Some(file_idx));
                pos = j + head.len().min(stream.len() - j);
                pending = 0;
            }
            _ => {
                file_of_block.push(None);
                pending += st.len();
            }
        }
    }

    // 3) 分组（保序）：未放置块附到前驱块所在文件（注入标题/分隔线等，渲染为新块）。
    // 例外：「章首标题块」（章标题 'Chapter 2: Tash' + 其后的正文章标题 'Tash ![](…)'）
    // 属于**下一章**——pack 阶段它们会因前缀匹配命中前一章（'Tash' 头 40 字符匹配
    // 前一章 h1），若不纠正则前一章末尾多出下一章标题、且下一章缺 h1 护栏的
    // heading 块导致英文 h1 残留（实测 ch02/ch25/ch36/ch45）。

    // 判定：从「未放置的干净章标题块」（is_injected_chapter_title）开始，
    // 到「该章第一个非 heading 块成功放置」为止，期间所有块归到下一个
    // 非 heading 块所在文件。
    let mut by_file: Vec<Vec<&TranslatedBlock>> = vec![Vec::new(); spine_files.len()];
    let mut last_file = 0usize;
    // 预扫描：每个块若为章首块，其目标文件 = 其后第一个成功放置的非 heading 块的文件
    let mut chapter_target_of: Vec<Option<usize>> = vec![None; blocks.len()];
    let mut in_chapter_header = false;
    for (i, tb) in blocks.iter().enumerate() {
        if tb.kind == "heading" && file_of_block[i].is_none() && is_injected_chapter_title(tb) {
            in_chapter_header = true;
        }
        if in_chapter_header && tb.kind != "heading" && file_of_block[i].is_some() {
            in_chapter_header = false;
        }
        if in_chapter_header {
            chapter_target_of[i] = Some(usize::MAX); // 占位，下面回填
        }
    }
    // 回填：每个章首块的目标 = 其后第一个非 heading 已定位块的文件
    let mut next_body_file: Vec<Option<usize>> = vec![None; blocks.len()];
    let mut next: Option<usize> = None;
    for i in (0..blocks.len()).rev() {
        if blocks[i].kind != "heading" && file_of_block[i].is_some() {
            next = file_of_block[i];
        }
        next_body_file[i] = next;
    }
    for (i, (tb, fo)) in blocks.iter().zip(file_of_block.iter()).enumerate() {
        let target = if chapter_target_of[i].is_some() {
            next_body_file[i].unwrap_or(last_file)
        } else {
            match fo {
                Some(f) => *f,
                None => last_file,
            }
        };
        by_file[target].push(tb);
        if fo.is_some() {
            last_file = target;
        }
    }
    let blocks_by_file = by_file
        .into_iter()
        .enumerate()
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let unplaced_blocks = file_of_block.iter().filter(|fo| fo.is_none()).count();
    Ok(Placement {
        blocks_by_file,
        unplaced_blocks,
    })
}

/// 判断 heading 块是否是「注入章标题」（管线生成的 markdown 标题，spine 正文里
/// 没有对应段）：kind=heading 且 chapter_title 是干净标题（不含图片/路径）。
/// 用于分组时把它归到「下一个已定位块」所在文件，而非前驱块所在文件。
/// 章标题形式：'Chapter N: Name'、'Chapter N'、'第N章' 等含 chapter/章 关键词，
/// 区别于正文章标题（'Tash ![](…)' 的人物名+内联图，或短人名）。
fn is_injected_chapter_title(tb: &TranslatedBlock) -> bool {
    if tb.kind != "heading" {
        return false;
    }
    let Some(ct) = tb.chapter_title.as_deref() else {
        return false;
    };
    let clean = ct.split("![").next().unwrap_or(ct).trim();
    if clean.contains("imgs/") || clean.contains("![") || clean.chars().count() > 60 {
        return false;
    }
    if normalize_stream_text(&tb.source_text) != normalize_stream_text(clean) {
        return false;
    }
    // 章标题必须含 chapter/章 关键词（'Chapter 2: Tash'），
    // 排除正文章标题（'Tash' 人名标题，虽也指向下一文件但由 h1 护栏处理）
    let lower = clean.to_ascii_lowercase();
    lower.starts_with("chapter") || clean.contains('章')
}

// ---- 章节 xhtml 重建 ----

/// 用译文重建一章 xhtml：段级内容锚定对齐，尽量保留原 body 的结构骨架与排版 class。
/// 返回 (xhtml, aligned_blocks, total_translatable_blocks)。

/// 策略：把原 body 切成顶层块级元素段，构建该文件的归一化文本流；
/// 每块用「段级游标」找到其源文本所在段（支持一段多块——如页内 <i> 分块、
/// blockquote 子段、pagebreak 拆段）。单块段保留原 open_tag（class/id/版式），
/// 多块段合并渲染为一个双列容器（版式让位于对齐正确性）。
/// 未放置块（注入标题/分隔线）按位置渲染为新块。
fn rebuild_chapter_xhtml(
    raw: &str,
    blocks: &[&TranslatedBlock],
    mode: EpubPackMode,
    stylesheet_href: &str,
    chapter_dir: &str,
) -> Result<(String, usize, usize), AppError> {
    let stylesheet_rel = relative_path(chapter_dir, stylesheet_href);

    let mut aligned = 0usize;
    let mut total = 0usize;
    for tb in blocks {
        if tb.translate {
            total += 1;
            if !tb.translated_text.trim().is_empty() {
                aligned += 1;
            }
        }
    }

    let title = blocks
        .iter()
        .find_map(|b| b.chapter_title.as_deref())
        .map(|t| {
            let clean = t.split("![").next().unwrap_or(t).trim();
            clean.trim_start_matches('#').trim()
        })
        .filter(|t| !t.is_empty())
        .unwrap_or("Translated Chapter");
    let title = escape_html(&title);
    let head = extract_head(raw).unwrap_or_else(|| {
        format!("<head>\n<meta charset=\"utf-8\" />\n<title>{title}</title>\n</head>")
    });
    let head = inject_stylesheet_into_head(&head, &stylesheet_rel);

    let body = rebuild_body_content_anchored(raw, blocks, mode);
    let body = resolve_image_paths_str(body, chapter_dir);

    let xhtml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"zh-CN\" lang=\"zh-CN\">\n{head}\n<body>\n{body}\n</body>\n</html>\n"
    );
    Ok((xhtml, aligned, total))
}

/// 抽取原 <head>…</head>（保留 meta/link/原有样式）。
fn extract_head(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let open = lower.find("<head")?;
    let open_end = lower[open..].find('>').map(|i| open + i + 1)?;
    let close = lower[open_end..].find("</head>").map(|i| open_end + i)?;
    Some(raw[open..close + "</head>".len()].to_string())
}

/// 在 <head> 内 </head> 前插入样式表 link（幂等）。
fn inject_stylesheet_into_head(head: &str, stylesheet_rel: &str) -> String {
    if head.contains(stylesheet_rel) {
        return head.to_string();
    }
    let link = format!("<link rel=\"stylesheet\" type=\"text/css\" href=\"{stylesheet_rel}\" />");
    head.replacen("</head>", &format!("{link}\n</head>"), 1)
}

/// 计算本章「注入标题」的归一化文本：块里第一个「源文与 chapterTitle 一致」的
/// heading 块的 chapter_title 归一化。用于识别并跳过 spine 中不存在的注入标题。
/// 返回空串表示无注入标题。
fn pack_chapter_title_norm(blocks: &[&TranslatedBlock]) -> String {
    for tb in blocks {
        if tb.kind != "heading" {
            continue;
        }
        if let Some(ct) = tb.chapter_title.as_deref() {
            let ct_clean = ct.split("![").next().unwrap_or(ct).trim();
            let src_norm = normalize_stream_text(&tb.source_text);
            let ct_norm = normalize_stream_text(ct_clean);
            if !ct_norm.is_empty() && src_norm == ct_norm {
                return ct_norm;
            }
        }
    }
    String::new()
}

/// 段级内容锚定重建：每块用段游标定位到原 body 的顶层段，原位替换文本。
/// 单块段保留 open_tag（版式）；多块段合并为一个双列容器；未匹配块渲染为新块。
fn rebuild_body_content_anchored(
    raw: &str,
    blocks: &[&TranslatedBlock],
    mode: EpubPackMode,
) -> String {
    let body_inner = extract_body_inner(raw);
    let body_inner = strip_pagebreak_spans(&body_inner);
    let segments = split_top_level_blocks(&body_inner);
    if segments.is_empty() {
        return render_blocks_flat(blocks, mode);
    }
    let seg_texts: Vec<String> = segments.iter().map(|s| normalize_stream_text(s)).collect();

    // 为每块找段（游标单调；支持同段多块）。空文本块（图片/分隔线）不定位，
    // 渲染时附到「下一个已定位块的段」之前，保持相对位置。

    // 注入标题护栏：章标题块（kind=heading 且其文本是该章目录标题）的源文
    // （如 'Copyright'/'Chapter 1: Tash'）是管线注入的 markdown 标题，spine 正文
    // 里**没有**对应段。若让它参与段级锚定，会被错误地锚到正文里「含同词」的段
    // （实测 'Copyright' 标题被锚到 'Copyright © 2023' 正文段，把游标推后，
    //  导致其后 'An Imprint...' 等正文段全部错位跳过、留英文）。
    // 判定：heading 块且其 norm 文本 == 章标题 norm → 视为注入标题，不定位。
    let chapter_title_norm = pack_chapter_title_norm(blocks);
    let mut seg_of_block: Vec<Option<usize>> = Vec::with_capacity(blocks.len());
    let mut block_covered_segs: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut si = 0usize;
    let mut consumed = 0usize;
    for tb in blocks {
        let bt = normalize_stream_text(&tb.source_text);
        if bt.is_empty() {
            seg_of_block.push(None);
            continue;
        }
        // 注入标题：跳过定位（渲染时作为新块插入，不占正文段）
        if tb.kind == "heading" && !chapter_title_norm.is_empty() && bt == chapter_title_norm {
            seg_of_block.push(None);
            continue;
        }
        // 章首 h1 标题段护栏：块的源文与该章 h1 段的文本一致（如 'Tash [小图]'），
        // 若让它锚定到 h1 段，replace 会把英文标题换成中文译文 + 原 h1 壳保留
        // （壳里的 <img>/class 还在）→ 中英双标题重复 + 顺序颠倒（实测 ch01:
        // mt-ch 中文标题在 section 外、h1chap 英文标题在 section 内）。
        // 不定位 → 渲染时作为新块在章首吐出，原 h1 段被「跳过」不重复输出。
        if tb.kind == "heading" && is_chapter_heading_segment(&bt, &segments, &seg_texts) {
            seg_of_block.push(None);
            continue;
        }
        // 分隔线块（管线把 chunk 边界的 `---` 输出为全 `-` paragraph）：
        // spine 里对应的是 <hr>（归一化后为空文本），永远匹配失败。
        // 不定位（渲染为 hr.mt-sep 新块），且**不阻塞游标**——否则后续块的
        // 段内匹配全部失败，整段原文残留、译文堆到章末（实测 ch03/ch09）。
        if bt.chars().all(|c| c == '-') {
            seg_of_block.push(None);
            continue;
        }
        let mut found = None;
        for k in si..segments.len() {
            let st = &seg_texts[k];
            let sub = if k == si {
                &st[consumed.min(st.len())..]
            } else {
                &st[..]
            };
            if let Some(idx) = sub.find(&bt) {
                if k == si {
                    consumed += idx + bt.len();
                } else {
                    si = k;
                    consumed = bt.len();
                }
                found = Some(k);
                break;
            }
        }
        // 跨段块回退：块（如 blockquote）的归一化文本横跨多个连续段（管线把多段
        // 引用块合成一块），单段内 find 必然失败。改用「跨段窗口」拼接段文本再
        // 匹配，命中后锚定到起始段，并把游标推到末尾段（游标按段前进，本块
        // 覆盖的段在渲染时整段替换，后续块从这些段之后继续）。
        if found.is_none() && bt.len() > 40 {
            let mut window = String::new();
            for k in si..segments.len() {
                window.push_str(&seg_texts[k]);
                if window.len() > bt.len() + 200 {
                    break;
                }
                if window.find(&bt).is_some() {
                    found = Some(si);
                    si = k;
                    consumed = seg_texts[k].len();
                    break;
                }
            }
        }
        // 记录块覆盖的段范围（跨段块覆盖 found..=si，单段块覆盖 found）
        if let Some(start_seg) = found {
            for covered in start_seg..=si {
                block_covered_segs.insert(covered);
            }
        }
        seg_of_block.push(found);
    }

    // 逐段渲染：先发制人地吐出「属于本段之前」的未定位块，再渲染本段。
    // 「章首标题段已由 heading 块接管」判定：若段是 h1-h6 标题段且某未定位
    // heading 块源文与其一致，则该段被跳过（不再输出英文原题），
    // 同时让对应的未定位 heading 块在「下一个段」之前就地吐出（中文版章首
    // 就是中文标题+正文），避免 h1 壳残留（`</span></h1>` 孤儿标签）。
    let mut skipped_chapter_heading_norms: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut out = String::new();
    let mut bi = 0usize;
    for (seg_idx, seg) in segments.iter().enumerate() {
        // 未定位块：若其后第一个已定位块落在「更后面的段」，则它属于本段之前，吐出；
        // 若落在本段或没有后续已定位块，则留给段后/尾部处理。
        // 全 `-` 分隔线块例外：就地吐出（hr.mt-sep），不等待「更后面的段」——
        // 它的目标位置就是当前段之前（对应 spine 的 <hr>），等下去会被推到章末。
        // 章首标题块例外：其英文原题段已被跳过，就地吐出中文标题块，
        // 不等「更后面的段」（否则标题被推到章中/章末，且英文壳残留）。
        while bi < blocks.len() && seg_of_block[bi].is_none() {
            let block_text = normalize_stream_text(&blocks[bi].source_text);
            // 全 `-` 分隔线块：若当前段本身以 <hr> 开头（secbreak），原 hr 已承担
            // 分隔作用，跳过该块（不重复渲染 mt-sep，避免双横线）；否则就地渲染
            // 为 hr.mt-sep，不等待「更后面的段」（等下去会被推到章末）。
            if !block_text.is_empty() && block_text.chars().all(|c| c == '-') {
                if seg.trim_start().starts_with("<hr") {
                    bi += 1; // 原 hr 保留，管线分隔块去重
                    continue;
                }
                out.push_str("<hr class=\"mt-sep\" />\n");
                bi += 1;
                continue;
            }
            if blocks[bi].kind == "heading" && skipped_chapter_heading_norms.contains(&block_text) {
                out.push_str(&render_translated_block(blocks[bi], mode));
                bi += 1;
                continue;
            }
            let next_seg = (bi + 1..blocks.len()).find_map(|i| seg_of_block[i]);
            match next_seg {
                Some(s) if s > seg_idx => {
                    out.push_str(&render_translated_block(blocks[bi], mode));
                    bi += 1;
                }
                _ => break,
            }
        }
        // 收集本段块
        let mut seg_blocks: Vec<&TranslatedBlock> = Vec::new();
        while bi < blocks.len() && seg_of_block[bi] == Some(seg_idx) {
            seg_blocks.push(blocks[bi]);
            bi += 1;
        }
        if seg_blocks.is_empty() {
            // 跨段块覆盖的段（非起始段）：该段的文本已被跨段块（如 blockquote）
            // 整体替换到起始段的渲染结果里，此处跳过避免英文原文残留。
            if block_covered_segs.contains(&seg_idx) {
                continue;
            }
            // 章首 h1 标题段：若某未定位 heading 块的源文与本段文本一致，
            // 说明该 h1 已由管线生成中文标题块（已在上文吐出），此处跳过原英文 h1 段，
            // 避免中英双标题重复 + 顺序颠倒（实测 ch01 顶部 mt-ch 中文 + h1chap 英文）。
            let seg_norm = normalize_stream_text(seg);
            if !seg_norm.is_empty()
                && blocks.iter().any(|tb| {
                    tb.kind == "heading" && normalize_stream_text(&tb.source_text) == seg_norm
                })
            {
                skipped_chapter_heading_norms.insert(seg_norm);
                continue;
            }
            // h1 壳残留段（seg 2：`</span></h1>`——h1 被拆成两半，前半段已跳过）：
            // 归一化为空文本、不含可译内容，且上一段刚被跳过（章首标题），
            // 此处同样是壳，跳过避免孤儿闭标签出现在正文顶部。
            if seg_norm.is_empty()
                && seg.trim_start().starts_with("</")
                && !skipped_chapter_heading_norms.is_empty()
            {
                continue;
            }
            out.push_str(seg);
            out.push('\n');
        } else {
            out.push_str(&render_segment_with_blocks(seg, &seg_blocks, mode));
        }
    }
    // 尾部剩余块（未定位 + 尾段后块）
    while bi < blocks.len() {
        out.push_str(&render_translated_block(blocks[bi], mode));
        bi += 1;
    }
    out
}

static HEADING_SEG_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\s*(?:<(?:section|div)\b[^>]*>\s*)*<h[1-6]\b").unwrap()
});

/// 判断 heading 块的源文是否与某 h1-h6 段的归一化文本一致（章首标题段）。
/// 用于识别「管线为章首 h1 生成的标题块」，避免它锚定回原 h1 段造成中英双标题。
fn is_chapter_heading_segment(block_text: &str, segments: &[String], seg_texts: &[String]) -> bool {
    if block_text.is_empty() {
        return false;
    }
    segments
        .iter()
        .zip(seg_texts.iter())
        .any(|(seg, st)| HEADING_SEG_RE.is_match(seg) && st == block_text)
}

/// 渲染一个顶层段及其归属块：单块保留 open_tag 版式原位替换；多块合并为一个容器。
fn render_segment_with_blocks(
    seg: &str,
    blocks: &[&TranslatedBlock],
    mode: EpubPackMode,
) -> String {
    if blocks.len() == 1 {
        return replace_block_content(seg, blocks[0], mode);
    }
    // 跨段大块（如 blockquote 覆盖多个连续段）：单块但源文横跨多段，
    // replace_block_content 只会替换起始段、后续段的英文原文残留。
    // 判定：块的归一化文本长度 > 起始段归一化文本长度（块超出单段范围）。
    // 策略：用译文整体替换起始段内容，后续被覆盖的段由渲染循环跳过
    // （游标已推到末尾段，这些段不会再被 RAW 输出）。
    if let &[tb] = blocks {
        let bt = normalize_stream_text(&tb.source_text);
        let st = normalize_stream_text(seg);
        if bt.len() > st.len() + 40 {
            return replace_block_content(seg, tb, mode);
        }
    }
    // 多块段：保留段的开标签（含 class），内部放多个双列对。
    let block_open = find_last_block_open_tag(seg);
    let Some((open_start, open_end)) = block_open else {
        // 无法解析段标签 → 逐块扁平渲染
        let mut out = String::new();
        for tb in blocks {
            out.push_str(&render_translated_block(tb, mode));
        }
        return out;
    };
    let prefix = &seg[..open_start];
    let open_tag = &seg[open_start..open_end];
    let tag_name = open_tag
        .trim_start_matches('<')
        .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .next()
        .unwrap_or("p")
        .to_ascii_lowercase();
    if open_tag.ends_with("/>") || tag_name == "hr" || tag_name == "img" {
        return seg.to_string();
    }
    let close_tag = format!("</{tag_name}>");
    let mut inner = String::new();
    for tb in blocks {
        let zh = if tb.translated_markdown.trim().is_empty() {
            tb.source_text.trim()
        } else {
            tb.translated_markdown.trim()
        };
        let en = tb.source_text.trim();
        match mode {
            EpubPackMode::Chinese => {
                inner.push_str(&format!(
                    "<span class=\"mt-zh\">{}</span><br/>\n",
                    inline(strip_heading_marks(zh))
                ));
            }
            EpubPackMode::Bilingual => {
                inner.push_str(&format!(
                    "<span class=\"mt-zh\">{}</span><br/><span class=\"mt-en\">{}</span><br/><br/>\n",
                    inline(strip_heading_marks(zh)),
                    inline(strip_heading_marks(en))
                ));
            }
        }
    }
    format!("{prefix}{open_tag}{inner}{close_tag}\n")
}

static BLOCK_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"(?is)(?:<(?:section|div)\b[^>]*>)*<(?:p|h[1-6]|figure|blockquote|hr|ul|ol|table|img)\b[^>]*>.*?(?:</(?:p|h[1-6]|figure|blockquote|ul|ol|table)>|<hr[^>]*/>|<img[^>]*/>|$)"#,
    ).unwrap()
});

/// 把 body 切成顶层块级元素段（<p>/<h1..h6>/<figure>/<blockquote>/<hr>/<ul>/<ol>/<table> 等）。
/// 容器标签（section/div）只作包装不当作可译块，故不匹配它们——让它们的开标签作为
/// 前一个可译块的前缀保留，从而保住 <section ...> 的结构与 aria 属性。
fn split_top_level_blocks(body: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut last = 0usize;
    for m in BLOCK_RE.find_iter(body) {
        if m.start() > last {
            segments.push(body[last..m.start()].to_string());
        }
        segments.push(m.as_str().to_string());
        last = m.end();
    }
    if last < body.len() {
        segments.push(body[last..].to_string());
    }
    segments
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// 在原块级元素内替换内容：保留开标签（含 class/id），替换内部文本为译文/双列。
/// 段首可能带容器开标签（<section ...>），需跳到最后一个块级开标签再替换。
fn replace_block_content(seg: &str, tb: &TranslatedBlock, mode: EpubPackMode) -> String {
    // 找最后一个块级开标签（跳过前置的 <section>/<div> 容器标签）。
    let block_open = find_last_block_open_tag(seg);
    let Some((open_start, open_end)) = block_open else {
        return render_translated_block(tb, mode);
    };
    let prefix = &seg[..open_start]; // 容器开标签（<section ...>）或前置 <hr/> 等
    let open_tag = &seg[open_start..open_end];
    let tag_name = open_tag
        .trim_start_matches('<')
        .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .next()
        .unwrap_or("p")
        .to_ascii_lowercase();
    let is_heading = matches!(tag_name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    let content = render_block_inner(tb, mode, is_heading);
    if open_tag.ends_with("/>") || tag_name == "hr" || tag_name == "img" {
        return seg.to_string();
    }
    // 段内嵌的内联格式标签（<span class="smallcaps">/<em>/<i> 等包裹段首词）：
    // 译文是完整翻译后的纯文本，保留这些英文内联标签会导致「半英半中」
    // （如 <span class="smallcaps">I ARRIVE</span> 剩下英文壳、中文正文接在后面）。
    // 故替换时丢弃段内所有内联标签，只保留最外层块级开标签（含 class 版式）。
    let close_tag = format!("</{tag_name}>");
    if seg.rfind(&close_tag).is_some() {
        // 保留前置容器开标签 + 原块开标签 + 新内容 + 闭标签（容器闭标签 </section> 由原
        // 段末的 close 匹配保留，或被丢弃——容器闭标签由下一个段的前缀重新带上，故此处
        // 若段末无 </section> 也无妨，下一区块会重开）。
        format!("{prefix}{open_tag}{content}{close_tag}\n")
    } else {
        seg.to_string()
    }
}

static BLOCK_OPEN_TAG_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"(?i)<(?:p|h[1-6]|figure|blockquote|ul|ol|table|img|hr)\b[^>]*>"#).unwrap()
});

/// 找 seg 中最后一个块级开标签（p/h1-6/figure/blockquote/ul/ol/table）的位置与结束。
/// 跳过 <section>/<div> 容器标签。
fn find_last_block_open_tag(seg: &str) -> Option<(usize, usize)> {
    let mut last: Option<(usize, usize)> = None;
    for m in BLOCK_OPEN_TAG_RE.find_iter(seg) {
        last = Some((m.start(), m.end()));
    }
    last
}

/// 渲染块内容（不含外层标签）。
fn render_block_inner(tb: &TranslatedBlock, mode: EpubPackMode, is_heading: bool) -> String {
    let zh = if tb.translated_markdown.trim().is_empty() {
        tb.source_text.trim()
    } else {
        tb.translated_markdown.trim()
    };
    let en = tb.source_text.trim();
    let zh_text = strip_heading_marks(zh);
    let en_text = strip_heading_marks(en);
    match mode {
        EpubPackMode::Chinese => inline(zh_text),
        EpubPackMode::Bilingual => {
            if is_heading {
                format!(
                    "<span class=\"mt-zh\">{}</span><br/><span class=\"mt-en\">{}</span>",
                    inline(zh_text),
                    inline(en_text)
                )
            } else {
                format!(
                    "<span class=\"mt-zh\">{}</span><br/><span class=\"mt-en\">{}</span>",
                    inline(zh_text),
                    inline(en_text)
                )
            }
        }
    }
}

/// 扁平渲染（无法切段时的回退）：直接逐块渲染。
fn render_blocks_flat(blocks: &[&TranslatedBlock], mode: EpubPackMode) -> String {
    let mut body = String::new();
    for tb in blocks {
        body.push_str(&render_translated_block(tb, mode));
    }
    body
}

/// 渲染单个对齐 block（管线产物）为 xhtml 片段。
fn render_translated_block(tb: &TranslatedBlock, mode: EpubPackMode) -> String {
    let kind = tb.kind.as_str();
    if !tb.translate {
        return render_structural_translated_block(tb);
    }
    let zh = if tb.translated_markdown.trim().is_empty() {
        tb.source_text.trim()
    } else {
        tb.translated_markdown.trim()
    };
    let en = tb.source_text.trim();
    let zh_text = strip_heading_marks(zh);
    let en_text = strip_heading_marks(en);
    match (kind, mode) {
        ("heading", EpubPackMode::Chinese) => format!("<h1 class=\"mt-ch\">{}</h1>\n", inline(zh_text)),
        ("heading", EpubPackMode::Bilingual) => format!(
            "<h1 class=\"mt-ch\">{}</h1>\n<h2 class=\"mt-en-h\">{}</h2>\n",
            inline(zh_text),
            inline(en_text)
        ),
        (_, EpubPackMode::Chinese) => format!("<p class=\"mt-ch\">{}</p>\n", inline(zh_text)),
        (_, EpubPackMode::Bilingual) => format!(
            "<div class=\"mt-pair\"><p class=\"mt-col mt-zh\">{}</p><p class=\"mt-col mt-en\">{}</p></div>\n",
            inline(zh_text),
            inline(en_text)
        ),
    }
}

/// 非译结构块（图片/分隔/空白）：尽量保留源 markdown/HTML 渲染。
fn render_structural_translated_block(tb: &TranslatedBlock) -> String {
    if tb.kind == "image" {
        if !tb.source_markdown.trim().is_empty() {
            return format!("<p class=\"mt-fig\">{}</p>\n", inline(&tb.source_markdown));
        }
        if !tb.source_text.trim().is_empty() {
            return format!("<p class=\"mt-fig\">{}</p>\n", inline(&tb.source_text));
        }
        return String::new();
    }
    if tb.source_markdown.trim() == "---" || tb.source_text.trim() == "---" {
        return "<hr class=\"mt-sep\" />\n".to_string();
    }
    if tb.source_text.trim().is_empty() {
        return String::new();
    }
    format!("<p class=\"mt-keep\">{}</p>\n", inline(&tb.source_text))
}

/// 去掉行首 markdown 标题标记（'#' 序列 + 空白），其余原样返回。
fn strip_heading_marks(text: &str) -> &str {
    let trimmed = text.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 {
        return trimmed;
    }
    trimmed[hashes..].trim_start()
}

/// 轻量 markdown 内联 → xhtml（粗体/斜体/图片），并转义其余文本。
fn inline(markdown: &str) -> String {
    let mut out = String::new();
    let mut rest = markdown;
    while let Some(start) = rest.find("![") {
        let (before, after) = rest.split_at(start);
        out.push_str(&escape_html(before));
        if let Some(close) = after.find("](") {
            let alt = &after[2..close];
            let after_close = &after[close + 2..];
            if let Some(paren) = after_close.find(')') {
                let src = &after_close[..paren];
                out.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\" />",
                    escape_html(src),
                    escape_html(alt)
                ));
                rest = &after_close[paren + 1..];
                continue;
            }
        }
        out.push_str(&escape_html(after));
        rest = "";
        break;
    }
    out.push_str(&escape_html(rest));
    let out = replace_pairs(&out, "**", "<strong>", "</strong>");
    let out = replace_pairs(&out, "__", "<strong>", "</strong>");
    replace_pairs(&out, "*", "<em>", "</em>")
}

fn replace_pairs(input: &str, marker: &str, open: &str, close: &str) -> String {
    let mut result = String::new();
    let mut parts = input.split(marker).peekable();
    let mut is_open = true;
    while let Some(part) = parts.next() {
        result.push_str(part);
        if parts.peek().is_some() {
            if is_open {
                result.push_str(open);
            } else {
                result.push_str(close);
            }
            is_open = !is_open;
        }
    }
    result
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn extract_body_inner(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let open = lower
        .find("<body")
        .and_then(|i| lower[i..].find('>').map(|j| i + j + 1));
    let close = lower.rfind("</body>");
    match (open, close) {
        (Some(o), Some(c)) if c > o => raw[o..c].to_string(),
        _ => raw.to_string(),
    }
}

// ---- OPF / 样式表 ----

fn stylesheet_for_mode(mode: EpubPackMode) -> &'static str {
    match mode {
        EpubPackMode::Chinese => {
            "body{margin:1em 6%;font:1em/1.8 \"Source Han Serif SC\",\"Noto Serif CJK SC\",serif;color:#1a1a1a}p{margin:0 0 1em}h1.mt-ch{line-height:1.3;margin:.2em 0 1em;font-family:\"Source Han Sans SC\",\"Noto Sans CJK SC\",sans-serif}.mt-en{display:none}img{max-width:100%;height:auto}hr.mt-sep{border:none;border-top:1px solid #ccc;margin:1.5em 0}\n"
        }
        EpubPackMode::Bilingual => {
            // 双语模式统一为段级上下对（中文在前英文在后），替代双列 flex。
            // 中英长度差异大（中文通常比英文短 30-40%），双列时英文栏底下留白多；
            // 小屏阅读器双列被压扁，可读性反而下降；上下对符合自然阅读路径。
            "body{margin:1em 4%;font:1em/1.8 \"Source Han Serif SC\",\"Noto Serif CJK SC\",serif;color:#1a1a1a}.mt-pair{display:block;margin:0 0 1em}.mt-col{margin:0}.mt-zh{font-family:\"Source Han Sans SC\",\"Noto Sans CJK SC\",sans-serif;font-size:1em;margin:0 0 .3em}.mt-en{color:#3a3a3a;font-size:.92em;margin:0 0 .8em}h1.mt-ch{line-height:1.3;margin:.2em 0 .2em;font-family:\"Source Han Sans SC\",\"Noto Sans CJK SC\",sans-serif}h2.mt-en-h{font-size:.8em;color:#666;font-weight:normal;margin:0 0 1em}img{max-width:100%;height:auto}hr.mt-sep{border:none;border-top:1px solid #ccc;margin:1.5em 0}@media (prefers-contrast:high){.mt-en{color:#000}}\n"
        }
    }
}

/// 在 OPF manifest 注册样式表、在 metadata 更新标题；不做其它改动。
fn rewrite_opf(opf: &str, stylesheet_href: &str, book_title: &str, mode: EpubPackMode) -> String {
    let suffix = match mode {
        EpubPackMode::Chinese => "（中文版）",
        EpubPackMode::Bilingual => "（中英对照）",
    };
    let new_title = format!("{book_title}{suffix}");
    let mut out = opf.to_string();
    if let Some((start, end)) = find_first_tag_text(&out, "dc:title") {
        out.replace_range(start..end, &escape_html(&new_title));
    }
    if !out.contains(stylesheet_href) {
        if let Some(pos) = out.find("</manifest>") {
            let item = format!(
                "<item id=\"musetranslate-style\" href=\"{stylesheet_href}\" media-type=\"text/css\"/>"
            );
            out.insert_str(pos, &item);
        }
    }
    out
}

fn find_first_tag_text(xml: &str, tag: &str) -> Option<(usize, usize)> {
    let open = format!("<{tag}");
    let open_start = xml.find(&open)?;
    let open_end = xml[open_start..].find('>').map(|i| open_start + i + 1)?;
    let close = format!("</{tag}>");
    let close_start = xml[open_end..].find(&close).map(|i| open_end + i)?;
    Some((open_end, close_start))
}

// ---- zip 读取 / 写出 ----

fn open_archive(path: &Path) -> Result<ZipArchive<File>, AppError> {
    let file = File::open(path)
        .map_err(|error| AppError::internal(format!("open EPUB failed: {error}")))?;
    ZipArchive::new(file)
        .map_err(|error| AppError::internal(format!("read EPUB archive failed: {error}")))
}

fn read_entry_to_string(archive: &mut ZipArchive<File>, path: &str) -> Result<String, AppError> {
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

fn write_output_epub(
    archive: &mut ZipArchive<File>,
    output: &Path,
    replacements: &HashMap<String, String>,
    stylesheet_archive_path: &str,
    stylesheet: &str,
) -> Result<(), AppError> {
    let out_file = File::create(output)
        .map_err(|error| AppError::internal(format!("create output EPUB failed: {error}")))?;
    let mut writer = ZipWriter::new(out_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut wrote_stylesheet = false;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|error| AppError::internal(format!("read EPUB entry #{i} failed: {error}")))?;
        let name = entry.name().to_string();
        if name == stylesheet_archive_path {
            wrote_stylesheet = true;
        }
        // 非替换条目（图片/字体/CSS 等未变内容）：raw_copy_file 零解压直拷，
        // 避免全量解压+重压缩的 I/O 与 CPU 浪费（图多 EPUB 打包耗时可降 50-90%）。
        if !replacements.contains_key(&name) && name != "mimetype" {
            writer.raw_copy_file(entry).map_err(|error| {
                AppError::internal(format!("copy EPUB entry ({name}) failed: {error}"))
            })?;
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|error| {
            AppError::internal(format!("read EPUB entry bytes ({name}) failed: {error}"))
        })?;
        let opts = if name == "mimetype" {
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
        } else {
            options
        };
        writer.start_file(&name, opts).map_err(|error| {
            AppError::internal(format!("write EPUB entry ({name}) failed: {error}"))
        })?;
        let data: &[u8] = match replacements.get(&name) {
            Some(text) => text.as_bytes(),
            None => &bytes,
        };
        writer.write_all(data).map_err(|error| {
            AppError::internal(format!("write EPUB bytes ({name}) failed: {error}"))
        })?;
    }

    if !wrote_stylesheet {
        writer
            .start_file(stylesheet_archive_path, options)
            .map_err(|error| AppError::internal(format!("write stylesheet failed: {error}")))?;
        writer.write_all(stylesheet.as_bytes()).map_err(|error| {
            AppError::internal(format!("write stylesheet bytes failed: {error}"))
        })?;
    }

    writer
        .finish()
        .map_err(|error| AppError::internal(format!("finalize EPUB failed: {error}")))?;
    Ok(())
}

// ---- 读取侧路径/OPF 工具（打包侧最小等价实现） ----

fn locate_opf_path(archive: &mut ZipArchive<File>) -> Result<String, AppError> {
    let container = read_entry_to_string(archive, "META-INF/container.xml")?;
    let mut reader = Reader::from_str(&container);
    reader.config_mut().trim_text(true);
    let mut opf_path = None;
    loop {
        match reader.read_event() {
            Ok(Event::Empty(event)) | Ok(Event::Start(event)) => {
                if event.name().as_ref() == b"rootfile" {
                    opf_path = decode_rootfile_path(&reader, &event)?;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!(
                    "parse container.xml failed: {error}"
                )));
            }
            _ => {}
        }
    }
    opf_path.ok_or_else(|| AppError::internal("EPUB missing OPF path"))
}

fn decode_rootfile_path(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    for attribute in event.attributes().flatten() {
        if attribute.key.as_ref() == b"full-path" {
            return Ok(Some(
                attribute
                    .decode_and_unescape_value(reader.decoder())
                    .map_err(|error| {
                        AppError::internal(format!("parse EPUB root path failed: {error}"))
                    })?
                    .to_string(),
            ));
        }
    }
    Ok(None)
}

fn parse_opf_manifest_and_spine(
    opf_content: &str,
) -> Result<(Vec<String>, Option<String>, Option<String>, Vec<String>), AppError> {
    let mut reader = Reader::from_str(opf_content);
    reader.config_mut().trim_text(true);
    let mut manifest = Vec::new();
    let mut spine = Vec::new();
    let mut cover_id = None;
    let mut nav_href = None;
    loop {
        match reader.read_event() {
            Ok(Event::Empty(event)) | Ok(Event::Start(event)) => match event.name().as_ref() {
                b"item" => {
                    let entry = parse_manifest_item(&reader, &event)?;
                    if entry.1 {
                        cover_id = Some(entry.0.id.clone());
                    }
                    if entry.2 {
                        nav_href = Some(entry.0.href.clone());
                    }
                    manifest.push(entry.0);
                }
                b"itemref" => {
                    if let Some(idref) = parse_itemref_idref(&reader, &event)? {
                        spine.push(idref);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!(
                    "parse content.opf failed: {error}"
                )));
            }
            _ => {}
        }
    }
    let content_paths = collect_content_paths(&manifest, spine);
    let image_hrefs = manifest
        .iter()
        .filter(|e| e.media_type.starts_with("image/") && !e.href.is_empty())
        .map(|e| e.href.clone())
        .collect();
    Ok((
        content_paths,
        cover_id.map(|id| {
            manifest
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.href.clone())
                .unwrap_or_default()
        }),
        nav_href,
        image_hrefs,
    ))
}

struct ManifestEntry {
    id: String,
    href: String,
    media_type: String,
}

fn parse_manifest_item(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<(ManifestEntry, bool, bool), AppError> {
    let mut id = String::new();
    let mut href = String::new();
    let mut media_type = String::new();
    let mut properties = String::new();
    for attribute in event.attributes().flatten() {
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|error| {
                AppError::internal(format!("parse OPF item attribute failed: {error}"))
            })?
            .to_string();
        match attribute.key.as_ref() {
            b"id" => id = value,
            b"href" => href = value,
            b"media-type" => media_type = value,
            b"properties" => properties = value,
            _ => {}
        }
    }
    let is_cover = properties.contains("cover-image");
    let is_nav = properties.split_whitespace().any(|p| p == "nav") && media_type.contains("html");
    Ok((
        ManifestEntry {
            id,
            href,
            media_type,
        },
        is_cover,
        is_nav,
    ))
}

fn parse_itemref_idref(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    for attribute in event.attributes().flatten() {
        if attribute.key.as_ref() == b"idref" {
            return Ok(Some(
                attribute
                    .decode_and_unescape_value(reader.decoder())
                    .map_err(|error| {
                        AppError::internal(format!("parse OPF spine failed: {error}"))
                    })?
                    .to_string(),
            ));
        }
    }
    Ok(None)
}

fn collect_content_paths(manifest: &[ManifestEntry], spine: Vec<String>) -> Vec<String> {
    let mut content_paths = Vec::new();
    for idref in spine {
        if let Some(entry) = manifest.iter().find(|entry| entry.id == idref) {
            if entry.media_type.contains("html") || entry.media_type.contains("xhtml") {
                content_paths.push(entry.href.clone());
            }
        }
    }
    content_paths
}

fn join_epub_path(base_dir: &str, href: &str) -> String {
    super::path::join_epub_path(base_dir, href)
}

fn normalize_href_key(raw: &str) -> String {
    super::path::normalize_href_key(raw)
}

fn parent_dir(path: &str) -> String {
    super::path::parent_dir(path)
}

fn relative_path(from_dir: &str, to_path: &str) -> String {
    let depth = if from_dir.trim().is_empty() {
        0
    } else {
        from_dir.split('/').filter(|s| !s.is_empty()).count()
    };
    let prefix = "../".repeat(depth);
    format!("{prefix}{to_path}")
}

/// 把 xhtml 中的 artifact 图片引用（imgs/epub/<绝对路径>）重写为相对本章的路径。
fn resolve_image_paths_str(xhtml: String, chapter_dir: &str) -> String {
    let prefix = "imgs/epub/";
    let mut out = String::with_capacity(xhtml.len());
    let mut rest = xhtml.as_str();
    while let Some(idx) = rest.find(prefix) {
        let (before, after) = rest.split_at(idx);
        out.push_str(before);
        let tail = &after[prefix.len()..];
        let end = tail
            .find(|c| c == '"' || c == '\'' || c == ' ' || c == '>')
            .unwrap_or(tail.len());
        let abs_img = &tail[..end];
        let rel = relative_path(chapter_dir, abs_img);
        out.push_str(&rel);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opf_title_and_multiple_creators_structurally() {
        let opf = r#"
            <package xmlns:dc="http://purl.org/dc/elements/1.1/">
              <metadata>
                <dc:title id="main">The Other Mothers</dc:title>
                <dc:creator>Katherine Faulkner</dc:creator>
                <dc:creator>Second Author</dc:creator>
              </metadata>
            </package>
        "#;

        let metadata = parse_opf_metadata(opf).expect("metadata should parse");

        assert_eq!(metadata.title.as_deref(), Some("The Other Mothers"));
        assert_eq!(
            metadata.authors,
            vec!["Katherine Faulkner", "Second Author"]
        );
    }

    fn make_translated(order: usize, src: &str, zh: &str) -> TranslatedBlock {
        TranslatedBlock {
            id: format!("ch-000-b-{order:04}"),
            order,
            chunk_index: Some(0),
            marker_id: Some("B000".to_string()),
            alignment_method: "chunkLocalMarkerOrder".to_string(),
            kind: "paragraph".to_string(),
            source_markdown: src.to_string(),
            translated_markdown: zh.to_string(),
            source_text: src.to_string(),
            translated_text: zh.to_string(),
            translate: true,
            heading_path: Vec::new(),
            chapter_title: Some("Chapter 1".to_string()),
            href: Some("Text/ch1.xhtml".to_string()),
            page: None,
        }
    }

    #[test]
    fn bilingual_block_renders_side_by_side_pair() {
        let tb = make_translated(0, "Hello world.", "你好，世界。");
        let html = render_translated_block(&tb, EpubPackMode::Bilingual);
        assert!(html.contains("mt-pair"));
        assert!(html.contains("mt-zh"));
        assert!(html.contains("mt-en"));
        assert!(html.contains("你好，世界。"));
        assert!(html.contains("Hello world."));
    }

    #[test]
    fn chinese_block_renders_translation_only() {
        let tb = make_translated(0, "Hello world.", "你好，世界。");
        let html = render_translated_block(&tb, EpubPackMode::Chinese);
        assert!(html.contains("mt-ch"));
        assert!(html.contains("你好，世界。"));
        assert!(!html.contains("mt-pair"));
    }

    #[test]
    fn untranslated_block_falls_back_to_source_text() {
        let tb = make_translated(0, "Keep me.", "");
        let html = render_translated_block(&tb, EpubPackMode::Chinese);
        assert!(html.contains("Keep me."));
    }

    #[test]
    fn image_block_preserves_source_html() {
        let mut tb = make_translated(1, "![a](../images/a.jpg)", "");
        tb.kind = "image".to_string();
        tb.translate = false;
        let html = render_translated_block(&tb, EpubPackMode::Chinese);
        assert!(html.contains("../images/a.jpg"));
    }

    #[test]
    fn rebuild_chapter_counts_aligned_blocks() {
        let b0 = make_translated(0, "Chapter 1", "第一章");
        let b1 = make_translated(1, "First.", "第一段。");
        let b2 = make_translated(2, "Second.", "第二段。");
        let blocks: Vec<&TranslatedBlock> = vec![&b0, &b1, &b2];
        let raw = "<html><head><title>t</title></head><body><p>ignored</p></body></html>";
        let (xhtml, aligned, total) = rebuild_chapter_xhtml(
            raw,
            &blocks,
            EpubPackMode::Bilingual,
            "musetranslate-style.css",
            "Text",
        )
        .expect("rebuild");
        assert_eq!(total, 3);
        assert_eq!(aligned, 3);
        assert!(xhtml.contains("第一章"));
        assert!(xhtml.contains("musetranslate-style.css"));
    }

    #[test]
    fn preserve_layout_keeps_original_classes() {
        let b0 = make_translated(0, "North Cornwall police station", "北康沃尔警察局");
        let b1 = make_translated(1, "April 2019", "2019年4月");
        let blocks: Vec<&TranslatedBlock> = vec![&b0, &b1];
        let raw = "<html><head><title>t</title></head><body><section><p class=\"dateline1\">North Cornwall police station</p>\n<p class=\"dateline2\">April 2019</p></section></body></html>";
        let (xhtml, _, _) =
            rebuild_chapter_xhtml(raw, &blocks, EpubPackMode::Chinese, "s.css", "Text")
                .expect("rebuild");
        assert!(
            xhtml.contains("dateline1"),
            "should keep original class, got: {xhtml}"
        );
        assert!(xhtml.contains("北康沃尔警察局"));
    }

    #[test]
    fn content_anchor_ignores_wrong_caller_assignment() {
        // 通用性核心：即使调用方把块归错章（如 ch01 正文块被标成属于 sub01），
        // 段级内容锚定也能凭源文本把块放回正文真正所在的段。
        let b0 = make_translated(0, "Tash", "塔什");
        let b1 = make_translated(1, "North Cornwall police station", "北康沃尔警察局");
        let b2 = make_translated(
            2,
            "WE MEET IN A room with no windows.",
            "我们在一间没有窗户的房间里见面。",
        );
        let blocks: Vec<&TranslatedBlock> = vec![&b0, &b1, &b2];
        let raw = "<html><head><title>1. Tash</title></head><body><section aria-labelledby=\"ch01_hd\" role=\"doc-chapter\"><h1 class=\"h1chap\" id=\"ch01_hd\">Tash</h1><p class=\"dateline1\">North Cornwall police station</p><p class=\"noindent\">WE MEET IN A room with no windows.</p></section></body></html>";
        let (xhtml, _, _) = rebuild_chapter_xhtml(
            raw,
            &blocks,
            EpubPackMode::Bilingual,
            "s.css",
            "e9781668024805/xhtml",
        )
        .expect("rebuild");
        assert!(xhtml.contains("h1chap"), "keep h1 class: {xhtml}");
        assert!(xhtml.contains("dateline1"), "keep dateline class: {xhtml}");
        assert!(xhtml.contains("noindent"), "keep noindent class: {xhtml}");
        assert!(xhtml.contains("北康沃尔警察局"));
        assert!(xhtml.contains("doc-chapter"), "keep section aria: {xhtml}");
    }

    #[test]
    fn multi_block_segment_merges_into_one_container() {
        // 一段多块（如段内被 pagebreak/inline 拆分）：合并为一个双列容器而非错位。
        let b0 = make_translated(0, "First part ", "第一部分");
        let b1 = make_translated(1, "second part.", "第二部分");
        let blocks: Vec<&TranslatedBlock> = vec![&b0, &b1];
        let raw = "<html><head><title>t</title></head><body><p class=\"indent\">First part second part.</p></body></html>";
        let (xhtml, _, _) =
            rebuild_chapter_xhtml(raw, &blocks, EpubPackMode::Bilingual, "s.css", "Text")
                .expect("rebuild");
        assert!(xhtml.contains("第一部分"), "{xhtml}");
        assert!(xhtml.contains("第二部分"), "{xhtml}");
    }

    #[test]
    fn unplaced_injected_heading_renders_as_new_block() {
        // 注入标题（spine 文本中不存在）渲染为新块而非丢失。
        let mut b0 = make_translated(0, "Chapter 1: Tash", "第1章：塔什");
        b0.kind = "heading".to_string();
        let b1 = make_translated(1, "Body text here.", "正文。");
        let blocks: Vec<&TranslatedBlock> = vec![&b0, &b1];
        let raw = "<html><head><title>t</title></head><body><p>Body text here.</p></body></html>";
        let (xhtml, _, _) =
            rebuild_chapter_xhtml(raw, &blocks, EpubPackMode::Chinese, "s.css", "Text")
                .expect("rebuild");
        assert!(
            xhtml.contains("第1章：塔什"),
            "injected heading kept: {xhtml}"
        );
        assert!(xhtml.contains("正文。"));
    }

    #[test]
    fn normalize_stream_text_handles_markdown_and_html() {
        assert_eq!(
            normalize_stream_text("*Hello* [world](http://x) ![img](a.jpg) <b>bold</b>"),
            "helloworldbold"
        );
        assert_eq!(
            normalize_stream_text("Tom &amp; Jerry&#39;s"),
            "tom&jerry's"
        );
    }

    #[test]
    fn dash_separator_block_does_not_block_cursor_or_duplicate_translation() {
        // 回归（ch03/ch09 实测灾难）：管线把 chunk 边界的 `---` 输出为全 `-`
        // paragraph 块，spine 里对应 <hr>（归一化后为空文本），段级匹配失败 →
        // 未定位。若让它卡住游标，后续同 chunk 块全部段内匹配失败，整段原文
        // 残留、译文堆到章末（英文一遍+中文一遍）。
        // 期望：分隔线块渲染为 hr.mt-sep 就地吐出、不阻塞游标，后续块正常归段。
        let b0 = make_translated(0, "Before the break.", "分隔前。");
        let mut b1 = make_translated(
            1,
            "------------------------------------------------------------------------",
            "------------------------------------------------------------------------",
        );
        b1.chapter_title = Some("Tash".to_string());
        let b2 = make_translated(
            2,
            "I ARRIVE in the kitchen just in time.",
            "我走进厨房时，正好赶上。",
        );
        let b3 = make_translated(3, "You can watch one Bing.", "你可以看一集《小兔兵兵》。");
        let blocks: Vec<&TranslatedBlock> = vec![&b0, &b1, &b2, &b3];
        let raw = "<html><head><title>t</title></head><body><section role=\"doc-chapter\"><p class=\"indent\">Before the break.</p><hr class=\"secbreak\"/>\n<p class=\"noindent\"><span class=\"smallcaps\">I ARRIVE</span> in the kitchen just in time.</p>\n<p class=\"indent\">You can watch one Bing.</p></section></body></html>";
        let (xhtml, _, _) =
            rebuild_chapter_xhtml(raw, &blocks, EpubPackMode::Chinese, "s.css", "Text")
                .expect("rebuild");
        // 译文各就各位，原文不残留
        assert!(
            xhtml.contains("我走进厨房时，正好赶上。"),
            "zh placed: {xhtml}"
        );
        assert!(
            xhtml.contains("你可以看一集《小兔兵兵》。"),
            "zh placed: {xhtml}"
        );
        assert!(
            !xhtml.contains("I ARRIVE in the kitchen"),
            "no english residue: {xhtml}"
        );
        assert!(
            !xhtml.contains("You can watch one Bing."),
            "no english residue: {xhtml}"
        );
        // 分隔线渲染为 hr（原 secbreak 保留 + mt-sep 至多一个，不重复堆 72 个 -）
        assert!(
            !xhtml.contains(
                "------------------------------------------------------------------------"
            ),
            "no raw dashes: {xhtml}"
        );
    }

    #[test]
    fn smallcaps_inline_tag_dropped_when_replacing_content() {
        // 段首 <span class="smallcaps">I ARRIVE</span> 这类内联格式标签：
        // 替换段内容时必须连壳丢弃，否则英文壳残留、中文接在后面（半英半中）。
        let b0 = make_translated(
            0,
            "I ARRIVE in the kitchen just in time.",
            "我走进厨房时，正好赶上。",
        );
        let blocks: Vec<&TranslatedBlock> = vec![&b0];
        let raw = "<html><head><title>t</title></head><body><p class=\"noindent\"><span class=\"smallcaps\">I ARRIVE</span> in the kitchen just in time.</p></body></html>";
        let (xhtml, _, _) =
            rebuild_chapter_xhtml(raw, &blocks, EpubPackMode::Chinese, "s.css", "Text")
                .expect("rebuild");
        assert!(xhtml.contains("我走进厨房时，正好赶上。"), "{xhtml}");
        assert!(
            !xhtml.contains("smallcaps"),
            "inline tag shell dropped: {xhtml}"
        );
        assert!(!xhtml.contains("I ARRIVE"), "no english shell: {xhtml}");
        assert!(xhtml.contains("noindent"), "outer class kept: {xhtml}");
    }

    #[test]
    fn nav_titles_translated_by_href() {
        let nav = r#"<nav epub:type="toc"><ol><li><a href="ch01.xhtml">Chapter 1: Tash</a></li><li><a href="ch02.xhtml">Chapter 2</a></li></ol></nav>"#;
        let mut map = HashMap::new();
        map.insert(
            normalize_href_key("Text/ch01.xhtml"),
            "第1章：塔什".to_string(),
        );
        let out = translate_nav_titles(nav, "Text", &map);
        assert!(out.contains("第1章：塔什"));
        assert!(out.contains("Chapter 2")); // 无映射保留原文
    }

    #[test]
    fn ncx_titles_translated_by_content_src() {
        // ncx 的 content src 是相对 ncx 所在目录；测试里 ncx 与章节同目录（base 传空）。
        let ncx = r#"<navPoint><navLabel><text>Chapter 1</text></navLabel><content src="Text/ch01.xhtml"/></navPoint>"#;
        let mut map = HashMap::new();
        map.insert(normalize_href_key("Text/ch01.xhtml"), "第1章".to_string());
        let out = translate_ncx_titles(ncx, "", &map);
        assert!(out.contains("第1章"), "got: {out}");
    }

    #[test]
    fn opf_rewrite_registers_stylesheet_and_title() {
        let opf = r#"<package><metadata><dc:title>Old</dc:title></metadata><manifest><item id="c" href="Text/c.xhtml" media-type="application/xhtml+xml"/></manifest></package>"#;
        let out = rewrite_opf(
            opf,
            "musetranslate-style.css",
            "My Book",
            EpubPackMode::Bilingual,
        );
        assert!(out.contains("My Book（中英对照）"));
        assert!(out.contains("musetranslate-style.css"));
        assert!(out.contains("text/css"));
    }

    #[test]
    fn opf_rewrite_is_idempotent_for_stylesheet() {
        let opf = r#"<package><manifest><item id="musetranslate-style" href="musetranslate-style.css" media-type="text/css"/></manifest></package>"#;
        let out = rewrite_opf(opf, "musetranslate-style.css", "T", EpubPackMode::Chinese);
        assert_eq!(out.matches("musetranslate-style.css").count(), 1);
    }
}
