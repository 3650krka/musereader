//! 译名首见锁定（first-seen locking）：chunk 译完后立即把「首见译名」写回
//! translation_context.proper_nouns[i].target，让后续 chunk 的 prompt 把该专名
//! 从「软提示」升级为「MUST USE EXACTLY」的 strict 硬约束。
//!
//! 背景（Sophie 苏菲/索菲 不一致的根因）：
//!   properNouns 能提取独立 Sophie（occ=298，Strict），但 target=None，
//!   只进「可能的专名」（algorithmic candidate，软提示），模型每个 chunk
//!   自由决定译名 → 苏菲/索菲 混用。后修复（统计猜测）没有足够语境判断
//!   「索菲==Sophie」，已证明会大规模误改。
//!
//! 本机制把「译名锁定」从「翻译后统一跑」（apply_first_seen_term_consistency，
//! 太晚）提前到「每个 chunk 译完后立即写回」：
//!   该 chunk 的 confirmedTerms 里 source 命中某 properNoun（target=None）时，
//!   把它的 translation 作为该专名的「首见锚定译名」写回 target。
//!   后续 chunk 的 format_translation_context_for_chunk 里，该专名（target.is_some）
//!   进入 strict_chunk_hits（MUST USE EXACTLY），模型被强制用锁定译名。
//!
//! 防误锁（宁缺勿滥）：
//!   - 只锁定「该 chunk confirmedTerms 里 sanitize 通过、且译文里确实出现该译名」的项
//!     （复用 sanitize_confirmed_term 的置信度/类型/译文包含校验）；
//!   - 只锁定 properNouns 里 target=None 的项（不覆盖已有锚定/seed）；
//!   - 一词多译时以「第一个被锁定的」为准（first-seen wins，与 consistency 通道一致）。

use super::term::{normalize_term_key, sanitize_confirmed_term};
use crate::pipeline::context::{term_match_level, TermMatchLevel};
use crate::pipeline::ChunkCheckpoint;

/// chunk 译完后调用：把该 chunk confirmedTerms 里「首见译名」写回 properNouns.target。
/// 返回新锁定的专名数。
pub(crate) fn lock_first_seen_translations(
    checkpoint: &mut ChunkCheckpoint,
    chunk_index: usize,
) -> usize {
    let Some(chunk) = checkpoint.chunks.iter().find(|c| c.index == chunk_index) else {
        return 0;
    };
    let Some(translated) = chunk.translated.as_deref() else {
        return 0;
    };
    let confirmed_terms = chunk.confirmed_terms.clone();
    let article_type = checkpoint.article_type.clone();
    let Some(context) = checkpoint.translation_context.as_mut() else {
        return 0;
    };

    let mut locked = 0usize;
    for term in &confirmed_terms {
        // 复用 confirmed-term 净化：置信度≥85、类型合法、译文确实含该译名。
        let Some(term) = sanitize_confirmed_term(term, translated, &article_type) else {
            continue;
        };
        let key = normalize_term_key(&term.source);
        // 找 properNouns 里 target=None 且与 term.source 匹配的项，写回首见译名。
        // 匹配策略分级（区分「同一实体变体」与「不同人物」）：
        //   L1 精确匹配（normalized key 相同）→ 锁定
        //   L2 包含匹配（Sophie vs Sophie Wentworth）→ 锁定
        //   L3 高相似度（编辑距离≥0.85 / 子序列≥0.9 / 前缀≥0.8）→ 锁定
        //   L4 低相似度（Sophia/Sophie 前缀重合 0.6-0.8）→ 不自动锁定，
        //      可能是同一人的拼写变体，也可能是不同人物，交给模型在 prompt 里判断。
        // 双向匹配：hint→term 和 term→hint 任一命中即算，处理全名↔短名两个方向。
        for hint in context.proper_nouns.iter_mut() {
            if hint.target.is_some() {
                continue; // 已有锚定/seed，不覆盖
            }
            let hint_key = normalize_term_key(&hint.source);
            let level = if hint_key == key {
                TermMatchLevel::Exact
            } else {
                term_match_level(&hint.source, &term.source)
            };
            if !level.is_lockable() {
                continue;
            }
            hint.target = Some(term.translation.clone());
            locked += 1;
            break; // 一个 term 只锁一个 hint
        }
    }
    locked
}

#[cfg(test)]
mod tests {
    use super::lock_first_seen_translations;
    use crate::llm::ConfirmedTerm;
    use crate::pipeline::{
        ChunkCheckpoint, ChunkProcessingStage, ChunkSegmentKind, ChunkState, ProperNounEnforcement,
        ProperNounHint, TranslationContext,
    };

    fn sophie_checkpoint() -> ChunkCheckpoint {
        ChunkCheckpoint {
            version: 1,
            task_id: "test".to_string(),
            source_pdf_path: "sample.epub".to_string(),
            article_type: "fiction".to_string(),
            system_prompt_hash: "hash".to_string(),
            skill_ids: vec![],
            translation_context: Some(TranslationContext {
                proper_nouns: vec![ProperNounHint {
                    source: "Sophie".to_string(),
                    target: None,
                    enforcement: ProperNounEnforcement::Strict,
                    occurrences: 3,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0, 1, 2],
                }],
                static_terms: Vec::new(),
            }),
            source_hash: "hash".to_string(),
            chunk_size: 3000,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            source_markdown_path: "source.md".to_string(),
            write_metrics: Default::default(),
            delta_sync: Default::default(),
            chunks: vec![
                ChunkState {
                    index: 0,
                    source: "Sophie entered.".to_string(),
                    translated: Some("苏菲走进来。".to_string()),
                    confirmed_terms: vec![ConfirmedTerm {
                        source: "Sophie".to_string(),
                        translation: "苏菲".to_string(),
                        term_type: "person".to_string(),
                        confidence: 95,
                        usage_role: "person_name".to_string(),
                    }],
                    attempts: 1,
                    updated_at: "now".to_string(),
                    segment_kind: ChunkSegmentKind::Body,
                    processing_stage: Some(ChunkProcessingStage::Translated),
                },
                ChunkState {
                    index: 1,
                    source: "Sophie spoke.".to_string(),
                    translated: None,
                    confirmed_terms: vec![],
                    attempts: 0,
                    updated_at: "now".to_string(),
                    segment_kind: ChunkSegmentKind::Body,
                    processing_stage: Some(ChunkProcessingStage::Pending),
                },
            ],
        }
    }

    #[test]
    fn locks_first_seen_translation_into_proper_noun_target() {
        let mut checkpoint = sophie_checkpoint();
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(locked, 1);
        let hint = &checkpoint
            .translation_context
            .as_ref()
            .unwrap()
            .proper_nouns[0];
        assert_eq!(hint.target.as_deref(), Some("苏菲"));
    }

    #[test]
    fn does_not_overwrite_existing_target() {
        let mut checkpoint = sophie_checkpoint();
        // 预填 target
        checkpoint
            .translation_context
            .as_mut()
            .unwrap()
            .proper_nouns[0]
            .target = Some("索菲".to_string());
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(locked, 0);
        let hint = &checkpoint
            .translation_context
            .as_ref()
            .unwrap()
            .proper_nouns[0];
        assert_eq!(
            hint.target.as_deref(),
            Some("索菲"),
            "existing target preserved"
        );
    }

    #[test]
    fn skips_term_not_in_translation() {
        let mut checkpoint = sophie_checkpoint();
        // confirmedTerm 的 translation 不在译文里（模型虚报）→ 不锁
        checkpoint.chunks[0].confirmed_terms[0].translation = "苏珊".to_string();
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(locked, 0);
        let hint = &checkpoint
            .translation_context
            .as_ref()
            .unwrap()
            .proper_nouns[0];
        assert!(hint.target.is_none());
    }

    #[test]
    fn skips_low_confidence_term() {
        let mut checkpoint = sophie_checkpoint();
        checkpoint.chunks[0].confirmed_terms[0].confidence = 50;
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(locked, 0);
    }

    #[test]
    fn locked_target_flows_into_strict_chunk_hits_for_later_chunks() {
        let mut checkpoint = sophie_checkpoint();
        let locked = lock_first_seen_translations(&mut checkpoint, 1);
        assert_eq!(locked, 0);
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(locked, 1);
        let context = checkpoint.translation_context.as_ref().unwrap();
        let rendered = crate::pipeline::context::format_translation_context_for_chunk(
            context,
            &checkpoint.article_type,
            1,
            &checkpoint.chunks[1].source,
        );
        assert!(
            rendered.contains("Sophie => MUST USE EXACTLY 苏菲"),
            "rendered prompt should contain strict hit, got:\n{rendered}"
        );
    }

    #[test]
    fn cast_block_contains_locked_names_even_when_chunk_lacks_source_name() {
        // Cast block 核心行为：chunk 源文只有代词（无 Sophie 字样）时，
        // 已锁定的角色译名仍恒定注入，供代词回指查名。
        let mut checkpoint = sophie_checkpoint();
        checkpoint.chunks[1].source = "She looked back and waved.".to_string();
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(locked, 1);
        let context = checkpoint.translation_context.as_ref().unwrap();
        let rendered = crate::pipeline::context::format_translation_context_for_chunk(
            context,
            &checkpoint.article_type,
            1,
            &checkpoint.chunks[1].source,
        );
        assert!(
            rendered.contains("Established cast"),
            "cast block header should appear, got:\n{rendered}"
        );
        assert!(
            rendered.contains("- Sophie => 苏菲"),
            "cast block should carry locked name, got:\n{rendered}"
        );
    }

    #[test]
    fn cast_block_is_absent_before_any_lock() {
        // 未锁定任何译名时不出现 cast block（避免空节占 prompt）。
        let checkpoint = sophie_checkpoint();
        let context = checkpoint.translation_context.as_ref().unwrap();
        let rendered = crate::pipeline::context::format_translation_context_for_chunk(
            context,
            &checkpoint.article_type,
            0,
            &checkpoint.chunks[0].source,
        );
        assert!(
            !rendered.contains("Established cast"),
            "no cast block before locking, got:\n{rendered}"
        );
    }

    #[test]
    fn locks_hint_with_similar_source_surface() {
        // 模型返回的 confirmedTerm source 表面与词表 hint 不完全一致
        // （如词表提取到全名 "Sophie Wentworth"，模型只确认 "Sophie"），
        // 复用 source_terms_are_similar 相似性匹配后仍能锁定。
        let mut checkpoint = sophie_checkpoint();
        checkpoint
            .translation_context
            .as_mut()
            .unwrap()
            .proper_nouns[0]
            .source = "Sophie Wentworth".to_string();
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(
            locked, 1,
            "similar source surface (Sophie vs Sophie Wentworth) should lock"
        );
        let hint = &checkpoint
            .translation_context
            .as_ref()
            .unwrap()
            .proper_nouns[0];
        assert_eq!(hint.target.as_deref(), Some("苏菲"));
    }

    #[test]
    fn does_not_lock_unrelated_name_with_partial_overlap() {
        // 分级匹配：Sophia vs Sophie 仅前缀重合（prefix_ratio≈0.83 但 similarity≈0.67），
        // term_match_level 判定为 L4 LowSimilarity（前缀 0.8-0.83 落在 L4 区间），
        // 不自动锁定——可能是同一人的拼写变体，也可能是不同人物，交给模型判断。
        let mut checkpoint = sophie_checkpoint();
        checkpoint
            .translation_context
            .as_mut()
            .unwrap()
            .proper_nouns[0]
            .source = "Sophia".to_string();
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        let hint = &checkpoint
            .translation_context
            .as_ref()
            .unwrap()
            .proper_nouns[0];
        // L4 低相似度不锁定：Sophia 和 Sophie 可能是不同人物
        assert_eq!(
            locked, 0,
            "Sophia vs Sophie is L4 low similarity, must not lock"
        );
        assert!(hint.target.is_none());
    }

    #[test]
    fn does_not_lock_when_names_share_no_significant_overlap() {
        // 防误锁：confirmedTerm 是 "Sophie"，词表 hint 是 "Charlotte"——
        // 完全不同的名字，不应锁定。
        let mut checkpoint = sophie_checkpoint();
        checkpoint
            .translation_context
            .as_mut()
            .unwrap()
            .proper_nouns[0]
            .source = "Charlotte".to_string();
        let locked = lock_first_seen_translations(&mut checkpoint, 0);
        assert_eq!(
            locked, 0,
            "Charlotte vs Sophie has no overlap, must not lock"
        );
        let hint = &checkpoint
            .translation_context
            .as_ref()
            .unwrap()
            .proper_nouns[0];
        assert!(hint.target.is_none());
    }
}
