use super::TermAnchor;
use crate::llm::ConfirmedTerm;
use crate::pipeline::context::collect_capitalized_phrases;
use crate::pipeline::{
    ChunkCheckpoint, ChunkSegmentKind, ProperNounEnforcement, ProperNounHint, TranslationContext,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(super) struct AnchorMatch<'a> {
    pub(super) anchor: &'a TermAnchor,
    pub(super) exact: bool,
}

pub(super) fn find_matching_anchor<'a>(
    anchors: &'a BTreeMap<String, TermAnchor>,
    term: &ConfirmedTerm,
    exact_key: &str,
) -> Option<AnchorMatch<'a>> {
    if let Some(anchor) = anchors.get(exact_key) {
        return Some(AnchorMatch {
            anchor,
            exact: true,
        });
    }
    if is_variant_merge_candidate(term) {
        for (anchor_key, anchor) in anchors {
            if !is_anchor_compatible_for_variant_merge(anchor, anchor_key, exact_key) {
                continue;
            }
            if is_translation_family_match(&anchor.translation, &term.translation) {
                return Some(AnchorMatch {
                    anchor,
                    exact: false,
                });
            }
        }
    }
    None
}

pub(super) fn count_potential_variants(
    context: &TranslationContext,
    checkpoint: &ChunkCheckpoint,
) -> usize {
    checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.segment_kind == ChunkSegmentKind::Body)
        .map(|chunk| {
            collect_capitalized_phrases(&chunk.source)
                .into_iter()
                .filter(|candidate| is_potential_variant(candidate, &context.proper_nouns))
                .count()
        })
        .sum()
}

fn is_variant_merge_candidate(term: &ConfirmedTerm) -> bool {
    matches!(term.term_type.as_str(), "coinage" | "disputed")
        && matches!(term.usage_role.as_str(), "coinage_modifier" | "people_noun")
}

fn is_anchor_compatible_for_variant_merge(
    anchor: &TermAnchor,
    anchor_key: &str,
    candidate_key: &str,
) -> bool {
    matches!(anchor.term_type.as_str(), "coinage" | "disputed")
        && is_source_family_match(anchor_key, candidate_key)
}

fn is_source_family_match(existing: &str, candidate: &str) -> bool {
    if existing == candidate {
        return true;
    }
    if existing.len() < 5 || candidate.len() < 5 {
        return false;
    }
    if existing.starts_with(candidate) || candidate.starts_with(existing) {
        return true;
    }
    normalized_similarity(existing, candidate) >= 0.82
        && common_prefix_ratio(existing, candidate) >= 0.72
}

fn common_prefix_ratio(left: &str, right: &str) -> f32 {
    let shared = left
        .chars()
        .zip(right.chars())
        .take_while(|(left_char, right_char)| left_char == right_char)
        .count();
    let longest = left.chars().count().max(right.chars().count());
    if longest == 0 {
        return 1.0;
    }
    shared as f32 / longest as f32
}

fn is_translation_family_match(existing: &str, candidate: &str) -> bool {
    let left = existing.trim();
    let right = candidate.trim();
    left.contains(right)
        || right.contains(left)
        || strip_variant_cn_suffix(left) == strip_variant_cn_suffix(right)
}

fn strip_variant_cn_suffix(value: &str) -> String {
    for suffix in [
        "\u{4eba}", "\u{65cf}", "\u{754c}", "\u{56fd}", "\u{57df}", "\u{5f0f}", "\u{6d3e}",
    ] {
        if let Some(stem) = value.strip_suffix(suffix) {
            return stem.to_string();
        }
    }
    value.to_string()
}

fn is_potential_variant(candidate: &str, hints: &[ProperNounHint]) -> bool {
    hints.iter().any(|hint| {
        hint.enforcement == ProperNounEnforcement::Contextual
            && candidate != hint.source
            && normalized_similarity(candidate, &hint.source) >= 0.82
    })
}

fn normalized_similarity(left: &str, right: &str) -> f32 {
    crate::pipeline::context::normalized_similarity(
        &left.to_ascii_lowercase(),
        &right.to_ascii_lowercase(),
    )
}
