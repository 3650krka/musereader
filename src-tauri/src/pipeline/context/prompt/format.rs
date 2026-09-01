use super::super::matching::{
    source_mentions_hint_cached, source_terms_are_similar, SourceCandidateCache,
};
use super::super::nouns::{infer_strict_hit_role, should_localize_unresolved_proper_nouns};
use super::select::{select_context_hints_for_chunk, select_static_glossary_hits_for_chunk};
use crate::llm::ConfirmedTerm;
use crate::pipeline::{
    env_flag_disabled, ProperNounEnforcement, StrictChunkHit, TranslationContext,
};

pub(crate) fn format_translation_context_for_chunk(
    context: &TranslationContext,
    article_type: &str,
    chunk_index: usize,
    source: &str,
) -> String {
    let should_localize_unknown_names = should_localize_unresolved_proper_nouns(article_type);
    let selected = select_context_hints_for_chunk(context, chunk_index, source);
    let static_hits = select_static_glossary_hits_for_chunk(context, source);
    // Cast block（借鉴 cast-block 设计）：已锁定译名的人物/地点/组织词，
    // 不按 chunk 过滤、恒定注入。作用有二：
    // 1) 代词回指一致性——chunk 里只出现 "he/she/they" 而没有名字时，
    //    模型仍能查到该角色已定译名，不会现场另造一个；
    // 2) 跨 chunk 稳定——同一本书任何 chunk 看到的角色表一致，避免中途漂移。
    // 限制 40 条并按 (first_chunk_index, source) 排序，保证内容随翻译推进只增不改，
    // 已生成的 chunk prompt 前缀保持 byte-stable（利 prompt cache）。
    // 仅叙事类文本启用（MUSETRANSLATE_CAST_BLOCK=0 可关）：academic/specialized
    // 的专名多保留原文，cast block 反而引导不必要的音译。
    let cast_enabled = crate::article_policy::ArticlePolicy::from_article_type(article_type)
        .is_narrative()
        && !env_flag_disabled("MUSETRANSLATE_CAST_BLOCK");
    let cast_hits = if cast_enabled {
        collect_cast_block_hints(context)
    } else {
        Vec::new()
    };
    let mut prefix_lines = Vec::new();
    if !cast_hits.is_empty() {
        prefix_lines.push("Established cast (locked translations, apply whenever these characters/places are referenced, including by pronoun):".to_string());
        for hint in &cast_hits {
            if let Some(target) = hint.target.as_deref() {
                prefix_lines.push(format!("- {} => {}", hint.source, target));
            }
        }
    }
    let cast_block = prefix_lines.join("\n");
    if selected.is_empty() {
        let mut lines = vec![
            "Translation context:".to_string(),
            if static_hits.is_empty() {
                "- No glossary hints for this chunk.".to_string()
            } else {
                "- Static glossary hints are available for this chunk.".to_string()
            },
            "- Translate the text faithfully and keep terminology consistent.".to_string(),
        ];
        if should_localize_unknown_names {
            lines.push(
                "- For Chinese narrative reading, transliterate or translate story-world proper nouns into natural Chinese instead of leaving raw Latin spelling in the final text."
                    .to_string(),
            );
        } else {
            lines.push(
                "- Preserve unresolved proper nouns only when the target article type genuinely requires the original Latin spelling."
                    .to_string(),
            );
        }
        if !static_hits.is_empty() {
            lines.push("Static glossary hits in this chunk:".to_string());
            lines.push(
                "- These source terms are user-provided glossary entries and take precedence over inferred terminology."
                    .to_string(),
            );
            for entry in &static_hits {
                lines.push(format!(
                    "- {} => MUST USE EXACTLY {}",
                    entry.source, entry.target
                ));
            }
        }
        if !cast_block.is_empty() {
            lines.push(cast_block);
        }
        return lines.join("\n");
    }

    let strict_chunk_hits = {
        let cache = SourceCandidateCache::new(source);
        selected
            .iter()
            .copied()
            .filter(|hint| {
                hint.enforcement == ProperNounEnforcement::Strict
                    && hint.target.is_some()
                    && source_mentions_hint_cached(source, &hint.source, &cache)
            })
            .collect::<Vec<_>>()
    };
    let contextual_chunk_hits = {
        let cache = SourceCandidateCache::new(source);
        selected
            .iter()
            .copied()
            .filter(|hint| {
                hint.enforcement == ProperNounEnforcement::Contextual
                    && hint.target.is_some()
                    && source_mentions_hint_cached(source, &hint.source, &cache)
            })
            .collect::<Vec<_>>()
    };

    let algorithmic_candidates = selected
        .iter()
        .copied()
        .filter(|hint| hint.target.is_none())
        .collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(selected.len() + strict_chunk_hits.len() + 12);
    lines.push("Translation context:".to_string());
    lines.push("- Keep names, places, and titles consistent with previous chunks.".to_string());

    if !static_hits.is_empty() {
        lines.push("Static glossary hits in this chunk:".to_string());
        lines.push(
            "- These source terms are user-provided glossary entries and take precedence over inferred terminology."
                .to_string(),
        );
        lines.push(
            "- Do not replace them with a different rendering unless the source term itself is absent."
                .to_string(),
        );
        for entry in &static_hits {
            lines.push(format!(
                "- {} => MUST USE EXACTLY {}",
                entry.source, entry.target
            ));
        }
    }

    if !strict_chunk_hits.is_empty() {
        lines.push("Strict glossary hits in this chunk:".to_string());
        lines.push(
            "- The following source terms appear in this chunk and MUST use exactly the mapped Chinese target in translatedText."
                .to_string(),
        );
        lines.push(
            "- Do not invent an alternate transliteration, shortened form, or synonymous Chinese rendering for these terms."
                .to_string(),
        );
        lines.push(
            "- You may repeat these already-known strict terms in confirmedTerms for this chunk to confirm the exact rendering actually used in translatedText."
                .to_string(),
        );
        for hint in &strict_chunk_hits {
            if let Some(target) = hint.target.as_deref() {
                lines.push(format!("- {} => MUST USE EXACTLY {}", hint.source, target));
            }
        }
    }

    if !contextual_chunk_hits.is_empty() {
        lines.push("Contextual glossary hits in this chunk:".to_string());
        lines.push(
            "- These terms should stay in the same translation family as prior chunks, but adjectival/group forms may use natural Chinese morphology."
                .to_string(),
        );
        for hint in &contextual_chunk_hits {
            if let Some(target) = hint.target.as_deref() {
                lines.push(format!(
                    "- {} -> {} when it names a people/group; use a natural Chinese modifier when adjectival",
                    hint.source, target
                ));
            }
        }
    }

    if !algorithmic_candidates.is_empty() {
        lines.push("Algorithmic proper-noun candidates in this chunk:".to_string());
        lines.push(
            "- These candidates were detected by deterministic text rules from the current source chunk."
                .to_string(),
        );
        lines.push(
            "- Treat them as useful signals for identifying names, places, titles, groups, or organizations, not as mandatory translations."
                .to_string(),
        );
        lines.push(
            "- If a candidate is actually ordinary prose, OCR noise, or a false positive, translate the sentence naturally."
                .to_string(),
        );
        for hint in &algorithmic_candidates {
            if should_localize_unknown_names {
                lines.push(format!(
                    "- {} -> likely proper noun; if it is a real name/title/place in this chunk, render it naturally in Chinese rather than leaving raw Latin spelling",
                    hint.source
                ));
            } else {
                lines.push(format!(
                    "- {} -> likely proper noun; preserve or translate according to academic convention and local sentence context",
                    hint.source
                ));
            }
        }
    }

    lines.push("Other glossary hints:".to_string());
    lines.push("- Apply these mappings when relevant.".to_string());

    for hint in selected {
        let line = match hint.target.as_deref() {
            Some(target) if hint.enforcement == ProperNounEnforcement::Contextual => format!(
                "- {} -> {} when it names a people/group; use a natural Chinese modifier when adjectival",
                hint.source, target
            ),
            Some(target) => format!("- {} -> {}", hint.source, target),
            None => continue,
        };
        lines.push(line);
    }

    if !cast_block.is_empty() {
        lines.push(cast_block);
    }

    lines.join("\n")
}

/// Cast block 收集：已锁定译名（target 已写回）的 Strict 级人物/地点/组织词。
/// 不按 chunk 过滤（这正是设计要点：代词回指时源名未必出现在本 chunk）。
/// 排序保证只增不改的前缀稳定性；上限 40 条防爆 prompt。
fn collect_cast_block_hints(context: &TranslationContext) -> Vec<&crate::pipeline::ProperNounHint> {
    const CAST_BLOCK_LIMIT: usize = 40;
    let mut cast = context
        .proper_nouns
        .iter()
        .filter(|hint| hint.enforcement == ProperNounEnforcement::Strict && hint.target.is_some())
        .collect::<Vec<_>>();
    cast.sort_by(|left, right| {
        left.first_chunk_index
            .cmp(&right.first_chunk_index)
            .then_with(|| left.source.cmp(&right.source))
    });
    cast.truncate(CAST_BLOCK_LIMIT);
    cast
}

pub(crate) fn collect_strict_chunk_hits(
    context: &TranslationContext,
    source: &str,
) -> Vec<StrictChunkHit> {
    let cache = SourceCandidateCache::new(source);
    let mut hits = context
        .static_terms
        .iter()
        .filter(|entry| {
            !entry.source.trim().is_empty()
                && !entry.target.trim().is_empty()
                && source_mentions_hint_cached(source, &entry.source, &cache)
                && !entry.enforcement.trim().eq_ignore_ascii_case("contextual")
        })
        .map(|entry| StrictChunkHit {
            source: entry.source.clone(),
            target: entry.target.trim().to_string(),
            usage_role: "static_glossary".to_string(),
            term_type: "static_term".to_string(),
        })
        .collect::<Vec<_>>();

    hits.extend(
        context
            .proper_nouns
            .iter()
            .filter(|hint| {
                hint.enforcement == ProperNounEnforcement::Strict
                    && hint.target.is_some()
                    && source_mentions_hint_cached(source, &hint.source, &cache)
            })
            .filter_map(|hint| {
                let target = hint.target.as_deref()?.trim();
                if target.is_empty() {
                    return None;
                }
                let (usage_role, term_type) = infer_strict_hit_role(&hint.source);
                Some(StrictChunkHit {
                    source: hint.source.clone(),
                    target: target.to_string(),
                    usage_role: usage_role.to_string(),
                    term_type: term_type.to_string(),
                })
            })
            .collect::<Vec<_>>(),
    );
    hits
}

pub(crate) fn prune_confirmed_algorithmic_hints(
    context: &mut TranslationContext,
    chunk_index: usize,
    confirmed_terms: &[ConfirmedTerm],
) -> usize {
    if confirmed_terms.is_empty() {
        return 0;
    }
    let before = context.proper_nouns.len();
    context.proper_nouns.retain(|hint| {
        if hint.target.is_some() || !algorithmic_hint_belongs_to_chunk(hint, chunk_index) {
            return true;
        }
        !confirmed_terms
            .iter()
            .any(|term| source_terms_are_similar(&hint.source, &term.source))
    });
    before.saturating_sub(context.proper_nouns.len())
}

fn algorithmic_hint_belongs_to_chunk(
    hint: &crate::pipeline::ProperNounHint,
    chunk_index: usize,
) -> bool {
    if hint.chunk_indexes.is_empty() {
        return hint.first_chunk_index == chunk_index;
    }
    hint.chunk_indexes.binary_search(&chunk_index).is_ok()
}

pub(crate) fn synthesize_missing_strict_hit_terms(
    strict_hits: &[StrictChunkHit],
    translated_text: &str,
    confirmed_terms: &mut Vec<ConfirmedTerm>,
) {
    for hit in strict_hits {
        if !translated_text.contains(&hit.target) {
            continue;
        }
        if confirmed_terms.iter().any(|term| {
            collapse_source_surface(&term.source)
                .eq_ignore_ascii_case(&collapse_source_surface(&hit.source))
                || term.translation.trim() == hit.target
        }) {
            continue;
        }
        confirmed_terms.push(ConfirmedTerm {
            source: hit.source.clone(),
            translation: hit.target.clone(),
            term_type: hit.term_type.clone(),
            confidence: 100,
            usage_role: hit.usage_role.clone(),
        });
    }
}

pub(crate) fn repair_strict_hit_aliases(
    translated_text: &str,
    confirmed_terms: &[ConfirmedTerm],
    strict_hits: &[StrictChunkHit],
) -> String {
    let mut repaired = translated_text.to_string();
    // 按 alias 长度降序：先替换长 alias（"爱丽斯·史密斯"），再替换短 alias（"爱丽丝"），
    // 避免短 alias 是长 alias 子串时误覆盖。
    let mut alias_pairs: Vec<(&str, &str)> = Vec::new();
    for hit in strict_hits {
        for term in confirmed_terms {
            if !collapse_source_surface(&term.source)
                .eq_ignore_ascii_case(&collapse_source_surface(&hit.source))
            {
                continue;
            }
            let alias = term.translation.trim();
            if alias.is_empty() || alias == hit.target {
                continue;
            }
            // alias 已包含 target（如 alias="爱丽丝·史密斯" 含 target="爱丽丝"）→ 跳过，
            // 避免把全名压成短名。
            if alias.contains(hit.target.as_str()) {
                continue;
            }
            alias_pairs.push((alias, &hit.target));
        }
    }
    alias_pairs.sort_by_key(|(alias, _)| std::cmp::Reverse(alias.chars().count()));
    alias_pairs.dedup();
    for (alias, target) in alias_pairs {
        if repaired.contains(alias) {
            repaired = repaired.replace(alias, target);
        }
    }
    repaired
}

pub(crate) fn normalize_strict_hit_confirmed_terms(
    strict_hits: &[StrictChunkHit],
    confirmed_terms: &mut Vec<ConfirmedTerm>,
) {
    for term in confirmed_terms {
        if let Some(hit) = strict_hits.iter().find(|hit| {
            collapse_source_surface(&term.source)
                .eq_ignore_ascii_case(&collapse_source_surface(&hit.source))
        }) {
            term.translation = hit.target.clone();
            term.term_type = hit.term_type.clone();
            term.usage_role = hit.usage_role.clone();
            term.confidence = term.confidence.max(100);
        }
    }
}

pub(crate) fn repair_canonical_proper_nouns(
    translated: &str,
    context: Option<&TranslationContext>,
) -> String {
    let Some(context) = context else {
        return translated.to_string();
    };
    let mut repaired = translated.to_string();
    let mut hints = context
        .proper_nouns
        .iter()
        .filter(|hint| hint.enforcement == ProperNounEnforcement::Strict)
        .filter_map(|hint| {
            hint.target
                .as_ref()
                .map(|target| (hint.source.as_str(), target.as_str()))
        })
        // 先按「译文中实际出现 source」过滤——多数 chunk 只命中个位数 hint，
        // 避免对数十-数百个 hint 做无谓的全文扫描+替换。
        .filter(|(source, _)| repaired.contains(*source))
        .collect::<Vec<_>>();
    hints.sort_by_key(|(source, _)| std::cmp::Reverse(source.len()));
    for (source, target) in hints {
        repaired = replace_source_term_with_boundary(&repaired, source, target);
    }
    repaired
}

fn replace_source_term_with_boundary(text: &str, source: &str, target: &str) -> String {
    if source.trim().is_empty() {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(relative_start) = text[cursor..].find(source) {
        let start = cursor + relative_start;
        let end = start + source.len();
        output.push_str(&text[cursor..start]);
        if is_source_term_boundary_match(text, start, end) {
            output.push_str(target);
        } else {
            output.push_str(&text[start..end]);
        }
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

fn is_source_term_boundary_match(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_latin_identifier_char) && !after.is_some_and(is_latin_identifier_char)
}

fn is_latin_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '.')
}

fn collapse_source_surface(source: &str) -> String {
    source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("- ", "-")
        .replace(" -", "-")
}
