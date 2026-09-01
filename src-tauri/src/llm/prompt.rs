pub(super) fn build_translation_user_prompt(
    source_text: &str,
    article_type: &str,
    translation_context: Option<&str>,
) -> String {
    let mut prompt = String::from(
        "Task: translate the Markdown source inside <SOURCE>...</SOURCE> into natural Simplified Chinese and return one JSON object.\n\
         Output rules:\n\
         1. Return exactly one valid JSON object. Do not output markdown fences, explanations, notes, or any text before or after the JSON.\n\
         2. translatedText must contain the complete final Chinese translation and must preserve markdown structure, headings, lists, tables, links, formulas, citations, and HTML tags.\n\
         3. For normal body prose, do not leave full English sentences or paragraphs untranslated.\n\
         4. English may remain only when it is genuinely required, such as DOI, URL, email, code, formula, reference entries, standard abbreviations, or names that the context/glossary explicitly requires to remain in English.\n\
         5. If a source paragraph is difficult, still translate it as fully as possible into Chinese rather than copying the original English.\n\
         6. Maintain terminology consistency for recurring named concepts, titles, people, places, organizations, and source-specific terms.\n\
         7. confirmedTerms should include newly confirmed cross-chunk glossary terms worth enforcing for the selected article type. Keep this list SMALL and selective: include only terms that (a) genuinely recur across chunks and (b) need a locked rendering. Do NOT enumerate every person/place/organization mentioned once, and do NOT include routine real-world names (doctors, editors, public figures, publishers) that appear in a single passage and do not recur. When in doubt, leave it out.\n\
         8. Each confirmed term must use the exact source surface from the input and a translation that appears in translatedText. If a proper noun already appears in the translation context (algorithmic candidates or glossary hints), prefer returning that context's canonical source surface rather than a variant form, so the system can lock it consistently.\n\
         9. Preserve paragraph structure exactly: do not merge, split, delete, duplicate, or reorder Markdown paragraphs/blocks. Keep blank-line paragraph boundaries, heading levels, list item boundaries, table rows, image lines, formulas, HTML comments, and passthrough metadata in the same relative positions.\n\
         10. If block alignment markers like [[B007]] appear, copy each marker exactly once before the corresponding translated block. Do not translate, renumber, delete, duplicate, or move these markers.\n\
         11. Translate ordinary English lexical content in prose into Chinese. Keep English only for structural or explicitly required content such as DOI, URL, email, code, formulas, reference entries, standard abbreviations, citations, required names, or glossary-mandated surfaces.\n\
         12. If a coined or source-specific term includes symbols such as +, -, *, /, or similar notation, preserve the symbol as part of the term surface while translating the lexical part when appropriate.\n\
         13. Table-of-contents and chapter heading rules: when <SOURCE> contains a table of contents block or heading lines (e.g. \"Chapter 11: Title\", \"第11章：标题\", numbered list entries with page numbers), keep the entry count and hierarchy exactly the same — never split, merge, add, or drop entries. Translate only the title text of each entry; keep page numbers, dotted leaders, anchors, and links unchanged. Numbering must follow the source entry's system: if the source entry uses Arabic numerals (Chapter 11 / 第11章 / 3.2), use Arabic numerals for every entry; if it uses Chinese numerals (第十一章), use Chinese numerals — never mix Arabic and Chinese numerals inside the same TOC or chapter series. Colons inside entries must be full-width \"：\" and never mixed with \":\"; if a source entry has no colon, do not add one.",
    );
    // Silent self-review 工作流（precision-rewrite 思路）：一次调用内
    // 「草稿→自审→终稿」，零额外请求消除翻译腔与漏译。
    // MUSETRANSLATE_SELF_REVIEW=0 可关（极短 chunk 或追求最快出稿时）。
    if !crate::pipeline::env_flag_disabled("MUSETRANSLATE_SELF_REVIEW") {
        prompt.push_str(
            "\n\
         Internal workflow (never reveal it in the output):\n\
         - First draft the translation silently.\n\
         - Then silently review the draft for translationese, omissions, terminology drift against the provided context, and unnatural Chinese word order.\n\
         - Output only the final, revised translation in translatedText. The draft and review notes must never appear in the output.",
        );
    }
    append_article_type_rules(&mut prompt, article_type);
    prompt.push_str(
        "\n         Only output this JSON shape:\n\
         {\"translatedText\":\"final translation\",\"confirmedTerms\":[{\"source\":\"term in source\",\"translation\":\"term used in translation\",\"termType\":\"coinage|disputed|person|location|organization|title|technical\",\"confidence\":95,\"usageRole\":\"coinage_modifier\"}]}",
    );
    if let Some(context) = translation_context {
        if !context.trim().is_empty() {
            prompt.push_str("\n\nInternal execution constraints below. Do not translate or repeat them in the output.\n");
            prompt.push_str(context.trim());
            prompt.push_str(
                "\nApply the constraints above, then translate only the content inside <SOURCE>...</SOURCE>.",
            );
        }
    }
    prompt.push_str("\n\n<SOURCE>\n");
    prompt.push_str(source_text);
    prompt.push_str("\n</SOURCE>");
    prompt
}

fn append_article_type_rules(prompt: &mut String, article_type: &str) {
    let policy = crate::article_policy::ArticlePolicy::from_article_type(article_type);
    if policy.is_academic() {
        prompt.push_str(
            "\n\
         Academic article rules:\n\
         - For recurring academic concepts, named methods, theoretical constructs, coined labels, and title-like phrases, choose one Chinese rendering and keep it stable.\n\
         - In academic prose, be especially strict with ordinary theoretical vocabulary and abstract nouns: translate them unless a general structural exception or explicit glossary/context constraint applies.\n\
         - Preserve semantic distinctions in academic coined labels: do not add meanings that are not in the source.",
        );
    }
    // 历史行为：fiction 专属（非全部 narrative）注入小说叙事规则。
    if article_type.trim().eq_ignore_ascii_case("fiction") {
        prompt.push_str(
            "\n\
         Fiction/narrative rules:\n\
         - Recurring invented creature names, object names, species names, vehicles, weapons, materials, places, and named artifacts should be translated or transliterated into natural Chinese and recorded in confirmedTerms when they may recur.\n\
         - Record ALL story-world proper nouns and main character names in confirmedTerms (even if they appear only once), so the system can lock their translations for later chunks.\n\
         - Do not append the original English in parentheses for localized fiction/narrative story-world terms.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::build_translation_user_prompt;

    #[test]
    fn prompt_wraps_full_source_inside_source_tags() {
        let source = "The second stage corresponded to the elicitation of the demand to receive extra information.";
        let prompt = build_translation_user_prompt(source, "academic", None);

        assert!(prompt.contains("<SOURCE>\nThe second stage corresponded to the elicitation of the demand to receive extra information.\n</SOURCE>"));
    }

    #[test]
    fn prompt_keeps_context_outside_source_block() {
        let source = "Body paragraph";
        let context = "Strict glossary: Veracity -> zhenshixing";
        let prompt = build_translation_user_prompt(source, "academic", Some(context));

        let source_start = prompt.rfind("<SOURCE>").expect("source start should exist");
        let context_start = prompt.find(context).expect("context should exist");

        assert!(context_start < source_start);
        assert!(prompt[source_start..].contains(source));
    }

    #[test]
    fn prompt_limits_translation_to_source_when_reference_context_exists() {
        let source = "Current paragraph.";
        let context = "Reference-only previous paragraph for context. Do not translate it.";
        let prompt = build_translation_user_prompt(source, "fiction", Some(context));

        assert!(prompt.contains("translate only the content inside <SOURCE>...</SOURCE>"));
        assert!(prompt.contains(context));
    }

    #[test]
    fn academic_prompt_enforces_academic_terms_only() {
        let prompt = build_translation_user_prompt(
            "Nudge plus combines heuristics and deliberation.",
            "academic",
            None,
        );

        assert!(prompt.contains("recurring academic concepts"));
        assert!(prompt.contains("ordinary theoretical vocabulary and abstract nouns"));
        assert!(prompt.contains("do not add meanings that are not in the source"));
        assert!(!prompt.contains("do not translate \"Nudge plus\""));
        assert!(!prompt.contains("upgrade, enhancement, or improvement"));
        assert!(!prompt.contains("invented creature names"));
        assert!(!prompt.contains("story-world terms"));
    }

    #[test]
    fn fiction_prompt_enforces_story_world_terms_only() {
        let prompt =
            build_translation_user_prompt("The orpods crossed the bright plain.", "fiction", None);

        assert!(prompt.contains("invented creature names"));
        assert!(prompt.contains("confirmedTerms"));
        assert!(prompt.contains("Do not append the original English in parentheses"));
        assert!(prompt.contains("symbols such as +, -, *, /"));
        assert!(prompt.contains("Translate ordinary English lexical content"));
        assert!(!prompt.contains("recurring academic concepts"));
        assert!(!prompt.contains("ordinary theoretical vocabulary and abstract nouns"));
        assert!(!prompt.contains("Nudge plus"));
    }

    #[test]
    fn prompt_avoids_ambiguous_established_proper_noun_rule() {
        let prompt = build_translation_user_prompt(
            "Marie Curie founded no institute here.",
            "academic",
            None,
        );

        assert!(!prompt.contains("established proper nouns"));
        assert!(prompt.contains("context/glossary explicitly requires to remain in English"));
    }

    #[test]
    fn prompt_enforces_paragraph_structure_preservation_for_all_article_types() {
        let prompt = build_translation_user_prompt("One.\n\nTwo.", "fiction", None);

        assert!(prompt.contains("Preserve paragraph structure exactly"));
        assert!(prompt.contains("do not merge, split, delete, duplicate, or reorder"));
        assert!(prompt.contains("Keep blank-line paragraph boundaries"));
        assert!(prompt.contains("[[B007]]"));
        assert!(prompt.contains("copy each marker exactly once"));
    }

    #[test]
    fn prompt_enforces_toc_entry_consistency_rules() {
        let prompt = build_translation_user_prompt(
            "Chapter 11: Tash\nChapter 12: Reepicheep",
            "fiction",
            None,
        );

        assert!(prompt.contains("Table-of-contents and chapter heading rules"));
        assert!(prompt.contains("never split, merge, add, or drop entries"));
        assert!(prompt.contains("never mix Arabic and Chinese numerals"));
        assert!(prompt.contains("full-width \"：\""));
    }
}
