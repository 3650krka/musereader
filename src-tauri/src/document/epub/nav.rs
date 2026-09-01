use crate::document::epub::archive::read_zip_entry_to_string;
use crate::document::epub::path::{join_epub_path, normalize_href_key, parent_dir};
use crate::error::AppError;
use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use zip::ZipArchive;

pub(super) fn parse_nav_title_map(
    archive: &mut ZipArchive<File>,
    nav_path: Option<&str>,
) -> Result<HashMap<String, String>, AppError> {
    let Some(nav_path) = nav_path else {
        return Ok(HashMap::new());
    };
    let nav_content = read_zip_entry_to_string(archive, nav_path)?;
    let nav_toc = extract_nav_toc_section(&nav_content).unwrap_or(nav_content.as_str());
    parse_nav_links(nav_toc, &parent_dir(nav_path))
}

fn extract_nav_toc_section(raw: &str) -> Option<&str> {
    let nav_re = Regex::new(r#"(?is)<nav\b[^>]*(?:epub:type|role)\s*=\s*["'][^"']*(?:toc|doc-toc)[^"']*["'][^>]*>(.*?)</nav>"#).ok()?;
    nav_re
        .captures(raw)
        .and_then(|captures| captures.get(1).map(|matched| matched.as_str()))
}

fn parse_nav_links(raw: &str, nav_base_dir: &str) -> Result<HashMap<String, String>, AppError> {
    let link_re = Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#)
        .map_err(|error| AppError::internal(format!("parse EPUB nav links failed: {error}")))?;
    let tag_re = Regex::new("(?is)<[^>]+>")
        .map_err(|error| AppError::internal(format!("parse EPUB nav text failed: {error}")))?;
    let mut mapping = HashMap::new();
    for captures in link_re.captures_iter(raw) {
        let href = captures
            .get(1)
            .map(|matched| matched.as_str())
            .unwrap_or("");
        let label = captures
            .get(2)
            .map(|matched| matched.as_str())
            .unwrap_or("");
        let title = normalize_nav_label(&tag_re.replace_all(label, " "));
        if !href.trim().is_empty() && !title.is_empty() {
            let joined_href = join_epub_path(nav_base_dir, href);
            mapping.insert(normalize_href_key(&joined_href), title);
        }
    }
    Ok(mapping)
}

fn normalize_nav_label(raw: &str) -> String {
    html_entity_decode(raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn html_entity_decode(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nav_toc_links() {
        let raw = r#"
            <html><body>
              <nav epub:type="toc">
                <ol>
                  <li><a href="chapters/one.xhtml#top">Chapter&nbsp;One</a></li>
                  <li><a href="chapters/two.xhtml"><span>Chapter Two</span></a></li>
                </ol>
              </nav>
            </body></html>
        "#;

        let section = extract_nav_toc_section(raw).expect("toc section");
        let mapping = parse_nav_links(section, "OEBPS").expect("nav links");

        assert_eq!(
            mapping.get("OEBPS/chapters/one.xhtml").map(String::as_str),
            Some("Chapter One")
        );
        assert_eq!(
            mapping.get("OEBPS/chapters/two.xhtml").map(String::as_str),
            Some("Chapter Two")
        );
    }

    #[test]
    fn parses_nav_links_relative_to_nav_file() {
        let raw = r#"<nav epub:type="toc"><a href="../Text/chapter.xhtml#top">Chapter</a></nav>"#;
        let section = extract_nav_toc_section(raw).expect("toc section");
        let mapping = parse_nav_links(section, "OEBPS/nav").expect("nav links");

        assert_eq!(
            mapping.get("OEBPS/Text/chapter.xhtml").map(String::as_str),
            Some("Chapter")
        );
    }
}
