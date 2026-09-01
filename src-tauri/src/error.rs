use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum ProviderErrorKind {
    Authentication,
    QuotaExhausted,
    RateLimited { retry_after: Option<Duration> },
    Timeout,
    Network,
    Server,
    BadRequest,
    EmptyResponse,
    Decode,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub provider: &'static str,
    pub kind: ProviderErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn new(
        provider: &'static str,
        kind: ProviderErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            kind,
            message: message.into(),
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ProviderErrorKind::RateLimited { .. }
                | ProviderErrorKind::QuotaExhausted
                | ProviderErrorKind::Timeout
                | ProviderErrorKind::Network
                | ProviderErrorKind::Server
                | ProviderErrorKind::EmptyResponse
                | ProviderErrorKind::Decode
        )
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self.kind {
            ProviderErrorKind::RateLimited { retry_after } => retry_after,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            retry_after_ms: None,
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("INVALID_INPUT", message, false)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", message, false)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("CONFLICT", message, false)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL", message, true)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new("CANCELLED", message, true)
    }

    pub fn from_provider(error: ProviderError) -> Self {
        let retryable = error.is_retryable();
        let code = provider_error_code(error.provider, &error.kind);
        let retry_after_ms = error
            .retry_after()
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64);
        Self {
            code,
            message: error.message,
            retryable,
            retry_after_ms,
        }
    }

    pub fn from_provider_error(error: ProviderError) -> Self {
        Self::from_provider(error)
    }
}

fn provider_error_code(provider: &str, kind: &ProviderErrorKind) -> &'static str {
    match provider {
        "ocr" => ocr_error_code(kind),
        "llm" => llm_error_code(kind),
        _ => generic_provider_error_code(kind),
    }
}

fn ocr_error_code(kind: &ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::Authentication => "OCR_AUTH_ERROR",
        ProviderErrorKind::QuotaExhausted => "OCR_QUOTA_EXHAUSTED",
        ProviderErrorKind::RateLimited { .. } => "OCR_RATE_LIMITED",
        ProviderErrorKind::Timeout => "OCR_TIMEOUT",
        ProviderErrorKind::Network => "OCR_NETWORK_ERROR",
        ProviderErrorKind::Server => "OCR_SERVER_ERROR",
        ProviderErrorKind::BadRequest => "OCR_BAD_REQUEST",
        ProviderErrorKind::EmptyResponse => "OCR_EMPTY_RESPONSE",
        ProviderErrorKind::Decode => "OCR_DECODE_ERROR",
        ProviderErrorKind::Unknown => "OCR_PROVIDER_ERROR",
    }
}

fn llm_error_code(kind: &ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::Authentication => "LLM_AUTH_ERROR",
        ProviderErrorKind::QuotaExhausted => "LLM_QUOTA_EXHAUSTED",
        ProviderErrorKind::RateLimited { .. } => "LLM_RATE_LIMITED",
        ProviderErrorKind::Timeout => "LLM_TIMEOUT",
        ProviderErrorKind::Network => "LLM_NETWORK_ERROR",
        ProviderErrorKind::Server => "LLM_SERVER_ERROR",
        ProviderErrorKind::BadRequest => "LLM_BAD_REQUEST",
        ProviderErrorKind::EmptyResponse => "LLM_EMPTY_RESPONSE",
        ProviderErrorKind::Decode => "LLM_DECODE_ERROR",
        ProviderErrorKind::Unknown => "LLM_PROVIDER_ERROR",
    }
}

fn generic_provider_error_code(kind: &ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::Authentication => "PROVIDER_AUTH_ERROR",
        ProviderErrorKind::QuotaExhausted => "PROVIDER_QUOTA_EXHAUSTED",
        ProviderErrorKind::RateLimited { .. } => "PROVIDER_RATE_LIMITED",
        ProviderErrorKind::Timeout => "PROVIDER_TIMEOUT",
        ProviderErrorKind::Network => "PROVIDER_NETWORK_ERROR",
        ProviderErrorKind::Server => "PROVIDER_SERVER_ERROR",
        ProviderErrorKind::BadRequest => "PROVIDER_BAD_REQUEST",
        ProviderErrorKind::EmptyResponse => "PROVIDER_EMPTY_RESPONSE",
        ProviderErrorKind::Decode => "PROVIDER_DECODE_ERROR",
        ProviderErrorKind::Unknown => "PROVIDER_ERROR",
    }
}
