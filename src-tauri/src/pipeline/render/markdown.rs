use super::super::layout::{
    parse_markdown_image, PDF_TABLE_END_MARKER, PDF_TABLE_IMAGE_MISSING_MARKER,
    PDF_TABLE_START_MARKER,
};
use crate::pipeline::artifact;
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub(super) fn render_markdown_to_html(markdown: &str) -> String {
    let mut renderer = MarkdownHtmlRenderer::new(artifact::ordered_heading_anchors(markdown));
    for raw_line in markdown.lines() {
        renderer.consume_line(raw_line.trim_end());
    }
    renderer.finish()
}

struct MarkdownHtmlRenderer {
    html: String,
    in_code_block: bool,
    list_depth: usize,
    code_buffer: String,
    pdf_table_buffer: String,
    in_pdf_table_block: bool,
    heading_anchors: Vec<String>,
    next_heading_anchor: usize,
    used_anchors: BTreeSet<String>,
}

impl MarkdownHtmlRenderer {
    fn new(heading_anchors: Vec<String>) -> Self {
        Self {
            html: fallback_html_prefix(),
            in_code_block: false,
            list_depth: 0,
            code_buffer: String::new(),
            pdf_table_buffer: String::new(),
            in_pdf_table_block: false,
            heading_anchors,
            next_heading_anchor: 0,
            used_anchors: BTreeSet::new(),
        }
    }

    fn consume_line(&mut self, line: &str) {
        if self.consume_pdf_table_line(line) {
            return;
        }
        if line.starts_with("```") {
            self.toggle_code_block();
            return;
        }
        if self.in_code_block {
            self.code_buffer.push_str(line);
            self.code_buffer.push('\n');
            return;
        }
        if line.is_empty() {
            self.close_list();
            return;
        }
        if self.consume_block_line(line) {
            return;
        }
        self.close_list();
        let _ = write!(self.html, "<p>{}</p>", inline_markdown(line));
    }

    fn finish(mut self) -> String {
        self.close_list();
        if self.in_code_block {
            self.flush_code_block();
        }
        if self.in_pdf_table_block {
            self.flush_pdf_table_block();
        }
        self.html.push_str("</body></html>");
        self.html
    }

    fn consume_pdf_table_line(&mut self, line: &str) -> bool {
        if line.trim() == PDF_TABLE_START_MARKER {
            self.close_list();
            self.in_pdf_table_block = true;
            self.pdf_table_buffer.clear();
            return true;
        }
        if !self.in_pdf_table_block {
            return false;
        }
        if line.trim() == PDF_TABLE_END_MARKER {
            self.flush_pdf_table_block();
            self.in_pdf_table_block = false;
            return true;
        }
        self.pdf_table_buffer.push_str(line);
        self.pdf_table_buffer.push('\n');
        true
    }

    fn consume_block_line(&mut self, line: &str) -> bool {
        if let Some(rendered) = self.render_heading_or_blockquote(line) {
            self.close_list();
            self.html.push_str(&rendered);
            return true;
        }
        if line == "---" || line == "***" {
            self.close_list();
            self.html.push_str("<hr>");
            return true;
        }
        if let Some((alt, src)) = parse_markdown_image(line) {
            self.close_list();
            self.push_image(&alt, &src);
            return true;
        }
        if let Some((depth, stripped)) = markdown_list_item(line) {
            self.push_list_item(depth, stripped);
            return true;
        }
        false
    }

    fn toggle_code_block(&mut self) {
        if self.in_code_block {
            self.flush_code_block();
            self.in_code_block = false;
        } else {
            self.close_list();
            self.in_code_block = true;
        }
    }

    fn flush_code_block(&mut self) {
        self.html.push_str("<pre><code>");
        self.html.push_str(&escape_html(&self.code_buffer));
        self.html.push_str("</code></pre>");
        self.code_buffer.clear();
    }

    fn close_list(&mut self) {
        while self.list_depth > 0 {
            self.html.push_str("</ul>");
            self.list_depth -= 1;
        }
    }

    fn flush_pdf_table_block(&mut self) {
        self.html
            .push_str("<figure class=\"media-block pdf-table-block pdf-table-image-block\">");
        if let Some((alt, src)) = first_table_image(&self.pdf_table_buffer) {
            let _ = write!(
                self.html,
                "<img src=\"{}\" alt=\"{}\" loading=\"lazy\">",
                escape_html(&src),
                escape_html(&alt)
            );
        } else {
            self.html
                .push_str("<div class=\"pdf-table-missing\">PDF table snapshot unavailable</div>");
        }
        self.html.push_str("</figure>");
        self.pdf_table_buffer.clear();
    }

    fn push_image(&mut self, alt: &str, src: &str) {
        let _ = write!(
            self.html,
            "<p><img src=\"{}\" alt=\"{}\" loading=\"lazy\"></p>",
            escape_html(src),
            escape_html(alt)
        );
    }

    fn push_list_item(&mut self, depth: usize, text: &str) {
        while self.list_depth < depth {
            self.html.push_str("<ul>");
            self.list_depth += 1;
        }
        while self.list_depth > depth {
            self.html.push_str("</ul>");
            self.list_depth -= 1;
        }
        let _ = write!(self.html, "<li>{}</li>", inline_markdown(text));
    }

    fn render_heading_or_blockquote(&mut self, line: &str) -> Option<String> {
        if let Some((level, stripped)) = markdown_heading(line) {
            let tag = match level {
                1 => "h1",
                2 => "h2",
                3 => "h3",
                4 => "h4",
                5 => "h5",
                6 => "h6",
                _ => return None,
            };
            let anchor = self.next_heading_anchor(stripped);
            return Some(format!(
                "<{tag} id=\"{anchor}\">{}</{tag}>",
                escape_html(stripped)
            ));
        }
        line.strip_prefix("> ")
            .map(|stripped| format!("<blockquote>{}</blockquote>", escape_html(stripped)))
    }

    fn next_heading_anchor(&mut self, title: &str) -> String {
        if let Some(anchor) = self.heading_anchors.get(self.next_heading_anchor) {
            self.next_heading_anchor += 1;
            self.used_anchors.insert(anchor.clone());
            return anchor.clone();
        }
        artifact::unique_heading_anchor(title, 1, 0, &mut self.used_anchors)
    }
}

fn fallback_html_prefix() -> String {
    String::from(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Translated Document</title><style>body{margin:0 auto;padding:32px 20px;max-width:860px;font:16px/1.75 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;color:#111827;background:#ffffff}h1,h2,h3,h4{line-height:1.25;margin:1.6em 0 .6em}p,ul,ol,blockquote,pre{margin:0 0 1em}pre{padding:16px;border-radius:12px;background:#f3f4f6;overflow:auto}code{font-family:ui-monospace,SFMono-Regular,Consolas,monospace}blockquote{margin-left:0;padding-left:16px;border-left:3px solid #d1d5db;color:#4b5563}img{max-width:100%;height:auto;border-radius:12px}hr{border:none;border-top:1px solid #e5e7eb;margin:24px 0}li+li{margin-top:.35em}</style></head><body>",
    )
}

fn markdown_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = trimmed.get(level..)?.trim();
    (!rest.is_empty()).then_some((level as u8, rest))
}

fn markdown_list_item(line: &str) -> Option<(usize, &str)> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    let trimmed = &line[indent..];
    let stripped = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))?;
    Some((indent / 2 + 1, stripped))
}

fn inline_markdown(line: &str) -> String {
    let mut output = render_links(&render_inline_tex_superscripts(line));
    output = output.replace("**", "<strong>");
    pair_replace(&output, "<strong>")
}

fn render_inline_tex_superscripts(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line;
    while let Some((prefix, marker, suffix)) = take_next_tex_superscript(rest) {
        output.push_str(&escape_html(prefix));
        let _ = write!(output, "<sup>{}</sup>", escape_html(marker.trim()));
        rest = suffix;
    }
    output.push_str(&escape_html(rest));
    output
}

fn take_next_tex_superscript(input: &str) -> Option<(&str, &str, &str)> {
    let dollar_start = input.find('$')?;
    let after_dollar = &input[dollar_start + 1..];
    let trimmed_start = after_dollar.trim_start();
    let skipped_start = after_dollar.len() - trimmed_start.len();
    let after_caret = trimmed_start.strip_prefix('^')?;
    let after_open = after_caret.trim_start().strip_prefix('{')?;
    let marker_end = after_open.find('}')?;
    let marker = &after_open[..marker_end];
    if !is_safe_superscript_marker(marker) {
        return find_later_tex_superscript(input, dollar_start);
    }
    let after_marker = &after_open[marker_end + 1..];
    let trimmed_end = after_marker.trim_start();
    if !trimmed_end.starts_with('$') {
        return find_later_tex_superscript(input, dollar_start);
    }
    let skipped_end = after_marker.len() - trimmed_end.len();
    let suffix_start = dollar_start
        + 1
        + skipped_start
        + 1
        + (after_caret.len() - after_caret.trim_start().len())
        + 1
        + marker_end
        + 1
        + skipped_end
        + 1;
    Some((&input[..dollar_start], marker, &input[suffix_start..]))
}

fn find_later_tex_superscript(input: &str, dollar_start: usize) -> Option<(&str, &str, &str)> {
    let next_start = dollar_start + 1;
    let (skipped, tail) = input.split_at(next_start);
    let (prefix, marker, suffix) = take_next_tex_superscript(tail)?;
    Some((&input[..skipped.len() + prefix.len()], marker, suffix))
}

fn is_safe_superscript_marker(marker: &str) -> bool {
    let marker = marker.trim();
    !marker.is_empty()
        && marker.len() <= 32
        && marker
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ',' | '-' | '*' | ' ' | ';' | '.'))
}

fn pair_replace(input: &str, tag: &str) -> String {
    let mut result = String::new();
    let mut parts = input.split(tag).peekable();
    let mut open = true;
    while let Some(part) = parts.next() {
        result.push_str(part);
        if parts.peek().is_some() {
            if open {
                result.push_str(tag);
            } else {
                result.push_str("</strong>");
            }
            open = !open;
        }
    }
    result
}

fn render_links(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let Some(label_start) = rest.find('[') else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..label_start]);
        let after_label_start = &rest[label_start + 1..];
        let Some(label_end) = after_label_start.find(']') else {
            output.push_str(&rest[label_start..]);
            break;
        };
        let label = &after_label_start[..label_end];
        let after_label = &after_label_start[label_end + 1..];
        let Some(after_paren) = after_label.strip_prefix('(') else {
            output.push_str(&rest[label_start..label_start + 1]);
            rest = after_label_start;
            continue;
        };
        let Some(href_end) = after_paren.find(')') else {
            output.push_str(&rest[label_start..]);
            break;
        };
        let href = &after_paren[..href_end];
        output.push_str(&format!("<a href=\"{}\">{}</a>", href, label));
        rest = &after_paren[href_end + 1..];
    }
    output
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn first_table_image(raw: &str) -> Option<(String, String)> {
    raw.lines()
        .map(str::trim)
        .filter(|line| *line != PDF_TABLE_IMAGE_MISSING_MARKER)
        .find_map(parse_markdown_image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_headings_receive_stable_ids() {
        let html = render_markdown_to_html("# 引言\n\n## 方法\n\n## 方法");

        assert!(html.contains("<h1 id=\"引言\">引言</h1>"));
        assert!(html.contains("<h2 id=\"方法-1\">方法</h2>"));
        assert!(html.contains("<h2 id=\"方法-2\">方法</h2>"));
    }

    #[test]
    fn renders_markdown_links_inside_list_items() {
        let html = render_markdown_to_html("- [引言](#引言)");

        assert!(html.contains("<ul><li><a href=\"#引言\">引言</a></li></ul>"));
    }

    #[test]
    fn renders_nested_lists_from_indented_markdown_items() {
        let html = render_markdown_to_html("- [引言](#引言)\n  - [方法](#方法)");

        assert!(html.contains(
            "<ul><li><a href=\"#引言\">引言</a></li><ul><li><a href=\"#方法\">方法</a></li></ul></ul>"
        ));
    }

    #[test]
    fn renders_protected_pdf_table_as_snapshot_only() {
        let markdown = format!(
            "{}\n<table><tr><td>A</td></tr></table>\n{}",
            PDF_TABLE_START_MARKER, PDF_TABLE_END_MARKER
        );

        let html = render_markdown_to_html(&markdown);

        assert!(html.contains("pdf-table-image-block"));
        assert!(html.contains("PDF table snapshot unavailable"));
        assert!(!html.contains("<table><tr><td>A</td></tr></table>"));
        assert!(!html.contains("&lt;table&gt;"));
    }

    #[test]
    fn renders_pdf_table_snapshot_image_when_available() {
        let markdown = format!(
            "{}\n![Table 1](imgs/table_1.png)\n{}",
            PDF_TABLE_START_MARKER, PDF_TABLE_END_MARKER
        );

        let html = render_markdown_to_html(&markdown);

        assert!(html.contains("pdf-table-image-block"));
        assert!(html.contains("<img src=\"imgs/table_1.png\" alt=\"Table 1\" loading=\"lazy\">"));
        assert!(!html.contains("PDF table snapshot unavailable"));
    }

    #[test]
    fn renders_tex_superscript_citations_document_wide() {
        let html = render_markdown_to_html("研究显示 $ ^{1-4} $ 后续工作支持该结论 $^{9,10}$。");

        assert!(html.contains("研究显示 <sup>1-4</sup> 后续工作支持该结论 <sup>9,10</sup>。"));
    }

    #[test]
    fn preserves_bracket_citation_style() {
        let html = render_markdown_to_html("参考文献 [1] 仍应保留方括号。");

        assert!(html.contains("<p>参考文献 [1] 仍应保留方括号。</p>"));
        assert!(!html.contains("<sup>1</sup>"));
    }

    #[test]
    fn leaves_tex_superscripts_inside_code_blocks_unrendered() {
        let html = render_markdown_to_html("```\n$ ^{1} $\n```");

        assert!(html.contains("<pre><code>$ ^{1} $"));
        assert!(!html.contains("<sup>1</sup>"));
    }

    #[test]
    fn leaves_general_inline_math_as_plain_text() {
        let html = render_markdown_to_html("公式 $x+y$ 不在当前最小契约内。");

        assert!(html.contains("<p>公式 $x+y$ 不在当前最小契约内。</p>"));
        assert!(!html.contains("<sup>"));
    }
}
