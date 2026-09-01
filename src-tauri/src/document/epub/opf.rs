use crate::document::epub::archive::read_zip_entry_to_string;
use crate::document::epub::path::{join_epub_path, parent_dir};
use crate::error::AppError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::fs::File;
use zip::ZipArchive;

struct ManifestEntry {
    id: String,
    href: String,
    media_type: String,
    properties: String,
}

pub(super) fn locate_opf_path(archive: &mut ZipArchive<File>) -> Result<String, AppError> {
    let container = read_zip_entry_to_string(archive, "META-INF/container.xml")?;
    let mut reader = Reader::from_str(&container);
    reader.config_mut().trim_text(true);
    let mut opf_path = None;

    loop {
        match reader.read_event() {
            Ok(Event::Empty(event)) | Ok(Event::Start(event)) => {
                if event.name().as_ref() == b"rootfile" {
                    opf_path = decode_rootfile_path(&reader, &event)?;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!(
                    "parse container.xml failed: {error}"
                )));
            }
            _ => {}
        }
    }

    opf_path.ok_or_else(|| AppError::internal("EPUB missing OPF path"))
}

pub(super) fn parse_opf_manifest_and_spine(
    opf_content: &str,
) -> Result<(Vec<String>, Option<String>, Option<String>, Vec<String>), AppError> {
    let mut reader = Reader::from_str(opf_content);
    reader.config_mut().trim_text(true);

    let mut manifest = Vec::new();
    let mut spine = Vec::new();
    let mut cover_id = None;
    let mut nav_href = None;

    loop {
        match reader.read_event() {
            Ok(Event::Empty(event)) | Ok(Event::Start(event)) => match event.name().as_ref() {
                b"item" => {
                    let (entry, item_cover_id) = parse_manifest_item(&reader, &event)?;
                    if item_cover_id.is_some() {
                        cover_id = item_cover_id;
                    }
                    if is_nav_item(&entry) {
                        nav_href = Some(entry.href.clone());
                    }
                    manifest.push(entry);
                }
                b"meta" => {
                    if let Some(meta_cover_id) = parse_cover_meta(&reader, &event)? {
                        cover_id = Some(meta_cover_id);
                    }
                }
                b"itemref" => {
                    if let Some(idref) = parse_itemref_idref(&reader, &event)? {
                        spine.push(idref);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!(
                    "parse content.opf failed: {error}"
                )));
            }
            _ => {}
        }
    }

    Ok((
        collect_content_paths(&manifest, spine),
        resolve_cover_href(&manifest, cover_id),
        nav_href,
        collect_image_hrefs(&manifest),
    ))
}

pub(super) fn locate_ncx_path(
    archive: &mut ZipArchive<File>,
    opf_path: &str,
) -> Result<Option<String>, AppError> {
    let opf_content = read_zip_entry_to_string(archive, opf_path)?;
    let mut reader = Reader::from_str(&opf_content);
    reader.config_mut().trim_text(true);

    loop {
        match reader.read_event() {
            Ok(Event::Empty(event)) | Ok(Event::Start(event))
                if event.name().as_ref() == b"item" =>
            {
                if let Some(href) = parse_ncx_manifest_href(&reader, &event)? {
                    return Ok(Some(join_epub_path(&parent_dir(opf_path), &href)));
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!("parse OPF NCX failed: {error}")));
            }
            _ => {}
        }
    }

    Ok(None)
}

fn decode_rootfile_path(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    for attribute in event.attributes().flatten() {
        if attribute.key.as_ref() == b"full-path" {
            return Ok(Some(
                attribute
                    .decode_and_unescape_value(reader.decoder())
                    .map_err(|error| {
                        AppError::internal(format!("parse EPUB root path failed: {error}"))
                    })?
                    .to_string(),
            ));
        }
    }
    Ok(None)
}

fn parse_manifest_item(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<(ManifestEntry, Option<String>), AppError> {
    let mut id = String::new();
    let mut href = String::new();
    let mut media_type = String::new();
    let mut properties = String::new();

    for attribute in event.attributes().flatten() {
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|error| {
                AppError::internal(format!("parse OPF item attribute failed: {error}"))
            })?
            .to_string();
        match attribute.key.as_ref() {
            b"id" => id = value,
            b"href" => href = value,
            b"media-type" => media_type = value,
            b"properties" => properties = value,
            _ => {}
        }
    }

    let cover_id = properties.contains("cover-image").then(|| id.clone());
    Ok((
        ManifestEntry {
            id,
            href,
            media_type,
            properties,
        },
        cover_id,
    ))
}

fn is_nav_item(entry: &ManifestEntry) -> bool {
    entry
        .properties
        .split_whitespace()
        .any(|prop| prop == "nav")
        && entry.media_type.contains("html")
        && !entry.href.is_empty()
}

fn parse_cover_meta(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    let mut meta_name = String::new();
    let mut content = String::new();
    for attribute in event.attributes().flatten() {
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|error| AppError::internal(format!("parse OPF meta failed: {error}")))?
            .to_string();
        match attribute.key.as_ref() {
            b"name" => meta_name = value,
            b"content" => content = value,
            _ => {}
        }
    }

    Ok((meta_name == "cover" && !content.is_empty()).then_some(content))
}

fn parse_itemref_idref(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    for attribute in event.attributes().flatten() {
        if attribute.key.as_ref() == b"idref" {
            return Ok(Some(
                attribute
                    .decode_and_unescape_value(reader.decoder())
                    .map_err(|error| {
                        AppError::internal(format!("parse OPF spine failed: {error}"))
                    })?
                    .to_string(),
            ));
        }
    }
    Ok(None)
}

fn collect_content_paths(manifest: &[ManifestEntry], spine: Vec<String>) -> Vec<String> {
    let mut content_paths = Vec::new();
    for idref in spine {
        if let Some(entry) = manifest.iter().find(|entry| entry.id == idref) {
            if entry.media_type.contains("html") || entry.media_type.contains("xhtml") {
                content_paths.push(entry.href.clone());
            }
        }
    }
    content_paths
}

fn collect_image_hrefs(manifest: &[ManifestEntry]) -> Vec<String> {
    manifest
        .iter()
        .filter(|entry| entry.media_type.starts_with("image/") && !entry.href.is_empty())
        .map(|entry| entry.href.clone())
        .collect()
}

fn resolve_cover_href(manifest: &[ManifestEntry], cover_id: Option<String>) -> Option<String> {
    cover_id.and_then(|target_id| {
        manifest
            .iter()
            .find(|entry| entry.id == target_id)
            .map(|entry| entry.href.clone())
    })
}

fn parse_ncx_manifest_href(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    let mut media_type = String::new();
    let mut href = String::new();
    for attribute in event.attributes().flatten() {
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|error| AppError::internal(format!("parse OPF NCX entry failed: {error}")))?
            .to_string();
        match attribute.key.as_ref() {
            b"media-type" => media_type = value,
            b"href" => href = value,
            _ => {}
        }
    }

    Ok((media_type == "application/x-dtbncx+xml" && !href.is_empty()).then_some(href))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opf_manifest_extracts_spine_cover_and_nav_href() {
        let opf = r#"
            <package>
              <manifest>
                <item id="nav" href="nav/nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="cover" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
                <item id="chapter" href="Text/chapter.xhtml" media-type="application/xhtml+xml"/>
              </manifest>
              <spine>
                <itemref idref="chapter"/>
              </spine>
            </package>
        "#;

        let (content_paths, cover_href, nav_href, image_hrefs) =
            parse_opf_manifest_and_spine(opf).expect("opf parse");

        assert_eq!(content_paths, vec!["Text/chapter.xhtml"]);
        assert_eq!(cover_href.as_deref(), Some("images/cover.jpg"));
        assert_eq!(nav_href.as_deref(), Some("nav/nav.xhtml"));
        assert_eq!(image_hrefs, vec!["images/cover.jpg"]);
    }
}
