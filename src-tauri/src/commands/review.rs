//! 译者校对工作台命令：段对修订的读写。
//!
//! 存储：`artifacts/<task_id>/review_overrides.json`（block id → 修订记录）。
//! 与 translated_blocks.json 解耦——不污染机器翻译产物，导出时叠加 overrides。
//! 这保留「原文-机翻-人校」三层可追溯性（译者工作流的基本要求）。

use super::{resolve_runtime_paths, AppError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOverride {
    pub block_id: String,
    /// 译者修订后的译文。
    pub edited_target: String,
    /// 译者批注（可选）。
    #[serde(default)]
    pub comment: String,
    /// 状态：edited=已修订 / approved=已确认（无需改动但已审）。
    #[serde(default = "default_status")]
    pub status: String,
    pub updated_at: String,
}

fn default_status() -> String {
    "edited".to_string()
}

/// 段对视图（校对工作台行）：机翻段对 + 叠加修订层。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSegment {
    pub block_id: String,
    pub order: usize,
    pub source: String,
    pub target: String,
    pub chapter: Option<String>,
    /// 修订层（若有）。
    pub override_edited: Option<ReviewOverride>,
    /// 有效译文 = 修订译文（若有）否则机翻。
    pub effective_target: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewExportResult {
    pub path: String,
    pub segment_count: usize,
    pub edited_count: usize,
}

fn overrides_path(task_id: &str) -> PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .join(task_id)
        .join("review_overrides.json")
}

fn blocks_path(task_id: &str) -> PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .join(task_id)
        .join("translated_blocks.json")
}

fn load_overrides(path: &PathBuf) -> Result<HashMap<String, ReviewOverride>, AppError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| AppError::internal(format!("读取 review_overrides.json 失败: {error}")))?;
    serde_json::from_str(&content)
        .map_err(|error| AppError::internal(format!("解析 review_overrides.json 失败: {error}")))
}

fn persist_overrides(
    path: &PathBuf,
    overrides: &HashMap<String, ReviewOverride>,
) -> Result<(), AppError> {
    let content = serde_json::to_string_pretty(overrides)
        .map_err(|error| AppError::internal(format!("序列化修订层失败: {error}")))?;
    super::storage::write_json_atomic(path, content.as_bytes())
}

/// 读取校对工作台段对（机翻 + 叠加修订层）。
/// 段对收集（检查/列表共用）：translated_blocks + 修订层叠加。
fn collect_review_segments(task_id: &str) -> Result<Vec<ReviewSegment>, AppError> {
    let content = std::fs::read_to_string(blocks_path(task_id)).map_err(|error| {
        AppError::internal(format!("读取 translated_blocks.json 失败: {error}"))
    })?;
    let blocks: Vec<crate::document::TranslatedBlock> =
        serde_json::from_str(&content).map_err(|error| {
            AppError::internal(format!("解析 translated_blocks.json 失败: {error}"))
        })?;
    let overrides = load_overrides(&overrides_path(task_id))?;
    Ok(blocks
        .into_iter()
        .filter(|block| block.translate && !block.source_text.trim().is_empty())
        .map(|block| {
            let override_edited = overrides.get(&block.id).cloned();
            let effective_target = override_edited
                .as_ref()
                .map(|entry| entry.edited_target.clone())
                .unwrap_or_else(|| block.translated_text.clone());
            ReviewSegment {
                block_id: block.id,
                order: block.order,
                source: block.source_text,
                target: block.translated_text,
                chapter: block.chapter_title,
                override_edited,
                effective_target,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn list_review_segments(task_id: String) -> Result<Vec<ReviewSegment>, AppError> {
    tokio::task::spawn_blocking(move || collect_review_segments(&task_id))
        .await
        .map_err(|error| AppError::internal(format!("加载段对失败: {error}")))?
}

/// 保存单段修订（edited_target 与机翻一致时视为 approved 确认）。
#[tauri::command]
pub async fn save_review_edit(
    task_id: String,
    block_id: String,
    edited_target: String,
    comment: Option<String>,
    state: tauri::State<'_, super::AppState>,
) -> Result<Vec<ReviewSegment>, AppError> {
    let _ = state; // 一致性：命令签名带 state；修订存储与任务产物同目录，无需全局锁
    if block_id.trim().is_empty() {
        return Err(AppError::invalid_input("block_id 为空"));
    }
    let path = overrides_path(&task_id);
    let mut overrides = load_overrides(&path)?;
    let edited = edited_target.trim().to_string();
    if edited.is_empty() {
        // 空译文 = 撤回修订
        overrides.remove(&block_id);
    } else {
        overrides.insert(
            block_id.clone(),
            ReviewOverride {
                block_id,
                edited_target: edited,
                comment: comment.unwrap_or_default(),
                status: default_status(),
                updated_at: Utc::now().to_rfc3339(),
            },
        );
    }
    persist_overrides(&path, &overrides)?;
    list_review_segments(task_id).await
}

/// 导出校对版 MD：有效译文按段拼接（章节标题保留），写入用户选择的路径。
#[tauri::command]
pub async fn export_reviewed_markdown(
    task_id: String,
    output_path: String,
    state: tauri::State<'_, super::AppState>,
) -> Result<ReviewExportResult, AppError> {
    let _ = state;
    let output_path = output_path.trim().to_string();
    if output_path.is_empty() {
        return Err(AppError::invalid_input("导出路径为空"));
    }
    let segments = list_review_segments(task_id).await?;
    let mut document = String::new();
    let mut last_chapter: Option<String> = None;
    let mut edited_count = 0usize;
    for segment in &segments {
        if segment.chapter != last_chapter {
            if let Some(chapter) = &segment.chapter {
                if !chapter.trim().is_empty() {
                    document.push_str(&format!("\n\n## {chapter}\n\n"));
                }
            }
            last_chapter = segment.chapter.clone();
        }
        if segment.override_edited.is_some() {
            edited_count += 1;
        }
        document.push_str(segment.effective_target.trim());
        document.push_str("\n\n");
    }
    tokio::fs::write(&output_path, document)
        .await
        .map_err(|error| AppError::internal(format!("写入校对版文件失败: {error}")))?;
    Ok(ReviewExportResult {
        path: output_path,
        segment_count: segments.len(),
        edited_count,
    })
}

/// 校对检查：
/// 四类规则——未翻译 / 残留英文 / 数字不一致 / 换行结构不一致。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCheckIssue {
    pub block_id: String,
    pub rule: String,
    pub detail: String,
    pub source: String,
    pub effective_target: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCheckOptions {
    #[serde(default = "default_check_true")]
    pub untranslated: bool,
    #[serde(default = "default_check_true")]
    pub residual: bool,
    #[serde(default = "default_check_true")]
    pub digits: bool,
    #[serde(default = "default_check_true")]
    pub newlines: bool,
    /// 术语一致性：原文命中术语而译文缺译法。
    #[serde(default = "default_check_true")]
    pub terminology: bool,
}

fn default_check_true() -> bool {
    true
}

impl Default for ReviewCheckOptions {
    fn default() -> Self {
        Self {
            untranslated: true,
            residual: true,
            digits: true,
            newlines: true,
            terminology: true,
        }
    }
}

fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn digit_multiset(text: &str) -> std::collections::BTreeMap<char, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            *counts.entry(c).or_insert(0) += 1;
        }
    }
    counts
}

fn run_checks_on_segments(
    segments: &[ReviewSegment],
    article_type: &str,
    opts: &ReviewCheckOptions,
    terms: &[crate::commands::glossary::GlossaryEntry],
) -> Vec<ReviewCheckIssue> {
    let mut issues = Vec::new();
    for seg in segments {
        let target = &seg.effective_target;
        if opts.untranslated {
            let empty = target.trim().is_empty();
            let same_as_source = !target.trim().is_empty()
                && target.trim().eq_ignore_ascii_case(seg.source.trim());
            let no_chinese = !target.trim().is_empty() && !has_cjk(target);
            if empty || same_as_source || no_chinese {
                issues.push(ReviewCheckIssue {
                    block_id: seg.block_id.clone(),
                    rule: "untranslated".to_string(),
                    detail: "译文缺失或与原文相同".to_string(),
                    source: seg.source.clone(),
                    effective_target: target.clone(),
                });
                continue;
            }
        }
        if opts.residual {
            let terms = crate::translation_validation::detect_residual_english_terms(
                article_type,
                &seg.source,
                target,
            );
            if !terms.is_empty() {
                let names: Vec<String> = terms.iter().take(4).map(|t| t.term.clone()).collect();
                issues.push(ReviewCheckIssue {
                    block_id: seg.block_id.clone(),
                    rule: "residual".to_string(),
                    detail: format!("残留英文：{}", names.join("、")),
                    source: seg.source.clone(),
                    effective_target: target.clone(),
                });
            }
        }
        if opts.digits {
            let src_digits = digit_multiset(&seg.source);
            let tgt_digits = digit_multiset(target);
            if src_digits != tgt_digits {
                issues.push(ReviewCheckIssue {
                    block_id: seg.block_id.clone(),
                    rule: "digits".to_string(),
                    detail: "数字与原文不一致（增减或改动）".to_string(),
                    source: seg.source.clone(),
                    effective_target: target.clone(),
                });
            }
        }
        if opts.newlines {
            let src_newlines = seg.source.matches('\n').count();
            let tgt_newlines = target.matches('\n').count();
            if src_newlines != tgt_newlines {
                issues.push(ReviewCheckIssue {
                    block_id: seg.block_id.clone(),
                    rule: "newlines".to_string(),
                    detail: format!("换行结构不一致（原文 {src_newlines} / 译文 {tgt_newlines}）"),
                    source: seg.source.clone(),
                    effective_target: target.clone(),
                });
            }
        }
        if opts.terminology && !terms.is_empty() {
            // 原文命中术语（大小写不敏感包含）而译文缺该译法 → 术语不一致。
            // 译法为空视为占位术语，跳过（无可校验目标）。
            let mut misses: Vec<&crate::commands::glossary::GlossaryEntry> = Vec::new();
            for term in terms {
                let translation = term.translation.trim();
                if translation.is_empty() {
                    continue;
                }
                if seg.source.to_lowercase().contains(&term.source.to_lowercase())
                    && !target.contains(translation)
                {
                    misses.push(term);
                }
            }
            if !misses.is_empty() {
                let detail = misses
                    .iter()
                    .take(3)
                    .map(|m| format!("{}→{}", m.source, m.translation))
                    .collect::<Vec<_>>()
                    .join("、");
                issues.push(ReviewCheckIssue {
                    block_id: seg.block_id.clone(),
                    rule: "terminology".to_string(),
                    detail: format!(
                        "术语不一致（{}{}）：译文未采用指定译法",
                        detail,
                        if misses.len() > 3 { "…" } else { "" }
                    ),
                    source: seg.source.clone(),
                    effective_target: target.clone(),
                });
            }
        }
    }
    issues
}

#[tauri::command]
pub async fn run_review_checks(
    task_id: String,
    article_type: Option<String>,
    options: Option<ReviewCheckOptions>,
) -> Result<Vec<ReviewCheckIssue>, AppError> {
    let segments = collect_review_segments(&task_id)?;
    let opts = options.unwrap_or_default();
    let article_type = article_type.unwrap_or_else(|| "general".to_string());
    // 术语一致性：读取任务术语表（含用户修订投影）。
    let terms = if opts.terminology {
        crate::commands::glossary::list_glossary_entries(task_id.clone(), None)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(run_checks_on_segments(&segments, &article_type, &opts, &terms))
}

#[cfg(test)]
mod check_tests {
    use super::*;

    fn seg(block_id: &str, source: &str, target: &str) -> ReviewSegment {
        ReviewSegment {
            block_id: block_id.to_string(),
            order: 0,
            source: source.to_string(),
            target: target.to_string(),
            chapter: None,
            override_edited: None,
            effective_target: target.to_string(),
        }
    }

    #[test]
    fn check_rules_detect_each_class() {
        let segments = vec![
            seg("b1", "The cat sat on the mat.", ""),
            seg("b2", "He scored 3 points.", "他得到 4 分。"),
            seg("b3", "First\nSecond", "第一行"),
        ];
        let opts = ReviewCheckOptions::default();
        let issues = run_checks_on_segments(&segments, "general", &opts, &[]);
        let rules: Vec<&str> = issues.iter().map(|i| i.rule.as_str()).collect();
        assert!(rules.contains(&"untranslated"));
        assert!(rules.contains(&"digits"));
        assert!(rules.contains(&"newlines"));
    }

    #[test]
    fn options_disable_rules() {
        let segments = vec![seg("b1", "The cat sat.", "")];
        let opts = ReviewCheckOptions { untranslated: false, residual: false, digits: false, newlines: false, terminology: false };
        assert!(run_checks_on_segments(&segments, "general", &opts, &[]).is_empty());
    }

    #[test]
    fn terminology_check_flags_missing_translation() {
        use crate::commands::glossary::GlossaryEntry;
        let mk = |source: &str, translation: &str| GlossaryEntry {
            source: source.to_string(),
            translation: translation.to_string(),
            category: "strict_term".to_string(),
            occurrences: 1,
            origin: "static".to_string(),
        };
        let terms = vec![mk("Hogwarts", "霍格沃茨"), mk("quidditch", "魁地奇")];
        let segments = vec![
            // 命中术语且译法缺失 → 报
            seg("b1", "He returned to Hogwarts.", "他回到了魔法学校。"),
            // 命中术语且译法齐全 → 不报
            seg("b2", "They played quidditch.", "他们打了魁地奇。"),
            // 原文不含术语 → 不报
            seg("b3", "It rained.", "下雨了。"),
        ];
        let issues = run_checks_on_segments(&segments, "fiction", &ReviewCheckOptions::default(), &terms);
        let hits: Vec<&ReviewCheckIssue> =
            issues.iter().filter(|i| i.rule == "terminology").collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].block_id, "b1");
        assert!(hits[0].detail.contains("Hogwarts→霍格沃茨"));

        // 关闭开关后不再报告
        let opts = ReviewCheckOptions { terminology: false, ..Default::default() };
        let issues = run_checks_on_segments(&segments, "fiction", &opts, &terms);
        assert!(issues.iter().all(|i| i.rule != "terminology"));
    }
}
