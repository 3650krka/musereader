//! 供应商连通性测试：一次最小 chat completion 请求，验证 URL/Key/模型可用。

use super::{normalize_llm_api_url, AppError};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Serialize)]
struct TestPayload<'a> {
    model: &'a str,
    messages: [TestMessage<'a>; 1],
    max_tokens: u8,
}

#[derive(Debug, Serialize)]
struct TestMessage<'a> {
    role: &'a str,
    content: &'a str,
}

const TEST_TIMEOUT_SECS: u64 = 12;

#[tauri::command]
pub async fn test_provider_connection(
    api_url: String,
    api_key: String,
    model: String,
) -> Result<String, AppError> {
    let trimmed_url = api_url.trim();
    let trimmed_key = api_key.trim();
    let trimmed_model = model.trim();
    if trimmed_url.is_empty() {
        return Err(AppError::invalid_input("请先填写 API URL"));
    }
    if trimmed_model.is_empty() {
        return Err(AppError::invalid_input("请先填写模型"));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TEST_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
        .build()
        .map_err(|error| AppError::internal(format!("创建测试客户端失败：{error}")))?;

    let response = client
        .post(normalize_llm_api_url(trimmed_url.to_string()))
        .header("Authorization", format!("Bearer {trimmed_key}"))
        .header("Content-Type", "application/json")
        .json(&TestPayload {
            model: trimmed_model,
            messages: [TestMessage {
                role: "user",
                content: "ping",
            }],
            max_tokens: 1,
        })
        .send()
        .await
        .map_err(|error| {
            AppError::new(
                "PROVIDER_UNREACHABLE",
                format!("无法连接供应商：{error}"),
                true,
            )
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        return Ok("连接成功".to_string());
    }
    // 401/403 = key 无效；其余错误码带响应体截断回显，方便定位
    let hint = match status.as_u16() {
        401 | 403 => "（认证失败：请检查 API Key）",
        404 => "（模型或端点不存在：请检查 URL 与模型名）",
        429 => "（限流：稍后重试）",
        _ => "",
    };
    Err(AppError::new(
        "PROVIDER_TEST_FAILED",
        format!(
            "连接失败 HTTP {}{}：{}",
            status.as_u16(),
            hint,
            truncate_body(&body, 240)
        ),
        false,
    ))
}

fn truncate_body(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max_chars).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_body_short_returns_whole() {
        assert_eq!(truncate_body("ok", 240), "ok");
        assert_eq!(truncate_body("  ok  ", 240), "ok");
    }

    #[test]
    fn truncate_body_long_cuts_with_ellipsis() {
        let long = "x".repeat(500);
        let out = truncate_body(&long, 240);
        assert_eq!(out.chars().count(), 241);
        assert!(out.ends_with('…'));
    }
}
