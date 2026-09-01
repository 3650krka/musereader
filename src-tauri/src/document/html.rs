use crate::error::AppError;
use regex::Regex;

pub(crate) fn extract_text_from_html(raw: &str) -> Result<String, AppError> {
    let script_re = Regex::new("(?is)<script.*?</script>")
        .map_err(|error| AppError::internal(format!("缂栬瘧 HTML 姝ｅ垯澶辫触: {error}")))?;
    let style_re = Regex::new("(?is)<style.*?</style>")
        .map_err(|error| AppError::internal(format!("缂栬瘧 HTML 姝ｅ垯澶辫触: {error}")))?;
    let br_re = Regex::new("(?i)<br\\s*/?>")
        .map_err(|error| AppError::internal(format!("缂栬瘧 HTML 姝ｅ垯澶辫触: {error}")))?;
    let block_re =
        Regex::new("(?i)</?(p|div|section|article|h1|h2|h3|h4|h5|h6|li|blockquote|tr|td|th)>")
            .map_err(|error| AppError::internal(format!("缂栬瘧 HTML 姝ｅ垯澶辫触: {error}")))?;
    let tag_re = Regex::new("(?is)<[^>]+>")
        .map_err(|error| AppError::internal(format!("缂栬瘧 HTML 姝ｅ垯澶辫触: {error}")))?;

    let cleaned = script_re.replace_all(raw, " ");
    let cleaned = style_re.replace_all(&cleaned, " ");
    let cleaned = br_re.replace_all(&cleaned, "\n");
    let cleaned = block_re.replace_all(&cleaned, "\n");
    let cleaned = tag_re.replace_all(&cleaned, " ");
    let decoded = html_entity_decode(&cleaned);

    let lines = decoded
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    Ok(lines.join("\n\n"))
}

pub(super) fn extract_html_title(raw: &str) -> Option<String> {
    let heading_re = Regex::new("(?is)<h[1-3][^>]*>(.*?)</h[1-3]>").ok()?;
    let captures = heading_re.captures(raw)?;
    let raw_title = captures.get(1)?.as_str();
    normalize_title_text(raw_title)
}

/// <head><title> 提取：无 h1-h3 的章节（出版书插页/新闻剪报等）常用
/// 有意义的 title（如 "1. Tash"），优于 spine 序号兜底 "Chapter N"。
pub(super) fn extract_html_head_title(raw: &str) -> Option<String> {
    let title_re = Regex::new("(?is)<title[^>]*>(.*?)</title>").ok()?;
    let captures = title_re.captures(raw)?;
    normalize_title_text(captures.get(1)?.as_str())
}

/// 标签剥除 + 实体解码 + 空白归一；空串返回 None。
fn normalize_title_text(raw_title: &str) -> Option<String> {
    let text = Regex::new("(?is)<[^>]+>")
        .ok()?
        .replace_all(raw_title, " ")
        .to_string();
    let normalized = html_entity_decode(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
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
