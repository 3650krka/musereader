use super::super::chunking::is_non_reference_tail_heading;
use super::super::layout::is_figure_or_table_label;

pub(super) fn postprocess_academic_article_html(html: &str, references_heading_zh: &str) -> String {
    let mut output = promote_media_labels(html);
    output = close_media_blocks(output);
    output = attach_table_metadata_to_pdf_blocks(&output);
    output = wrap_reference_block(&output, references_heading_zh);
    output
}

pub(super) fn postprocess_block_rendered_academic_html(
    html: &str,
    references_heading_zh: &str,
) -> String {
    let output = attach_table_metadata_to_pdf_blocks(html);
    wrap_reference_block(&output, references_heading_zh)
}

pub(super) fn apply_article_template(
    template: &str,
    title: &str,
    rendered_body: &str,
) -> Option<String> {
    let article_markup = build_template_article_markup(title, rendered_body);
    let html = replace_between(template, "<title>", "</title>", &escape_html(title))?;
    replace_article_markup(&html, &article_markup)
}

fn build_template_article_markup(title: &str, rendered_body: &str) -> String {
    format!(
        "<article class=\"px-8 md:px-16\"><header class=\"text-center mb-12\"><h1 class=\"text-3xl md:text-[2.5rem] font-bold leading-snug mb-6 text-[#2B2824]\">{}</h1></header><div class=\"article-body\">{}</div><div class=\"ornament-divider\"><span>✦</span></div></article>",
        escape_html(title),
        rendered_body
    )
}

fn promote_media_labels(html: &str) -> String {
    let mut output = String::with_capacity(html.len() + 64);
    let mut first = true;
    for segment in html.split("<p>") {
        if first {
            output.push_str(segment);
            first = false;
            continue;
        }
        if let Some(label) = segment.strip_suffix("</p>") {
            let trimmed = label.trim();
            if is_figure_or_table_label(trimmed) {
                output.push_str(&format!(
                    "<figure class=\"media-block\"><div class=\"media-label\">{}</div>",
                    escape_html(trimmed)
                ));
                continue;
            }
        }
        output.push_str("<p>");
        output.push_str(segment);
    }
    output
}

fn close_media_blocks(html: String) -> String {
    let mut output = String::with_capacity(html.len() + 64);
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<figure class=\"media-block\">") {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some((figure, consumed)) = consume_media_figure_html(rest) {
            output.push_str(&figure);
            rest = &rest[consumed..];
        } else {
            let Some((paragraph, consumed)) = demote_media_label(rest) else {
                output.push_str(rest);
                return output;
            };
            output.push_str(&paragraph);
            rest = &rest[consumed..];
        }
    }
    output.push_str(rest);
    output
}

fn consume_media_figure_html(input: &str) -> Option<(String, usize)> {
    let label_end = input.find("</div>")? + "</div>".len();
    let after_label = &input[label_end..];
    let (image_html, image_consumed) = consume_image_paragraphs(after_label)?;
    if image_html.is_empty() {
        return None;
    }
    let after_image = &after_label[image_consumed..];
    let (caption_html, caption_consumed) = consume_figure_caption(after_image)
        .or_else(|| consume_next_paragraph(after_image))
        .unwrap_or_else(|| ("".to_string(), 0usize));
    Some((
        build_media_figure_html(input, label_end, &image_html, &caption_html),
        label_end + image_consumed + caption_consumed,
    ))
}

fn consume_image_paragraphs(input: &str) -> Option<(String, usize)> {
    let mut consumed = 0usize;
    let mut images = String::new();
    let mut rest = input;
    while let Some((paragraph, paragraph_len)) = consume_next_paragraph(rest) {
        if !paragraph.contains("<img ") {
            break;
        }
        images.push_str(&paragraph);
        consumed += paragraph_len;
        rest = &rest[paragraph_len..];
    }
    (!images.is_empty()).then_some((images, consumed))
}

fn demote_media_label(input: &str) -> Option<(String, usize)> {
    let label_end = input.find("</div>")? + "</div>".len();
    let label_html = &input[..label_end];
    let label_start =
        label_html.find("<div class=\"media-label\">")? + "<div class=\"media-label\">".len();
    let label = &label_html[label_start..label_html.len() - "</div>".len()];
    Some((format!("<p>{label}</p>"), label_end))
}

fn build_media_figure_html(
    input: &str,
    label_end: usize,
    image_html: &str,
    caption_html: &str,
) -> String {
    let mut figure = String::new();
    figure.push_str(&input[..label_end]);
    figure.push_str(image_html);
    if !caption_html.is_empty() {
        figure.push_str(&paragraph_to_figcaption(caption_html));
    }
    figure.push_str("</figure>");
    figure
}

fn consume_next_paragraph(input: &str) -> Option<(String, usize)> {
    if !input.starts_with("<p>") {
        return None;
    }
    let end = input.find("</p>")? + "</p>".len();
    Some((input[..end].to_string(), end))
}

fn paragraph_to_figcaption(paragraph: &str) -> String {
    let content = paragraph
        .strip_prefix("<p>")
        .and_then(|value| value.strip_suffix("</p>"))
        .unwrap_or(paragraph);
    format!("<figcaption>{content}</figcaption>")
}

fn consume_figure_caption(input: &str) -> Option<(String, usize)> {
    consume_html_figure_caption(input).or_else(|| consume_markdown_figure_caption(input))
}

fn consume_html_figure_caption(input: &str) -> Option<(String, usize)> {
    let opening = "<p>::: figure-caption</p>";
    if !input.starts_with(opening) {
        return None;
    }

    let mut consumed = opening.len();
    let mut rest = &input[consumed..];
    let mut paragraphs = Vec::new();

    while let Some((paragraph, paragraph_len)) = consume_next_paragraph(rest) {
        let body = paragraph
            .strip_prefix("<p>")
            .and_then(|value| value.strip_suffix("</p>"))
            .map(str::trim)
            .unwrap_or_default();
        if body == ":::" {
            consumed += paragraph_len;
            let content = paragraphs.join("</p><p>");
            return Some((format!("<p>{content}</p>"), consumed));
        }
        paragraphs.push(
            paragraph
                .strip_prefix("<p>")
                .and_then(|value| value.strip_suffix("</p>"))
                .unwrap_or(&paragraph)
                .to_string(),
        );
        consumed += paragraph_len;
        rest = &rest[paragraph_len..];
    }

    None
}

fn consume_markdown_figure_caption(input: &str) -> Option<(String, usize)> {
    let start = "::: figure-caption";
    if !input.starts_with(start) {
        return None;
    }
    let after_start = &input[start.len()..];
    let after_newline = after_start.strip_prefix('\n').unwrap_or(after_start);
    let end_marker = "\n:::";
    let end = after_newline.find(end_marker)?;
    let content = after_newline[..end].trim();
    let consumed = start.len() + (after_start.len() - after_newline.len()) + end + end_marker.len();
    Some((format!("<p>{}</p>", escape_html(content)), consumed))
}

fn attach_table_metadata_to_pdf_blocks(html: &str) -> String {
    let marker = "<figure class=\"media-block pdf-table-block";
    let mut output = String::with_capacity(html.len() + 64);
    let mut rest = html;

    while let Some(start) = rest.find(marker) {
        let before = &rest[..start];
        let (trimmed_before, label) = strip_trailing_table_labels(before);
        output.push_str(trimmed_before);

        let figure_rest = &rest[start..];
        let Some(open_end_relative) = figure_rest.find('>') else {
            output.push_str(figure_rest);
            return output;
        };
        let open_end = open_end_relative + 1;
        output.push_str(&figure_rest[..open_end]);
        if let Some(label) = label {
            output.push_str("<div class=\"media-label\">");
            output.push_str(&escape_html(&label));
            output.push_str("</div>");
        }
        let remaining = &figure_rest[open_end..];
        if let Some(close_relative) = remaining.find("</figure>") {
            output.push_str(&remaining[..close_relative]);
            let after_figure = &remaining[close_relative + "</figure>".len()..];
            let (after_note, note_html) = strip_leading_table_note(after_figure);
            if let Some(note_html) = note_html.as_deref() {
                output.push_str(&blockquote_to_figcaption(note_html));
            }
            output.push_str("</figure>");
            rest = after_note;
            continue;
        }
        rest = remaining;
    }

    output.push_str(rest);
    output
}

fn strip_trailing_table_labels(input: &str) -> (&str, Option<String>) {
    let mut cursor = input.len();
    let mut labels = Vec::new();

    while let Some((start, end, body)) = trailing_paragraph(input, cursor) {
        if !is_table_label(body.trim()) {
            break;
        }
        labels.push(body.trim().to_string());
        cursor = start;
        if start == 0 || end == 0 {
            break;
        }
    }

    if labels.is_empty() {
        return (input, None);
    }
    let preferred = labels.first().cloned();
    (input[..cursor].trim_end(), preferred)
}

fn trailing_paragraph(input: &str, end: usize) -> Option<(usize, usize, &str)> {
    let prefix = input[..end].trim_end_matches(char::is_whitespace);
    let trimmed_end = prefix.len();
    if trimmed_end < "</p>".len() || !prefix.ends_with("</p>") {
        return None;
    }
    let start = prefix.rfind("<p>")?;
    let body_start = start + "<p>".len();
    let body_end = trimmed_end - "</p>".len();
    Some((start, trimmed_end, &prefix[body_start..body_end]))
}

fn blockquote_to_figcaption(blockquote: &str) -> String {
    let content = blockquote
        .strip_prefix("<blockquote>")
        .and_then(|value| value.strip_suffix("</blockquote>"))
        .unwrap_or(blockquote);
    format!("<figcaption>{content}</figcaption>")
}

fn strip_leading_table_note(input: &str) -> (&str, Option<String>) {
    let trimmed = input.trim_start_matches(char::is_whitespace);
    let offset = input.len() - trimmed.len();
    if let Some(rest) = trimmed.strip_prefix("<blockquote>") {
        if let Some(end_relative) = rest.find("</blockquote>") {
            let end = offset + "<blockquote>".len() + end_relative + "</blockquote>".len();
            return (
                &input[end..],
                Some(
                    trimmed[.."<blockquote>".len() + end_relative + "</blockquote>".len()]
                        .to_string(),
                ),
            );
        }
    }
    (input, None)
}

fn is_table_label(value: &str) -> bool {
    let normalized = value.trim();
    if let Some(number) = normalized.strip_prefix("Table ") {
        return table_label_number_is_valid(number);
    }
    if let Some(number) = normalized.strip_prefix("表") {
        return table_label_number_is_valid(number);
    }
    false
}

fn table_label_number_is_valid(raw: &str) -> bool {
    let number = raw
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, '.' | ':' | '：' | '|' | '-' | '–' | '—'))
        .trim();
    let digits = number
        .chars()
        .take_while(|ch| {
            ch.is_ascii_digit() || matches!(ch, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')
        })
        .collect::<String>();
    !digits.is_empty()
}

pub(super) fn wrap_reference_block(html: &str, references_heading_zh: &str) -> String {
    let reference_heading = format!("<h2>{references_heading_zh}</h2>");
    let Some(start) = html.find(&reference_heading) else {
        return html.to_string();
    };
    let content_start = start + reference_heading.len();
    let end = find_reference_block_html_end(&html[content_start..])
        .map(|offset| content_start + offset)
        .unwrap_or(html.len());
    let mut output = String::with_capacity(html.len() + 64);
    output.push_str(&html[..start]);
    output.push_str("<section class=\"reference-block\">");
    output.push_str(&html[start..end]);
    output.push_str("</section>");
    output.push_str(&html[end..]);
    output
}

fn find_reference_block_html_end(html_after_reference_heading: &str) -> Option<usize> {
    let mut search_from = 0usize;
    loop {
        let relative_start = html_after_reference_heading[search_from..].find("<h2>")?;
        let heading_start = search_from + relative_start;
        let heading_body_start = heading_start + "<h2>".len();
        let relative_end = html_after_reference_heading[heading_body_start..].find("</h2>")?;
        let heading_body_end = heading_body_start + relative_end;
        let heading = html_after_reference_heading[heading_body_start..heading_body_end].trim();
        if is_non_reference_tail_heading(&format!("## {heading}")) {
            return Some(heading_start);
        }
        search_from = heading_body_end + "</h2>".len();
    }
}

fn replace_between(input: &str, start: &str, end: &str, replacement: &str) -> Option<String> {
    let start_at = input.find(start)?;
    let content_start = start_at + start.len();
    let end_at = input[content_start..].find(end)? + content_start;
    let mut output = String::with_capacity(input.len() + replacement.len());
    output.push_str(&input[..content_start]);
    output.push_str(replacement);
    output.push_str(&input[end_at..]);
    Some(output)
}

fn replace_article_markup(input: &str, replacement: &str) -> Option<String> {
    let start = input.find("<article")?;
    let open_end = input[start..].find('>')? + start + 1;
    let end = input[open_end..].find("</article>")? + open_end + "</article>".len();
    let mut output = String::with_capacity(input.len() + replacement.len());
    output.push_str(&input[..start]);
    output.push_str(replacement);
    output.push_str(&input[end..]);
    Some(output)
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_split_figure_images_into_one_media_block() {
        let html = concat!(
            "<p>Before.</p>",
            "<p>Figure 2:</p>",
            "<p><img src=\"a.png\" alt=\"a\" loading=\"lazy\"></p>",
            "<p><img src=\"b.png\" alt=\"b\" loading=\"lazy\"></p>",
            "<p>Composite caption.</p>",
            "<p>After.</p>"
        );

        let output = postprocess_academic_article_html(html, "参考文献");

        assert!(output
            .contains("<figure class=\"media-block\"><div class=\"media-label\">Figure 2:</div>"));
        assert!(output.contains(
            "<p><img src=\"a.png\" alt=\"a\" loading=\"lazy\"></p><p><img src=\"b.png\" alt=\"b\" loading=\"lazy\"></p>"
        ));
        assert!(
            output.contains("<figcaption>Composite caption.</figcaption></figure><p>After.</p>")
        );
    }

    #[test]
    fn keeps_non_image_after_media_label_outside_media_block() {
        let html = "<p>Figure 2:</p><p>No image here.</p><p>After.</p>";

        let output = postprocess_academic_article_html(html, "参考文献");

        assert!(output.contains("<p>Figure 2:</p><p>No image here.</p><p>After.</p>"));
        assert!(!output.contains("<figure class=\"media-block\"><div class=\"media-label\">Figure 2:</div><p>No image here.</p>"));
    }

    #[test]
    fn consumes_explicit_figure_caption_block_into_figcaption() {
        let html = concat!(
            "<p>Figure 4</p>",
            "<p><img src=\"fig4.png\" alt=\"fig4\" loading=\"lazy\"></p>",
            "::: figure-caption\n",
            "Distribution by treatment arm.\n",
            ":::",
            "<p>After.</p>"
        );

        let output = postprocess_academic_article_html(html, "参考文献");

        assert!(output
            .contains("<figure class=\"media-block\"><div class=\"media-label\">Figure 4</div>"));
        assert!(output.contains(
            "<figcaption>Distribution by treatment arm.</figcaption></figure><p>After.</p>"
        ));
    }

    #[test]
    fn consumes_html_figure_caption_block_into_figcaption() {
        let html = concat!(
            "<p>Figure 1</p>",
            "<p><img src=\"fig1.png\" alt=\"fig1\" loading=\"lazy\"></p>",
            "<p>::: figure-caption</p>",
            "<p>任务描述。</p>",
            "<p>鉴于他们的选择，参与者需要继续报价。</p>",
            "<p>:::</p>",
            "<p>After.</p>"
        );

        let output = postprocess_academic_article_html(html, "参考文献");

        assert!(output.contains(
            "<figcaption>任务描述。</p><p>鉴于他们的选择，参与者需要继续报价。</figcaption>"
        ));
        assert!(!output.contains("::: figure-caption"));
        assert!(!output.contains("<p>:::</p>"));
    }

    #[test]
    fn attaches_single_table_label_to_pdf_table_block() {
        let html = concat!(
            "<p>Before.</p>",
            "<p>Table 1</p>",
            "<figure class=\"media-block pdf-table-block pdf-table-image-block\">",
            "<img src=\"imgs/table_snapshot_001.jpg\" alt=\"PDF table 1\" loading=\"lazy\">",
            "</figure>",
            "<blockquote>Note.</blockquote>"
        );

        let output = postprocess_academic_article_html(html, "参考文献");

        assert!(output.contains(
            "<figure class=\"media-block pdf-table-block pdf-table-image-block\"><div class=\"media-label\">Table 1</div><img src=\"imgs/table_snapshot_001.jpg\" alt=\"PDF table 1\" loading=\"lazy\"><figcaption>Note.</figcaption></figure>"
        ));
        assert_eq!(output.matches("<p>Table 1</p>").count(), 0);
        assert_eq!(output.matches("<blockquote>Note.</blockquote>").count(), 0);
    }

    #[test]
    fn attaches_zh_table_label_to_pdf_table_block() {
        let html = concat!(
            "<p>表1 | 助推+的一些工作实例。</p>",
            "<figure class=\"media-block pdf-table-block pdf-table-image-block\">",
            "<img src=\"imgs/table_snapshot_001.jpg\" alt=\"PDF table 1\" loading=\"lazy\">",
            "</figure>"
        );

        let output = postprocess_academic_article_html(html, "鍙傝€冩枃鐚?");

        assert_eq!(
            output.matches("<p>表1 | 助推+的一些工作实例。</p>").count(),
            0
        );
        assert!(output.contains(
            "<figure class=\"media-block pdf-table-block pdf-table-image-block\"><div class=\"media-label\">表1 | 助推+的一些工作实例。</div><img src=\"imgs/table_snapshot_001.jpg\" alt=\"PDF table 1\" loading=\"lazy\"></figure>"
        ));
    }

    #[test]
    fn collapses_duplicate_table_labels_before_pdf_table_block() {
        let html = concat!(
            "<p>Table 1</p>",
            "<p>Table 1</p>",
            "<figure class=\"media-block pdf-table-block pdf-table-image-block\">",
            "<img src=\"imgs/table_snapshot_001.jpg\" alt=\"PDF table 1\" loading=\"lazy\">",
            "</figure>"
        );

        let output = postprocess_academic_article_html(html, "参考文献");

        assert_eq!(output.matches("<p>Table 1</p>").count(), 0);
        assert_eq!(
            output
                .matches("<div class=\"media-label\">Table 1</div>")
                .count(),
            1
        );
    }

    #[test]
    fn replaces_entire_template_article_instead_of_inner_body_only() {
        let template = r#"<!DOCTYPE html><html><head><title>Old</title></head><body><main><article class="px-8 md:px-16"><header><h1>Old title</h1></header><div class="article-body"><p>Old body</p></div><div class="ornament-divider"><span>✦</span></div><footer>Old footer</footer></article></main></body></html>"#;

        let output = apply_article_template(template, "New Title", "<p>New body</p>")
            .expect("template should render");

        assert!(output.contains("<title>New Title</title>"));
        assert!(output.contains("<h1 class=\"text-3xl md:text-[2.5rem] font-bold leading-snug mb-6 text-[#2B2824]\">New Title</h1>"));
        assert!(output.contains("<div class=\"article-body\"><p>New body</p></div>"));
        assert!(!output.contains("Old title"));
        assert!(!output.contains("Old body"));
        assert!(!output.contains("Old footer"));
    }
}
