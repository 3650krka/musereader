use super::super::matching::source_mentions_hint;
use crate::pipeline::{
    ProperNounEnforcement, ProperNounHint, StaticGlossaryEntry, TranslationContext,
};

pub(super) fn select_context_hints_for_chunk<'a>(
    context: &'a TranslationContext,
    chunk_index: usize,
    source: &str,
) -> Vec<&'a ProperNounHint> {
    let mut selected = context
        .proper_nouns
        .iter()
        .filter(|hint| hint_applies_to_chunk(hint, chunk_index, source))
        .collect::<Vec<_>>();

    selected.sort_by_key(|hint| {
        (
            if hint.enforcement == ProperNounEnforcement::Strict && hint.target.is_some() {
                0
            } else {
                1
            },
            hint.first_chunk_index,
            hint.source.clone(),
        )
    });
    selected
}

pub(super) fn select_static_glossary_hits_for_chunk<'a>(
    context: &'a TranslationContext,
    source: &str,
) -> Vec<&'a StaticGlossaryEntry> {
    let mut selected = context
        .static_terms
        .iter()
        .filter(|entry| {
            !entry.source.trim().is_empty()
                && !entry.target.trim().is_empty()
                && source_mentions_hint(source, &entry.source)
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.source.cmp(&right.source));
    selected
}

fn hint_applies_to_chunk(hint: &ProperNounHint, chunk_index: usize, source: &str) -> bool {
    if hint.target.is_some() {
        return source_mentions_hint(source, &hint.source);
    }
    if hint.chunk_indexes.is_empty() {
        return hint.first_chunk_index == chunk_index;
    }
    hint.chunk_indexes.binary_search(&chunk_index).is_ok()
}
