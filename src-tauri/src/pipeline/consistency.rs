use super::*;
use std::collections::{BTreeMap, BTreeSet};

mod first_seen;
mod render_variant;
mod term;
mod variant;

use render_variant::{apply_rendered_variant_repair, RenderedVariantSummary};
use term::{
    enforcement_for_term_type, enforcement_term_type, normalize_term_key, sanitize_confirmed_term,
};
use variant::{count_potential_variants, find_matching_anchor};

pub(crate) use first_seen::lock_first_seen_translations as lock_chunk_first_seen_translations;

pub(crate) fn sanitize_dynamic_confirmed_term(
    term: &crate::llm::ConfirmedTerm,
    translated_text: &str,
    article_type: &str,
) -> Option<crate::llm::ConfirmedTerm> {
    sanitize_confirmed_term(term, translated_text, article_type)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct TermConsistencySummary {
    anchored_terms: usize,
    repaired_chunks: usize,
    potential_variants: usize,
    // 渲染端专名变体回扫（苏菲/索菲 类）：与 confirmed-term 通道分开计数，
    // 便于事件归因与「只检测不修复」模式下的报告。
    rendered_variant_groups: usize,
    rendered_variant_replacements: usize,
    rendered_variant_repaired_chunks: usize,
}

impl TermConsistencySummary {
    pub(super) fn has_changes(&self) -> bool {
        self.anchored_terms > 0
            || self.repaired_chunks > 0
            || self.potential_variants > 0
            || self.rendered_variant_replacements > 0
    }

    pub(super) fn to_event_message(&self) -> String {
        format!(
            "term consistency checked; anchored_terms={}; repaired_chunks={}; potential_variants={}; rendered_variant_groups={}; rendered_variant_replacements={}; rendered_variant_repaired_chunks={}",
            self.anchored_terms,
            self.repaired_chunks,
            self.potential_variants,
            self.rendered_variant_groups,
            self.rendered_variant_replacements,
            self.rendered_variant_repaired_chunks
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TermAnchor {
    translation: String,
    term_type: String,
}

pub(super) fn apply_first_seen_term_consistency(
    checkpoint: &mut ChunkCheckpoint,
) -> TermConsistencySummary {
    let potential_variants = checkpoint
        .translation_context
        .as_ref()
        .map(|context| count_potential_variants(context, checkpoint))
        .unwrap_or_default();
    let Some(context) = checkpoint.translation_context.as_mut() else {
        return TermConsistencySummary::default();
    };

    let mut anchors = seed_term_anchors(context);
    let mut new_anchor_keys = BTreeSet::new();
    let mut repaired_chunks = BTreeSet::new();

    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_mut() else {
            continue;
        };

        for term in &chunk.confirmed_terms {
            let Some(term) = sanitize_confirmed_term(term, translated, &checkpoint.article_type)
            else {
                continue;
            };
            let key = normalize_term_key(&term.source);
            if let Some(anchor_match) = find_matching_anchor(&anchors, &term, &key) {
                if anchor_match.exact
                    && anchor_match.anchor.translation != term.translation
                    && translated.contains(&term.translation)
                {
                    *translated =
                        translated.replace(&term.translation, &anchor_match.anchor.translation);
                    repaired_chunks.insert(chunk.index);
                }
                continue;
            }

            anchors.insert(
                key.clone(),
                TermAnchor {
                    translation: term.translation.clone(),
                    term_type: term.term_type.clone(),
                },
            );
            new_anchor_keys.insert(key);
        }
    }

    let anchored_terms = write_back_anchors(context, &anchors, &new_anchor_keys);
    // 渲染端专名变体回扫【已禁用——大规模误改】：
    // apply_rendered_variant_repair 的「共享汉字聚类」把不同词（西蒙/亲密/建设/湿）
    // 误判为同一实体的词形变体，多数派（杰兹/什么）替换少数派，实测改坏 3353 处、
    // 23+ chunk 文本损坏。其词形还原假设（同名音译共享汉字）在高频词面前失效。
    // 待重新设计（需更强的「同一实体」证据，如 NER 或上下文约束）后再启用。
    let RenderedVariantSummary {
        groups: rendered_variant_groups,
        replacements: rendered_variant_replacements,
        repaired_chunks: rendered_variant_repaired_chunks,
    } = RenderedVariantSummary::default();
    let _ = apply_rendered_variant_repair; // 保留实现供重新设计参考
    TermConsistencySummary {
        anchored_terms,
        repaired_chunks: repaired_chunks.len(),
        potential_variants,
        rendered_variant_groups,
        rendered_variant_replacements,
        rendered_variant_repaired_chunks,
    }
}

fn seed_term_anchors(context: &TranslationContext) -> BTreeMap<String, TermAnchor> {
    let mut anchors = BTreeMap::new();
    for hint in &context.proper_nouns {
        let Some(target) = hint.target.as_deref() else {
            continue;
        };
        anchors
            .entry(normalize_term_key(&hint.source))
            .or_insert(TermAnchor {
                translation: target.trim().to_string(),
                term_type: enforcement_term_type(hint.enforcement).to_string(),
            });
    }
    anchors
}

fn write_back_anchors(
    context: &mut TranslationContext,
    anchors: &BTreeMap<String, TermAnchor>,
    new_anchor_keys: &BTreeSet<String>,
) -> usize {
    let mut changed = 0usize;
    for hint in &mut context.proper_nouns {
        let key = normalize_term_key(&hint.source);
        let Some(anchor) = anchors.get(&key) else {
            continue;
        };
        if hint.target.is_none() && new_anchor_keys.contains(&key) {
            hint.target = Some(anchor.translation.clone());
            hint.enforcement = enforcement_for_term_type(&anchor.term_type);
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
#[path = "consistency_tests.rs"]
mod tests;
