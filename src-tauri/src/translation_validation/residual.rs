mod detect;
mod prompt;

pub use detect::detect_residual_english_terms;
#[cfg(test)]
pub use prompt::build_llm_validation_review_prompt;
pub use prompt::{
    build_llm_paragraph_patch_prompt, build_llm_sentence_patch_prompt,
    build_llm_validation_chunk_review_prompt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidualEnglishTerm {
    pub term: String,
    pub count: usize,
    pub source_excerpt: String,
    pub translated_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentencePatchTarget {
    pub source_sentence: String,
    pub translated_sentence: String,
    pub source_context: String,
    pub translated_context: String,
}
