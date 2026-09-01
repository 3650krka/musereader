use super::super::types::{ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind};
use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResumeCompatibility {
    ExactSourceHash,
    MatchingTranslatedSources,
}

impl ResumeCompatibility {
    pub(super) fn should_refresh_source_hash(self) -> bool {
        matches!(self, Self::MatchingTranslatedSources)
    }
}

pub(super) fn checkpoint_resume_compatibility(
    checkpoint: &ChunkCheckpoint,
    source_hash: &str,
    chunk_size: usize,
    system_prompt_hash: &str,
    source_chunks: &[String],
) -> Option<ResumeCompatibility> {
    if !has_resume_metadata_match(checkpoint, chunk_size, system_prompt_hash) {
        return None;
    }
    if checkpoint.source_hash == source_hash {
        return Some(ResumeCompatibility::ExactSourceHash);
    }
    translated_sources_match_current_chunks(checkpoint, source_chunks)
        .then_some(ResumeCompatibility::MatchingTranslatedSources)
}

fn has_resume_metadata_match(
    checkpoint: &ChunkCheckpoint,
    chunk_size: usize,
    system_prompt_hash: &str,
) -> bool {
    checkpoint.chunk_size == chunk_size
        && !checkpoint.system_prompt_hash.trim().is_empty()
        && checkpoint.system_prompt_hash == system_prompt_hash
}

fn translated_sources_match_current_chunks(
    checkpoint: &ChunkCheckpoint,
    source_chunks: &[String],
) -> bool {
    if checkpoint.chunks.len() != source_chunks.len() {
        return false;
    }
    let mut has_translated_chunk = false;
    for (position, chunk) in checkpoint.chunks.iter().enumerate() {
        if chunk.translated.is_none() {
            continue;
        }
        if position != chunk.index {
            return false;
        }
        let Some(current_source) = source_chunks.get(chunk.index) else {
            return false;
        };
        if chunk.source != *current_source {
            return false;
        }
        has_translated_chunk = true;
    }
    has_translated_chunk
}

pub(super) fn invalidate_polluted_translations(checkpoint: &mut ChunkCheckpoint) -> usize {
    let mut invalidated = 0usize;
    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_deref() else {
            continue;
        };
        if !super::encoding::looks_like_mojibake(translated) {
            if translated == chunk.source.as_str()
                || is_high_confidence_untranslated_body_chunk(&chunk.source, translated)
            {
                reset_chunk_for_retranslation(chunk);
                invalidated += 1;
            }
            continue;
        }
        reset_chunk_for_retranslation(chunk);
        invalidated += 1;
    }
    if invalidated > 0 {
        checkpoint.updated_at = Utc::now().to_rfc3339();
    }
    invalidated
}

fn reset_chunk_for_retranslation(chunk: &mut super::super::types::ChunkState) {
    chunk.translated = None;
    chunk.confirmed_terms.clear();
    chunk.attempts = 0;
    chunk.processing_stage = Some(ChunkProcessingStage::Pending);
    chunk.updated_at = Utc::now().to_rfc3339();
}

fn is_high_confidence_untranslated_body_chunk(source: &str, translated: &str) -> bool {
    if contains_cjk(translated) {
        return false;
    }
    if ascii_word_count(source) < 8 || source.chars().count() < 48 {
        return false;
    }
    let normalized_source = normalize_resume_text(source);
    let normalized_translated = normalize_resume_text(translated);
    if normalized_source.is_empty() || normalized_translated.is_empty() {
        return false;
    }
    if normalized_source == normalized_translated {
        return true;
    }
    let overlap = shared_prefix_len(&normalized_source, &normalized_translated)
        + shared_suffix_len(&normalized_source, &normalized_translated);
    let shorter = normalized_source.len().min(normalized_translated.len());
    overlap >= shorter.saturating_mul(9) / 10
}

fn normalize_resume_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn ascii_word_count(value: &str) -> usize {
    value
        .split_whitespace()
        .filter(|token| token.chars().any(|ch| ch.is_ascii_alphabetic()))
        .count()
}

fn contains_cjk(value: &str) -> bool {
    value.chars().any(|ch| {
        ('\u{4E00}'..='\u{9FFF}').contains(&ch)
            || ('\u{3400}'..='\u{4DBF}').contains(&ch)
            || ('\u{F900}'..='\u{FAFF}').contains(&ch)
    })
}

fn shared_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(l, r)| l == r)
        .count()
}

fn shared_suffix_len(left: &str, right: &str) -> usize {
    left.chars()
        .rev()
        .zip(right.chars().rev())
        .take_while(|(l, r)| l == r)
        .count()
}

#[cfg(test)]
mod tests {
    use super::super::super::types::ChunkState;
    use super::*;

    #[test]
    fn invalidates_long_english_chunk_with_format_only_drift() {
        let mut checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "task".to_string(),
            source_pdf_path: "demo.pdf".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: None,
            source_hash: "source".to_string(),
            chunk_size: 120,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            source_markdown_path: "source.md".to_string(),
            chunks: vec![ChunkState {
                index: 0,
                source: "Satampra Zeiros crossed the square before dawn and answered the queen in the same language.".to_string(),
                translated: Some("Satampra Zeiros crossed the square before dawn, and answered the queen in the same language.".to_string()),
                confirmed_terms: Vec::new(),
                attempts: 1,
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
            write_metrics: Default::default(),
            delta_sync: Default::default(),
        };

        let invalidated = invalidate_polluted_translations(&mut checkpoint);

        assert_eq!(invalidated, 1);
        assert!(checkpoint.chunks[0].translated.is_none());
        assert_eq!(
            checkpoint.chunks[0].processing_stage,
            Some(ChunkProcessingStage::Pending)
        );
    }

    #[test]
    fn keeps_chunk_when_translation_contains_chinese_text() {
        let mut checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "task".to_string(),
            source_pdf_path: "demo.pdf".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: None,
            source_hash: "source".to_string(),
            chunk_size: 120,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            source_markdown_path: "source.md".to_string(),
            chunks: vec![ChunkState {
                index: 0,
                source: "Satampra Zeiros crossed the square before dawn and answered the queen in the same language.".to_string(),
                translated: Some("萨坦普拉·泽罗斯在黎明前穿过广场，并用同样的语言回答了女王。".to_string()),
                confirmed_terms: Vec::new(),
                attempts: 1,
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
            write_metrics: Default::default(),
            delta_sync: Default::default(),
        };

        let invalidated = invalidate_polluted_translations(&mut checkpoint);

        assert_eq!(invalidated, 0);
        assert!(checkpoint.chunks[0].translated.is_some());
    }
}
