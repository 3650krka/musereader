use crate::document::TocArtifact;
use crate::error::{ProviderError, ProviderErrorKind};
use crate::llm::{map_http_status, parse_retry_after};
use reqwest::header::RETRY_AFTER;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

mod async_api;
mod parse;

use async_api::{
    build_sync_request, fetch_jsonl_result, poll_async_job, read_success_body, submit_async_job,
};
use parse::{preview_text, should_fallback_to_async};

const OCR_TIMEOUT_SECONDS: u64 = 180;
const OCR_ASYNC_JOB_URL: &str = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs";
const OCR_ASYNC_MODEL: &str = "PaddleOCR-VL-1.6";
const OCR_ASYNC_POLL_INTERVAL_SECONDS: u64 = 5;
const OCR_ASYNC_POLL_LIMIT: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OcrPhase {
    Layout,
    Structure,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OcrRequest {
    pub file: String,
    #[serde(rename = "fileType")]
    pub file_type: u8,
    #[serde(rename = "useDocOrientationClassify")]
    pub use_doc_orientation_classify: bool,
    #[serde(rename = "useDocUnwarping")]
    pub use_doc_unwarping: bool,
    #[serde(rename = "useChartRecognition")]
    pub use_chart_recognition: bool,
    #[serde(rename = "useLayoutDetection")]
    pub use_layout_detection: bool,
    #[serde(rename = "restructurePages")]
    pub restructure_pages: bool,
    #[serde(rename = "mergeTables")]
    pub merge_tables: bool,
    #[serde(rename = "relevelTitles")]
    pub relevel_titles: bool,
    #[serde(rename = "prettifyMarkdown")]
    pub prettify_markdown: bool,
    #[serde(rename = "showFormulaNumber")]
    pub show_formula_number: bool,
    #[serde(rename = "visualize")]
    pub visualize: bool,
    #[serde(rename = "layoutThreshold", skip_serializing_if = "Option::is_none")]
    pub layout_threshold: Option<f32>,
    #[serde(rename = "layoutNms", skip_serializing_if = "Option::is_none")]
    pub layout_nms: Option<bool>,
    #[serde(rename = "layoutUnclipRatio", skip_serializing_if = "Option::is_none")]
    pub layout_unclip_ratio: Option<f32>,
    #[serde(
        rename = "layoutMergeBboxesMode",
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_merge_bboxes_mode: Option<String>,
    #[serde(rename = "layoutShapeMode", skip_serializing_if = "Option::is_none")]
    pub layout_shape_mode: Option<String>,
    #[serde(rename = "promptLabel", skip_serializing_if = "Option::is_none")]
    pub prompt_label: Option<String>,
    #[serde(rename = "repetitionPenalty", skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f32>,
    #[serde(rename = "temperature", skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(rename = "minPixels", skip_serializing_if = "Option::is_none")]
    pub min_pixels: Option<u32>,
    #[serde(rename = "maxPixels", skip_serializing_if = "Option::is_none")]
    pub max_pixels: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OcrResponse {
    #[serde(rename = "logId")]
    pub log_id: Option<String>,
    #[serde(rename = "errorCode")]
    pub error_code: Option<i32>,
    #[serde(rename = "errorMsg")]
    pub error_msg: Option<String>,
    pub result: Option<OcrResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OcrResult {
    #[serde(rename = "layoutParsingResults")]
    pub layout_parsing_results: Vec<LayoutParsingResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutParsingResult {
    pub markdown: MarkdownResult,
    #[serde(rename = "prunedResult")]
    pub pruned_result: serde_json::Value,
    #[serde(rename = "outputImages", default)]
    pub output_images: HashMap<String, String>,
    #[serde(rename = "inputImage")]
    pub input_image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrLayoutBlock {
    pub label: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub bbox: Vec<i32>,
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub page: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrPageLayout {
    pub blocks: Vec<OcrLayoutBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownResult {
    pub text: String,
    #[serde(default)]
    pub images: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct OcrDocument {
    pub markdown: String,
    pub cover_data_url: Option<String>,
    pub page_layouts: Vec<OcrPageLayout>,
    pub markdown_images: HashMap<String, String>,
    pub toc: Option<TocArtifact>,
    pub epub_blocks: Vec<crate::document::EpubBlock>,
    pub source_blocks: Vec<crate::document::SourceBlock>,
}

#[derive(Debug, Clone)]
pub(crate) struct OcrDocumentWithRaw {
    pub(crate) document: OcrDocument,
    pub(crate) layout_results: Vec<LayoutParsingResult>,
    pub(crate) structure_results: Vec<LayoutParsingResult>,
    pub(crate) layout_raw_json: serde_json::Value,
    pub(crate) structure_raw_json: serde_json::Value,
}

struct OcrPhaseResult {
    results: Vec<LayoutParsingResult>,
    raw_json: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrJobSubmitResponse {
    pub(crate) data: OcrJobSubmitData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrJobSubmitData {
    #[serde(rename = "jobId")]
    pub(crate) job_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrJobStatusResponse {
    pub(crate) data: OcrJobStatusData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrJobStatusData {
    pub(crate) state: String,
    #[serde(rename = "errorMsg")]
    pub(crate) error_msg: Option<String>,
    #[serde(rename = "resultUrl")]
    pub(crate) result_url: Option<OcrJobResultUrl>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrJobResultUrl {
    #[serde(rename = "jsonUrl")]
    pub(crate) json_url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OcrJsonlLine {
    pub(crate) result: OcrResult,
}

pub struct BaiduOcrClient {
    api_url: String,
    token: String,
    client: reqwest::Client,
}

impl BaiduOcrClient {
    pub fn new(api_url: String, token: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(OCR_TIMEOUT_SECONDS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            api_url,
            token,
            client,
        }
    }

    pub(crate) fn cache_fingerprint(&self) -> String {
        format!(
            "api={}; async_model={}; contract=layout-structure-v1",
            self.api_url.trim(),
            OCR_ASYNC_MODEL
        )
    }

    #[allow(dead_code)]
    pub async fn convert_pdf(&self, pdf_path: &Path) -> Result<OcrDocument, ProviderError> {
        Ok(self.convert_pdf_with_raw(pdf_path).await?.document)
    }

    pub(crate) async fn convert_pdf_with_raw(
        &self,
        pdf_path: &Path,
    ) -> Result<OcrDocumentWithRaw, ProviderError> {
        if self.token.trim().is_empty() {
            return Err(ProviderError::new(
                "ocr",
                ProviderErrorKind::Authentication,
                "OCR token is empty",
            ));
        }

        let layout_phase = self.convert_pdf_phase(pdf_path, OcrPhase::Layout).await?;
        let structure_phase = match self.convert_pdf_phase(pdf_path, OcrPhase::Structure).await {
            Ok(results) => results,
            Err(error) => {
                eprintln!(
                    "[ocr-structure-fallback] provider=ocr; message={}",
                    error.message
                );
                OcrPhaseResult {
                    results: layout_phase.results.clone(),
                    raw_json: serde_json::json!({
                        "fallback": "layout",
                        "error": error.message,
                        "layoutRawJson": layout_phase.raw_json.clone()
                    }),
                }
            }
        };

        let document = parse::ocr_layouts_to_document_with_layout_source(
            structure_phase.results.clone(),
            layout_phase.results.clone(),
        )?;
        Ok(OcrDocumentWithRaw {
            document,
            layout_results: layout_phase.results,
            structure_results: structure_phase.results,
            layout_raw_json: layout_phase.raw_json,
            structure_raw_json: structure_phase.raw_json,
        })
    }

    async fn convert_pdf_phase(
        &self,
        pdf_path: &Path,
        phase: OcrPhase,
    ) -> Result<OcrPhaseResult, ProviderError> {
        let request = build_sync_request(pdf_path, phase).await?;
        match self.convert_pdf_sync(&request).await {
            Ok(results) => Ok(results),
            Err(error) if should_fallback_to_async(&error) => {
                self.convert_pdf_async(pdf_path, phase).await
            }
            Err(error) => Err(error),
        }
    }

    async fn convert_pdf_sync(
        &self,
        request: &OcrRequest,
    ) -> Result<OcrPhaseResult, ProviderError> {
        let response = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("token {}", self.token))
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let body = read_success_body(response).await?;
        let response: OcrResponse = serde_json::from_str(&body).map_err(|error| {
            ProviderError::new(
                "ocr",
                ProviderErrorKind::Decode,
                format!(
                    "failed to parse sync OCR response: {error}; body={}",
                    preview_text(&body)
                ),
            )
        })?;
        let raw_json = serde_json::to_value(&response).map_err(|error| {
            ProviderError::new(
                "ocr",
                ProviderErrorKind::Decode,
                format!("failed to preserve sync OCR response JSON: {error}"),
            )
        })?;
        let results = parse::ocr_response_to_layout_results(response)?;
        Ok(OcrPhaseResult { results, raw_json })
    }

    async fn convert_pdf_async(
        &self,
        pdf_path: &Path,
        phase: OcrPhase,
    ) -> Result<OcrPhaseResult, ProviderError> {
        let job_id = submit_async_job(&self.client, &self.token, pdf_path, phase).await?;
        let jsonl_url = poll_async_job(&self.client, &self.token, &job_id).await?;
        let jsonl = fetch_jsonl_result(&self.client, &jsonl_url).await?;
        let results = parse::parse_jsonl_layout_results(&jsonl)?;
        let raw_json = parse_jsonl_raw_value(&jsonl)?;
        Ok(OcrPhaseResult { results, raw_json })
    }
}

impl OcrDocumentWithRaw {
    pub(crate) fn from_raw_json(
        layout_raw_json: serde_json::Value,
        structure_raw_json: serde_json::Value,
    ) -> Result<Self, ProviderError> {
        let layout_results = parse::raw_ocr_value_to_layout_results(&layout_raw_json)?;
        let structure_results = if parse::raw_ocr_value_is_layout_fallback(&structure_raw_json) {
            layout_results.clone()
        } else {
            parse::raw_ocr_value_to_layout_results(&structure_raw_json)?
        };
        let document = parse::ocr_layouts_to_document_with_layout_source(
            structure_results.clone(),
            layout_results.clone(),
        )?;
        Ok(Self {
            document,
            layout_results,
            structure_results,
            layout_raw_json,
            structure_raw_json,
        })
    }
}

fn parse_jsonl_raw_value(jsonl: &str) -> Result<serde_json::Value, ProviderError> {
    let mut lines = Vec::new();
    for line in jsonl.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<serde_json::Value>(trimmed).map_err(|error| {
            ProviderError::new(
                "ocr",
                ProviderErrorKind::Decode,
                format!(
                    "failed to preserve async OCR JSONL line: {error}; line={}",
                    preview_text(trimmed)
                ),
            )
        })?;
        lines.push(value);
    }
    Ok(serde_json::json!({ "jsonl": lines }))
}

fn map_reqwest_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        return ProviderError::new(
            "ocr",
            ProviderErrorKind::Timeout,
            format!("OCR request timed out: {error}"),
        );
    }
    if error.is_connect() || error.is_request() {
        return ProviderError::new(
            "ocr",
            ProviderErrorKind::Network,
            format!("OCR network request failed: {error}"),
        );
    }
    ProviderError::new(
        "ocr",
        ProviderErrorKind::Unknown,
        format!("OCR request failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    // reserved for future OCR client unit tests
}
