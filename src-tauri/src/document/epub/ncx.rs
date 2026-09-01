use crate::document::epub::archive::read_zip_entry_to_string;
use crate::document::epub::opf::locate_ncx_path;
use crate::document::epub::path::normalize_href_key;
use crate::error::AppError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs::File;
use zip::ZipArchive;

pub(super) fn parse_ncx_title_map(
    archive: &mut ZipArchive<File>,
    opf_path: &str,
) -> Result<HashMap<String, String>, AppError> {
    let Some(ncx_path) = locate_ncx_path(archive, opf_path)? else {
        return Ok(HashMap::new());
    };

    let ncx_content = read_zip_entry_to_string(archive, &ncx_path)?;
    let mut reader = Reader::from_str(&ncx_content);
    reader.config_mut().trim_text(true);

    let mut in_text = false;
    let mut current_label: Option<String> = None;
    let mut current_src: Option<String> = None;
    let mut mapping = HashMap::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) if event.name().as_ref() == b"text" => {
                in_text = true;
            }
            Ok(Event::End(event)) if event.name().as_ref() == b"text" => {
                in_text = false;
            }
            Ok(Event::Text(event)) if in_text => {
                current_label = Some(
                    event
                        .decode()
                        .map_err(|error| {
                            AppError::internal(format!("parse NCX text failed: {error}"))
                        })?
                        .to_string(),
                );
            }
            Ok(Event::Empty(event)) | Ok(Event::Start(event))
                if event.name().as_ref() == b"content" =>
            {
                current_src = parse_content_src(&reader, &event)?;
            }
            Ok(Event::End(event)) if event.name().as_ref() == b"navPoint" => {
                if let (Some(label), Some(src)) = (current_label.take(), current_src.take()) {
                    mapping.insert(normalize_href_key(&src), label.trim().to_string());
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!("parse NCX failed: {error}")));
            }
            _ => {}
        }
    }

    Ok(mapping)
}

fn parse_content_src(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Option<String>, AppError> {
    for attribute in event.attributes().flatten() {
        if attribute.key.as_ref() == b"src" {
            return Ok(Some(
                attribute
                    .decode_and_unescape_value(reader.decoder())
                    .map_err(|error| {
                        AppError::internal(format!("parse NCX content failed: {error}"))
                    })?
                    .to_string(),
            ));
        }
    }
    Ok(None)
}
