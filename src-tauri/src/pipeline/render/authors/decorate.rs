use super::{escape_html, AcademicAuthor};

#[cfg(test)]
pub(super) fn decorate_academic_authors(html: &str, authors: &[AcademicAuthor]) -> String {
    let html = convert_author_marker_math_to_sup(html);
    let Some((start, end, wrapped)) = author_fragment_range(&html, authors) else {
        return html;
    };

    let mut paragraph = html[start..end].to_string();
    for author in authors {
        paragraph = replace_author_fragment(&paragraph, author);
    }
    paragraph = mark_author_fragment(&paragraph, wrapped);

    let mut decorated = String::with_capacity(html.len() + paragraph.len());
    decorated.push_str(&html[..start]);
    decorated.push_str(&paragraph);
    decorated.push_str(&html[end..]);
    decorated
}

pub(super) fn render_academic_front_matter(
    authors: &[AcademicAuthor],
    extras: &[String],
) -> String {
    let mut sections = Vec::new();
    if !authors.is_empty() {
        sections.push(render_author_line(authors));
    }
    let extras = extras
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .filter(|item| !is_hidden_front_matter_extra(item))
        .collect::<Vec<_>>();
    if !extras.is_empty() {
        sections.push(render_front_matter_extras(&extras));
    }
    if sections.is_empty() {
        String::new()
    } else {
        format!(
            "<section class=\"academic-front-matter\">{}</section>",
            sections.join("")
        )
    }
}

fn render_author_line(authors: &[AcademicAuthor]) -> String {
    let mut line = String::from("<p class=\"academic-author-line\">");
    for author in authors {
        line.push_str(&render_author_chip(author));
    }
    line.push_str("</p>");
    line
}

fn render_author_chip(author: &AcademicAuthor) -> String {
    let name = escape_html(&author.name);
    let suffix = marker_markup(&author.markers);
    let popup = build_author_popup_html(author);
    let href = author
        .email
        .as_ref()
        .map(|email| format!("mailto:{}", escape_html(email)))
        .unwrap_or_else(|| format!("#{}", author_anchor_id(author)));
    if popup.is_empty() {
        return format!(
            "<span class=\"academic-author-chip\"><a class=\"academic-author-link\" href=\"{href}\">{name}</a>{suffix}</span>"
        );
    }
    format!(
        "<span class=\"academic-author-chip\"><a class=\"academic-author-link\" href=\"{href}\">{name}</a>{suffix}<span id=\"{}\" class=\"academic-author-popover\" role=\"tooltip\">{popup}</span></span>",
        author_anchor_id(author)
    )
}

fn render_front_matter_extras(extras: &[&str]) -> String {
    let items = extras
        .iter()
        .map(|item| render_front_matter_extra_item(item))
        .collect::<Vec<_>>()
        .join("");
    format!("<div class=\"academic-front-matter-extras\">{items}</div>")
}

fn render_front_matter_extra_item(item: &str) -> String {
    let trimmed = item.trim();
    let escaped = escape_html(trimmed);
    if trimmed.to_ascii_lowercase().starts_with("https://doi.org/")
        || trimmed.to_ascii_lowercase().starts_with("http://doi.org/")
    {
        return format!(
            "<span class=\"academic-front-matter-extra\"><span>DOI:</span><a href=\"{escaped}\" target=\"_blank\" rel=\"noreferrer\">{escaped}</a></span>"
        );
    }
    format!("<span class=\"academic-front-matter-extra\">{escaped}</span>")
}

fn is_hidden_front_matter_extra(item: &str) -> bool {
    let normalized = item.trim().to_ascii_lowercase();
    normalized == "check for updates"
        || normalized == "检查更新"
        || normalized == "检测更新"
        || normalized == "查看更新"
        || normalized == "注意更新"
}

#[cfg(test)]
fn author_fragment_range(html: &str, authors: &[AcademicAuthor]) -> Option<(usize, usize, bool)> {
    let first_author = authors.first().map(|author| escape_html(&author.name))?;
    let heading_boundary = html.find("<h2").unwrap_or(html.len());
    let search_scope = &html[..heading_boundary];

    let mut cursor = 0usize;
    while let Some(relative_start) = search_scope[cursor..].find("<p") {
        let start = cursor + relative_start;
        let Some(relative_end) = search_scope[start..].find("</p>") else {
            break;
        };
        let end = start + relative_end + "</p>".len();
        let paragraph = &search_scope[start..end];
        if paragraph.contains(&first_author) {
            return Some((start, end, true));
        }
        cursor = end;
    }

    let name_start = search_scope.find(&first_author)?;
    let fragment_start = search_scope[..name_start]
        .rfind("<p>")
        .map(|index| index + "<p>".len())
        .or_else(|| {
            search_scope[..name_start]
                .rfind("</p>")
                .map(|index| index + "</p>".len())
        })
        .unwrap_or(name_start);
    let fragment_end = search_scope[name_start..]
        .find("</p>")
        .map(|offset| name_start + offset + "</p>".len())
        .unwrap_or(heading_boundary);
    let fragment = &search_scope[fragment_start..fragment_end];
    fragment.contains(&first_author).then_some((
        fragment_start,
        fragment_end,
        fragment_start < name_start,
    ))
}

#[cfg(test)]
fn mark_author_fragment(fragment: &str, wrapped: bool) -> String {
    if wrapped {
        fragment.replacen("<p>", "<p class=\"academic-author-line\">", 1)
    } else {
        let content = fragment
            .trim()
            .strip_suffix("</p>")
            .unwrap_or(fragment.trim())
            .trim();
        format!("<p class=\"academic-author-line\">{content}</p>")
    }
}

#[cfg(test)]
fn replace_author_fragment(paragraph: &str, author: &AcademicAuthor) -> String {
    let escaped_name = escape_html(&author.name);
    let Some(name_start) = paragraph.find(&escaped_name) else {
        return paragraph.to_string();
    };
    let suffix = author_suffix_markup(paragraph, name_start, author);
    let fragment_end =
        find_author_fragment_end(paragraph, name_start + escaped_name.len(), &suffix, author);
    let popup = build_author_popup_html(author);
    let href = author
        .email
        .as_ref()
        .map(|email| format!("mailto:{}", escape_html(email)))
        .unwrap_or_else(|| format!("#{}", author_anchor_id(author)));

    let replacement = if popup.is_empty() {
        format!(
            "<span class=\"academic-author-chip\"><a class=\"academic-author-link\" href=\"{}\">{}</a>{}</span>",
            href, escaped_name, suffix
        )
    } else {
        format!(
            "<span class=\"academic-author-chip\"><a class=\"academic-author-link\" href=\"{}\">{}</a>{}<span id=\"{}\" class=\"academic-author-popover\" role=\"tooltip\">{}</span></span>",
            href,
            escaped_name,
            suffix,
            author_anchor_id(author),
            popup
        )
    };

    let mut decorated = String::with_capacity(paragraph.len() + replacement.len());
    decorated.push_str(&paragraph[..name_start]);
    decorated.push_str(&replacement);
    decorated.push_str(&paragraph[fragment_end..]);
    decorated
}

#[cfg(test)]
fn find_author_fragment_end(
    paragraph: &str,
    name_end: usize,
    suffix: &str,
    author: &AcademicAuthor,
) -> usize {
    if let Some(length) = ocr_author_marker_suffix_len(&paragraph[name_end..], author) {
        return name_end + length;
    }
    if suffix.is_empty() {
        return name_end;
    }
    paragraph[name_end..]
        .find(&suffix[..])
        .map(|offset| name_end + offset + suffix.len())
        .unwrap_or(name_end)
}

#[cfg(test)]
fn author_suffix_markup(paragraph: &str, name_start: usize, author: &AcademicAuthor) -> String {
    let suffix_start = name_start + escape_html(&author.name).len();
    let tail = &paragraph[suffix_start..];

    if let Some(existing) = extract_existing_suffix_markup(tail) {
        return existing;
    }

    marker_markup(&author.markers)
}

#[cfg(test)]
fn extract_existing_suffix_markup(tail: &str) -> Option<String> {
    let mut end = 0usize;
    let mut rest = tail;

    if rest.starts_with("* ID") || rest.starts_with(" ID") {
        return None;
    }

    if rest.starts_with('*') {
        end += '*'.len_utf8();
        rest = &rest['*'.len_utf8()..];
    }

    if rest.starts_with("<sup>") {
        let local_end = rest.find("</sup>")? + "</sup>".len();
        end += local_end;
        return Some(tail[..end].to_string());
    }

    if rest.starts_with('（') || rest.starts_with('(') {
        let open = rest.chars().next()?;
        let close = if open == '（' { '）' } else { ')' };
        let start = open.len_utf8();
        let local_end = rest[start..].find(close)? + start + close.len_utf8();
        end += local_end;
        return Some(tail[..end].to_string());
    }

    (end > 0).then(|| tail[..end].to_string())
}

#[cfg(test)]
fn ocr_author_marker_suffix_len(tail: &str, author: &AcademicAuthor) -> Option<usize> {
    if !author.markers.iter().any(|marker| marker == "ID") {
        return None;
    }
    if tail.starts_with("* ID") {
        return Some("* ID".len());
    }
    if tail.starts_with(" ID") {
        return Some(" ID".len());
    }
    None
}

fn marker_markup(markers: &[String]) -> String {
    if markers.is_empty() {
        String::new()
    } else {
        format!("<sup>{}</sup>", escape_html(&markers.join(",")))
    }
}

#[cfg(test)]
pub(super) fn convert_author_marker_math_to_sup(html: &str) -> String {
    let normalized = convert_simple_author_marker_math_to_sup(html);
    convert_textcircled_author_marker_math_to_sup(&normalized)
}

#[cfg(test)]
fn convert_simple_author_marker_math_to_sup(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(relative_start) = html[cursor..].find("$ ^{") {
        let start = cursor + relative_start;
        output.push_str(&html[cursor..start]);
        let marker_start = start + "$ ^{".len();
        let Some(relative_end) = html[marker_start..].find("} $") else {
            output.push_str(&html[start..]);
            return output;
        };
        let marker_end = marker_start + relative_end;
        let marker = html[marker_start..marker_end].trim();
        if is_author_marker(marker) {
            output.push_str(&format!("<sup>{}</sup>", escape_html(marker)));
        } else {
            output.push_str(&html[start..marker_end + "} $".len()]);
        }
        cursor = marker_end + "} $".len();
    }
    output.push_str(&html[cursor..]);
    output
}

#[cfg(test)]
fn convert_textcircled_author_marker_math_to_sup(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(relative_start) = html[cursor..].find(r"$\textcircled{D}^{") {
        let start = cursor + relative_start;
        output.push_str(&html[cursor..start]);
        let marker_start = start + r"$\textcircled{D}^{".len();
        let Some(relative_end) = html[marker_start..].find("}$") else {
            output.push_str(&html[start..]);
            return output;
        };
        let marker_end = marker_start + relative_end;
        let marker = html[marker_start..marker_end].trim();
        if is_author_marker(marker) {
            output.push_str(&format!("<sup>{}</sup>", escape_html(marker)));
        } else {
            output.push_str(&html[start..marker_end + "}$".len()]);
        }
        cursor = marker_end + "}$".len();
    }
    output.push_str(&html[cursor..]);
    output
}

#[cfg(test)]
fn is_author_marker(marker: &str) -> bool {
    !marker.is_empty()
        && marker
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == ',' || ch.is_ascii_whitespace())
}

fn build_author_popup_html(author: &AcademicAuthor) -> String {
    let mut parts = Vec::new();
    let mut seen_plain_rows = Vec::new();
    if author.is_corresponding {
        push_unique_popup_row(
            &mut parts,
            &mut seen_plain_rows,
            "通讯作者".to_string(),
            "<span class=\"academic-author-popover-row academic-author-role\">通讯作者</span>"
                .to_string(),
        );
    }
    let normalized_note = author.correspondence_note.as_deref().map(|note| {
        note.strip_prefix("$ ^{*} $")
            .unwrap_or(note)
            .trim()
            .to_string()
    });
    if !author.affiliations.is_empty() {
        for item in &author.affiliations {
            push_unique_popup_row(
                &mut parts,
                &mut seen_plain_rows,
                normalize_popup_row_text(item),
                format!(
                    "<span class=\"academic-author-popover-row\">{}</span>",
                    escape_html(item)
                ),
            );
        }
    }
    if let Some(email) = author.email.as_deref() {
        push_unique_popup_row(
            &mut parts,
            &mut seen_plain_rows,
            normalize_popup_row_text(email),
            format!(
                "<span class=\"academic-author-popover-row academic-author-email\">{}</span>",
                escape_html(email)
            ),
        );
    }
    if let Some(note) = normalized_note.as_deref() {
        let normalized_note_text = normalize_popup_row_text(note);
        let stripped_note_text = normalize_popup_row_text(
            note.strip_prefix("通讯作者：")
                .or_else(|| note.strip_prefix("通讯作者:"))
                .unwrap_or(note)
                .trim(),
        );
        if !seen_plain_rows.iter().any(|existing| {
            existing == &normalized_note_text
                || (!stripped_note_text.is_empty() && existing.contains(&stripped_note_text))
        }) {
            push_unique_popup_row(
                &mut parts,
                &mut seen_plain_rows,
                normalized_note_text,
                format!(
                    "<span class=\"academic-author-popover-row\">{}</span>",
                    escape_html(note)
                ),
            );
        }
    }
    parts.join("")
}

fn push_unique_popup_row(
    parts: &mut Vec<String>,
    seen_plain_rows: &mut Vec<String>,
    plain_text: String,
    html: String,
) {
    if plain_text.is_empty()
        || seen_plain_rows
            .iter()
            .any(|existing| existing == &plain_text)
    {
        return;
    }
    seen_plain_rows.push(plain_text);
    parts.push(html);
}

fn normalize_popup_row_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<String>()
        .replace('：', ":")
        .trim()
        .to_string()
}

fn author_anchor_id(author: &AcademicAuthor) -> String {
    format!(
        "author-meta-{}",
        super::canonical_author_key(&author.name).replace(' ', "-")
    )
}
