#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeTranslationRules {
    pub main_translation: Option<&'static str>,
    pub residual_repair: Option<&'static str>,
    pub preservation: Option<&'static str>,
    pub confirmed_terms: Option<&'static str>,
}

const STORY_WORLD_MAIN_RULES: &str = "- Fiction rule: invented creature names, invented object names, species names, vehicles, weapons, materials, places, and named artifacts must be translated or transliterated into natural Chinese when they function as story-world nouns.\n\
     - Fiction rule: do not keep the original English in parentheses after the Chinese translation.\n\
     - Fiction rule: genuinely alien chants, spells, ritual syllables, quoted nonsense words, or intentionally unpronounceable utterances may remain as-is when translating them would damage the text.";

const STORY_WORLD_RESIDUAL_RULES: &str = "- Fiction repair rule: if a candidate is one of the invented creature names, invented object names, species names, vehicles, weapons, materials, places, or named artifacts, repair it into natural Chinese or transliteration.\n\
     - Fiction repair rule: do not keep the original English in parentheses after the Chinese translation; do not append the original English in parentheses.\n\
     - Fiction repair rule: alien chants, spells, ritual syllables, quoted nonsense words, and intentionally unpronounceable utterances may be left unchanged when they are not story-world nouns.";

const STORY_WORLD_PRESERVATION_RULES: &str = "- Preserve alien chants, spells, ritual syllables, quoted nonsense words, and intentionally unpronounceable utterances when translation would damage the intended effect.\n\
     - Do not preserve invented story-world nouns merely because they look unfamiliar.";

const STORY_WORLD_CONFIRMED_TERMS_RULES: &str = "- For fiction or narrative text, confirmedTerms should include newly established translations for recurring invented creature names, invented object names, species, vehicles, weapons, materials, places, and named artifacts.\n\
     - Do not add alien chants, spell syllables, or one-off nonsense utterances to confirmedTerms unless the text clearly uses them as named story-world nouns.";

pub fn runtime_translation_rules(article_type: &str) -> RuntimeTranslationRules {
    if crate::article_policy::ArticlePolicy::has_story_world(article_type) {
        RuntimeTranslationRules {
            main_translation: Some(STORY_WORLD_MAIN_RULES),
            residual_repair: Some(STORY_WORLD_RESIDUAL_RULES),
            preservation: Some(STORY_WORLD_PRESERVATION_RULES),
            confirmed_terms: Some(STORY_WORLD_CONFIRMED_TERMS_RULES),
        }
    } else {
        RuntimeTranslationRules::default()
    }
}
