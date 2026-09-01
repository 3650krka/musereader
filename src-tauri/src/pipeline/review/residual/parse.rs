#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedResidualRepair {
    pub(crate) repaired_text: Option<String>,
    pub(crate) confirmed_terms: Vec<crate::llm::ConfirmedTerm>,
}

pub(crate) fn parse_repaired_translation(review_response: &str) -> Option<String> {
    parse_residual_repair(review_response).repaired_text
}

pub(crate) fn parse_residual_repair(review_response: &str) -> ParsedResidualRepair {
    let Some(json) = extract_json_object(review_response) else {
        return ParsedResidualRepair::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return ParsedResidualRepair::default();
    };
    let Some(decision) = value.get("decision").and_then(serde_json::Value::as_str) else {
        return ParsedResidualRepair::default();
    };
    if !matches!(decision, "repair" | "patch" | "keep" | "correct") {
        return ParsedResidualRepair::default();
    }
    let repaired_text = value
        .get("patchedSentence")
        .or_else(|| value.get("patched_sentence"))
        .or_else(|| value.get("repairedText"))
        .or_else(|| value.get("repaired_text"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);

    let confirmed_terms = value
        .get("confirmedTerms")
        .or_else(|| value.get("confirmed_terms"))
        .cloned()
        .and_then(|terms| serde_json::from_value::<Vec<crate::llm::ConfirmedTerm>>(terms).ok())
        .unwrap_or_default();

    ParsedResidualRepair {
        repaired_text,
        confirmed_terms,
    }
}

fn extract_json_object(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if start >= end {
        return None;
    }
    Some(&trimmed[start..=end])
}
