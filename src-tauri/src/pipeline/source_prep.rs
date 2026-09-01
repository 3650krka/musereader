use super::artifact;
use super::cache;
use super::layout::{
    collapse_adjacent_pdf_table_blocks, inject_layout_captions, normalize_pdf_source_markdown,
    rebuild_pdf_markdown_from_layout_json, replace_layout_tables_with_snapshots,
    replace_missing_pdf_table_snapshots, replace_split_figure_clusters_with_crops,
};
use super::paths::ArtifactPaths;
use super::runtime::{ensure_not_cancelled, fnv1a64_hex, retry_async_with_observer};
use super::{PipelineProgressStage, PipelineRunConfig, OCR_RETRY_LIMIT};
use crate::document::{
    build_source_blocks_from_markdown, detect_document_kind, extract_epub_chapter_by_title,
    extract_epub_document, extract_pdf_outline, SourceDocumentKind, TocArtifact, TocEntry,
    TocSourceKind,
};
use crate::error::AppError;
use crate::ocr::{BaiduOcrClient, OcrDocument, OcrDocumentWithRaw};
use crate::translation_validation::sanitize_epub_chapter_markdown;
use std::path::Path;
use std::time::Instant;
use tokio::time::Duration;

const SOURCE_RAW_MARKDOWN_STAGE: &str = "source_raw_markdown_ready";
const SOURCE_JSON_REBUILD_STAGE: &str = "source_json_rebuild_ready";
const SOURCE_MD_CLEANUP_STAGE: &str = "source_md_cleanup_ready";
const SOURCE_VISUAL_SNAPSHOT_STAGE: &str = "source_visual_snapshot_ready";

pub(super) async fn prepare_source_document(
    ocr_client: &BaiduOcrClient,
    config: &PipelineRunConfig,
    paths: &ArtifactPaths,
    progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
    mut record_event: impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<(OcrDocument, String), AppError> {
    let source_prep_started = Instant::now();
    ensure_not_cancelled(&config.cancellation_token)?;
    let document_kind = detect_document_kind(&config.pdf_path)?;

    if let Some(mut source_document) = artifact::load_source_document(paths).await? {
        record_event("ocr_reused", "reused persisted OCR artifacts", 18)?;
        progress_callback(PipelineProgressStage::Imported, "reused OCR artifacts", 18);
        apply_pdf_visual_snapshots(paths, &mut source_document, &mut record_event).await?;
        let source_hash = fnv1a64_hex(source_document.markdown.as_bytes());
        record_event(
            "pipeline_stage_timing",
            &format!(
                "stage=source_prep_internal; elapsed_ms={}; document_kind={}; reused=true; chars={}; source_blocks={}; epub_blocks={}; toc_entries={}",
                source_prep_started.elapsed().as_millis(),
                source_document_kind_label(document_kind),
                source_document.markdown.chars().count(),
                source_document.source_blocks.len(),
                source_document.epub_blocks.len(),
                source_document.toc.as_ref().map(|toc| toc.entries.len()).unwrap_or(0)
            ),
            18,
        )?;
        return Ok((source_document, source_hash));
    }

    let source_document = match document_kind {
        SourceDocumentKind::Pdf => {
            progress_callback(PipelineProgressStage::OcrRunning, "running OCR", 20);
            let raw_ocr = load_or_run_pdf_ocr(ocr_client, config, paths, &mut record_event).await?;
            build_pdf_document_from_raw_ocr(
                raw_ocr,
                paths,
                &config.article_type,
                &config.pdf_path,
                &mut record_event,
            )
            .await?
        }
        SourceDocumentKind::Epub => {
            record_event("epub_extract_started", "starting EPUB extraction", 20)?;
            progress_callback(PipelineProgressStage::Imported, "extracting EPUB", 20);
            let epub_extract_started = Instant::now();
            let source = extract_epub_document(&config.pdf_path)?;
            record_event(
                "pipeline_stage_timing",
                &format!(
                    "stage=epub_extract; elapsed_ms={}; chars={}; source_blocks={}; epub_blocks={}; toc_entries={}; image_count={}",
                    epub_extract_started.elapsed().as_millis(),
                    source.markdown.chars().count(),
                    source.source_blocks.len(),
                    source.epub_blocks.len(),
                    source.toc.as_ref().map(|toc| toc.entries.len()).unwrap_or(0),
                    source.markdown_images.len()
                ),
                22,
            )?;
            if let Some(chapter_title) = config.epub_chapter_title.as_deref() {
                let chapter_extract_started = Instant::now();
                let chapter = extract_epub_chapter_by_title(&config.pdf_path, chapter_title)?;
                record_event(
                    "pipeline_stage_timing",
                    &format!(
                        "stage=epub_chapter_extract; elapsed_ms={}; chapter_order={}; chars={}; blocks={}",
                        chapter_extract_started.elapsed().as_millis(),
                        chapter.order,
                        chapter.markdown.chars().count(),
                        chapter.blocks.len()
                    ),
                    23,
                )?;
                OcrDocument {
                    markdown: sanitize_epub_chapter_markdown(&format!(
                        "<!-- epub-chapter-order: {} -->\n\n# {}\n\n{}",
                        chapter.order, chapter.title, chapter.markdown
                    )),
                    cover_data_url: source.cover_data_url,
                    page_layouts: Vec::new(),
                    markdown_images: source.markdown_images,
                    toc: source.toc,
                    epub_blocks: chapter.blocks,
                    source_blocks: build_source_blocks_from_markdown(
                        &sanitize_epub_chapter_markdown(&format!(
                            "<!-- epub-chapter-order: {} -->\n\n# {}\n\n{}",
                            chapter.order, chapter.title, chapter.markdown
                        )),
                    ),
                }
            } else {
                OcrDocument {
                    markdown: source.markdown,
                    cover_data_url: source.cover_data_url,
                    page_layouts: Vec::new(),
                    markdown_images: source.markdown_images,
                    toc: source.toc,
                    epub_blocks: source.epub_blocks,
                    source_blocks: source.source_blocks,
                }
            }
        }
        SourceDocumentKind::Markdown | SourceDocumentKind::Text => {
            record_event("text_extract_started", "reading text/markdown file", 20)?;
            progress_callback(PipelineProgressStage::Imported, "reading text file", 20);
            let content = std::fs::read_to_string(&config.pdf_path).map_err(|error| {
                AppError::internal(format!(
                    "read text file failed ({}): {error}",
                    config.pdf_path.display()
                ))
            })?;
            // 编码嗅探：若含 BOM 或大量 UTF-8 替换字符，尝试 GBK 解码。
            let markdown = if content.starts_with('\u{feff}') {
                content.trim_start_matches('\u{feff}').to_string()
            } else {
                content
            };
            OcrDocument {
                markdown,
                cover_data_url: None,
                page_layouts: Vec::new(),
                markdown_images: Default::default(),
                toc: None,
                epub_blocks: Vec::new(),
                source_blocks: Vec::new(), // 由下方 normalized_source_blocks 兜底
            }
        }
        SourceDocumentKind::Docx => {
            record_event("text_extract_started", "extracting docx document", 20)?;
            progress_callback(PipelineProgressStage::Imported, "extracting docx", 20);
            let markdown = crate::document::docx::extract_docx_markdown(&config.pdf_path)?;
            OcrDocument {
                markdown,
                cover_data_url: None,
                page_layouts: Vec::new(),
                markdown_images: Default::default(),
                toc: None, // 由 infer_source_toc 从标题层级推断
                epub_blocks: Vec::new(),
                source_blocks: Vec::new(),
            }
        }
    };

    let normalized_toc = source_document
        .toc
        .or_else(|| infer_source_toc(&source_document.markdown));
    let normalized_source_blocks = if source_document.source_blocks.is_empty() {
        build_source_blocks_from_markdown(&source_document.markdown)
    } else {
        source_document.source_blocks
    };
    let mut source_document = OcrDocument {
        toc: normalized_toc,
        markdown: source_document.markdown,
        cover_data_url: source_document.cover_data_url,
        page_layouts: source_document.page_layouts,
        markdown_images: source_document.markdown_images,
        epub_blocks: source_document.epub_blocks,
        source_blocks: normalized_source_blocks,
    };
    artifact::persist_source_document(paths, &source_document).await?;
    if matches!(document_kind, SourceDocumentKind::Pdf) {
        apply_pdf_visual_snapshots(paths, &mut source_document, &mut record_event).await?;
    }
    let source_hash = fnv1a64_hex(source_document.markdown.as_bytes());
    record_event(
        "pipeline_stage_timing",
        &format!(
            "stage=source_prep_internal; elapsed_ms={}; document_kind={}; chars={}; source_blocks={}; epub_blocks={}; toc_entries={}",
            source_prep_started.elapsed().as_millis(),
            source_document_kind_label(document_kind),
            source_document.markdown.chars().count(),
            source_document.source_blocks.len(),
            source_document.epub_blocks.len(),
            source_document.toc.as_ref().map(|toc| toc.entries.len()).unwrap_or(0)
        ),
        26,
    )?;

    Ok((source_document, source_hash))
}

fn source_document_kind_label(kind: SourceDocumentKind) -> &'static str {
    match kind {
        SourceDocumentKind::Pdf => "pdf",
        SourceDocumentKind::Epub => "epub",
        SourceDocumentKind::Markdown => "markdown",
        SourceDocumentKind::Text => "text",
        SourceDocumentKind::Docx => "docx",
    }
}

async fn load_or_run_pdf_ocr(
    ocr_client: &BaiduOcrClient,
    config: &PipelineRunConfig,
    paths: &ArtifactPaths,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<OcrDocumentWithRaw, AppError> {
    let cache_key = if config.cache_config.effective_ocr_cache_enabled() {
        match cache::build_ocr_raw_cache_key(&config.pdf_path, &ocr_client.cache_fingerprint()) {
            Ok(key) => Some(key),
            Err(error) => {
                record_event(
                    "ocr_cache_key_failed",
                    &format!(
                        "OCR cache key unavailable; falling back to provider: {}",
                        error.message
                    ),
                    20,
                )?;
                None
            }
        }
    } else {
        record_event(
            "ocr_cache_disabled",
            "OCR raw cache disabled for this run",
            20,
        )?;
        None
    };

    if let Some(key) = cache_key.as_ref() {
        match cache::load_ocr_raw_cache(&config.cache_config.root_dir, key) {
            Ok(Some(raw_ocr)) => {
                record_event(
                    "ocr_cache_hit",
                    &format!("loaded raw OCR cache; cache_key={}", key.key),
                    20,
                )?;
                persist_raw_ocr_artifacts(paths, &raw_ocr, record_event).await?;
                return Ok(raw_ocr);
            }
            Ok(None) => {
                record_event(
                    "ocr_cache_miss",
                    &format!("raw OCR cache miss; cache_key={}", key.key),
                    20,
                )?;
            }
            Err(error) => {
                record_event(
                    "ocr_cache_invalid",
                    &format!(
                        "raw OCR cache invalid; cache_key={}; message={}",
                        key.key, error.message
                    ),
                    20,
                )?;
            }
        }
    }

    let raw_ocr = run_pdf_ocr_provider(ocr_client, config, record_event).await?;
    persist_raw_ocr_artifacts(paths, &raw_ocr, record_event).await?;
    if let Some(key) = cache_key.as_ref() {
        match cache::store_ocr_raw_cache(&config.cache_config.root_dir, key, &raw_ocr).await {
            Ok(()) => record_event(
                "ocr_cache_stored",
                &format!("stored raw OCR cache; cache_key={}", key.key),
                21,
            )?,
            Err(error) => record_event(
                "ocr_cache_store_failed",
                &format!(
                    "raw OCR cache store failed; cache_key={}; message={}",
                    key.key, error.message
                ),
                21,
            )?,
        }
    }
    Ok(raw_ocr)
}

async fn run_pdf_ocr_provider(
    ocr_client: &BaiduOcrClient,
    config: &PipelineRunConfig,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<OcrDocumentWithRaw, AppError> {
    record_event("ocr_started", "starting OCR", 20)?;
    retry_async_with_observer(
        OCR_RETRY_LIMIT,
        Duration::from_secs(2),
        || config.cancellation_token.is_cancelled(),
        |attempt| {
            let event_result = record_event("ocr_attempt", &format!("OCR attempt {attempt}"), 20);
            async move {
                event_result?;
                ocr_client
                    .convert_pdf_with_raw(&config.pdf_path)
                    .await
                    .map_err(AppError::from_provider_error)
            }
        },
        |attempt, error, delay| {
            eprintln!(
                "[ocr-attempt-failed] attempt={attempt}; code={}; delay_ms={}; message={}",
                error.code,
                delay.as_millis(),
                error.message
            );
        },
    )
    .await
}

async fn persist_raw_ocr_artifacts(
    paths: &ArtifactPaths,
    raw_ocr: &OcrDocumentWithRaw,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<(), AppError> {
    artifact::write_json(&paths.source_ocr_layout_raw_path, &raw_ocr.layout_raw_json).await?;
    artifact::write_json(
        &paths.source_ocr_structure_raw_path,
        &raw_ocr.structure_raw_json,
    )
    .await?;
    record_event(
        "source_ocr_raw_json_ready",
        &format!(
            "persisted raw OCR JSON; layout_results={}; structure_results={}",
            raw_ocr.layout_results.len(),
            raw_ocr.structure_results.len()
        ),
        21,
    )
}

async fn build_pdf_document_from_raw_ocr(
    raw_ocr: OcrDocumentWithRaw,
    paths: &ArtifactPaths,
    article_type: &str,
    pdf_path: &Path,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<OcrDocument, AppError> {
    let mut document = raw_ocr.document;
    let raw_markdown = document.markdown.clone();
    persist_source_markdown_stage(paths, &paths.source_raw_ocr_markdown_path, &raw_markdown)
        .await?;
    artifact::persist_markdown_images_only(paths, &document.markdown_images).await?;
    record_event(
        SOURCE_RAW_MARKDOWN_STAGE,
        &format!(
            "persisted raw OCR markdown; chars={}",
            raw_markdown.chars().count()
        ),
        22,
    )?;
    let rebuilt_markdown = rebuild_pdf_markdown_from_json_stage(
        paths,
        &raw_markdown,
        &document.page_layouts,
        record_event,
    )?;
    persist_source_markdown_stage(
        paths,
        &paths.source_json_rebuilt_markdown_path,
        &rebuilt_markdown,
    )
    .await?;
    record_event(
        SOURCE_JSON_REBUILD_STAGE,
        &format!(
            "persisted JSON rebuild markdown; chars={}",
            rebuilt_markdown.chars().count()
        ),
        23,
    )?;
    let cleaned_markdown = normalize_pdf_source_markdown(
        &rebuilt_markdown,
        &document.page_layouts,
        &document.markdown_images,
        article_type,
    );
    persist_source_markdown_stage(
        paths,
        &paths.source_md_cleanup_markdown_path,
        &cleaned_markdown,
    )
    .await?;
    record_event(
        SOURCE_MD_CLEANUP_STAGE,
        &format!(
            "persisted markdown cleanup stage; chars={}",
            cleaned_markdown.chars().count()
        ),
        24,
    )?;
    document.markdown = cleaned_markdown;
    document.toc = source_toc_for_pdf(extract_pdf_outline(pdf_path)?, &document.markdown);
    Ok(document)
}

async fn apply_pdf_visual_snapshots(
    paths: &ArtifactPaths,
    source_document: &mut OcrDocument,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let artifact_root = paths
        .source_markdown_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let figure_markdown = replace_split_figure_clusters_with_crops(
        &source_document.markdown,
        &source_document.markdown_images,
        artifact_root,
    );
    let layout_table_markdown =
        if figure_markdown.contains(crate::pipeline::layout::PDF_TABLE_START_MARKER) {
            figure_markdown
        } else {
            replace_layout_tables_with_snapshots(
                &figure_markdown,
                &source_document.page_layouts,
                artifact_root,
            )
        };
    let table_markdown = replace_missing_pdf_table_snapshots(
        &layout_table_markdown,
        &source_document.page_layouts,
        artifact_root,
    );
    let caption_markdown = inject_layout_captions(&table_markdown, &source_document.page_layouts);
    if caption_markdown == source_document.markdown {
        return Ok(());
    }
    source_document.markdown = caption_markdown;
    source_document.source_blocks = build_source_blocks_from_markdown(&source_document.markdown);
    persist_source_markdown_stage(
        paths,
        &paths.source_markdown_path,
        &source_document.markdown,
    )
    .await?;
    persist_source_markdown_stage(
        paths,
        &paths.source_md_cleanup_markdown_path,
        &source_document.markdown,
    )
    .await?;
    record_event(
        SOURCE_VISUAL_SNAPSHOT_STAGE,
        &format!(
            "persisted visual snapshot stage; chars={}",
            source_document.markdown.chars().count()
        ),
        26,
    )?;
    artifact::persist_source_document(paths, source_document).await
}

fn rebuild_pdf_markdown_from_json_stage(
    paths: &ArtifactPaths,
    markdown: &str,
    page_layouts: &[crate::ocr::OcrPageLayout],
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<String, AppError> {
    let block_count: usize = page_layouts.iter().map(|page| page.blocks.len()).sum();
    let dropped_numbers = count_layout_number_blocks(page_layouts);
    let without_numbers = drop_layout_number_blocks(markdown, page_layouts);
    let promoted_titles = count_layout_paragraph_titles(page_layouts);
    let artifact_root = paths
        .source_markdown_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let layout_rebuild =
        rebuild_pdf_markdown_from_layout_json(&without_numbers, page_layouts, artifact_root);
    let with_titles = promote_layout_paragraph_titles(&layout_rebuild.markdown, page_layouts);
    let with_backfilled_tables =
        replace_missing_pdf_table_snapshots(&with_titles, page_layouts, artifact_root);
    let rebuilt = collapse_adjacent_pdf_table_blocks(&with_backfilled_tables);
    let json_table_blocks = rebuilt
        .matches(crate::pipeline::layout::PDF_TABLE_START_MARKER)
        .count();
    record_event(
        "json_rebuild_structured_pdf",
        &format!(
            "JSON rebuild stage structured pass; pages={}; blocks={block_count}; dropped_number_blocks={dropped_numbers}; paragraph_titles={promoted_titles}; rendered_blocks={}; figure_blocks={}; table_blocks={json_table_blocks}; skipped_media_blocks={}; reference_blocks_backfilled={}; used_layout_renderer={}; table_text_matching_skipped=true",
            page_layouts.len(),
            layout_rebuild.report.rendered_blocks,
            layout_rebuild.report.figure_blocks,
            layout_rebuild.report.skipped_media_blocks,
            layout_rebuild.report.reference_blocks_backfilled,
            layout_rebuild.report.used_layout_renderer,
        ),
        23,
    )?;
    if cfg!(debug_assertions) {
        let before_tokens = [
            "Table 1. Some working examples of nudge plus.",
            "Timing of nudge plus",
            "<!-- musetranslate:pdf-table:start -->",
        ]
        .into_iter()
        .map(|token| {
            format!(
                "{token}=>titles:{} backfilled:{} rebuilt:{}",
                with_titles.contains(token),
                with_backfilled_tables.contains(token),
                rebuilt.contains(token)
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
        record_event(
            "json_rebuild_table_probe",
            &format!("JSON rebuild probe; {before_tokens}"),
            23,
        )?;
    }
    Ok(rebuilt)
}

async fn persist_source_markdown_stage(
    paths: &ArtifactPaths,
    path: &std::path::Path,
    markdown: &str,
) -> Result<(), AppError> {
    if path == paths.source_markdown_path {
        artifact::write_utf8(path, markdown).await
    } else {
        artifact::write_utf8(path, markdown).await
    }
}

fn count_layout_number_blocks(page_layouts: &[crate::ocr::OcrPageLayout]) -> usize {
    page_layouts
        .iter()
        .flat_map(|page| page.blocks.iter())
        .filter(|block| block.label.eq_ignore_ascii_case("number"))
        .count()
}

fn count_layout_paragraph_titles(page_layouts: &[crate::ocr::OcrPageLayout]) -> usize {
    page_layouts
        .iter()
        .flat_map(|page| page.blocks.iter())
        .filter(|block| block.label.eq_ignore_ascii_case("paragraph_title"))
        .filter(|block| !block.content.trim().is_empty())
        .count()
}

fn drop_layout_number_blocks(markdown: &str, page_layouts: &[crate::ocr::OcrPageLayout]) -> String {
    let number_texts = page_layouts
        .iter()
        .flat_map(|page| page.blocks.iter())
        .filter(|block| block.label.eq_ignore_ascii_case("number"))
        .map(|block| block.content.trim())
        .filter(|content| is_standalone_number_block(content))
        .collect::<std::collections::HashSet<_>>();
    if number_texts.is_empty() {
        return markdown.to_string();
    }
    split_markdown_blocks(markdown)
        .into_iter()
        .filter(|block| {
            let trimmed = block.trim();
            !(is_standalone_number_block(trimmed) && number_texts.contains(trimmed))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn promote_layout_paragraph_titles(
    markdown: &str,
    page_layouts: &[crate::ocr::OcrPageLayout],
) -> String {
    let title_candidates = layout_paragraph_title_candidates(page_layouts);
    if title_candidates.is_empty() {
        return markdown.to_string();
    }
    let title_blocks = title_candidates
        .iter()
        .map(|candidate| candidate.title.clone())
        .collect::<std::collections::HashSet<_>>();
    let mut promoted_titles = existing_markdown_heading_titles(markdown);
    let mut pending_candidates = title_candidates;
    split_markdown_blocks(markdown)
        .into_iter()
        .map(|block| {
            let trimmed = block.trim();
            if title_blocks.contains(trimmed) && !trimmed.starts_with('#') {
                promoted_titles.insert(trimmed.to_string());
                format!("## {trimmed}")
            } else {
                prefix_missing_layout_titles(&block, &mut pending_candidates, &mut promoted_titles)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[derive(Debug, Clone)]
struct LayoutTitleCandidate {
    title: String,
    anchor: Option<String>,
}

fn layout_paragraph_title_candidates(
    page_layouts: &[crate::ocr::OcrPageLayout],
) -> Vec<LayoutTitleCandidate> {
    let mut candidates = Vec::new();
    for page in page_layouts {
        for (index, block) in page.blocks.iter().enumerate() {
            if !block.label.eq_ignore_ascii_case("paragraph_title") {
                continue;
            }
            let title = normalized_layout_content(&block.content);
            if title.is_empty() {
                continue;
            }
            candidates.push(LayoutTitleCandidate {
                title,
                anchor: next_text_anchor(&page.blocks[index + 1..]),
            });
        }
    }
    candidates
}

fn next_text_anchor(blocks: &[crate::ocr::OcrLayoutBlock]) -> Option<String> {
    blocks
        .iter()
        .find(|block| layout_block_can_anchor_title(block))
        .map(|block| normalized_layout_content(&block.content))
        .filter(|content| !content.is_empty())
        .map(|content| content.chars().take(96).collect())
}

fn layout_block_can_anchor_title(block: &crate::ocr::OcrLayoutBlock) -> bool {
    matches!(block.label.as_str(), "text" | "abstract")
        && normalized_layout_content(&block.content)
            .split_whitespace()
            .count()
            >= 6
}

fn normalized_layout_content(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn existing_markdown_heading_titles(markdown: &str) -> std::collections::HashSet<String> {
    markdown
        .lines()
        .filter_map(markdown_heading_title)
        .map(ToString::to_string)
        .collect()
}

fn markdown_heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let marker_count = trimmed.chars().take_while(|ch| *ch == '#').count();
    if marker_count == 0 || marker_count > 6 {
        return None;
    }
    let rest = trimmed.get(marker_count..)?;
    rest.starts_with(' ')
        .then(|| rest.trim())
        .filter(|title| !title.is_empty())
}

fn prefix_missing_layout_titles(
    block: &str,
    pending_candidates: &mut [LayoutTitleCandidate],
    promoted_titles: &mut std::collections::HashSet<String>,
) -> String {
    let block_surface = normalized_layout_content(block);
    let titles = pending_candidates
        .iter_mut()
        .filter_map(|candidate| {
            if promoted_titles.contains(&candidate.title) {
                return None;
            }
            let anchor = candidate.anchor.as_deref()?;
            if !block_surface.contains(anchor) {
                return None;
            }
            promoted_titles.insert(candidate.title.clone());
            Some(format!("## {}", candidate.title))
        })
        .collect::<Vec<_>>();
    if titles.is_empty() {
        block.to_string()
    } else {
        format!("{}\n\n{}", titles.join("\n\n"), block)
    }
}

fn is_standalone_number_block(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= 4 && text.chars().all(|ch| ch.is_ascii_digit())
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(super) fn infer_source_toc(markdown: &str) -> Option<TocArtifact> {
    infer_heading_toc(markdown).or_else(|| infer_layout_toc(markdown))
}

fn source_toc_for_pdf(outline: Option<TocArtifact>, markdown: &str) -> Option<TocArtifact> {
    match outline {
        Some(outline_toc) => Some(merge_outline_with_heading_toc(outline_toc, markdown)),
        None => infer_source_toc(markdown),
    }
}

fn merge_outline_with_heading_toc(outline: TocArtifact, markdown: &str) -> TocArtifact {
    let Some(heading_toc) = infer_heading_toc(markdown) else {
        return outline;
    };
    let mut outline_entries = outline.entries.clone();
    let mut merged = Vec::with_capacity(heading_toc.entries.len().max(outline_entries.len()));

    for heading in heading_toc.entries {
        let entry = match take_matching_outline_entry(&mut outline_entries, &heading.source_title) {
            Some(outline_entry) => merged_toc_entry_from_heading(heading, Some(outline_entry)),
            None => merged_toc_entry_from_heading(heading, None),
        };
        merged.push(entry);
    }
    renumber_toc_entries(&mut merged);

    TocArtifact {
        source_kind: TocSourceKind::PdfOutline,
        confidence: outline.confidence,
        entries: merged,
    }
}

fn take_matching_outline_entry(entries: &mut Vec<TocEntry>, heading: &str) -> Option<TocEntry> {
    let heading_key = toc_title_match_key(heading);
    let index = entries
        .iter()
        .position(|entry| toc_title_match_key(&entry.source_title) == heading_key)?;
    Some(entries.remove(index))
}

fn merged_toc_entry_from_heading(heading: TocEntry, outline: Option<TocEntry>) -> TocEntry {
    let Some(outline) = outline else {
        return heading;
    };
    TocEntry {
        source_title: heading.source_title,
        translated_title: heading.translated_title,
        level: heading.level,
        order: heading.order,
        page: outline.page,
        href: outline.href,
        source_kind: outline.source_kind,
        confidence: outline.confidence,
    }
}

fn renumber_toc_entries(entries: &mut [TocEntry]) {
    for (order, entry) in entries.iter_mut().enumerate() {
        entry.order = order;
    }
}

fn toc_title_match_key(title: &str) -> String {
    let title = strip_structural_heading_prefix(title);
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn strip_structural_heading_prefix(title: &str) -> &str {
    let trimmed = title.trim();
    let Some((prefix, rest)) = trimmed.split_once(char::is_whitespace) else {
        return trimmed;
    };
    if is_structural_heading_prefix(prefix) {
        rest.trim()
    } else {
        trimmed
    }
}

fn is_structural_heading_prefix(prefix: &str) -> bool {
    let normalized = prefix.trim_end_matches('.');
    if normalized.is_empty() {
        return false;
    }
    let mut saw_digit = false;
    for part in normalized.split('.') {
        if part.is_empty() {
            return false;
        }
        if part.chars().all(|ch| ch.is_ascii_digit()) {
            saw_digit = true;
            continue;
        }
        let mut chars = part.chars();
        let is_single_letter =
            chars.next().is_some_and(|ch| ch.is_ascii_alphabetic()) && chars.next().is_none();
        if !is_single_letter {
            return false;
        }
    }
    saw_digit
}

fn infer_heading_toc(markdown: &str) -> Option<TocArtifact> {
    let entries = markdown
        .lines()
        .enumerate()
        .filter_map(|(line_index, line)| heading_toc_entry(line, line_index))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    Some(TocArtifact {
        source_kind: TocSourceKind::HeadingInferred,
        confidence: 0.7,
        entries,
    })
}

fn infer_layout_toc(markdown: &str) -> Option<TocArtifact> {
    let entries = markdown
        .lines()
        .enumerate()
        .filter_map(|(line_index, line)| layout_toc_entry(line, line_index))
        .collect::<Vec<_>>();
    if entries.len() < 2 {
        return None;
    }
    Some(TocArtifact {
        source_kind: TocSourceKind::LayoutInferred,
        confidence: 0.6,
        entries,
    })
}

fn layout_toc_entry(line: &str, order: usize) -> Option<TocEntry> {
    let title = normalize_candidate_title(line)?;
    if is_non_toc_line(&title) {
        return None;
    }
    let (level, confidence) = classify_layout_title(&title)?;
    Some(TocEntry {
        source_title: title,
        translated_title: None,
        level,
        order,
        page: None,
        href: None,
        source_kind: TocSourceKind::LayoutInferred,
        confidence,
    })
}

fn normalize_candidate_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return None;
    }
    let title = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

fn classify_layout_title(title: &str) -> Option<(u8, f32)> {
    let lower = title.to_ascii_lowercase();
    if academic_section_title(&lower) {
        return Some((1, 0.65));
    }
    if let Some(level) = numbered_heading_level(title) {
        return Some((level, 0.7));
    }
    if is_english_chapter_title(&lower) {
        return Some((1, 0.65));
    }
    if is_chinese_chapter_title(title) {
        return Some((1, 0.65));
    }
    if is_chinese_numbered_title(title) {
        return Some((1, 0.6));
    }
    None
}

fn numbered_heading_level(title: &str) -> Option<u8> {
    let (prefix, rest) = title.split_once(char::is_whitespace)?;
    if prefix.is_empty()
        || rest.len() < 3
        || !rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
    {
        return None;
    }
    let mut saw_digit = false;
    for part in prefix.split('.') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        saw_digit = true;
    }
    saw_digit.then_some((prefix.matches('.').count() + 1).min(6) as u8)
}

fn is_english_chapter_title(lower: &str) -> bool {
    let Some(rest) = lower.strip_prefix("chapter ") else {
        return false;
    };
    rest.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn is_chinese_chapter_title(title: &str) -> bool {
    title.starts_with('第') && (title.contains('章') || title.contains('节')) && title.len() >= 4
}

fn is_chinese_numbered_title(title: &str) -> bool {
    let Some(first) = title.chars().next() else {
        return false;
    };
    "一二三四五六七八九十".contains(first) && (title.contains('、') || title.contains('.'))
}

fn academic_section_title(lower: &str) -> bool {
    matches!(
        lower,
        "abstract"
            | "introduction"
            | "background"
            | "method"
            | "methods"
            | "materials and methods"
            | "results"
            | "discussion"
            | "conclusion"
            | "conclusions"
            | "references"
    )
}

fn is_non_toc_line(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    lower.starts_with("figure ")
        || lower.starts_with("fig. ")
        || lower.starts_with("table ")
        || lower.starts_with("doi:")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.starts_with('[')
}

fn heading_toc_entry(line: &str, order: usize) -> Option<TocEntry> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let title = trimmed[level..].trim();
    if title.is_empty() {
        return None;
    }
    if is_non_toc_line(title) {
        return None;
    }
    Some(TocEntry {
        source_title: title.to_string(),
        translated_title: None,
        level: level as u8,
        order,
        page: None,
        href: None,
        source_kind: TocSourceKind::HeadingInferred,
        confidence: 0.7,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::OcrDocument;

    #[test]
    fn heading_toc_inference_keeps_titles_and_order() {
        let toc = infer_heading_toc("# Introduction\n\nBody\n\n## Method\n\n### Result")
            .expect("heading toc");

        assert_eq!(toc.source_kind, TocSourceKind::HeadingInferred);
        assert_eq!(toc.entries.len(), 3);
        assert_eq!(toc.entries[0].source_title, "Introduction");
        assert_eq!(toc.entries[1].source_title, "Method");
        assert_eq!(toc.entries[1].level, 2);
        assert_eq!(toc.entries[2].order, 6);
        assert!(toc.entries.iter().all(|entry| entry.href.is_none()));
    }

    #[test]
    fn heading_toc_inference_ignores_non_heading_lines() {
        let toc = infer_heading_toc("Title\n\n####### too deep\n\n#\n\nParagraph # marker");

        assert!(toc.is_none());
    }

    #[test]
    fn layout_toc_inference_detects_numbered_and_academic_sections() {
        let markdown = "Title page\n\nAbstract\n\nBody\n\n1 Introduction\n\nText\n\n1.1 Background";
        let toc = infer_layout_toc(markdown).expect("layout toc");

        assert_eq!(toc.source_kind, TocSourceKind::LayoutInferred);
        assert_eq!(toc.entries.len(), 3);
        assert_eq!(toc.entries[0].source_title, "Abstract");
        assert_eq!(toc.entries[1].source_title, "1 Introduction");
        assert_eq!(toc.entries[2].level, 2);
    }

    #[test]
    fn layout_toc_inference_filters_captions_and_requires_multiple_entries() {
        let markdown = "Figure 1. Example\n\nTable 2. Data\n\n1 Introduction";

        assert!(infer_layout_toc(markdown).is_none());
    }

    #[test]
    fn pdf_outline_toc_merges_markdown_headings_and_preserves_outline_pages() {
        let outline = TocArtifact {
            source_kind: TocSourceKind::PdfOutline,
            confidence: 0.9,
            entries: vec![TocEntry {
                source_title: "Introduction".to_string(),
                translated_title: None,
                level: 1,
                order: 0,
                page: Some(3),
                href: None,
                source_kind: TocSourceKind::PdfOutline,
                confidence: 0.9,
            }],
        };
        let markdown = concat!(
            "# Toolbox of Interventions Against Online Misinformation\n\n",
            "## Abstract\n\n",
            "## 1 Introduction\n\n",
            "## Declarations\n\n",
            "## References\n\n",
            "## Reprints and permissions information is available at http://www.nature.com/reprints\n"
        );

        let toc = source_toc_for_pdf(Some(outline), markdown).expect("merged toc");
        let titles = toc
            .entries
            .iter()
            .map(|entry| entry.source_title.as_str())
            .collect::<Vec<_>>();

        assert_eq!(toc.source_kind, TocSourceKind::PdfOutline);
        assert!(titles.contains(&"Toolbox of Interventions Against Online Misinformation"));
        assert!(titles.contains(&"Abstract"));
        assert!(titles.contains(&"1 Introduction"));
        assert!(titles.contains(&"Declarations"));
        assert!(titles.contains(&"References"));
        assert!(!titles
            .iter()
            .any(|title| title.contains("http://www.nature.com/reprints")));
        let introduction = toc
            .entries
            .iter()
            .find(|entry| entry.source_title == "1 Introduction")
            .expect("matched introduction");
        assert_eq!(introduction.page, Some(3));
        assert_eq!(introduction.source_kind, TocSourceKind::PdfOutline);
        assert_eq!(introduction.level, 2);
    }

    #[test]
    fn pdf_outline_toc_does_not_append_unmatched_outline_residue_when_headings_exist() {
        let outline = TocArtifact {
            source_kind: TocSourceKind::PdfOutline,
            confidence: 0.9,
            entries: vec![
                TocEntry {
                    source_title: "Introduction".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 0,
                    page: Some(3),
                    href: None,
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
                TocEntry {
                    source_title: "Supplementary information".to_string(),
                    translated_title: None,
                    level: 2,
                    order: 1,
                    page: Some(12),
                    href: None,
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
                TocEntry {
                    source_title: "Supplementary Information for the Toolbox of Interventions"
                        .to_string(),
                    translated_title: None,
                    level: 1,
                    order: 2,
                    page: Some(18),
                    href: None,
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
            ],
        };
        let markdown = concat!(
            "# Toolbox of Interventions\n\n",
            "## 1 Introduction\n\n",
            "Supplementary information. This paragraph is body text.\n\n",
            "## Appendix A Supplementary Information for the Toolbox of Interventions\n\n",
            "## References\n"
        );

        let toc = source_toc_for_pdf(Some(outline), markdown).expect("merged toc");
        let titles = toc
            .entries
            .iter()
            .map(|entry| entry.source_title.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            titles,
            vec![
                "Toolbox of Interventions",
                "1 Introduction",
                "Appendix A Supplementary Information for the Toolbox of Interventions",
                "References"
            ]
        );
        assert_eq!(
            toc.entries
                .iter()
                .find(|entry| entry.source_title == "1 Introduction")
                .and_then(|entry| entry.page),
            Some(3)
        );
    }

    #[test]
    fn json_rebuild_drops_standalone_layout_number_blocks() {
        let markdown = "Paragraph one.\n\n3\n\nParagraph two.";
        let layouts = vec![crate::ocr::OcrPageLayout {
            blocks: vec![crate::ocr::OcrLayoutBlock {
                label: "number".to_string(),
                content: "3".to_string(),
                bbox: vec![0, 0, 10, 10],
                order: None,
                page: Some(0),
            }],
        }];

        let rebuilt = drop_layout_number_blocks(markdown, &layouts);

        assert_eq!(rebuilt, "Paragraph one.\n\nParagraph two.");
    }

    #[test]
    fn json_rebuild_keeps_body_digits_inside_text_blocks() {
        let markdown = "There were 3 intervention groups in the study.";
        let layouts = vec![crate::ocr::OcrPageLayout {
            blocks: vec![crate::ocr::OcrLayoutBlock {
                label: "number".to_string(),
                content: "3".to_string(),
                bbox: vec![0, 0, 10, 10],
                order: None,
                page: Some(0),
            }],
        }];

        let rebuilt = drop_layout_number_blocks(markdown, &layouts);

        assert_eq!(rebuilt, markdown);
    }

    #[test]
    fn json_rebuild_promotes_layout_paragraph_title_blocks() {
        let markdown = "Abstract\n\nBody paragraph.\n\n1 Introduction\n\nNext paragraph.";
        let layouts = vec![crate::ocr::OcrPageLayout {
            blocks: vec![
                crate::ocr::OcrLayoutBlock {
                    label: "paragraph_title".to_string(),
                    content: "Abstract".to_string(),
                    bbox: vec![0, 0, 10, 10],
                    order: Some(1),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "paragraph_title".to_string(),
                    content: "1 Introduction".to_string(),
                    bbox: vec![0, 20, 10, 30],
                    order: Some(2),
                    page: Some(0),
                },
            ],
        }];

        let rebuilt = promote_layout_paragraph_titles(markdown, &layouts);

        assert!(rebuilt.contains("## Abstract"));
        assert!(rebuilt.contains("## 1 Introduction"));
        assert!(rebuilt.contains("Body paragraph."));
    }

    #[test]
    fn json_rebuild_does_not_double_promote_existing_heading() {
        let markdown = "## Abstract\n\nBody paragraph.";
        let layouts = vec![crate::ocr::OcrPageLayout {
            blocks: vec![crate::ocr::OcrLayoutBlock {
                label: "paragraph_title".to_string(),
                content: "Abstract".to_string(),
                bbox: vec![0, 0, 10, 10],
                order: Some(1),
                page: Some(0),
            }],
        }];

        let rebuilt = promote_layout_paragraph_titles(markdown, &layouts);

        assert_eq!(rebuilt, markdown);
    }

    #[test]
    fn json_rebuild_inserts_layout_titles_missing_from_markdown_before_anchor_text() {
        let markdown = "Title paragraph.\n\n269 participants joined the online experiment.";
        let layouts = vec![crate::ocr::OcrPageLayout {
            blocks: vec![
                crate::ocr::OcrLayoutBlock {
                    label: "paragraph_title".to_string(),
                    content: "Methods".to_string(),
                    bbox: vec![0, 0, 10, 10],
                    order: Some(1),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "paragraph_title".to_string(),
                    content: "Participants".to_string(),
                    bbox: vec![0, 20, 10, 30],
                    order: Some(2),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "269 participants joined the online experiment.".to_string(),
                    bbox: vec![0, 40, 10, 60],
                    order: Some(3),
                    page: Some(0),
                },
            ],
        }];

        let rebuilt = promote_layout_paragraph_titles(markdown, &layouts);

        assert!(rebuilt.contains(
            "## Methods\n\n## Participants\n\n269 participants joined the online experiment."
        ));
    }

    #[test]
    fn json_rebuild_does_not_replace_body_text_by_table_similarity() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-source-json-table-similarity-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("ocr_input")).expect("create input dir");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let paths = ArtifactPaths::new(&root);
        let body = "Alpha header and beta row remain body text. Gamma tail also remains body text.";
        let table_html = concat!(
            "<table>",
            "<tr><td>Alpha header</td></tr>",
            "<tr><td>beta row</td></tr>",
            "<tr><td>Gamma tail</td></tr>",
            "</table>"
        );
        let layouts = vec![crate::ocr::OcrPageLayout {
            blocks: vec![
                crate::ocr::OcrLayoutBlock {
                    label: "text".to_string(),
                    content: body.to_string(),
                    bbox: vec![20, 20, 860, 90],
                    order: Some(1),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "figure_title".to_string(),
                    content: "Table 1: Demo".to_string(),
                    bbox: vec![20, 110, 240, 130],
                    order: Some(2),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "table".to_string(),
                    content: table_html.to_string(),
                    bbox: vec![20, 140, 860, 320],
                    order: Some(3),
                    page: Some(0),
                },
            ],
        }];

        let mut events = Vec::new();
        let mut record_event = |stage: &str, message: &str, _: u8| {
            events.push((stage.to_string(), message.to_string()));
            Ok(())
        };
        let rebuilt = rebuild_pdf_markdown_from_json_stage(
            &paths,
            "stale raw markdown should not drive table matching",
            &layouts,
            &mut record_event,
        )
        .expect("json rebuild");

        assert!(rebuilt.contains(body), "{rebuilt}");
        assert_eq!(
            rebuilt
                .matches(crate::pipeline::layout::PDF_TABLE_START_MARKER)
                .count(),
            1,
            "{rebuilt}"
        );
        assert!(rebuilt.contains("Table 1: Demo"));
        assert!(rebuilt.contains("![PDF table 1](imgs/table_snapshot_001.jpg)"));
        assert!(events
            .iter()
            .any(|(_, message)| { message.contains("table_text_matching_skipped=true") }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pdf_visual_snapshot_rebuilds_source_blocks_after_markdown_change() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-source-snapshot-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("temp dir should exist");
        let paths = ArtifactPaths::new(&root);
        let original_markdown = concat!("Before\n\n", "[Image #1]\n\n", "After\n");
        let mut source_document = OcrDocument {
            markdown: original_markdown.to_string(),
            cover_data_url: None,
            page_layouts: Vec::new(),
            markdown_images: std::collections::HashMap::from([(
                "imgs/figure-1.png".to_string(),
                "aGVsbG8=".to_string(),
            )]),
            toc: None,
            epub_blocks: Vec::new(),
            source_blocks: build_source_blocks_from_markdown(original_markdown),
        };
        artifact::persist_source_document(&paths, &source_document)
            .await
            .expect("persist source document");

        let mut record_event = |_: &str, _: &str, _: u8| Ok(());
        apply_pdf_visual_snapshots(&paths, &mut source_document, &mut record_event)
            .await
            .expect("apply snapshots");

        let reloaded = artifact::load_source_document(&paths)
            .await
            .expect("load source document")
            .expect("source document should exist");
        let rebuilt_blocks = build_source_blocks_from_markdown(&reloaded.markdown);
        assert_eq!(reloaded.source_blocks, rebuilt_blocks);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pdf_visual_snapshot_does_not_duplicate_existing_pdf_table_blocks() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-source-existing-table-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("ocr_input")).expect("temp dir should exist");
        let image = image::RgbImage::from_pixel(900, 420, image::Rgb([255, 255, 255]));
        image
            .save(root.join("ocr_input/page_0.jpg"))
            .expect("save page");
        let paths = ArtifactPaths::new(&root);
        let original_markdown = concat!(
            "Table 1. Demo\n\n",
            "<!-- musetranslate:pdf-table:start -->\n",
            "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
            "<!-- musetranslate:pdf-table:end -->\n\n",
            "After\n"
        );
        let mut source_document = OcrDocument {
            markdown: original_markdown.to_string(),
            cover_data_url: None,
            page_layouts: vec![crate::ocr::OcrPageLayout {
                blocks: vec![crate::ocr::OcrLayoutBlock {
                    label: "table".to_string(),
                    content: "<table><tr><td>A</td></tr></table>".to_string(),
                    bbox: vec![20, 30, 160, 90],
                    order: Some(4),
                    page: Some(0),
                }],
            }],
            markdown_images: std::collections::HashMap::new(),
            toc: None,
            epub_blocks: Vec::new(),
            source_blocks: build_source_blocks_from_markdown(original_markdown),
        };

        let mut record_event = |_: &str, _: &str, _: u8| Ok(());
        apply_pdf_visual_snapshots(&paths, &mut source_document, &mut record_event)
            .await
            .expect("apply snapshots");

        assert_eq!(
            source_document
                .markdown
                .matches(crate::pipeline::layout::PDF_TABLE_START_MARKER)
                .count(),
            1
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
