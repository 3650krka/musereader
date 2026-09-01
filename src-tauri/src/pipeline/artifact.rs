use super::*;

mod io;
mod report;
mod source;
mod toc;

pub(super) use io::{
    append_checkpoint_wal, append_json_line, checkpoint_flush_interval, ensure_dir,
    persist_checkpoint, read_json, read_json_lines, write_json, write_utf8, CheckpointWalEntry,
    CHECKPOINT_FLUSH_MIN_INTERVAL_SECS,
};
pub(super) use report::{
    build_glossary_artifact, build_manifest, build_metrics_artifact,
    build_translated_chunk_artifact, build_validation_report,
};
pub(super) use source::{
    load_source_document, persist_markdown_images_only, persist_source_document,
};
pub(super) use toc::{
    build_translated_toc_artifact, ordered_heading_anchors, unique_heading_anchor,
};

#[cfg(test)]
mod artifact_tests {
    use super::*;
    use crate::document::{TocArtifact, TocEntry, TocSourceKind};
    use crate::ocr::OcrDocument;
    use crate::ocr::{OcrLayoutBlock, OcrPageLayout};
    use base64::Engine as _;
    use std::collections::HashMap;

    #[test]
    fn strict_proper_noun_requires_exact_target_translation() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Satampra Zeiros",
            "萨坦普拉路泽罗斯",
            "Satampra Zeiros entered the city.",
            "泽罗斯进入了城市。",
        );

        let report = build_validation_report("test", &checkpoint);

        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
    }

    #[test]
    fn contextual_proper_noun_allows_natural_adjectival_translation() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Contextual,
            "Martian",
            "火星人",
            "Martian rulers watched the sky.",
            "火星统治者注视着天空。",
        );

        let report = build_validation_report("test", &checkpoint);

        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
    }

    #[test]
    fn fiction_unknown_proper_noun_can_be_localized_without_warning() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Queen Cunambria",
            "",
            "The jewels of Queen Cunambria were hidden.",
            "库南布里亚女王的珠宝被藏起来了。",
        );
        let mut checkpoint = checkpoint;
        checkpoint.translation_context = Some(TranslationContext {
            proper_nouns: vec![ProperNounHint {
                source: "Queen Cunambria".to_string(),
                target: None,
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 1,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            }],
            static_terms: Vec::new(),
        });

        let report = build_validation_report("test", &checkpoint);

        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "UNKNOWN_PROPER_NOUN_WITHOUT_CANONICAL_TARGET"));
    }

    #[test]
    fn root_term_does_not_false_match_longer_variant_in_source() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Hyperborea",
            "许珀耳玻瑞亚",
            "Hyperborean rulers watched the sky.",
            "许珀耳玻瑞亚统治者注视着天空。",
        );

        let report = build_validation_report("test", &checkpoint);

        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
    }

    #[test]
    fn builds_metrics_artifact_from_checkpoint_and_validation() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Satampra Zeiros",
            "萨坦普拉路泽罗斯",
            "Satampra Zeiros entered the city.",
            "萨坦普拉路泽罗斯走进了城市。",
        );
        let mut checkpoint = checkpoint;
        checkpoint.chunks[0].translated =
            Some("钀ㄥ潶鏅媺璺辰缃楁柉走进了城市。助推+在这里发挥作用。".to_string());
        checkpoint.chunks[0].confirmed_terms.push(ConfirmedTerm {
            source: "nudge plus".to_string(),
            translation: "助推+".to_string(),
            term_type: "coinage".to_string(),
            confidence: 95,
            usage_role: "coinage_noun".to_string(),
        });
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("metrics build");

        assert_eq!(metrics.total_chunks, 1);
        assert_eq!(metrics.translated_chunks, 1);
        assert!(metrics.strict_glossary_entries >= 1);
        assert_eq!(
            metrics.validation_issue_count,
            validation.summary.issue_count
        );
    }

    #[test]
    fn metrics_artifact_records_source_toc_observability() {
        let checkpoint = checkpoint_with_context(
            ProperNounEnforcement::Strict,
            "Example",
            "Example",
            "Example source.",
            "Example translation.",
        );
        let validation = build_validation_report("test", &checkpoint);
        let glossary = build_glossary_artifact("fiction", &checkpoint);
        let toc = sample_toc_artifact();
        let source_markdown = "# Chapter 1\n\n# Chapter 2\n\n# Chapter 2";
        let translated_markdown = "# 第一章\n\n# 第二章甲\n\n# 第二章乙";
        let translated_toc =
            build_translated_toc_artifact(&toc, source_markdown, translated_markdown, &[], &[]);

        let metrics = build_metrics_artifact(
            &checkpoint,
            &validation,
            &glossary,
            None,
            Some(&toc),
            Some(&translated_toc),
            Some(source_markdown),
            Some(translated_markdown),
            None,
        )
        .expect("metrics build");

        assert_eq!(metrics.toc_source_kind.as_deref(), Some("epubNcx"));
        assert_eq!(metrics.toc_entry_count, 3);
        assert_eq!(metrics.toc_confidence, Some(0.95));
        assert_eq!(metrics.toc_low_confidence_entry_count, 1);
        assert_eq!(metrics.toc_duplicate_title_count, 1);
        assert_eq!(metrics.toc_missing_locator_count, 1);
        assert_eq!(metrics.translated_toc_entry_count, 3);
        assert_eq!(metrics.translated_toc_missing_title_count, 2);
        assert_eq!(metrics.translated_toc_structural_mismatch_count, 0);
        assert_eq!(metrics.translated_toc_locator_mutation_count, 0);
        assert_eq!(metrics.translated_toc_matched_title_count, 1);
        assert_eq!(metrics.translated_toc_unmatched_title_count, 2);
        assert_eq!(metrics.translated_toc_exact_structure_match_count, 3);
        assert!(metrics.translated_toc_title_backfill_rate > 0.3);
    }

    fn checkpoint_with_context(
        enforcement: ProperNounEnforcement,
        source_term: &str,
        target_term: &str,
        source: &str,
        translated: &str,
    ) -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec!["translation:core".to_string()],
            translation_context: Some(TranslationContext {
                proper_nouns: vec![ProperNounHint {
                    source: source_term.to_string(),
                    target: Some(target_term.to_string()),
                    enforcement,
                    occurrences: 1,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                }],
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 1000,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: source.to_string(),
                translated: Some(translated.to_string()),
                confirmed_terms: Vec::new(),
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            }],
        }
    }

    #[tokio::test]
    async fn persists_and_restores_source_sidecar_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-artifact-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let paths = ArtifactPaths::new(&root);
        let source_document = OcrDocument {
            markdown: "Figure 1\n\n![](imgs/example.png)\n\ncaption".to_string(),
            cover_data_url: None,
            toc: Some(sample_toc_artifact()),
            page_layouts: vec![OcrPageLayout {
                blocks: vec![OcrLayoutBlock {
                    label: "image".to_string(),
                    content: "figure".to_string(),
                    bbox: vec![1, 2, 3, 4],
                    order: Some(1),
                    page: Some(1),
                }],
            }],
            markdown_images: HashMap::from([(
                "imgs/example.png".to_string(),
                format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(b"fake-image")
                ),
            )]),
            epub_blocks: Vec::new(),
            source_blocks: crate::document::build_source_blocks_from_markdown(
                "Figure 1\n\n![](imgs/example.png)\n\ncaption",
            ),
        };

        persist_source_document(&paths, &source_document)
            .await
            .expect("persist source document");
        let restored = load_source_document(&paths)
            .await
            .expect("load source document")
            .expect("restored source document");

        assert_eq!(restored.markdown, source_document.markdown);
        assert_eq!(restored.toc.as_ref().map(|toc| toc.entries.len()), Some(3));
        assert_eq!(restored.page_layouts.len(), 1);
        assert!(!restored.source_blocks.is_empty());
        assert_eq!(restored.source_blocks[0].kind, "paragraph");
        assert_eq!(restored.markdown_images.len(), 1);
        assert!(paths.source_toc_path.exists());
        assert!(root.join("imgs/example.png").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    fn sample_toc_artifact() -> TocArtifact {
        TocArtifact {
            source_kind: TocSourceKind::EpubNcx,
            confidence: 0.95,
            entries: vec![
                TocEntry {
                    source_title: "Chapter 1".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 0,
                    page: None,
                    href: Some("chapter-1.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.95,
                },
                TocEntry {
                    source_title: "Chapter 2".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 1,
                    page: None,
                    href: None,
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.55,
                },
                TocEntry {
                    source_title: "Chapter 2".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 2,
                    page: None,
                    href: Some("chapter-2-duplicate.xhtml".to_string()),
                    source_kind: TocSourceKind::EpubNcx,
                    confidence: 0.95,
                },
            ],
        }
    }
}
