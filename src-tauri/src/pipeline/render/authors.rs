use serde::{Deserialize, Serialize};

mod decorate;
#[cfg(test)]
mod llm;
mod parse;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcademicAuthor {
    pub(crate) name: String,
    pub(crate) email: Option<String>,
    #[serde(default)]
    pub(crate) markers: Vec<String>,
    #[serde(default)]
    pub(crate) affiliations: Vec<String>,
    #[serde(default)]
    pub(crate) is_corresponding: bool,
    #[serde(default)]
    pub(crate) affiliation_refs: Vec<String>,
    #[serde(default)]
    pub(crate) equal_contribution: Vec<String>,
    #[serde(default)]
    pub(crate) author_notes: Vec<String>,
    pub(crate) correspondence_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcademicAffiliationEntry {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcademicFrontMatterExtras {
    pub(crate) raw: String,
    pub(crate) kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcademicFrontMatter {
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) authors: Vec<AcademicAuthor>,
    #[serde(default)]
    pub(crate) extras: Vec<String>,
    #[serde(default)]
    pub(crate) affiliation_catalog: Vec<AcademicAffiliationEntry>,
    #[serde(default)]
    pub(crate) extra_entries: Vec<AcademicFrontMatterExtras>,
}

fn canonical_author_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(super) fn extract_academic_authors(markdown: &str) -> Vec<AcademicAuthor> {
    parse::extract_academic_authors(markdown)
}

pub(crate) fn has_academic_front_matter(front_matter: &AcademicFrontMatter) -> bool {
    !front_matter.authors.is_empty() || !front_matter.extras.is_empty()
}

#[cfg(test)]
pub(super) fn should_refine_academic_authors(authors: &[AcademicAuthor]) -> bool {
    !authors.is_empty()
        && authors.iter().any(|author| {
            (author.is_corresponding && author.email.is_none())
                || author.affiliations.is_empty()
                || author.markers.is_empty()
        })
}

#[cfg(test)]
fn decorate_academic_authors(html: &str, authors: &[AcademicAuthor]) -> String {
    decorate::decorate_academic_authors(html, authors)
}

pub(super) fn render_academic_front_matter(
    authors: &[AcademicAuthor],
    extras: &[String],
) -> String {
    decorate::render_academic_front_matter(authors, extras)
}

#[cfg(test)]
pub(super) async fn refine_academic_authors_with_llm(
    llm_client: &crate::llm::SensenovaClient,
    markdown: &str,
    first_page_footnotes: &[String],
    authors: &[AcademicAuthor],
) -> Result<Vec<AcademicAuthor>, crate::error::AppError> {
    llm::refine_academic_authors_with_llm(llm_client, markdown, first_page_footnotes, authors).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorates_only_author_paragraph() {
        let authors = vec![AcademicAuthor {
            name: "Jinyi Kuang".to_string(),
            email: Some("jinyi@example.test".to_string()),
            markers: vec!["1".to_string(), "2".to_string(), "a".to_string()],
            affiliations: vec!["University".to_string()],
            is_corresponding: false,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: Vec::new(),
            correspondence_note: None,
        }];
        let html = "<p>Jinyi Kuang $ ^{1,2,a} $</p><p>Jinyi Kuang appears later.</p>";

        let output = decorate_academic_authors(html, &authors);

        assert_eq!(output.matches("academic-author-link").count(), 1);
        assert!(output.contains("<sup>1,2,a</sup>"));
        assert!(output.contains("academic-author-popover"));
        assert!(output.contains("<p>Jinyi Kuang appears later.</p>"));
    }

    #[test]
    fn decorates_author_fragment_without_opening_paragraph_tag() {
        let authors = vec![AcademicAuthor {
            name: "Jinyi Kuang".to_string(),
            email: Some("jinyi@example.test".to_string()),
            markers: vec!["1".to_string(), "2".to_string(), "a".to_string()],
            affiliations: vec!["University".to_string()],
            is_corresponding: false,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: Vec::new(),
            correspondence_note: None,
        }];
        let html =
            "Jinyi Kuang <sup>1,2,a</sup> and Cristina Bicchieri <sup>1,2,3,a</sup></p><p>Body</p>";

        let output = decorate_academic_authors(html, &authors);

        assert!(output.contains("academic-author-line"));
        assert!(output.contains("academic-author-popover"));
        assert!(output.contains("<p>Body</p>"));
    }

    #[test]
    fn decorates_realistic_translated_author_block_before_abstract() {
        let authors = vec![
            AcademicAuthor {
                name: "桑查扬·班纳吉".to_string(),
                email: None,
                markers: vec!["ID".to_string()],
                affiliations: vec!["伦敦政治经济学院地理与环境系，英国伦敦".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            },
            AcademicAuthor {
                name: "彼得·约翰".to_string(),
                email: Some("peter.john@kcl.ac.uk".to_string()),
                markers: vec!["ID".to_string(), "*".to_string()],
                affiliations: vec!["伦敦国王学院政治经济系，英国伦敦".to_string()],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some(
                    "通讯作者：伦敦国王学院政治经济系，布什大厦，奥尔德威奇40号，伦敦WC2B 4PH，英国。"
                        .to_string(),
                ),
            },
        ];
        let html = concat!(
            "桑查扬·班纳吉<sup>ID</sup>与彼得·约翰<sup>ID,*</sup></p>",
            "<p>伦敦政治经济学院地理与环境系，英国伦敦；及伦敦国王学院政治经济系，英国伦敦</p>",
            "<p><sup>*</sup>通讯作者：伦敦国王学院政治经济系，布什大厦，奥尔德威奇40号，伦敦WC2B 4PH，英国。</p>",
            "<p>电子邮箱：peter.john@kcl.ac.uk</p>",
            "<h2 id=\"摘要\">摘要</h2>",
            "<p>正文</p>"
        );

        let output = decorate_academic_authors(html, &authors);

        assert!(output.contains("academic-author-line"));
        assert!(output.contains("academic-author-link"));
        assert!(output.contains("academic-author-popover"));
        assert!(output.contains("mailto:peter.john@kcl.ac.uk"));
        assert!(output.contains("通讯作者"));
        assert!(output.contains("<h2 id=\"摘要\">摘要</h2>"));
        assert!(output.contains("<p>正文</p>"));
        assert!(!output.contains("<div class=\"academic-author-popover-row\">"));
        assert!(
            !output.contains("<div class=\"academic-author-popover-row academic-author-role\">")
        );
        assert!(
            !output.contains("<div class=\"academic-author-popover-row academic-author-email\">")
        );
    }

    #[test]
    fn decorates_ocr_author_line_without_leaking_raw_id_text() {
        let authors = vec![
            AcademicAuthor {
                name: "Sanchayan Banerjee".to_string(),
                email: None,
                markers: vec!["ID".to_string()],
                affiliations: vec![
                    "\u{4F26}\u{6566}\u{653F}\u{6CBB}\u{7ECF}\u{6D4E}\u{5B66}\u{9662}".to_string(),
                ],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            },
            AcademicAuthor {
                name: "Peter John".to_string(),
                email: Some("peter.john@kcl.ac.uk".to_string()),
                markers: vec!["ID".to_string(), "*".to_string()],
                affiliations: vec!["\u{4F26}\u{6566}\u{56FD}\u{738B}\u{5B66}\u{9662}".to_string()],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some(
                    "\u{901A}\u{8BAF}\u{4F5C}\u{8005}\u{FF1A}\u{56FD}\u{738B}\u{5B66}\u{9662}"
                        .to_string(),
                ),
            },
        ];
        let html =
            "<p>Sanchayan Banerjee ID \u{548C} Peter John* ID</p><h2 id=\"abstract\">Abstract</h2>";

        let output = decorate_academic_authors(html, &authors);
        let author_line = output
            .split("<h2")
            .next()
            .expect("author line should precede abstract");

        assert!(author_line.contains("<sup>ID</sup>"));
        assert!(author_line.contains("<sup>ID,*</sup>"));
        assert!(author_line.contains("academic-author-popover"));
        assert!(!author_line.contains("Banerjee</a><sup>ID</sup> ID"));
        assert!(!author_line.contains("John</a><sup>ID,*</sup>* ID"));
        assert!(!author_line.contains("> ID \u{548C}"));
        assert!(!author_line.contains("* ID</p>"));
    }

    #[test]
    fn keeps_affiliation_inside_popover_markup() {
        let authors = vec![AcademicAuthor {
            name: "桑查扬·班纳吉".to_string(),
            email: None,
            markers: vec!["ID".to_string()],
            affiliations: vec![
                "伦敦政治经济学院地理与环境系，英国伦敦；及伦敦国王学院政治经济系，英国伦敦"
                    .to_string(),
            ],
            is_corresponding: false,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: Vec::new(),
            correspondence_note: None,
        }];
        let html = "<p>桑查扬·班纳吉（ID）</p><h2 id=\"摘要\">摘要</h2><p>正文</p>";

        let output = decorate_academic_authors(html, &authors);

        assert!(output.contains(
            "<span id=\"author-meta-桑查扬·班纳吉\" class=\"academic-author-popover\" role=\"tooltip\"><span class=\"academic-author-popover-row\">伦敦政治经济学院地理与环境系，英国伦敦；及伦敦国王学院政治经济系，英国伦敦</span></span>"
        ));
        assert_eq!(output.matches("academic-author-popover-row").count(), 1);
    }

    #[test]
    fn converts_generic_author_markers() {
        let html = "<p>A. Name $ ^{2,b,10} $</p>";

        let output = decorate::convert_author_marker_math_to_sup(html);

        assert!(output.contains("<sup>2,b,10</sup>"));
    }

    #[test]
    fn merges_duplicate_affiliation_markers() {
        let markdown = "$ ^{2} $ Department of Psychology, University A\n$ ^{2} $ Center for Social Norms, University A\n";

        let affiliations = parse::extract_affiliation_map(markdown);

        assert_eq!(
            affiliations.get("2"),
            Some(
                &"Department of Psychology, University A | Center for Social Norms, University A"
                    .to_string()
            )
        );
    }

    #[test]
    fn parses_corresponding_author_metadata_from_note_and_email_lines() {
        let markdown = "# Title\n\nPeter John $ ^{1,*} $\n\n$ ^{1} $ Department of Political Economy, King's College London\n\n$ ^{*} $通讯作者：Department of Political Economy, Bush House, London\n电子邮箱：Peter John (peter.john@kcl.ac.uk)\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "Peter John");
        assert_eq!(authors[0].email.as_deref(), Some("peter.john@kcl.ac.uk"));
        assert!(authors[0].is_corresponding);
        assert!(authors[0]
            .correspondence_note
            .as_deref()
            .is_some_and(|note| note.contains("通讯作者")));
        assert!(authors[0]
            .affiliations
            .iter()
            .any(|item| item.contains("King's College London")));
    }

    #[test]
    fn parses_translated_author_line_with_parenthetical_markers() {
        let markdown = "# 标题\n\n桑查扬·班纳吉（ID）与彼得·约翰*（ID）\n\n伦敦政治经济学院地理与环境系，英国伦敦；及伦敦国王学院政治经济系，英国伦敦\n\n$ ^{*} $通讯作者：伦敦国王学院政治经济系，布什大厦，奥尔德威奇40号，伦敦WC2B 4PH，英国。\n电子邮箱：peter.john@kcl.ac.uk\n\n## 摘要\n\n正文\n";

        let authors = extract_academic_authors(markdown);

        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].name, "桑查扬·班纳吉");
        assert_eq!(authors[0].markers, vec!["ID"]);
        assert_eq!(authors[1].name, "彼得·约翰");
        assert!(authors[1].markers.iter().any(|marker| marker == "ID"));
        assert!(authors[1].markers.iter().any(|marker| marker == "*"));
        assert_eq!(authors[1].email.as_deref(), Some("peter.john@kcl.ac.uk"));
        assert!(authors[1].is_corresponding);
    }
}
