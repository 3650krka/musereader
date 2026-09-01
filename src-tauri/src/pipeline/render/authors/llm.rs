use super::AcademicAuthor;
use crate::error::AppError;
use crate::llm::SensenovaClient;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorMetadataResponse {
    authors: Vec<AuthorMetadataItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorMetadataItem {
    name: String,
    email: Option<String>,
    is_corresponding: Option<bool>,
    correspondence_note: Option<String>,
    affiliations: Option<Vec<String>>,
    markers: Option<Vec<String>>,
}

pub(super) async fn refine_academic_authors_with_llm(
    llm_client: &SensenovaClient,
    markdown: &str,
    first_page_footnotes: &[String],
    existing: &[AcademicAuthor],
) -> Result<Vec<AcademicAuthor>, AppError> {
    if existing.is_empty() {
        return Ok(Vec::new());
    }
    let snippet = extract_author_metadata_snippet(markdown);
    let footnote_context = collect_first_page_footnote_context(first_page_footnotes);
    if snippet.trim().is_empty() && footnote_context.trim().is_empty() {
        return Ok(existing.to_vec());
    }

    let prompt = build_author_metadata_prompt(&snippet, &footnote_context, existing);
    let response = llm_client
        .review_translation_issue(&prompt)
        .await
        .map_err(AppError::from_provider_error)?;
    let parsed = parse_author_metadata_response(&response)?;
    Ok(merge_refined_authors(existing, parsed.authors))
}

fn extract_author_metadata_snippet(markdown: &str) -> String {
    let mut lines = Vec::new();
    let mut seen_author_line = false;

    for line in markdown.lines().take(32) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if seen_author_line && !lines.is_empty() {
                break;
            }
            continue;
        }
        if !seen_author_line {
            lines.push(line);
            if looks_like_author_line(trimmed) {
                seen_author_line = true;
            }
            continue;
        }
        if trimmed.starts_with('#') || is_probable_body_line(trimmed) {
            break;
        }
        if is_author_metadata_line(trimmed) {
            lines.push(line);
            continue;
        }
        break;
    }

    lines.join("\n")
}

fn build_author_metadata_prompt(
    snippet: &str,
    footnote_context: &str,
    existing: &[AcademicAuthor],
) -> String {
    let existing_summary = existing
        .iter()
        .map(|author| {
            format!(
                "- name={}; markers={}; email={}; corresponding={}; affiliations={}",
                author.name,
                author.markers.join(","),
                author.email.clone().unwrap_or_default(),
                author.is_corresponding,
                author.affiliations.join(" | ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are extracting structured academic author metadata from translated Markdown.\n\
         Improve the existing extraction conservatively.\n\
         Do not invent authors. Do not rewrite names. Do not add HTML.\n\
         Keep the same author order as the existing extraction.\n\
         Use the first-page footnote context when it contains affiliations, equal-contribution notes, or corresponding-author details.\n\
         Do not treat abstract text, body text, figure captions, table notes, or discussion paragraphs as affiliations.\n\
         Fill only missing or obviously incomplete fields like email, corresponding-author flag, correspondence note, affiliations, and markers.\n\
         Return JSON only.\n\n\
         Existing extraction:\n{existing_summary}\n\n\
         Markdown snippet:\n{snippet}\n\n\
         First-page footnote context:\n{footnote_context}\n\n\
         JSON schema:\n\
         {{\"authors\":[{{\"name\":\"...\",\"email\":\"... or null\",\"isCorresponding\":true,\"correspondenceNote\":\"... or null\",\"affiliations\":[\"...\"],\"markers\":[\"1\",\"*\"]}}]}}"
    )
}

fn collect_first_page_footnote_context(first_page_footnotes: &[String]) -> String {
    first_page_footnotes
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_author_metadata_response(raw: &str) -> Result<AuthorMetadataResponse, AppError> {
    let trimmed = raw.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| AppError::internal("author metadata LLM response missing JSON object"))?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| AppError::internal("author metadata LLM response missing JSON end"))?;
    serde_json::from_str::<AuthorMetadataResponse>(&trimmed[start..=end]).map_err(|error| {
        AppError::internal(format!("author metadata LLM JSON parse failed: {error}"))
    })
}

fn merge_refined_authors(
    existing: &[AcademicAuthor],
    refined: Vec<AuthorMetadataItem>,
) -> Vec<AcademicAuthor> {
    existing
        .iter()
        .map(|author| {
            let candidate = refined
                .iter()
                .find(|item| item.name.trim() == author.name.trim());
            AcademicAuthor {
                name: author.name.clone(),
                email: author.email.clone().or_else(|| {
                    candidate
                        .and_then(|item| item.email.clone())
                        .filter(|v| !v.trim().is_empty())
                }),
                markers: if author.markers.is_empty() {
                    candidate
                        .and_then(|item| item.markers.clone())
                        .unwrap_or_default()
                } else {
                    author.markers.clone()
                },
                affiliations: if author.affiliations.is_empty() {
                    candidate
                        .and_then(|item| item.affiliations.clone())
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|item| looks_like_affiliation_value(item))
                        .collect()
                } else {
                    author.affiliations.clone()
                },
                is_corresponding: author.is_corresponding
                    || candidate
                        .and_then(|item| item.is_corresponding)
                        .unwrap_or(false),
                affiliation_refs: author.affiliation_refs.clone(),
                equal_contribution: author.equal_contribution.clone(),
                author_notes: author.author_notes.clone(),
                correspondence_note: author
                    .correspondence_note
                    .clone()
                    .or_else(|| candidate.and_then(|item| item.correspondence_note.clone())),
            }
        })
        .collect()
}

fn looks_like_author_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    (line.contains("$^{")
        || line.contains("$ ^{")
        || line.contains("\\textcircled")
        || line.contains(" ID")
        || line.contains("（ID）")
        || line.contains("(ID)")
        || line.contains('*'))
        && !lower.contains("corresponding author")
        && !lower.contains("department")
        && !lower.contains("university")
        && !line.contains('@')
}

fn is_author_metadata_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    line.contains('@')
        || line.contains("通讯作者")
        || line.contains("电子邮箱")
        || line.contains("大学")
        || line.contains("学院")
        || line.contains("研究所")
        || line.contains("系")
        || lower.contains("corresponding author")
        || lower.contains("correspondence to:")
        || lower.contains("department")
        || lower.contains("university")
        || lower.contains("college")
        || lower.contains("school")
        || lower.contains("institute")
}

fn is_probable_body_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower == "abstract" || line == "摘要" {
        return true;
    }
    line.chars().count() >= 80
        && (line.contains('。')
            || line.contains('.')
            || line.contains('？')
            || line.contains('?')
            || line.contains('！')
            || line.contains('!'))
}

fn looks_like_affiliation_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || is_probable_body_line(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    trimmed.contains("大学")
        || trimmed.contains("学院")
        || trimmed.contains("研究所")
        || trimmed.contains("系")
        || lower.contains("department")
        || lower.contains("university")
        || lower.contains("college")
        || lower.contains("school")
        || lower.contains("institute")
        || lower.contains("laboratory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_stops_before_abstract_body_without_heading() {
        let markdown = "# Title\n\nValentin Guigon $^{1,2}$, Marie Claire Villeval $^{2,3,4}$\n\n我们如何评估模糊新闻的真实性？元认知是否指导我们决定寻求更多信息？在受控实验中，参与者评估了模糊新闻的真实性。\n\n## 方法\n\n正文";

        let snippet = extract_author_metadata_snippet(markdown);

        assert!(snippet.contains("Valentin Guigon"));
        assert!(!snippet.contains("我们如何评估模糊新闻的真实性"));
    }

    #[test]
    fn rejects_body_paragraph_as_affiliation_value() {
        assert!(!looks_like_affiliation_value("我们如何评估模糊新闻的真实性？元认知是否指导我们决定寻求更多信息？在受控实验中，参与者评估了模糊新闻的真实性。"));
        assert!(looks_like_affiliation_value(
            "Department of Political Economy, King's College London, London, UK"
        ));
    }
}
