use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

mod epub;
mod html;
mod pdf_outline;

pub mod docx;

#[allow(unused_imports)] // 打包器经此 re-export 供 pipeline_tests 端到端使用
pub use epub::pack;

const PDF_TABLE_START_MARKER: &str = "<!-- musetranslate:pdf-table:start -->";
const PDF_TABLE_END_MARKER: &str = "<!-- musetranslate:pdf-table:end -->";
const FRONT_MATTER_PROTECTED_START_MARKER: &str = "<!-- musetranslate:front-matter:start -->";
const FRONT_MATTER_PROTECTED_END_MARKER: &str = "<!-- musetranslate:front-matter:end -->";

pub use epub::reader::{
    prepare_epub_reader_package_with_overrides, search_epub_reader, EpubReaderPackage,
    EpubSearchResult,
};
pub use epub::EpubChapter;

/// 网页抓取复用的清洗能力（commands/web_capture）：
/// - `strip_web_boilerplate`：结构性残渣剥离（EPUB 管线同源算法）
/// - `html_to_plain_markdown`：HTML → 纯文本段落（段落间空行）
pub use epub::strip_web_boilerplate_for_capture;
pub(crate) use html::extract_text_from_html as html_to_plain_markdown;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDocumentKind {
    Pdf,
    Epub,
    Markdown,
    Text,
    Docx,
}

#[derive(Debug, Clone)]
pub struct SourceDocument {
    pub markdown: String,
    pub cover_data_url: Option<String>,
    pub markdown_images: HashMap<String, String>,
    pub toc: Option<TocArtifact>,
    pub epub_blocks: Vec<EpubBlock>,
    pub source_blocks: Vec<SourceBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubBlock {
    pub id: String,
    pub chapter_order: usize,
    pub chapter_title: String,
    pub href: Option<String>,
    pub order: usize,
    pub kind: String,
    pub source_html: String,
    pub source_markdown: String,
    pub text: String,
    pub translate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceBlock {
    pub id: String,
    pub order: usize,
    pub kind: String,
    pub markdown: String,
    pub text: String,
    pub translate: bool,
    #[serde(default)]
    pub heading_path: Vec<String>,
    #[serde(default)]
    pub chapter_title: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedBlock {
    pub id: String,
    pub order: usize,
    #[serde(default)]
    pub chunk_index: Option<usize>,
    #[serde(default)]
    pub marker_id: Option<String>,
    #[serde(default)]
    pub alignment_method: String,
    pub kind: String,
    pub source_markdown: String,
    pub translated_markdown: String,
    pub source_text: String,
    pub translated_text: String,
    pub translate: bool,
    #[serde(default)]
    pub heading_path: Vec<String>,
    #[serde(default)]
    pub chapter_title: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocArtifact {
    pub source_kind: TocSourceKind,
    pub confidence: f32,
    pub entries: Vec<TocEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TocSourceKind {
    EpubNcx,
    EpubNav,
    EpubHtmlTitle,
    EpubSpineFallback,
    PdfOutline,
    HeadingInferred,
    LayoutInferred,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocEntry {
    pub source_title: String,
    pub translated_title: Option<String>,
    pub level: u8,
    pub order: usize,
    pub page: Option<usize>,
    pub href: Option<String>,
    pub source_kind: TocSourceKind,
    pub confidence: f32,
}

pub fn detect_document_kind(path: &Path) -> Result<SourceDocumentKind, AppError> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("pdf") => Ok(SourceDocumentKind::Pdf),
        Some(ext) if ext.eq_ignore_ascii_case("epub") => Ok(SourceDocumentKind::Epub),
        Some(ext) if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") => {
            Ok(SourceDocumentKind::Markdown)
        }
        Some(ext) if ext.eq_ignore_ascii_case("txt") => Ok(SourceDocumentKind::Text),
        Some(ext) if ext.eq_ignore_ascii_case("docx") => Ok(SourceDocumentKind::Docx),
        _ => Err(AppError::invalid_input(
            "仅支持 PDF、EPUB、DOCX、Markdown (md) 或纯文本 (txt) 文件",
        )),
    }
}

pub fn extract_epub_document(path: &Path) -> Result<SourceDocument, AppError> {
    let extracted = epub::extract_epub_contents(path)?;
    let toc = epub_toc_artifact(&extracted.chapters);
    let epub_blocks = extracted
        .chapters
        .iter()
        .flat_map(|chapter| chapter.blocks.clone())
        .collect::<Vec<_>>();
    let sections = extracted
        .chapters
        .into_iter()
        .map(format_epub_chapter_markdown)
        .collect::<Vec<_>>();
    let markdown = sections.join("\n\n");

    Ok(SourceDocument {
        source_blocks: build_source_blocks_from_markdown(&markdown),
        markdown,
        cover_data_url: extracted.cover_data_url,
        markdown_images: extracted.markdown_images,
        toc,
        epub_blocks,
    })
}

fn format_epub_chapter_markdown(chapter: EpubChapter) -> String {
    let markdown = chapter.markdown.trim();
    let title = chapter.title.trim();
    if title.is_empty() {
        return markdown.to_string();
    }
    if chapter.title_source == TocSourceKind::EpubSpineFallback {
        return markdown.to_string();
    }
    if chapter_markdown_starts_with_heading_title(markdown, title) {
        return markdown.to_string();
    }
    if chapter_markdown_starts_with_plain_title(markdown, title) {
        return promote_epub_plain_title_to_heading(markdown, title);
    }
    format!("# {title}\n\n{markdown}")
}

fn chapter_markdown_starts_with_heading_title(markdown: &str, title: &str) -> bool {
    let Some(first_line) = markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return false;
    };
    if !first_line.starts_with('#') {
        return false;
    }
    let normalized_line = first_line.trim_start_matches('#').trim();
    normalized_line.eq_ignore_ascii_case(title)
}

fn chapter_markdown_starts_with_plain_title(markdown: &str, title: &str) -> bool {
    let Some(first_line) = markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return false;
    };
    !first_line.starts_with('#') && first_line.eq_ignore_ascii_case(title)
}

fn promote_epub_plain_title_to_heading(markdown: &str, title: &str) -> String {
    let mut promoted = Vec::new();
    let mut title_promoted = false;
    for line in markdown.lines() {
        if !title_promoted && line.trim().eq_ignore_ascii_case(title) {
            promoted.push(format!("# {}", title.trim()));
            title_promoted = true;
        } else {
            promoted.push(line.to_string());
        }
    }
    promoted.join("\n")
}

pub fn build_source_blocks_from_markdown(markdown: &str) -> Vec<SourceBlock> {
    let mut heading_path = Vec::<String>::new();
    split_markdown_blocks(markdown)
        .into_iter()
        .enumerate()
        .map(|(order, block)| {
            let kind = classify_markdown_block(&block).to_string();
            if kind == "heading" {
                update_heading_path(&mut heading_path, &block);
            }
            SourceBlock {
                id: format!("block-{order:04}"),
                order,
                kind,
                markdown: block.clone(),
                text: plain_text_from_block(&block),
                translate: block_is_translatable(&block),
                heading_path: heading_path.clone(),
                chapter_title: heading_path.last().cloned(),
                href: None,
                page: None,
            }
        })
        .collect()
}

#[cfg(test)]
pub fn build_translated_blocks(
    source_blocks: &[SourceBlock],
    translated_markdown: &str,
) -> Vec<TranslatedBlock> {
    build_translated_blocks_with_chunk_indexes(source_blocks, translated_markdown, &[])
}

#[cfg(test)]
pub fn build_translated_blocks_with_chunk_indexes(
    source_blocks: &[SourceBlock],
    translated_markdown: &str,
    source_block_chunk_indexes: &[Option<usize>],
) -> Vec<TranslatedBlock> {
    let translated_block_lookup =
        global_translated_block_lookup(source_blocks, translated_markdown);
    let source_block_marker_ids =
        source_block_marker_ids(source_blocks, source_block_chunk_indexes);
    let alignment_methods =
        default_alignment_methods(source_blocks.len(), &translated_block_lookup, "global");
    translated_blocks_from_lookup(
        source_blocks,
        source_block_chunk_indexes,
        &source_block_marker_ids,
        &alignment_methods,
        &translated_block_lookup,
    )
}

pub fn build_translated_blocks_with_chunk_markdowns(
    source_blocks: &[SourceBlock],
    translated_markdown: &str,
    source_block_chunk_indexes: &[Option<usize>],
    translated_chunk_markdowns: &[(usize, String)],
) -> Vec<TranslatedBlock> {
    let mut translated_block_lookup =
        global_translated_block_lookup(source_blocks, translated_markdown);
    let source_block_marker_ids =
        source_block_marker_ids(source_blocks, source_block_chunk_indexes);
    let mut alignment_methods =
        default_alignment_methods(source_blocks.len(), &translated_block_lookup, "global");
    apply_chunk_local_translated_block_lookup(
        &mut translated_block_lookup,
        &mut alignment_methods,
        source_blocks,
        source_block_chunk_indexes,
        translated_chunk_markdowns,
    );
    translated_blocks_from_lookup(
        source_blocks,
        source_block_chunk_indexes,
        &source_block_marker_ids,
        &alignment_methods,
        &translated_block_lookup,
    )
}

fn global_translated_block_lookup(
    source_blocks: &[SourceBlock],
    translated_markdown: &str,
) -> Vec<Option<SourceBlock>> {
    let translated_source_blocks = normalize_translated_pdf_table_blocks(
        build_source_blocks_from_markdown(translated_markdown),
    );
    let protected_source_blocks = explicit_front_matter_source_block_flags(source_blocks);
    let visible_source_blocks = source_blocks
        .iter()
        .zip(protected_source_blocks.iter())
        .filter_map(|(block, protected)| (!protected).then_some(block.clone()))
        .collect::<Vec<_>>();
    let translated_index_map =
        align_translated_block_indexes(&visible_source_blocks, &translated_source_blocks);
    let mut visible_index_by_source_index = Vec::with_capacity(source_blocks.len());
    let mut visible_index = 0usize;
    for protected in &protected_source_blocks {
        if *protected {
            visible_index_by_source_index.push(None);
        } else {
            visible_index_by_source_index.push(Some(visible_index));
            visible_index += 1;
        }
    }
    source_blocks
        .iter()
        .enumerate()
        .map(|(index, _source_block)| {
            visible_index_by_source_index
                .get(index)
                .and_then(|visible_index| *visible_index)
                .and_then(|visible_index| translated_index_map.get(visible_index))
                .and_then(|translated_index| translated_source_blocks.get(*translated_index))
                .cloned()
        })
        .collect()
}

fn apply_chunk_local_translated_block_lookup(
    translated_block_lookup: &mut [Option<SourceBlock>],
    alignment_methods: &mut [String],
    source_blocks: &[SourceBlock],
    source_block_chunk_indexes: &[Option<usize>],
    translated_chunk_markdowns: &[(usize, String)],
) {
    if translated_chunk_markdowns.is_empty() || source_block_chunk_indexes.is_empty() {
        return;
    }

    let protected_source_blocks = explicit_front_matter_source_block_flags(source_blocks);
    let translated_by_chunk = translated_chunk_markdowns
        .iter()
        .map(|(index, markdown)| (*index, markdown.as_str()))
        .collect::<HashMap<_, _>>();
    let mut source_indexes_by_chunk = BTreeMap::<usize, Vec<usize>>::new();

    for (source_index, chunk_index) in source_block_chunk_indexes.iter().enumerate() {
        if protected_source_blocks
            .get(source_index)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let Some(chunk_index) = chunk_index else {
            continue;
        };
        if !translated_by_chunk.contains_key(chunk_index) {
            continue;
        }
        source_indexes_by_chunk
            .entry(*chunk_index)
            .or_default()
            .push(source_index);
    }

    for (chunk_index, source_indexes) in source_indexes_by_chunk {
        let Some(translated_markdown) = translated_by_chunk.get(&chunk_index) else {
            continue;
        };
        let translated_source_blocks = normalize_translated_pdf_table_blocks(
            build_source_blocks_from_markdown(translated_markdown),
        );
        if translated_source_blocks.is_empty() {
            continue;
        }
        let source_chunk_blocks = source_indexes
            .iter()
            .filter_map(|source_index| source_blocks.get(*source_index).cloned())
            .collect::<Vec<_>>();
        let translated_index_map =
            chunk_local_translated_index_map(&source_chunk_blocks, &translated_source_blocks);
        for (local_index, source_index) in source_indexes.iter().copied().enumerate() {
            let translated_block = translated_index_map
                .get(local_index)
                .and_then(|translated_index| translated_source_blocks.get(*translated_index))
                .cloned();
            if let Some(slot) = translated_block_lookup.get_mut(source_index) {
                *slot = translated_block;
            }
            if let Some(method) = alignment_methods.get_mut(source_index) {
                *method = "chunkLocalMarkerOrder".to_string();
            }
        }
    }
}

fn chunk_local_translated_index_map(
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) -> Vec<usize> {
    if source_blocks.is_empty() || translated_blocks.is_empty() {
        return Vec::new();
    }
    if source_blocks.len() <= translated_blocks.len() {
        return default_translated_index_map(source_blocks, translated_blocks);
    }

    let last_translated_index = translated_blocks.len().saturating_sub(1);
    let mut translated_index = 0usize;
    let mut mapping = Vec::with_capacity(source_blocks.len());
    for (source_index, source_block) in source_blocks.iter().enumerate() {
        if source_index == 0 {
            mapping.push(translated_index);
            continue;
        }
        let previous_source = &source_blocks[source_index - 1];
        if !is_continuation_source_block(previous_source, source_block) {
            translated_index = translated_index
                .saturating_add(1)
                .min(last_translated_index);
        }
        mapping.push(translated_index);
    }
    mapping
}

fn is_continuation_source_block(previous: &SourceBlock, current: &SourceBlock) -> bool {
    if previous.kind != "paragraph" || current.kind != "paragraph" {
        return false;
    }
    let previous_text = previous.text.trim();
    let current_text = current.text.trim_start();
    if previous_text.is_empty() || current_text.is_empty() {
        return false;
    }
    !ends_with_sentence_boundary(previous_text) && starts_like_sentence_continuation(current_text)
}

fn ends_with_sentence_boundary(text: &str) -> bool {
    text.chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_some_and(|ch| {
            matches!(
                ch,
                '.' | '?' | '!' | ';' | ':' | '。' | '？' | '！' | '；' | '：'
            )
        })
}

fn starts_like_sentence_continuation(text: &str) -> bool {
    let Some(first) = text.chars().find(|ch| !ch.is_whitespace()) else {
        return false;
    };
    first.is_ascii_lowercase() || matches!(first, ',' | ';' | ':' | ')' | ']' | '}')
}

fn source_block_marker_ids(
    source_blocks: &[SourceBlock],
    source_block_chunk_indexes: &[Option<usize>],
) -> Vec<Option<String>> {
    if source_block_chunk_indexes.is_empty() {
        return vec![None; source_blocks.len()];
    }
    let protected_source_blocks = explicit_front_matter_source_block_flags(source_blocks);
    let mut next_by_chunk = BTreeMap::<usize, usize>::new();
    source_block_chunk_indexes
        .iter()
        .enumerate()
        .map(|(source_index, chunk_index)| {
            if protected_source_blocks
                .get(source_index)
                .copied()
                .unwrap_or(false)
            {
                return None;
            }
            let chunk_index = (*chunk_index)?;
            let next = next_by_chunk.entry(chunk_index).or_insert(0);
            let marker_id = format!("B{next:03}");
            *next += 1;
            Some(marker_id)
        })
        .collect()
}

fn default_alignment_methods(
    len: usize,
    translated_block_lookup: &[Option<SourceBlock>],
    method: &str,
) -> Vec<String> {
    (0..len)
        .map(|index| {
            if translated_block_lookup
                .get(index)
                .and_then(Option::as_ref)
                .is_some()
            {
                method.to_string()
            } else {
                "none".to_string()
            }
        })
        .collect()
}

fn translated_blocks_from_lookup(
    source_blocks: &[SourceBlock],
    source_block_chunk_indexes: &[Option<usize>],
    source_block_marker_ids: &[Option<String>],
    alignment_methods: &[String],
    translated_block_lookup: &[Option<SourceBlock>],
) -> Vec<TranslatedBlock> {
    source_blocks
        .iter()
        .enumerate()
        .map(|(index, source_block)| {
            let translated_block = translated_block_lookup.get(index).and_then(Option::as_ref);
            let structure_rejected = translated_block.is_some_and(|block| {
                !translated_block_matches_source_structure(source_block, block)
            });
            let translated_block = if structure_rejected {
                None
            } else {
                translated_block
            };
            TranslatedBlock {
                id: source_block.id.clone(),
                order: source_block.order,
                chunk_index: source_block_chunk_indexes.get(index).copied().flatten(),
                marker_id: source_block_marker_ids.get(index).cloned().flatten(),
                alignment_method: if structure_rejected {
                    "structureRejected".to_string()
                } else {
                    alignment_methods.get(index).cloned().unwrap_or_default()
                },
                kind: source_block.kind.clone(),
                source_markdown: source_block.markdown.clone(),
                translated_markdown: translated_block
                    .map(|block| block.markdown.clone())
                    .unwrap_or_default(),
                source_text: source_block.text.clone(),
                translated_text: translated_block
                    .map(|block| block.text.clone())
                    .unwrap_or_default(),
                translate: source_block.translate,
                heading_path: source_block.heading_path.clone(),
                chapter_title: source_block.chapter_title.clone(),
                href: source_block.href.clone(),
                page: source_block.page,
            }
        })
        .collect()
}

fn translated_block_matches_source_structure(
    source_block: &SourceBlock,
    translated_block: &SourceBlock,
) -> bool {
    if source_block.kind != "heading" {
        return true;
    }
    translated_block.kind == "heading"
        && translated_heading_text_is_plausible(&source_block.text, &translated_block.text)
}

fn translated_heading_text_is_plausible(source_title: &str, translated_title: &str) -> bool {
    let title = translated_title.trim();
    if title.is_empty() || title.lines().filter(|line| !line.trim().is_empty()).count() > 1 {
        return false;
    }
    let char_count = title.chars().filter(|ch| !ch.is_whitespace()).count();
    if char_count == 0 || char_count > 160 {
        return false;
    }
    let source_char_count = source_title
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count()
        .max(1);
    if sentence_terminator_count(title) >= 2 && char_count > 32 {
        return false;
    }
    let relative_limit = source_char_count
        .saturating_mul(2)
        .saturating_add(18)
        .max(48);
    if heading_ends_with_sentence_terminator(title)
        && char_count > relative_limit
        && contains_clause_punctuation(title)
    {
        return false;
    }
    !(char_count > source_char_count.saturating_mul(5).saturating_add(32) && char_count > 96)
}

fn sentence_terminator_count(title: &str) -> usize {
    let strong_count = title
        .chars()
        .filter(|ch| matches!(ch, '。' | '！' | '？' | '!' | '?'))
        .count();
    strong_count + ascii_sentence_period_count(title)
}

fn ascii_sentence_period_count(title: &str) -> usize {
    title
        .char_indices()
        .filter(|(index, ch)| {
            if *ch != '.' {
                return false;
            }
            title
                .get(index + ch.len_utf8()..)
                .and_then(|rest| rest.chars().next())
                .is_none_or(char::is_whitespace)
        })
        .count()
}

fn heading_ends_with_sentence_terminator(title: &str) -> bool {
    title
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '。' | '！' | '？' | '!' | '?' | '.'))
}

fn contains_clause_punctuation(title: &str) -> bool {
    title
        .chars()
        .any(|ch| matches!(ch, ',' | '，' | ';' | '；'))
}

fn explicit_front_matter_source_block_flags(source_blocks: &[SourceBlock]) -> Vec<bool> {
    let mut flags = Vec::with_capacity(source_blocks.len());
    let mut inside_front_matter = false;
    for block in source_blocks {
        let starts_here = block.markdown.contains(FRONT_MATTER_PROTECTED_START_MARKER);
        let ends_here = block.markdown.contains(FRONT_MATTER_PROTECTED_END_MARKER);
        let protected = inside_front_matter || starts_here || ends_here;
        flags.push(protected);
        if starts_here && !ends_here {
            inside_front_matter = true;
        }
        if ends_here {
            inside_front_matter = false;
        }
    }
    flags
}

fn align_translated_block_indexes(
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) -> Vec<usize> {
    if source_blocks.is_empty() || translated_blocks.is_empty() {
        return (0..source_blocks.len()).collect();
    }

    if let Some(mapping) = anchor_heading_alignment_map(source_blocks, translated_blocks) {
        return mapping;
    }

    let source_heading_indexes = heading_indexes(source_blocks);
    let translated_heading_indexes = heading_indexes(translated_blocks);
    if source_heading_indexes.len() != translated_heading_indexes.len() {
        return default_translated_index_map(source_blocks, translated_blocks);
    }

    let mut mapping = vec![0usize; source_blocks.len()];
    let mut source_section_start = 0usize;
    let mut translated_section_start = 0usize;

    for (source_heading_index, translated_heading_index) in source_heading_indexes
        .iter()
        .copied()
        .zip(translated_heading_indexes.iter().copied())
    {
        map_section_from_start(
            &mut mapping,
            source_section_start,
            source_heading_index,
            translated_section_start,
            translated_heading_index,
            translated_blocks.len(),
        );
        if source_heading_index < mapping.len() {
            mapping[source_heading_index] =
                translated_heading_index.min(translated_blocks.len() - 1);
        }
        source_section_start = source_heading_index + 1;
        translated_section_start = translated_heading_index + 1;
    }

    map_section_from_start(
        &mut mapping,
        source_section_start,
        source_blocks.len(),
        translated_section_start,
        translated_blocks.len(),
        translated_blocks.len(),
    );

    refine_mapping_with_section_ascii_anchors(&mut mapping, source_blocks, translated_blocks);
    refine_mapping_with_local_similarity(&mut mapping, source_blocks, translated_blocks);

    mapping
}

fn anchor_heading_alignment_map(
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) -> Option<Vec<usize>> {
    let source_heading_indexes = heading_indexes(source_blocks);
    let translated_heading_indexes = heading_indexes(translated_blocks);
    if source_heading_indexes.is_empty() || translated_heading_indexes.is_empty() {
        return None;
    }

    let mut aligned_headings = Vec::new();
    let mut translated_cursor = 0usize;
    for source_index in source_heading_indexes.iter().copied() {
        let source_heading = &source_blocks[source_index];
        let source_anchor = heading_anchor_key(&source_heading.text);
        let matched = translated_heading_indexes
            .iter()
            .copied()
            .find(|candidate| {
                *candidate >= translated_cursor
                    && heading_alignment_anchor_match(
                        &source_anchor,
                        &heading_anchor_key(&translated_blocks[*candidate].text),
                    )
            });
        let Some(matched_index) = matched else {
            continue;
        };
        if matched_index < translated_cursor {
            continue;
        }
        aligned_headings.push((source_index, matched_index));
        translated_cursor = matched_index + 1;
    }

    if aligned_headings.is_empty() {
        return None;
    }

    let mut mapping = vec![0usize; source_blocks.len()];
    let mut source_section_start = 0usize;
    let mut translated_section_start = 0usize;
    for (source_heading_index, translated_heading_index) in aligned_headings.iter().copied() {
        map_section_from_start(
            &mut mapping,
            source_section_start,
            source_heading_index,
            translated_section_start,
            translated_heading_index,
            translated_blocks.len(),
        );
        mapping[source_heading_index] = translated_heading_index;
        source_section_start = source_heading_index + 1;
        translated_section_start = translated_heading_index + 1;
    }
    map_section_from_start(
        &mut mapping,
        source_section_start,
        source_blocks.len(),
        translated_section_start,
        translated_blocks.len(),
        translated_blocks.len(),
    );
    tighten_front_matter_to_first_heading_alignment(&mut mapping, source_blocks, translated_blocks);
    tighten_tail_to_last_heading_alignment(&mut mapping, source_blocks, translated_blocks);
    refine_mapping_with_section_ascii_anchors(&mut mapping, source_blocks, translated_blocks);
    refine_mapping_with_local_similarity(&mut mapping, source_blocks, translated_blocks);
    Some(mapping)
}

fn heading_indexes(blocks: &[SourceBlock]) -> Vec<usize> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| (block.kind == "heading").then_some(index))
        .collect()
}

fn heading_anchor_key(text: &str) -> String {
    normalize_alignment_text(text)
        .replace('：', ":")
        .replace('，', ",")
        .replace('（', "(")
        .replace('）', ")")
}

fn heading_alignment_anchor_match(source: &str, translated: &str) -> bool {
    if source.is_empty() || translated.is_empty() {
        return false;
    }
    if source == translated {
        return true;
    }
    let source_semantic = academic_heading_semantic_key(source);
    let translated_semantic = academic_heading_semantic_key(translated);
    source_semantic == translated_semantic
}

fn academic_heading_semantic_key(text: &str) -> String {
    let normalized = normalize_alignment_text(text);
    let stripped = normalized
        .trim_start_matches("abstract ")
        .trim_start_matches("keywords ")
        .trim_start_matches("acknowledgments ")
        .trim_start_matches("acknowledgements ")
        .trim_start_matches("references ");
    if matches!(normalized.as_str(), "abstract" | "摘要" | "摘 要") {
        return "abstract".to_string();
    }
    if matches!(normalized.as_str(), "keywords" | "关键字" | "关键词") {
        return "keywords".to_string();
    }
    if matches!(normalized.as_str(), "references" | "参考文献") {
        return "references".to_string();
    }
    if matches!(
        normalized.as_str(),
        "acknowledgments" | "acknowledgements" | "致谢"
    ) {
        return "acknowledgments".to_string();
    }
    stripped.to_string()
}

fn default_translated_index_map(
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) -> Vec<usize> {
    let last_index = translated_blocks.len().saturating_sub(1);
    (0..source_blocks.len())
        .map(|index| index.min(last_index))
        .collect()
}

fn map_section_from_start(
    mapping: &mut [usize],
    source_start: usize,
    source_end: usize,
    translated_start: usize,
    translated_end: usize,
    translated_len: usize,
) {
    if source_start >= source_end || translated_len == 0 {
        return;
    }
    let translated_last = translated_len.saturating_sub(1);
    for (offset, source_index) in (source_start..source_end).enumerate() {
        let translated_index = translated_start
            .saturating_add(offset)
            .min(translated_end.saturating_sub(1))
            .min(translated_last);
        mapping[source_index] = translated_index;
    }
}

fn tighten_front_matter_to_first_heading_alignment(
    mapping: &mut [usize],
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) {
    let Some(first_heading_index) = source_blocks
        .iter()
        .position(|block| block.kind == "heading")
    else {
        return;
    };
    let Some(first_translated_heading_index) = mapping.get(first_heading_index).copied() else {
        return;
    };
    if first_heading_index == 0 || first_translated_heading_index == 0 {
        return;
    }

    let translated_limit =
        first_translated_heading_index.min(translated_blocks.len().saturating_sub(1));
    let mut last_assigned = 0usize;
    for source_index in 0..first_heading_index {
        let source_kind = source_blocks[source_index].kind.as_str();
        let source_text = source_blocks[source_index].text.trim();
        if source_text.is_empty() {
            continue;
        }
        let remaining_source_blocks = first_heading_index.saturating_sub(source_index + 1);
        let translated_upper_bound = translated_limit.saturating_sub(remaining_source_blocks);
        let current = mapping[source_index]
            .min(translated_upper_bound)
            .max(last_assigned);
        let mut best_index = current;
        let mut best_score =
            front_matter_alignment_score(source_kind, source_text, &translated_blocks[current]);
        let search_start = last_assigned;
        let search_end = translated_upper_bound;
        for candidate in search_start..=search_end {
            let score = front_matter_alignment_score(
                source_kind,
                source_text,
                &translated_blocks[candidate],
            );
            if score > best_score {
                best_score = score;
                best_index = candidate;
            }
        }
        mapping[source_index] = best_index;
        last_assigned = best_index;
    }
}

fn tighten_tail_to_last_heading_alignment(
    mapping: &mut [usize],
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) {
    let Some(last_heading_index) = source_blocks
        .iter()
        .rposition(|block| block.kind == "heading")
    else {
        return;
    };
    let Some(last_translated_heading_index) = mapping.get(last_heading_index).copied() else {
        return;
    };
    if last_heading_index + 1 >= source_blocks.len()
        || last_translated_heading_index + 1 >= translated_blocks.len()
    {
        return;
    }

    let mut translated_cursor = last_translated_heading_index + 1;
    let translated_last = translated_blocks.len().saturating_sub(1);
    for source_index in (last_heading_index + 1)..source_blocks.len() {
        let source_kind = source_blocks[source_index].kind.as_str();
        let source_text = source_blocks[source_index].text.trim();
        if source_text.is_empty() {
            mapping[source_index] = translated_cursor.min(translated_last);
            continue;
        }
        let current = mapping[source_index]
            .max(translated_cursor)
            .min(translated_last);
        let mut best_index = current;
        let mut best_score =
            block_alignment_score(source_kind, source_text, &translated_blocks[current]);
        let search_end = (translated_cursor + 3).min(translated_last);
        for candidate in translated_cursor..=search_end {
            let score =
                block_alignment_score(source_kind, source_text, &translated_blocks[candidate]);
            if score > best_score {
                best_score = score;
                best_index = candidate;
            }
        }
        mapping[source_index] = best_index;
        translated_cursor = best_index.saturating_add(1).min(translated_last);
    }
}

fn front_matter_alignment_score(
    source_kind: &str,
    source_text: &str,
    translated_block: &SourceBlock,
) -> i32 {
    let mut score = block_alignment_score(source_kind, source_text, translated_block);
    let normalized_source = normalize_alignment_text(source_text);
    let normalized_translated = normalize_alignment_text(translated_block.text.trim());
    if looks_like_front_matter_metadata(&normalized_source)
        == looks_like_front_matter_metadata(&normalized_translated)
    {
        score += 5;
    }
    if looks_like_author_line(&normalized_source) == looks_like_author_line(&normalized_translated)
    {
        score += 4;
    }
    score
}

fn looks_like_front_matter_metadata(text: &str) -> bool {
    text.contains("received")
        || text.contains("accepted")
        || text.contains("published")
        || text.contains("copyright")
        || text.contains("correspondence")
        || text.contains("e-mail")
        || text.contains("email")
        || text.contains("通讯")
        || text.contains("收稿")
        || text.contains("作者")
}

fn looks_like_author_line(text: &str) -> bool {
    let token_count = text.split_whitespace().count();
    token_count <= 12
        && (text.contains(" and ") || text.contains(',') || text.contains('*'))
        && !looks_like_front_matter_metadata(text)
}

fn refine_mapping_with_local_similarity(
    mapping: &mut [usize],
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) {
    if source_blocks.is_empty() || translated_blocks.is_empty() {
        return;
    }
    let translated_last = translated_blocks.len().saturating_sub(1);
    for source_index in 0..source_blocks.len() {
        let current = mapping[source_index].min(translated_last);
        let source_text = source_blocks[source_index].text.trim();
        if source_text.is_empty() {
            continue;
        }
        let mut best_index = current;
        let mut best_score = block_alignment_score(
            source_blocks[source_index].kind.as_str(),
            source_text,
            &translated_blocks[current],
        );
        let search_start = current.saturating_sub(2);
        let search_end = (current + 2).min(translated_last);
        for candidate in search_start..=search_end {
            let score = block_alignment_score(
                source_blocks[source_index].kind.as_str(),
                source_text,
                &translated_blocks[candidate],
            );
            if score > best_score {
                best_score = score;
                best_index = candidate;
            }
        }
        mapping[source_index] = best_index;
    }
}

fn refine_mapping_with_section_ascii_anchors(
    mapping: &mut [usize],
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
) {
    if source_blocks.is_empty() || translated_blocks.is_empty() {
        return;
    }

    let mut section_start = 0usize;
    for source_index in 0..=source_blocks.len() {
        let is_boundary =
            source_index == source_blocks.len() || source_blocks[source_index].kind == "heading";
        if !is_boundary {
            continue;
        }
        refine_section_ascii_anchors(
            mapping,
            source_blocks,
            translated_blocks,
            section_start,
            source_index,
        );
        section_start = source_index.saturating_add(1);
    }
}

fn refine_section_ascii_anchors(
    mapping: &mut [usize],
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
    source_start: usize,
    source_end: usize,
) {
    if source_start >= source_end {
        return;
    }

    let translated_start = if source_start > 0 {
        mapping[source_start - 1].saturating_add(1)
    } else {
        0
    };
    let translated_end = if source_end < source_blocks.len() {
        mapping[source_end].min(translated_blocks.len())
    } else {
        translated_blocks.len()
    };
    if translated_start >= translated_blocks.len() || translated_start >= translated_end {
        return;
    }

    let anchors = collect_section_ascii_anchor_pairs(
        source_blocks,
        translated_blocks,
        source_start,
        source_end,
        translated_start,
        translated_end,
    );
    for (anchor_source, anchor_translated) in anchors {
        mapping[anchor_source] = anchor_translated;
    }
}

fn collect_section_ascii_anchor_pairs(
    source_blocks: &[SourceBlock],
    translated_blocks: &[SourceBlock],
    source_start: usize,
    source_end: usize,
    translated_start: usize,
    translated_end: usize,
) -> Vec<(usize, usize)> {
    let mut anchors = Vec::new();
    let mut used_translated = std::collections::BTreeSet::<usize>::new();

    for source_index in source_start..source_end {
        let source_text = source_blocks[source_index].text.trim();
        if source_text.is_empty() {
            continue;
        }
        let Some((translated_index, _score)) = best_unique_ascii_anchor_match(
            source_index,
            source_text,
            translated_blocks,
            translated_start,
            translated_end,
            &used_translated,
        ) else {
            continue;
        };
        anchors.push((source_index, translated_index));
        used_translated.insert(translated_index);
    }

    anchors
}

fn best_unique_ascii_anchor_match(
    source_index: usize,
    source_text: &str,
    translated_blocks: &[SourceBlock],
    translated_start: usize,
    translated_end: usize,
    used_translated: &std::collections::BTreeSet<usize>,
) -> Option<(usize, usize)> {
    let tokens = collect_distinctive_ascii_tokens(source_text);
    if tokens.is_empty() {
        return None;
    }

    let mut best: Option<(usize, usize)> = None;
    let mut tie = false;
    for translated_index in translated_start..translated_end.min(translated_blocks.len()) {
        if used_translated.contains(&translated_index) {
            continue;
        }
        let translated_text = translated_blocks[translated_index].text.trim();
        if translated_text.is_empty() {
            continue;
        }
        let score = ascii_anchor_overlap_score(&tokens, translated_text);
        if score == 0 {
            continue;
        }
        match best {
            None => {
                best = Some((translated_index, score));
                tie = false;
            }
            Some((_, best_score)) if score > best_score => {
                best = Some((translated_index, score));
                tie = false;
            }
            Some((_, best_score)) if score == best_score => {
                tie = true;
            }
            _ => {}
        }
    }

    let Some((translated_index, score)) = best else {
        return None;
    };
    if tie || !ascii_anchor_score_is_strong_enough(source_index, source_text, score) {
        return None;
    }
    Some((translated_index, score))
}

fn ascii_anchor_score_is_strong_enough(
    source_index: usize,
    source_text: &str,
    score: usize,
) -> bool {
    if score >= 2 {
        return true;
    }
    score == 1
        && source_index > 0
        && collect_distinctive_ascii_tokens(source_text)
            .iter()
            .any(|token| token.len() >= 8)
}

fn collect_distinctive_ascii_tokens(text: &str) -> Vec<String> {
    let mut tokens = text
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '\'' | '.')))
        .filter_map(|token| {
            let normalized = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase();
            let has_alpha = normalized.chars().any(|ch| ch.is_ascii_alphabetic());
            (normalized.len() >= 4 && has_alpha).then_some(normalized)
        })
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn ascii_anchor_overlap_score(tokens: &[String], translated_text: &str) -> usize {
    let normalized = normalize_alignment_text(translated_text);
    tokens
        .iter()
        .filter(|token| contains_alignment_token(&normalized, token))
        .count()
}

fn contains_alignment_token(text: &str, token: &str) -> bool {
    let mut start = 0usize;
    while let Some(found) = text[start..].find(token) {
        let match_start = start + found;
        let match_end = match_start + token.len();
        let before_ok = text[..match_start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        let after_ok = text[match_end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = match_end;
    }
    false
}

fn block_alignment_score(
    source_kind: &str,
    source_text: &str,
    translated_block: &SourceBlock,
) -> i32 {
    let translated_text = translated_block.text.trim();
    let source_is_pdf_table = is_pdf_table_block_text(source_text);
    let translated_is_pdf_table = is_pdf_table_block_text(&translated_block.markdown);
    if source_is_pdf_table || translated_is_pdf_table {
        return match (source_is_pdf_table, translated_is_pdf_table) {
            (true, true) => 100,
            (true, false) | (false, true) => -100,
            (false, false) => 0,
        };
    }
    if translated_text.is_empty() {
        return 0;
    }
    let normalized_source = normalize_alignment_text(source_text);
    let normalized_translated = normalize_alignment_text(translated_text);
    let mut score = 0i32;
    if source_kind == translated_block.kind {
        score += 3;
    }
    if normalized_source == normalized_translated {
        score += 12;
    }
    if !normalized_source.is_empty()
        && !normalized_translated.is_empty()
        && (normalized_source.contains(&normalized_translated)
            || normalized_translated.contains(&normalized_source))
    {
        score += 8;
    }
    if first_ascii_token(&normalized_source)
        .is_some_and(|token| normalized_translated.starts_with(token))
    {
        score += 4;
    }
    score
}

fn normalize_translated_pdf_table_blocks(blocks: Vec<SourceBlock>) -> Vec<SourceBlock> {
    let mut normalized = Vec::with_capacity(blocks.len());
    let mut index = 0usize;
    while index < blocks.len() {
        if let Some(merged) = merge_split_pdf_table_block(&blocks, index) {
            normalized.push(merged);
            index += 2;
            continue;
        }
        normalized.push(blocks[index].clone());
        index += 1;
    }
    normalized
}

fn merge_split_pdf_table_block(blocks: &[SourceBlock], index: usize) -> Option<SourceBlock> {
    let image_block = blocks.get(index)?;
    let marker_block = blocks.get(index + 1)?;
    if image_block.kind != "image" || marker_block.kind != "paragraph" {
        return None;
    }
    let marker_trimmed = marker_block.markdown.trim();
    let image_trimmed = image_block.markdown.trim();
    if !looks_like_empty_pdf_table_marker_block(marker_trimmed)
        || !looks_like_pdf_table_image(image_trimmed)
    {
        return None;
    }

    let merged_markdown = format!(
        "{}\n{}\n{}",
        PDF_TABLE_START_MARKER, image_trimmed, PDF_TABLE_END_MARKER
    );
    let mut merged = marker_block.clone();
    merged.kind = "paragraph".to_string();
    merged.markdown = merged_markdown.clone();
    merged.text = plain_text_from_block(&merged_markdown);
    merged.translate = false;
    Some(merged)
}

fn looks_like_empty_pdf_table_marker_block(block: &str) -> bool {
    let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
    matches!(
        (lines.next(), lines.next(), lines.next()),
        (
            Some(PDF_TABLE_START_MARKER),
            Some(PDF_TABLE_END_MARKER),
            None
        )
    )
}

fn is_pdf_table_block_text(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.contains(PDF_TABLE_START_MARKER) && trimmed.contains(PDF_TABLE_END_MARKER)
}

fn looks_like_pdf_table_image(block: &str) -> bool {
    let trimmed = block.trim();
    (trimmed.starts_with("![PDF table ") || trimmed.starts_with("![PDF Table "))
        && trimmed.contains("table_snapshot_")
}

fn normalize_alignment_text(value: &str) -> String {
    value
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn first_ascii_token(value: &str) -> Option<&str> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .find(|token| !token.is_empty())
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn classify_markdown_block(block: &str) -> &'static str {
    let trimmed = block.trim();
    if trimmed.starts_with('#') {
        "heading"
    } else if trimmed.starts_with("![](") || trimmed.starts_with("![") {
        "image"
    } else if trimmed.starts_with(">") {
        "blockquote"
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("1. ") {
        "list"
    } else if trimmed.starts_with("```") {
        "code"
    } else if trimmed
        .lines()
        .all(|line| line.trim_start().starts_with('|'))
    {
        "table"
    } else {
        "paragraph"
    }
}

fn block_is_translatable(block: &str) -> bool {
    !matches!(classify_markdown_block(block), "image" | "code")
}

fn plain_text_from_block(block: &str) -> String {
    block
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches('#')
                .trim_start_matches('>')
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn update_heading_path(heading_path: &mut Vec<String>, block: &str) {
    let trimmed = block.trim();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count().max(1);
    let title = trimmed.trim_start_matches('#').trim();
    if title.is_empty() {
        return;
    }
    if heading_path.len() >= level {
        heading_path.truncate(level - 1);
    }
    heading_path.push(title.to_string());
}

pub fn extract_epub_chapters(path: &Path) -> Result<Vec<EpubChapter>, AppError> {
    Ok(epub::extract_epub_contents(path)?.chapters)
}

pub fn extract_pdf_outline(path: &Path) -> Result<Option<TocArtifact>, AppError> {
    pdf_outline::extract_pdf_outline(path)
}

fn epub_toc_artifact(chapters: &[EpubChapter]) -> Option<TocArtifact> {
    if chapters.is_empty() {
        return None;
    }
    let entries = chapters
        .iter()
        .map(|chapter| TocEntry {
            source_title: chapter.title.clone(),
            translated_title: None,
            level: 1,
            order: chapter.order,
            page: None,
            href: chapter.href.clone(),
            source_kind: chapter.title_source,
            confidence: toc_source_confidence(chapter.title_source),
        })
        .collect::<Vec<_>>();
    let source_kind = entries
        .iter()
        .map(|entry| entry.source_kind)
        .min_by_key(|kind| toc_source_rank(*kind))
        .unwrap_or(TocSourceKind::None);
    Some(TocArtifact {
        source_kind,
        confidence: toc_source_confidence(source_kind),
        entries,
    })
}

fn toc_source_rank(kind: TocSourceKind) -> u8 {
    match kind {
        TocSourceKind::EpubNcx => 0,
        TocSourceKind::EpubNav => 1,
        TocSourceKind::PdfOutline => 2,
        TocSourceKind::EpubHtmlTitle => 3,
        TocSourceKind::HeadingInferred => 4,
        TocSourceKind::LayoutInferred => 5,
        TocSourceKind::EpubSpineFallback => 6,
        TocSourceKind::None => 7,
    }
}

fn toc_source_confidence(kind: TocSourceKind) -> f32 {
    match kind {
        TocSourceKind::EpubNcx => 0.95,
        TocSourceKind::EpubNav => 0.9,
        TocSourceKind::PdfOutline => 0.9,
        TocSourceKind::EpubHtmlTitle => 0.8,
        TocSourceKind::HeadingInferred => 0.7,
        TocSourceKind::LayoutInferred => 0.6,
        TocSourceKind::EpubSpineFallback => 0.45,
        TocSourceKind::None => 0.0,
    }
}

pub fn extract_epub_chapter_by_title(
    path: &Path,
    chapter_title: &str,
) -> Result<EpubChapter, AppError> {
    let normalized_target = chapter_title.trim().to_lowercase();
    if normalized_target.is_empty() {
        return Err(AppError::invalid_input("EPUB 章节标题不能为空"));
    }

    extract_epub_chapters(path)?
        .into_iter()
        .find(|chapter| chapter.title.trim().to_lowercase() == normalized_target)
        .ok_or_else(|| AppError::not_found(format!("未找到 EPUB 章节: {chapter_title}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn epub_chapter_markdown_injects_missing_title() {
        let chapter = EpubChapter {
            title: "The Tale".to_string(),
            markdown: "Once upon a time.".to_string(),
            order: 0,
            href: Some("Text/chapter.xhtml".to_string()),
            title_source: TocSourceKind::EpubNcx,
            blocks: Vec::new(),
        };

        assert_eq!(
            format_epub_chapter_markdown(chapter),
            "# The Tale\n\nOnce upon a time."
        );
    }

    #[test]
    fn epub_chapter_markdown_does_not_duplicate_existing_heading() {
        let chapter = EpubChapter {
            title: "The Tale".to_string(),
            markdown: "# The Tale\n\nOnce upon a time.".to_string(),
            order: 0,
            href: Some("Text/chapter.xhtml".to_string()),
            title_source: TocSourceKind::EpubNcx,
            blocks: Vec::new(),
        };

        assert_eq!(
            format_epub_chapter_markdown(chapter),
            "# The Tale\n\nOnce upon a time."
        );
    }

    #[test]
    fn epub_chapter_markdown_promotes_plain_opening_title() {
        let chapter = EpubChapter {
            title: "The Tale".to_string(),
            markdown: "The Tale\n\nOnce upon a time.".to_string(),
            order: 0,
            href: Some("Text/chapter.xhtml".to_string()),
            title_source: TocSourceKind::EpubNcx,
            blocks: Vec::new(),
        };

        assert_eq!(
            format_epub_chapter_markdown(chapter),
            "# The Tale\n\nOnce upon a time."
        );
    }

    #[test]
    fn epub_chapter_markdown_injects_when_opening_title_differs() {
        let chapter = EpubChapter {
            title: "The Tale".to_string(),
            markdown: "Different opening\n\nOnce upon a time.".to_string(),
            order: 0,
            href: Some("Text/chapter.xhtml".to_string()),
            title_source: TocSourceKind::EpubNcx,
            blocks: Vec::new(),
        };

        assert_eq!(
            format_epub_chapter_markdown(chapter),
            "# The Tale\n\nDifferent opening\n\nOnce upon a time."
        );
    }

    #[test]
    fn epub_chapter_markdown_does_not_inject_spine_fallback_title() {
        let chapter = EpubChapter {
            title: "Chapter 1".to_string(),
            markdown: "![Cover](imgs/epub/Images/cover.jpeg)".to_string(),
            order: 0,
            href: Some("Text/titlepage.xhtml".to_string()),
            title_source: TocSourceKind::EpubSpineFallback,
            blocks: Vec::new(),
        };

        assert_eq!(
            format_epub_chapter_markdown(chapter),
            "![Cover](imgs/epub/Images/cover.jpeg)"
        );
    }

    #[test]
    fn chunk_local_translated_blocks_preserve_order_for_keyword_blocks() {
        let source_markdown = [
            "## Abstract",
            "Nudge plus is a modification of the behavioral public policy toolkit.",
            "Keywords: nudge; nudge plus; think; dual-process theory",
            "A nudge with reflection can seem contradictory at first.",
            "Nonetheless, recent work suggests nudge plus can be effective.",
        ]
        .join("\n\n");
        let translated_markdown = [
            "## 摘要",
            "助推升级版是对行为公共政策工具包的一种改进。",
            "关键词：助推；助推升级版；思考；双重过程理论",
            "融入反思的助推起初可能显得矛盾。",
            "尽管如此，近期研究表明助推升级版可能有效。",
        ]
        .join("\n\n");
        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let chunk_indexes = vec![Some(1); source_blocks.len()];

        let blocks = build_translated_blocks_with_chunk_markdowns(
            &source_blocks,
            &translated_markdown,
            &chunk_indexes,
            &[(1, translated_markdown.clone())],
        );

        assert_eq!(
            blocks[2].translated_markdown,
            "关键词：助推；助推升级版；思考；双重过程理论"
        );
        assert_eq!(blocks[2].marker_id.as_deref(), Some("B002"));
        assert_eq!(blocks[2].alignment_method, "chunkLocalMarkerOrder");
    }

    #[test]
    fn chunk_local_translated_blocks_absorb_source_sentence_continuations() {
        let source_markdown = [
            "The first source paragraph is split before its implicit",
            "support from later words and then ends as a complete sentence.",
            "There is more to cognitive processes than a sequential model.",
        ]
        .join("\n\n");
        let translated_markdown = [
            "第一个源段落及其后续文字被模型合并成一个完整译段。",
            "认知过程并不只是顺序模型。",
        ]
        .join("\n\n");
        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let chunk_indexes = vec![Some(6); source_blocks.len()];

        let blocks = build_translated_blocks_with_chunk_markdowns(
            &source_blocks,
            &translated_markdown,
            &chunk_indexes,
            &[(6, translated_markdown.clone())],
        );

        assert_eq!(blocks[0].translated_markdown, blocks[1].translated_markdown);
        assert_eq!(blocks[2].translated_markdown, "认知过程并不只是顺序模型。");
        assert_eq!(blocks[2].marker_id.as_deref(), Some("B002"));
    }

    #[test]
    fn extracts_epub_from_env_path() {
        let Ok(path) = std::env::var("MUSETRANSLATE_EPUB_TEST_PATH") else {
            return;
        };

        let document = extract_epub_document(Path::new(&path)).expect("EPUB should parse");
        assert!(document.markdown.chars().count() > 1000);
        assert!(document.markdown.split("\n\n").count() > 10);
        assert!(document.epub_blocks.len() > 10);
        assert!(document
            .epub_blocks
            .iter()
            .any(|block| block.kind == "paragraph"));
        assert!(document.cover_data_url.is_some());
    }

    #[test]
    fn exports_epub_source_artifacts_from_env_path() {
        let Ok(path) = std::env::var("MUSETRANSLATE_EPUB_TEST_PATH") else {
            return;
        };
        let Ok(output_dir) = std::env::var("MUSETRANSLATE_EPUB_EXPORT_DIR") else {
            return;
        };

        let document = extract_epub_document(Path::new(&path)).expect("EPUB should parse");
        let output_dir = Path::new(&output_dir);
        std::fs::create_dir_all(output_dir).expect("export dir should be created");
        std::fs::write(output_dir.join("source.md"), &document.markdown)
            .expect("source markdown should be written");
        std::fs::write(
            output_dir.join("source_blocks.json"),
            serde_json::to_string_pretty(&document.epub_blocks).expect("blocks should serialize"),
        )
        .expect("source blocks should be written");
        write_exported_epub_images(output_dir, &document.markdown_images);
        if let Some(toc) = document.toc {
            std::fs::write(
                output_dir.join("source_toc.json"),
                serde_json::to_string_pretty(&toc).expect("toc should serialize"),
            )
            .expect("source toc should be written");
        }
    }

    #[test]
    fn extracts_named_epub_chapter_from_env_path() {
        let Ok(path) = std::env::var("MUSETRANSLATE_EPUB_TEST_PATH") else {
            return;
        };

        let chapter = extract_epub_chapter_by_title(Path::new(&path), "The Abominations of Yondo")
            .expect("chapter should parse");

        assert_eq!(chapter.title, "The Abominations of Yondo");
        assert!(chapter.markdown.contains("The Abominations of Yondo"));
        assert!(chapter.markdown.chars().count() > 1000);
    }

    #[test]
    fn exports_named_epub_chapter_layout_from_env_path() {
        let Ok(path) = std::env::var("MUSETRANSLATE_EPUB_TEST_PATH") else {
            return;
        };
        let Ok(output_dir) = std::env::var("MUSETRANSLATE_EPUB_CHAPTER_EXPORT_DIR") else {
            return;
        };
        let chapter_title = std::env::var("MUSETRANSLATE_EPUB_CHAPTER_TITLE")
            .unwrap_or_else(|_| "The Abominations of Yondo".to_string());

        let chapter = extract_epub_chapter_by_title(Path::new(&path), &chapter_title)
            .expect("chapter should parse");
        let blocks = chapter.blocks.clone();
        let markdown = format_epub_chapter_markdown(chapter);
        let output_dir = Path::new(&output_dir);
        std::fs::create_dir_all(output_dir).expect("export dir should be created");
        std::fs::write(output_dir.join("source.md"), markdown)
            .expect("chapter source markdown should be written");
        std::fs::write(
            output_dir.join("source_blocks.json"),
            serde_json::to_string_pretty(&blocks).expect("blocks should serialize"),
        )
        .expect("chapter source blocks should be written");
    }

    #[test]
    fn translated_block_alignment_keeps_reference_section_stable_after_extra_body_block() {
        let source_markdown = [
            "# Title",
            "",
            "Body one.",
            "",
            "Body two.",
            "",
            "## References",
            "",
            "Ref A.",
            "",
            "Ref B.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 标题",
            "",
            "正文一。",
            "",
            "正文二。",
            "",
            "额外致谢。",
            "",
            "## 参考文献",
            "",
            "Ref A.",
            "",
            "Ref B.",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(translated_blocks[3].translated_markdown, "## 参考文献");
        assert_eq!(translated_blocks[4].translated_markdown, "Ref A.");
        assert_eq!(translated_blocks[5].translated_markdown, "Ref B.");
    }

    #[test]
    fn translated_block_alignment_reanchors_heading_after_inserted_acknowledgement_block() {
        let source_markdown = [
            "# Title",
            "",
            "Final body paragraph.",
            "",
            "## References",
            "",
            "Ref A.",
            "",
            "Ref B.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 标题",
            "",
            "最终正文段落。",
            "",
            "致谢：感谢评论意见。",
            "",
            "## References",
            "",
            "Ref A.",
            "",
            "Ref B.",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(translated_blocks[2].translated_markdown, "## References");
        assert_eq!(translated_blocks[3].translated_markdown, "Ref A.");
        assert_eq!(translated_blocks[4].translated_markdown, "Ref B.");
    }

    #[test]
    fn translated_block_alignment_stays_stable_after_front_matter_is_rewritten() {
        let rewritten_source = [
            "# Normalized Title",
            "",
            "Author One $ ^{1} $, Author Two $ ^{2,*} $",
            "",
            "1 Department A",
            "",
            "2 Department B",
            "",
            "* Corresponding author",
            "",
            "Email: author.two@example.test",
            "",
            "https://doi.org/10.1/example",
            "",
            "Abstract body.",
            "",
            "## From nudge to nudge plus",
            "",
            "Body section one.",
            "",
            "## References",
            "",
            "Ref A.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 规范化标题",
            "",
            "作者一 $ ^{1} $, 作者二 $ ^{2,*} $",
            "",
            "1 Department A",
            "",
            "2 Department B",
            "",
            "* Corresponding author",
            "",
            "Email: author.two@example.test",
            "",
            "https://doi.org/10.1/example",
            "",
            "摘要正文。",
            "",
            "## 从助推到助推+",
            "",
            "第一节正文。",
            "",
            "## References",
            "",
            "Ref A.",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&rewritten_source);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(translated_blocks[0].translated_markdown, "# 规范化标题");
        assert_eq!(
            translated_blocks[6].translated_markdown,
            "https://doi.org/10.1/example"
        );
        assert_eq!(translated_blocks[7].translated_markdown, "摘要正文。");
        assert_eq!(translated_blocks[8].translated_markdown, "## 从助推到助推+");
        assert_eq!(translated_blocks[10].translated_markdown, "## References");
        assert_eq!(translated_blocks[11].translated_markdown, "Ref A.");
    }

    #[test]
    fn translated_block_alignment_skips_explicit_front_matter_removed_from_visible_markdown() {
        let rewritten_source = [
            FRONT_MATTER_PROTECTED_START_MARKER,
            "# English Title",
            "",
            "Author One",
            "",
            "Department A",
            FRONT_MATTER_PROTECTED_END_MARKER,
            "",
            "## Abstract",
            "",
            "Abstract body.",
            "",
            "## References",
            "",
            "Ref A.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 中文标题",
            "",
            "## 摘要",
            "",
            "摘要正文。",
            "",
            "## 参考文献",
            "",
            "Ref A.",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&rewritten_source);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(translated_blocks[0].translated_markdown, "");
        assert_eq!(translated_blocks[1].translated_markdown, "");
        assert_eq!(translated_blocks[2].translated_markdown, "");
        assert_eq!(translated_blocks[3].translated_markdown, "## 摘要");
        assert_eq!(translated_blocks[4].translated_markdown, "摘要正文。");
        assert_eq!(translated_blocks[5].translated_markdown, "## 参考文献");
        assert_eq!(translated_blocks[6].translated_markdown, "Ref A.");
    }

    #[test]
    fn translated_block_alignment_repairs_split_pdf_table_marker_blocks() {
        let source_markdown = [
            "## Section",
            "",
            "Lead paragraph.",
            "",
            "Table 1 | Caption.",
            "",
            "<!-- musetranslate:pdf-table:start -->",
            "![PDF table 1](imgs/table_snapshot_001.jpg)",
            "<!-- musetranslate:pdf-table:end -->",
            "",
            "Following paragraph.",
        ]
        .join("\n");
        let translated_markdown = [
            "## 章节",
            "",
            "引导段。",
            "",
            "表1 | 标题。",
            "",
            "![PDF table 1](imgs/table_snapshot_001.jpg)",
            "",
            "<!-- musetranslate:pdf-table:start -->",
            "<!-- musetranslate:pdf-table:end -->",
            "",
            "后续段落。",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(
            translated_blocks[3].translated_markdown,
            "<!-- musetranslate:pdf-table:start -->\n![PDF table 1](imgs/table_snapshot_001.jpg)\n<!-- musetranslate:pdf-table:end -->"
        );
        assert_eq!(translated_blocks[4].translated_markdown, "后续段落。");
    }

    #[test]
    fn translated_block_alignment_keeps_front_matter_details_out_of_abstract_body() {
        let rewritten_source = [
            FRONT_MATTER_PROTECTED_START_MARKER,
            "# English Title",
            "",
            "Author One",
            "",
            "https://doi.org/10.1/example",
            FRONT_MATTER_PROTECTED_END_MARKER,
            "",
            "## Abstract",
            "",
            "Abstract body.",
            "",
            "## Methods",
            "",
            "Body paragraph.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 中文标题",
            "",
            "## 摘要",
            "",
            "摘要正文。",
            "",
            "## 方法",
            "",
            "正文段落。",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&rewritten_source);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(translated_blocks[0].translated_markdown, "");
        assert_eq!(translated_blocks[1].translated_markdown, "");
        assert_eq!(translated_blocks[2].translated_markdown, "");
        assert_eq!(translated_blocks[3].translated_markdown, "## 摘要");
        assert_eq!(translated_blocks[4].translated_markdown, "摘要正文。");
        assert_eq!(translated_blocks[5].translated_markdown, "## 方法");
        assert_eq!(translated_blocks[6].translated_markdown, "正文段落。");
    }

    #[test]
    fn translated_block_alignment_rejects_paragraph_like_heading_pollution() {
        let source_markdown = [
            "# The Tale of Satampra Zeiros",
            "",
            "I, Satampra Zeiros of Uzuldaroum, shall write with my left hand.",
        ]
        .join("\n");
        let translated_markdown = [
            "我，乌祖尔达罗姆的萨坦普拉·塞罗斯，将用我的左手——因我已再无别的手可用了——书写下提鲁夫·翁帕利奥斯与我本人在查图格伽神龛中遭遇的一切。",
            "",
            "我将用苏瓦纳棕榈的紫色汁液书写。",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert_eq!(translated_blocks[0].translated_markdown, "");
        assert_eq!(translated_blocks[0].translated_text, "");
        assert_eq!(translated_blocks[0].alignment_method, "structureRejected");
    }

    fn write_exported_epub_images(output_dir: &Path, images: &HashMap<String, String>) {
        if images.is_empty() {
            return;
        }
        std::fs::write(
            output_dir.join("source_images.json"),
            serde_json::to_string_pretty(images).expect("images should serialize"),
        )
        .expect("source images should be written");
        for (relative_path, value) in images {
            let Some(bytes) = decode_data_url_image(value) else {
                continue;
            };
            let output_path = output_dir.join(relative_path);
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).expect("image dir should be created");
            }
            std::fs::write(output_path, bytes).expect("image should be written");
        }
    }

    #[test]
    fn translated_block_alignment_reanchors_long_section_by_distinctive_ascii_tokens() {
        let source_markdown = [
            "# Story",
            "",
            "Opening paragraph.",
            "",
            "Tirouv Ompallios argued for bread in Uzuldaroum.",
            "",
            "Satampra Zeiros preferred pomegranate wine.",
            "",
            "Commoriom was already deserted.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 故事",
            "",
            "开场段落。",
            "",
            "额外插入段落。",
            "",
            "Satampra Zeiros 更喜欢石榴酒。",
            "",
            "Commoriom 早已荒废。",
            "",
            "Tirouv Ompallios 在 Uzuldaroum 主张先买面包。",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert!(translated_blocks[2]
            .translated_markdown
            .contains("Tirouv Ompallios"));
        assert!(translated_blocks[3]
            .translated_markdown
            .contains("Satampra Zeiros"));
        assert!(translated_blocks[4]
            .translated_markdown
            .contains("Commoriom"));
    }

    #[test]
    fn translated_block_alignment_handles_epub_style_local_reordering_with_named_anchors() {
        let source_markdown = [
            "# The Tale of Satampra Zeiros",
            "",
            "One evening, in an alley of the more humble quarter of Uzuldaroum, we stopped to count our available resources.",
            "",
            "Tirouv Ompallios argued that bread would better sustain us.",
            "",
            "Satampra Zeiros preferred the pomegranate wine.",
            "",
            "Now Commoriom, as all the world knows, was deserted many hundred years ago.",
        ]
        .join("\n");
        let translated_markdown = [
            "# 萨坦普拉·泽罗斯的故事",
            "",
            "关于阿尔沃究竟遭遇何事，几乎无人能查明确切信息。",
            "",
            "Satampra Zeiros 更喜欢石榴酒。",
            "",
            "Now Commoriom，正如世人所知，早在数百年前便已荒废。",
            "",
            "Tirouv Ompallios 认为面包更能维持我们的体力。",
        ]
        .join("\n");

        let source_blocks = build_source_blocks_from_markdown(&source_markdown);
        let translated_blocks = build_translated_blocks(&source_blocks, &translated_markdown);

        assert!(translated_blocks[2]
            .translated_markdown
            .contains("Tirouv Ompallios"));
        assert!(translated_blocks[3]
            .translated_markdown
            .contains("Satampra Zeiros"));
        assert!(translated_blocks[4]
            .translated_markdown
            .contains("Commoriom"));
    }

    fn decode_data_url_image(value: &str) -> Option<Vec<u8>> {
        let (_, data) = value.split_once(',')?;
        base64::engine::general_purpose::STANDARD.decode(data).ok()
    }
}
