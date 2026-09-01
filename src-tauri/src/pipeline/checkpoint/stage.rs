use super::super::types::{ChunkProcessingStage, ChunkSegmentKind, ChunkState};

impl ChunkProcessingStage {
    fn rank(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Translated => 10,
            Self::TermConsistencyApplied => 20,
            Self::ResidualReviewed => 30,
            // 失败块停留在流水线末端之后：本轮不再参与任何后处理；
            // 恢复时由加载层重置为 Pending（见 reset_failed_chunks_for_retry）。
            Self::TranslationFailed => 35,
            Self::MergeReady => 40,
            Self::ProtectedPassthrough => 40,
        }
    }
}

pub(super) fn current_chunk_processing_stage(chunk: &ChunkState) -> ChunkProcessingStage {
    infer_legacy_chunk_stage(chunk)
}

pub(super) fn infer_legacy_chunk_stage(chunk: &ChunkState) -> ChunkProcessingStage {
    if let Some(stage) = chunk.processing_stage {
        return stage;
    }
    if chunk.segment_kind != ChunkSegmentKind::Body {
        return if chunk.translated.as_deref() == Some(chunk.source.as_str()) {
            ChunkProcessingStage::ProtectedPassthrough
        } else {
            ChunkProcessingStage::Pending
        };
    }

    if chunk.translated.is_some() {
        ChunkProcessingStage::Translated
    } else {
        ChunkProcessingStage::Pending
    }
}

pub(super) fn advance_chunk_processing_stage(
    chunk: &mut ChunkState,
    next: ChunkProcessingStage,
) -> bool {
    let current = current_chunk_processing_stage(chunk);
    if next.rank() < current.rank() {
        return false;
    }
    if chunk.processing_stage == Some(next) {
        return false;
    }
    chunk.processing_stage = Some(next);
    true
}

pub(super) fn ensure_chunk_processing_stage(chunk: &mut ChunkState) {
    if chunk.processing_stage.is_none() {
        chunk.processing_stage = Some(infer_legacy_chunk_stage(chunk));
    }
}

#[cfg(test)]
pub(super) fn can_advance_chunk_stage(
    current: ChunkProcessingStage,
    next: ChunkProcessingStage,
) -> bool {
    next.rank() >= current.rank()
}

#[cfg(test)]
pub(super) fn next_resume_target_stage(chunk: &ChunkState) -> ChunkProcessingStage {
    match infer_legacy_chunk_stage(chunk) {
        ChunkProcessingStage::Pending => ChunkProcessingStage::Pending,
        ChunkProcessingStage::Translated => ChunkProcessingStage::TermConsistencyApplied,
        ChunkProcessingStage::TermConsistencyApplied => ChunkProcessingStage::ResidualReviewed,
        ChunkProcessingStage::ResidualReviewed => ChunkProcessingStage::MergeReady,
        ChunkProcessingStage::TranslationFailed => ChunkProcessingStage::Pending,
        ChunkProcessingStage::MergeReady => ChunkProcessingStage::MergeReady,
        ChunkProcessingStage::ProtectedPassthrough => ChunkProcessingStage::MergeReady,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_body_chunks_only_infer_pending_or_translated() {
        let pending = chunk(None, ChunkSegmentKind::Body);
        let translated = chunk(Some("译文"), ChunkSegmentKind::Body);

        assert_eq!(
            infer_legacy_chunk_stage(&pending),
            ChunkProcessingStage::Pending
        );
        assert_eq!(
            infer_legacy_chunk_stage(&translated),
            ChunkProcessingStage::Translated
        );
    }

    #[test]
    fn protected_passthrough_requires_source_text_as_translation() {
        let ready = chunk(Some("source"), ChunkSegmentKind::Reference);
        let missing = chunk(None, ChunkSegmentKind::Reference);
        let mismatched = chunk(Some("changed"), ChunkSegmentKind::ProtectedMetadata);

        assert_eq!(
            infer_legacy_chunk_stage(&ready),
            ChunkProcessingStage::ProtectedPassthrough
        );
        assert_eq!(
            infer_legacy_chunk_stage(&missing),
            ChunkProcessingStage::Pending
        );
        assert_eq!(
            infer_legacy_chunk_stage(&mismatched),
            ChunkProcessingStage::Pending
        );
    }

    #[test]
    fn chunk_stage_advancement_is_monotonic() {
        assert!(can_advance_chunk_stage(
            ChunkProcessingStage::Pending,
            ChunkProcessingStage::Translated
        ));
        assert!(can_advance_chunk_stage(
            ChunkProcessingStage::Translated,
            ChunkProcessingStage::ResidualReviewed
        ));
        assert!(!can_advance_chunk_stage(
            ChunkProcessingStage::ResidualReviewed,
            ChunkProcessingStage::Translated
        ));
        assert!(!can_advance_chunk_stage(
            ChunkProcessingStage::MergeReady,
            ChunkProcessingStage::Pending
        ));
    }

    #[test]
    fn resume_target_does_not_treat_legacy_translation_as_merge_ready() {
        let translated = chunk(Some("译文"), ChunkSegmentKind::Body);
        let protected = chunk(Some("source"), ChunkSegmentKind::ProtectedMetadata);

        assert_eq!(
            next_resume_target_stage(&translated),
            ChunkProcessingStage::TermConsistencyApplied
        );
        assert_eq!(
            next_resume_target_stage(&protected),
            ChunkProcessingStage::MergeReady
        );
    }

    #[test]
    fn explicit_stage_takes_precedence_over_legacy_inference() {
        let mut chunk = chunk(Some("译文"), ChunkSegmentKind::Body);
        chunk.processing_stage = Some(ChunkProcessingStage::ResidualReviewed);

        assert_eq!(
            infer_legacy_chunk_stage(&chunk),
            ChunkProcessingStage::ResidualReviewed
        );
        assert_eq!(
            next_resume_target_stage(&chunk),
            ChunkProcessingStage::MergeReady
        );
    }

    #[test]
    fn missing_stage_is_backfilled_from_legacy_shape() {
        let mut chunk = chunk(Some("译文"), ChunkSegmentKind::Body);

        ensure_chunk_processing_stage(&mut chunk);

        assert_eq!(
            chunk.processing_stage,
            Some(ChunkProcessingStage::Translated)
        );
    }

    fn chunk(translated: Option<&str>, segment_kind: ChunkSegmentKind) -> ChunkState {
        ChunkState {
            index: 0,
            source: "source".to_string(),
            translated: translated.map(str::to_string),
            confirmed_terms: Vec::new(),
            attempts: u8::from(translated.is_some()),
            updated_at: "now".to_string(),
            segment_kind,
            processing_stage: None,
        }
    }
}
