use super::artifact;
use super::render::authors::AcademicFrontMatter;
use super::runtime::fnv1a64_hex;
use super::structure::{HeaderBlockCandidate, HeaderBoundary};
use crate::error::AppError;
use crate::ocr::OcrDocumentWithRaw;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const OCR_CACHE_VERSION: u8 = 1;
const OCR_CACHE_FAMILY: &str = "ocr_raw";
const OCR_CACHE_CONTRACT: &str = "paddle-vl-1.6-layout-structure-raw-v1";
const FRONT_MATTER_CACHE_VERSION: u8 = 1;
const FRONT_MATTER_CACHE_FAMILY: &str = "front_matter";
const FRONT_MATTER_CACHE_CONTRACT: &str = "academic-front-matter-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OcrRawCacheKey {
    pub(super) key: String,
    pub(super) pdf_hash: String,
    pub(super) ocr_fingerprint_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrontMatterCacheKey {
    pub(super) key: String,
    pub(super) candidate_hash: String,
    pub(super) model_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcrRawCacheManifest {
    version: u8,
    cache_key: String,
    pdf_hash: String,
    ocr_fingerprint_hash: String,
    contract: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrontMatterCacheManifest {
    version: u8,
    cache_key: String,
    candidate_hash: String,
    model_hash: String,
    contract: String,
    created_at: String,
}

pub(super) fn build_ocr_raw_cache_key(
    pdf_path: &Path,
    ocr_fingerprint: &str,
) -> Result<OcrRawCacheKey, AppError> {
    let pdf_bytes = std::fs::read(pdf_path).map_err(|error| {
        AppError::internal(format!(
            "read PDF for OCR cache key failed({}): {error}",
            pdf_path.display()
        ))
    })?;
    let pdf_hash = fnv1a64_hex(&pdf_bytes);
    let ocr_fingerprint_hash = fnv1a64_hex(ocr_fingerprint.as_bytes());
    let key =
        fnv1a64_hex(format!("{OCR_CACHE_CONTRACT}|{pdf_hash}|{ocr_fingerprint_hash}").as_bytes());
    Ok(OcrRawCacheKey {
        key,
        pdf_hash,
        ocr_fingerprint_hash,
    })
}

pub(super) fn load_ocr_raw_cache(
    root_dir: &Path,
    key: &OcrRawCacheKey,
) -> Result<Option<OcrDocumentWithRaw>, AppError> {
    let cache_dir = cache_entry_dir(root_dir, OCR_CACHE_FAMILY, &key.key);
    let manifest_path = cache_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest = artifact::read_json::<OcrRawCacheManifest>(&manifest_path)?;
    if !ocr_manifest_matches(&manifest, key) {
        return Ok(None);
    }
    let layout_raw_json =
        artifact::read_json::<serde_json::Value>(&cache_dir.join("layout_raw.json"))?;
    let structure_raw_json =
        artifact::read_json::<serde_json::Value>(&cache_dir.join("structure_raw.json"))?;
    let raw = OcrDocumentWithRaw::from_raw_json(layout_raw_json, structure_raw_json)
        .map_err(AppError::from_provider_error)?;
    Ok(Some(raw))
}

pub(super) async fn store_ocr_raw_cache(
    root_dir: &Path,
    key: &OcrRawCacheKey,
    raw_ocr: &OcrDocumentWithRaw,
) -> Result<(), AppError> {
    let cache_dir = cache_entry_dir(root_dir, OCR_CACHE_FAMILY, &key.key);
    artifact::write_json(&cache_dir.join("layout_raw.json"), &raw_ocr.layout_raw_json).await?;
    artifact::write_json(
        &cache_dir.join("structure_raw.json"),
        &raw_ocr.structure_raw_json,
    )
    .await?;
    artifact::write_json(
        &cache_dir.join("manifest.json"),
        &OcrRawCacheManifest {
            version: OCR_CACHE_VERSION,
            cache_key: key.key.clone(),
            pdf_hash: key.pdf_hash.clone(),
            ocr_fingerprint_hash: key.ocr_fingerprint_hash.clone(),
            contract: OCR_CACHE_CONTRACT.to_string(),
            created_at: Utc::now().to_rfc3339(),
        },
    )
    .await
}

pub(super) fn build_front_matter_cache_key(
    candidate: &HeaderBlockCandidate,
    active_model: &str,
) -> FrontMatterCacheKey {
    let candidate_hash = fnv1a64_hex(front_matter_candidate_cache_payload(candidate).as_bytes());
    let model_hash = fnv1a64_hex(active_model.trim().as_bytes());
    let key = fnv1a64_hex(
        format!("{FRONT_MATTER_CACHE_CONTRACT}|{candidate_hash}|{model_hash}").as_bytes(),
    );
    FrontMatterCacheKey {
        key,
        candidate_hash,
        model_hash,
    }
}

pub(super) fn load_front_matter_cache(
    root_dir: &Path,
    key: &FrontMatterCacheKey,
) -> Result<Option<AcademicFrontMatter>, AppError> {
    let cache_dir = cache_entry_dir(root_dir, FRONT_MATTER_CACHE_FAMILY, &key.key);
    let manifest_path = cache_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest = artifact::read_json::<FrontMatterCacheManifest>(&manifest_path)?;
    if !front_matter_manifest_matches(&manifest, key) {
        return Ok(None);
    }
    artifact::read_json::<AcademicFrontMatter>(&cache_dir.join("front_matter.json")).map(Some)
}

pub(super) async fn store_front_matter_cache(
    root_dir: &Path,
    key: &FrontMatterCacheKey,
    front_matter: &AcademicFrontMatter,
) -> Result<(), AppError> {
    let cache_dir = cache_entry_dir(root_dir, FRONT_MATTER_CACHE_FAMILY, &key.key);
    artifact::write_json(&cache_dir.join("front_matter.json"), front_matter).await?;
    artifact::write_json(
        &cache_dir.join("manifest.json"),
        &FrontMatterCacheManifest {
            version: FRONT_MATTER_CACHE_VERSION,
            cache_key: key.key.clone(),
            candidate_hash: key.candidate_hash.clone(),
            model_hash: key.model_hash.clone(),
            contract: FRONT_MATTER_CACHE_CONTRACT.to_string(),
            created_at: Utc::now().to_rfc3339(),
        },
    )
    .await
}

fn cache_entry_dir(root_dir: &Path, family: &str, key: &str) -> PathBuf {
    root_dir.join(family).join(key)
}

fn ocr_manifest_matches(manifest: &OcrRawCacheManifest, key: &OcrRawCacheKey) -> bool {
    manifest.version == OCR_CACHE_VERSION
        && manifest.cache_key == key.key
        && manifest.pdf_hash == key.pdf_hash
        && manifest.ocr_fingerprint_hash == key.ocr_fingerprint_hash
        && manifest.contract == OCR_CACHE_CONTRACT
}

fn front_matter_manifest_matches(
    manifest: &FrontMatterCacheManifest,
    key: &FrontMatterCacheKey,
) -> bool {
    manifest.version == FRONT_MATTER_CACHE_VERSION
        && manifest.cache_key == key.key
        && manifest.candidate_hash == key.candidate_hash
        && manifest.model_hash == key.model_hash
        && manifest.contract == FRONT_MATTER_CACHE_CONTRACT
}

fn front_matter_candidate_cache_payload(candidate: &HeaderBlockCandidate) -> String {
    format!(
        "blocks:\n{}\n\nprobe:\n{}\n\nbody_start_block:{}\nboundary:{}",
        candidate.blocks.join("\n\n"),
        candidate.probe_blocks.join("\n\n"),
        candidate.body_start_block,
        header_boundary_label(candidate.boundary)
    )
}

fn header_boundary_label(boundary: HeaderBoundary) -> &'static str {
    match boundary {
        HeaderBoundary::FirstBodyParagraph => "firstBodyParagraph",
        HeaderBoundary::FirstSecondLevelHeading => "firstSecondLevelHeading",
        HeaderBoundary::DocumentEnd => "documentEnd",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::render::authors::{AcademicAuthor, AcademicFrontMatter};

    #[test]
    fn front_matter_cache_key_changes_with_candidate_content() {
        let mut candidate = HeaderBlockCandidate {
            blocks: vec!["Title".to_string(), "Jane Doe".to_string()],
            probe_blocks: Vec::new(),
            body_start_block: 2,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };
        let original = build_front_matter_cache_key(&candidate, "model-a");

        candidate.blocks.push("Department A".to_string());
        let changed = build_front_matter_cache_key(&candidate, "model-a");

        assert_ne!(original.key, changed.key);
    }

    #[tokio::test]
    async fn front_matter_cache_round_trips_structured_metadata() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-front-matter-cache-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let candidate = HeaderBlockCandidate {
            blocks: vec!["Title".to_string(), "Jane Doe".to_string()],
            probe_blocks: Vec::new(),
            body_start_block: 2,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };
        let key = build_front_matter_cache_key(&candidate, "model-a");
        let front_matter = AcademicFrontMatter {
            title: Some("Title".to_string()),
            authors: vec![AcademicAuthor {
                name: "Jane Doe".to_string(),
                email: None,
                markers: Vec::new(),
                affiliations: vec!["Department A".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            }],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        store_front_matter_cache(&root, &key, &front_matter)
            .await
            .expect("cache should store");
        let loaded = load_front_matter_cache(&root, &key)
            .expect("cache should load")
            .expect("front matter should exist");

        assert_eq!(loaded.title.as_deref(), Some("Title"));
        assert_eq!(loaded.authors.len(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ocr_raw_cache_round_trips_raw_json_and_rebuilds_document() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-ocr-raw-cache-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let raw_json = serde_json::json!({
            "errorCode": 0,
            "result": {
                "layoutParsingResults": [{
                    "markdown": {
                        "text": "# Title\n\nBody.",
                        "images": {}
                    },
                    "prunedResult": [{
                        "label": "text",
                        "content": "Body.",
                        "bbox": [0, 0, 100, 40]
                    }],
                    "outputImages": {},
                    "inputImage": null
                }]
            }
        });
        let raw_ocr =
            OcrDocumentWithRaw::from_raw_json(raw_json.clone(), raw_json).expect("raw OCR");
        let key = OcrRawCacheKey {
            key: "cache-key".to_string(),
            pdf_hash: "pdf-hash".to_string(),
            ocr_fingerprint_hash: "ocr-hash".to_string(),
        };

        store_ocr_raw_cache(&root, &key, &raw_ocr)
            .await
            .expect("OCR cache should store");
        let loaded = load_ocr_raw_cache(&root, &key)
            .expect("OCR cache should load")
            .expect("OCR cache should exist");

        assert!(loaded.document.markdown.contains("Body."));
        assert_eq!(loaded.layout_results.len(), 1);
        assert_eq!(loaded.document.page_layouts[0].blocks[0].label, "text");

        let _ = std::fs::remove_dir_all(root);
    }
}
