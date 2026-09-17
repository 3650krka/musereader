use super::ResidualEnglishTerm;
pub(crate) use crate::article_policy::ArticlePolicy;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct AsciiToken {
    text: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct ResidualAccumulator {
    count: usize,
    first_start: usize,
    first_end: usize,
}

pub fn detect_residual_english_terms(
    article_type: &str,
    source: &str,
    translated: &str,
) -> Vec<ResidualEnglishTerm> {
    let policy = ArticlePolicy::from_article_type(article_type);
    let story_world = ArticlePolicy::has_story_world(article_type);
    let masked_source = mask_non_residual_spans(source, policy);
    let masked_translated = mask_non_residual_spans(translated, policy);
    let source_terms = collect_ascii_word_tokens_with_spans(&masked_source)
        .into_iter()
        .map(|token| token.text.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut terms = BTreeMap::<String, ResidualAccumulator>::new();

    for token in collect_ascii_word_tokens_with_spans(&masked_translated) {
        if should_ignore_residual_english_token(&token.text, policy, story_world) {
            continue;
        }
        if !source_terms.contains(&token.text.to_ascii_lowercase()) {
            continue;
        }
        let key = token.text.to_ascii_lowercase();
        let entry = terms.entry(key).or_insert(ResidualAccumulator {
            count: 0,
            first_start: token.start,
            first_end: token.end,
        });
        entry.count = entry.count.saturating_add(1);
    }

    terms
        .into_iter()
        .map(|(term, item)| ResidualEnglishTerm {
            source_excerpt: excerpt_around_term(source, &term),
            translated_excerpt: excerpt_by_char_span(translated, item.first_start, item.first_end),
            term,
            count: item.count,
        })
        .collect()
}

fn mask_non_residual_spans(text: &str, policy: ArticlePolicy) -> String {
    let mut mask = vec![false; text.chars().count()];
    mask_common_non_residual_spans(text, &mut mask, policy);
    if masks_specialized_spans(policy) {
        mask_specialized_non_residual_spans(text, &mut mask);
    }
    apply_char_mask(text, &mask)
}

fn mask_common_non_residual_spans(text: &str, mask: &mut [bool], policy: ArticlePolicy) {
    mask_markdown_code_spans(text, mask);
    mask_markdown_link_destinations(text, mask);
    mask_regex_spans(text, mask, r"(?is)<!--.*?-->");
    mask_regex_spans(text, mask, r"(?is)<[^>]+>");
    mask_regex_spans(text, mask, r"https?://[^\s)\u{ff09}]+");
    mask_regex_spans(
        text,
        mask,
        r"(?i)\bdoi\s*:\s*\S+|\b10\.\d{4,9}/[-._;()/:A-Z0-9]+",
    );
    mask_regex_spans(
        text,
        mask,
        r"(?i)\bisbn(?:-1[03])?\s*:?\s*[0-9X][0-9X\-\s]{8,}",
    );
    mask_regex_spans(
        text,
        mask,
        r"(?i)\b(?:www\.)?[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z]{2,})(?:/[^\s)\u{ff09}]*)?",
    );
    mask_regex_spans(
        text,
        mask,
        r"(?i)\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b",
    );
    mask_regex_spans(text, mask, r"\[[0-9,\-\s]+\]");
    mask_parenthetical_spans(text, mask, '(', ')', policy);
    mask_parenthetical_spans(text, mask, '\u{ff08}', '\u{ff09}', policy);
    mask_year_author_spans(text, mask);
}

fn masks_specialized_spans(policy: ArticlePolicy) -> bool {
    policy.is_specialized_or_academic()
}

fn mask_specialized_non_residual_spans(text: &str, mask: &mut [bool]) {
    mask_reference_section(text, mask);
    mask_academic_name_reference_spans(text, mask);
    mask_academic_acknowledgement_name_spans(text, mask);
    mask_academic_type_label_name_spans(text, mask);
    mask_academic_named_entity_preservation_spans(text, mask);
    mask_regex_spans(text, mask, r"\b[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+\b");
    mask_regex_spans(
        text,
        mask,
        r"\b[A-Z][A-Za-z0-9.+#_-]*\s+\d+(?:\.\d+)+[A-Za-z]?\b",
    );
    mask_regex_spans(
        text,
        mask,
        r"(?i)\b(?:[A-Z]\s+)?(?:package|library)\s+[A-Za-z][A-Za-z0-9_.-]+\b",
    );
    mask_regex_spans(
        text,
        mask,
        r"(?i)\b[A-Za-z][A-Za-z0-9_.-]+\s+(?:package|library)\b",
    );
    mask_regex_spans(text, mask, r"\b[A-Z][A-Za-z0-9]*[A-Z][A-Za-z0-9]*\b");
    mask_regex_spans(
        text,
        mask,
        r"(?:^|[^A-Za-z])([A-Z][A-Za-z0-9]*[A-Z][A-Za-z0-9]*)",
    );
}

fn mask_academic_name_reference_spans(text: &str, mask: &mut [bool]) {
    mask_regex_spans(
        text,
        mask,
        r"\b[A-Z][A-Za-z']+(?:\s*(?:and|&)\s*[A-Z][A-Za-z']+)*\s*\[[0-9,\-\s]+\]",
    );
    mask_regex_spans(
        text,
        mask,
        r"\b[A-Z][A-Za-z']+(?:\s+[A-Z][A-Za-z']+){1,3}\s*[:\u{ff1a}]",
    );
    mask_regex_spans(
        text,
        mask,
        r"\b[A-Z][A-Za-z'’\-]+(?:\s*(?:&|and)\s*[A-Z][A-Za-z'’\-]+)*(?:\s+et\s+al\.)?\s*\((?:19|20)\d{2}[a-z]?\)",
    );
    mask_regex_spans(
        text,
        mask,
        r"\b[A-Z][A-Za-z'’\-]+(?:\s*(?:&|and)\s*[A-Z][A-Za-z'’\-]+)*(?:\s+et\s+al\.)?,\s*(?:19|20)\d{2}[a-z]?\b",
    );
    mask_regex_spans(
        text,
        mask,
        r"\b[A-Z][A-Za-z'’\-]+(?:'s)?\s+[A-Za-z][A-Za-z'’\-]+(?:-[A-Za-z'’\-]+)*",
    );
}

fn mask_academic_acknowledgement_name_spans(text: &str, mask: &mut [bool]) {
    let chars = text.chars().collect::<Vec<_>>();
    let mut char_index = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if looks_like_acknowledgement_line(trimmed) {
            mask_name_like_spans_in_line(trimmed, mask, char_index);
        }
        char_index = char_index.saturating_add(line.chars().count());
        if char_index >= chars.len() {
            break;
        }
    }
}

fn looks_like_acknowledgement_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("we thank")
        || lower.contains("we are grateful to")
        || lower.contains("authors thank")
        || lower.contains("earlier draft of this article")
        || lower.contains("for discussant comments")
        || lower.contains("for participating in")
        || lower.contains("seminar on")
        || (lower.contains("funding") && lower.contains("from"))
        || (lower.contains("grant") && lower.contains("from"))
        || lower.contains("conflict of interest")
}

fn mask_name_like_spans_in_line(line: &str, mask: &mut [bool], line_char_start: usize) {
    let regex =
        Regex::new(r"\b[A-Z][A-Za-z'\-]+(?:\s+[A-Z][A-Za-z'\-]+){0,3}\b").expect("ack name regex");
    for matched in regex.find_iter(line) {
        let start = line_char_start + byte_to_char_index(line, matched.start());
        let end = line_char_start + byte_to_char_index(line, matched.end());
        mask_char_range(mask, start, end);
    }
}

fn mask_academic_type_label_name_spans(text: &str, mask: &mut [bool]) {
    mask_regex_spans(text, mask, r"\b[A-Z][A-Za-z'’\-]+\s*\(type\s*[12]\)");
}

fn mask_academic_named_entity_preservation_spans(text: &str, mask: &mut [bool]) {
    mask_regex_spans(
        text,
        mask,
        r"\b(?:Take|consider|called|like|such as)\s+([A-Z][A-Za-z0-9'’\-]+)\b",
    );
}

fn mask_markdown_code_spans(text: &str, mask: &mut [bool]) {
    mask_regex_spans(text, mask, r"(?s)```.*?```");
    mask_regex_spans(text, mask, r"`[^`\n]+`");
}

fn mask_markdown_link_destinations(text: &str, mask: &mut [bool]) {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index + 1 < chars.len() {
        if chars[index] != ']' || chars[index + 1] != '(' {
            index += 1;
            continue;
        }
        let Some(end) = find_next_char(&chars, index + 2, ')') else {
            break;
        };
        mask_char_range(mask, index + 1, end + 1);
        index = end + 1;
    }
}

fn mask_parenthetical_spans(
    text: &str,
    mask: &mut [bool],
    open: char,
    close: char,
    policy: ArticlePolicy,
) {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != open {
            index += 1;
            continue;
        }
        let Some(end) = find_next_char(&chars, index + 1, close) else {
            break;
        };
        let content = chars_between(text, index + 1, end);
        if should_mask_parenthetical_content(&content, policy) {
            mask_char_range(mask, index, end + 1);
        }
        index = end + 1;
    }
}

fn should_mask_parenthetical_content(content: &str, policy: ArticlePolicy) -> bool {
    let trimmed = content.trim();
    !trimmed.is_empty()
        && (trimmed.contains("http")
            || trimmed.contains("doi")
            || looks_like_year_citation(trimmed)
            || (masks_specialized_spans(policy)
                && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())))
}

fn mask_year_author_spans(text: &str, mask: &mut [bool]) {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '(' && chars[index] != '\u{ff08}' {
            index += 1;
            continue;
        }
        let close = if chars[index] == '(' { ')' } else { '\u{ff09}' };
        let Some(end) = find_next_char(&chars, index + 1, close) else {
            break;
        };
        let content = chars_between(text, index + 1, end);
        if looks_like_year_citation(&content) {
            let start = find_author_span_start(&chars, index);
            mask_char_range(mask, start, end + 1);
        }
        index = end + 1;
    }
}

fn looks_like_year_citation(content: &str) -> bool {
    let trimmed = content.trim();
    Regex::new(r"(?i)\b(?:19|20)\d{2}[a-z]?\b")
        .expect("year citation regex")
        .is_match(trimmed)
}

fn find_author_span_start(chars: &[char], paren_start: usize) -> usize {
    let Some(start) = author_phrase_suffix_start(chars, paren_start) else {
        return paren_start;
    };
    if start < paren_start {
        start
    } else {
        paren_start
    }
}

fn author_phrase_suffix_start(chars: &[char], end: usize) -> Option<usize> {
    let mut cursor = end;
    let mut start = end;
    let mut saw_author_token = false;
    loop {
        while cursor > 0 && chars[cursor - 1].is_whitespace() {
            cursor -= 1;
        }
        if cursor == 0 {
            break;
        }
        let token_end = cursor;
        while cursor > 0 && is_author_token_char(chars[cursor - 1]) {
            cursor -= 1;
        }
        if cursor == token_end {
            break;
        }
        let token = chars[cursor..token_end]
            .iter()
            .collect::<String>()
            .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
            .to_string();
        if token.is_empty() {
            break;
        }
        if token
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        {
            saw_author_token = true;
            start = cursor;
            continue;
        }
        if is_author_connector_token(&token) {
            start = cursor;
            continue;
        }
        break;
    }
    saw_author_token.then_some(start)
}

fn is_author_token_char(ch: char) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, '\'' | '-' | '&' | '.')
}

fn is_author_connector_token(token: &str) -> bool {
    matches!(token.to_ascii_lowercase().as_str(), "and" | "et" | "al")
}

fn mask_reference_section(text: &str, mask: &mut [bool]) {
    let mut byte_index = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r').trim();
        if is_reference_heading_line(trimmed) {
            let start = byte_to_char_index(text, byte_index);
            mask_char_range(mask, start, text.chars().count());
            return;
        }
        byte_index = byte_index.saturating_add(line.len());
    }
}

fn is_reference_heading_line(line: &str) -> bool {
    let normalized = line
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "references" | "bibliography" | "works cited" | "literature cited"
    )
}

fn mask_regex_spans(text: &str, mask: &mut [bool], pattern: &str) {
    let regex = Regex::new(pattern).expect("regex should compile");
    for matched in regex.find_iter(text) {
        mask_char_range(
            mask,
            byte_to_char_index(text, matched.start()),
            byte_to_char_index(text, matched.end()),
        );
    }
}

fn mask_char_range(mask: &mut [bool], start: usize, end: usize) {
    for index in start.min(mask.len())..end.min(mask.len()) {
        mask[index] = true;
    }
}

fn apply_char_mask(text: &str, mask: &[bool]) -> String {
    text.chars()
        .enumerate()
        .map(|(index, ch)| {
            if mask.get(index).copied().unwrap_or(false) {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn collect_ascii_word_tokens_with_spans(text: &str) -> Vec<AsciiToken> {
    let mut tokens = Vec::new();
    let chars = text.chars().collect::<Vec<_>>();
    let mut start = None;
    for (index, ch) in chars.iter().enumerate() {
        if ch.is_ascii_alphabetic() {
            start.get_or_insert(index);
            continue;
        }
        if let Some(word_start) = start.take() {
            push_ascii_word_token(&chars, word_start, index, &mut tokens);
        }
    }
    if let Some(word_start) = start {
        push_ascii_word_token(&chars, word_start, chars.len(), &mut tokens);
    }
    tokens
}

fn push_ascii_word_token(chars: &[char], start: usize, end: usize, tokens: &mut Vec<AsciiToken>) {
    let token = chars[start..end].iter().collect::<String>();
    if token.len() < 2 {
        return;
    }
    tokens.push(AsciiToken {
        text: token,
        start,
        end,
    });
}

fn should_ignore_residual_english_token(
    token: &str,
    policy: ArticlePolicy,
    story_world: bool,
) -> bool {
    let lower = token.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "a" | "an"
            | "about"
            | "after"
            | "again"
            | "against"
            | "all"
            | "also"
            | "although"
            | "am"
            | "and"
            | "any"
            | "anyone"
            | "as"
            | "at"
            | "be"
            | "because"
            | "been"
            | "being"
            | "before"
            | "both"
            | "but"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "down"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "he"
            | "her"
            | "here"
            | "hers"
            | "him"
            | "his"
            | "how"
            | "however"
            | "i"
            | "if"
            | "in"
            | "is"
            | "it"
            | "its"
            | "may"
            | "me"
            | "might"
            | "more"
            | "most"
            | "must"
            | "my"
            | "no"
            | "not"
            | "of"
            | "on"
            | "only"
            | "onto"
            | "or"
            | "our"
            | "out"
            | "over"
            | "shall"
            | "she"
            | "should"
            | "so"
            | "some"
            | "such"
            | "than"
            | "that"
            | "the"
            | "their"
            | "them"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "though"
            | "to"
            | "under"
            | "up"
            | "very"
            | "vs"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "who"
            | "whom"
            | "whose"
            | "why"
            | "will"
            | "with"
            | "without"
            | "would"
            | "you"
            | "your"
    ) {
        return true;
    }
    if token.chars().all(|ch| ch.is_ascii_uppercase()) {
        return true;
    }
    // 故事世界文本（fiction/children）：专名强制音译，TitleCase 不再豁免——
    // Marlin 这类残留必须送审修复；blog/news/general 仍允许保留现实品牌名。
    if story_world {
        return false;
    }
    policy.is_narrative() && looks_like_preserved_titlecase_token(token)
}

fn looks_like_preserved_titlecase_token(token: &str) -> bool {
    // 过滤句首普通词（Then/This/When/After 等），它们不是专名，是漏翻。
    // 专名通常 ≥4 字符或含非常见前缀；句首普通词短且高频。
    if token.chars().count() < 4 {
        return false;
    }
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase() && chars.any(|ch| ch.is_ascii_lowercase())
}

fn excerpt_around_term(text: &str, term: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let Some(start) = lower.find(term) else {
        return text.chars().take(160).collect();
    };
    let start = byte_to_char_index(text, start);
    excerpt_by_char_span(text, start.saturating_sub(32), start.saturating_add(96))
}

fn excerpt_by_char_span(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect::<String>()
        .trim()
        .to_string()
}

fn chars_between(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn byte_to_char_index(text: &str, byte_index: usize) -> usize {
    text[..byte_index.min(text.len())].chars().count()
}

fn find_next_char(chars: &[char], start: usize, needle: char) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, ch)| (*ch == needle).then_some(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn academic_residual_detection_ignores_versioned_tool_names() {
        let source = "Analyses were performed on MATLAB R2020b, R 3.4.1, and Python 3.11.5.";
        let translated = "分析在 MATLAB R2020b、R 3.4.1 和 Python 3.11.5 上进行。";

        let terms = detect_residual_english_terms("academic", source, translated);

        assert!(
            terms.iter().all(|term| term.term != "python"),
            "terms={terms:?}"
        );
    }

    #[test]
    fn academic_residual_detection_ignores_mixed_case_standards_next_to_cjk() {
        let source = "CRediT author statement: Jane Doe: Conceptualization.";
        let translated = "CRediT作者声明：Jane Doe：概念化。";

        let terms = detect_residual_english_terms("academic", source, translated);

        assert!(
            terms.iter().all(|term| term.term != "credit"),
            "terms={terms:?}"
        );
    }

    #[test]
    fn academic_residual_detection_requires_exact_source_token_match() {
        let source = "We do not claim that the article covers the whole field.";
        let translated = "我们并不声称覆盖整个领域。感谢Deb Ain的编辑。";

        let terms = detect_residual_english_terms("academic", source, translated);

        assert!(
            terms.iter().all(|term| term.term != "ain"),
            "terms={terms:?}"
        );
    }

    #[test]
    fn academic_residual_detection_ignores_package_names_and_connector_names() {
        let source = "The rows used the R package reactable [80]. The website used the workflow package [81]. Sander Linden contributed.";
        let translated =
            "这些行使用R包reactable [80]。网站使用 workflow 包 [81]。Sander Linden参与。";

        let terms = detect_residual_english_terms("academic", source, translated);

        assert!(
            terms
                .iter()
                .all(|term| !matches!(term.term.as_str(), "reactable" | "workflow")),
            "terms={terms:?}"
        );
    }

    #[test]
    fn narrative_residual_detection_ignores_book_metadata_noise() {
        let source = "ISBN: 978-1-59780-028-0. See www.eldritchdark.com. HPL wrote a note.";
        let translated = "ISBN: 978-1-59780-028-0。见 www.eldritchdark.com。HPL 写了一条注释。";

        let terms = detect_residual_english_terms("fiction", source, translated);

        assert!(terms.is_empty(), "terms={terms:?}");
    }

    #[test]
    fn narrative_residual_detection_keeps_lowercase_untranslated_words() {
        let source = "A faint rattling followed the hateful deliberation of the looming head.";
        let translated =
            "随后传来 rattling 声，头部带着可恨的 deliberation 和 looming 的姿态逼近。";

        let terms = detect_residual_english_terms("fiction", source, translated);
        let detected = terms
            .iter()
            .map(|term| term.term.as_str())
            .collect::<Vec<_>>();

        assert!(detected.contains(&"rattling"), "terms={terms:?}");
        assert!(detected.contains(&"deliberation"), "terms={terms:?}");
        assert!(detected.contains(&"looming"), "terms={terms:?}");
    }

    #[test]
    fn story_world_flags_titlecase_brand_for_forced_transliteration() {
        // 回归：小说/儿童文本专名强制音译，TitleCase 品牌名不再豁免
        // （001 事故：Marlin 步枪的 Marlin 被 looks_like_preserved_titlecase_token 放过）
        let source = "In a corner stood my Marlin rifle, with ten cartridges in the magazine.";
        let translated = "客厅角落里放着我的 Marlin 步枪，弹匧里有十发子弹。";

        let terms = detect_residual_english_terms("fiction", source, translated);
        let detected = terms
            .iter()
            .map(|term| term.term.as_str())
            .collect::<Vec<_>>();

        assert!(detected.contains(&"marlin"), "terms={terms:?}");
    }

    #[test]
    fn non_story_world_narrative_still_allows_titlecase_proper_nouns() {
        // 对照：blog/news/general 不是故事世界，仍允许保留 TitleCase 专名（如 iPhone）
        let source = "In a corner stood my Marlin rifle, with ten cartridges in the magazine.";
        let translated = "客厅角落里放着我的 Marlin 步枪，弹匧里有十发子弹。";

        let terms = detect_residual_english_terms("blog", source, translated);
        let detected = terms
            .iter()
            .map(|term| term.term.as_str())
            .collect::<Vec<_>>();

        assert!(!detected.contains(&"marlin"), "terms={terms:?}");
    }
}
