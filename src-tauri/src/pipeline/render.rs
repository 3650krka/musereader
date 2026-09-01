use crate::document::TranslatedBlock;
use crate::pipeline::layout::{is_figure_or_table_label, normalize_figure_table_label};
use article::prepare_markdown_for_article_body;
use authors::{extract_academic_authors as extract_authors_impl, render_academic_front_matter};
#[cfg(test)]
use authors::{
    refine_academic_authors_with_llm as refine_authors_impl,
    should_refine_academic_authors as should_refine_authors_impl,
};
use html::{
    apply_article_template, postprocess_academic_article_html,
    postprocess_block_rendered_academic_html as postprocess_block_html_impl,
};
use markdown::render_markdown_to_html;
use std::path::PathBuf;

mod article;
pub(super) mod authors;
mod html;
mod markdown;

const DEFAULT_RENDERED_TITLE: &str = "Translated Document";
const ABSTRACT_HEADING_ZH: &str = "\u{6458}\u{8981}";
const REFERENCES_HEADING_ZH: &str = "\u{53c2}\u{8003}\u{6587}\u{732e}";
const TEMPLATE_FILE_NAME: &str = "\u{6a21}\u{677f}.html";

pub(super) fn render_translated_html_with_authors(
    markdown: &str,
    front_matter: &authors::AcademicFrontMatter,
) -> String {
    let cleaned_markdown = sanitize_markdown_for_rendering(markdown);
    let title = extract_render_title(&cleaned_markdown, front_matter);
    let looks_academic =
        crate::pipeline::chunking::is_academic_article_from_markdown(&cleaned_markdown);
    let body_source_markdown = if authors::has_academic_front_matter(front_matter) {
        strip_front_matter_region(&cleaned_markdown, front_matter)
    } else {
        cleaned_markdown.clone()
    };
    let body_markdown = prepare_markdown_for_article_body(
        &body_source_markdown,
        &title,
        ABSTRACT_HEADING_ZH,
        REFERENCES_HEADING_ZH,
    );
    let mut rendered_body = render_markdown_body(&body_markdown);
    rendered_body = postprocess_academic_article_html(&rendered_body, REFERENCES_HEADING_ZH);
    if authors::has_academic_front_matter(front_matter) {
        let rendered_front_matter =
            render_academic_front_matter(&front_matter.authors, &front_matter.extras);
        rendered_body = format!("{rendered_front_matter}\n{rendered_body}");
    }
    if !authors::has_academic_front_matter(front_matter) && !looks_academic {
        if let Some(template) = load_article_template() {
            return apply_article_template(&template, &title, &rendered_body)
                .unwrap_or_else(|| wrap_rendered_article_html(&title, &rendered_body));
        }
    }
    wrap_rendered_article_html(&title, &rendered_body)
}

pub(super) fn render_translated_html_from_blocks_with_authors(
    markdown: &str,
    translated_blocks: &[TranslatedBlock],
    front_matter: &authors::AcademicFrontMatter,
) -> Option<String> {
    if translated_blocks.is_empty() {
        return None;
    }
    let cleaned_markdown = sanitize_markdown_for_rendering(markdown);
    if !translated_blocks_match_markdown(&cleaned_markdown, translated_blocks) {
        return None;
    }
    let title = extract_render_title(&cleaned_markdown, front_matter);
    let mut body_blocks = if authors::has_academic_front_matter(front_matter) {
        strip_front_matter_blocks(translated_blocks, front_matter)
    } else {
        translated_blocks.to_vec()
    };
    if body_blocks.is_empty() {
        return None;
    }
    if body_blocks.first().is_some_and(|block| {
        block.kind == "heading"
            && normalize_inline_text(&block.translated_markdown) == normalize_inline_text(&title)
    }) {
        body_blocks.remove(0);
    }
    let mut rendered_body = render_academic_body_from_blocks(&body_blocks);
    if rendered_body.trim().is_empty() {
        return None;
    }
    if authors::has_academic_front_matter(front_matter) {
        let rendered_front_matter =
            render_academic_front_matter(&front_matter.authors, &front_matter.extras);
        rendered_body = format!("{rendered_front_matter}\n{rendered_body}");
    }
    Some(wrap_rendered_article_html(&title, &rendered_body))
}

fn translated_blocks_match_markdown(markdown: &str, translated_blocks: &[TranslatedBlock]) -> bool {
    if translated_blocks.len() <= 8 {
        return true;
    }
    let block_text = translated_blocks
        .iter()
        .filter(|block| !is_explicit_front_matter_source_block(block))
        .map(block_markdown)
        .collect::<Vec<_>>()
        .join("\n\n");
    let markdown_key = normalize_render_integrity_key(markdown);
    let block_key = normalize_render_integrity_key(&block_text);
    if markdown_key.is_empty() || block_key.is_empty() {
        return true;
    }
    let prefix_len = common_prefix_char_count(&markdown_key, &block_key);
    let shorter_len = markdown_key.chars().count().min(block_key.chars().count());
    prefix_len * 100 >= shorter_len * 85
}

fn normalize_render_integrity_key(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn common_prefix_char_count(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn extract_render_title(markdown: &str, front_matter: &authors::AcademicFrontMatter) -> String {
    if authors::has_academic_front_matter(front_matter) {
        return first_front_matter_title(front_matter)
            .or_else(|| extract_markdown_title(markdown))
            .unwrap_or_else(|| DEFAULT_RENDERED_TITLE.to_string());
    }
    extract_markdown_title(markdown)
        .or_else(|| first_front_matter_title(front_matter))
        .unwrap_or_else(|| DEFAULT_RENDERED_TITLE.to_string())
}

fn first_front_matter_title(front_matter: &authors::AcademicFrontMatter) -> Option<String> {
    front_matter
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string)
}

pub(super) fn strip_front_matter_region(
    markdown: &str,
    front_matter: &authors::AcademicFrontMatter,
) -> String {
    let line_stripped = strip_front_matter_leading_lines(markdown, front_matter);
    let blocks = split_markdown_blocks(&line_stripped);
    let body_start = blocks
        .iter()
        .position(|block| is_body_start_block(block, front_matter))
        .or_else(|| {
            blocks
                .iter()
                .position(|block| !is_leading_front_matter_block(block, front_matter))
        })
        .unwrap_or(0);
    let stripped = blocks
        .into_iter()
        .skip(body_start)
        .collect::<Vec<_>>()
        .join("\n\n");
    strip_front_matter_body_prefix(&stripped, front_matter)
}

fn strip_front_matter_leading_lines(
    markdown: &str,
    front_matter: &authors::AcademicFrontMatter,
) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index].trim();
        if line.is_empty() {
            index += 1;
            continue;
        }
        if is_leading_front_matter_line(line, front_matter) {
            index += 1;
            continue;
        }
        break;
    }
    lines[index..].join("\n")
}

fn strip_front_matter_body_prefix(
    markdown: &str,
    front_matter: &authors::AcademicFrontMatter,
) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index].trim();
        if line.is_empty() {
            index += 1;
            continue;
        }
        if looks_like_front_matter_body_prefix_line(line, front_matter) {
            index += 1;
            continue;
        }
        break;
    }
    lines[index..].join("\n")
}

fn looks_like_front_matter_body_prefix_line(
    line: &str,
    front_matter: &authors::AcademicFrontMatter,
) -> bool {
    looks_like_update_notice_line(line)
        || line.starts_with("http://")
        || line.starts_with("https://")
        || is_front_matter_author_block(line, front_matter)
        || is_front_matter_extra_line_fallback(line)
        || is_front_matter_detail_block(line)
        || is_front_matter_affiliation_line(line, front_matter)
}

fn is_front_matter_extra_line_fallback(line: &str) -> bool {
    let normalized_line = normalize_inline_text(line);
    normalized_line.contains("check for updates")
        || normalized_line.contains("注意更新")
        || normalized_line.contains("检查更新")
        || normalized_line.contains("查看更新")
        || normalized_line.contains("检测更新")
}

fn is_leading_front_matter_line(line: &str, front_matter: &authors::AcademicFrontMatter) -> bool {
    is_front_matter_title_block(line, front_matter)
        || is_front_matter_author_block(line, front_matter)
        || is_front_matter_extra_line(line, front_matter)
        || is_front_matter_detail_block(line)
        || is_front_matter_affiliation_line(line, front_matter)
}

fn is_front_matter_extra_line(line: &str, front_matter: &authors::AcademicFrontMatter) -> bool {
    let normalized_line = normalize_inline_text(line);
    if normalized_line.is_empty() {
        return false;
    }
    front_matter.extras.iter().any(|extra| {
        let normalized_extra = normalize_inline_text(extra);
        !normalized_extra.is_empty()
            && (normalized_line == normalized_extra || normalized_line.contains(&normalized_extra))
    }) || line.starts_with("http://")
        || line.starts_with("https://")
        || normalized_line.contains("check for updates")
        || normalized_line.contains("检查更新")
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn strip_front_matter_blocks(
    blocks: &[TranslatedBlock],
    front_matter: &authors::AcademicFrontMatter,
) -> Vec<TranslatedBlock> {
    let blocks = blocks
        .iter()
        .filter(|block| !is_explicit_front_matter_source_block(block))
        .cloned()
        .collect::<Vec<_>>();
    let body_start = blocks
        .iter()
        .position(|block| is_body_start_block(&block_markdown(block), front_matter))
        .or_else(|| {
            blocks.iter().position(|block| {
                !is_leading_front_matter_block(&block_markdown(block), front_matter)
            })
        })
        .unwrap_or(0);
    blocks.into_iter().skip(body_start).collect()
}

fn is_explicit_front_matter_source_block(block: &TranslatedBlock) -> bool {
    block
        .source_markdown
        .contains(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START)
        || block
            .source_markdown
            .contains(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END)
        || block
            .translated_markdown
            .contains(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START)
        || block
            .translated_markdown
            .contains(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END)
}

fn is_leading_front_matter_block(block: &str, front_matter: &authors::AcademicFrontMatter) -> bool {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return true;
    }
    if is_front_matter_title_block(trimmed, front_matter)
        || is_front_matter_author_block(trimmed, front_matter)
        || is_front_matter_extra_line(trimmed, front_matter)
        || is_front_matter_affiliation_line(trimmed, front_matter)
        || trimmed.contains("$ ^{")
        || trimmed.contains(r"\textcircled")
        || is_front_matter_detail_block(trimmed)
    {
        return true;
    }

    false
}

fn is_front_matter_title_block(block: &str, front_matter: &authors::AcademicFrontMatter) -> bool {
    let normalized_block = normalize_inline_text(block);
    first_front_matter_title(front_matter)
        .map(|title| normalize_inline_text(&title) == normalized_block)
        .unwrap_or(false)
}

fn is_front_matter_author_block(block: &str, front_matter: &authors::AcademicFrontMatter) -> bool {
    let normalized_block = normalize_inline_text(block);
    front_matter.authors.iter().any(|author| {
        let normalized_name = normalize_inline_text(&author.name);
        !normalized_name.is_empty() && normalized_block.contains(&normalized_name)
    })
}

fn is_front_matter_affiliation_line(
    block: &str,
    front_matter: &authors::AcademicFrontMatter,
) -> bool {
    let normalized_block = normalize_inline_text(block);
    if normalized_block.is_empty() {
        return false;
    }

    if front_matter.authors.iter().any(|author| {
        author.affiliations.iter().any(|affiliation| {
            let normalized_affiliation = normalize_inline_text(affiliation);
            !normalized_affiliation.is_empty()
                && (normalized_block == normalized_affiliation
                    || normalized_block.contains(&normalized_affiliation)
                    || normalized_affiliation.contains(&normalized_block))
        }) || author
            .correspondence_note
            .as_deref()
            .map(normalize_inline_text)
            .filter(|note| !note.is_empty())
            .map(|note| normalized_block == note || normalized_block.contains(&note))
            .unwrap_or(false)
            || author
                .email
                .as_deref()
                .map(|email| normalized_block.contains(&email.to_ascii_lowercase()))
                .unwrap_or(false)
    }) {
        return true;
    }

    starts_with_affiliation_marker(block)
        || normalized_block.contains("鍚岀瓑璐＄尞")
        || normalized_block.contains("these authors contributed equally")
        || normalized_block.contains("equal contribution")
}

fn starts_with_affiliation_marker(block: &str) -> bool {
    let trimmed = block.trim_start();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_digit() || first == '*' || first == '\u{2020}' || first == '\u{2021}')
        && chars.any(|ch| ch.is_alphabetic() || ('\u{4e00}'..='\u{9fff}').contains(&ch))
}

fn is_front_matter_detail_block(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    block.contains('@')
        || block.contains("通讯作者")
        || block.contains("电子邮箱")
        || block.contains("邮箱")
        || block.contains("收稿日期")
        || block.contains("接受日期")
        || block.contains("在线发表")
        || block.contains("Received:")
        || block.contains("Accepted:")
        || block.contains("Published online:")
        || lower.contains("email:")
        || lower.contains("received")
        || lower.contains("accepted")
        || lower.contains("published online")
        || lower.contains("corresponding author")
        || lower.contains("correspondence to:")
        || lower.contains("department")
        || lower.contains("university")
        || lower.contains("institute")
        || lower.contains("college")
}

fn normalize_inline_text(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('#')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn looks_like_update_notice_line(line: &str) -> bool {
    let normalized_line = normalize_inline_text(line);
    normalized_line.contains("check for updates")
        || normalized_line.contains("注意更新")
        || normalized_line.contains("检查更新")
        || normalized_line.contains("查看更新")
}

fn has_sentence_punctuation(value: &str) -> bool {
    value.contains('.')
        || value.contains('?')
        || value.contains('!')
        || value.contains('。')
        || value.contains('；')
        || value.contains('？')
}

fn is_body_start_block(block: &str, front_matter: &authors::AcademicFrontMatter) -> bool {
    let trimmed = block.trim();
    if trimmed.is_empty() || is_leading_front_matter_block(trimmed, front_matter) {
        return false;
    }

    if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
        return true;
    }

    trimmed.chars().count() >= 48 && has_sentence_punctuation(trimmed)
}

pub(super) fn extract_academic_authors(markdown: &str) -> Vec<authors::AcademicAuthor> {
    extract_authors_impl(markdown)
}

#[cfg(test)]
pub(super) async fn refine_academic_authors_with_llm(
    llm_client: &crate::llm::SensenovaClient,
    markdown: &str,
    first_page_footnotes: &[String],
    authors: &[authors::AcademicAuthor],
) -> Result<Vec<authors::AcademicAuthor>, crate::error::AppError> {
    refine_authors_impl(llm_client, markdown, first_page_footnotes, authors).await
}

#[cfg(test)]
pub(super) fn should_refine_academic_authors(authors: &[authors::AcademicAuthor]) -> bool {
    should_refine_authors_impl(authors)
}

fn render_markdown_body(markdown: &str) -> String {
    let full = render_markdown_to_html(markdown);
    let Some(body_start) = full.find("<body>") else {
        return full;
    };
    let Some(body_end) = full.rfind("</body>") else {
        return full;
    };
    full[body_start + "<body>".len()..body_end].to_string()
}

fn render_academic_body_from_blocks(blocks: &[TranslatedBlock]) -> String {
    let mut html = String::new();
    let mut index = 0usize;
    while index < blocks.len() {
        let block = &blocks[index];
        let markdown = block_markdown(block);
        if is_figure_or_table_label(markdown.trim()) {
            if let Some((figure_html, consumed)) = try_render_media_sequence(&blocks[index..]) {
                html.push_str(&figure_html);
                index += consumed;
                continue;
            }
        }
        html.push_str(&render_single_block_html(&markdown));
        index += 1;
    }
    postprocess_block_rendered_academic_html(&html, REFERENCES_HEADING_ZH)
}

fn try_render_media_sequence(blocks: &[TranslatedBlock]) -> Option<(String, usize)> {
    let label_block = blocks.first()?;
    let media_block = blocks.get(1)?;
    let media_html = render_single_block_html(&block_markdown(media_block));
    if !media_html.contains("<img ") && !media_html.contains("<table") {
        return None;
    }
    let label = normalize_figure_table_label(&block_markdown(label_block));
    let mut consumed = 2usize;
    let mut caption_html = String::new();
    if let Some(caption_block) = blocks.get(2) {
        let caption_markdown = block_markdown(caption_block);
        if caption_block.kind == "paragraph"
            && !looks_like_following_media_label(caption_block, &caption_markdown)
        {
            caption_html = render_single_block_html(&caption_markdown);
            consumed += 1;
        }
    }
    let mut figure = String::from("<figure class=\"media-block\">");
    figure.push_str("<div class=\"media-label\">");
    figure.push_str(&escape_html_min(&label));
    figure.push_str("</div>");
    figure.push_str(&media_html);
    if !caption_html.is_empty() {
        figure.push_str(
            &caption_html
                .replace("<p>", "<figcaption>")
                .replace("</p>", "</figcaption>"),
        );
    }
    figure.push_str("</figure>");
    Some((figure, consumed))
}

fn render_single_block_html(markdown: &str) -> String {
    let trimmed = markdown.trim();
    if trimmed.starts_with("<table") && trimmed.contains("</table>") {
        return trimmed.to_string();
    }
    render_markdown_body(markdown)
}

fn postprocess_block_rendered_academic_html(html: &str, references_heading_zh: &str) -> String {
    postprocess_block_html_impl(html, references_heading_zh)
}

fn looks_like_following_media_label(block: &TranslatedBlock, rendered_markdown: &str) -> bool {
    let source_trimmed = block.source_markdown.trim();
    if !source_trimmed.is_empty() && !is_figure_or_table_label(source_trimmed) {
        return false;
    }
    is_figure_or_table_label(rendered_markdown.trim())
}

fn block_markdown(block: &TranslatedBlock) -> String {
    let translated = block.translated_markdown.trim();
    if !translated.is_empty() {
        translated.to_string()
    } else {
        block.source_markdown.clone()
    }
}

fn wrap_rendered_article_html(title: &str, rendered_body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>:root{{--article-font-zh:\"Noto Serif SC\",\"Songti SC\",SimSun,serif;--article-font-en:\"Times New Roman\",Times,serif;--article-accent:#7d6235;--article-border:#d8d2c8}}body{{margin:0;background:#f7f5f1;color:#1f2937;font:16px/1.8 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif}}main{{max-width:960px;margin:0 auto;padding:32px 20px 56px}}article.article-shell{{background:#fff;border:1px solid #ece7df;border-radius:14px;box-shadow:0 10px 30px rgba(17,24,39,.05);padding:36px 34px 44px}}h1.article-title{{margin:0 0 1.6rem;font:700 clamp(1.8rem,2.8vw,2.55rem)/1.28 var(--article-font-zh);color:#231f1b}}.article-body{{font-family:var(--article-font-zh);font-size:1rem}}.article-body h2,.article-body h3,.article-body h4{{display:block;margin:2.1em 0 .85em;font-weight:700;color:#2B2824}}.article-body h2{{font-size:1.35rem}}.article-body h3{{font-size:1.14rem}}p,ul,ol,blockquote,pre,figure{{margin:0 0 1em}}pre{{padding:16px;border-radius:12px;background:#f3f4f6;overflow:auto}}code{{font-family:ui-monospace,SFMono-Regular,Consolas,monospace}}blockquote{{margin-left:0;padding-left:16px;border-left:3px solid #d1d5db;color:#4b5563}}img{{max-width:100%;height:auto;border-radius:12px}}hr{{border:none;border-top:1px solid #e5e7eb;margin:24px 0}}li+li{{margin-top:.35em}}.academic-front-matter{{margin:0 0 1.8rem;padding:0 0 1.2rem;border-bottom:1px solid var(--article-border)}}.academic-author-line{{display:flex;flex-wrap:wrap;gap:.5rem 1rem;align-items:flex-start}}.academic-author-chip{{position:relative;display:inline-flex;align-items:flex-start;gap:.12rem}}.academic-author-line sup{{font-size:.72em;line-height:1;margin-top:.08rem;color:#6f6a61}}.academic-author-link{{color:var(--article-accent);text-decoration:underline;text-decoration-thickness:1px;text-underline-offset:3px;cursor:help}}.academic-author-link:hover{{color:#2B2824;background:#f4efe4}}.academic-author-chip:hover .academic-author-popover,.academic-author-chip:focus-within .academic-author-popover{{opacity:1;transform:translateY(0);pointer-events:auto}}.academic-author-popover{{position:absolute;left:0;top:calc(100% + .55rem);z-index:12;min-width:18rem;max-width:min(28rem,70vw);padding:.7rem .85rem;border:1px solid var(--article-border);border-radius:12px;background:#fffdfa;color:#35302a;box-shadow:0 10px 30px rgba(43,40,36,.12);font-size:.92rem;line-height:1.55;opacity:0;transform:translateY(6px);pointer-events:none;transition:opacity .18s ease,transform .18s ease}}.academic-author-popover-row{{display:block}}.academic-author-popover-row+.academic-author-popover-row{{margin-top:.35rem}}.academic-author-email{{font-family:var(--article-font-en);font-size:.9rem;color:#5f4d2e}}.academic-front-matter-extras{{display:flex;flex-wrap:wrap;gap:.55rem 1rem;margin-top:.85rem;color:#6b5a3c;font-size:.93rem}}.academic-front-matter-extra{{display:inline-flex;align-items:center;gap:.35rem}}.academic-front-matter-extra a{{color:var(--article-accent);text-decoration:underline;text-underline-offset:3px}}.reference-block{{margin-top:2em;padding-top:.9em;border-top:1px solid var(--article-border);color:#35302a;font-family:var(--article-font-en);font-size:.92em;line-height:1.5}}.reference-block h2{{font-family:var(--article-font-zh);font-size:1.12em;margin:0 0 .55em}}.reference-block p{{margin:0 0 .45em}}.media-block{{break-inside:avoid;margin:1.4em 0;padding:.7em 0;border-top:1px solid #e7e0d6;border-bottom:1px solid #e7e0d6}}.media-block img{{display:block;width:100%;height:auto;border-radius:10px}}.media-block figcaption{{margin-top:.55em;font-size:.94em;line-height:1.55;color:#5b564d}}.media-label{{font-weight:700;color:#5f4d2e;margin-bottom:.55em}}.pdf-table-scroll{{overflow-x:auto}}.pdf-table-block table{{width:max-content;min-width:100%;border-collapse:collapse;font-family:var(--article-font-en);font-size:.9em;line-height:1.35}}.pdf-table-block th,.pdf-table-block td{{padding:.35rem .55rem;border-top:1px solid var(--article-border);vertical-align:top}}.pdf-table-block tr:last-child td{{border-bottom:1px solid var(--article-border)}}</style></head><body><main><article class=\"article-shell\"><h1 class=\"article-title\">{}</h1><div class=\"article-body\">{}</div></article></main></body></html>",
        escape_html_min(title),
        escape_html_min(title),
        rendered_body
    )
}

fn escape_html_min(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn load_article_template() -> Option<String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let click_root = manifest_dir.parent()?.parent()?;
    let template_path = click_root.join(TEMPLATE_FILE_NAME);
    std::fs::read_to_string(template_path).ok()
}

fn extract_markdown_title(markdown: &str) -> Option<String> {
    let mut inside_toc_block = false;
    markdown.lines().find_map(|line| {
        let stripped = line.trim();
        if stripped == "<!-- musetranslate:toc:start -->" {
            inside_toc_block = true;
            return None;
        }
        if stripped == "<!-- musetranslate:toc:end -->" {
            inside_toc_block = false;
            return None;
        }
        if inside_toc_block {
            return None;
        }
        stripped
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn sanitize_markdown_for_rendering(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            is_renderable_comment(trimmed)
                || !(trimmed.starts_with("<!--") && trimmed.ends_with("-->"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod block_renderer_tests {
    use super::*;

    #[test]
    fn block_renderer_keeps_media_label_image_and_caption_together() {
        let blocks = vec![
            TranslatedBlock {
                id: "b1".to_string(),
                order: 0,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "heading".to_string(),
                source_markdown: "## Results".to_string(),
                translated_markdown: "## 缁撴灉".to_string(),
                source_text: "Results".to_string(),
                translated_text: "缁撴灉".to_string(),
                translate: true,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "b2".to_string(),
                order: 1,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "paragraph".to_string(),
                source_markdown: "Figure 1".to_string(),
                translated_markdown: "Figure 1".to_string(),
                source_text: "Figure 1".to_string(),
                translated_text: "Figure 1".to_string(),
                translate: false,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "b3".to_string(),
                order: 2,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "image".to_string(),
                source_markdown: "![](imgs/example.png)".to_string(),
                translated_markdown: "![](imgs/example.png)".to_string(),
                source_text: "".to_string(),
                translated_text: "".to_string(),
                translate: false,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "b4".to_string(),
                order: 3,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "paragraph".to_string(),
                source_markdown: "Figure 1 caption.".to_string(),
                translated_markdown: "图1说明。".to_string(),
                source_text: "Figure 1 caption.".to_string(),
                translated_text: "图1说明。".to_string(),
                translate: true,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
        ];

        let html = render_translated_html_from_blocks_with_authors(
            "## 缁撴灉",
            &blocks,
            &Default::default(),
        )
        .expect("block renderer should render");

        assert!(html.contains("<figure class=\"media-block\">"), "{html}");
        assert!(
            html.contains("<div class=\"media-label\">Figure 1</div>"),
            "{html}"
        );
        assert!(html.contains("<img src=\"imgs/example.png\""), "{html}");
        assert!(html.contains("<figcaption>"), "{html}");
        assert!(html.contains("</figcaption>"), "{html}");
    }

    #[test]
    fn block_renderer_keeps_table_label_snapshot_and_note_together() {
        let blocks = vec![
            TranslatedBlock {
                id: "t1".to_string(),
                order: 0,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "paragraph".to_string(),
                source_markdown: "Table 1".to_string(),
                translated_markdown: "Table 1".to_string(),
                source_text: "Table 1".to_string(),
                translated_text: "Table 1".to_string(),
                translate: false,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "t2".to_string(),
                order: 1,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "table".to_string(),
                source_markdown: "<table><tr><td>A</td></tr></table>".to_string(),
                translated_markdown: "<table><tr><td>A</td></tr></table>".to_string(),
                source_text: "A".to_string(),
                translated_text: "A".to_string(),
                translate: false,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "t3".to_string(),
                order: 2,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "paragraph".to_string(),
                source_markdown: "Table note.".to_string(),
                translated_markdown: "表格注释。".to_string(),
                source_text: "Table note.".to_string(),
                translated_text: "表格注释。".to_string(),
                translate: true,
                heading_path: vec!["Results".to_string()],
                chapter_title: Some("Results".to_string()),
                href: None,
                page: None,
            },
        ];

        let html = render_translated_html_from_blocks_with_authors(
            "# Results",
            &blocks,
            &Default::default(),
        )
        .expect("block renderer should render table block");

        assert!(html.contains("<figure class=\"media-block\">"), "{html}");
        assert!(
            html.contains("<div class=\"media-label\">Table 1</div>"),
            "{html}"
        );
        assert!(
            html.contains("<table><tr><td>A</td></tr></table>"),
            "{html}"
        );
        assert!(
            html.contains("<figcaption>表格注释。</figcaption>"),
            "{html}"
        );
    }

    #[test]
    fn block_renderer_preserves_abstract_before_keywords() {
        let blocks = vec![
            TranslatedBlock {
                id: "a1".to_string(),
                order: 0,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "heading".to_string(),
                source_markdown: "## Abstract".to_string(),
                translated_markdown: "## 鎽樿".to_string(),
                source_text: "Abstract".to_string(),
                translated_text: "鎽樿".to_string(),
                translate: true,
                heading_path: vec!["Abstract".to_string()],
                chapter_title: Some("Abstract".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "a2".to_string(),
                order: 1,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "paragraph".to_string(),
                source_markdown: "Body text.".to_string(),
                translated_markdown: "这是摘要正文。".to_string(),
                source_text: "Body text.".to_string(),
                translated_text: "这是摘要正文。".to_string(),
                translate: true,
                heading_path: vec!["Abstract".to_string()],
                chapter_title: Some("Abstract".to_string()),
                href: None,
                page: None,
            },
            TranslatedBlock {
                id: "a3".to_string(),
                order: 2,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: "paragraph".to_string(),
                source_markdown: "Keywords: test".to_string(),
                translated_markdown: "鍏抽敭璇嶏細娴嬭瘯".to_string(),
                source_text: "Keywords: test".to_string(),
                translated_text: "鍏抽敭璇嶏細娴嬭瘯".to_string(),
                translate: true,
                heading_path: vec!["Abstract".to_string()],
                chapter_title: Some("Abstract".to_string()),
                href: None,
                page: None,
            },
        ];

        let html = render_translated_html_from_blocks_with_authors(
            "# Paper Title",
            &blocks,
            &Default::default(),
        )
        .expect("block renderer should render abstract section");

        let abstract_index = html.find(">鎽樿</h2>").expect("abstract heading");
        let body_index = html.find("这是摘要正文。").expect("abstract body");
        let keyword_index = html.find("鍏抽敭璇嶏細娴嬭瘯").expect("keywords");

        assert!(abstract_index < body_index, "{html}");
        assert!(body_index < keyword_index, "{html}");
    }

    #[test]
    fn block_renderer_rejects_misaligned_translated_blocks() {
        let markdown = "# 标题\n\n第一段正确。\n\n第二段正确。\n\n第三段正确。\n\n第四段正确。";
        let blocks = (0..9)
            .map(|index| TranslatedBlock {
                id: format!("b{index}"),
                order: index,
                chunk_index: None,
                marker_id: None,
                alignment_method: String::new(),
                kind: if index == 0 { "heading" } else { "paragraph" }.to_string(),
                source_markdown: format!("source {index}"),
                translated_markdown: if index < 3 {
                    ["# 标题", "第一段正确。", "第二段正确。"][index].to_string()
                } else {
                    "错误重复段。".to_string()
                },
                source_text: format!("source {index}"),
                translated_text: String::new(),
                translate: true,
                heading_path: Vec::new(),
                chapter_title: None,
                href: None,
                page: None,
            })
            .collect::<Vec<_>>();

        assert!(render_translated_html_from_blocks_with_authors(
            markdown,
            &blocks,
            &Default::default()
        )
        .is_none());
    }
}

fn is_renderable_comment(trimmed: &str) -> bool {
    matches!(
        trimmed,
        crate::pipeline::layout::PDF_TABLE_START_MARKER
            | crate::pipeline::layout::PDF_TABLE_END_MARKER
    )
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn removes_author_metadata_lines_from_article_body() {
        let markdown = "# Title\n\nAuthor Name $ ^{1,a} $\n\n## Addresses\n\n$ ^{1} $ University A\n\nCorresponding authors: Name, Author (author@example.test)\n\n## Abstract\n\nBody.";
        let prepared = prepare_markdown_for_article_body(
            markdown,
            "Title",
            ABSTRACT_HEADING_ZH,
            REFERENCES_HEADING_ZH,
        );

        assert!(prepared.contains("Author Name $ ^{1,a} $"));
        assert!(!prepared.contains("University A"));
        assert!(!prepared.contains("Corresponding authors:"));
        assert!(prepared.contains("## \u{6458}\u{8981}"));
    }

    #[test]
    fn removes_affiliation_lines_even_without_addresses_heading() {
        let markdown = "# Title\n\nAuthor Name $ ^{1,a} $\n\n$ ^{1} $ University A\n\n$ ^{a} $ Philadelphia, PA\n\n## Abstract\n\nBody.";
        let prepared = prepare_markdown_for_article_body(
            markdown,
            "Title",
            ABSTRACT_HEADING_ZH,
            REFERENCES_HEADING_ZH,
        );

        assert!(prepared.contains("Author Name $ ^{1,a} $"));
        assert!(!prepared.contains("University A"));
        assert!(!prepared.contains("Philadelphia, PA"));
        assert!(prepared.contains("## \u{6458}\u{8981}"));
    }

    #[test]
    fn removes_author_received_dates_before_abstract() {
        let markdown = "# Title\n\nAuthor Name (ID)\n\n$ ^{*} $通讯作者：Department of Policy\n电子邮箱：author@example.test\n\n（2020年1月1日接收；2020年12月2日修订；2021年1月10日接受；2021年1月16日首次在线发表）\n\n## Abstract\n\nBody.";
        let prepared = prepare_markdown_for_article_body(
            markdown,
            "Title",
            ABSTRACT_HEADING_ZH,
            REFERENCES_HEADING_ZH,
        );

        assert!(prepared.contains("Author Name (ID)"));
        assert!(!prepared.contains("author@example.test"));
        assert!(!prepared.contains("首次在线发表"));
        assert!(prepared.contains("## \u{6458}\u{8981}"));
    }

    #[test]
    fn extract_markdown_title_skips_generated_toc_heading() {
        let markdown = "<!-- musetranslate:toc:start -->\n## 鐩綍\n- [绔犺妭涓€](#绔犺妭涓€)\n<!-- musetranslate:toc:end -->\n\n# 姝ｆ枃鏍囬\n\n姝ｆ枃";

        assert_eq!(
            extract_markdown_title(markdown).as_deref(),
            Some("姝ｆ枃鏍囬")
        );
    }

    #[test]
    fn rendered_html_uses_body_title_instead_of_generated_toc_heading() {
        let markdown = "<!-- musetranslate:toc:start -->\n## 鐩綍\n- [姝ｆ枃鏍囬](#姝ｆ枃鏍囬)\n  - [鏂规硶](#鏂规硶)\n<!-- musetranslate:toc:end -->\n\n# 姝ｆ枃鏍囬\n\n## 鏂规硶\n\n姝ｆ枃";

        let front_matter = Default::default();
        let html = render_translated_html_with_authors(markdown, &front_matter);

        assert!(html.contains("<title>姝ｆ枃鏍囬</title>"));
        assert!(html.contains(">姝ｆ枃鏍囬</h1>"));
        assert!(html.contains("<a href=\"#姝ｆ枃鏍囬\">姝ｆ枃鏍囬</a>"));
        assert!(html.contains("<a href=\"#鏂规硶\">鏂规硶</a>"));
    }

    #[test]
    fn sanitize_render_markdown_keeps_pdf_table_markers_only() {
        let markdown = format!(
            "<!-- ordinary comment -->\n{}\n<table><tr><td>A</td></tr></table>\n{}",
            crate::pipeline::layout::PDF_TABLE_START_MARKER,
            crate::pipeline::layout::PDF_TABLE_END_MARKER
        );

        let cleaned = sanitize_markdown_for_rendering(&markdown);

        assert!(!cleaned.contains("ordinary comment"));
        assert!(cleaned.contains(crate::pipeline::layout::PDF_TABLE_START_MARKER));
        assert!(cleaned.contains("<table><tr><td>A</td></tr></table>"));
        assert!(cleaned.contains(crate::pipeline::layout::PDF_TABLE_END_MARKER));
    }

    #[test]
    fn rendered_html_keeps_abstract_body_before_keywords() {
        let markdown = "# Nudge plus\n\nSanchayan Banerjee ID and Peter John* ID\n\nDepartment of Geography and Environment, London School of Economics, London, UK\n\n$ ^{*} $Correspondence to: Department of Political Economy, King's College London.\n\nE-mail: peter.john@kcl.ac.uk\n\n(Received 6 May 2020; revised 22 December 2020)\n\n## \u{6458}\u{8981}\n\n\u{52A9}\u{63A8}+\u{662F}\u{5BF9}\u{884C}\u{4E3A}\u{516C}\u{5171}\u{653F}\u{7B56}\u{5DE5}\u{5177}\u{7BB1}\u{7684}\u{4E00}\u{79CD}\u{6539}\u{8FDB}\u{3002}\u{8BE5}\u{8BBA}\u{70B9}\u{57FA}\u{4E8E}\u{5BF9}\u{53CC}\u{91CD}\u{7CFB}\u{7EDF}\u{7684}\u{5F00}\u{521B}\u{6027}\u{7814}\u{7A76}\u{3002}\n\n\u{5173}\u{952E}\u{8BCD}\u{FF1A}\u{52A9}\u{63A8}\u{FF1B}\u{53CC}\u{8FC7}\u{7A0B}\u{7406}\u{8BBA}\n";
        let authors = extract_academic_authors(markdown);
        let front_matter = authors::AcademicFrontMatter {
            title: Some("Nudge plus".to_string()),
            authors,
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let html = render_translated_html_with_authors(markdown, &front_matter);

        let abstract_index = html
            .find(">\u{6458}\u{8981}</h2>")
            .expect("abstract heading");
        let body_index = html
            .find("\u{884C}\u{4E3A}\u{516C}\u{5171}\u{653F}\u{7B56}\u{5DE5}\u{5177}\u{7BB1}")
            .expect("abstract body");
        let keyword_index = html.find("\u{5173}\u{952E}\u{8BCD}").expect("keywords");

        assert!(abstract_index < body_index);
        assert!(body_index < keyword_index);
        assert!(!html.contains("<p>Department of Geography"));
        assert!(!html.contains("peter.john@kcl.ac.uk</p>"));
    }

    #[test]
    fn structured_authors_replace_original_front_matter_region() {
        let markdown = "# Nudge plus\n\nSanchayan Banerjee ID and Peter John* ID\n\nDepartment of Geography and Environment, London School of Economics, London, UK\n\nE-mail: peter.john@kcl.ac.uk\n\nThis article studies how reflection changes behavioral public policy. It contains enough words to be treated as the first body paragraph by the front matter boundary detector.\n\n## Methods\n\nBody.";
        let authors = vec![
            authors::AcademicAuthor {
                name: "Sanchayan Banerjee".to_string(),
                email: None,
                markers: vec!["ID".to_string()],
                affiliations: vec!["London School of Economics".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            },
            authors::AcademicAuthor {
                name: "Peter John".to_string(),
                email: Some("peter.john@kcl.ac.uk".to_string()),
                markers: vec!["ID".to_string(), "*".to_string()],
                affiliations: vec!["King's College London".to_string()],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some("Corresponding author".to_string()),
            },
        ];
        let front_matter = authors::AcademicFrontMatter {
            title: Some("Nudge plus".to_string()),
            authors,
            extras: vec!["https://doi.org/example".to_string()],
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let html = render_translated_html_with_authors(markdown, &front_matter);

        assert!(html.contains("<section class=\"academic-front-matter\">"));
        assert!(html.contains("Sanchayan Banerjee"));
        assert!(html.contains("Peter John"));
        assert!(html.contains("https://doi.org/example"));
        assert!(html.contains("mailto:peter.john@kcl.ac.uk"));
        assert!(html.contains("This article studies how reflection changes"));
        assert!(!html.contains("<p>Department of Geography"));
        assert!(!html.contains("<p>E-mail: peter.john@kcl.ac.uk</p>"));
        assert_eq!(
            html.matches("<section class=\"academic-front-matter\">")
                .count(),
            1
        );
    }

    #[test]
    fn structured_front_matter_allows_missing_optional_fields() {
        let markdown = "# Article Title\n\nAuthor One\n\nThis abstract starts directly without explicit author affiliations or email. It is long enough to be treated as body content and should remain visible after rendering.";
        let front_matter = authors::AcademicFrontMatter {
            title: Some("Article Title".to_string()),
            authors: vec![authors::AcademicAuthor {
                name: "Author One".to_string(),
                email: None,
                markers: Vec::new(),
                affiliations: Vec::new(),
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            }],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let html = render_translated_html_with_authors(markdown, &front_matter);

        assert!(html.contains("<section class=\"academic-front-matter\">"));
        assert!(html.contains("Author One"));
        assert!(html.contains("This abstract starts directly"));
        assert!(!html.contains("mailto:"));
        let article_start = html
            .find("<section class=\"academic-front-matter\">")
            .expect("front matter section");
        let article_html = &html[article_start..];
        assert!(!article_html.contains("academic-front-matter-extra\">"));
        assert!(!article_html.contains("academic-author-email\">"));
    }

    #[test]
    fn structured_front_matter_title_wins_over_markdown_title() {
        let markdown = "# Original English Title\n\n## 摘要\n\n正文。";
        let front_matter = authors::AcademicFrontMatter {
            title: Some("涓枃鏍囬".to_string()),
            authors: vec![authors::AcademicAuthor {
                name: "Author One".to_string(),
                email: None,
                markers: Vec::new(),
                affiliations: Vec::new(),
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            }],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let html = render_translated_html_with_authors(markdown, &front_matter);

        assert!(html.contains("<title>涓枃鏍囬</title>"));
        assert!(html.contains("<h1 class=\"article-title\">涓枃鏍囬</h1>"));
    }

    #[test]
    fn empty_front_matter_does_not_trigger_render_time_author_guessing() {
        let markdown = "# Article Title\n\nValentin Guigon $^{1,2}$, Marie Claire Villeval $^{2,3,4}$\n\n我们如何评估模糊新闻的真实性？元认知是否指导我们决定寻求更多信息？在受控实验中，参与者评估了模糊新闻的真实性。\n\n## 结果\n\n正文";

        let html = render_translated_html_with_authors(markdown, &Default::default());

        assert!(!html.contains("academic-front-matter"));
        assert!(html.contains("我们如何评估模糊新闻的真实性？"));
    }

    #[test]
    fn rendered_html_does_not_depend_on_legacy_template_shell() {
        let markdown = "# 正文标题\n\n第一段正文。\n\n## 方法\n\n第二段正文。";

        let html = render_translated_html_with_authors(markdown, &Default::default());

        assert!(html.contains("<title>正文标题</title>"));
        assert!(html.contains("The Classical Chronicle"));
        assert!(html.contains("<div class=\"article-body\">"));
        assert!(html.contains("正文标题"));
        assert!(html.contains("<h2 id=\"方法\">方法</h2>"));
        assert!(!html.contains("legacy template author"));
        assert!(!html.contains("legacy template subtitle"));
    }

    #[test]
    fn regenerate_gate_pdf_policy_short_html_snapshot() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let artifact_dir = manifest_dir.join(".runtime/artifacts/gate-pdf-policy-short-001");
        let markdown_path = artifact_dir.join("translated.md");
        let html_path = artifact_dir.join("translated.html");
        let front_matter_path = artifact_dir.join("front_matter.json");
        if !markdown_path.exists() {
            eprintln!(
                "skip gate HTML snapshot regeneration: missing {}",
                markdown_path.display()
            );
            return;
        }

        let markdown = std::fs::read_to_string(&markdown_path).expect("read translated markdown");
        let front_matter = load_snapshot_front_matter(&front_matter_path);
        let html = render_translated_html_with_authors(&markdown, &front_matter);
        std::fs::write(&html_path, html).expect("write translated html");
    }

    #[test]
    fn regenerate_gate_pdf_empirical_singlepdf_html_snapshot() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let artifact_dir = manifest_dir.join(
            ".runtime/artifacts/gate-pdf-empirical-article-001-latest-20260530-singlepdf-001",
        );
        let markdown_path = artifact_dir.join("translated.md");
        let html_path = artifact_dir.join("translated.html");
        let front_matter_path = artifact_dir.join("front_matter.json");
        if !markdown_path.exists() {
            eprintln!(
                "skip empirical singlepdf HTML snapshot regeneration: missing {}",
                markdown_path.display()
            );
            return;
        }

        let markdown = std::fs::read_to_string(&markdown_path).expect("read translated markdown");
        let front_matter = load_snapshot_front_matter(&front_matter_path);
        let html = render_translated_html_with_authors(&markdown, &front_matter);
        std::fs::write(&html_path, html).expect("write translated html");
    }

    #[test]
    fn regenerate_gate_pdf_empirical_singlepdf_html_current_snapshot() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let artifact_dir = manifest_dir
            .join(".runtime/artifacts/gate-pdf-empirical-article-001-singlepdf-html-001");
        let markdown_path = artifact_dir.join("translated.md");
        let html_path = artifact_dir.join("translated.html");
        let front_matter_path = artifact_dir.join("front_matter.json");
        if !markdown_path.exists() {
            eprintln!(
                "skip empirical current HTML snapshot regeneration: missing {}",
                markdown_path.display()
            );
            return;
        }

        let markdown = std::fs::read_to_string(&markdown_path).expect("read translated markdown");
        let front_matter = load_snapshot_front_matter(&front_matter_path);
        let html = render_translated_html_with_authors(&markdown, &front_matter);
        std::fs::write(&html_path, html).expect("write translated html");
    }

    #[test]
    fn empirical_current_snapshot_renders_without_legacy_template_shell() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let artifact_dir = manifest_dir
            .join(".runtime/artifacts/gate-pdf-empirical-article-001-singlepdf-html-001");
        let markdown_path = artifact_dir.join("translated.md");
        let front_matter_path = artifact_dir.join("front_matter.json");
        if !markdown_path.exists() || !front_matter_path.exists() {
            eprintln!(
                "skip empirical current render assertion: missing {} or {}",
                markdown_path.display(),
                front_matter_path.display()
            );
            return;
        }

        let markdown = std::fs::read_to_string(&markdown_path).expect("read translated markdown");
        let front_matter = load_snapshot_front_matter(&front_matter_path);
        let html = render_translated_html_with_authors(&markdown, &front_matter);

        assert!(
            html.contains("article-shell"),
            "expected built-in academic shell"
        );
        assert!(
            html.contains("DOI:"),
            "expected DOI label in front matter extras"
        );
        assert!(
            !html.contains("The Classical Chronicle"),
            "legacy template should not wrap academic PDF output"
        );
        assert!(
            !html.contains("<p>Valentin Guigon <sup>1,2</sup>"),
            "raw front matter block should be stripped from article body"
        );
    }

    #[test]
    fn empirical_singlepdf_snapshot_front_matter_loads_nonempty() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let artifact_dir = manifest_dir.join(
            ".runtime/artifacts/gate-pdf-empirical-article-001-latest-20260530-singlepdf-001",
        );
        let front_matter_path = artifact_dir.join("front_matter.json");
        if !front_matter_path.exists() {
            eprintln!(
                "skip empirical front matter load test: missing {}",
                front_matter_path.display()
            );
            return;
        }

        let front_matter = load_snapshot_front_matter(&front_matter_path);

        assert!(
            authors::has_academic_front_matter(&front_matter),
            "expected non-empty front matter from {}",
            front_matter_path.display()
        );
        assert!(
            front_matter
                .authors
                .iter()
                .any(|author| author.name.contains("Valentin Guigon")),
            "expected Valentin Guigon in loaded front matter: {:?}",
            front_matter.authors
        );
    }

    #[test]
    fn empirical_singlepdf_snapshot_renders_structured_front_matter() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let artifact_dir = manifest_dir.join(
            ".runtime/artifacts/gate-pdf-empirical-article-001-latest-20260530-singlepdf-001",
        );
        let markdown_path = artifact_dir.join("translated.md");
        let front_matter_path = artifact_dir.join("front_matter.json");
        if !markdown_path.exists() || !front_matter_path.exists() {
            eprintln!(
                "skip empirical structured front matter render test: missing {} or {}",
                markdown_path.display(),
                front_matter_path.display()
            );
            return;
        }

        let markdown = std::fs::read_to_string(&markdown_path).expect("read translated markdown");
        let front_matter = load_snapshot_front_matter(&front_matter_path);
        let html = render_translated_html_with_authors(&markdown, &front_matter);

        assert!(
            html.contains("academic-front-matter"),
            "expected structured front matter in rendered HTML"
        );
        assert!(
            html.contains("Valentin Guigon"),
            "expected author name in rendered HTML"
        );
        assert!(
            !html.contains("<p>https://doi.org/10.1038/s44271-024-00170-w</p>"),
            "expected raw DOI line to be removed from article body"
        );
        assert!(
            !html.contains("<p>妫€鏌ユ洿鏂?/p>"),
            "expected raw update notice to be removed from article body"
        );
    }

    fn load_snapshot_front_matter(path: &std::path::Path) -> authors::AcademicFrontMatter {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| {
                serde_json::from_str::<authors::AcademicFrontMatter>(&raw)
                    .ok()
                    .or_else(|| {
                        serde_json::from_str::<crate::pipeline::structure::FrontMatterExtraction>(
                            &raw,
                        )
                        .ok()
                        .map(crate::pipeline::structure::front_matter_to_academic_front_matter)
                    })
            })
            .unwrap_or_default()
    }

    #[test]
    fn academic_markdown_without_front_matter_does_not_fall_back_to_legacy_template() {
        let markdown = "# Metacognition biases information seeking in response to fake news\n\nReceived 17 January 2024; accepted 4 June 2024\n\nKeywords: misinformation; curiosity; metacognition\n\n## Abstract\n\nThis study examines how participants respond to ambiguous news.\n\n## References\n\nSmith, J. (2024). Journal of Testing.";

        let html = render_translated_html_with_authors(markdown, &Default::default());

        assert!(html.contains("article-shell"));
        assert!(!html.contains("The Classical Chronicle"));
        assert!(html.contains("Metacognition biases information seeking in response to fake news"));
    }
}
