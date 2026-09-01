use super::LlmResponse;
use crate::error::{ProviderError, ProviderErrorKind};
use reqwest::StatusCode;
use std::time::Duration;

pub(super) fn parse_llm_response_bytes(body: &[u8]) -> Result<LlmResponse, ProviderError> {
    serde_json::from_slice::<LlmResponse>(body).map_err(|error| {
        ProviderError::new(
            "llm",
            ProviderErrorKind::Decode,
            format!(
                "parse LLM response failed: {error}; bytes={}; preview={}",
                body.len(),
                response_preview(body)
            ),
        )
    })
}

fn response_preview(body: &[u8]) -> String {
    const LIMIT: usize = 240;
    let take = body.len().min(LIMIT);
    String::from_utf8_lossy(&body[..take])
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

pub fn map_reqwest_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        return ProviderError::new(
            "llm",
            ProviderErrorKind::Timeout,
            format!("LLM request timed out: {error}"),
        );
    }
    if error.is_body() || error.is_decode() {
        return ProviderError::new(
            "llm",
            ProviderErrorKind::Decode,
            format!("LLM response body decode failed: {error}"),
        );
    }
    if error.is_connect() || error.is_request() {
        return ProviderError::new(
            "llm",
            ProviderErrorKind::Network,
            format!("LLM network request failed: {error}"),
        );
    }
    ProviderError::new(
        "llm",
        ProviderErrorKind::Unknown,
        format!("LLM request failed: {error}"),
    )
}

pub fn map_http_status(
    provider: &'static str,
    status: StatusCode,
    retry_after: Option<Duration>,
    body: String,
) -> ProviderError {
    let detail = if body.trim().is_empty() {
        status.to_string()
    } else {
        format!("{status}: {}", body.trim())
    };
    let kind = if is_rate_limit_response(status, &body) {
        ProviderErrorKind::RateLimited { retry_after }
    } else if is_quota_exhausted_response(status, &body) {
        ProviderErrorKind::QuotaExhausted
    } else {
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorKind::Authentication,
            StatusCode::TOO_MANY_REQUESTS => ProviderErrorKind::RateLimited { retry_after },
            StatusCode::BAD_REQUEST
            | StatusCode::UNPROCESSABLE_ENTITY
            | StatusCode::PAYLOAD_TOO_LARGE => ProviderErrorKind::BadRequest,
            status if status.is_server_error() => ProviderErrorKind::Server,
            _ => ProviderErrorKind::Unknown,
        }
    };
    ProviderError::new(
        provider,
        kind,
        format!("{provider} API returned error: {detail}"),
    )
}

fn is_rate_limit_response(_status: StatusCode, body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    [
        "rpm",
        "qps",
        "tpm",
        "rate limit",
        "rate_limit",
        "too many request",
        "requests per minute",
        "request per minute",
        "tokens per minute",
        "request rate",
        "frequency limit",
        "concurrency limit",
    ]
    .iter()
    .any(|marker| body.contains(marker))
}

fn is_quota_exhausted_response(status: StatusCode, body: &str) -> bool {
    if !matches!(
        status,
        StatusCode::BAD_REQUEST
            | StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::PAYMENT_REQUIRED
    ) {
        return false;
    }
    let body = body.to_ascii_lowercase();
    [
        "insufficient_quota",
        "quota_exceeded",
        "quota exceeded",
        "insufficient quota",
        "insufficient balance",
        "balance not enough",
        "account_balance_not_enough",
        "not enough balance",
        "credit exhausted",
        "hard quota",
        "余额",
        "额度",
        "欠费",
    ]
    .iter()
    .any(|marker| body.contains(marker))
}

pub fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let raw = value?.to_str().ok()?.trim();
    let seconds = raw.parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds.min(300)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_exhaustion_body_maps_to_quota_error() {
        let error = map_http_status(
            "llm",
            StatusCode::FORBIDDEN,
            None,
            r#"{"error":{"message":"insufficient quota, please recharge"}}"#.to_string(),
        );

        assert!(matches!(error.kind, ProviderErrorKind::QuotaExhausted));
    }

    #[test]
    fn regular_rate_limit_stays_rate_limited() {
        let error = map_http_status(
            "llm",
            StatusCode::TOO_MANY_REQUESTS,
            None,
            r#"{"error":{"message":"rate limit exceeded"}}"#.to_string(),
        );

        assert!(matches!(error.kind, ProviderErrorKind::RateLimited { .. }));
    }

    #[test]
    fn rpm_exhausted_maps_to_rate_limit_not_hard_quota() {
        let error = map_http_status(
            "llm",
            StatusCode::TOO_MANY_REQUESTS,
            None,
            r#"{"error":{"message":"rpm exhausted, please retry later"}}"#.to_string(),
        );

        assert!(matches!(error.kind, ProviderErrorKind::RateLimited { .. }));
    }

    #[test]
    fn hard_quota_429_still_maps_to_quota_exhausted() {
        let error = map_http_status(
            "llm",
            StatusCode::TOO_MANY_REQUESTS,
            None,
            r#"{"error":{"message":"insufficient quota, please recharge"}}"#.to_string(),
        );

        assert!(matches!(error.kind, ProviderErrorKind::QuotaExhausted));
    }

    #[test]
    fn non_429_rate_limit_text_maps_to_rate_limit() {
        let error = map_http_status(
            "llm",
            StatusCode::BAD_REQUEST,
            None,
            r#"{"message":"requests per minute exceeded"}"#.to_string(),
        );

        assert!(matches!(error.kind, ProviderErrorKind::RateLimited { .. }));
    }
}
