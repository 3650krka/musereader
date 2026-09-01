use crate::llm::ConfirmedTerm;
use crate::pipeline::context::normalize_resource_term_key;
use crate::pipeline::ProperNounEnforcement;

pub(crate) fn sanitize_confirmed_term(
    term: &ConfirmedTerm,
    translated_text: &str,
    article_type: &str,
) -> Option<ConfirmedTerm> {
    let normalized = normalize_confirmed_term(term);
    if !should_accept_confirmed_term(&normalized) {
        return None;
    }
    if !is_term_type_allowed_for_article(&normalized, article_type) {
        return None;
    }
    if normalized.translation == normalized.source && !is_source_preservation_allowed(&normalized) {
        return None;
    }
    if normalized.has_untranslated_source_token() {
        return None;
    }
    if !translated_text.contains(&normalized.translation) {
        return None;
    }
    Some(normalized)
}

fn should_accept_confirmed_term(term: &ConfirmedTerm) -> bool {
    if !matches!(
        term.term_type.as_str(),
        "coinage" | "disputed" | "person" | "location" | "organization" | "title" | "technical"
    ) {
        return false;
    }
    term.confidence >= 85
}

fn normalize_confirmed_term(term: &ConfirmedTerm) -> ConfirmedTerm {
    let inferred_type = infer_term_type(&term.term_type, &term.usage_role);
    ConfirmedTerm {
        source: normalize_source_surface(&term.source),
        translation: term.translation.trim().to_string(),
        term_type: inferred_type.to_string(),
        confidence: term.confidence.min(100),
        usage_role: term.usage_role.trim().to_string(),
    }
}

fn normalize_source_surface(source: &str) -> String {
    collapse_hyphen_spacing(&source.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn collapse_hyphen_spacing(source: &str) -> String {
    source.replace("- ", "-").replace(" -", "-")
}

fn infer_term_type<'a>(term_type: &'a str, usage_role: &'a str) -> &'a str {
    match usage_role {
        "person_name" => "person",
        "place_name" => "location",
        "organization_name" => "organization",
        "book_title" => "title",
        "coinage_noun" | "coinage_modifier" | "people_noun" => "coinage",
        "technical_term" => {
            if term_type == "technical" {
                "technical"
            } else {
                term_type
            }
        }
        _ => term_type,
    }
}

fn is_term_type_allowed_for_article(term: &ConfirmedTerm, article_type: &str) -> bool {
    if term.term_type != "person" {
        return true;
    }
    crate::article_policy::ArticlePolicy::accepts_person_terms_for(article_type)
}

fn is_source_preservation_allowed(term: &ConfirmedTerm) -> bool {
    matches!(term.term_type.as_str(), "person" | "organization")
        && matches!(
            term.usage_role.as_str(),
            "person_name" | "organization_name"
        )
}

pub(super) fn normalize_term_key(source: &str) -> String {
    normalize_resource_term_key(source)
}

pub(super) fn enforcement_for_term_type(term_type: &str) -> ProperNounEnforcement {
    let _ = term_type;
    ProperNounEnforcement::Strict
}

pub(super) fn enforcement_term_type(enforcement: ProperNounEnforcement) -> &'static str {
    match enforcement {
        ProperNounEnforcement::Strict => "person",
        ProperNounEnforcement::Contextual => "coinage",
    }
}
