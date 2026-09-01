use super::super::chunking::{
    is_academic_article, is_markdown_image_chunk, looks_like_protected_metadata_chunk,
    looks_like_reference_chunk, split_reference_aware_markdown, starts_reference_section,
    starts_with_non_reference_tail_heading,
};
use super::super::layout::is_pdf_table_block;
use super::super::types::{ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind, ChunkState};
use chrono::Utc;

pub(super) fn reconcile_reference_aware_chunks(
    checkpoint: &mut ChunkCheckpoint,
    source_markdown: &str,
    chunk_size: usize,
    article_type: &str,
) {
    let expected_sources =
        split_reference_aware_markdown(source_markdown, chunk_size, article_type);
    if checkpoint.chunks.len() == expected_sources.len()
        && checkpoint
            .chunks
            .iter()
            .zip(expected_sources.iter())
            .all(|(chunk, expected)| chunk.source == *expected)
    {
        return;
    }

    let now = Utc::now().to_rfc3339();
    let existing_translations = checkpoint
        .chunks
        .iter()
        .map(|chunk| {
            (
                chunk.source.clone(),
                chunk.translated.clone(),
                chunk.confirmed_terms.clone(),
                chunk.attempts,
                chunk.segment_kind,
                chunk.processing_stage,
            )
        })
        .collect::<Vec<_>>();

    checkpoint.chunks = expected_sources
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let existing = existing_translations
                .iter()
                .find(|(existing_source, _, _, _, _, _)| existing_source == &source);
            if let Some((
                _,
                translated,
                confirmed_terms,
                attempts,
                segment_kind,
                processing_stage,
            )) = existing
            {
                ChunkState {
                    index,
                    source,
                    translated: translated.clone(),
                    confirmed_terms: confirmed_terms.clone(),
                    attempts: *attempts,
                    updated_at: now.clone(),
                    segment_kind: *segment_kind,
                    processing_stage: *processing_stage,
                }
            } else {
                ChunkState {
                    index,
                    source,
                    translated: None,
                    confirmed_terms: Vec::new(),
                    attempts: 0,
                    updated_at: now.clone(),
                    segment_kind: ChunkSegmentKind::Body,
                    processing_stage: Some(ChunkProcessingStage::Pending),
                }
            }
        })
        .collect();
    checkpoint.updated_at = now;
}

pub(super) fn apply_reference_chunk_policy(checkpoint: &mut ChunkCheckpoint) {
    let mut in_reference_section = false;
    for chunk in &mut checkpoint.chunks {
        if matches!(
            chunk.segment_kind,
            ChunkSegmentKind::ProtectedMetadata | ChunkSegmentKind::PdfTable
        ) {
            continue;
        }

        if is_pdf_table_block(&chunk.source) {
            chunk.segment_kind = ChunkSegmentKind::PdfTable;
            chunk.translated = Some(chunk.source.clone());
            chunk.confirmed_terms.clear();
            chunk.processing_stage = Some(ChunkProcessingStage::ProtectedPassthrough);
            continue;
        }

        if is_markdown_image_chunk(&chunk.source) {
            chunk.segment_kind = ChunkSegmentKind::ProtectedMetadata;
            chunk.translated = Some(chunk.source.clone());
            chunk.confirmed_terms.clear();
            chunk.processing_stage = Some(ChunkProcessingStage::ProtectedPassthrough);
            continue;
        }

        if is_academic_article(&checkpoint.article_type)
            && looks_like_protected_metadata_chunk(&chunk.source)
        {
            chunk.segment_kind = ChunkSegmentKind::ProtectedMetadata;
            chunk.translated = Some(chunk.source.clone());
            chunk.confirmed_terms.clear();
            chunk.processing_stage = Some(ChunkProcessingStage::ProtectedPassthrough);
            continue;
        }

        if in_reference_section && starts_with_non_reference_tail_heading(&chunk.source) {
            in_reference_section = false;
        }

        if starts_reference_section(&chunk.source) {
            in_reference_section = true;
        }

        if in_reference_section || looks_like_reference_chunk(&chunk.source) {
            chunk.segment_kind = ChunkSegmentKind::Reference;
            chunk.translated = Some(chunk.source.clone());
            chunk.confirmed_terms.clear();
            chunk.processing_stage = Some(ChunkProcessingStage::ProtectedPassthrough);
        } else {
            chunk.segment_kind = ChunkSegmentKind::Body;
        }
    }
}
