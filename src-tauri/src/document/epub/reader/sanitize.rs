use crate::error::AppError;
use regex::Regex;
use std::sync::OnceLock;

const READER_CSP: &str = "default-src 'none'; img-src 'self' asset: http://asset.localhost https://asset.localhost data: blob:; style-src 'self' asset: http://asset.localhost https://asset.localhost 'unsafe-inline' data:; font-src 'self' asset: http://asset.localhost https://asset.localhost data:; media-src 'self' asset: http://asset.localhost https://asset.localhost data: blob:; connect-src 'none'; script-src 'none'; frame-src 'none'; object-src 'none'; base-uri asset: http://asset.localhost https://asset.localhost; form-action 'none'";

pub(super) fn sanitize_reader_html(input: &str) -> Result<String, AppError> {
    let without_active_content = strip_active_content(input)?;
    let normalized_encoding = normalize_declared_encoding(&without_active_content)?;
    inject_reader_security_meta(&normalized_encoding)
}

fn strip_active_content(input: &str) -> Result<String, AppError> {
    let mut output = active_block_regex()?.replace_all(input, "").into_owned();
    output = active_tag_regex()?.replace_all(&output, "").into_owned();
    output = base_tag_regex()?.replace_all(&output, "").into_owned();
    output = refresh_meta_regex()?.replace_all(&output, "").into_owned();
    output = event_handler_regex()?.replace_all(&output, "").into_owned();
    Ok(dangerous_url_regex()?
        .replace_all(&output, "${attribute}=\"#\"")
        .into_owned())
}

fn normalize_declared_encoding(input: &str) -> Result<String, AppError> {
    Ok(xml_encoding_regex()?
        .replace(input, "${prefix}utf-8${suffix}")
        .into_owned())
}

fn inject_reader_security_meta(input: &str) -> Result<String, AppError> {
    let meta = format!(
        "<meta charset=\"utf-8\" /><meta http-equiv=\"Content-Security-Policy\" content=\"{READER_CSP}\" />"
    );
    if let Some(head) = head_tag_regex()?.find(input) {
        let mut output = String::with_capacity(input.len() + meta.len());
        output.push_str(&input[..head.end()]);
        output.push_str(&meta);
        output.push_str(&input[head.end()..]);
        return Ok(output);
    }
    Ok(format!("{meta}{input}"))
}

fn active_block_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(
        &REGEX,
        r"(?is)<(?:script|iframe|object|embed)\b[^>]*>.*?</(?:script|iframe|object|embed)\s*>",
        "active block",
    )
}

fn active_tag_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(
        &REGEX,
        r"(?is)<(?:script|iframe|object|embed)\b[^>]*/\s*>",
        "active tag",
    )
}

fn base_tag_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(&REGEX, r"(?is)<base\b[^>]*>", "base tag")
}

fn refresh_meta_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(
        &REGEX,
        r#"(?is)<meta\b[^>]*http-equiv\s*=\s*[\"']?refresh[\"']?[^>]*>"#,
        "refresh meta",
    )
}

fn event_handler_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(
        &REGEX,
        r#"(?is)\s+on[a-z][a-z0-9:_-]*\s*=\s*(?:\"[^\"]*\"|'[^']*'|[^\s>]+)"#,
        "event handler",
    )
}

fn dangerous_url_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(
        &REGEX,
        r#"(?is)(?P<attribute>(?:xlink:)?href|src|action|formaction)\s*=\s*[\"']\s*(?:javascript|vbscript):[^\"']*[\"']"#,
        "dangerous URL",
    )
}

fn xml_encoding_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(
        &REGEX,
        r#"(?is)(?P<prefix><\?xml\b[^>]*encoding\s*=\s*[\"'])(?:[^\"']+)(?P<suffix>[\"'])"#,
        "XML encoding",
    )
}

fn head_tag_regex() -> Result<&'static Regex, AppError> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    compiled_regex(&REGEX, r"(?is)<head\b[^>]*>", "head tag")
}

fn compiled_regex(
    cell: &'static OnceLock<Result<Regex, regex::Error>>,
    pattern: &'static str,
    label: &str,
) -> Result<&'static Regex, AppError> {
    match cell.get_or_init(|| Regex::new(pattern)) {
        Ok(regex) => Ok(regex),
        Err(error) => Err(AppError::internal(format!(
            "初始化 EPUB {label} 清洗规则失败: {error}"
        ))),
    }
}
