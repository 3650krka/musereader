mod residual;
mod sanitize;
mod style_rules;

#[cfg(test)]
pub use residual::build_llm_validation_review_prompt;
pub use residual::{
    build_llm_paragraph_patch_prompt, build_llm_sentence_patch_prompt,
    build_llm_validation_chunk_review_prompt, detect_residual_english_terms, ResidualEnglishTerm,
    SentencePatchTarget,
};
pub use sanitize::{sanitize_epub_chapter_markdown, sanitize_translated_markdown};
pub use style_rules::runtime_translation_rules;

#[cfg(test)]
mod tests {
    use super::*;

    struct ResidualCase<'a> {
        name: &'a str,
        article_type: &'a str,
        source: &'a str,
        translated: &'a str,
        expected_terms: &'a [&'a str],
    }

    fn assert_residual_terms(case: ResidualCase<'_>) {
        let issues = detect_residual_english_terms(case.article_type, case.source, case.translated);
        let actual_terms = issues
            .iter()
            .map(|issue| issue.term.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actual_terms, case.expected_terms, "case={}", case.name);
    }

    #[test]
    fn flags_plain_residual_english_in_fiction() {
        assert_residual_terms(ResidualCase {
            name: "fiction_plain_residual_word",
            article_type: "fiction",
            source: "a vague fear of the monstrous jungle",
            translated: "a translated line still keeps monstrous in place",
            expected_terms: &["monstrous"],
        });
    }

    #[test]
    fn ignores_half_width_parenthetical_terms() {
        assert_residual_terms(ResidualCase {
            name: "academic_half_width_parenthetical",
            article_type: "academic",
            source: "semantic memory improves recall",
            translated: "translated note (semantic memory)",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_full_width_parenthetical_terms() {
        assert_residual_terms(ResidualCase {
            name: "academic_full_width_parenthetical",
            article_type: "academic",
            source: "semantic memory improves recall",
            translated: "translated note \u{ff08}semantic memory\u{ff09}",
            expected_terms: &[],
        });
    }

    #[test]
    fn fiction_detects_residual_english_inside_parentheses() {
        assert_residual_terms(ResidualCase {
            name: "fiction_parenthetical_residual_word",
            article_type: "fiction",
            source: "a vague fear of the monstrous jungle",
            translated: "他仍感到一种（monstrous）的恐惧。",
            expected_terms: &["monstrous"],
        });
    }

    #[test]
    fn fiction_ignores_markdown_image_and_link_destinations() {
        assert_residual_terms(ResidualCase {
            name: "fiction_markdown_destinations",
            article_type: "fiction",
            source: "The map showed a harbor.\n\n![](imgs/harbor-map.png)\n\nSee [archive](notes/harbor.html).",
            translated: "地图显示了港口。\n\n![](imgs/harbor-map.png)\n\n见[档案](notes/harbor.html)。",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_author_before_year_citation() {
        assert_residual_terms(ResidualCase {
            name: "academic_author_year_citation",
            article_type: "academic",
            source: "Smith argued that memory changes.",
            translated: "Conclusion from Smith (2020).",
            expected_terms: &[],
        });
    }

    #[test]
    fn academic_detects_body_residual_before_parenthetical_citation() {
        assert_residual_terms(ResidualCase {
            name: "academic_body_residual_before_citation",
            article_type: "academic",
            source: "Debate and deliberation can help individuals achieve their objectives.",
            translated: "辩论和 deliberation 可以帮助个体实现其目标（John et al., 2019）。",
            expected_terms: &["deliberation"],
        });
    }

    #[test]
    fn ignores_multi_author_year_citations() {
        assert_residual_terms(ResidualCase {
            name: "academic_multi_author_citations",
            article_type: "academic",
            source: "Evidence was expanded later.",
            translated:
                "Smith and Jones (2020). Smith et al. (2021). (Smith, Jones, and Brown, 2022).",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_academic_author_name_before_numeric_reference() {
        assert_residual_terms(ResidualCase {
            name: "academic_numeric_reference_credit",
            article_type: "academic",
            source: "Kuang and Bicchieri examined vague language effects.",
            translated: "Finding by Kuang and Bicchieri [11].",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_academic_credit_author_statement_names() {
        assert_residual_terms(ResidualCase {
            name: "academic_credit_statement",
            article_type: "academic",
            source: "Jinyi Kuang conceptualized the paper. Cristina Bicchieri supervised it.",
            translated: "Jinyi Kuang: conceptualization. Cristina Bicchieri: supervision.",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_multiple_citations_inside_one_parenthetical_span() {
        assert_residual_terms(ResidualCase {
            name: "academic_multi_citation_parenthetical",
            article_type: "academic",
            source: "Support exists.",
            translated: "(Smith, 2020; Johnson and Lee, 2021; Brown et al., 2022).",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_reference_section_for_academic_text() {
        assert_residual_terms(ResidualCase {
            name: "academic_reference_section",
            article_type: "academic",
            source: "References\nSmith, J. Memory Studies.",
            translated: "## References\nSmith, J. Memory Studies. Journal of Tests.",
            expected_terms: &[],
        });
    }

    #[test]
    fn academic_body_word_references_does_not_mask_later_residuals() {
        assert_residual_terms(ResidualCase {
            name: "academic_body_references_word",
            article_type: "academic",
            source: "This section references the experimental protocol and later reports calibration drift.",
            translated: "本节 references 了实验协议，后文仍保留 calibration drift。",
            expected_terms: &["calibration", "drift", "references"],
        });
    }

    #[test]
    fn ignores_author_year_citation_name_in_body_clause() {
        assert_residual_terms(ResidualCase {
            name: "academic_body_author_year_name",
            article_type: "academic",
            source: "Kastrup, 2017). However, irrespective of whether this experience of reflection is conscious or unconscious, the argument still follows.",
            translated: "Kastrup, 2017). 然而，无论这种反思体验是有意识还是无意识，论证仍然成立。",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_type_label_example_names_in_academic_body() {
        assert_residual_terms(ResidualCase {
            name: "academic_type_label_example_names",
            article_type: "academic",
            source: "These characters, slightly renamed, Bobbie (type 1) and Joey (type 2), participate together in a pub quiz.",
            translated: "这些角色经过轻微改名后，Bobbie (type 1) 和 Joey (type 2) 一起参加了酒吧问答比赛。",
            expected_terms: &[],
        });
    }

    #[test]
    fn ignores_code_url_and_doi() {
        assert_residual_terms(ResidualCase {
            name: "tech_doc_code_url_doi",
            article_type: "tech_doc",
            source: "Use fetch and https://example.com with DOI 10.1000/test.",
            translated: "See `fetch()` near https://example.com. DOI: 10.1000/test.",
            expected_terms: &[],
        });
    }

    #[test]
    fn residual_detection_matrix_covers_regression_edges() {
        let cases = [
            ResidualCase {
                name: "narrative_detects_repeated_source_word",
                article_type: "fiction",
                source: "The lantern flickered above the harbor.",
                translated: "码头上方的 lantern 还在摇晃，lantern 没被翻掉。",
                expected_terms: &["lantern"],
            },
            ResidualCase {
                name: "narrative_ignores_word_absent_from_source",
                article_type: "fiction",
                source: "The lantern flickered above the harbor.",
                translated: "码头上方的 harborside 风很大。",
                expected_terms: &[],
            },
            ResidualCase {
                name: "academic_ignores_email_and_inline_code",
                article_type: "academic",
                source: "Contact the study team for replication material.",
                translated: "如需复现实验，请联系 test@example.com，并运行 `train_model()`。",
                expected_terms: &[],
            },
            ResidualCase {
                name: "specialized_ignores_dotted_identifiers",
                article_type: "tech_doc",
                source: "Configure the service client before startup.",
                translated: "请确认 com.example.ServiceClient 已经初始化。",
                expected_terms: &[],
            },
            ResidualCase {
                name: "fiction_detects_multiple_distinct_terms_in_order",
                article_type: "fiction",
                source: "The captain gripped the compass beside the rail.",
                translated: "船长握着 compass，脚边的 rail 也还没翻译。",
                expected_terms: &["compass", "rail"],
            },
            ResidualCase {
                name: "fiction_detects_embedded_manner_and_motion_words",
                article_type: "fiction",
                source: "Its movement had hateful deliberation. The looming head filled the cave with light.",
                translated: "它的动作中带着可恨的 deliberation。洞穴墙壁因它 looming 头部的光芒而变亮。",
                expected_terms: &["deliberation", "looming"],
            },
        ];

        for case in cases {
            assert_residual_terms(case);
        }
    }

    #[test]
    fn keeps_epub_without_short_title_unchanged() {
        let markdown = "# Chapter One\n\nThis is the first real paragraph.\n\nBody continues.";

        assert_eq!(sanitize_epub_chapter_markdown(markdown), markdown);
    }

    #[test]
    fn removes_repeated_epub_opening_title_pair() {
        let markdown = "# Chapter One\n\nChapter One\n\nThis is the first real paragraph.";
        let cleaned = sanitize_epub_chapter_markdown(markdown);

        assert_eq!(
            cleaned,
            "# Chapter One\n\nThis is the first real paragraph."
        );
    }

    #[test]
    fn builds_repair_agent_prompt_with_json_contract() {
        let prompt = build_llm_validation_review_prompt(
            "fiction",
            "monstrous",
            "a vague fear of the monstrous jungle",
            "a translated line still keeps monstrous in place",
            "a translated line still keeps monstrous in place",
        );

        assert!(prompt.contains("\"decision\":\"false_positive\""));
        assert!(prompt.contains("\"decision\":\"repair\""));
        assert!(prompt.contains("\"repairedText\""));
    }
}
