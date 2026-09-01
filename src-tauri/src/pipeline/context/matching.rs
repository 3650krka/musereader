use std::collections::BTreeSet;

pub(crate) fn collect_capitalized_phrases(line: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    for span in line.split(capitalized_phrase_span_boundary) {
        collect_capitalized_phrases_from_span(span, &mut phrases);
    }
    phrases
}

fn collect_capitalized_phrases_from_span<'a>(span: &'a str, phrases: &mut Vec<String>) {
    let mut current = Vec::<&'a str>::new();
    for token in span.split(capitalized_phrase_token_boundary) {
        if token.is_empty() {
            continue;
        }
        if is_capitalized_token(token) {
            current.push(token);
            continue;
        }
        flush_capitalized_phrase(&mut current, phrases);
    }
    flush_capitalized_phrase(&mut current, phrases);
}

fn flush_capitalized_phrase(current: &mut Vec<&str>, phrases: &mut Vec<String>) {
    if !current.is_empty() {
        phrases.push(current.join(" "));
        current.clear();
    }
}

fn capitalized_phrase_span_boundary(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | ';' | ':' | ',' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

fn capitalized_phrase_token_boundary(ch: char) -> bool {
    !(ch.is_ascii_alphabetic() || ch == '\'' || ch == '-')
}

fn is_capitalized_token(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && chars.all(|ch| ch.is_ascii_alphabetic() || ch == '\'' || ch == '-')
}

#[derive(Debug, Clone)]
pub(crate) struct SourceCandidateCache {
    fuzzy_ascii_candidates: Vec<AsciiMatchCandidate>,
    normalized_alias_candidates: BTreeSet<String>,
    normalized_structural_spans: Vec<String>,
}

#[derive(Debug, Clone)]
struct AsciiMatchCandidate {
    normalized: String,
    token_count: usize,
}

impl SourceCandidateCache {
    pub(crate) fn new(source: &str) -> Self {
        let fuzzy_ascii_candidates = collect_ascii_fuzzy_candidates(source);
        let normalized_alias_candidates =
            collect_ascii_alias_candidates(source).into_iter().collect();
        let normalized_structural_spans = source
            .lines()
            .flat_map(split_structural_ascii_spans)
            .map(|span| normalize_ascii_match_term(&span))
            .filter(|normalized| !normalized.is_empty())
            .collect();
        Self {
            fuzzy_ascii_candidates,
            normalized_alias_candidates,
            normalized_structural_spans,
        }
    }

    fn contains_normalized(&self, value: &str) -> bool {
        self.normalized_alias_candidates.contains(value)
    }

    fn contains_normalized_structural_span(&self, normalized_hint: &str) -> bool {
        if normalized_hint.is_empty() {
            return false;
        }
        self.normalized_structural_spans
            .iter()
            .filter(|normalized| normalized.len() >= normalized_hint.len())
            .any(|normalized| normalized.contains(normalized_hint))
    }

    pub(crate) fn normalized_keys(&self) -> &BTreeSet<String> {
        &self.normalized_alias_candidates
    }
}

pub(crate) fn source_mentions_hint(source: &str, hint: &str) -> bool {
    let cache = SourceCandidateCache::new(source);
    source_mentions_hint_cached(source, hint, &cache)
}

pub(crate) fn source_mentions_hint_cached(
    source: &str,
    hint: &str,
    cache: &SourceCandidateCache,
) -> bool {
    if hint.is_empty() {
        return false;
    }
    if !hint.is_ascii() {
        return source.contains(hint);
    }
    if contains_ascii_phrase_with_boundary(source, hint) {
        return true;
    }
    let normalized_hint = normalize_ascii_match_term(hint);
    if normalized_hint.len() < 4 {
        return false;
    }

    if cache.contains_normalized_structural_span(&normalized_hint) {
        return true;
    }
    if cache.contains_normalized(&normalized_hint) {
        return true;
    }
    if source_mentions_hint_via_fuzzy_candidates(hint, &normalized_hint, cache) {
        return true;
    }

    let possessive_hint = strip_leading_ascii_possessive(source, hint);
    if let Some(possessive_hint) = possessive_hint {
        // 结构信号：hint 的首 token 在 source 中以 "'s" 所有格出现（如 Tirouv's），
        // 说明 source 确实提及该实体，只是 hint 还带有 source 中未并列出现的部分
        // （如全名的姓）。此时按首 token 判定归属，而非要求整个 hint 被模糊命中。
        let normalized_first_token = possessive_hint;
        if normalized_first_token.chars().count() >= 4 {
            let first_token_possessive_present =
                cache.fuzzy_ascii_candidates.iter().any(|candidate| {
                    candidate.normalized.len() >= 4
                        && is_similar_ascii_term_match(
                            &normalized_first_token,
                            &normalized_first_token,
                            candidate.token_count,
                            &candidate.normalized,
                        )
                });
            if first_token_possessive_present {
                return true;
            }
        }
    }

    false
}

fn strip_leading_ascii_possessive(source: &str, hint: &str) -> Option<String> {
    let hint_start = hint.split_whitespace().next()?;
    if hint_start.len() < 2 {
        return None;
    }
    let possessive_marker = format!("{hint_start}'s");
    if !source.contains(&possessive_marker) {
        return None;
    }
    let normalized = normalize_ascii_match_term(hint_start);
    (normalized.chars().count() >= 4).then(|| normalized)
}

fn source_mentions_hint_via_fuzzy_candidates(
    raw_hint: &str,
    normalized_hint: &str,
    cache: &SourceCandidateCache,
) -> bool {
    if uses_title_match_policy(raw_hint) {
        return false;
    }

    cache.fuzzy_ascii_candidates.iter().any(|candidate| {
        is_similar_ascii_term_match(
            raw_hint,
            normalized_hint,
            candidate.token_count,
            &candidate.normalized,
        )
    })
}

fn split_structural_ascii_spans(line: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut current = String::new();

    for ch in line.chars() {
        if matches!(ch, '.' | '!' | '?' | ';' | '\r' | '\n') {
            push_structural_ascii_span(&mut spans, &mut current);
            continue;
        }
        current.push(ch);
    }

    push_structural_ascii_span(&mut spans, &mut current);
    spans
}

fn push_structural_ascii_span(spans: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() && trimmed.is_ascii() {
        spans.push(trimmed.to_string());
    }
    current.clear();
}

/// 近似匹配等级：区分「同一实体变体」与「不同人物」的可信度。
///
/// Sophia/Sophie 这类前缀重合的名字，既可能是同一人（拼写变体、OCR 误差、
/// 昵称/全称），也可能是不同人物。单一布尔阈值无法区分这两种情况，
/// 因此把匹配结果分级，不同级别对应不同的锁定/合并行为。
///
/// 注意：enum variant 声明顺序即 Ord 顺序（None < LowSimilarity < Contains < HighSimilarity < Exact），
/// `is_lockable` 判定「可自动锁定/合并」的阈值为 Contains 及以上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TermMatchLevel {
    /// L0：不匹配。
    None,
    /// L4：低相似度（仅前缀重合 0.6-0.8 或编辑距离 0.7-0.85，如 Sophia/Sophie）。
    /// 可能是同一人的拼写变体，也可能是不同人物——不自动锁定，需额外校验。
    LowSimilarity,
    /// L2：包含匹配（一方 normalized 包含另一方，如 Sophie vs Sophie Wentworth）。
    Contains,
    /// L3：高相似度（编辑距离≥0.85 或共享子序列≥0.9 或前缀≥0.8）。
    HighSimilarity,
    /// L1：精确匹配（normalized key 完全相同）。
    Exact,
}

impl TermMatchLevel {
    /// 是否达到「可自动锁定/合并」的阈值（L2 Contains 及以上）。
    pub(crate) fn is_lockable(self) -> bool {
        self >= TermMatchLevel::Contains
    }
}

/// 计算两个 source 表面的匹配等级（双向取较高者）。
pub(crate) fn term_match_level(left: &str, right: &str) -> TermMatchLevel {
    let normalized_left = normalize_ascii_match_term(left);
    let normalized_right = normalize_ascii_match_term(right);
    if normalized_left.is_empty() || normalized_right.is_empty() {
        return if left.trim() == right.trim() {
            TermMatchLevel::Exact
        } else {
            TermMatchLevel::None
        };
    }
    let forward = ascii_term_match_level(
        left,
        &normalized_left,
        ascii_token_count(right),
        &normalized_right,
    );
    let backward = ascii_term_match_level(
        right,
        &normalized_right,
        ascii_token_count(left),
        &normalized_left,
    );
    forward.max(backward)
}

/// 单向匹配等级（保留原 is_similar_ascii_term_match 的 token 数检查语义）。
fn ascii_term_match_level(
    raw_hint: &str,
    expected: &str,
    candidate_token_count: usize,
    candidate: &str,
) -> TermMatchLevel {
    let expected_token_count = ascii_token_count(raw_hint);
    if expected_token_count > 1 && candidate_token_count < expected_token_count {
        return TermMatchLevel::None;
    }
    if expected == candidate {
        return TermMatchLevel::Exact;
    }
    if expected.contains(candidate) || candidate.contains(expected) {
        return TermMatchLevel::Contains;
    }
    let expected_len = expected.chars().count();
    let candidate_len = candidate.chars().count();
    if expected_len < 4 || candidate_len < 4 {
        return TermMatchLevel::None;
    }
    // 短路重排：先算最便宜的 prefix_ratio（O(min_len)），很多候选免于 DP。
    let prefix_ratio = common_prefix_ratio(expected, candidate);
    let len_diff = expected_len.abs_diff(candidate_len);
    // 长度带早退：Levenshtein 在 |len_diff| / max_len > 1 - threshold 时不可能达标。
    let max_len = expected_len.max(candidate_len);
    if len_diff as f32 / max_len as f32 > 0.3 {
        return TermMatchLevel::None;
    }
    let similarity = normalized_similarity(expected, candidate);
    let sequence_ratio = shared_sequence_ratio(expected, candidate);
    // L3：高置信度同一实体（编辑距离很近 / 子序列几乎一致）。
    // 前缀重合单独不足以判定高置信度（Sophia/Sophie 前缀 0.83 但可能是不同人物）。
    if similarity >= 0.85 || sequence_ratio >= 0.9 {
        return TermMatchLevel::HighSimilarity;
    }
    // L4：低置信度，可能是同一人的拼写变体，也可能是不同人物（如 Sophia/Sophie）。
    if similarity >= 0.70 || prefix_ratio >= 0.6 || sequence_ratio >= 0.8 {
        return TermMatchLevel::LowSimilarity;
    }
    TermMatchLevel::None
}

pub(crate) fn source_terms_are_similar(left: &str, right: &str) -> bool {
    let normalized_left = normalize_ascii_match_term(left);
    let normalized_right = normalize_ascii_match_term(right);
    if normalized_left.is_empty() || normalized_right.is_empty() {
        return left.trim() == right.trim();
    }
    is_similar_ascii_term_match(
        left,
        &normalized_left,
        ascii_token_count(right),
        &normalized_right,
    )
}

fn contains_ascii_phrase_with_boundary(source: &str, hint: &str) -> bool {
    let mut start = 0usize;
    while let Some(found) = source[start..].find(hint) {
        let match_start = start + found;
        let match_end = match_start + hint.len();
        if is_ascii_phrase_boundary(source, match_start, match_end) {
            return true;
        }
        start = match_end;
    }
    false
}

fn is_ascii_phrase_boundary(source: &str, start: usize, end: usize) -> bool {
    let before_ok = source[..start]
        .chars()
        .next_back()
        .is_none_or(is_non_ascii_word_char_boundary);
    let after_ok = source[end..]
        .chars()
        .next()
        .is_none_or(is_non_ascii_word_char_boundary);
    before_ok && after_ok
}

fn is_non_ascii_word_char_boundary(ch: char) -> bool {
    !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn collect_ascii_fuzzy_candidates(source: &str) -> Vec<AsciiMatchCandidate> {
    let tokens = source
        .split(|ch: char| !(ch.is_ascii_alphabetic() || ch == '\'' || ch == '-'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let raw_ascii_tokens = source
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                ch.is_ascii_whitespace()
                    || matches!(ch, '(' | ')' | '[' | ']' | '{' | '}' | '"' | ',')
            })
        })
        .filter(|token| !token.is_empty() && token.is_ascii())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();

    for token in &tokens {
        let normalized = normalize_ascii_match_term(token);
        if normalized.len() >= 4 {
            candidates.push(AsciiMatchCandidate {
                normalized,
                token_count: 1,
            });
        }
    }

    for token in &raw_ascii_tokens {
        let normalized = normalize_ascii_match_term(token);
        if normalized.len() >= 4 {
            candidates.push(AsciiMatchCandidate {
                normalized,
                token_count: 1,
            });
        }
    }

    candidates.sort_by(|left, right| {
        left.normalized
            .cmp(&right.normalized)
            .then(left.token_count.cmp(&right.token_count))
    });
    candidates.dedup_by(|left, right| {
        left.normalized == right.normalized && left.token_count == right.token_count
    });
    candidates
}

fn collect_ascii_alias_candidates(source: &str) -> Vec<String> {
    let mut candidates = collect_ascii_fuzzy_candidates(source)
        .into_iter()
        .map(|candidate| candidate.normalized)
        .collect::<Vec<_>>();
    candidates.extend(
        source
            .lines()
            .flat_map(split_ascii_alias_segments)
            .filter_map(|segment| {
                let normalized =
                    normalize_ascii_match_term(&sanitize_ascii_alias_segment(&segment));
                (normalized.len() >= 4).then_some(normalized)
            }),
    );
    candidates.sort();
    candidates.dedup();
    candidates
}

fn split_ascii_alias_segments(line: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();

    for ch in line.chars() {
        if should_keep_ascii_alias_char(ch) {
            current.push(ch);
            continue;
        }
        push_ascii_alias_segment(&mut segments, &mut current);
    }

    push_ascii_alias_segment(&mut segments, &mut current);
    segments
}

fn should_keep_ascii_alias_char(ch: char) -> bool {
    ch.is_ascii()
        && !matches!(ch, '\r' | '\n')
        && !matches!(
            ch,
            '[' | ']' | '{' | '}' | '<' | '>' | '"' | '`' | '!' | '?'
        )
}

fn sanitize_ascii_alias_segment(segment: &str) -> String {
    segment
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_matches(|ch: char| matches!(ch, ':' | ';' | ',' | '.'))
        .to_string()
}

fn push_ascii_alias_segment(segments: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    current.clear();
}

pub(crate) fn normalize_resource_term_key(value: &str) -> String {
    let mut normalized = String::new();
    let mut current = String::new();

    let chars = value.chars().collect::<Vec<_>>();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
            continue;
        }
        flush_ascii_token(&mut normalized, &mut current);
        if let Some(symbol_name) = symbol_name_token(&chars, index, ch) {
            normalized.push_str(symbol_name);
        }
    }
    flush_ascii_token(&mut normalized, &mut current);
    normalized
}

fn normalize_ascii_match_term(value: &str) -> String {
    normalize_resource_term_key(value)
}

fn flush_ascii_token(output: &mut String, current: &mut String) {
    if !current.is_empty() {
        output.push_str(current);
        current.clear();
    }
}

fn symbol_name_token(chars: &[char], index: usize, ch: char) -> Option<&'static str> {
    match ch {
        '+' => Some("plus"),
        '-' => is_standalone_ascii_symbol(chars, index).then_some("minus"),
        '&' => Some("and"),
        '@' => Some("at"),
        '#' => Some("hash"),
        '%' => Some("percent"),
        '=' => Some("equals"),
        '/' | '\\' | '_' | '.' | ':' | ';' | ',' | '\'' | '"' | '(' | ')' | '[' | ']' | '{'
        | '}' | '!' | '?' | '*' | '^' | '~' | '`' | '|' | '<' | '>' => None,
        _ => None,
    }
}

fn is_standalone_ascii_symbol(chars: &[char], index: usize) -> bool {
    let before_is_word = index
        .checked_sub(1)
        .and_then(|position| chars.get(position))
        .is_some_and(|ch| ch.is_ascii_alphanumeric());
    let after_is_word = chars
        .get(index + 1)
        .is_some_and(|ch| ch.is_ascii_alphanumeric());
    !(before_is_word && after_is_word)
}

fn uses_title_match_policy(hint: &str) -> bool {
    let words = hint.split_whitespace().collect::<Vec<_>>();
    if words.len() < 2 {
        return false;
    }
    let first = words
        .first()
        .map(|value| value.trim_matches(|ch: char| !ch.is_ascii_alphabetic()))
        .unwrap_or_default();
    matches!(first, "The" | "A" | "An")
        || hint.contains(" of ")
        || hint.contains(" Of ")
        || hint.contains(" to ")
        || hint.contains(" To ")
        || hint.contains(':')
}

fn is_similar_ascii_term_match(
    raw_hint: &str,
    expected: &str,
    candidate_token_count: usize,
    candidate: &str,
) -> bool {
    let expected_token_count = ascii_token_count(raw_hint);
    if expected_token_count > 1 && candidate_token_count < expected_token_count {
        return false;
    }
    if expected == candidate {
        return true;
    }
    if expected.contains(candidate) || candidate.contains(expected) {
        return true;
    }
    let expected_len = expected.chars().count();
    let candidate_len = candidate.chars().count();
    if expected_len < 4 || candidate_len < 4 {
        return false;
    }
    let similarity = normalized_similarity(expected, candidate);
    similarity >= 0.70
        || common_prefix_ratio(expected, candidate) >= 0.6
        || shared_sequence_ratio(expected, candidate) >= 0.8
}

fn ascii_token_count(value: &str) -> usize {
    value
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .count()
}

pub(crate) fn normalized_similarity(left: &str, right: &str) -> f32 {
    let distance = levenshtein_distance(left, right);
    let max_len = left.chars().count().max(right.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (distance as f32 / max_len as f32)
}

fn common_prefix_ratio(left: &str, right: &str) -> f32 {
    let shared = left
        .chars()
        .zip(right.chars())
        .take_while(|(left_char, right_char)| left_char == right_char)
        .count();
    let longest = left.chars().count().max(right.chars().count());
    if longest == 0 {
        return 1.0;
    }
    shared as f32 / longest as f32
}

fn shared_sequence_ratio(left: &str, right: &str) -> f32 {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let shared = longest_common_subsequence_len(&left_chars, &right_chars);
    let shortest = left_chars.len().min(right_chars.len());
    if shortest == 0 {
        return 1.0;
    }
    shared as f32 / shortest as f32
}

fn longest_common_subsequence_len(left: &[char], right: &[char]) -> usize {
    let mut previous = vec![0usize; right.len() + 1];
    let mut current = vec![0usize; right.len() + 1];

    for left_char in left {
        for (index, right_char) in right.iter().enumerate() {
            current[index + 1] = if left_char == right_char {
                previous[index] + 1
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }

    previous[right.len()]
}

pub(crate) fn levenshtein_distance(left: &str, right: &str) -> usize {
    let right_len = right.chars().count();
    let mut costs = (0..=right_len).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut previous = costs[0];
        costs[0] = left_index + 1;
        for (right_index, right_char) in right.chars().enumerate() {
            let insertion = costs[right_index + 1] + 1;
            let deletion = costs[right_index] + 1;
            let substitution = previous + usize::from(left_char != right_char);
            previous = costs[right_index + 1];
            costs[right_index + 1] = insertion.min(deletion).min(substitution);
        }
    }
    costs[right_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalized_phrase_collection_does_not_cross_structural_punctuation() {
        let phrases = collect_capitalized_phrases(
            "labels (Table 1). All types were tested; William Brady, Ziv Epstein replied.",
        );

        assert!(phrases.contains(&"Table".to_string()));
        assert!(phrases.contains(&"All".to_string()));
        assert!(phrases.contains(&"William Brady".to_string()));
        assert!(phrases.contains(&"Ziv Epstein".to_string()));
        assert!(!phrases.contains(&"Table All".to_string()));
        assert!(!phrases.contains(&"William Brady Ziv Epstein".to_string()));
    }

    #[test]
    fn cached_matching_keeps_similarity_fallback_for_non_title_terms() {
        let source = "The chapter discusses deliberative tools in detail.";
        let cache = SourceCandidateCache::new(source);

        assert!(source_mentions_hint_cached(source, "deliberation", &cache));
    }

    #[test]
    fn title_like_terms_use_normalized_aliases_without_fuzzy_similarity() {
        let source = "## The End-of-the-Story";
        let cache = SourceCandidateCache::new(source);

        assert!(source_mentions_hint_cached(
            source,
            "The End of the Story",
            &cache
        ));
        assert!(!source_mentions_hint_cached(
            source,
            "The End of the Store",
            &cache
        ));
    }

    #[test]
    fn symbolic_terms_match_textual_variants() {
        let source = "The paper studies nudge+ and C++ interventions.";
        let cache = SourceCandidateCache::new(source);

        assert!(source_mentions_hint_cached(source, "nudge plus", &cache));
        assert!(source_mentions_hint_cached(source, "C plus plus", &cache));
    }

    #[test]
    fn symbolic_multiword_terms_match_structural_spans() {
        let source = "The nudge+ instrument stayed visible.";
        let cache = SourceCandidateCache::new(source);

        assert!(source_mentions_hint_cached(
            source,
            "nudge plus instrument",
            &cache
        ));
    }

    #[test]
    fn symbolic_terms_match_minus_variants() {
        let source = "The paper compares nudge- with loss-framed and risk@scale variants.";
        let cache = SourceCandidateCache::new(source);

        assert!(source_mentions_hint_cached(source, "nudge minus", &cache));
        assert!(source_mentions_hint_cached(source, "risk at scale", &cache));
    }

    #[test]
    fn cache_does_not_create_false_phrase_hits_from_window_expansion() {
        let source = "The paper compares nudge with a baseline intervention.";
        let cache = SourceCandidateCache::new(source);

        assert!(!source_mentions_hint_cached(
            source,
            "nudge baseline",
            &cache
        ));
    }

    #[test]
    fn normalize_resource_term_key_keeps_symbol_distinctions() {
        assert_eq!(normalize_resource_term_key("nudge+"), "nudgeplus");
        assert_eq!(normalize_resource_term_key("nudge-"), "nudgeminus");
        assert_eq!(normalize_resource_term_key("risk@scale"), "riskatscale");
        assert_eq!(normalize_resource_term_key("A/B"), "ab");
        assert_eq!(
            normalize_resource_term_key("The End-of-the-Story"),
            "theendofthestory"
        );
    }
}
