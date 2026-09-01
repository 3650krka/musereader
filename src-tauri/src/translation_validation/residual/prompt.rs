use super::{ResidualEnglishTerm, SentencePatchTarget};

#[cfg(test)]
pub fn build_llm_validation_review_prompt(
    article_type: &str,
    issue_term: &str,
    source_excerpt: &str,
    translated_excerpt: &str,
    full_translated_chunk: &str,
) -> String {
    format!(
        "You are a translation validation agent. Decide whether the candidate item is a false positive.\n\
         If it is not a false positive, repair the full translated chunk.\n\
         Article type: {article_type}\n\
         Candidate term: {issue_term}\n\
         Source excerpt:\n{source_excerpt}\n\n\
         Translated excerpt:\n{translated_excerpt}\n\n\
         Full translated chunk:\n{full_translated_chunk}\n\n\
         Return JSON only:\n\
         {{\"decision\":\"false_positive\",\"reason\":\"...\"}}\n\
         or\n\
         {{\"decision\":\"repair\",\"repairedText\":\"complete repaired translated chunk\"}}\n\
         False positives include glossary terms, citations, DOI, URL, code, formulas, reference text, and intentional proper-noun preservation."
    )
}

pub fn build_llm_validation_chunk_review_prompt(
    article_type: &str,
    source_chunk: &str,
    translated_chunk: &str,
    residual_terms: &[ResidualEnglishTerm],
) -> String {
    let fiction_rules = residual_story_world_rules(article_type);
    let candidates = residual_terms
        .iter()
        .enumerate()
        .map(|(index, term)| {
            format!(
                "{}. term={} count={} sourceExcerpt={} translatedExcerpt={}",
                index + 1,
                term.term,
                term.count,
                term.source_excerpt,
                term.translated_excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are a translation validation agent. Review one translated Markdown chunk for residual untranslated English.\n\
         Article type: {article_type}\n\n\
         Repair style:\n\
         - Prefer natural, fluent Chinese in the local genre rather than literal word-for-word replacement.\n\
         - For fiction or narrative text, preserve narrative flow, tone, rhythm, imagery, and emotional force.\n\
         - Do not make the repaired Chinese sound like glossary fragments mechanically inserted into a sentence.\n\n\
         Source chunk:\n{source_chunk}\n\n\
         Current translated chunk:\n{translated_chunk}\n\n\
         Candidate residual English items:\n{candidates}\n\n\
         Rules:\n\
         - Reject false positives: author names, citations, affiliations, formulas, DOI, URL, email, code, acronyms, section labels that should stay English, and bibliography/reference text.\n\
         - Keep proper names consistent with the existing translation when they are intentionally preserved or already translated elsewhere.\n\
         - If any ordinary English word, adjective, verb, or common noun remains untranslated, repair the entire translated chunk.\n\
         - An English adjective or noun embedded in a Chinese sentence and modifying a Chinese word is not a false positive; translate it unless it is a proper name, citation, formula, code, URL, email, or bibliography text.\n\
         {fiction_rules}\
         - Return false_positive only if every listed candidate is intentionally preserved.\n\
         - If you repair the chunk, you may include confirmedTerms only for candidate residual items that have no existing glossary mapping and that clearly function as stable technical terms, coined labels, recurring proper names, or recurring fiction/narrative story-world nouns.\n\
         - Do not add confirmedTerms for ordinary leftover English words or for any item outside the listed candidate residual items.\n\
         - Do not summarize. Do not add commentary. Preserve Markdown structure.\n\
         - When a more natural Chinese phrasing requires a tiny local rewrite around the residual English, prefer that over a stiff literal substitution, but keep the repair local to the affected area.\n\
         - Return only JSON.\n\n\
         JSON schema:\n\
         {{\"decision\":\"false_positive\",\"reason\":\"...\",\"confirmedTerms\":[]}}\n\
         or\n\
         {{\"decision\":\"repair\",\"repairedText\":\"complete repaired translated chunk\",\"confirmedTerms\":[{{\"source\":\"term in source\",\"translation\":\"term used in repairedText\",\"termType\":\"coinage|disputed|person|location|organization|title|technical\",\"confidence\":95,\"usageRole\":\"coinage_noun|technical_term|person_name|place_name|organization_name|book_title\"}}]}}"
    )
}

pub fn build_llm_paragraph_patch_prompt(
    article_type: &str,
    source_excerpt: &str,
    translated_excerpt: &str,
    patch_terms: &[(&str, &str)],
    scope_label: &str,
) -> String {
    let preservation_rules = sentence_patch_preservation_rules(article_type);
    let fiction_rules = residual_story_world_rules(article_type);
    let glossary = if patch_terms.is_empty() {
        "- No explicit patch glossary terms.".to_string()
    } else {
        patch_terms
            .iter()
            .map(|(source, target)| {
                let target = target.trim();
                if target.is_empty() {
                    format!("- {} => translate naturally in this excerpt", source)
                } else {
                    format!("- {} => {}", source, target)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "You are a translation repair agent. Your task is minimal {scope_label}-level repair only.\n\
         Article type: {article_type}\n\n\
         Goal:\n\
         - Repair only the current translated excerpt.\n\
         - Keep unrelated wording, tone, markdown, citations, and paragraph boundaries unchanged.\n\
         - Keep the repaired Chinese fluent and natural in context, not glossary-like or mechanically literal.\n\
         - Only change the matched residual English words or the smallest nearby Chinese wording needed to make the sentence read naturally.\n\
         - If the excerpt is already correct, return only the correct signal.\n\
         - Only when a detected item has no explicit glossary mapping in Patch glossary terms may you decide whether that specific detected item should become a newly confirmed term.\n\
         - If you decide a currently detected unmapped item is a real term worth future consistency, it must be one of: a stable technical term, coined label, recurring proper name, or recurring fiction/narrative story-world noun.\n\
         - Do not propose confirmed terms for ordinary leftover English words, adjectives, verbs, adverbs, one-off phrases, or any item outside the current excerpt patch candidates.\n\n\
         {fiction_rules}\
         Preservation rules for this article type:\n{preservation_rules}\n\n\
         Patch glossary terms:\n{glossary}\n\n\
         Style guidance:\n\
         - Prefer natural sentence-level phrasing over direct lexical substitution.\n\
         - For fiction or narrative text, preserve atmosphere, cadence, and readability.\n\
         - If a tiny local rewrite is needed so the repaired excerpt reads like original Chinese prose, do it, but do not rewrite unrelated content.\n\n\
         Source excerpt:\n{source_excerpt}\n\n\
         Current translated excerpt:\n{translated_excerpt}\n\n\
         Return JSON only:\n\
         {{\"decision\":\"correct\",\"confirmedTerms\":[]}}\n\
         or\n\
         {{\"decision\":\"repair\",\"repairedText\":\"complete repaired translated excerpt\",\"confirmedTerms\":[{{\"source\":\"term in source\",\"translation\":\"term used in repairedText\",\"termType\":\"coinage|disputed|person|location|organization|title|technical\",\"confidence\":95,\"usageRole\":\"coinage_noun|technical_term|person_name|place_name|organization_name|book_title\"}}]}}"
    )
}

pub fn build_llm_sentence_patch_prompt(
    article_type: &str,
    target: &SentencePatchTarget,
    patch_terms: &[(&str, &str)],
) -> String {
    let preservation_rules = sentence_patch_preservation_rules(article_type);
    let fiction_rules = residual_story_world_rules(article_type);
    let detected_terms = if patch_terms.is_empty() {
        "- No detected residual term candidates.".to_string()
    } else {
        patch_terms
            .iter()
            .map(|(source, _)| format!("- {}", source))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let glossary = if patch_terms.is_empty() {
        "- No explicit patch glossary terms.".to_string()
    } else {
        patch_terms
            .iter()
            .map(|(source, target)| {
                let target = target.trim();
                if target.is_empty() {
                    format!("- {} => translate naturally in this sentence", source)
                } else {
                    format!("- {} => {}", source, target)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "You are a translation repair agent. Your task is minimal sentence-level repair only.\n\
         Article type: {article_type}\n\n\
         Goal:\n\
         - Repair only the target translated sentence.\n\
         - Keep all unrelated wording, tone, structure, citations, and markdown unchanged.\n\
         - Keep the repaired sentence fluent and natural in Chinese, not like a glossary entry pasted into prose.\n\
         - Only change the matched term or short phrase, or the smallest nearby wording needed so the full sentence reads naturally.\n\
         - If the sentence is already correct, do not rewrite it. Return only the correct signal.\n\n\
         - Treat the detected items below as an explicit checklist for this sentence.\n\
         - If a detected item appears literally in the current translated sentence, replace that exact word or short phrase unless a preservation rule below clearly protects it.\n\
         - If no target translation is supplied for a detected item, infer the most natural Chinese translation from the source sentence and local context.\n\
         - Only when a detected item has no explicit glossary mapping in Patch glossary terms may you decide whether that specific detected item should become a newly confirmed term.\n\
         - If you decide a currently detected unmapped item is a real term worth future consistency, it must be one of: a stable technical term, coined label, recurring proper name, or recurring fiction/narrative story-world noun.\n\
         - Do not propose confirmed terms for ordinary leftover English words, adjectives, verbs, adverbs, one-off phrases, or any item not in the detected list.\n\
         - If an English adjective, noun, verb, or adverb remains inside a Chinese sentence, it is not correct; translate that minimal phrase.\n\
         - For fiction/narrative text, ordinary descriptive words such as manner, motion, shape, light, sound, fear, body, and place words should normally be translated, not preserved.\n\
         - For fiction or narrative text, prefer idiomatic narrative phrasing over abstract or technical diction when both are faithful.\n\n\
         {fiction_rules}\
         Preservation rules for this article type:\n{preservation_rules}\n\n\
         Detected possible untranslated source terms for this patch:\n{detected_terms}\n\n\
         Patch glossary terms:\n{glossary}\n\n\
         Source sentence:\n{source_sentence}\n\n\
         Current translated sentence:\n{translated_sentence}\n\n\
         Source context window:\n{source_context}\n\n\
         Translated context window:\n{translated_context}\n\n\
         Return JSON only:\n\
         {{\"decision\":\"correct\",\"confirmedTerms\":[]}}\n\
         or\n\
         {{\"decision\":\"patch\",\"patchedSentence\":\"...\",\"confirmedTerms\":[{{\"source\":\"term in source\",\"translation\":\"term used in patchedSentence\",\"termType\":\"coinage|disputed|person|location|organization|title|technical\",\"confidence\":95,\"usageRole\":\"coinage_noun|technical_term|person_name|place_name|organization_name|book_title\"}}]}}",
        source_sentence = target.source_sentence,
        translated_sentence = target.translated_sentence,
        source_context = target.source_context,
        translated_context = target.translated_context,
    )
}

fn sentence_patch_preservation_rules(article_type: &str) -> &'static str {
    match crate::article_policy::ArticlePolicy::from_article_type(article_type) {
        crate::article_policy::ArticlePolicy::Academic => {
            "- Keep citation author names, acknowledgement names, bibliography names, established project or campaign names, DOI, URL, email, formulas, and code in English when they are intentionally preserved."
        }
        crate::article_policy::ArticlePolicy::Specialized => {
            "- Keep intentionally preserved product names, organization names, standard identifiers, DOI, URL, email, formulas, and code in English."
        }
        crate::article_policy::ArticlePolicy::Narrative => {
            "- Keep only genuinely required English such as DOI, URL, email, formulas, code, or intentionally preserved proper names."
        }
    }
}

fn residual_story_world_rules(article_type: &str) -> String {
    crate::translation_validation::runtime_translation_rules(article_type)
        .residual_repair
        .map(|rules| format!("{rules}\n"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_patch_prompt_lists_detected_possible_untranslated_terms() {
        let target = SentencePatchTarget {
            source_context: "First sentence. The mechanism induces deliberation. Third sentence."
                .to_string(),
            translated_context: "第一句。这一机制引发 deliberation。第三句。".to_string(),
            source_sentence: "The mechanism induces deliberation.".to_string(),
            translated_sentence: "这一机制引发 deliberation。".to_string(),
        };

        let prompt = build_llm_sentence_patch_prompt(
            "academic",
            &target,
            &[("deliberation", "审议"), ("nudge+", "助推+")],
        );

        assert!(prompt.contains("Detected possible untranslated source terms"));
        assert!(prompt.contains("- deliberation"));
        assert!(prompt.contains("- nudge+"));
        assert!(prompt.contains("deliberation => 审议"));
    }

    #[test]
    fn sentence_patch_prompt_treats_empty_patch_target_as_contextual_translation() {
        let target = SentencePatchTarget {
            source_context: "Its movement had hateful deliberation. The looming head shone."
                .to_string(),
            translated_context: "它的动作中带着可恨的 deliberation。looming 头部发出光芒。"
                .to_string(),
            source_sentence: "Its movement had hateful deliberation. The looming head shone."
                .to_string(),
            translated_sentence: "它的动作中带着可恨的 deliberation。looming 头部发出光芒。"
                .to_string(),
        };

        let prompt = build_llm_sentence_patch_prompt(
            "fiction",
            &target,
            &[("deliberation", ""), ("looming", "")],
        );

        assert!(prompt.contains("explicit checklist"));
        assert!(prompt.contains("deliberation => translate naturally in this sentence"));
        assert!(prompt.contains("looming => translate naturally in this sentence"));
        assert!(prompt.contains("For fiction/narrative text"));
    }

    #[test]
    fn fiction_sentence_patch_prompt_requires_invented_nouns_without_parenthetical_english() {
        let target = SentencePatchTarget {
            source_context: "The orpods came through the gate.".to_string(),
            translated_context: "orpods 穿过了门。".to_string(),
            source_sentence: "The orpods came through the gate.".to_string(),
            translated_sentence: "orpods 穿过了门。".to_string(),
        };

        let prompt = build_llm_sentence_patch_prompt("fiction", &target, &[("orpods", "")]);

        assert!(prompt.contains("invented creature names"));
        assert!(prompt.contains("do not keep the original English in parentheses"));
    }

    #[test]
    fn fiction_chunk_review_prompt_allows_alien_chants_but_not_story_world_nouns() {
        let terms = vec![ResidualEnglishTerm {
            term: "orpods".to_string(),
            count: 2,
            source_excerpt: "the approach of the orpods".to_string(),
            translated_excerpt: "orpods".to_string(),
        }];

        let prompt = build_llm_validation_chunk_review_prompt(
            "fiction",
            "The orpods approached. Yug! n'gha!",
            "orpods 逼近。Yug! n'gha!",
            &terms,
        );

        assert!(prompt.contains("story-world nouns"));
        assert!(prompt.contains("alien chants"));
    }

    #[test]
    fn paragraph_patch_prompt_uses_structured_fiction_rules() {
        let prompt = build_llm_paragraph_patch_prompt(
            "fiction",
            "The orpods moved with hateful deliberation.",
            "orpods 带着可恨的 deliberation 移动。",
            &[("orpods", ""), ("deliberation", "")],
            "paragraph",
        );

        assert!(prompt.contains("minimal paragraph-level repair"));
        assert!(prompt.contains("invented creature"));
        assert!(prompt.contains("do not append the original English in parentheses"));
        assert!(prompt.contains("deliberation => translate naturally"));
    }
}
