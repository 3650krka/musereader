use super::sanitize::sanitize_reader_html;
use super::{prepare_epub_reader_package_with_overrides, search_epub_reader};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[test]
fn sanitizer_removes_active_content_and_injects_csp() {
    let input = r#"<?xml version="1.0" encoding="iso-8859-1"?><html><head><base href="https://example.test/"><meta http-equiv="refresh" content="0;url=https://example.test"><script>alert(1)</script></head><body><a href="javascript:alert(1)">Link</a><p>Keep me</p></body></html>"#;

    let output = sanitize_reader_html(input).expect("sanitize reader HTML");

    assert!(output.contains("Content-Security-Policy"));
    assert!(output.contains("encoding=\"utf-8\""));
    assert!(output.contains("Keep me"));
    assert!(!output.contains("<script"));
    assert!(!output.contains("<base"));
    assert!(!output
        .to_ascii_lowercase()
        .contains("http-equiv=\"refresh\""));
    assert!(!output.contains("javascript:"));
    assert!(!output.contains("onclick="));
}

#[test]
fn package_preserves_spine_order_and_supports_search() {
    let root = temporary_directory("reader-package");
    let epub_path = root.join("sample.epub");
    write_sample_epub(&epub_path);
    let cache_root = root.join("cache");

    let package = prepare_epub_reader_package_with_overrides(&epub_path, &cache_root, &[])
        .expect("reader package should be prepared");
    let results =
        search_epub_reader(&epub_path, "second needle", 10).expect("reader search should succeed");

    assert_eq!(package.sections.len(), 2);
    assert_eq!(package.sections[0].title, "First");
    assert_eq!(package.sections[1].title, "Second");
    assert!(package
        .sections
        .iter()
        .all(|section| Path::new(&section.file_path).is_file()));
    assert!(package.has_bilingual_markup);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Second");
    assert_eq!(results[0].occurrences, 1);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn toc_overrides_rename_sections_by_href() {
    // 目录覆盖层（toc_overrides 叠加）：改名条目按 href 命中 section，
    // 阅读器目录与目录管理显示一致；未命中条目保持原解析标题。
    let root = temporary_directory("reader-toc-overrides");
    let epub_path = root.join("sample.epub");
    write_sample_epub(&epub_path);

    let package = prepare_epub_reader_package_with_overrides(
        &epub_path,
        &root.join("cache"),
        &[
            ("OPS/first.xhtml".to_string(), "第一章：塔什".to_string()),
            ("OPS/second.xhtml#frag".to_string(), "插页".to_string()),
        ],
    )
    .expect("reader package with overrides");

    assert_eq!(package.sections[0].title, "第一章：塔什");
    assert_eq!(package.sections[1].title, "插页");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cache_rejects_zip_slip_paths() {    let root = temporary_directory("reader-zip-slip");
    let epub_path = root.join("unsafe.epub");
    let file = File::create(&epub_path).expect("create unsafe EPUB");
    let mut writer = ZipWriter::new(file);
    write_entry(&mut writer, "META-INF/container.xml", CONTAINER_XML);
    write_entry(&mut writer, "OPS/content.opf", OPF_XML);
    write_entry(&mut writer, "OPS/nav.xhtml", NAV_XHTML);
    write_entry(&mut writer, "OPS/first.xhtml", FIRST_XHTML);
    write_entry(&mut writer, "OPS/second.xhtml", SECOND_XHTML);
    writer
        .start_file("../escape.xhtml", SimpleFileOptions::default())
        .expect("start unsafe entry");
    writer
        .write_all(b"<html></html>")
        .expect("write unsafe entry");
    writer.finish().expect("finish unsafe EPUB");

    let error = prepare_epub_reader_package_with_overrides(&epub_path, &root.join("cache"), &[])
        .expect_err("unsafe path should fail");

    assert!(matches!(error.code, "INVALID_INPUT" | "INTERNAL"));
    assert!(!root.join("escape.xhtml").exists());
    let _ = fs::remove_dir_all(root);
}

fn temporary_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("musetranslate-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create test directory");
    path
}

fn write_sample_epub(path: &Path) {
    let file = File::create(path).expect("create sample EPUB");
    let mut writer = ZipWriter::new(file);
    write_entry(&mut writer, "META-INF/container.xml", CONTAINER_XML);
    write_entry(&mut writer, "OPS/content.opf", OPF_XML);
    write_entry(&mut writer, "OPS/nav.xhtml", NAV_XHTML);
    write_entry(&mut writer, "OPS/first.xhtml", FIRST_XHTML);
    write_entry(&mut writer, "OPS/second.xhtml", SECOND_XHTML);
    writer.finish().expect("finish sample EPUB");
}

fn write_entry(writer: &mut ZipWriter<File>, name: &str, content: &str) {
    writer
        .start_file(name, SimpleFileOptions::default())
        .expect("start EPUB entry");
    writer
        .write_all(content.as_bytes())
        .expect("write EPUB entry");
}

const CONTAINER_XML: &str = r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OPS/content.opf" /></rootfiles></container>"#;
const OPF_XML: &str = r#"<package><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="first" href="first.xhtml" media-type="application/xhtml+xml"/><item id="second" href="second.xhtml" media-type="application/xhtml+xml"/></manifest><spine page-progression-direction="ltr"><itemref idref="first"/><itemref idref="second"/></spine></package>"#;
const NAV_XHTML: &str = r#"<html><body><nav epub:type="toc"><ol><li><a href="first.xhtml">First</a></li><li><a href="second.xhtml">Second</a></li></ol></nav></body></html>"#;
const FIRST_XHTML: &str = r#"<html><head><title>First fallback</title></head><body><p onclick="alert(1)"><span class="mt-zh">第一段</span><br/><span class="mt-en">First paragraph</span></p></body></html>"#;
const SECOND_XHTML: &str = r#"<html><head><title>Second fallback</title></head><body><p>There is a second needle in this chapter.</p><script>alert(1)</script></body></html>"#;
