use super::super::{load_single_json, AppError};
use crate::document::{TocArtifact, TocSourceKind};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderTocItem {
    pub title: String,
    pub level: u8,
    pub order: usize,
    pub href: Option<String>,
    pub progress: u8,
    pub source_title: Option<String>,
}

#[tauri::command]
pub async fn read_reader_toc(
    translated_toc_path: Option<String>,
    source_toc_path: Option<String>,
) -> Result<Vec<ReaderTocItem>, AppError> {
    let translated = translated_toc_path
        .as_deref()
        .and_then(read_toc_artifact)
        .map(drop_spine_fallback_entries)
        .map(|mut toc| {
        apply_overrides_for(&translated_toc_path, &mut toc);
        toc
    });
    let source = source_toc_path
        .as_deref()
        .and_then(read_toc_artifact)
        .map(drop_spine_fallback_entries)
        .map(|mut toc| {
        apply_overrides_for(&source_toc_path, &mut toc);
        toc
    });
    if translated.is_none() && source.is_none() {
        return Err(AppError::not_found("未找到目录工件"));
    }
    Ok(build_reader_toc_items(
        translated.as_ref(),
        source
            .as_ref()
            .or(translated.as_ref())
            .expect("toc presence checked"),
    ))
}

/// 目录编辑覆盖层（toc_overrides.json）叠加。
/// task_id 从 toc 工件路径（artifacts/<task_id>/xxx.json）的父目录名推导。
fn apply_overrides_for(toc_path: &Option<String>, artifact: &mut crate::document::TocArtifact) {
    let Some(path) = toc_path.as_deref() else { return };
    let Some(task_id) = std::path::Path::new(path).parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) else {
        return;
    };
    let Ok(overrides) = crate::commands::toc_edit::load_overrides(task_id) else {
        return;
    };
    crate::commands::toc_edit::apply_toc_overrides(artifact, &overrides);
}

fn read_toc_artifact(path: &str) -> Option<TocArtifact> {
    load_single_json::<TocArtifact>(Path::new(path))
}

/// 剔除 "Chapter N" spine 兜底占位条目（与产物构建同源的过滤）：
/// 历史工件（剔除逻辑上线前生成）里残留的占位条目也一并清掉——
/// 旧书不重翻也能干净显示。在覆盖层叠加之前执行，保证 removed/renames
/// 键与当前条目序号对齐。
fn drop_spine_fallback_entries(mut artifact: TocArtifact) -> TocArtifact {
    artifact
        .entries
        .retain(|entry| entry.source_kind != TocSourceKind::EpubSpineFallback);
    for (index, entry) in artifact.entries.iter_mut().enumerate() {
        entry.order = index;
    }
    artifact
}

fn build_reader_toc_items(
    translated_toc: Option<&TocArtifact>,
    fallback_toc: &TocArtifact,
) -> Vec<ReaderTocItem> {
    let total = fallback_toc.entries.len().max(1);
    fallback_toc
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, fallback_entry)| {
            let translated_entry = translated_toc.and_then(|toc| toc.entries.get(index));
            let title = translated_entry
                .and_then(|entry| entry.translated_title.as_deref())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&fallback_entry.source_title)
                .trim()
                .to_string();
            if title.is_empty() {
                return None;
            }
            let source_title = (fallback_entry.source_title.trim() != title)
                .then(|| fallback_entry.source_title.clone());
            let href = translated_entry
                .and_then(|entry| entry.href.clone())
                .or_else(|| fallback_entry.href.clone());
            let progress = (((index + 1) * 100) / total).clamp(1, 100) as u8;
            Some(ReaderTocItem {
                title,
                level: fallback_entry.level.max(1),
                order: fallback_entry.order,
                href,
                progress,
                source_title,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{TocEntry, TocSourceKind};

    #[test]
    fn prefers_translated_titles_without_changing_outline_order() {
        let source = TocArtifact {
            source_kind: TocSourceKind::PdfOutline,
            confidence: 0.9,
            entries: vec![
                TocEntry {
                    source_title: "Abstract".to_string(),
                    translated_title: None,
                    level: 1,
                    order: 0,
                    page: None,
                    href: Some("#abstract".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
                TocEntry {
                    source_title: "Method".to_string(),
                    translated_title: None,
                    level: 2,
                    order: 1,
                    page: None,
                    href: Some("#method".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
            ],
        };
        let translated = TocArtifact {
            source_kind: TocSourceKind::PdfOutline,
            confidence: 0.9,
            entries: vec![
                TocEntry {
                    source_title: "Abstract".to_string(),
                    translated_title: Some("摘要".to_string()),
                    level: 1,
                    order: 0,
                    page: None,
                    href: Some("#摘要".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
                TocEntry {
                    source_title: "Method".to_string(),
                    translated_title: Some("方法".to_string()),
                    level: 2,
                    order: 1,
                    page: None,
                    href: Some("#方法".to_string()),
                    source_kind: TocSourceKind::PdfOutline,
                    confidence: 0.9,
                },
            ],
        };

        let items = build_reader_toc_items(Some(&translated), &source);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "摘要");
        assert_eq!(items[1].title, "方法");
        assert_eq!(items[0].source_title.as_deref(), Some("Abstract"));
        assert_eq!(items[1].href.as_deref(), Some("#方法"));
    }
}
