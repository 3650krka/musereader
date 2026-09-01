use crate::document::TocSourceKind;
use crate::error::AppError;
use std::collections::HashMap;
use std::path::Path;

mod archive;
mod nav;
mod ncx;
mod opf;
pub mod pack;
mod parse;
pub(crate) mod path;
pub(crate) mod reader;

#[derive(Debug, Clone)]
pub struct EpubChapter {
    pub title: String,
    pub markdown: String,
    pub order: usize,
    pub href: Option<String>,
    pub title_source: TocSourceKind,
    pub blocks: Vec<super::EpubBlock>,
}

pub(super) struct ExtractedEpub {
    pub(super) chapters: Vec<EpubChapter>,
    pub(super) cover_data_url: Option<String>,
    pub(super) markdown_images: HashMap<String, String>,
}

/// 网页抓取复用：结构性 boilerplate 剥离（commands/web_capture 用）。
/// 与 EPUB 管线同一算法，只是调用点不同。
pub fn strip_web_boilerplate_for_capture(html: &str) -> String {
    parse::strip_web_boilerplate_pub(html)
}

pub(super) fn extract_epub_contents(path: &Path) -> Result<ExtractedEpub, AppError> {
    let mut archive = archive::open_epub_archive(path)?;
    let opf_path = opf::locate_opf_path(&mut archive)?;
    let opf_content = archive::read_zip_entry_to_string(&mut archive, &opf_path)?;
    let (content_paths, cover_href, nav_href, image_hrefs) =
        opf::parse_opf_manifest_and_spine(&opf_content)?;
    let title_map = ncx::parse_ncx_title_map(&mut archive, &opf_path).unwrap_or_default();
    let nav_path = nav_href.map(|href| path::join_epub_path(&path::parent_dir(&opf_path), &href));
    let nav_title_map =
        nav::parse_nav_title_map(&mut archive, nav_path.as_deref()).unwrap_or_default();
    let base_dir = path::parent_dir(&opf_path);
    let chapters = parse::extract_spine_chapters(
        &mut archive,
        &base_dir,
        content_paths,
        &title_map,
        &nav_title_map,
    )?;

    if chapters.is_empty() {
        return Err(AppError::internal("EPUB produced no readable chapters"));
    }

    let cover_data_url = archive::extract_epub_cover(&mut archive, &base_dir, cover_href);
    let markdown_images = archive::extract_epub_images(&mut archive, &base_dir, image_hrefs);

    Ok(ExtractedEpub {
        chapters,
        cover_data_url,
        markdown_images,
    })
}

pub(super) fn artifact_image_path(epub_relative_path: &str) -> String {
    format!("imgs/epub/{}", path::normalize_href_key(epub_relative_path))
}
