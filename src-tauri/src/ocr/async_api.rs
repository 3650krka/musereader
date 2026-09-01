use super::{
    map_http_status, parse_retry_after, preview_text, OcrJobStatusResponse, OcrJobSubmitResponse,
    OcrPhase, OcrRequest, OCR_ASYNC_JOB_URL, OCR_ASYNC_MODEL, OCR_ASYNC_POLL_INTERVAL_SECONDS,
    OCR_ASYNC_POLL_LIMIT, RETRY_AFTER,
};
use crate::error::{ProviderError, ProviderErrorKind};
use serde_json::json;
use std::path::Path;
use tokio::time::{sleep, Duration};

pub(super) async fn build_sync_request(
    pdf_path: &Path,
    phase: OcrPhase,
) -> Result<OcrRequest, ProviderError> {
    let pdf_bytes = tokio::fs::read(pdf_path).await.map_err(|error| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::BadRequest,
            format!("failed to read PDF: {error}"),
        )
    })?;
    let file_base64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pdf_bytes);

    Ok(OcrRequest {
        file: file_base64,
        file_type: 0,
        use_doc_orientation_classify: false,
        use_doc_unwarping: false,
        use_chart_recognition: false,
        use_layout_detection: phase_uses_layout_detection(phase),
        restructure_pages: phase_enables_restructure_pages(phase),
        merge_tables: phase_enables_merge_tables(phase),
        relevel_titles: phase_enables_relevel_titles(phase),
        prettify_markdown: false,
        show_formula_number: false,
        visualize: true,
        layout_threshold: Some(0.45),
        layout_nms: Some(true),
        layout_unclip_ratio: Some(1.0),
        layout_merge_bboxes_mode: Some("large".to_string()),
        layout_shape_mode: Some("rect".to_string()),
        prompt_label: phase_prompt_label(phase).map(ToString::to_string),
        repetition_penalty: Some(1.05),
        temperature: Some(0.1),
        top_p: Some(0.9),
        min_pixels: None,
        max_pixels: None,
    })
}

fn async_optional_payload(phase: OcrPhase) -> serde_json::Value {
    json!({
        "useDocOrientationClassify": false,
        "useDocUnwarping": false,
        "useChartRecognition": false,
        "useLayoutDetection": phase_uses_layout_detection(phase),
        "restructurePages": phase_enables_restructure_pages(phase),
        "mergeTables": phase_enables_merge_tables(phase),
        "relevelTitles": phase_enables_relevel_titles(phase),
        "prettifyMarkdown": false,
        "showFormulaNumber": false,
        "visualize": true,
        "layoutThreshold": 0.45,
        "layoutNms": true,
        "layoutUnclipRatio": 1.0,
        "layoutMergeBboxesMode": "large",
        "layoutShapeMode": "rect",
        "repetitionPenalty": 1.05,
        "temperature": 0.1,
        "topP": 0.9,
        "promptLabel": phase_prompt_label(phase)
    })
}

pub(super) async fn submit_async_job(
    client: &reqwest::Client,
    token: &str,
    pdf_path: &Path,
    phase: OcrPhase,
) -> Result<String, ProviderError> {
    let (content_type, body) = build_async_multipart_body(pdf_path, phase).await?;
    let response = client
        .post(OCR_ASYNC_JOB_URL)
        .header("Authorization", format!("bearer {}", token))
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await
        .map_err(super::map_reqwest_error)?;
    let body = read_success_body(response).await?;
    let response: OcrJobSubmitResponse = serde_json::from_str(&body).map_err(|error| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::Decode,
            format!(
                "failed to parse async OCR submit response: {error}; body={}",
                preview_text(&body)
            ),
        )
    })?;
    Ok(response.data.job_id)
}

pub(super) async fn poll_async_job(
    client: &reqwest::Client,
    token: &str,
    job_id: &str,
) -> Result<String, ProviderError> {
    for _ in 0..OCR_ASYNC_POLL_LIMIT {
        let status = fetch_status(client, token, job_id).await?;
        if let Some(json_url) = resolve_async_job_status(status)? {
            return Ok(json_url);
        }
        sleep(Duration::from_secs(OCR_ASYNC_POLL_INTERVAL_SECONDS)).await;
    }

    Err(ProviderError::new(
        "ocr",
        ProviderErrorKind::Timeout,
        "async OCR polling timed out",
    ))
}

pub(super) async fn fetch_status(
    client: &reqwest::Client,
    token: &str,
    job_id: &str,
) -> Result<OcrJobStatusResponse, ProviderError> {
    let response = client
        .get(format!("{OCR_ASYNC_JOB_URL}/{job_id}"))
        .header("Authorization", format!("bearer {}", token))
        .send()
        .await
        .map_err(super::map_reqwest_error)?;
    let body = read_success_body(response).await?;
    serde_json::from_str(&body).map_err(|error| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::Decode,
            format!(
                "failed to parse async OCR status: {error}; body={}",
                preview_text(&body)
            ),
        )
    })
}

pub(super) async fn fetch_jsonl_result(
    client: &reqwest::Client,
    jsonl_url: &str,
) -> Result<String, ProviderError> {
    let response = client
        .get(jsonl_url)
        .send()
        .await
        .map_err(super::map_reqwest_error)?;
    read_success_body(response).await
}

async fn build_async_multipart_body_with_phase(
    pdf_path: &Path,
    phase: OcrPhase,
) -> Result<(String, Vec<u8>), ProviderError> {
    let file_bytes = tokio::fs::read(pdf_path).await.map_err(|error| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::BadRequest,
            format!("failed to read PDF for async OCR: {error}"),
        )
    })?;
    let boundary = format!(
        "musetranslate-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let file_name = pdf_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("document.pdf");
    let mut body = Vec::new();
    append_text_part(&mut body, &boundary, "model", OCR_ASYNC_MODEL);
    append_text_part(
        &mut body,
        &boundary,
        "optionalPayload",
        &async_optional_payload(phase).to_string(),
    );
    append_file_part(&mut body, &boundary, "file", file_name, &file_bytes);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((format!("multipart/form-data; boundary={boundary}"), body))
}

async fn build_async_multipart_body(
    pdf_path: &Path,
    phase: OcrPhase,
) -> Result<(String, Vec<u8>), ProviderError> {
    build_async_multipart_body_with_phase(pdf_path, phase).await
}

fn append_text_part(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn append_file_part(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    file_name: &str,
    file_bytes: &[u8],
) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/pdf\r\n\r\n");
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");
}

pub(super) async fn read_success_body(
    response: reqwest::Response,
) -> Result<String, ProviderError> {
    if !response.status().is_success() {
        let status = response.status();
        let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
        let body = response.text().await.unwrap_or_default();
        return Err(map_http_status("ocr", status, retry_after, body));
    }
    response.text().await.map_err(|error| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::Decode,
            format!("failed to read OCR response body: {error}"),
        )
    })
}

fn resolve_async_job_status(status: OcrJobStatusResponse) -> Result<Option<String>, ProviderError> {
    match status.data.state.as_str() {
        "done" => resolve_completed_async_job(status.data.result_url),
        "failed" => Err(ProviderError::new(
            "ocr",
            ProviderErrorKind::Unknown,
            status
                .data
                .error_msg
                .unwrap_or_else(|| "async OCR job failed".to_string()),
        )),
        "pending" | "running" => Ok(None),
        other => Err(ProviderError::new(
            "ocr",
            ProviderErrorKind::Decode,
            format!("unknown async OCR state: {other}"),
        )),
    }
}

fn resolve_completed_async_job(
    result_url: Option<super::OcrJobResultUrl>,
) -> Result<Option<String>, ProviderError> {
    let Some(result_url) = result_url else {
        return Err(ProviderError::new(
            "ocr",
            ProviderErrorKind::EmptyResponse,
            "async OCR completed without result URL",
        ));
    };
    Ok(Some(result_url.json_url))
}

fn phase_uses_layout_detection(phase: OcrPhase) -> bool {
    matches!(phase, OcrPhase::Layout)
}

fn phase_enables_restructure_pages(phase: OcrPhase) -> bool {
    matches!(phase, OcrPhase::Structure)
}

fn phase_enables_merge_tables(phase: OcrPhase) -> bool {
    matches!(phase, OcrPhase::Structure)
}

fn phase_enables_relevel_titles(phase: OcrPhase) -> bool {
    matches!(phase, OcrPhase::Structure)
}

fn phase_prompt_label(phase: OcrPhase) -> Option<&'static str> {
    match phase {
        OcrPhase::Layout => None,
        OcrPhase::Structure => Some("ocr"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_phase_disables_layout_detection() {
        assert!(!phase_uses_layout_detection(OcrPhase::Structure));
        assert!(phase_enables_restructure_pages(OcrPhase::Structure));
        assert!(phase_enables_merge_tables(OcrPhase::Structure));
        assert!(phase_enables_relevel_titles(OcrPhase::Structure));
        assert_eq!(phase_prompt_label(OcrPhase::Structure), Some("ocr"));
    }

    #[test]
    fn layout_phase_keeps_layout_detection() {
        assert!(phase_uses_layout_detection(OcrPhase::Layout));
        assert!(!phase_enables_restructure_pages(OcrPhase::Layout));
        assert!(!phase_enables_merge_tables(OcrPhase::Layout));
        assert!(!phase_enables_relevel_titles(OcrPhase::Layout));
        assert_eq!(phase_prompt_label(OcrPhase::Layout), None);
    }
}
