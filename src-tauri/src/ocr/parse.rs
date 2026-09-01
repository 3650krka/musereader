use super::{
    LayoutParsingResult, OcrDocument, OcrJsonlLine, OcrLayoutBlock, OcrPageLayout, OcrResponse,
};
use crate::error::{ProviderError, ProviderErrorKind};
use std::collections::HashMap;

pub(super) fn ocr_response_to_layout_results(
    response: OcrResponse,
) -> Result<Vec<LayoutParsingResult>, ProviderError> {
    if response.error_code.unwrap_or(0) != 0 {
        return Err(map_ocr_business_error(
            response.error_code.unwrap_or_default(),
            response
                .error_msg
                .unwrap_or_else(|| "OCR business error".to_string()),
        ));
    }
    let _ = &response.log_id;
    let result = response.result.ok_or_else(|| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::EmptyResponse,
            "OCR response did not include result",
        )
    })?;
    Ok(result.layout_parsing_results)
}

pub(super) fn parse_jsonl_layout_results(
    jsonl: &str,
) -> Result<Vec<LayoutParsingResult>, ProviderError> {
    let mut results = Vec::new();
    for line in jsonl.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let item: OcrJsonlLine = serde_json::from_str(trimmed).map_err(|error| {
            ProviderError::new(
                "ocr",
                ProviderErrorKind::Decode,
                format!(
                    "failed to parse OCR JSONL line: {error}; line={}",
                    preview_text(trimmed)
                ),
            )
        })?;
        results.extend(item.result.layout_parsing_results);
    }
    Ok(results)
}

pub(super) fn raw_ocr_value_to_layout_results(
    value: &serde_json::Value,
) -> Result<Vec<LayoutParsingResult>, ProviderError> {
    if let Some(lines) = value.get("jsonl").and_then(serde_json::Value::as_array) {
        return parse_raw_jsonl_values(lines);
    }
    let response = serde_json::from_value::<OcrResponse>(value.clone()).map_err(|error| {
        ProviderError::new(
            "ocr",
            ProviderErrorKind::Decode,
            format!(
                "failed to parse cached OCR response JSON: {error}; body={}",
                preview_text(&value.to_string())
            ),
        )
    })?;
    ocr_response_to_layout_results(response)
}

pub(super) fn raw_ocr_value_is_layout_fallback(value: &serde_json::Value) -> bool {
    value
        .get("fallback")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|fallback| fallback.eq_ignore_ascii_case("layout"))
}

fn parse_raw_jsonl_values(
    lines: &[serde_json::Value],
) -> Result<Vec<LayoutParsingResult>, ProviderError> {
    let mut results = Vec::new();
    for line in lines {
        let item = serde_json::from_value::<OcrJsonlLine>(line.clone()).map_err(|error| {
            ProviderError::new(
                "ocr",
                ProviderErrorKind::Decode,
                format!(
                    "failed to parse cached OCR JSONL line: {error}; line={}",
                    preview_text(&line.to_string())
                ),
            )
        })?;
        results.extend(item.result.layout_parsing_results);
    }
    Ok(results)
}

pub(super) fn ocr_layouts_to_document_with_layout_source(
    markdown_layouts: Vec<LayoutParsingResult>,
    layout_source: Vec<LayoutParsingResult>,
) -> Result<OcrDocument, ProviderError> {
    let mut merged_markdown = String::new();
    let mut first_cover: Option<String> = None;
    let mut page_layouts = Vec::new();
    let markdown_images = collect_markdown_images(&markdown_layouts, &layout_source);

    for (page_index, result) in layout_source.into_iter().enumerate() {
        page_layouts.push(OcrPageLayout {
            blocks: parse_pruned_layout_blocks(&result.pruned_result, page_index),
        });
    }

    for result in markdown_layouts {
        if first_cover.is_none() {
            first_cover = first_image_data_url(&result.markdown.images);
        }
        merged_markdown.push_str(&result.markdown.text);
        merged_markdown.push_str("\n\n");
    }

    let markdown = merged_markdown.trim();
    if markdown.is_empty() {
        return Err(ProviderError::new(
            "ocr",
            ProviderErrorKind::EmptyResponse,
            "OCR did not return usable text",
        ));
    }

    Ok(OcrDocument {
        markdown: markdown.to_string(),
        cover_data_url: first_cover,
        page_layouts,
        markdown_images,
        toc: None,
        epub_blocks: Vec::new(),
        source_blocks: crate::document::build_source_blocks_from_markdown(markdown),
    })
}

fn parse_pruned_layout_blocks(value: &serde_json::Value, page_index: usize) -> Vec<OcrLayoutBlock> {
    parse_layout_block_array(value)
        .or_else(|| {
            [
                "blocks",
                "layoutDetections",
                "layout_det_res",
                "parsingResList",
                "parsing_res_list",
            ]
            .iter()
            .find_map(|key| value.get(key).and_then(parse_layout_block_array))
        })
        .map(|blocks| fill_missing_block_pages(blocks, page_index))
        .unwrap_or_default()
}

fn parse_layout_block_array(value: &serde_json::Value) -> Option<Vec<OcrLayoutBlock>> {
    let array = value.as_array()?;
    let blocks = array
        .iter()
        .filter_map(parse_layout_block)
        .collect::<Vec<_>>();
    Some(blocks)
}

fn parse_layout_block(value: &serde_json::Value) -> Option<OcrLayoutBlock> {
    let label = first_string(
        value,
        &["label", "blockLabel", "block_label", "type", "category"],
    )?;
    Some(OcrLayoutBlock {
        label,
        content: first_string(
            value,
            &[
                "content",
                "text",
                "blockContent",
                "block_content",
                "recText",
                "rec_text",
            ],
        )
        .unwrap_or_default(),
        bbox: first_bbox(
            value,
            &["bbox", "blockBbox", "block_bbox", "box", "coordinate"],
        )
        .unwrap_or_default(),
        order: first_i32(value, &["order", "index", "blockOrder", "block_order"]),
        page: first_i32(value, &["page", "pageIndex", "page_index"]),
    })
}

fn fill_missing_block_pages(blocks: Vec<OcrLayoutBlock>, page_index: usize) -> Vec<OcrLayoutBlock> {
    blocks
        .into_iter()
        .map(|mut block| {
            block.page.get_or_insert(page_index as i32);
            block
        })
        .collect()
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(key))
        .find_map(json_string)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn json_string(value: &serde_json::Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .as_object()
            .and_then(|object| object.get("text"))
            .and_then(serde_json::Value::as_str)
    })
}

fn first_i32(value: &serde_json::Value, keys: &[&str]) -> Option<i32> {
    keys.iter()
        .filter_map(|key| value.get(key))
        .find_map(json_i32)
}

fn json_i32(value: &serde_json::Value) -> Option<i32> {
    value
        .as_i64()
        .and_then(|number| i32::try_from(number).ok())
        .or_else(|| value.as_f64().map(|number| number.round() as i32))
}

fn first_bbox(value: &serde_json::Value, keys: &[&str]) -> Option<Vec<i32>> {
    keys.iter()
        .filter_map(|key| value.get(key))
        .find_map(json_bbox)
}

fn json_bbox(value: &serde_json::Value) -> Option<Vec<i32>> {
    let values = flatten_numbers(value);
    normalize_bbox_numbers(&values)
}

fn normalize_bbox_numbers(values: &[i32]) -> Option<Vec<i32>> {
    if values.len() < 4 {
        return None;
    }
    if values.len() >= 8 {
        let points = values.chunks_exact(2).take(4).collect::<Vec<_>>();
        let left = points.iter().filter_map(|point| point.first()).min()?;
        let top = points.iter().filter_map(|point| point.get(1)).min()?;
        let right = points.iter().filter_map(|point| point.first()).max()?;
        let bottom = points.iter().filter_map(|point| point.get(1)).max()?;
        return (right > left && bottom > top).then(|| vec![*left, *top, *right, *bottom]);
    }
    let bbox = values.iter().take(4).copied().collect::<Vec<_>>();
    (bbox[2] > bbox[0] && bbox[3] > bbox[1]).then_some(bbox)
}

fn flatten_numbers(value: &serde_json::Value) -> Vec<i32> {
    match value {
        serde_json::Value::Number(_) => json_i32(value).into_iter().collect(),
        serde_json::Value::Array(items) => items.iter().flat_map(flatten_numbers).collect(),
        _ => Vec::new(),
    }
}

fn collect_markdown_images(
    markdown_layouts: &[LayoutParsingResult],
    layout_source: &[LayoutParsingResult],
) -> HashMap<String, String> {
    let mut images = HashMap::new();
    for layout in markdown_layouts {
        for (path, value) in &layout.markdown.images {
            images.entry(path.clone()).or_insert_with(|| value.clone());
        }
    }
    for (page_index, layout) in layout_source.iter().enumerate() {
        for (name, value) in &layout.output_images {
            let path = format!("ocr_output/page_{page_index}_{name}.jpg");
            images.entry(path).or_insert_with(|| value.clone());
        }
        if let Some(value) = layout.input_image.as_deref() {
            let path = format!("ocr_input/page_{page_index}.jpg");
            images.entry(path).or_insert_with(|| value.to_string());
        }
    }
    images
}

fn first_image_data_url(images: &HashMap<String, String>) -> Option<String> {
    let mut entries: Vec<_> = images.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    entries
        .first()
        .map(|(_, value)| normalize_image_data_url(value))
}

pub(super) fn normalize_image_data_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("data:image") || trimmed.starts_with("http") {
        return trimmed.to_string();
    }
    format!("data:image/png;base64,{}", trimmed)
}

pub(super) fn should_fallback_to_async(error: &ProviderError) -> bool {
    matches!(
        error.kind,
        ProviderErrorKind::Timeout
            | ProviderErrorKind::Decode
            | ProviderErrorKind::EmptyResponse
            | ProviderErrorKind::Server
            | ProviderErrorKind::Unknown
    )
}

pub(super) fn map_ocr_business_error(code: i32, message: String) -> ProviderError {
    let kind = match code {
        6 | 19 | 100 | 110 | 111 => ProviderErrorKind::Authentication,
        4 | 14 => ProviderErrorKind::RateLimited { retry_after: None },
        216200..=216299 => ProviderErrorKind::BadRequest,
        _ => ProviderErrorKind::Unknown,
    };
    ProviderError::new("ocr", kind, format!("OCR error {code}: {message}"))
}

pub(super) fn preview_text(value: &str) -> String {
    const LIMIT: usize = 500;
    let mut output = value.chars().take(LIMIT).collect::<String>();
    if value.chars().count() > LIMIT {
        output.push_str("...");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn document_can_use_structure_markdown_with_layout_blocks() {
        let structure = LayoutParsingResult {
            markdown: super::super::MarkdownResult {
                text: "# Title\n\n## Acknowledgments\n\nThanks.\n\n## References\n\nRef."
                    .to_string(),
                images: HashMap::new(),
            },
            pruned_result: json!([]),
            output_images: HashMap::new(),
            input_image: None,
        };
        let layout = LayoutParsingResult {
            markdown: super::super::MarkdownResult {
                text: "# Title\n\nThanks.\n\n## References\n\nRef.".to_string(),
                images: HashMap::new(),
            },
            pruned_result: json!([
                {
                    "label": "footnote",
                    "content": "1 Department of Example.",
                    "bbox": [10, 20, 200, 40]
                }
            ]),
            output_images: HashMap::new(),
            input_image: None,
        };

        let document =
            ocr_layouts_to_document_with_layout_source(vec![structure], vec![layout]).unwrap();

        assert!(document.markdown.contains("## Acknowledgments"));
        assert_eq!(document.page_layouts.len(), 1);
        assert_eq!(document.page_layouts[0].blocks.len(), 1);
        assert_eq!(document.page_layouts[0].blocks[0].label, "footnote");
    }

    #[test]
    fn parses_pruned_result_when_top_level_is_block_array() {
        let value = json!([
            {
                "label": "footnote",
                "content": "1 Department of Example.",
                "bbox": [10, 20, 300, 40],
                "order": 9
            }
        ]);

        let blocks = parse_pruned_layout_blocks(&value, 0);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label, "footnote");
        assert_eq!(blocks[0].content, "1 Department of Example.");
        assert_eq!(blocks[0].bbox, vec![10, 20, 300, 40]);
        assert_eq!(blocks[0].order, Some(9));
    }

    #[test]
    fn parses_pruned_result_when_blocks_are_nested_under_common_keys() {
        let value = json!({
            "parsingResList": [
                {
                    "block_label": "footnote",
                    "block_content": "e-mail: jane@example.test",
                    "block_bbox": [[72.0, 700.0], [520.0, 700.0], [520.0, 735.0], [72.0, 735.0]],
                    "block_order": 12,
                    "page_index": 0
                }
            ]
        });

        let blocks = parse_pruned_layout_blocks(&value, 0);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label, "footnote");
        assert_eq!(blocks[0].content, "e-mail: jane@example.test");
        assert_eq!(blocks[0].bbox, vec![72, 700, 520, 735]);
        assert_eq!(blocks[0].order, Some(12));
        assert_eq!(blocks[0].page, Some(0));
    }

    #[test]
    fn normalizes_polygon_bbox_to_bounds() {
        let value = json!({
            "blocks": [
                {
                    "label": "table",
                    "bbox": [[50, 80], [260, 78], [262, 180], [48, 184]]
                }
            ]
        });

        let blocks = parse_pruned_layout_blocks(&value, 3);

        assert_eq!(blocks[0].bbox, vec![48, 78, 262, 184]);
        assert_eq!(blocks[0].page, Some(3));
    }
}
