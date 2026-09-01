use super::super::chunking::should_skip_residual_review;
use super::super::context::normalize_resource_term_key;
use super::super::context::source_mentions_hint;
use super::super::layout::PDF_TABLE_IMAGE_MISSING_MARKER;
use super::*;
use crate::translation_validation::detect_residual_english_terms;
use chrono::Utc;
use std::collections::BTreeMap;

const MSG_UNTRANSLATED_CHUNK: &str =
    "\u{5206}\u{5757}\u{6CA1}\u{6709}\u{751F}\u{6210}\u{8BD1}\u{6587}";
const MSG_EMPTY_TRANSLATION: &str = "\u{5206}\u{5757}\u{8BD1}\u{6587}\u{4E3A}\u{7A7A}";
const MSG_SOURCE_EQUALS_TRANSLATION: &str =
    "\u{8BD1}\u{6587}\u{4E0E}\u{539F}\u{6587}\u{5B8C}\u{5168}\u{76F8}\u{540C}\u{FF0C}\u{7591}\u{4F3C}\u{672A}\u{7FFB}\u{8BD1}";

pub(crate) fn build_validation_report(
    task_id: &str,
    checkpoint: &ChunkCheckpoint,
) -> ValidationReport {
    let mut issues = Vec::new();
    let mut untranslated_chunk_count = 0usize;
    let mut empty_chunk_count = 0usize;
    let mut markdown_heading_count = 0usize;

    for chunk in &checkpoint.chunks {
        match chunk.translated.as_ref() {
            None => {
                untranslated_chunk_count += 1;
                issues.push(ValidationIssue {
                    code: "UNTRANSLATED_CHUNK".to_string(),
                    level: "error".to_string(),
                    chunk_index: Some(chunk.index),
                    message: MSG_UNTRANSLATED_CHUNK.to_string(),
                });
            }
            Some(translated) => {
                if chunk.segment_kind != ChunkSegmentKind::Body {
                    continue;
                }
                let source_is_blank = chunk.source.trim().is_empty();
                if translated.trim().is_empty() && !source_is_blank {
                    empty_chunk_count += 1;
                    issues.push(ValidationIssue {
                        code: "EMPTY_TRANSLATION".to_string(),
                        level: "error".to_string(),
                        chunk_index: Some(chunk.index),
                        message: MSG_EMPTY_TRANSLATION.to_string(),
                    });
                }
                if translated.contains("# ") {
                    markdown_heading_count += translated.matches("# ").count();
                }
                if translated == &chunk.source && !source_is_blank {
                    issues.push(ValidationIssue {
                        code: "SOURCE_EQUALS_TRANSLATION".to_string(),
                        level: "warning".to_string(),
                        chunk_index: Some(chunk.index),
                        message: MSG_SOURCE_EQUALS_TRANSLATION.to_string(),
                    });
                }
                collect_pdf_table_snapshot_issues(&mut issues, chunk.index, translated);
                collect_unresolved_proper_noun_issues(
                    &mut issues,
                    checkpoint.translation_context.as_ref(),
                    &checkpoint.article_type,
                    chunk.index,
                    &chunk.source,
                    translated,
                );
                collect_residual_english_issues(
                    &mut issues,
                    &checkpoint.article_type,
                    chunk.index,
                    &chunk.source,
                    translated,
                );
            }
        }
    }

    collect_proper_noun_count_issues(&mut issues, checkpoint);
    collect_content_drift_issues(&mut issues, checkpoint);

    let summary = ValidationSummary {
        issue_count: issues.len(),
        untranslated_chunk_count,
        empty_chunk_count,
        markdown_heading_count,
    };

    ValidationReport {
        version: 1,
        task_id: task_id.to_string(),
        created_at: Utc::now().to_rfc3339(),
        summary,
        issues,
    }
}

fn collect_pdf_table_snapshot_issues(
    issues: &mut Vec<ValidationIssue>,
    chunk_index: usize,
    translated: &str,
) {
    if translated.contains(PDF_TABLE_IMAGE_MISSING_MARKER) {
        issues.push(ValidationIssue {
            code: "PDF_TABLE_SNAPSHOT_MISSING".to_string(),
            level: "error".to_string(),
            chunk_index: Some(chunk_index),
            message:
                "PDF table snapshot missing; table block would render as an unavailable placeholder"
                    .to_string(),
        });
    }
}

fn collect_residual_english_issues(
    issues: &mut Vec<ValidationIssue>,
    article_type: &str,
    chunk_index: usize,
    source: &str,
    translated: &str,
) {
    if should_skip_residual_review(article_type, source) {
        return;
    }
    for residual in detect_residual_english_terms(article_type, source, translated) {
        issues.push(ValidationIssue {
            code: "RESIDUAL_ENGLISH_WORD".to_string(),
            level: "warning".to_string(),
            chunk_index: Some(chunk_index),
            message: format!(
                "Suspected residual English: {} x{}; source excerpt: {}; translated excerpt: {}",
                residual.term,
                residual.count,
                compact_validation_excerpt(&residual.source_excerpt),
                compact_validation_excerpt(&residual.translated_excerpt)
            ),
        });
    }
}

fn compact_validation_excerpt(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

/// 译文格式校验信号（曲引号配对/章标题双语残留）→ validation warning。
/// 只报 warning 不阻断 pipeline，供人工/复审聚焦。
/// （注：原「数字/否定语义漂移」信号已按用户决策移除——漂移是模型语义理解
///  问题，表面词守恒测不准且误报，检测层面无解。）
fn collect_content_drift_issues(issues: &mut Vec<ValidationIssue>, checkpoint: &ChunkCheckpoint) {
    for signal in crate::pipeline::drift::detect_content_drift(checkpoint) {
        issues.push(ValidationIssue {
            code: signal.code.to_string(),
            level: "warning".to_string(),
            chunk_index: Some(signal.chunk_index),
            message: signal.detail,
        });
    }
}

fn collect_proper_noun_count_issues(
    issues: &mut Vec<ValidationIssue>,
    checkpoint: &ChunkCheckpoint,
) {
    let Some(context) = checkpoint.translation_context.as_ref() else {
        return;
    };
    let source_text = checkpoint
        .chunks
        .iter()
        .map(|chunk| chunk.source.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let translated_text = checkpoint
        .chunks
        .iter()
        .filter_map(|chunk| chunk.translated.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    let mut counted = BTreeMap::<String, (String, String)>::new();
    for hint in &context.proper_nouns {
        if hint.enforcement != ProperNounEnforcement::Strict {
            continue;
        }
        let Some(target) = hint
            .target
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        counted
            .entry(normalize_resource_term_key(&hint.source))
            .or_insert_with(|| (hint.source.clone(), target.trim().to_string()));
    }
    for (_, (source_term, target)) in counted {
        let source_count = source_text.matches(&source_term).count();
        let target_count = translated_text.matches(&target).count();
        if source_count > 0 && target_count == 0 {
            issues.push(ValidationIssue {
                code: "PROPER_NOUN_COUNT_MISSING_TARGET".to_string(),
                level: "warning".to_string(),
                chunk_index: None,
                message: format!(
                    "Canonical target missing in aggregate counts: {} x{} -> {} x{}",
                    source_term, source_count, target, target_count
                ),
            });
        }
    }
}

fn collect_unresolved_proper_noun_issues(
    issues: &mut Vec<ValidationIssue>,
    context: Option<&TranslationContext>,
    _article_type: &str,
    chunk_index: usize,
    source: &str,
    translated: &str,
) {
    let Some(context) = context else {
        return;
    };
    for hint in validated_resource_hints(context, chunk_index, source) {
        let Some(target) = hint.target.as_deref() else {
            continue;
        };
        if translated.contains(&hint.source) {
            issues.push(ValidationIssue {
                code: "UNRESOLVED_PROPER_NOUN".to_string(),
                level: "warning".to_string(),
                chunk_index: Some(chunk_index),
                message: format!("Proper noun still left in source language: {}", hint.source),
            });
        }
        if hint.enforcement == ProperNounEnforcement::Strict && !translated.contains(target) {
            issues.push(ValidationIssue {
                code: "MISSING_CANONICAL_PROPER_NOUN_TRANSLATION".to_string(),
                level: "warning".to_string(),
                chunk_index: Some(chunk_index),
                message: format!(
                    "Canonical proper noun translation missing: {} -> {}",
                    hint.source, target
                ),
            });
        }
    }
}

fn validated_resource_hints(
    context: &TranslationContext,
    chunk_index: usize,
    source: &str,
) -> Vec<ProperNounHint> {
    let mut resources = BTreeMap::<String, ProperNounHint>::new();
    for hint in &context.proper_nouns {
        if validation_hint_has_locator(hint) {
            if !validation_hint_belongs_to_chunk(hint, chunk_index) {
                continue;
            }
        } else if !source_mentions_hint(source, &hint.source) {
            continue;
        }
        let Some(target) = hint
            .target
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let key = normalize_resource_term_key(&hint.source);
        resources.entry(key).or_insert_with(|| ProperNounHint {
            source: hint.source.clone(),
            target: Some(target.trim().to_string()),
            enforcement: hint.enforcement,
            occurrences: hint.occurrences,
            first_chunk_index: hint.first_chunk_index,
            chunk_indexes: hint.chunk_indexes.clone(),
        });
    }
    resources.into_values().collect()
}

fn validation_hint_belongs_to_chunk(hint: &ProperNounHint, chunk_index: usize) -> bool {
    if hint.chunk_indexes.is_empty() {
        return hint.first_chunk_index == chunk_index;
    }
    hint.chunk_indexes.binary_search(&chunk_index).is_ok()
}

fn validation_hint_has_locator(hint: &ProperNounHint) -> bool {
    !hint.chunk_indexes.is_empty() || hint.first_chunk_index > 0
}
