mod residual;
mod sentence_patch;
mod translation;
mod wave_patch;
mod wave_repair;

pub(crate) use residual::collect_residual_review_stats;
#[allow(unused_imports)]
pub(crate) use residual::parse_repaired_translation;
pub(crate) use residual::review_residual_english_candidates;
pub(crate) use sentence_patch::{
    find_source_like_patch_residual, locate_patch_target, locate_term_patch_target,
    replace_sentence_once, replace_source_like_phrase,
};
pub(crate) use translation::fill_translation_workers;
pub(crate) use wave_patch::{append_wave_term_key_index, collect_wave_term_delta};
pub(crate) use wave_repair::{
    apply_wave_term_repairs_for_chunk, collect_wave_patch_terms_for_chunk,
    frozen_chunk_patch_index_is_current, rebuild_frozen_chunk_patch_terms,
};
