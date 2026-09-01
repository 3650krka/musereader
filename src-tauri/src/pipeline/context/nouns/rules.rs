use super::ProperNounEnforcement;
use std::collections::BTreeMap;

pub(super) fn is_noise_phrase(phrase: &str) -> bool {
    phrase.len() < 3
        || phrase.len() > 64
        || super::PROPER_NOUN_NOISE_WORDS.contains(&phrase)
        || is_low_value_function_phrase(phrase)
        || phrase.chars().all(|ch| ch.is_ascii_uppercase())
}

fn is_low_value_function_phrase(phrase: &str) -> bool {
    let tokens = phrase.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 2 {
        return false;
    }
    tokens.iter().all(|token| {
        let normalized = token.trim_matches(|ch: char| !ch.is_ascii_alphabetic());
        normalized.is_empty() || super::PROPER_NOUN_NOISE_WORDS.contains(&normalized)
    })
}

pub(super) fn canonicalize_proper_noun_phrase(raw: &str) -> String {
    const LEADING_NOISE: &[&str] = &["I", "Now", "And", "But", "Then", "The"];
    const TRAILING_NOISE: &[&str] = &[
        "And", "We", "His", "Her", "Its", "Their", "Before", "Indeed", "Finally",
    ];
    let mut tokens = raw.split_whitespace().collect::<Vec<_>>();
    while let Some(first) = tokens.first().copied() {
        if LEADING_NOISE.contains(&first) && tokens.len() > 1 {
            tokens.remove(0);
        } else {
            break;
        }
    }
    while let Some(last) = tokens.last().copied() {
        if (last.len() == 1 || TRAILING_NOISE.contains(&last)) && tokens.len() > 1 {
            tokens.pop();
        } else {
            break;
        }
    }
    tokens.join(" ")
}

pub(super) fn should_keep_proper_noun(
    phrase: &str,
    count: usize,
    structural_evidence_count: usize,
    article_type: &str,
    seed_map: &BTreeMap<&'static str, &'static str>,
) -> bool {
    if seed_map.contains_key(phrase) {
        return true;
    }
    if phrase.contains(' ') {
        return count >= 1;
    }
    if super::super::super::chunking::is_academic_article(article_type)
        && structural_evidence_count == 0
    {
        return false;
    }
    count >= 2 && phrase.len() >= 6
}

pub(super) fn proper_noun_enforcement(
    source: &str,
    target: Option<&str>,
    seed_map: &BTreeMap<&'static str, &'static str>,
) -> ProperNounEnforcement {
    if target.is_some_and(|value| is_contextual_fictional_derivative(source, value, seed_map)) {
        ProperNounEnforcement::Contextual
    } else {
        ProperNounEnforcement::Strict
    }
}

fn is_contextual_fictional_derivative(
    source: &str,
    target: &str,
    seed_map: &BTreeMap<&'static str, &'static str>,
) -> bool {
    has_people_or_group_suffix(target)
        && derived_fictional_root(source)
            .as_deref()
            .is_some_and(|root| seed_map.contains_key(root))
}

pub(super) fn derived_fictional_root(source: &str) -> Option<String> {
    if source.contains(char::is_whitespace) || source.len() < 6 {
        return None;
    }
    for suffix in ["ean", "ian", "an"] {
        if let Some(stem) = source.strip_suffix(suffix) {
            return Some(format!("{stem}a"));
        }
    }
    None
}

pub(super) fn has_people_or_group_suffix(target: &str) -> bool {
    let trimmed = target.trim();
    ["人", "族"].iter().any(|suffix| trimmed.ends_with(suffix))
}

pub(super) fn built_in_name_seed_map() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::new()
}
