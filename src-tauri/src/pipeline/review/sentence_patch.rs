use crate::translation_validation::{ResidualEnglishTerm, SentencePatchTarget};

pub(crate) fn locate_patch_target(
    source_text: &str,
    translated_text: &str,
    term: &ResidualEnglishTerm,
) -> Option<SentencePatchTarget> {
    let source_sentence = find_sentence_containing(source_text, &term.source_excerpt)
        .or_else(|| find_sentence_containing(source_text, &term.term))?;
    let translated_sentence = find_sentence_containing(translated_text, &term.translated_excerpt)
        .or_else(|| find_sentence_containing(translated_text, &term.term))?;

    Some(SentencePatchTarget {
        source_context: build_sentence_window(source_text, &source_sentence),
        translated_context: build_sentence_window(translated_text, &translated_sentence),
        source_sentence,
        translated_sentence,
    })
}

pub(crate) fn locate_term_patch_target(
    source_text: &str,
    translated_text: &str,
    source_term: &str,
    target_term: &str,
) -> Option<SentencePatchTarget> {
    let source_sentences = split_sentences(source_text);
    let translated_sentences = split_sentences(translated_text);
    let source_index = source_sentences
        .iter()
        .position(|sentence| sentence.contains(source_term))?;
    let source_sentence = source_sentences.get(source_index)?.clone();

    // 句级索引对齐不可靠：源文按 [.!?\n] 切分、译文按 [。！？\n] 切分，
    // 句数不同（如 src 54 vs zh 52）导致索引错位（实测 chunk 6：playgroup 在
    // src[9]，但 zh[9] 是 littleness 句，LLM 被要求修补错误的句子，把
    // 「这个词的渺小与天真」替换成了「我们是在游戏班认识的」）。
    // 策略：先在译文中找「含该 term 译文的句子」（如 playgroup→亲子游戏组），
    // 找到则用它作为修补目标；找不到才退回索引对齐（宁缺勿滥）。
    let translated_index = if !target_term.trim().is_empty() {
        translated_sentences
            .iter()
            .position(|sentence| sentence.contains(target_term))
    } else {
        None
    };
    let translated_index = translated_index.unwrap_or(source_index);
    let translated_sentence = translated_sentences
        .get(translated_index)
        .cloned()
        .or_else(|| translated_sentences.last().cloned())?;

    Some(SentencePatchTarget {
        source_context: build_window_from_sentences(&source_sentences, source_index),
        translated_context: build_window_from_sentences(
            &translated_sentences,
            translated_index.min(translated_sentences.len().saturating_sub(1)),
        ),
        source_sentence,
        translated_sentence,
    })
}

pub(crate) fn replace_sentence_once(
    original: &str,
    current_sentence: &str,
    patched_sentence: &str,
) -> Option<String> {
    if current_sentence.is_empty() || current_sentence == patched_sentence {
        return Some(original.to_string());
    }
    let start = original.find(current_sentence)?;
    let end = start + current_sentence.len();
    let mut output = String::with_capacity(original.len() + patched_sentence.len());
    output.push_str(&original[..start]);
    output.push_str(patched_sentence);
    output.push_str(&original[end..]);
    Some(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceLikePatchResidual {
    pub(crate) term: String,
    pub(crate) translated_excerpt: String,
}

pub(crate) fn find_source_like_patch_residual(
    translated: &str,
    source_term: &str,
    canonical_target: &str,
) -> Option<SourceLikePatchResidual> {
    if canonical_target.trim().is_empty() || translated.contains(canonical_target) {
        return None;
    }
    let span = find_exact_source_term_span(translated, source_term)
        .or_else(|| find_mixed_source_target_span(translated, source_term, canonical_target))?;
    Some(SourceLikePatchResidual {
        term: span.term,
        translated_excerpt: translated[span.start..span.end].trim().to_string(),
    })
}

pub(crate) fn replace_source_like_phrase(
    sentence: &str,
    source_term: &str,
    canonical_target: &str,
) -> String {
    if canonical_target.trim().is_empty() {
        return sentence.to_string();
    }
    if sentence.contains(source_term) {
        return sentence.replace(source_term, canonical_target);
    }
    if let Some(span) = find_mixed_source_target_span(sentence, source_term, canonical_target) {
        return replace_byte_span(sentence, span.start, span.end, canonical_target);
    }
    if let Some(span) = find_first_source_token_span(sentence, source_term) {
        return replace_byte_span(sentence, span.start, span.end, canonical_target);
    }
    sentence.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceLikePatchSpan {
    start: usize,
    end: usize,
    term: String,
}

fn find_mixed_source_target_span(
    translated: &str,
    source_term: &str,
    canonical_target: &str,
) -> Option<SourceLikePatchSpan> {
    let target_spans = target_fragment_spans(translated, canonical_target);
    if target_spans.is_empty() {
        return None;
    }

    let mut best = None::<(usize, SourceLikePatchSpan)>;
    for token in source_ascii_tokens(source_term) {
        if token.len() < 4 {
            continue;
        }
        for token_span in find_ascii_token_spans(translated, &token) {
            for target_span in &target_spans {
                let distance = byte_span_distance(token_span, *target_span);
                if distance > 48 {
                    continue;
                }
                let start = token_span.0.min(target_span.0);
                let end = token_span.1.max(target_span.1);
                let candidate = SourceLikePatchSpan {
                    start,
                    end,
                    term: token.clone(),
                };
                if best
                    .as_ref()
                    .is_none_or(|(best_distance, _)| distance < *best_distance)
                {
                    best = Some((distance, candidate));
                }
            }
        }
    }
    best.map(|(_, span)| trim_ascii_joiners(translated, span))
}

fn find_exact_source_term_span(text: &str, source_term: &str) -> Option<SourceLikePatchSpan> {
    let source_term = source_term.trim();
    if source_term.len() < 4 || !source_term.is_ascii() {
        return None;
    }
    let first_token = source_ascii_tokens(source_term).into_iter().next()?;
    for (start, _) in text.match_indices(source_term) {
        let end = start + source_term.len();
        if is_ascii_boundary_match(text, start, end) {
            return Some(SourceLikePatchSpan {
                start,
                end,
                term: first_token,
            });
        }
    }
    None
}

fn source_ascii_tokens(source_term: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in source_term.chars() {
        if ch.is_ascii_alphabetic() {
            current.push(ch);
            continue;
        }
        if current.len() >= 2 {
            tokens.push(current.clone());
        }
        current.clear();
    }
    if current.len() >= 2 {
        tokens.push(current);
    }
    tokens.sort_by_key(|token| std::cmp::Reverse(token.len()));
    tokens.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    tokens
}

fn find_first_source_token_span(text: &str, source_term: &str) -> Option<SourceLikePatchSpan> {
    for token in source_ascii_tokens(source_term) {
        for (start, end) in find_ascii_token_spans(text, &token) {
            return Some(SourceLikePatchSpan {
                start,
                end,
                term: token,
            });
        }
    }
    None
}

fn find_ascii_token_spans(text: &str, token: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (start, _) in text.match_indices(token) {
        let end = start + token.len();
        if is_ascii_boundary_match(text, start, end) {
            spans.push((start, end));
        }
    }
    spans
}

fn target_fragment_spans(text: &str, canonical_target: &str) -> Vec<(usize, usize)> {
    let chars = canonical_target.chars().collect::<Vec<_>>();
    let max_len = chars.len().min(8);
    let mut spans = Vec::new();
    for len in (2..=max_len).rev() {
        for start in 0..=chars.len().saturating_sub(len) {
            let fragment = chars[start..start + len].iter().collect::<String>();
            if !fragment.chars().all(is_cjk_unified) {
                continue;
            }
            for (byte_start, _) in text.match_indices(&fragment) {
                spans.push((byte_start, byte_start + fragment.len()));
            }
        }
        if !spans.is_empty() {
            break;
        }
    }
    spans
}

fn is_cjk_unified(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

fn byte_span_distance(left: (usize, usize), right: (usize, usize)) -> usize {
    if left.1 <= right.0 {
        right.0.saturating_sub(left.1)
    } else if right.1 <= left.0 {
        left.0.saturating_sub(right.1)
    } else {
        0
    }
}

fn trim_ascii_joiners(text: &str, span: SourceLikePatchSpan) -> SourceLikePatchSpan {
    let mut start = span.start;
    let mut end = span.end;
    while start > 0 {
        let Some((prev_start, ch)) = text[..start].char_indices().next_back() else {
            break;
        };
        if !matches!(ch, ' ' | '\t' | '\u{00a0}') {
            break;
        }
        start = prev_start;
    }
    while end < text.len() {
        let Some(ch) = text[end..].chars().next() else {
            break;
        };
        if !matches!(ch, ' ' | '\t' | '\u{00a0}') {
            break;
        }
        end += ch.len_utf8();
    }
    SourceLikePatchSpan { start, end, ..span }
}

fn replace_byte_span(text: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut output = String::with_capacity(text.len() + replacement.len());
    output.push_str(&text[..start]);
    output.push_str(replacement);
    output.push_str(&text[end..]);
    output
}

fn is_ascii_boundary_match(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_ascii_identifier_char) && !after.is_some_and(is_ascii_identifier_char)
}

fn is_ascii_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

fn build_sentence_window(text: &str, sentence: &str) -> String {
    let sentences = split_sentences(text);
    let Some(index) = sentences.iter().position(|item| item == sentence) else {
        return sentence.to_string();
    };
    build_window_from_sentences(&sentences, index)
}

fn build_window_from_sentences(sentences: &[String], index: usize) -> String {
    let start = index.saturating_sub(1);
    let end = (index + 2).min(sentences.len());
    sentences[start..end].join(" ")
}

fn find_sentence_containing(text: &str, needle: &str) -> Option<String> {
    let normalized = needle.trim();
    if normalized.is_empty() {
        return None;
    }
    split_sentences(text)
        .into_iter()
        .find(|sentence| sentence.contains(normalized))
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '。' | '！' | '？' | '\n') {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            current.clear();
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_sentence_window_for_residual_term() {
        let term = ResidualEnglishTerm {
            term: "deliberation".to_string(),
            count: 1,
            source_excerpt: "heuristics and deliberation.".to_string(),
            translated_excerpt: "启发式与 deliberation 相结合".to_string(),
        };

        let target = locate_patch_target(
            "First sentence. Nudge plus builds on work combining heuristics and deliberation. Third sentence.",
            "第一句。助推+建立在将启发式与 deliberation 相结合的研究之上。第三句。",
            &term,
        )
        .expect("target should exist");

        assert!(target
            .source_sentence
            .contains("heuristics and deliberation"));
        assert!(target.translated_sentence.contains("deliberation"));
    }

    #[test]
    fn locates_policy_article_residual_sentence_with_deliberation() {
        let terms = crate::translation_validation::detect_residual_english_terms(
            "academic",
            "This mechanism operates as an information signal as the plus provides additional knowledge that induces deliberation, much like conventional informational interventions.",
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{4F5C}\u{4E3A}\u{201C}+\u{201D}\u{7684}\u{4FE1}\u{606F}\u{4FE1}\u{53F7}\u{63D0}\u{4F9B}\u{4E86}\u{5F15}\u{53D1} deliberation \u{7684}\u{989D}\u{5916}\u{77E5}\u{8BC6}\u{FF0C}\u{5C31}\u{50CF}\u{5E38}\u{89C4}\u{4FE1}\u{606F}\u{5E72}\u{9884}\u{4E00}\u{6837}\u{3002}",
        );
        let term = terms
            .iter()
            .find(|term| term.term == "deliberation")
            .expect("deliberation should be detected as residual English");

        let target = locate_patch_target(
            "This mechanism operates as an information signal as the plus provides additional knowledge that induces deliberation, much like conventional informational interventions.",
            "\u{8FD9}\u{4E00}\u{673A}\u{5236}\u{4F5C}\u{4E3A}\u{201C}+\u{201D}\u{7684}\u{4FE1}\u{606F}\u{4FE1}\u{53F7}\u{63D0}\u{4F9B}\u{4E86}\u{5F15}\u{53D1} deliberation \u{7684}\u{989D}\u{5916}\u{77E5}\u{8BC6}\u{FF0C}\u{5C31}\u{50CF}\u{5E38}\u{89C4}\u{4FE1}\u{606F}\u{5E72}\u{9884}\u{4E00}\u{6837}\u{3002}",
            term,
        )
        .expect("target should exist");

        assert!(target.source_sentence.contains("induces deliberation"));
        assert!(target.translated_sentence.contains("deliberation"));
    }

    #[test]
    fn replaces_only_one_sentence() {
        let original = "第一句。第二句包含 deliberation。第三句。";
        let patched = replace_sentence_once(
            original,
            "第二句包含 deliberation。",
            "第二句包含深思熟虑。",
        )
        .expect("replacement should succeed");

        assert_eq!(patched, "第一句。第二句包含深思熟虑。第三句。");
    }

    #[test]
    fn locates_term_patch_target_by_sentence_order() {
        let target = locate_term_patch_target(
            "First sentence. Nudge plus preserves autonomy. Third sentence.",
            "第一句。助推升级维护自主性。第三句。",
            "Nudge plus",
            "助推升级",
        )
        .expect("target should exist");

        assert_eq!(target.source_sentence, "Nudge plus preserves autonomy.");
        assert_eq!(target.translated_sentence, "助推升级维护自主性。");
    }

    #[test]
    fn locates_term_patch_target_by_target_term_not_index() {
        // 句级索引错位防护（chunk 6 实测灾难）：源文 54 句、译文 52 句，
        // playgroup 在 src[9] 但 zh[9] 是 littleness 句。
        // 用 target_term（亲子游戏组）定位译文句，而非索引。
        let target = locate_term_patch_target(
            "“We met…” It comes out as a croak. I clear my throat, start again. “We met at the playgroup.”\n\nThe littleness, the innocence of the word almost makes me laugh out loud.",
            "“我们是在……”我的声音沙哑。我清了清嗓子，重新开口：“我们是在亲子游戏组认识的。”\n\n这个词的渺小与天真，几乎让我笑出声来。",
            "playgroup",
            "亲子游戏组",
        )
        .expect("target should exist");

        assert!(target.source_sentence.contains("playgroup"));
        assert!(
            target.translated_sentence.contains("亲子游戏组"),
            "should locate the playgroup translation sentence, got: {}",
            target.translated_sentence
        );
    }

    #[test]
    fn source_like_patch_residual_detects_mixed_confirmed_term_fragment() {
        let residual = find_source_like_patch_residual(
            "我正躺在以太飞船的铺位上，卡法克斯 Admiral 与三位医生在身旁。",
            "Admiral Carfax",
            "卡法克斯海军上将",
        )
        .expect("mixed source token should be detected");

        assert_eq!(residual.term, "Admiral");
        assert!(residual.translated_excerpt.contains("卡法克斯 Admiral"));
    }

    #[test]
    fn source_like_patch_residual_detects_exact_confirmed_term_left_raw() {
        let residual = find_source_like_patch_residual(
            "这位神祇是史前大陆Hyperborea的主神。",
            "Hyperborea",
            "海波波利亚",
        )
        .expect("raw confirmed source term should be detected");

        assert_eq!(residual.term, "Hyperborea");
        assert_eq!(residual.translated_excerpt, "Hyperborea");
    }

    #[test]
    fn source_like_patch_replaces_mixed_fragment_and_source_token() {
        let patched = replace_source_like_phrase(
            "卡法克斯 Admiral 与三位医生在身旁。",
            "Admiral Carfax",
            "卡法克斯海军上将",
        );

        assert_eq!(patched, "卡法克斯海军上将与三位医生在身旁。");
    }
}
