use crate::document::{TocArtifact, TocEntry, TocSourceKind};
use crate::error::AppError;
use lopdf::{Document, Error as LopdfError};
use std::path::Path;

pub(super) fn extract_pdf_outline(path: &Path) -> Result<Option<TocArtifact>, AppError> {
    validate_pdf_path(path)?;
    let document = Document::load(path)
        .map_err(|error| AppError::internal(format!("read PDF outline failed: {error}")))?;
    let toc = match document.get_toc() {
        Ok(toc) => toc,
        Err(LopdfError::NoOutline) => return Ok(None),
        Err(error) => {
            return Err(AppError::internal(format!(
                "parse PDF outline failed: {error}"
            )));
        }
    };
    let entries = toc
        .toc
        .into_iter()
        .enumerate()
        .filter_map(|(order, item)| pdf_outline_entry(order, item.level, item.title, item.page))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(TocArtifact {
        source_kind: TocSourceKind::PdfOutline,
        confidence: 0.9,
        entries,
    }))
}

fn validate_pdf_path(path: &Path) -> Result<(), AppError> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("pdf") => Ok(()),
        _ => Err(AppError::invalid_input(
            "PDF outline extraction requires a PDF path",
        )),
    }
}

fn pdf_outline_entry(order: usize, level: usize, title: String, page: usize) -> Option<TocEntry> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    Some(TocEntry {
        source_title: title,
        translated_title: None,
        level: level.clamp(1, 6) as u8,
        order,
        page: Some(page),
        href: None,
        source_kind: TocSourceKind::PdfOutline,
        confidence: 0.9,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn pdf_outline_entry_preserves_title_level_and_page() {
        let entry =
            pdf_outline_entry(3, 2, "  Chapter   One  ".to_string(), 5).expect("outline entry");

        assert_eq!(entry.source_title, "Chapter One");
        assert_eq!(entry.level, 2);
        assert_eq!(entry.order, 3);
        assert_eq!(entry.page, Some(5));
        assert_eq!(entry.href, None);
        assert_eq!(entry.source_kind, TocSourceKind::PdfOutline);
    }

    #[test]
    fn pdf_outline_stub_rejects_non_pdf_paths() {
        let error = extract_pdf_outline(Path::new("sample.epub")).expect_err("non-pdf rejected");

        assert!(error.message.contains("PDF outline"));
    }

    #[test]
    fn extracts_pdf_outline_from_env_path() {
        let Ok(path) = std::env::var("MUSETRANSLATE_PDF_OUTLINE_TEST_PATH") else {
            return;
        };

        let outline = extract_pdf_outline(Path::new(&path)).expect("PDF outline should parse");

        if let Some(toc) = outline {
            assert_eq!(toc.source_kind, TocSourceKind::PdfOutline);
            assert!(!toc.entries.is_empty());
            assert!(toc.entries.iter().all(|entry| entry.href.is_none()));
        }
    }
}
