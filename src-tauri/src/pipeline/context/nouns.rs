mod collection;
mod rules;

use super::super::chunking::split_reference_aware_markdown;
use super::*;
use crate::pipeline::StaticGlossaryEntry;
use collection::extract_proper_noun_hints;

pub(crate) fn build_translation_context(
    source_markdown: &str,
    chunk_size: usize,
    article_type: &str,
    static_terms: &[StaticGlossaryEntry],
) -> Option<TranslationContext> {
    let chunks = split_reference_aware_markdown(source_markdown, chunk_size, article_type);
    let proper_nouns = extract_proper_noun_hints(&chunks, article_type);
    let static_terms = static_terms.to_vec();
    (!proper_nouns.is_empty() || !static_terms.is_empty()).then_some(TranslationContext {
        proper_nouns,
        static_terms,
    })
}

pub(crate) fn should_localize_unresolved_proper_nouns(article_type: &str) -> bool {
    // 叙事类文本对未解析专名做本地化音译（集中策略见 article_policy）。
    crate::article_policy::ArticlePolicy::from_article_type(article_type).is_narrative()
}

pub(crate) fn infer_strict_hit_role(source: &str) -> (&'static str, &'static str) {
    let normalized = source.trim();
    if normalized.starts_with("The Tale of ") {
        return ("book_title", "title");
    }
    if normalized.starts_with("Queen ")
        || normalized.starts_with("King ")
        || normalized.starts_with("Prince ")
        || normalized.starts_with("Princess ")
        || normalized.starts_with("Lord ")
        || normalized.starts_with("Lady ")
    {
        return ("person_name", "person");
    }
    if normalized.starts_with("Temple of ")
        || normalized.starts_with("Church of ")
        || normalized.starts_with("University of ")
    {
        return ("organization_name", "organization");
    }
    if normalized.contains(" of ") {
        return ("other", "technical");
    }
    if normalized.split_whitespace().count() >= 2 {
        return ("person_name", "person");
    }
    ("place_name", "location")
}

#[cfg(test)]
mod tests {
    use super::rules::{derived_fictional_root, has_people_or_group_suffix};
    use super::*;

    fn hint_snapshot(
        context: &TranslationContext,
    ) -> Vec<(String, Option<String>, ProperNounEnforcement, usize, usize)> {
        context
            .proper_nouns
            .iter()
            .map(|hint| {
                (
                    hint.source.clone(),
                    hint.target.clone(),
                    hint.enforcement,
                    hint.occurrences,
                    hint.first_chunk_index,
                )
            })
            .collect()
    }

    #[test]
    fn translation_context_preserves_high_value_fiction_hints() {
        let source = [
            "Satampra Zeiros walked beside Queen Cunambria.",
            "Satampra Zeiros returned to Hyperborea.",
            "Hyperborea trembled at dusk.",
        ]
        .join("\n");

        let context = build_translation_context(&source, 120, "fiction", &[])
            .expect("fiction sample should produce translation context");
        let snapshot = hint_snapshot(&context);

        assert_eq!(snapshot.len(), 3);
        assert!(snapshot.contains(&(
            "Satampra Zeiros".to_string(),
            None,
            ProperNounEnforcement::Strict,
            2,
            0,
        )));
        assert!(snapshot.contains(&(
            "Queen Cunambria".to_string(),
            None,
            ProperNounEnforcement::Strict,
            1,
            0,
        )));
        assert!(snapshot.contains(&(
            "Hyperborea".to_string(),
            None,
            ProperNounEnforcement::Strict,
            2,
            0,
        )));
    }

    #[test]
    fn translation_context_filters_noise_and_short_singletons() {
        let source = [
            "The Captain arrived at dawn.",
            "Captain looked around again.",
            "Now Morgan looked around.",
            "Morgan answered quietly.",
            "Though the road was dark, the riders continued.",
            "Though the city slept, the bells rang.",
            "Before the moon rose, the door opened.",
            "Before the dawn, the house vanished.",
            "Perhaps the witness lied.",
            "Perhaps the map was wrong.",
            "ABC shouted once.",
            "Mars flickered.",
        ]
        .join("\n");

        let context = build_translation_context(&source, 120, "fiction", &[])
            .expect("sample should still produce translation context");
        let names = context
            .proper_nouns
            .iter()
            .map(|hint| hint.source.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"Captain"));
        assert!(names.contains(&"Morgan"));
        assert!(!names.contains(&"The Captain"));
        assert!(!names.contains(&"Now Morgan"));
        assert!(!names.contains(&"Though"));
        assert!(!names.contains(&"Before"));
        assert!(!names.contains(&"Perhaps"));
        assert!(!names.contains(&"ABC"));
        assert!(!names.contains(&"Mars"));
    }

    #[test]
    fn translation_context_skips_academic_reference_and_metadata_chunks() {
        let source = [
            "# Paper",
            "",
            "Alice Smith proposed the first model.",
            "",
            "## References",
            "",
            "Smith, A. and Johnson, B. (2020). Journal of Tests.",
            "",
            "<!-- Authors: Alice Smith, Bob Johnson -->",
        ]
        .join("\n");

        let context = build_translation_context(&source, 80, "academic", &[])
            .expect("academic body should still yield context");
        let names = context
            .proper_nouns
            .iter()
            .map(|hint| hint.source.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"Alice Smith"));
        assert!(!names.contains(&"Smith"));
        assert!(!names.contains(&"Johnson"));
        assert!(!names.contains(&"Journal"));
        assert!(!names.contains(&"Authors"));
    }

    #[test]
    fn academic_translation_context_filters_sentence_initial_common_word_hints() {
        let source = [
            "# Paper",
            "",
            "Another experiment showed the same pattern.",
            "Furthermore, the effect remained stable.",
            "Nonetheless, the limitation should be noted.",
            "Another analysis was reported in the appendix.",
            "Furthermore, researchers replicated the result.",
            "Nonetheless, more work is needed.",
            "Evidence from Western countries is overrepresented.",
            "Experts outside Western countries contributed additional studies.",
            "## Table 1 | Overview of intervention types",
            "Table All interventions and outcomes",
            "labels (Table 1). All nine intervention types are supported.",
            "Structure                                      Experts                                      Criteria for inclusion",
            "Accuracy prompts                Inoculation                Social norms",
            crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START,
            "University of Western Australia, Australia.",
            crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END,
            "Sunstein, 2016 argued for transparent nudges.",
            "Later work followed Sunstein, 2019 on educative nudges.",
            "William Brady commented on the study.",
            "Ziv Epstein commented on the study.",
        ]
        .join("\n");

        let context = build_translation_context(&source, 500, "academic", &[])
            .expect("academic sample should produce proper noun context");
        let names = context
            .proper_nouns
            .iter()
            .map(|hint| hint.source.as_str())
            .collect::<Vec<_>>();

        assert!(!names.contains(&"Another"));
        assert!(!names.contains(&"Furthermore"));
        assert!(!names.contains(&"Nonetheless"));
        assert!(!names.contains(&"Western"));
        assert!(!names.contains(&"Table All"));
        assert!(!names.contains(&"Structure Experts Criteria"));
        assert!(!names.contains(&"Accuracy Inoculation Social"));
        assert!(!names.contains(&"Overview"));
        assert!(names.contains(&"Sunstein"));
        assert!(names.contains(&"William Brady"));
        assert!(names.contains(&"Ziv Epstein"));
    }

    #[test]
    fn contextual_derivative_detection_marks_people_suffix_forms() {
        assert!(has_people_or_group_suffix("许珀耳玻瑞亚人"));
        assert!(!has_people_or_group_suffix("许珀耳玻瑞亚"));
        assert_eq!(
            derived_fictional_root("Hyperborean").as_deref(),
            Some("Hyperbora")
        );
    }

    #[test]
    fn translation_context_ignores_markdown_structure_noise() {
        let source = [
            "# The Arrival",
            "",
            "The End of the Story",
            "",
            "The End of the Story",
            "",
            "![Queen Cunambria](imgs/Queen-Cunambria.png)",
            "",
            "See [Appendix Notes](notes/Appendix-Notes.html).",
            "",
            "Queen Cunambria entered the hall.",
            "Queen Cunambria spoke softly.",
        ]
        .join("\n");

        let context = build_translation_context(&source, 160, "fiction", &[])
            .expect("sample should produce real proper noun context");
        let names = context
            .proper_nouns
            .iter()
            .map(|hint| hint.source.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"Queen Cunambria"));
        assert!(!names.contains(&"The Arrival"));
        assert!(!names.contains(&"End of the Story"));
        assert!(!names.contains(&"The End of the Story"));
        assert!(!names.contains(&"Appendix Notes"));
        assert!(!names.contains(&"Queen-Cunambria"));
    }

    #[test]
    fn translation_context_filters_repeated_running_titles_across_chunks() {
        let source = [
            "Chronicles of Mars",
            "",
            "Artemis Vale entered the observatory.",
            "",
            "Chronicles of Mars",
            "",
            "Artemis Vale studied the red horizon.",
            "",
            "Chronicles of Mars",
            "",
            "Artemis Vale closed the brass telescope.",
        ]
        .join("\n");

        let context = build_translation_context(&source, 70, "fiction", &[])
            .expect("sample should produce real proper noun context");
        let names = context
            .proper_nouns
            .iter()
            .map(|hint| hint.source.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"Artemis Vale"));
        assert!(!names.contains(&"Chronicles of Mars"));
    }

    #[test]
    fn translation_context_keeps_static_glossary_entries() {
        let context = build_translation_context(
            "Ordinary paragraph without detected names.",
            120,
            "fiction",
            &[StaticGlossaryEntry {
                source: "Nudge plus".to_string(),
                target: "助推升级".to_string(),
                scope: "global".to_string(),
                enforcement: "strict".to_string(),
                notes: String::new(),
            }],
        )
        .expect("static glossary should still produce translation context");

        assert_eq!(context.static_terms.len(), 1);
        assert_eq!(context.static_terms[0].source, "Nudge plus");
        assert_eq!(context.static_terms[0].target, "助推升级");
    }
}
