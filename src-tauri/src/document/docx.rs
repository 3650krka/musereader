//! DOCX (Office Open XML) 输入提取。
//!
//! 仅依赖 zip + quick-xml：读取 word/document.xml，按 <w:p> 段落提取文本，
//! 依据 pStyle（经 word/styles.xml 的 styleId→名称 解析）映射标题层级
//! （Heading1-6 / 标题1-6 → #…######），列表段落（numPr）前缀 "- "，
//! 表格按行展开为段落流。图片与复杂版式不提取（学术翻译以文本为主）。

use crate::error::AppError;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

/// 从 docx 提取 markdown 文本流。
pub fn extract_docx_markdown(path: &Path) -> Result<String, AppError> {
    let file = File::open(path)
        .map_err(|error| AppError::internal(format!("open docx failed: {error}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| AppError::invalid_input(format!("not a valid docx (zip): {error}")))?;
    let style_names = archive
        .by_name("word/styles.xml")
        .ok()
        .and_then(|mut entry| {
            let mut buf = String::new();
            entry.read_to_string(&mut buf).ok()?;
            Some(parse_styles_xml(&buf))
        })
        .unwrap_or_default();
    let xml = read_document_xml(&mut archive)?;
    let markdown = document_xml_to_markdown(&xml, &style_names)?;
    if markdown.trim().is_empty() {
        return Err(AppError::invalid_input(
            "DOCX 中未提取到文本内容（可能为纯图片文档）",
        ));
    }
    Ok(markdown)
}

fn read_document_xml(archive: &mut ZipArchive<File>) -> Result<String, AppError> {
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::invalid_input("DOCX 缺少 word/document.xml"))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|error| AppError::internal(format!("read word/document.xml failed: {error}")))?;
    Ok(buf)
}

struct Paragraph {
    text: String,
    style: Option<String>,
    is_list: bool,
}

fn document_xml_to_markdown(
    xml: &str,
    style_names: &HashMap<String, String>,
) -> Result<String, AppError> {
    let mut reader = Reader::from_str(xml);
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut current: Option<Paragraph> = None;
    let mut in_text = false;
    let mut in_paragraph_properties = false;
    let mut in_deleted = false; // w:del 修订删除内容不提取

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let name = element.name();
                let local = local_name(name.as_ref());
                match local {
                    b"p" => {
                        current = Some(Paragraph {
                            text: String::new(),
                            style: None,
                            is_list: false,
                        });
                    }
                    b"pPr" => in_paragraph_properties = true,
                    b"pStyle" if in_paragraph_properties => {
                        if let Some(paragraph) = current.as_mut() {
                            let style_id = attr_value(&element, b"w:val")
                                .or_else(|| attr_value(&element, b"val"));
                            // pStyle 引用 styleId（常为 "1"/"2"），需经样式表解析为名称。
                            paragraph.style =
                                style_id.map(|id| style_names.get(&id).cloned().unwrap_or(id));
                        }
                    }
                    b"numPr" if in_paragraph_properties => {
                        if let Some(paragraph) = current.as_mut() {
                            paragraph.is_list = true;
                        }
                    }
                    b"t" => in_text = true,
                    b"tab" | b"br" | b"cr" => {
                        if let Some(paragraph) = current.as_mut() {
                            paragraph.text.push(' ');
                        }
                    }
                    b"del" => in_deleted = true,
                    _ => {}
                }
            }
            Ok(Event::End(element)) => {
                let name = element.name();
                let local = local_name(name.as_ref());
                match local {
                    b"p" => {
                        if let Some(paragraph) = current.take() {
                            paragraphs.push(paragraph);
                        }
                    }
                    b"pPr" => in_paragraph_properties = false,
                    b"t" => in_text = false,
                    b"del" => in_deleted = false,
                    _ => {}
                }
            }
            Ok(Event::Empty(element)) => {
                // pStyle/numPr/tab/br 均为 empty-element（<w:pStyle w:val="..."/>）。
                let name = element.name();
                let local = local_name(name.as_ref());
                match local {
                    b"pStyle" if in_paragraph_properties => {
                        if let Some(paragraph) = current.as_mut() {
                            let style_id = attr_value(&element, b"w:val")
                                .or_else(|| attr_value(&element, b"val"));
                            // pStyle 引用 styleId（常为 "1"/"2"），需经样式表解析为名称。
                            paragraph.style =
                                style_id.map(|id| style_names.get(&id).cloned().unwrap_or(id));
                        }
                    }
                    b"numPr" if in_paragraph_properties => {
                        if let Some(paragraph) = current.as_mut() {
                            paragraph.is_list = true;
                        }
                    }
                    b"tab" | b"br" | b"cr" => {
                        if let Some(paragraph) = current.as_mut() {
                            paragraph.text.push(' ');
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(text)) => {
                if in_text && !in_deleted {
                    if let Some(paragraph) = current.as_mut() {
                        let decoded = text.decode().map_err(|error| {
                            AppError::internal(format!("docx text decode failed: {error}"))
                        })?;
                        paragraph.text.push_str(&decoded);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::internal(format!(
                    "parse docx document.xml failed: {error}"
                )));
            }
            _ => {}
        }
    }

    Ok(paragraphs_to_markdown(&paragraphs))
}

/// 解析 word/styles.xml：styleId → 样式名称（如 "1" → "heading 1"）。
fn parse_styles_xml(xml: &str) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let mut reader = Reader::from_str(xml);
    let mut current_style_id: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let name = element.name();
                let local = local_name(name.as_ref());
                match local {
                    b"style" => {
                        current_style_id = attr_value(&element, b"w:styleId")
                            .or_else(|| attr_value(&element, b"styleId"));
                    }
                    b"name" => {
                        if let Some(id) = current_style_id.clone() {
                            if let Some(value) = attr_value(&element, b"w:val")
                                .or_else(|| attr_value(&element, b"val"))
                            {
                                names.insert(id, value);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(element)) => {
                // <w:name w:val="heading 1"/> 常以 empty-element 出现。
                let name = element.name();
                let local = local_name(name.as_ref());
                if local == b"name" {
                    if let Some(id) = current_style_id.clone() {
                        if let Some(value) =
                            attr_value(&element, b"w:val").or_else(|| attr_value(&element, b"val"))
                        {
                            names.insert(id, value);
                        }
                    }
                }
            }
            Ok(Event::End(element)) => {
                if local_name(element.name().as_ref()) == b"style" {
                    current_style_id = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    names
}

fn paragraphs_to_markdown(paragraphs: &[Paragraph]) -> String {
    let mut out = String::new();
    let mut previous_blank = true; // 起始视为空行，避免首行多余换行
    for paragraph in paragraphs {
        let text = paragraph.text.trim();
        if text.is_empty() {
            if !previous_blank {
                out.push('\n');
                previous_blank = true;
            }
            continue;
        }
        if !previous_blank {
            out.push('\n');
        }
        let heading_level = paragraph.style.as_deref().and_then(parse_heading_level);
        if let Some(level) = heading_level {
            for _ in 0..level.min(6) {
                out.push('#');
            }
            out.push(' ');
            out.push_str(text);
        } else if paragraph.is_list {
            out.push_str("- ");
            out.push_str(text);
        } else {
            out.push_str(text);
        }
        out.push('\n');
        previous_blank = false;
    }
    out
}

/// Heading1/heading 1/标题 1/Title 等样式 → 标题层级（1-6）。
fn parse_heading_level(style: &str) -> Option<usize> {
    let normalized = style.trim().to_ascii_lowercase().replace(' ', "");
    for level in 1..=6usize {
        if normalized == format!("heading{level}")
            || normalized == format!("标题{level}")
            || normalized == format!("heading{level}char")
        {
            return Some(level);
        }
    }
    if normalized == "title" || normalized == "标题" {
        return Some(1);
    }
    if normalized == "subtitle" || normalized == "副标题" {
        return Some(2);
    }
    None
}

fn local_name(qualified: &[u8]) -> &[u8] {
    qualified
        .iter()
        .rposition(|byte| *byte == b':')
        .map(|pos| &qualified[pos + 1..])
        .unwrap_or(qualified)
}

fn attr_value(element: &quick_xml::events::BytesStart, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .filter_map(Result::ok)
        // 命名空间无关匹配：w:val 与 val 都命中（与 local_name 同策略）。
        .find(|attr| local_name(attr.key.as_ref()) == name || attr.key.as_ref() == name)
        .and_then(|attr| String::from_utf8(attr.value.to_vec()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_styles() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn parses_headings_paragraphs_and_lists() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Introduction</w:t></w:r></w:p>
    <w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>world</w:t></w:r></w:p>
    <w:p><w:pPr><w:pStyle w:val="标题2"/></w:pPr><w:r><w:t>背景</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>item one</w:t></w:r></w:p>
    <w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Big Title</w:t></w:r></w:p>
  </w:body>
</w:document>"#;

        let markdown =
            document_xml_to_markdown(xml, &empty_styles()).expect("parse should succeed");
        assert!(markdown.contains("# Introduction"), "markdown: {markdown}");
        assert!(markdown.contains("Hello world"), "markdown: {markdown}");
        assert!(markdown.contains("## 背景"), "markdown: {markdown}");
        assert!(markdown.contains("- item one"), "markdown: {markdown}");
        assert!(markdown.contains("# Big Title"), "markdown: {markdown}");
    }

    #[test]
    fn style_id_is_resolved_through_styles_table() {
        let styles_xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:styleId="1"><w:name w:val="heading 1"/></w:style>
  <w:style w:styleId="2"><w:name w:val="heading 2"/></w:style>
</w:styles>"#;
        let styles = parse_styles_xml(styles_xml);
        assert_eq!(styles.get("1").map(String::as_str), Some("heading 1"));
        assert_eq!(styles.get("2").map(String::as_str), Some("heading 2"));

        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="1"/></w:pPr><w:r><w:t>Chapter One</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let markdown = document_xml_to_markdown(xml, &styles).expect("parse should succeed");
        assert!(markdown.contains("# Chapter One"), "markdown: {markdown}");
    }

    #[test]
    fn skips_deleted_revision_text_and_empty_paragraphs() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>kept</w:t></w:r><w:del><w:r><w:delText>removed</w:delText></w:r></w:del></w:p>
    <w:p><w:r><w:t>   </w:t></w:r></w:p>
    <w:p><w:r><w:t>next</w:t></w:r></w:p>
  </w:body>
</w:document>"#;

        let markdown =
            document_xml_to_markdown(xml, &empty_styles()).expect("parse should succeed");
        assert!(markdown.contains("kept"));
        assert!(!markdown.contains("removed"));
        assert!(markdown.contains("kept\n\nnext"), "markdown: {markdown:?}");
    }

    #[test]
    fn namespace_agnostic_local_names() {
        assert_eq!(local_name(b"w:p"), b"p");
        assert_eq!(local_name(b"p"), b"p");
        assert_eq!(local_name(b"w14:p"), b"p");
    }

    #[test]
    fn heading_style_variants_map_to_levels() {
        assert_eq!(parse_heading_level("Heading1"), Some(1));
        assert_eq!(parse_heading_level("heading 3"), Some(3));
        assert_eq!(parse_heading_level("标题4"), Some(4));
        assert_eq!(parse_heading_level("Title"), Some(1));
        assert_eq!(parse_heading_level("Normal"), None);
    }
}
