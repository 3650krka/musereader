use super::{ProperNounEnforcement, ProperNounHint, TranslationContext, PROPER_NOUN_NOISE_WORDS};

mod matching;
mod nouns;
mod prompt;

pub(super) use matching::{
    collect_capitalized_phrases, normalize_resource_term_key, normalized_similarity,
    source_mentions_hint, source_mentions_hint_cached, source_terms_are_similar, term_match_level,
    SourceCandidateCache, TermMatchLevel,
};
pub(super) use nouns::build_translation_context;
pub(super) use prompt::{
    collect_strict_chunk_hits, format_translation_context_for_chunk,
    normalize_strict_hit_confirmed_terms, prune_confirmed_algorithmic_hints,
    repair_canonical_proper_nouns, repair_strict_hit_aliases, synthesize_missing_strict_hit_terms,
};
