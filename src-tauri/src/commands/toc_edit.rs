//! 目录编辑命令（校对工作台）：重命名 / 合并删除 / 新建目录条目。
//!
//! 存储：`artifacts/<task_id>/toc_overrides.json`——非破坏性覆盖层
//! （与 review_overrides 同哲学）：不改写管线产物 translated_toc.json，
//! 读取时叠加。合并语义：删除多余目录条目后，其导引的正文仍随前一章节呈现
//! （目录是大纲视图，内容块不随目录条目删除）。

use super::resolve_runtime_paths;
use crate::document::{TocArtifact, TocEntry, TocSourceKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 目录覆盖层：renames 按原条目 order 索引；removed 记录被合并删除的 order；
/// added 为用户新建条目（插入位置以 after_order 锚定）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TocOverrides {
    #[serde(default)]
    pub renames: HashMap<usize, String>,
    #[serde(default)]
    pub removed: Vec<usize>,
    #[serde(default)]
    pub added: Vec<AddedTocEntry>,
    /// 页码修订：原条目 order → 用户指定页码。
    #[serde(default)]
    pub pages: HashMap<usize, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedTocEntry {
    pub id: String,
    pub title: String,
    /// 插到哪个原条目之后（None=最前）。
    pub after_order: Option<usize>,
    #[serde(default = "default_level")]
    pub level: u8,
    pub page: Option<usize>,
}

fn default_level() -> u8 {
    1
}

/// 目录编辑视图行（前端目录管理列表）：叠加覆盖层后的条目 + 稳定 id。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TocEditItem {
    /// 稳定标识：原条目 "o{order}"；新增 "a{id}"。
    pub id: String,
    pub title: String,
    pub source_title: Option<String>,
    pub level: u8,
    pub page: Option<usize>,
    pub href: Option<String>,
    /// original=产物条目 / added=用户新建。
    pub kind: String,
}

fn overrides_path(task_id: &str) -> PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .join(task_id)
        .join("toc_overrides.json")
}

fn translated_toc_path(task_id: &str) -> PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .join(task_id)
        .join("translated_toc.json")
}

fn source_toc_path(task_id: &str) -> PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .join(task_id)
        .join("source_toc.json")
}

pub(crate) fn load_overrides(task_id: &str) -> Result<TocOverrides, super::AppError> {
    let path = overrides_path(task_id);
    if !path.exists() {
        return Ok(TocOverrides::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| super::AppError::internal(format!("读取 toc_overrides.json 失败: {error}")))?;
    parse_overrides_lenient(&content).map_err(|error| {
        super::AppError::internal(format!("解析 toc_overrides.json 失败: {error}"))
    })
}

/// 宽容解析：整体失败时逐字段降级，单字段损坏不拖垮整个目录面板。
///
/// 历史教训：外部脚本曾把 removed 写成字符串数组（["5","15","20"]），
/// 严格解析失败后 load_overrides 返回 Err——校对目录面板整页变成
/// 「没有目录工件」，而同一份覆盖层在阅读器侧只是静默跳过（症状不对称）。
fn parse_overrides_lenient(content: &str) -> Result<TocOverrides, String> {
    match serde_json::from_str::<TocOverrides>(content) {
        Ok(overrides) => Ok(overrides),
        Err(strict_error) => {
            eprintln!(
                "toc_overrides.json 整体解析失败，逐字段降级读取: {strict_error}"
            );
            let value: serde_json::Value = serde_json::from_str(content)
                .map_err(|error| format!("{error}"))?;
            let object = value.as_object().ok_or("覆盖层不是 JSON 对象")?;
            let renames = serde_json::from_value(
                object.get("renames").cloned().unwrap_or(serde_json::Value::Null),
            )
            .unwrap_or_default();
            let removed: Vec<usize> = value_from(object, "removed")
                .as_array()
                .map(|items| items.iter().filter_map(coerce_usize).collect())
                .unwrap_or_default();
            let added = serde_json::from_value(value_from(object, "added")).unwrap_or_default();
            let pages = serde_json::from_value(value_from(object, "pages")).unwrap_or_default();
            Ok(TocOverrides {
                renames,
                removed,
                added,
                pages,
            })
        }
    }
}

fn value_from(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> serde_json::Value {
    object.get(key).cloned().unwrap_or(serde_json::Value::Null)
}

/// 数字或数字字符串统一转为 usize（覆盖层历史数据两种形态都出现过）。
fn coerce_usize(value: &serde_json::Value) -> Option<usize> {
    match value {
        serde_json::Value::Number(number) => number.as_u64().map(|v| v as usize),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn persist_overrides(task_id: &str, overrides: &TocOverrides) -> Result<(), super::AppError> {
    let content = serde_json::to_string_pretty(overrides)
        .map_err(|error| super::AppError::internal(format!("序列化目录覆盖层失败: {error}")))?;
    super::storage::write_json_atomic(&overrides_path(task_id), content.as_bytes())
}

fn load_toc_artifact(task_id: &str) -> Result<Option<TocArtifact>, super::AppError> {
    for path in [translated_toc_path(task_id), source_toc_path(task_id)] {
        if let Some(artifact) = super::load_single_json::<TocArtifact>(&path) {
            return Ok(Some(artifact));
        }
    }
    Ok(None)
}

/// 叠加覆盖层：改名（translated_title 覆盖）→ 删除（合并语义）→ 插入新增，
/// order 连续重编号。纯函数，便于单测。
pub fn apply_toc_overrides(artifact: &mut TocArtifact, overrides: &TocOverrides) {
    for (order, title) in &overrides.renames {
        if let Some(entry) = artifact.entries.get_mut(*order) {
            entry.translated_title = Some(title.clone());
        }
    }
    for (order, page) in &overrides.pages {
        if let Some(entry) = artifact.entries.get_mut(*order) {
            entry.page = Some(*page);
        }
    }
    let removed: std::collections::HashSet<usize> = overrides.removed.iter().copied().collect();
    artifact.entries.retain(|entry| !removed.contains(&entry.order));
    for added in &overrides.added {
        let position = match added.after_order {
            Some(order) => artifact
                .entries
                .iter()
                .position(|entry| entry.order == order)
                .map(|index| index + 1)
                .unwrap_or(artifact.entries.len()),
            None => 0,
        };
        artifact.entries.insert(
            position,
            TocEntry {
                source_title: added.title.clone(),
                translated_title: Some(added.title.clone()),
                level: added.level.max(1),
                order: 0, // 稍后统一重编号
                page: added.page,
                href: None,
                source_kind: TocSourceKind::None,
                confidence: 1.0,
            },
        );
    }
    for (index, entry) in artifact.entries.iter_mut().enumerate() {
        entry.order = index;
    }
}

fn build_edit_items(task_id: &str) -> Result<Vec<TocEditItem>, super::AppError> {
    let Some(mut artifact) = load_toc_artifact(task_id)? else {
        return Err(super::AppError::not_found("未找到目录工件"));
    };
    let overrides = load_overrides(task_id)?;
    // 先按未叠加状态确定原条目 id（order 是产物 order）
    let added_ids: std::collections::HashSet<String> =
        overrides.added.iter().map(|entry| entry.id.clone()).collect();
    let original_orders_before: Vec<usize> = artifact.entries.iter().map(|entry| entry.order).collect();
    apply_toc_overrides(&mut artifact, &overrides);

    // 重建视图：用双指针把叠加后的条目映射回 id
    let mut originals = original_orders_before.into_iter();
    let mut added_iter = overrides.added.iter();
    let mut items = Vec::with_capacity(artifact.entries.len());
    for entry in &artifact.entries {
        // 新增条目特征：source_kind == None 且 title 与某个 added 匹配
        if entry.source_kind == TocSourceKind::None {
            if let Some(added) = added_iter.next() {
                items.push(TocEditItem {
                    id: format!("a{}", added.id),
                    title: entry.translated_title.clone().unwrap_or_default(),
                    source_title: None,
                    level: entry.level,
                    page: entry.page,
                    href: None,
                    kind: "added".to_string(),
                });
                continue;
            }
        }
        let order = originals.next().unwrap_or(entry.order);
        let title = entry
            .translated_title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&entry.source_title)
            .to_string();
        let source_title = (entry.source_title.trim() != title)
            .then(|| entry.source_title.clone());
        items.push(TocEditItem {
            id: format!("o{order}"),
            title,
            source_title,
            level: entry.level,
            page: entry.page,
            href: entry.href.clone(),
            kind: "original".to_string(),
        });
    }
    let _ = added_ids;
    Ok(items)
}

/// 目录编辑列表（叠加覆盖层）。
#[tauri::command]
pub async fn list_toc_edit_items(task_id: String) -> Result<Vec<TocEditItem>, super::AppError> {
    build_edit_items(&task_id)
}

/// 重命名目录条目（id："o{order}" 原条目 / "a{uuid}" 新增条目）。
#[tauri::command]
pub async fn rename_toc_entry(
    task_id: String,
    entry_id: String,
    title: String,
) -> Result<Vec<TocEditItem>, super::AppError> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(super::AppError::invalid_input("目录标题为空"));
    }
    let mut overrides = load_overrides(&task_id)?;
    if let Some(order) = parse_original_id(&entry_id) {
        overrides.renames.insert(order, title);
    } else if let Some(added_id) = entry_id.strip_prefix('a') {
        let Some(entry) = overrides.added.iter_mut().find(|entry| entry.id == added_id) else {
            return Err(super::AppError::not_found("目录条目不存在"));
        };
        entry.title = title;
    } else {
        return Err(super::AppError::invalid_input("目录条目 id 非法"));
    }
    persist_overrides(&task_id, &overrides)?;
    build_edit_items(&task_id)
}

/// 删除/合并目录条目：原条目移入 removed（其导引内容并入相邻章节，内容不删）；
/// 新增条目直接从覆盖层移除。
#[tauri::command]
pub async fn remove_toc_entry(
    task_id: String,
    entry_id: String,
) -> Result<Vec<TocEditItem>, super::AppError> {
    let mut overrides = load_overrides(&task_id)?;
    if let Some(order) = parse_original_id(&entry_id) {
        overrides.renames.remove(&order);
        if !overrides.removed.contains(&order) {
            overrides.removed.push(order);
        }
    } else if let Some(added_id) = entry_id.strip_prefix('a') {
        let before = overrides.added.len();
        overrides.added.retain(|entry| entry.id != added_id);
        if overrides.added.len() == before {
            return Err(super::AppError::not_found("目录条目不存在"));
        }
    } else {
        return Err(super::AppError::invalid_input("目录条目 id 非法"));
    }
    persist_overrides(&task_id, &overrides)?;
    build_edit_items(&task_id)
}

/// 新建目录条目：插到 after_id 之后（None=最前）；page 为页码（PDF 类书籍）。
#[tauri::command]
pub async fn add_toc_entry(
    task_id: String,
    title: String,
    after_id: Option<String>,
    page: Option<usize>,
) -> Result<Vec<TocEditItem>, super::AppError> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(super::AppError::invalid_input("目录标题为空"));
    }
    let mut overrides = load_overrides(&task_id)?;
    let after_order = match &after_id {
        None => None,
        Some(id) => Some(parse_original_id(id).ok_or_else(|| {
            super::AppError::invalid_input("新目录只能插入到产物条目之后")
        })?),
    };
    overrides.added.push(AddedTocEntry {
        id: uuid::Uuid::new_v4().to_string(),
        title,
        after_order,
        level: 1,
        page,
    });
    persist_overrides(&task_id, &overrides)?;
    build_edit_items(&task_id)
}

fn parse_original_id(entry_id: &str) -> Option<usize> {
    entry_id.strip_prefix('o').and_then(|value| value.parse::<usize>().ok())
}


/// 页码预览（新建目录条目用）：
/// 1) PDF 类书籍：source_layouts.json 每页一个 OcrPageLayout，总页数=页数；
/// 2) EPUB/无版面工件回退：
///    source_blocks.json 按 SOURCE_BLOCKS_PER_PAGE 块聚合成伪分页，
///    预览=该伪页的块文本拼接截断——页码语义与 PDF 页一致（源文档阅读序）。
///    过滤口径与校对段对（collect_review_segments：translate && 原文非空）完全一致，
///    保证伪页 k ⇔ 校对工作台全量段对 [k*30, (k+1)*30)——分页视图与页码跳转共用同一坐标。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePreviewResult {
    pub total_pages: usize,
    pub preview: String,
}

const PAGE_PREVIEW_MAX_CHARS: usize = 600;
/// EPUB 伪分页粒度：约每页 30 个内容块（≈2–3 屏正文）。 */
const SOURCE_BLOCKS_PER_PAGE: usize = 30;

#[tauri::command]
pub async fn get_page_preview(task_id: String, page: usize) -> Result<PagePreviewResult, super::AppError> {
    let artifact_dir = resolve_runtime_paths().artifact_root_dir.join(&task_id);
    let layouts_path = artifact_dir.join("source_layouts.json");
    if let Ok(content) = tokio::fs::read_to_string(&layouts_path).await {
        let layouts: Vec<crate::ocr::OcrPageLayout> = serde_json::from_str(&content)
            .map_err(|error| super::AppError::internal(format!("source_layouts.json 解析失败: {error}")))?;
        let total_pages = layouts.len();
        let preview = layouts
            .get(page.saturating_sub(1))
            .map(|layout| {
                let joined: Vec<&str> = layout
                    .blocks
                    .iter()
                    .map(|block| block.content.trim())
                    .filter(|content| !content.is_empty())
                    .collect();
                truncate_preview(&joined.join("\n"))
            })
            .unwrap_or_default();
        return Ok(PagePreviewResult { total_pages, preview });
    }

    // EPUB 回退：source_blocks.json 伪分页（过滤口径对齐校对段对）
    let blocks_path = artifact_dir.join("source_blocks.json");
    let raw = match tokio::fs::read_to_string(&blocks_path).await {
        Ok(content) => content,
        Err(_) => return Ok(PagePreviewResult { total_pages: 0, preview: String::new() }),
    };
    let blocks: Vec<crate::document::SourceBlock> = serde_json::from_str(&raw)
        .map_err(|error| super::AppError::internal(format!("source_blocks.json 解析失败: {error}")))?;
    let review_blocks: Vec<&crate::document::SourceBlock> = blocks
        .iter()
        .filter(|block| block.translate && !block.text.trim().is_empty())
        .collect();
    let total_pages = review_blocks.len().div_ceil(SOURCE_BLOCKS_PER_PAGE).max(1);
    let start = page.saturating_sub(1) * SOURCE_BLOCKS_PER_PAGE;
    let preview = review_blocks
        .iter()
        .skip(start)
        .take(SOURCE_BLOCKS_PER_PAGE)
        .map(|block| block.text.trim())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(PagePreviewResult { total_pages, preview: truncate_preview(&preview) })
}

fn truncate_preview(text: &str) -> String {
    if text.chars().count() <= PAGE_PREVIEW_MAX_CHARS {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(PAGE_PREVIEW_MAX_CHARS).collect();
        format!("{truncated}…")
    }
}

/// 修改目录条目页码。
#[tauri::command]
pub async fn set_toc_entry_page(
    task_id: String,
    entry_id: String,
    page: Option<usize>,
) -> Result<Vec<TocEditItem>, super::AppError> {
    let mut overrides = load_overrides(&task_id)?;
    if let Some(order) = parse_original_id(&entry_id) {
        match page {
            Some(page) => {
                overrides.pages.insert(order, page);
            }
            None => {
                overrides.pages.remove(&order);
            }
        }
    } else if let Some(added_id) = entry_id.strip_prefix('a') {
        let Some(entry) = overrides.added.iter_mut().find(|entry| entry.id == added_id) else {
            return Err(super::AppError::not_found("目录条目不存在"));
        };
        entry.page = page;
    } else {
        return Err(super::AppError::invalid_input("目录条目 id 非法"));
    }
    persist_overrides(&task_id, &overrides)?;
    build_edit_items(&task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实事故回归：外部脚本曾把 removed 写成字符串数组，严格解析失败后
    /// 校对目录面板整页降级成「没有目录工件」——宽容解析必须救回全部字段。
    #[test]
    fn parse_overrides_lenient_recovers_string_removed_without_losing_renames() {
        let content = r#"{
            "renames": { "6": "第2章：塔什", "101": "出版社后置插页" },
            "removed": ["5", "15", 20],
            "added": [],
            "pages": { "3": 7 }
        }"#;
        let overrides = parse_overrides_lenient(content).expect("宽容解析应成功");
        assert_eq!(overrides.removed, vec![5, 15, 20]);
        assert_eq!(
            overrides.renames.get(&6).map(String::as_str),
            Some("第2章：塔什")
        );
        assert_eq!(
            overrides.renames.get(&101).map(String::as_str),
            Some("出版社后置插页")
        );
        assert_eq!(overrides.pages.get(&3), Some(&7));
        assert!(overrides.added.is_empty());
    }

    #[test]
    fn parse_overrides_strict_shape_round_trips() {
        let overrides = TocOverrides {
            renames: [(2usize, "改名".to_string())].into_iter().collect(),
            removed: vec![4],
            added: vec![AddedTocEntry {
                id: "x".to_string(),
                title: "新条目".to_string(),
                after_order: Some(1),
                level: 1,
                page: None,
            }],
            pages: [(2usize, 9usize)].into_iter().collect(),
        };
        let json = serde_json::to_string(&overrides).expect("序列化应成功");
        let parsed = parse_overrides_lenient(&json).expect("严格形态应直接解析");
        assert_eq!(parsed.removed, vec![4]);
        assert_eq!(parsed.renames, overrides.renames);
        assert_eq!(parsed.pages, overrides.pages);
        assert_eq!(parsed.added.len(), 1);
    }

    fn sample_artifact() -> TocArtifact {
        TocArtifact {
            source_kind: TocSourceKind::PdfOutline,
            confidence: 0.9,
            entries: vec![
                TocEntry {
                    source_title: "Chapter 1: Tash".to_string(),
                    translated_title: Some("第1章：塔什".to_string()),
                    level: 1,
                    order: 0,
                    page: Some(1),
                    href: Some("#ch1".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
                TocEntry {
                    source_title: "Chapter 1 Appendix".to_string(),
                    translated_title: Some("第1章 附录".to_string()),
                    level: 2,
                    order: 1,
                    page: Some(9),
                    href: Some("#ch1a".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
                TocEntry {
                    source_title: "Chapter 2: Reepicheep".to_string(),
                    translated_title: Some("第2章：雷佩奇普".to_string()),
                    level: 1,
                    order: 2,
                    page: Some(14),
                    href: Some("#ch2".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
            ],
        }
    }

    #[test]
    fn rename_overrides_translated_title() {
        let mut artifact = sample_artifact();
        let overrides = TocOverrides {
            renames: HashMap::from([(1, "附录（并入第1章）".to_string())]),
            removed: vec![],
            added: vec![],
            pages: HashMap::new(),
        };
        apply_toc_overrides(&mut artifact, &overrides);
        assert_eq!(artifact.entries[1].translated_title.as_deref(), Some("附录（并入第1章）"));
        assert_eq!(artifact.entries.len(), 3);
    }

    #[test]
    fn pages_override_updates_page_without_touching_titles() {
        // 校对页码编辑（set_toc_entry_page → overrides.pages）：
        // 只改目标条目 page，标题与其余条目不受影响；越界 order 静默忽略。
        let mut artifact = sample_artifact();
        let overrides = TocOverrides {
            renames: HashMap::new(),
            removed: vec![],
            added: vec![],
            pages: HashMap::from([(0usize, 42usize), (99usize, 7usize)]),
        };
        apply_toc_overrides(&mut artifact, &overrides);
        assert_eq!(artifact.entries[0].page, Some(42));
        assert_eq!(artifact.entries[0].translated_title.as_deref(), Some("第1章：塔什"));
        assert_eq!(artifact.entries[1].page, Some(9)); // 未被越界 order 波及
        assert_eq!(artifact.entries.len(), 3);
    }

    #[test]
    fn remove_merges_entry_out_of_outline_without_touching_others() {
        let mut artifact = sample_artifact();
        let overrides = TocOverrides {
            renames: HashMap::new(),
            removed: vec![1],
            added: vec![],
            pages: HashMap::new(),
        };
        apply_toc_overrides(&mut artifact, &overrides);
        assert_eq!(artifact.entries.len(), 2);
        assert_eq!(artifact.entries[1].source_title, "Chapter 2: Reepicheep");
        // order 连续重编号
        assert_eq!(artifact.entries[0].order, 0);
        assert_eq!(artifact.entries[1].order, 1);
    }

    #[test]
    fn add_inserts_after_anchor_and_renumbers() {
        let mut artifact = sample_artifact();
        let overrides = TocOverrides {
            renames: HashMap::new(),
            removed: vec![],
            added: vec![AddedTocEntry {
                id: "x1".to_string(),
                title: "新目录：第11章".to_string(),
                after_order: Some(0),
                level: 1,
                page: Some(11),
            }],
            pages: HashMap::new(),
        };
        apply_toc_overrides(&mut artifact, &overrides);
        assert_eq!(artifact.entries.len(), 4);
        assert_eq!(artifact.entries[1].translated_title.as_deref(), Some("新目录：第11章"));
        assert_eq!(artifact.entries[1].page, Some(11));
        assert_eq!(artifact.entries[3].order, 3);
    }

    #[test]
    fn epub_pseudo_pagination_preview_falls_back_from_missing_layouts() {
        // 无 source_layouts.json 的 EPUB：伪分页按「校对段对」口径过滤
        // （translate && 原文非空，与 collect_review_segments 一致），30 段对/页。
        let blocks: Vec<crate::document::SourceBlock> = (0..200)
            .map(|i| crate::document::SourceBlock {
                id: format!("b{i}"),
                order: i,
                kind: "para".to_string(),
                markdown: String::new(),
                // i%4==0 文本为空（被过滤）；i%2==1 不可翻译（被过滤）→ 段对 = i%4==2
                text: if i % 4 == 0 { String::new() } else { format!("para-{i}") },
                translate: i % 2 == 0,
                heading_path: vec![],
                chapter_title: None,
                href: None,
                page: None,
            })
            .collect();
        let review: Vec<&crate::document::SourceBlock> = blocks
            .iter()
            .filter(|b| b.translate && !b.text.trim().is_empty())
            .collect();
        assert_eq!(review.len(), 50); // i ∈ {2,6,...,198}
        let total = review.len().div_ceil(SOURCE_BLOCKS_PER_PAGE);
        assert_eq!(total, 2); // 50 段对 → 2 伪页
        // 第 2 伪页（page=2）取段对 30..49（末页不足 30 段对）：
        // review[k] = 块 4k+2 → 块 122 起；与命令实现一致用 skip/take 而非切片
        let start = (2usize - 1) * SOURCE_BLOCKS_PER_PAGE;
        let text: Vec<&str> = review
            .iter()
            .skip(start)
            .take(SOURCE_BLOCKS_PER_PAGE)
            .map(|b| b.text.trim())
            .collect();
        assert_eq!(text.len(), 20); // 50 - 30
        assert_eq!(text.first().copied(), Some("para-122"));
        assert!(text.iter().all(|t| !t.is_empty())); // 过滤口径生效：页内无空段
        // 截断护栏
        let long = "字".repeat(PAGE_PREVIEW_MAX_CHARS + 10);
        assert!(truncate_preview(&long).ends_with('…'));
        assert_eq!(truncate_preview("短文本"), "短文本");
    }

    #[test]
    fn parse_original_id_roundtrip() {
        assert_eq!(parse_original_id("o12"), Some(12));
        assert_eq!(parse_original_id("a-uuid"), None);
        assert_eq!(parse_original_id("oNaN"), None);
    }
}
