//! NER 预提取（pre-translation entity extraction）：翻译前对全书采样段落做
//! 一次性 LLM 实体提取（角色/地点/组织/物品/标题），产出静态词表注入翻译上下文。
//!
//! 与 first-seen lock 的互补关系：
//! - NER 预提取覆盖**首批 chunk**（第一个 chunk 翻译时词表已就位，避免首见漂移）；
//! - first-seen lock 覆盖**长尾**（NER 采样未覆盖的章节里新出现的实体）。
//!
//! 设计：
//! - 单次调用（我们的 chunk 采样比 map-reduce 便宜：全书长尾由 first-seen lock 兜底）；
//! - 采样策略复用 style_profile 的首/中/尾分布式采样（同一本书两份采样一致）；
//! - evidence-only gender：prompt 强制 "unknown 是正确答案，猜错比空着更糟"；
//! - 结果按 source_hash 缓存到 ner_glossary.json，失败静默降级；
//! - MUSETRANSLATE_PRE_NER=1 开启（默认关：短文档/技术文档收益不明显）。

use crate::error::AppError;
use crate::llm::SensenovaClient;
use crate::pipeline::{ProperNounEnforcement, ProperNounHint};
use serde::{Deserialize, Serialize};
use std::path::Path;

const NER_GLOSSARY_VERSION: u8 = 1;
const NER_TERM_LIMIT: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct NerGlossary {
    pub version: u8,
    pub source_hash: String,
    pub entries: Vec<NerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct NerEntry {
    pub source: String,
    pub target: String,
    pub category: String,
    #[serde(default)]
    pub gender: String,
}

/// 从全书 markdown 提取实体词表；失败返回 None（降级，不阻塞）。
pub(super) async fn extract_ner_glossary(
    llm_client: &SensenovaClient,
    source_hash: &str,
    source_markdown: &str,
) -> Option<NerGlossary> {
    let sample = super::style_profile::sample_passages_for_style(source_markdown);
    if sample.trim().is_empty() {
        return None;
    }
    let prompt = build_ner_extraction_prompt(&sample);
    let route = llm_client.reserve_route();
    let response = llm_client
        .review_translation_issue_on_route(&route, &prompt)
        .await
        .ok()?;
    parse_ner_response(source_hash, &response)
}

fn build_ner_extraction_prompt(sample: &str) -> String {
    format!(
        "You are a named-entity extraction expert for a book translation pipeline. Read the excerpt and extract recurring proper nouns worth a locked Chinese translation.\n\
         \n\
         Output JSON only, with this exact shape:\n\
         {{\"entities\": [{{\"source\": \"...\", \"target\": \"...\", \"category\": \"...\", \"gender\": \"...\"}}]}}\n\
         \n\
         Requirements:\n\
         - source: the exact surface form from the text (preserve case/punctuation). Proper nouns only — no common nouns.\n\
         - target: a natural Simplified Chinese rendering (transliterate person/place names).\n\
         - category: one of character | location | organization | item | title | other.\n\
         - gender: ONLY from passage evidence (pronouns like she/her, kinship/address like Miss/Lady/mother). \"unknown\" is the correct, expected answer when evidence is absent — a wrong guess is far more damaging than an honest unknown, because downstream systems trust stated genders but review unknowns. Never infer from the name alone.\n\
         - Only include entities that appear at least twice or are clearly central (main character, primary location).\n\
         - At most {NER_TERM_LIMIT} entities, most important first.\n\
         \n\
         Excerpt:\n\
         ---\n\
         {sample}\n\
         ---"
    )
}

fn parse_ner_response(source_hash: &str, response: &str) -> Option<NerGlossary> {
    let json_text = extract_json_object(response)?;
    let parsed: serde_json::Value = serde_json::from_str(&json_text).ok()?;
    let entities = parsed
        .get("entities")
        .and_then(serde_json::Value::as_array)?;
    let mut seen = std::collections::HashSet::new();
    let entries = entities
        .iter()
        .filter_map(serde_json::Value::as_object)
        .filter_map(|obj| {
            let source = obj
                .get("source")
                .and_then(serde_json::Value::as_str)?
                .trim();
            let target = obj
                .get("target")
                .and_then(serde_json::Value::as_str)?
                .trim();
            if source.is_empty() || target.is_empty() {
                return None;
            }
            let category = obj
                .get("category")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("other")
                .trim()
                .to_ascii_lowercase();
            let gender = obj
                .get("gender")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .trim()
                .to_ascii_lowercase();
            if !seen.insert(source.to_ascii_lowercase()) {
                return None;
            }
            Some(NerEntry {
                source: source.to_string(),
                target: target.to_string(),
                category,
                gender,
            })
        })
        .take(NER_TERM_LIMIT)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    Some(NerGlossary {
        version: NER_GLOSSARY_VERSION,
        source_hash: source_hash.to_string(),
        entries,
    })
}

fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(text[start..start + offset + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// 把 NER 词表转为 ProperNounHint（target 预填 = 预锁定，Strict 级）。
/// 与 first-seen lock 兼容：hint.target.is_some() 的项不会被 lock 覆盖。
pub(super) fn ner_entries_to_hints(entries: &[NerEntry]) -> Vec<ProperNounHint> {
    entries
        .iter()
        .map(|entry| ProperNounHint {
            source: entry.source.clone(),
            target: Some(entry.target.clone()),
            enforcement: ProperNounEnforcement::Strict,
            occurrences: 0,
            first_chunk_index: 0,
            chunk_indexes: Vec::new(),
        })
        .collect()
}

pub(super) fn load_cached_ner_glossary(path: &Path, source_hash: &str) -> Option<NerGlossary> {
    let raw = std::fs::read_to_string(path).ok()?;
    let glossary: NerGlossary = serde_json::from_str(&raw).ok()?;
    if glossary.version == NER_GLOSSARY_VERSION && glossary.source_hash == source_hash {
        Some(glossary)
    } else {
        None
    }
}

pub(super) fn persist_ner_glossary(path: &Path, glossary: &NerGlossary) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(glossary)
        .map_err(|error| AppError::internal(format!("serialize ner glossary failed: {error}")))?;
    std::fs::write(path, json)
        .map_err(|error| AppError::internal(format!("write ner glossary failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entities_with_gender_and_category() {
        let response = r#"{"entities": [
            {"source": "Sophie", "target": "苏菲", "category": "character", "gender": "female"},
            {"source": "Hallownest", "target": "圣巢", "category": "location", "gender": "unknown"}
        ]}"#;
        let glossary = parse_ner_response("hash-1", response).expect("should parse");
        assert_eq!(glossary.entries.len(), 2);
        assert_eq!(glossary.entries[0].source, "Sophie");
        assert_eq!(glossary.entries[0].gender, "female");
        assert_eq!(glossary.entries[1].category, "location");
    }

    #[test]
    fn dedupes_by_source_case_insensitively() {
        let response = r#"{"entities": [
            {"source": "Sophie", "target": "苏菲", "category": "character"},
            {"source": "sophie", "target": "索菲", "category": "character"}
        ]}"#;
        let glossary = parse_ner_response("hash-1", response).expect("should parse");
        assert_eq!(glossary.entries.len(), 1);
    }

    #[test]
    fn skips_entries_missing_source_or_target() {
        let response = r#"{"entities": [
            {"source": "", "target": "苏菲", "category": "character"},
            {"source": "Sophie", "target": "", "category": "character"},
            {"source": "Hallownest", "target": "圣巢", "category": "location"}
        ]}"#;
        let glossary = parse_ner_response("hash-1", response).expect("should parse");
        assert_eq!(glossary.entries.len(), 1);
        assert_eq!(glossary.entries[0].source, "Hallownest");
    }

    #[test]
    fn empty_entities_yield_none() {
        let response = r#"{"entities": []}"#;
        assert!(parse_ner_response("hash-1", response).is_none());
    }

    #[test]
    fn hints_are_strict_with_prefilled_target() {
        let entries = vec![NerEntry {
            source: "Sophie".to_string(),
            target: "苏菲".to_string(),
            category: "character".to_string(),
            gender: "female".to_string(),
        }];
        let hints = ner_entries_to_hints(&entries);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].target.as_deref(), Some("苏菲"));
        assert_eq!(hints[0].enforcement, ProperNounEnforcement::Strict);
    }

    #[test]
    fn cached_glossary_requires_matching_source_hash() {
        let root = std::env::temp_dir().join(format!(
            "musetranslate-ner-glossary-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("ner_glossary.json");
        let glossary = NerGlossary {
            version: NER_GLOSSARY_VERSION,
            source_hash: "hash-a".to_string(),
            entries: vec![NerEntry {
                source: "Sophie".to_string(),
                target: "苏菲".to_string(),
                category: "character".to_string(),
                gender: "female".to_string(),
            }],
        };
        persist_ner_glossary(&path, &glossary).expect("persist");
        assert!(load_cached_ner_glossary(&path, "hash-a").is_some());
        assert!(load_cached_ner_glossary(&path, "hash-b").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
