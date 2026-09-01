use super::*;
use crate::commands::{
    load_runtime_config, load_runtime_env_sources, resolve_env_defaults, resolve_runtime_paths,
};
use crate::skill_library::{ensure_seeded, load_translation_prompt_bundle};
use std::path::PathBuf;

fn test_env_value(primary: &str, aliases: &[&str]) -> Option<String> {
    std::env::var(primary)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            aliases
                .iter()
                .find_map(|name| std::env::var(name).ok())
                .filter(|value| !value.trim().is_empty())
        })
}

#[tokio::test]
async fn llm_encoding_smoke_from_env() {
    if !should_run_llm_encoding_smoke() {
        return;
    }
    let Some(api_key) = test_env_value("MUSETRANSLATE_LLM_API_KEY", &["SENSENOVA_API_KEY"]) else {
        return;
    };
    let api_url = test_env_value(
        "MUSETRANSLATE_LLM_API_URL",
        &["LLM_API_URL", "SENSENOVA_BASE_URL", "OPENAI_BASE_URL"],
    )
    .unwrap_or_else(|| "https://token.sensenova.cn/v1/chat/completions".to_string());
    let model = test_env_value("MUSETRANSLATE_LLM_MODEL", &["LLM_MODEL", "SENSENOVA_MODEL"])
        .unwrap_or_else(|| "deepseek-v4-flash".to_string());
    let client = SensenovaClient::new(api_url, api_key, model);
    let response = client
        .translate_markdown_with_context(
            "Language framing shapes social norm perception.",
            "academic",
            "Translate faithfully into natural Simplified Chinese. Return only the translation.",
            None,
        )
        .await
        .expect("LLM smoke translation should succeed");
    write_llm_encoding_smoke_artifact(&response.translated_text);
    assert_llm_response_is_utf8_chinese(&response.translated_text);
}

#[tokio::test]
#[ignore = "real LLM structured translation smoke"]
async fn llm_structured_translation_two_paragraph_fiction_smoke() {
    let runtime_paths = resolve_runtime_paths();
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    if runtime_config.llm_api_key.trim().is_empty() || runtime_config.llm_api_url.trim().is_empty()
    {
        return;
    }

    let skill_root = runtime_paths.skill_library_root_dir;
    ensure_seeded(&skill_root).expect("skill library should seed");
    let prompt_bundle =
        load_translation_prompt_bundle(&skill_root, "fiction").expect("fiction prompt bundle");
    let client = SensenovaClient::new(
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let sample = satampra_zeiros_two_paragraph_sample();

    let response = client
        .translate_markdown_with_context(sample, "fiction", &prompt_bundle.system_prompt, None)
        .await
        .expect("structured translation should succeed");

    write_llm_structured_smoke_artifact("satampra-two-paragraph", &response);
    assert!(!response.translated_text.trim().is_empty());
    let surfaced_terms = response
        .confirmed_terms
        .iter()
        .filter(|term| response.translated_text.contains(&term.translation))
        .count();
    assert!(response.confirmed_terms.iter().all(|term| {
        source_surface_matches_sample(sample, &term.source) && term.confidence <= 100
    }));
    assert!(surfaced_terms > 0);
}

fn source_surface_matches_sample(sample: &str, source: &str) -> bool {
    sample.contains(source)
        || sample.contains(&source.replace("- ", "-").replace(" -", "-"))
        || sample.contains(&source.replace('-', "- ").replace("  ", " "))
}

fn should_run_llm_encoding_smoke() -> bool {
    std::env::var("MUSETRANSLATE_RUN_LLM_ENCODING_SMOKE")
        .ok()
        .as_deref()
        == Some("1")
}

fn write_llm_encoding_smoke_artifact(response: &str) {
    let artifact_dir = llm_encoding_smoke_artifact_dir();
    std::fs::create_dir_all(&artifact_dir).expect("smoke artifact dir should be created");
    std::fs::write(artifact_dir.join("response.txt"), response.as_bytes())
        .expect("smoke response should be writable");
}

fn llm_encoding_smoke_artifact_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".runtime")
        .join("artifacts")
        .join("llm-encoding-smoke")
}

fn write_llm_structured_smoke_artifact(task_id: &str, response: &StructuredTranslation) {
    let artifact_dir = llm_encoding_smoke_artifact_dir().join(task_id);
    std::fs::create_dir_all(&artifact_dir).expect("structured smoke artifact dir");
    std::fs::write(
        artifact_dir.join("response.json"),
        serde_json::to_vec_pretty(response).expect("serialize structured response"),
    )
    .expect("structured smoke response should be writable");
}

fn assert_llm_response_is_utf8_chinese(response: &str) {
    for marker in ["\u{9435}?", "\u{95b9}?", "\u{5a11}?", "\u{95bf}?"] {
        assert!(!response.contains(marker));
    }
    assert!(response.chars().any(is_cjk_unified_ideograph));
}

fn is_cjk_unified_ideograph(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

fn satampra_zeiros_two_paragraph_sample() -> &'static str {
    "# The Tale of Satampra Zeiros\n\nI , Satampra Zeiros of Uzuldaroum, shall write with my left hand, since I have no longer any other, the tale of everything that befell Tirouv Ompallios and myself in the shrine of the god Tsathoggua, which lies neglected by the worship of man in the jungle-taken suburbs of Commoriom, that long-deserted capital of the Hyperborean rulers. I shall write it with the violet juice of the  suvana- palm, which turns to a blood-red rubric with the passage of years, on a strong vellum that is made from the skin of the mastodon, as a warning to all good thieves and adventurers who may hear some lying legend of the lost treasures of Commoriom and be tempted thereby.\n\nNow, Tirouv Ompallios was my life-long friend and my trustworthy companion in all such enterprises as require deft fingers and a habit of mind both agile and adroit. I can say without flattering myself, or Tirouv Ompallios either, that we carried to an incomparable success more than one undertaking from which fellow-craftsmen of a much wider renown than ourselves might well have recoiled in dismay. To be more explicit, I refer to the theft of the jewels of Queen Cunambria, which were kept in a room where two-score venomous reptiles wandered at will; and the breaking of the adamantine box of Acromi, in which were all the medallions of an early dynasty of Hyperborean kings. It is true that these medallions were difficult and perilous to dispose of, and that we sold them at a dire sacrifice to the captain of a barbarian vessel from remote Lemuria: but nevertheless, the breaking of that box was a glorious feat, for it had to be done in absolute silence, on account of the proximity of a dozen guards who were all armed with tridents. We made use of a rare and mordant acid\u{9225}?but I must not linger too long and too garrulously by the way, however great the temptation to ramble on amid heroic memories and the high glamour of valiant or sleightful deeds."
}

#[test]
fn usage_from_json_maps_all_protocol_namings() {
    // OpenAI Chat 命名
    let chat: serde_json::Value = serde_json::from_str(
        r#"{"usage":{"prompt_tokens":100,"completion_tokens":40,"total_tokens":140,"prompt_tokens_details":{"cached_tokens":30}}}"#,
    )
    .expect("chat json");
    let usage = types::usage_from_json(&chat).expect("chat usage");
    assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (100, 40, 140));
    assert_eq!(usage.prompt_tokens_details.map(|d| d.cached_tokens), Some(30));

    // Anthropic Messages 命名（含 cache_read 归并为 cached）
    let anthropic: serde_json::Value = serde_json::from_str(
        r#"{"usage":{"input_tokens":80,"output_tokens":20,"cache_read_input_tokens":12}}"#,
    )
    .expect("anthropic json");
    let usage = types::usage_from_json(&anthropic).expect("anthropic usage");
    assert_eq!(usage.prompt_tokens, 80);
    assert_eq!(usage.completion_tokens, 20);
    assert_eq!(usage.total_tokens, 100, "缺 total 时应回退为 prompt+completion");
    assert_eq!(usage.prompt_tokens_details.map(|d| d.cached_tokens), Some(12));

    // OpenAI Responses 命名（input_tokens_details.cached_tokens）
    let responses: serde_json::Value = serde_json::from_str(
        r#"{"usage":{"input_tokens":50,"output_tokens":15,"total_tokens":65,"input_tokens_details":{"cached_tokens":5}}}"#,
    )
    .expect("responses json");
    let usage = types::usage_from_json(&responses).expect("responses usage");
    assert_eq!(usage.prompt_tokens, 50);
    assert_eq!(usage.completion_tokens, 15);
    assert_eq!(usage.prompt_tokens_details.map(|d| d.cached_tokens), Some(5));
}

#[test]
fn usage_from_json_treats_missing_or_empty_as_none() {
    let missing: serde_json::Value = serde_json::from_str(r#"{"id":"x"}"#).expect("json");
    assert!(types::usage_from_json(&missing).is_none());

    let empty: serde_json::Value = serde_json::from_str(r#"{"usage":{}}"#).expect("json");
    assert!(types::usage_from_json(&empty).is_none());

    let zeros: serde_json::Value =
        serde_json::from_str(r#"{"usage":{"input_tokens":0,"output_tokens":0}}"#).expect("json");
    assert!(types::usage_from_json(&zeros).is_none());

    let null_usage: serde_json::Value = serde_json::from_str(r#"{"usage":null}"#).expect("json");
    assert!(types::usage_from_json(&null_usage).is_none());
}
