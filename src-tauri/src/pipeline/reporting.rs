use super::artifact;
use super::render::{
    render_translated_html_from_blocks_with_authors, render_translated_html_with_authors,
};
use super::structure;
use super::{
    ensure_not_cancelled, AppError, ArtifactPaths, ChunkCheckpoint, PipelineProgressStage,
    PipelineResult, PipelineRunConfig, RuntimePromptConfig, TranslationPipeline,
};
use crate::document::TocArtifact;
use crate::document::{
    build_source_blocks_from_markdown, build_translated_blocks_with_chunk_markdowns,
    detect_document_kind, SourceBlock, SourceDocumentKind, TranslatedBlock,
};
use crate::pipeline::render::authors::AcademicFrontMatter;

// 主翻译窗口内预取 front_matter 译文的后台任务句柄（方案 B：与主翻译并行 + 落盘供断点复用）。
pub(super) type FrontMatterPrefetch =
    tokio::task::JoinHandle<Result<AcademicFrontMatter, AppError>>;

pub(super) async fn finalize_translation_outputs(
    pipeline: &TranslationPipeline,
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    checkpoint: &ChunkCheckpoint,
    source_hash: &str,
    translated_markdown: &str,
    source_front_matter: &AcademicFrontMatter,
    front_matter_prefetch: Option<FrontMatterPrefetch>,
    cover_data_url: Option<String>,
    mut record_event: impl FnMut(&str, &str, u8) -> Result<(), AppError>,
    progress_callback: &impl Fn(PipelineProgressStage, &str, u8),
) -> Result<PipelineResult, AppError> {
    ensure_not_cancelled(&config.cancellation_token)?;
    let toc = artifact::read_json::<TocArtifact>(&paths.source_toc_path).ok();
    let source_markdown = std::fs::read_to_string(&paths.source_markdown_path).ok();
    let translated_front_matter = build_translated_front_matter(
        pipeline,
        paths,
        source_front_matter,
        front_matter_prefetch,
        &mut record_event,
    )
    .await?;
    artifact::write_json(&paths.source_front_matter_path, source_front_matter).await?;
    artifact::write_json(
        &paths.translated_front_matter_path,
        &translated_front_matter,
    )
    .await?;
    artifact::write_json(&paths.front_matter_path, &translated_front_matter).await?;

    let final_markdown = build_user_facing_markdown(translated_markdown, &translated_front_matter);
    artifact::write_utf8(&paths.output_markdown_path, &final_markdown).await?;
    let translated_chunks = artifact::build_translated_chunk_artifact(checkpoint);
    artifact::write_json(&paths.translated_chunks_path, &translated_chunks).await?;
    let source_blocks = artifact::read_json::<Vec<SourceBlock>>(&paths.source_blocks_path)
        .ok()
        .unwrap_or_else(|| {
            build_source_blocks_from_markdown(source_markdown.as_deref().unwrap_or_default())
        });
    let source_block_chunk_indexes = source_markdown
        .as_deref()
        .map(|markdown| map_source_blocks_to_chunk_indexes(markdown, &source_blocks, checkpoint))
        .unwrap_or_default();
    let translated_chunk_markdowns = checkpoint
        .chunks
        .iter()
        .filter_map(|chunk| {
            chunk
                .translated
                .as_ref()
                .map(|translated| (chunk.index, translated.clone()))
        })
        .collect::<Vec<_>>();
    let translated_blocks = build_translated_blocks_with_chunk_markdowns(
        &source_blocks,
        &final_markdown,
        &source_block_chunk_indexes,
        &translated_chunk_markdowns,
    );
    artifact::write_json(&paths.translated_blocks_path, &translated_blocks).await?;
    let translated_toc = if let Some(toc) = toc.as_ref() {
        let translated_toc = artifact::build_translated_toc_artifact(
            toc,
            source_markdown.as_deref().unwrap_or_default(),
            &final_markdown,
            &source_blocks,
            &translated_blocks,
        );
        artifact::write_json(&paths.translated_toc_path, &translated_toc).await?;
        Some(translated_toc)
    } else {
        None
    };

    record_event("rendering", "rendering HTML", 94)?;
    progress_callback(PipelineProgressStage::Rendering, "rendering HTML", 94);
    let translated_html = build_translated_html(paths, &final_markdown, &translated_front_matter)?;
    artifact::write_utf8(&paths.output_html_path, &translated_html).await?;

    record_event("validating", "building validation report", 97)?;
    progress_callback(
        PipelineProgressStage::WritingArtifacts,
        "building validation report",
        97,
    );
    let validation_report = artifact::build_validation_report(&config.task_id, checkpoint);
    artifact::write_json(&paths.validation_report_path, &validation_report).await?;
    record_event("validation_report_written", "validation report written", 97)?;

    record_event("glossary_start", "building glossary artifact", 98)?;
    let glossary = artifact::build_glossary_artifact(&runtime_prompt.article_type, checkpoint);
    artifact::write_json(&paths.glossary_path, &glossary).await?;
    record_event("glossary_done", "glossary artifact written", 98)?;

    record_event("metrics_start", "building metrics artifact", 99)?;
    let metrics_started = std::time::Instant::now();
    let metrics = artifact::build_metrics_artifact(
        checkpoint,
        &validation_report,
        &glossary,
        Some(&paths.event_log_path),
        toc.as_ref(),
        translated_toc.as_ref(),
        source_markdown.as_deref(),
        Some(&final_markdown),
        Some(pipeline.llm_client.usage_totals()),
    )?;
    artifact::write_json(&paths.metrics_path, &metrics).await?;
    record_event(
        "metrics_done",
        &format!(
            "metrics artifact written; elapsed_ms={}",
            metrics_started.elapsed().as_millis()
        ),
        99,
    )?;

    // EPUB 打包：仅当源文档是 EPUB 时，用内容锚定把译文写回原 EPUB 结构，
    // 产出中文版（book_zh.epub）和双语对照版（book_bilingual.epub）。
    // PDF 源文档跳过（无 EPUB 结构可写回）。
    if matches!(
        detect_document_kind(&config.pdf_path).ok(),
        Some(SourceDocumentKind::Epub)
    ) {
        record_event("epub_pack_started", "packing translated EPUB", 99)?;
        progress_callback(PipelineProgressStage::WritingArtifacts, "packing EPUB", 99);
        let pack_result = pack_translated_epub(
            config,
            paths,
            &translated_blocks,
            translated_toc.as_ref(),
            &mut record_event,
        );
        if let Err(error) = pack_result {
            // EPUB 打包失败不阻塞主流程（markdown/html 产物已就绪），记录后继续。
            let _ = record_event(
                "epub_pack_failed",
                &format!("EPUB pack failed (non-fatal): {error:?}"),
                99,
            );
        }
    }

    // manifest 必须在 EPUB 打包之后生成，才能把最终成书纳入工件清单。
    record_event("manifest_start", "building manifest artifact", 99)?;
    let manifest = artifact::build_manifest(
        config,
        runtime_prompt,
        paths,
        checkpoint,
        &validation_report,
        &glossary,
    )?;
    artifact::write_json(&paths.manifest_path, &manifest).await?;
    record_event("manifest_done", "manifest artifact written", 99)?;

    record_event("completed", "translation completed", 100)?;
    progress_callback(
        PipelineProgressStage::Completed,
        "translation completed",
        100,
    );

    Ok(build_pipeline_result(
        checkpoint,
        paths,
        source_hash,
        &final_markdown,
        &translated_html,
        cover_data_url,
    ))
}

/// 用内容锚定把译文写回原 EPUB，产出中文版和双语对照版。
/// 归章逻辑：逻辑段 = 干净章标题之间的全部块；spine 章 = 全部保留（含 back/front matter）。
fn pack_translated_epub(
    config: &PipelineRunConfig,
    paths: &ArtifactPaths,
    translated_blocks: &[TranslatedBlock],
    translated_toc: Option<&TocArtifact>,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let chapters = crate::document::extract_epub_chapters(&config.pdf_path)?;

    // 归章键：heading 的 chapterTitle 可能被重写成「人物名+内联图」（如 'Tash ![](…)'），
    // 与干净目录标题（'Chapter 1: Tash'）割裂。用「最近的干净标题」作为每块的逻辑章：
    // 干净的（不含图片/较短的）标题出现即开启新章，重写形式并入上一章。
    let is_clean_title = |ct: &Option<String>, kind: &str| {
        kind == "heading"
            && ct
                .as_deref()
                .map(|t| {
                    !t.contains("![") && !t.contains("imgs/") && t.trim().chars().count() <= 60
                })
                .unwrap_or(false)
    };
    let mut segments: Vec<(Option<String>, Vec<TranslatedBlock>)> = Vec::new();
    for b in translated_blocks.iter().cloned() {
        let start_new = is_clean_title(&b.chapter_title, &b.kind) || segments.is_empty();
        if start_new {
            segments.push((b.chapter_title.clone(), vec![b]));
        } else {
            segments.last_mut().unwrap().1.push(b);
        }
    }

    // 归章候选章：保留全部 spine 章（含低词数的 back/front matter）。
    let content_chapters: Vec<_> = chapters.iter().collect();
    // 归章：段与内容章节按序对齐（指针单调前移，保 spine 顺序）。
    let norm = |s: &str| {
        s.split("![")
            .next()
            .unwrap_or(s)
            .trim()
            .trim_start_matches('#')
            .trim()
            .to_ascii_lowercase()
    };
    let mut pack_chapters = Vec::new();
    let mut cursor = 0usize;
    for (seg_title, seg_blocks) in segments {
        if seg_blocks
            .iter()
            .all(|b| b.source_text.trim().is_empty() && b.translated_text.trim().is_empty())
        {
            continue;
        }
        let seg_block_count = seg_blocks.len();
        let mut best: Option<(usize, i64)> = None;
        for i in cursor..content_chapters.len() {
            let ch = content_chapters[i];
            let ctn = norm(&ch.title);
            let mut score = 0i64;
            if let Some(st) = seg_title.as_deref() {
                let stn = norm(st);
                if !stn.is_empty() && !ctn.is_empty() {
                    if ctn == stn {
                        score += 1000;
                    } else if ctn.contains(&stn) || stn.contains(&ctn) {
                        score += 500;
                    }
                }
            }
            let ch_block_count = ch.blocks.len().max(1);
            let size_diff = (ch_block_count as i64 - seg_block_count as i64).abs();
            score -= size_diff.min(200);
            if score > 0 && best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((i, score));
            }
        }
        let i = best.map(|(i, _)| i).unwrap_or(cursor);
        if i < content_chapters.len() {
            pack_chapters.push(crate::document::pack::EpubPackChapter {
                translated: seg_blocks,
            });
            cursor = i + 1;
        }
    }
    if pack_chapters.is_empty() {
        return Err(AppError::internal("no chapter matched translated blocks"));
    }

    // 书名优先取 OPF dc:title；env 可覆盖。
    let book_title = crate::document::pack::read_opf_title(&config.pdf_path)
        .unwrap_or_else(|| "Translated Book".to_string());
    let toc_entries: Vec<crate::document::TocEntry> = translated_toc
        .map(|t| t.entries.clone())
        .unwrap_or_default();

    let output_dir = paths
        .bilingual_epub_path
        .parent()
        .ok_or_else(|| AppError::internal("invalid EPUB output directory"))?;
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| AppError::internal(format!("create pack output dir failed: {e}")))?;
    let zh_out = &paths.translated_epub_path;
    let bi_out = &paths.bilingual_epub_path;

    let report_zh = crate::document::pack::write_epub(
        &config.pdf_path,
        zh_out,
        crate::document::pack::EpubPackMode::Chinese,
        &pack_chapters,
        &book_title,
        &toc_entries,
    )?;
    let report_bi = crate::document::pack::write_epub(
        &config.pdf_path,
        bi_out,
        crate::document::pack::EpubPackMode::Bilingual,
        &pack_chapters,
        &book_title,
        &toc_entries,
    )?;

    record_event(
        "epub_pack_done",
        &format!(
            "EPUB packed; zh={} bi={} aligned_blocks={} chapters={} spine_files={} unplaced_blocks_zh={} unplaced_blocks_bi={}",
            zh_out.display(),
            bi_out.display(),
            report_zh.aligned_blocks,
            report_bi.chapters,
            report_zh.spine_content_paths,
            report_zh.unplaced_blocks,
            report_bi.unplaced_blocks,
        ),
        99,
    )?;
    Ok(())
}

#[cfg(test)]
mod markdown_cleanup_regression_tests {
    use super::*;

    #[test]
    fn user_facing_markdown_removes_boundary_inserted_duplicate_heading() {
        let markdown = [
            "# Previous Story",
            "",
            "Body text.",
            "",
            "# To Demon",
            "",
            "# To Daemon",
            "",
            "Tell me many tales.",
        ]
        .join("\n");

        let visible = build_user_facing_markdown(&markdown, &AcademicFrontMatter::default());

        assert!(!visible.contains("# To Demon\n\n# To Daemon"));
        assert!(!visible.contains("# To Demon"));
        assert!(visible.contains("# To Daemon\n\nTell me many tales."));
    }

    #[test]
    fn user_facing_markdown_removes_heading_followed_by_split_title_text() {
        let markdown = [
            "# Appendix One: Story Notes",
            "",
            "Appendix One:",
            "Story Notes",
            "",
            "The notes begin here.",
        ]
        .join("\n");

        let visible = build_user_facing_markdown(&markdown, &AcademicFrontMatter::default());

        assert_eq!(visible.matches("Appendix One").count(), 1);
        assert!(visible.contains("# Appendix One: Story Notes\n\nThe notes begin here."));
    }
}

#[cfg(test)]
mod front_matter_title_tests {
    use super::*;

    #[test]
    fn user_facing_markdown_prefers_front_matter_title_without_repeating_body_title() {
        let markdown = [
            crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START,
            "# Original English Title",
            crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END,
            "",
            "# 中文标题",
            "",
            "## 摘要",
            "",
            "## 摘要：研究设计",
            "",
            "正文。",
        ]
        .join("\n");
        let front_matter = AcademicFrontMatter {
            title: Some("中文标题".to_string()),
            authors: vec![crate::pipeline::render::authors::AcademicAuthor {
                name: "Author One".to_string(),
                email: None,
                markers: Vec::new(),
                affiliations: vec!["University A".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            }],
            ..AcademicFrontMatter::default()
        };

        let visible = build_user_facing_markdown(&markdown, &front_matter);

        assert_eq!(visible.matches("# 中文标题").count(), 1);
        assert!(!visible.contains("Original English Title"));
        assert_eq!(visible.matches("## 摘要").count(), 1);
        assert!(visible.contains("正文。"));
    }
}

fn build_user_facing_markdown(markdown: &str, front_matter: &AcademicFrontMatter) -> String {
    if !crate::pipeline::render::authors::has_academic_front_matter(front_matter) {
        return remove_adjacent_duplicate_headings(markdown);
    }

    let body = strip_explicit_front_matter_region(markdown);
    let body = remove_adjacent_duplicate_headings(body.trim_start());
    let Some(title) = front_matter
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    else {
        return body;
    };

    if first_markdown_heading(&body)
        .map(|heading| normalize_heading_text(heading) == normalize_heading_text(title))
        .unwrap_or(false)
    {
        return body;
    }

    if body.is_empty() {
        format!("# {title}")
    } else {
        format!("# {title}\n\n{body}")
    }
}

fn remove_adjacent_duplicate_headings(markdown: &str) -> String {
    let blocks = split_markdown_blocks(markdown);
    if blocks.len() < 2 {
        return markdown.to_string();
    }

    let mut output: Vec<String> = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        if let Some(previous) = output.last() {
            let next = blocks.get(index + 1).map(String::as_str);
            if should_replace_previous_adjacent_heading(previous, block, next) {
                output.pop();
                output.push(block.clone());
                continue;
            }
            if should_drop_adjacent_heading(previous, block) {
                continue;
            }
            if should_drop_title_text_block(previous, block) {
                continue;
            }
        }
        output.push(block.clone());
    }
    output.join("\n\n")
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn should_drop_adjacent_heading(previous: &str, current: &str) -> bool {
    let Some((previous_level, previous_title)) = parse_heading_block(previous) else {
        return false;
    };
    let Some((current_level, current_title)) = parse_heading_block(current) else {
        return false;
    };
    if previous_level != current_level {
        return false;
    }
    headings_are_near_duplicates(previous_title, current_title)
}

fn should_replace_previous_adjacent_heading(
    previous: &str,
    current: &str,
    next: Option<&str>,
) -> bool {
    let Some((previous_level, previous_title)) = parse_heading_block(previous) else {
        return false;
    };
    let Some((current_level, current_title)) = parse_heading_block(current) else {
        return false;
    };
    if previous_level != current_level || previous_level > 2 {
        return false;
    }
    if next.is_none_or(|block| parse_heading_block(block).is_some()) {
        return false;
    }
    is_short_title(previous_title) && is_short_title(current_title)
}

fn should_drop_title_text_block(previous: &str, current: &str) -> bool {
    let Some((_, previous_title)) = parse_heading_block(previous) else {
        return false;
    };
    if parse_heading_block(current).is_some() || current.lines().count() > 3 {
        return false;
    }
    let current_title = current
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if current_title.is_empty() || looks_like_prose(&current_title) {
        return false;
    }
    headings_are_near_duplicates(previous_title, &current_title)
}

fn parse_heading_block(block: &str) -> Option<(usize, &str)> {
    let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let title = line.get(level..)?.trim();
    if title.is_empty() {
        return None;
    }
    Some((level, title))
}

fn is_short_title(title: &str) -> bool {
    title.chars().filter(|ch| !ch.is_whitespace()).count() <= 40
}

fn looks_like_prose(text: &str) -> bool {
    text.chars().count() > 80
        || text.contains('.')
        || text.contains('!')
        || text.contains('?')
        || text.contains('\u{3002}')
        || text.contains('\u{ff01}')
        || text.contains('\u{ff1f}')
}

fn headings_are_near_duplicates(left: &str, right: &str) -> bool {
    let left_key = normalize_heading_similarity_key(left);
    let right_key = normalize_heading_similarity_key(right);
    if left_key.is_empty() || right_key.is_empty() {
        return false;
    }
    left_key == right_key || left_key.contains(&right_key) || right_key.contains(&left_key)
}

fn normalize_heading_similarity_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .collect::<String>()
        .to_ascii_lowercase()
}

fn strip_explicit_front_matter_region(markdown: &str) -> String {
    let start_marker = crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START;
    let end_marker = crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END;
    let Some(start) = markdown.find(start_marker) else {
        return markdown.to_string();
    };
    let search_after_start = start + start_marker.len();
    let Some(relative_end) = markdown[search_after_start..].find(end_marker) else {
        return markdown
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed != start_marker && trimmed != end_marker
            })
            .collect::<Vec<_>>()
            .join("\n");
    };
    let end = search_after_start + relative_end + end_marker.len();
    let mut stripped = String::new();
    stripped.push_str(&markdown[..start]);
    stripped.push_str(&markdown[end..]);
    stripped.trim_start_matches('\n').to_string()
}

fn first_markdown_heading(markdown: &str) -> Option<&str> {
    markdown.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|heading| !heading.is_empty())
    })
}

fn normalize_heading_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_translated_html(
    paths: &ArtifactPaths,
    markdown: &str,
    translated_front_matter: &AcademicFrontMatter,
) -> Result<String, AppError> {
    let translated_blocks =
        artifact::read_json::<Vec<TranslatedBlock>>(&paths.translated_blocks_path)
            .ok()
            .unwrap_or_default();
    Ok(render_translated_html_from_blocks_with_authors(
        markdown,
        &translated_blocks,
        translated_front_matter,
    )
    .unwrap_or_else(|| render_translated_html_with_authors(markdown, translated_front_matter)))
}

pub(super) fn map_source_blocks_to_chunk_indexes(
    source_markdown: &str,
    source_blocks: &[SourceBlock],
    checkpoint: &ChunkCheckpoint,
) -> Vec<Option<usize>> {
    let chunk_spans = locate_chunk_source_spans(source_markdown, checkpoint);
    if chunk_spans.is_empty() {
        return vec![None; source_blocks.len()];
    }

    let block_spans = locate_source_block_spans(source_markdown, source_blocks);
    block_spans
        .iter()
        .map(|span| {
            span.clone()
                .and_then(|span| best_overlapping_chunk_index(span, &chunk_spans))
        })
        .collect()
}

fn locate_chunk_source_spans(
    source_markdown: &str,
    checkpoint: &ChunkCheckpoint,
) -> Vec<(usize, std::ops::Range<usize>)> {
    let mut chunks = checkpoint.chunks.iter().collect::<Vec<_>>();
    chunks.sort_by_key(|chunk| chunk.index);
    let mut cursor = 0usize;
    let mut spans = Vec::with_capacity(chunks.len());

    for chunk in chunks {
        let Some(span) = locate_markdown_fragment(source_markdown, &chunk.source, cursor) else {
            continue;
        };
        cursor = span.end;
        spans.push((chunk.index, span));
    }
    spans
}

fn locate_source_block_spans(
    source_markdown: &str,
    source_blocks: &[SourceBlock],
) -> Vec<Option<std::ops::Range<usize>>> {
    let mut cursor = 0usize;
    source_blocks
        .iter()
        .map(|block| {
            // 块按文档顺序排列，cursor 单调推进即可（与 chunk span 共用同一定位原语，
            // 块自身无 chunk 的尾部换行问题，正常路径即精确命中）。
            let span = locate_markdown_fragment(source_markdown, &block.markdown, cursor);
            if let Some(span) = span.as_ref() {
                cursor = span.end;
            }
            span
        })
        .collect()
}

fn locate_markdown_fragment(
    markdown: &str,
    fragment: &str,
    cursor: usize,
) -> Option<std::ops::Range<usize>> {
    if fragment.is_empty() || cursor > markdown.len() {
        return None;
    }
    if let Some(relative_start) = markdown[cursor..].find(fragment) {
        let start = cursor + relative_start;
        return Some(start..start + fragment.len());
    }
    // chunk.source 由 split_markdown 逐行 push('\n') 生成，末行常带尾随换行；而 sanitize 后
    // 的 source.md 可能以非换行符结尾（或段间换行被归一）。字节精确匹配因此失配，导致
    // 该 chunk 的所有 block 退化为 global 重对齐并错配（如 playgroup 段）。此处做尾随
    // 空白容差：去掉 fragment 末尾空白后再定位，映射回真实字节区间。
    let trimmed = fragment.trim_end();
    if trimmed.len() != fragment.len() && !trimmed.is_empty() {
        if let Some(relative_start) = markdown[cursor..].find(trimmed) {
            let start = cursor + relative_start;
            return Some(start..start + trimmed.len());
        }
    }
    // 恢复性兜底：trimmed 仍未命中时，退化为按 fragment 首个非空行定位，避免级联错配。
    // 仅限 chunk span（多行整块）使用；单行 block 的 first_line==自身，已由精确匹配覆盖。
    let first_line = fragment
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty());
    if let Some(first_line) = first_line {
        if let Some(relative_start) = markdown[cursor..].find(first_line) {
            let start = cursor + relative_start;
            return Some(start..start + first_line.len());
        }
    }
    None
}

fn best_overlapping_chunk_index(
    block_span: std::ops::Range<usize>,
    chunk_spans: &[(usize, std::ops::Range<usize>)],
) -> Option<usize> {
    chunk_spans
        .iter()
        .filter_map(|(chunk_index, chunk_span)| {
            let overlap_start = block_span.start.max(chunk_span.start);
            let overlap_end = block_span.end.min(chunk_span.end);
            (overlap_end > overlap_start).then(|| (*chunk_index, overlap_end - overlap_start))
        })
        .max_by_key(|(_, overlap)| *overlap)
        .map(|(chunk_index, _)| chunk_index)
}

async fn build_translated_front_matter(
    pipeline: &TranslationPipeline,
    paths: &ArtifactPaths,
    source_front_matter: &AcademicFrontMatter,
    front_matter_prefetch: Option<FrontMatterPrefetch>,
    record_event: &mut impl FnMut(&str, &str, u8) -> Result<(), AppError>,
) -> Result<AcademicFrontMatter, AppError> {
    if !crate::pipeline::render::authors::has_academic_front_matter(source_front_matter) {
        return Ok(source_front_matter.clone());
    }

    // 断点续跑：已有预取落盘译文时直接复用，跳过 LLM（幂等恢复）。
    if front_matter_prefetch.is_none() {
        if let Ok(cached) =
            artifact::read_json::<AcademicFrontMatter>(&paths.translated_front_matter_path)
        {
            if translated_front_matter_matches_source(source_front_matter, &cached) {
                record_event(
                    "front_matter_translate_reused",
                    "reused persisted translated front matter from prior run",
                    94,
                )?;
                return Ok(cached);
            }
        }
    }

    if let Some(handle) = front_matter_prefetch {
        record_event(
            "front_matter_translate",
            "joining prefetched front matter translation",
            94,
        )?;
        let translated = handle.await.map_err(|error| {
            AppError::internal(format!("front matter prefetch task join failed: {error}"))
        })??;
        // 落盘预取结果，供断点续跑复用（finalize 处另有统一写出，此处提前保障中断场景）。
        let _ = artifact::write_json(&paths.translated_front_matter_path, &translated).await;
        return Ok(translated);
    }

    record_event(
        "front_matter_translate",
        "translating structured front matter",
        94,
    )?;
    structure::translate_front_matter_with_llm(&pipeline.llm_client, source_front_matter).await
}

// 判断持久化的 front_matter 译文是否仍对应当前源 front matter（标题与作者集合一致即可信）。
fn translated_front_matter_matches_source(
    source: &AcademicFrontMatter,
    translated: &AcademicFrontMatter,
) -> bool {
    if !crate::pipeline::render::authors::has_academic_front_matter(translated) {
        return false;
    }
    if source.authors.len() != translated.authors.len() {
        return false;
    }
    // 译文应非空且与源不同（标题被真正翻译过才算有效；源为空标题时按无 front matter 早退）。
    match (source.title.as_deref(), translated.title.as_deref()) {
        (Some(src), Some(dst)) => !src.trim().is_empty() && !dst.trim().is_empty(),
        _ => true,
    }
}

fn build_pipeline_result(
    checkpoint: &ChunkCheckpoint,
    paths: &ArtifactPaths,
    source_hash: &str,
    translated_markdown: &str,
    translated_html: &str,
    cover_data_url: Option<String>,
) -> PipelineResult {
    PipelineResult {
        translated_markdown: translated_markdown.to_string(),
        translated_html: translated_html.to_string(),
        cover_data_url,
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        output_markdown_path: paths.output_markdown_path.to_string_lossy().to_string(),
        output_html_path: paths.output_html_path.to_string_lossy().to_string(),
        translated_epub_path: paths
            .translated_epub_path
            .exists()
            .then(|| paths.translated_epub_path.to_string_lossy().to_string()),
        bilingual_epub_path: paths
            .bilingual_epub_path
            .exists()
            .then(|| paths.bilingual_epub_path.to_string_lossy().to_string()),
        artifact_manifest_path: paths.manifest_path.to_string_lossy().to_string(),
        checkpoint_path: paths.checkpoint_path.to_string_lossy().to_string(),
        event_log_path: paths.event_log_path.to_string_lossy().to_string(),
        validation_report_path: paths.validation_report_path.to_string_lossy().to_string(),
        glossary_path: paths.glossary_path.to_string_lossy().to_string(),
        metrics_path: paths.metrics_path.to_string_lossy().to_string(),
        source_hash: source_hash.to_string(),
        translated_chunks: checkpoint
            .chunks
            .iter()
            .filter(|chunk| chunk.translated.is_some())
            .count(),
        total_chunks: checkpoint.chunks.len(),
        failed_chunks: super::failed_body_chunk_indexes(checkpoint).len(),
        failed_chunk_indexes: super::failed_body_chunk_indexes(checkpoint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_markdown_removes_explicit_front_matter_region() {
        let markdown = format!(
            "{}\n# Original English Title\n\nAuthor One\n\nUniversity A\n{}\n\n## 摘要\n\n正文。",
            crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START,
            crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END
        );
        let front_matter = AcademicFrontMatter {
            title: Some("中文标题".to_string()),
            authors: vec![crate::pipeline::render::authors::AcademicAuthor {
                name: "Author One".to_string(),
                email: None,
                markers: Vec::new(),
                affiliations: vec!["University A".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            }],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let visible = build_user_facing_markdown(&markdown, &front_matter);

        assert!(visible.starts_with("# 中文标题\n\n## 摘要"));
        assert!(!visible.contains("musetranslate:front-matter"));
        assert!(!visible.contains("Original English Title"));
        assert!(!visible.contains("University A"));
    }

    #[test]
    fn user_facing_markdown_removes_adjacent_duplicate_headings_only() {
        let markdown = [
            "## 双过程理论与助推升级版",
            "",
            "## 双过程理论与助推升级版",
            "",
            "正文。",
            "",
            "## 结论",
            "",
            "另一段正文。",
            "",
            "## 结论",
        ]
        .join("\n");

        let visible = build_user_facing_markdown(&markdown, &AcademicFrontMatter::default());

        assert_eq!(visible.matches("## 双过程理论与助推升级版").count(), 1);
        assert_eq!(visible.matches("## 结论").count(), 2);
    }

    #[test]
    fn user_facing_markdown_removes_adjacent_contained_heading_duplicate() {
        let markdown = [
            "## 双过程理论与助推升级版",
            "",
            "## 双过程理论与助推升级版：认知框架",
            "",
            "正文。",
        ]
        .join("\n");

        let visible = build_user_facing_markdown(&markdown, &AcademicFrontMatter::default());

        assert_eq!(visible.matches("## ").count(), 1);
        assert!(visible.contains("## 双过程理论与助推升级版"));
    }

    #[test]
    fn locate_markdown_fragment_matches_chunk_with_trailing_newline_at_eof() {
        // 复现 playgroup 事故：末尾 chunk.source 带尾随 '\n'，而源文以非换行符结尾。
        let source = "第一段。\n\n末尾一段，没有尾换行。";
        let chunk_source = "末尾一段，没有尾换行。\n";
        let cursor = source.find("末尾").unwrap();
        let span = locate_markdown_fragment(source, chunk_source, cursor)
            .expect("trailing-newline chunk should still locate");
        assert_eq!(&source[span.clone()], "末尾一段，没有尾换行。");
    }

    #[test]
    fn locate_markdown_fragment_prefers_exact_match() {
        let source = "alpha\n\nbeta\n\ngamma";
        // 精确命中存在时不得触发容差路径。
        let span = locate_markdown_fragment(source, "beta", 0).expect("exact match");
        assert_eq!(&source[span], "beta");
    }

    #[test]
    fn chunk_source_spans_recover_last_chunk_via_trimmed_match() {
        let source = "甲段。\n\n乙段。\n\n末尾丙段。";
        let chunks = vec![
            chunk_state_for_span_test(0, "甲段。\n\n乙段。\n\n"),
            chunk_state_for_span_test(1, "末尾丙段。\n"),
        ];
        let checkpoint = checkpoint_for_span_test(chunks);
        let spans = locate_chunk_source_spans(source, &checkpoint);
        assert_eq!(spans.len(), 2, "both chunks should locate, got {spans:?}");
        assert_eq!(spans[1].0, 1);
        assert_eq!(&source[spans[1].1.clone()], "末尾丙段。");
    }

    fn chunk_state_for_span_test(index: usize, source: &str) -> crate::pipeline::types::ChunkState {
        crate::pipeline::types::ChunkState {
            index,
            source: source.to_string(),
            translated: None,
            confirmed_terms: Vec::new(),
            attempts: 0,
            updated_at: String::new(),
            segment_kind: crate::pipeline::types::ChunkSegmentKind::Body,
            processing_stage: None,
        }
    }

    fn checkpoint_for_span_test(
        chunks: Vec<crate::pipeline::types::ChunkState>,
    ) -> crate::pipeline::types::ChunkCheckpoint {
        crate::pipeline::types::ChunkCheckpoint {
            version: 1,
            task_id: "span-test".to_string(),
            source_pdf_path: String::new(),
            article_type: "fiction".to_string(),
            system_prompt_hash: String::new(),
            skill_ids: Vec::new(),
            translation_context: None,
            source_hash: String::new(),
            chunk_size: 3000,
            created_at: String::new(),
            updated_at: String::new(),
            source_markdown_path: String::new(),
            chunks,
            write_metrics: Default::default(),
            delta_sync: Default::default(),
        }
    }
}
