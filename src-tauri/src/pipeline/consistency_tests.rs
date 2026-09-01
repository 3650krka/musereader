use super::*;
use crate::llm::ConfirmedTerm;

const ALICE_A: &str = "\u{7231}\u{4e3d}\u{4e1d}\u{00b7}\u{7ea6}\u{7ff0}\u{900a}";
const ALICE_B: &str = "\u{827e}\u{4e3d}\u{65af}\u{00b7}\u{7ea6}\u{7ff0}\u{900a}";
const HYPERBOREA: &str = "\u{8bb8}\u{683c}\u{73c0}\u{5c14}\u{73bb}\u{745e}\u{4e9a}";
const HYPERBOREAN_RULER: &str =
    "\u{8bb8}\u{683c}\u{73c0}\u{5c14}\u{73bb}\u{745e}\u{4e9a}\u{7edf}\u{6cbb}\u{8005}";
const HYPERBOREAN_PEOPLE: &str = "\u{8bb8}\u{683c}\u{73c0}\u{5c14}\u{73bb}\u{745e}\u{4e9a}\u{4eba}";
const JOHN_SMITH: &str = "\u{7ea6}\u{7ff0}\u{00b7}\u{53f2}\u{5bc6}\u{65af}";

#[test]
fn keeps_first_confirmed_translation_as_anchor() {
    let mut checkpoint = checkpoint_with_unknown_name(vec![
        chunk_case(
            0,
            "Alice Johnson entered.",
            &format!("{ALICE_A}\u{8d70}\u{8fdb}\u{6765}\u{4e86}\u{3002}"),
            vec![confirmed_term(
                "Alice Johnson",
                ALICE_A,
                "person",
                95,
                "person_name",
            )],
        ),
        chunk_case(
            1,
            "Alice Johnson spoke.",
            &format!("{ALICE_B}\u{5f00}\u{53e3}\u{8bf4}\u{8bdd}\u{3002}"),
            vec![confirmed_term(
                "Alice Johnson",
                ALICE_B,
                "person",
                92,
                "person_name",
            )],
        ),
    ]);

    let summary = apply_first_seen_term_consistency(&mut checkpoint);
    let expected = format!("{ALICE_A}\u{5f00}\u{53e3}\u{8bf4}\u{8bdd}\u{3002}");

    assert_eq!(summary.anchored_terms, 1);
    assert_eq!(summary.repaired_chunks, 1);
    assert_eq!(
        checkpoint.chunks[1].translated.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(
        checkpoint.translation_context.unwrap().proper_nouns[0]
            .target
            .as_deref(),
        Some(ALICE_A)
    );
}

#[test]
fn low_confidence_terms_do_not_enter_glossary() {
    let mut checkpoint = checkpoint_with_unknown_name(vec![chunk_case(
        0,
        "Alice Johnson entered.",
        &format!("{ALICE_A}\u{8d70}\u{8fdb}\u{6765}\u{4e86}\u{3002}"),
        vec![confirmed_term(
            "Alice Johnson",
            ALICE_A,
            "person",
            72,
            "person_name",
        )],
    )]);

    let summary = apply_first_seen_term_consistency(&mut checkpoint);

    assert_eq!(summary.anchored_terms, 0);
    assert_eq!(
        checkpoint.translation_context.unwrap().proper_nouns[0]
            .target
            .as_deref(),
        None
    );
}

#[test]
fn contextual_terms_emit_variant_candidates_without_auto_fix() {
    let mut checkpoint = contextual_checkpoint(
        "Hyperborean rulers watched.",
        &format!("{HYPERBOREAN_RULER}\u{6ce8}\u{89c6}\u{7740}\u{3002}"),
        confirmed_term(
            "Hyperborean",
            HYPERBOREAN_RULER,
            "coinage",
            91,
            "coinage_modifier",
        ),
    );

    let summary = apply_first_seen_term_consistency(&mut checkpoint);

    assert_eq!(summary.repaired_chunks, 0);
    assert!(summary.potential_variants >= 1);
}

#[test]
fn coined_labels_become_strict_anchors_after_first_confirmation() {
    let mut checkpoint = contextual_checkpoint(
        "Nudge plus can preserve autonomy.",
        "助推+可以维护自主性。",
        confirmed_term("nudge plus", "助推+", "coinage", 95, "coinage_noun"),
    );
    checkpoint.translation_context = Some(TranslationContext {
        proper_nouns: vec![ProperNounHint {
            source: "nudge plus".to_string(),
            target: None,
            enforcement: ProperNounEnforcement::Contextual,
            occurrences: 2,
            first_chunk_index: 0,
            chunk_indexes: vec![0],
        }],
        static_terms: Vec::new(),
    });

    let summary = apply_first_seen_term_consistency(&mut checkpoint);
    let hint = &checkpoint
        .translation_context
        .as_ref()
        .unwrap()
        .proper_nouns[0];

    assert_eq!(summary.anchored_terms, 1);
    assert_eq!(hint.target.as_deref(), Some("助推+"));
    assert_eq!(hint.enforcement, ProperNounEnforcement::Strict);
}

#[test]
fn variant_terms_merge_into_existing_root_without_new_anchor() {
    let mut checkpoint = contextual_checkpoint(
        "Hyperborean rulers watched.",
        &format!("{HYPERBOREAN_PEOPLE}\u{7edf}\u{6cbb}\u{8005}\u{6ce8}\u{89c6}\u{7740}\u{3002}"),
        confirmed_term(
            "Hyperborean",
            HYPERBOREAN_PEOPLE,
            "coinage",
            93,
            "people_noun",
        ),
    );

    let summary = apply_first_seen_term_consistency(&mut checkpoint);

    assert_eq!(summary.anchored_terms, 0);
    assert_eq!(
        checkpoint.translation_context.unwrap().proper_nouns[1]
            .target
            .as_deref(),
        None
    );
}

#[test]
fn untranslated_confirmed_terms_are_rejected_before_glossary() {
    let mut checkpoint = checkpoint_with_unknown_name(vec![chunk_case(
        0,
        "Hyperborean rulers watched.",
        "Hyperborean watched.",
        vec![confirmed_term(
            "Hyperborean",
            "Hyperborean",
            "coinage",
            96,
            "people_noun",
        )],
    )]);
    checkpoint.translation_context = Some(TranslationContext {
        proper_nouns: vec![ProperNounHint {
            source: "Hyperborean".to_string(),
            target: None,
            enforcement: ProperNounEnforcement::Contextual,
            occurrences: 1,
            first_chunk_index: 0,
            chunk_indexes: vec![0],
        }],
        static_terms: Vec::new(),
    });

    let summary = apply_first_seen_term_consistency(&mut checkpoint);

    assert_eq!(summary.anchored_terms, 0);
    assert_eq!(
        checkpoint.translation_context.unwrap().proper_nouns[0]
            .target
            .as_deref(),
        None
    );
}

#[test]
fn partially_untranslated_confirmed_terms_are_rejected_before_glossary() {
    let mut checkpoint = checkpoint_with_unknown_name(vec![chunk_case(
        0,
        "A deliberative instrument can trigger reflection.",
        "\u{4E00}\u{79CD} deliberative \u{5DE5}\u{5177}\u{53EF}\u{4EE5}\u{89E6}\u{53D1}\u{53CD}\u{601D}\u{3002}",
        vec![confirmed_term(
            "deliberative instrument",
            "deliberative \u{5DE5}\u{5177}",
            "technical",
            95,
            "technical_term",
        )],
    )]);
    checkpoint.translation_context = Some(TranslationContext {
        proper_nouns: vec![ProperNounHint {
            source: "deliberative instrument".to_string(),
            target: None,
            enforcement: ProperNounEnforcement::Strict,
            occurrences: 1,
            first_chunk_index: 0,
            chunk_indexes: vec![0],
        }],
        static_terms: Vec::new(),
    });

    let summary = apply_first_seen_term_consistency(&mut checkpoint);

    assert_eq!(summary.anchored_terms, 0);
    assert_eq!(
        checkpoint.translation_context.unwrap().proper_nouns[0]
            .target
            .as_deref(),
        None
    );
}

#[test]
fn academic_person_names_do_not_enter_glossary() {
    let mut checkpoint = checkpoint_with_unknown_name(vec![chunk_case(
        0,
        "John Smith argued.",
        &format!("{JOHN_SMITH}\u{8ba4}\u{4e3a}\u{3002}"),
        vec![confirmed_term(
            "John Smith",
            JOHN_SMITH,
            "person",
            97,
            "person_name",
        )],
    )]);
    checkpoint.article_type = "academic".to_string();

    let summary = apply_first_seen_term_consistency(&mut checkpoint);

    assert_eq!(summary.anchored_terms, 0);
}

fn contextual_checkpoint(source: &str, translated: &str, term: ConfirmedTerm) -> ChunkCheckpoint {
    ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: Some(TranslationContext {
            proper_nouns: vec![
                ProperNounHint {
                    source: "Hyperborea".to_string(),
                    target: Some(HYPERBOREA.to_string()),
                    enforcement: ProperNounEnforcement::Contextual,
                    occurrences: 1,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                },
                ProperNounHint {
                    source: "Hyperborean".to_string(),
                    target: None,
                    enforcement: ProperNounEnforcement::Contextual,
                    occurrences: 1,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                },
            ],
            static_terms: Vec::new(),
        }),
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![chunk_case(0, source, translated, vec![term])],
    }
}

fn checkpoint_with_unknown_name(chunks: Vec<ChunkState>) -> ChunkCheckpoint {
    ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: Some(TranslationContext {
            proper_nouns: vec![ProperNounHint {
                source: "Alice Johnson".to_string(),
                target: None,
                enforcement: ProperNounEnforcement::Strict,
                occurrences: chunks.len(),
                first_chunk_index: 0,
                chunk_indexes: chunks.iter().map(|chunk| chunk.index).collect(),
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
        chunks,
    }
}

fn chunk_case(
    index: usize,
    source: &str,
    translated: &str,
    confirmed_terms: Vec<ConfirmedTerm>,
) -> ChunkState {
    ChunkState {
        index,
        source: source.to_string(),
        translated: Some(translated.to_string()),
        confirmed_terms,
        attempts: 1,
        updated_at: "now".to_string(),
        segment_kind: ChunkSegmentKind::Body,
        processing_stage: None,
    }
}

fn confirmed_term(
    source: &str,
    translation: &str,
    term_type: &str,
    confidence: u8,
    usage_role: &str,
) -> ConfirmedTerm {
    ConfirmedTerm {
        source: source.to_string(),
        translation: translation.to_string(),
        term_type: term_type.to_string(),
        confidence,
        usage_role: usage_role.to_string(),
    }
}

// ---- 渲染端专名变体一致性（苏菲/索菲 类）----

fn checkpoint_with_sophie_variants(chunks: Vec<ChunkState>) -> ChunkCheckpoint {
    ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: Some(TranslationContext {
            proper_nouns: Vec::new(),
            static_terms: Vec::new(),
        }),
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks,
    }
}

// render_variant 已禁用（大规模误改），其测试不再适用——apply 已不调用 render_variant。
// 保留测试代码供重新设计参考，但标记为 ignore。
#[test]
#[ignore = "render_variant disabled: causes mass false-positive replacements"]
fn rendered_variant_locks_minority_transliteration_to_majority() {
    // Sophie 被译成「苏菲」(3 块) 与「索菲」(1 块)，多数派「苏菲」应锁定少数派。
    let mut checkpoint = checkpoint_with_sophie_variants(vec![
        chunk_case(0, "Sophie entered.", "苏菲走进来。", vec![]),
        chunk_case(1, "Sophie spoke.", "苏菲开口说话。", vec![]),
        chunk_case(2, "Sophie left.", "苏菲离开了。", vec![]),
        chunk_case(3, "Sophie returned.", "索菲回来了。", vec![]),
    ]);
    let summary = apply_first_seen_term_consistency(&mut checkpoint);
    assert_eq!(summary.rendered_variant_replacements, 1);
    assert_eq!(
        checkpoint.chunks[3].translated.as_deref(),
        Some("苏菲回来了。")
    );
}

#[test]
#[ignore = "render_variant disabled"]
fn rendered_variant_does_not_merge_distinct_names_sharing_a_char() {
    let mut checkpoint = checkpoint_with_sophie_variants(vec![
        chunk_case(0, "Sophie entered.", "苏菲走进来。", vec![]),
        chunk_case(1, "Susan entered.", "苏珊走进来。", vec![]),
        chunk_case(2, "Sophie left.", "苏菲离开了。", vec![]),
    ]);
    let summary = apply_first_seen_term_consistency(&mut checkpoint);
    assert_eq!(summary.rendered_variant_replacements, 0);
    assert_eq!(
        checkpoint.chunks[1].translated.as_deref(),
        Some("苏珊走进来。")
    );
}

#[test]
#[ignore = "render_variant disabled"]
fn rendered_variant_skips_when_counts_are_too_close() {
    let mut checkpoint = checkpoint_with_sophie_variants(vec![
        chunk_case(0, "Sophie entered.", "苏菲走进来。", vec![]),
        chunk_case(1, "Sophie spoke.", "苏菲开口。", vec![]),
        chunk_case(2, "Sophie left.", "索菲离开。", vec![]),
        chunk_case(3, "Sophie returned.", "索菲回来。", vec![]),
    ]);
    let summary = apply_first_seen_term_consistency(&mut checkpoint);
    assert_eq!(summary.rendered_variant_replacements, 0);
}

#[test]
#[ignore = "render_variant disabled"]
fn rendered_variant_skips_chunk_where_source_lacks_the_name() {
    let mut checkpoint = checkpoint_with_sophie_variants(vec![
        chunk_case(0, "Sophie entered.", "苏菲走进来。", vec![]),
        chunk_case(1, "Sophie spoke.", "苏菲开口。", vec![]),
        chunk_case(2, "Sophie left.", "苏菲离开。", vec![]),
        chunk_case(3, "She returned home.", "索菲回到家。", vec![]),
    ]);
    let summary = apply_first_seen_term_consistency(&mut checkpoint);
    assert_eq!(summary.rendered_variant_replacements, 0);
    assert_eq!(
        checkpoint.chunks[3].translated.as_deref(),
        Some("索菲回到家。")
    );
}
