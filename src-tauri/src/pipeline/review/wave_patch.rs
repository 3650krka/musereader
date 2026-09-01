use crate::llm::ConfirmedTerm;
use crate::pipeline::{ChunkCheckpoint, ChunkSegmentKind, WaveTermDelta};

pub(crate) fn append_wave_term_key_index(
    checkpoint: &mut ChunkCheckpoint,
    wave_delta: &[WaveTermDelta],
) {
    for (offset, term) in wave_delta.iter().enumerate() {
        let key = crate::pipeline::context::normalize_resource_term_key(&term.source);
        if key.is_empty() {
            continue;
        }
        checkpoint
            .delta_sync
            .wave_term_key_index
            .entry(key)
            .or_default()
            .push(checkpoint.delta_sync.wave_delta_log.len() + offset);
    }
}

pub(crate) fn collect_wave_term_delta(
    checkpoint: &ChunkCheckpoint,
    chunk_indexes: &[usize],
) -> Vec<WaveTermDelta> {
    let mut deltas = Vec::new();
    for chunk_index in chunk_indexes {
        let Some(chunk) = checkpoint
            .chunks
            .iter()
            .find(|chunk| &chunk.index == chunk_index)
        else {
            continue;
        };
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        for term in &chunk.confirmed_terms {
            if !should_emit_wave_term(term, &checkpoint.article_type) {
                continue;
            }
            let source = term.source.trim();
            let target = term.translation.trim();
            if source.is_empty() || target.is_empty() {
                continue;
            }
            deltas.push(WaveTermDelta {
                seq: 0,
                source: source.to_string(),
                target: target.to_string(),
                chunk_index: *chunk_index,
            });
        }
    }
    deltas.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then(left.target.cmp(&right.target))
            .then(left.chunk_index.cmp(&right.chunk_index))
    });
    deltas.dedup_by(|left, right| {
        left.source.eq_ignore_ascii_case(&right.source) && left.target == right.target
    });
    deltas
}

fn should_emit_wave_term(term: &ConfirmedTerm, article_type: &str) -> bool {
    term.confidence >= 85
        && !term.has_untranslated_source_token()
        && (matches!(
            term.term_type.as_str(),
            "technical" | "organization" | "title" | "disputed" | "coinage"
        ) || should_emit_narrative_named_wave_term(term, article_type))
}

fn should_emit_narrative_named_wave_term(term: &ConfirmedTerm, article_type: &str) -> bool {
    if !should_propagate_narrative_named_terms(article_type) {
        return false;
    }
    match term.term_type.as_str() {
        "person" => source_ascii_token_count(&term.source) >= 2,
        "location" => source_ascii_token_count(&term.source) >= 1,
        _ => false,
    }
}

fn should_propagate_narrative_named_terms(article_type: &str) -> bool {
    // 叙事类文本跨 chunk 传播人名/地名（集中策略见 article_policy）。
    crate::article_policy::ArticlePolicy::from_article_type(article_type).is_narrative()
}

fn source_ascii_token_count(source: &str) -> usize {
    let mut count = 0usize;
    let mut in_token = false;
    for ch in source.chars() {
        if ch.is_ascii_alphabetic() {
            if !in_token {
                count = count.saturating_add(1);
                in_token = true;
            }
        } else {
            in_token = false;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ConfirmedTerm;
    use crate::pipeline::{ChunkProcessingStage, ChunkState, TranslationContext};

    #[test]
    fn collects_unique_wave_terms_from_completed_chunks() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: "nudge plus".to_string(),
                translated: Some("助推+".to_string()),
                confirmed_terms: vec![
                    ConfirmedTerm {
                        source: "nudge plus".to_string(),
                        translation: "助推+".to_string(),
                        term_type: "coinage".to_string(),
                        confidence: 95,
                        usage_role: "coinage_noun".to_string(),
                    },
                    ConfirmedTerm {
                        source: "nudge plus".to_string(),
                        translation: "助推+".to_string(),
                        term_type: "coinage".to_string(),
                        confidence: 95,
                        usage_role: "coinage_noun".to_string(),
                    },
                ],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let delta = collect_wave_term_delta(&checkpoint, &[0]);
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].source, "nudge plus");
    }

    #[test]
    fn rejects_wave_terms_with_untranslated_source_tokens() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: "deliberative instrument".to_string(),
                translated: Some("\u{5BA1}\u{8BAE}\u{6027}\u{5DE5}\u{5177}\u{3002}".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "deliberative instrument".to_string(),
                    translation: "deliberative \u{5DE5}\u{5177}".to_string(),
                    term_type: "technical".to_string(),
                    confidence: 95,
                    usage_role: "technical_term".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let delta = collect_wave_term_delta(&checkpoint, &[0]);
        assert!(delta.is_empty());
    }

    #[test]
    fn emits_multi_token_person_terms_for_narrative_articles() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: "Admiral Carfax entered.".to_string(),
                translated: Some("卡法克斯海军上将进来了。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "Admiral Carfax".to_string(),
                    translation: "卡法克斯海军上将".to_string(),
                    term_type: "person".to_string(),
                    confidence: 95,
                    usage_role: "character_title".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let delta = collect_wave_term_delta(&checkpoint, &[0]);

        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].source, "Admiral Carfax");
    }

    #[test]
    fn does_not_emit_person_terms_for_academic_articles() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: "Jinyi Kuang contributed.".to_string(),
                translated: Some("Jinyi Kuang 参与了研究。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "Jinyi Kuang".to_string(),
                    translation: "匡津仪".to_string(),
                    term_type: "person".to_string(),
                    confidence: 95,
                    usage_role: "author_name".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let delta = collect_wave_term_delta(&checkpoint, &[0]);

        assert!(delta.is_empty());
    }

    #[test]
    fn emits_location_terms_for_narrative_articles() {
        let checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![ChunkState {
                index: 0,
                source: "Hyperborea vanished.".to_string(),
                translated: Some("海波波利亚消失了。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "Hyperborea".to_string(),
                    translation: "海波波利亚".to_string(),
                    term_type: "location".to_string(),
                    confidence: 95,
                    usage_role: "place_name".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::Translated),
            }],
        };

        let delta = collect_wave_term_delta(&checkpoint, &[0]);

        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].source, "Hyperborea");
    }

    #[test]
    fn appends_wave_term_key_index_for_normalized_lookup() {
        let mut checkpoint = ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.pdf".to_string(),
            article_type: "academic".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: Vec::new(),
            translation_context: Some(TranslationContext {
                proper_nouns: Vec::new(),
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 500,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: Vec::new(),
        };
        let wave_delta = vec![WaveTermDelta {
            seq: 1,
            source: "nudge+".to_string(),
            target: "助推+".to_string(),
            chunk_index: 0,
        }];

        append_wave_term_key_index(&mut checkpoint, &wave_delta);

        let key = crate::pipeline::context::normalize_resource_term_key("nudge plus");
        assert_eq!(
            checkpoint.delta_sync.wave_term_key_index.get(&key),
            Some(&vec![0])
        );
    }
}
