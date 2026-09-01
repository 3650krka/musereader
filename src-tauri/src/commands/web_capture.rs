//! 网页抓取导入：URL → 正文提取 → MD 文件入导入目录。
//!
//! 与 ）；
//! 我们把其中「抓取→清洗→入阅读器」的最短因果链做进桌面程序命令层，
//! 复用 EPUB 管线的 boilerplate 剥离算法（结构性标记驱动），不引入 readability 依赖——
//! 正文定位用「最长文本块启发式」（main/article/最大文本密度容器），
//! 覆盖常见文章页；复杂 SPA 页面超出静态抓取能力（浏览器扩展方案列入未来计划）。
//!
//! 防御边界：
//! - 仅 http/https；响应大小上限 8MB；超时 30s。
//! - 正文提取失败返回 invalid_input（用户可改用手动复制）。

use super::{resolve_runtime_paths, AppError};
use serde::Serialize;
use uuid::Uuid;

const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const FETCH_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebCaptureResult {
    pub title: String,
    pub markdown_chars: usize,
    pub output_path: String,
}

/// 抓取网页正文并保存为 MD（导入目录），返回标题与路径。
/// 前端随后可经 start_translation（md 输入）或直接放入阅读器书库。
#[tauri::command]
pub async fn capture_web_article(url: String) -> Result<WebCaptureResult, AppError> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AppError::invalid_input("仅支持 http/https 链接"));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )
        .build()
        .map_err(|error| AppError::internal(format!("构建抓取客户端失败: {error}")))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::internal(format!("网页抓取失败: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::internal(format!(
            "网页返回错误状态: {}",
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::internal(format!("读取网页内容失败: {error}")))?;
    if bytes.len() > MAX_HTML_BYTES {
        return Err(AppError::invalid_input(
            "网页过大（>8MB），请改用浏览器扩展方案",
        ));
    }
    let html = String::from_utf8_lossy(&bytes).into_owned();

    let title = extract_html_title_text(&html).unwrap_or_else(|| "网页剪藏".to_string());
    let article_html = isolate_article_html(&html);
    let cleaned = crate::document::strip_web_boilerplate_for_capture(&article_html);
    let markdown = crate::document::html_to_plain_markdown(&cleaned)
        .map_err(|error| AppError::internal(format!("正文转换失败: {error:?}")))?;
    let markdown = markdown.trim().to_string();
    if markdown.chars().count() < 120 {
        return Err(AppError::invalid_input(
            "未能提取到正文（内容过短）；该页可能是动态渲染页面，需浏览器扩展方案",
        ));
    }

    let document = format!("# {title}\n\n> 来源：{url}\n\n{markdown}\n");
    let import_dir = resolve_runtime_paths().import_root_dir;
    tokio::fs::create_dir_all(&import_dir)
        .await
        .map_err(|error| AppError::internal(format!("创建导入目录失败: {error}")))?;
    let file_name = format!("{}_{}.md", Uuid::new_v4(), sanitize_file_component(&title));
    let output_path = import_dir.join(file_name);
    tokio::fs::write(&output_path, &document)
        .await
        .map_err(|error| AppError::internal(format!("保存剪藏文件失败: {error}")))?;

    Ok(WebCaptureResult {
        title,
        markdown_chars: markdown.chars().count(),
        output_path: output_path.to_string_lossy().to_string(),
    })
}

/// <title> 提取（head 内，大小写不敏感）。
fn extract_html_title_text(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title>")? + open_end;
    let title = html[open_end..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(decode_basic_entities(title))
    }
}

/// 正文定位启发式：优先 <article>/<main> 容器；否则取文本密度最高的 <div>/<section>。
/// 全部失败时回退 <body>（boilerplate 剥离仍会清掉 nav/footer/script 等残渣）。
fn isolate_article_html(html: &str) -> String {
    if let Some(block) = extract_first_element_block(html, "article") {
        return block;
    }
    if let Some(block) = extract_first_element_block(html, "main") {
        return block;
    }
    densest_text_block(html)
        .unwrap_or_else(|| extract_body(html).unwrap_or_else(|| html.to_string()))
}

fn extract_body(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<body")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower.rfind("</body>").unwrap_or(html.len());
    Some(html[open_end..end.min(html.len())].to_string())
}

/// 提取首个 <tag ...>...</tag> 完整块（同名嵌套深度计数）。
fn extract_first_element_block(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open_marker = format!("<{tag}");
    let start = lower.find(&open_marker)?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close_marker = format!("</{tag}>");
    // 深度计数：同名嵌套标签需匹配到最外层闭标签
    let mut depth = 1usize;
    let mut cursor = open_end;
    while depth > 0 {
        let next_open = lower[cursor..].find(&open_marker).map(|i| i + cursor);
        let next_close = lower[cursor..].find(&close_marker).map(|i| i + cursor);
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                cursor = o + open_marker.len();
            }
            (_, Some(c)) => {
                depth -= 1;
                cursor = c + close_marker.len();
            }
            _ => return None,
        }
    }
    Some(html[start..cursor].to_string())
}

/// 文本密度最高的块级容器（div/section）：纯文本字符数近似密度。
/// 简单有效：文章正文的文字量几乎总是远超侧栏/导航/评论块。
fn densest_text_block(html: &str) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for tag in ["div", "section"] {
        let lower = html.to_ascii_lowercase();
        let open_marker = format!("<{tag}");
        let mut search_from = 0usize;
        while let Some(rel) = lower[search_from..].find(&open_marker) {
            let start = search_from + rel;
            if let Some(block) = extract_first_element_block(&html[start..], tag) {
                let text_len = strip_tags(&block).chars().count();
                if text_len > best.as_ref().map(|(n, _)| *n).unwrap_or(0) {
                    best = Some((text_len, block));
                }
                search_from = start + open_marker.len();
            } else {
                break;
            }
        }
    }
    // 阈值：块至少 400 字才认作正文候选（避免抓到导航大列表）
    best.filter(|(len, _)| *len >= 400).map(|(_, block)| block)
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn sanitize_file_component(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .take(60)
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "web-article".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_extraction_decodes_entities() {
        let html = "<html><head><title>Tom &amp; Jerry</title></head><body>x</body></html>";
        assert_eq!(
            extract_html_title_text(html).as_deref(),
            Some("Tom & Jerry")
        );
    }

    #[test]
    fn article_tag_wins_over_body() {
        let html = "<body><nav>menu items</nav><article><p>real content here</p></article></body>";
        let isolated = isolate_article_html(html);
        assert!(isolated.contains("real content here"));
        assert!(!isolated.contains("menu items"));
    }

    #[test]
    fn densest_block_selected_when_no_semantic_container() {
        let long_text = "lorem ipsum dolor sit amet ".repeat(40);
        let html = format!(
            "<body><div id=\"nav\">home about contact</div><div id=\"content\"><p>{long_text}</p></div></body>"
        );
        let isolated = isolate_article_html(&html);
        assert!(isolated.contains("lorem ipsum"));
        assert!(!isolated.contains("home about contact"));
    }

    #[test]
    fn nested_same_tag_depth_counted() {
        let html = "<body><article><div>a<article><p>inner</p></article></div></article></body>";
        let block = extract_first_element_block(html, "article").expect("outer article block");
        assert!(block.contains("inner")); // 外层块应包含嵌套的同名标签
    }

    #[test]
    fn file_component_sanitized_and_truncated() {
        assert_eq!(sanitize_file_component("a/b:c*?"), "a_b_c__");
        assert!(sanitize_file_component(&"长".repeat(100)).chars().count() <= 60);
        assert_eq!(sanitize_file_component("  "), "web-article");
    }
}
