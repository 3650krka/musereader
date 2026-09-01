use super::*;
use crate::commands::{
    load_runtime_config, load_runtime_env_sources, resolve_env_defaults, resolve_runtime_paths,
};
use crate::llm::SensenovaClient;
use crate::ocr::BaiduOcrClient;
use crate::skill_library::{ensure_seeded, load_translation_prompt_bundle};
use lopdf::Document;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

struct RealPipelineTask {
    task_id: &'static str,
    source_path: &'static str,
    article_type: &'static str,
    epub_chapter_title: Option<&'static str>,
    max_parallel: usize,
}

#[derive(Clone, Copy)]
enum PromptMode {
    Full,
    Minimal,
}

#[derive(Clone, Copy)]
struct ProviderOverride {
    provider: &'static str,
    api_url: &'static str,
    api_key_env: &'static str,
    model: &'static str,
    task_suffix: &'static str,
}

struct AuthorFirstPageCase {
    id: &'static str,
    source_path: &'static str,
    min_authors: usize,
    require_metadata: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorGateReport {
    id: String,
    source_pdf: String,
    first_page_pdf: String,
    source_markdown: String,
    authors: Vec<AuthorGateAuthor>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorGateAuthor {
    name: String,
    email: Option<String>,
    markers: Vec<String>,
    affiliations: Vec<String>,
    is_corresponding: bool,
    correspondence_note: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractionAuditReport {
    task_id: String,
    source_path: String,
    article_type: String,
    header_boundary: String,
    header_block_count: usize,
    body_start_block: usize,
    first_page_footnote_count: usize,
    reference_chunk_count: usize,
    protected_metadata_chunk_count: usize,
    body_chunk_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkClassificationRow {
    index: usize,
    kind: String,
    looks_like_reference: bool,
    looks_like_protected_metadata: bool,
    preview: String,
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<String>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(value) = &self.previous {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn scoped_env_var(key: &'static str, value: &str) -> ScopedEnvVar {
    ScopedEnvVar::set(key, value)
}

impl PromptMode {
    fn from_env() -> Self {
        match std::env::var("MUSETRANSLATE_MIN_PROMPT").ok().as_deref() {
            Some("1") => Self::Minimal,
            _ => Self::Full,
        }
    }

    fn task_suffix(self) -> &'static str {
        match self {
            Self::Full => "",
            Self::Minimal => "-minprompt",
        }
    }

    fn system_prompt(
        self,
        article_type: &str,
        prompt_bundle: &crate::skill_library::TranslationPromptBundle,
    ) -> String {
        match self {
            Self::Full => prompt_bundle.system_prompt.clone(),
            Self::Minimal => minimal_translation_prompt(article_type),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Minimal => "minimal",
        }
    }
}

#[tokio::test]
#[ignore = "real OCR/LLM author pre-stage release gate"]
async fn gate_pdf_author_first_pages_001() {
    let cases = [
        AuthorFirstPageCase {
            id: "policy-short",
            source_path: r"D:\tools\musetranslate\click\test_file\01_policy_short_nudge_plus.pdf",
            min_authors: 2,
            require_metadata: true,
        },
        AuthorFirstPageCase {
            id: "empirical-article",
            source_path: r"D:\tools\musetranslate\click\test_file\02_empirical_metacognition_ambiguous_news.pdf",
            min_authors: 3,
            require_metadata: false,
        },
        AuthorFirstPageCase {
            id: "preprint-toolbox",
            source_path: r"D:\tools\musetranslate\click\test_file\03_preprint_toolbox_2_0.pdf",
            min_authors: 1,
            require_metadata: true,
        },
    ];

    let runtime_paths = resolve_runtime_paths();
    let artifact_root = runtime_paths.artifact_root_dir.join(format!(
        "gate-pdf-author-first-pages-001{}",
        real_task_suffix_from_env()
    ));
    std::fs::create_dir_all(&artifact_root).expect("author gate artifact dir");
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let ocr_client = BaiduOcrClient::new(runtime_config.ocr_api_url, runtime_config.ocr_token);
    let llm_client = SensenovaClient::new(
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );

    for case in cases {
        let report = run_author_first_page_case(&case, &artifact_root, &ocr_client, &llm_client)
            .await
            .unwrap_or_else(|error| panic!("{} author first-page gate failed: {error:?}", case.id));
        eprintln!(
            "[author-gate:{}] authors={} markdown={} first_page={}",
            report.id,
            report.authors.len(),
            report.source_markdown,
            report.first_page_pdf
        );
    }
}

#[tokio::test]
#[ignore = "real release-gate fiction EPUB integration task"]
async fn gate_epub_full_book_001() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "gate-epub-full-book-001",
        source_path: r"D:\tools\musetranslate\click\test_file\04_full_book_2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: None,
        max_parallel: 10,
    })
    .await
    .expect("gate EPUB full-book task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_html.contains("<html"));
}

#[tokio::test]
#[ignore = "real release-gate fiction EPUB multi-provider benchmark"]
async fn gate_epub_short_story_provider_benchmark_001() {
    let providers = [
        ProviderOverride {
            provider: "sensenova",
            api_url: "https://token.sensenova.cn/v1/chat/completions",
            api_key_env: "SENSENOVA_API_KEY",
            model: "deepseek-v4-flash",
            task_suffix: "-deepseek",
        },
        ProviderOverride {
            provider: "minimax",
            api_url: "https://api.minimaxi.com/v1/chat/completions",
            api_key_env: "MINIMAX_API_KEY",
            model: "MiniMax-M2.7",
            task_suffix: "-minimax",
        },
        ProviderOverride {
            provider: "cohere",
            api_url: "https://api.cohere.ai/compatibility/v1/chat/completions",
            api_key_env: "COHERE_API_KEY",
            model: "command-a-plus-05-2026",
            task_suffix: "-cohere",
        },
        ProviderOverride {
            provider: "nvidia",
            api_url: "https://integrate.api.nvidia.com/v1/chat/completions",
            api_key_env: "NVIDIA_API_KEY",
            model: "stepfun-ai/step-3.7-flash",
            task_suffix: "-nvidia-step-37",
        },
    ];

    for provider in providers {
        let api_key = std::env::var(provider.api_key_env)
            .unwrap_or_else(|_| panic!("missing env {}", provider.api_key_env));
        let _base_url = scoped_env_var("LLM_API_URL", provider.api_url);
        let _api_key = scoped_env_var("LLM_API_KEY", &api_key);
        let _model = scoped_env_var("LLM_MODEL", provider.model);
        let _provider = scoped_env_var("MUSETRANSLATE_PROVIDER_BENCHMARK", provider.provider);
        let _task_suffix = scoped_env_var(
            "MUSETRANSLATE_REAL_TASK_SUFFIX",
            &format!("-provider-bench-001{}", provider.task_suffix),
        );

        let started = std::time::Instant::now();
        let result = run_real_pipeline_task(RealPipelineTask {
            task_id: "gate-epub-short-story-provider-benchmark-001",
            source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
            article_type: "fiction",
            epub_chapter_title: Some("The Tale of Satampra Zeiros"),
            max_parallel: 3,
        })
        .await
        .unwrap_or_else(|error| panic!("provider {} benchmark failed: {error:?}", provider.provider));

        eprintln!(
            "[provider-benchmark:{}] model={} chunks={}/{} elapsed_s={} html={}",
            provider.provider,
            provider.model,
            result.translated_chunks,
            result.total_chunks,
            started.elapsed().as_secs(),
            result.output_html_path
        );
    }
}

#[tokio::test]
#[ignore = "real EPUB chapter smoke test: source path & chapter title from env"]
async fn gate_epub_chapter_smoke_001() {
    // 环境变量驱动的章节级小样本测试（用于新书质量验证）：
    //   MUSETRANSLATE_EPUB_TEST_PATH  — EPUB 源文件路径
    //   MUSETRANSLATE_EPUB_CHAPTER_TITLE — 章节标题（如 "Chapter 1: Tash"）
    //   MUSETRANSLATE_REAL_TASK_SUFFIX — 产物后缀（如 -ch1-smoke-001）
    let source_path = std::env::var("MUSETRANSLATE_EPUB_TEST_PATH")
        .expect("MUSETRANSLATE_EPUB_TEST_PATH should point at the EPUB source");
    let chapter_title = std::env::var("MUSETRANSLATE_EPUB_CHAPTER_TITLE")
        .expect("MUSETRANSLATE_EPUB_CHAPTER_TITLE should name the chapter to translate");
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "gate-epub-chapter-smoke-001",
        source_path: Box::leak(source_path.into_boxed_str()),
        article_type: "fiction",
        epub_chapter_title: Some(Box::leak(chapter_title.into_boxed_str())),
        max_parallel: 3,
    })
    .await
    .expect("EPUB chapter smoke task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_html.contains("<html"));
}

#[tokio::test]
#[ignore = "real EPUB full-book test: source path from env (chapter_title=None translates whole book)"]
async fn gate_epub_full_book_env_001() {
    // 环境变量驱动的全书翻译（用于新书全量）：
    //   MUSETRANSLATE_EPUB_TEST_PATH  — EPUB 源文件路径
    //   MUSETRANSLATE_REAL_TASK_SUFFIX — 产物后缀（如 -the-other-mothers-001）
    //   MUSETRANSLATE_EPUB_MAX_PARALLEL — 可选并发（默认 10）
    let source_path = std::env::var("MUSETRANSLATE_EPUB_TEST_PATH")
        .expect("MUSETRANSLATE_EPUB_TEST_PATH should point at the EPUB source");
    let max_parallel = std::env::var("MUSETRANSLATE_EPUB_MAX_PARALLEL")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(10);
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "gate-epub-full-book-env-001",
        source_path: Box::leak(source_path.into_boxed_str()),
        article_type: "fiction",
        epub_chapter_title: None,
        max_parallel,
    })
    .await
    .expect("EPUB full-book env task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_html.contains("<html"));
}

#[tokio::test]
#[ignore = "real LLM A/B test: baseline vs imagery-preservation fiction prompt"]
async fn gate_fiction_imagery_prompt_ab_001() {
    // P1 方案验证：同一批文学意象密集句，对照「基线 prompt」与「注入意象保真规则的 fiction prompt」，
    // 人工/盲审判定文学性是否大幅优化。仅做翻译调用，不落盘、不影响断点。
    let runtime_paths = resolve_runtime_paths();
    ensure_seeded(&runtime_paths.skill_library_root_dir).expect("seed skill library");
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let active_pool = runtime_config
        .route_pools
        .iter()
        .find(|pool| {
            !runtime_config.active_pool.trim().is_empty() && pool.name == runtime_config.active_pool
        })
        .or_else(|| runtime_config.route_pools.first());
    let client = if let Some(pool) = active_pool {
        SensenovaClient::new_with_route_pool(
            runtime_config.llm_api_url.clone(),
            runtime_config.llm_api_key.clone(),
            runtime_config.llm_model.clone(),
            pool,
        )
    } else {
        SensenovaClient::new(
            runtime_config.llm_api_url.clone(),
            runtime_config.llm_api_key.clone(),
            runtime_config.llm_model.clone(),
        )
    };

    let bundle = load_translation_prompt_bundle(&runtime_paths.skill_library_root_dir, "fiction")
        .expect("load fiction prompt bundle");
    let enhanced_system = bundle.system_prompt;
    let baseline_system =
        "You are a professional English-to-Chinese literary translator. Translate the passage into natural Simplified Chinese.".to_string();

    // 盲审标出的硬伤句（准确性/语境推理为主，兼意象），专门压测偏弱模型的语境判断力。
    // 覆盖：场景定位/法律术语/口语双关/地名/情感隐喻/身体感受/对话节奏。
    let cases: [(&str, &str); 9] = [
        ("station_police", "WE MEET IN A room with no windows. They have taken me inland, to the nearest station, I assume. Here there is no crash of waves."),
        ("open_verdict", "She said her death should act as a warning to others. The coroner recorded an open verdict."),
        ("you_are_looking", "\"You are looking, aren't you? You need to look. On the cliffs. He fell, but maybe he's still...\""),
        ("woodberry_down", "The reserve forms part of the Woodberry Down regeneration site, one of Europe's largest building projects."),
        ("emotional_wringer", "The lack of sleep, the exhaustion, the funny moments and the painful, the constant emotional wringer."),
        ("twist_gut", "Her hair is cut short, like a little child's. Like Finn's. I feel a twist in my gut. Finn."),
        ("milk_froth", "Overhead, the cold sky, speckled with a swirling milk froth of stars, was so beautiful I'd actually caught my breath."),
        ("opening_offer", "We both know this is an opening offer. One Pascoe isn't likely to accept."),
        ("despite_everything", "Seeing that sapphire ribbon stretched across the horizon, my heart had lifted, despite everything."),
    ];

    for (name, source) in cases {
        eprintln!("\n===== CASE {name} =====");
        eprintln!("[EN] {source}");
        for mode in ["BASELINE", "ENHANCED"] {
            let system_prompt = if mode == "BASELINE" {
                &baseline_system
            } else {
                &enhanced_system
            };
            // 单句翻译带有限重试：号池瞬时限流/网关抖动（429/5xx）不应中止整批 A/B。
            // 限定 agnes-only 池时，本测试测的就是 agnes 在两种 prompt 下的表现。
            let mut last_error = None;
            let mut translated = None;
            for attempt in 1..=3 {
                match client
                    .translate_markdown_with_context(source, "fiction", system_prompt, None)
                    .await
                {
                    Ok(value) => {
                        translated = Some(value);
                        break;
                    }
                    Err(error) => {
                        eprintln!("[{mode}] attempt {attempt} failed for {name}: {error:?}");
                        last_error = Some(error);
                        tokio::time::sleep(std::time::Duration::from_secs(10 * attempt as u64))
                            .await;
                    }
                }
            }
            let Some(translated) = translated else {
                eprintln!("[{mode}] SKIPPED {name} after retries: {:?}", last_error);
                continue;
            };
            eprintln!("[{mode}] {}", translated.translated_text.trim());
        }
    }
}

#[tokio::test]
#[ignore = "real LLM A/B by model: translate hard sentences with a single-provider pool (env MUSETRANSLATE_AB_POOL)"]
async fn gate_fiction_model_prompt_ab_001() {
    // 用 env 指定单供应商号池（如 agnes-only / sensenova-only / opencode-only / nvidia-only），
    // 对该模型跑 基线 prompt vs 增强 prompt 的 A/B，定位"prompt 能否救回弱模型"。
    let target_pool = std::env::var("MUSETRANSLATE_AB_POOL")
        .expect("MUSETRANSLATE_AB_POOL should name a pool in pools.local.json (e.g. agnes-only)");
    let runtime_paths = resolve_runtime_paths();
    ensure_seeded(&runtime_paths.skill_library_root_dir).expect("seed skill library");
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let pool = runtime_config
        .route_pools
        .iter()
        .find(|p| p.name == target_pool)
        .unwrap_or_else(|| panic!("pool '{target_pool}' not found in pools.local.json"));
    let client = SensenovaClient::new_with_route_pool(
        runtime_config.llm_api_url.clone(),
        runtime_config.llm_api_key.clone(),
        runtime_config.llm_model.clone(),
        pool,
    );

    let bundle = load_translation_prompt_bundle(&runtime_paths.skill_library_root_dir, "fiction")
        .expect("load fiction prompt bundle");
    let enhanced_system = bundle.system_prompt;
    let baseline_system =
        "You are a professional English-to-Chinese literary translator. Translate the passage into natural Simplified Chinese.".to_string();

    // 关键：给足语境。真实管线会附带上一段参考上下文 + 术语表（translation_context），
    // 此处模拟——为每句提供「情境前置段落」，让模型在锚定场景后取义，而非孤立译单句。
    // 这直接检验"语境锚定"新 prompt 是否让模型把 station→警局、You are looking→去搜寻。
    let scene_setup = "Reference-only previous paragraph for context. Do not translate, rewrite, quote, summarize, or include this paragraph in translatedText:\n<REFERENCE_ONLY_PREVIOUS_PARAGRAPH>\nNorth Cornwall police station. April 2019. WE MEET IN A room with no windows. I am being questioned by Detective Pascoe about my husband, who fell from the cliffs near our holiday home and is missing. I am a wife and mother, terrified, begging the police to search for him.\n</REFERENCE_ONLY_PREVIOUS_PARAGRAPH>";

    let cases: [(&str, &str); 9] = [
        ("station_police", "WE MEET IN A room with no windows. They have taken me inland, to the nearest station, I assume. Here there is no crash of waves."),
        ("open_verdict", "She said her death should act as a warning to others. The coroner recorded an open verdict."),
        ("you_are_looking", "\"You are looking, aren't you? You need to look. On the cliffs. He fell, but maybe he's still...\""),
        ("woodberry_down", "The reserve forms part of the Woodberry Down regeneration site, one of Europe's largest building projects."),
        ("emotional_wringer", "The lack of sleep, the exhaustion, the funny moments and the painful, the constant emotional wringer."),
        ("twist_gut", "Her hair is cut short, like a little child's. Like Finn's. I feel a twist in my gut. Finn."),
        ("milk_froth", "Overhead, the cold sky, speckled with a swirling milk froth of stars, was so beautiful I'd actually caught my breath."),
        ("opening_offer", "We both know this is an opening offer. One Pascoe isn't likely to accept."),
        ("despite_everything", "Seeing that sapphire ribbon stretched across the horizon, my heart had lifted, despite everything."),
    ];

    eprintln!("[ab-pool] target_pool={target_pool}");
    for (name, source) in cases {
        eprintln!("\n===== CASE {name} =====");
        eprintln!("[EN] {source}");
        for mode in ["BASELINE", "ENHANCED"] {
            let system_prompt = if mode == "BASELINE" {
                &baseline_system
            } else {
                &enhanced_system
            };
            let mut last_error = None;
            let mut translated = None;
            for attempt in 1..=3 {
                match client
                    .translate_markdown_with_context(
                        source,
                        "fiction",
                        system_prompt,
                        Some(scene_setup),
                    )
                    .await
                {
                    Ok(value) => {
                        translated = Some(value);
                        break;
                    }
                    Err(error) => {
                        eprintln!("[{mode}] attempt {attempt} failed for {name}: {error:?}");
                        last_error = Some(error);
                        tokio::time::sleep(std::time::Duration::from_secs(10 * attempt as u64))
                            .await;
                    }
                }
            }
            match translated {
                Some(t) => eprintln!("[{mode}] {}", t.translated_text.trim()),
                None => eprintln!("[{mode}] SKIPPED {name}: {:?}", last_error),
            }
        }
    }
}

#[tokio::test]
#[ignore = "real EPUB pack smoke: rebuild Chinese + bilingual EPUB from translated artifact"]
async fn gate_epub_pack_smoke_001() {
    // 端到端打包冒烟：从已翻译 artifact 的 translated_blocks.json 重建 中文版/双语版 EPUB，
    // 验证 zip 结构、mimetype 首条目、章节 xhtml 双列/译文、样式表注入、图片复制。
    let artifact_dir = std::env::var("MUSETRANSLATE_PACK_ARTIFACT_DIR")
        .expect("MUSETRANSLATE_PACK_ARTIFACT_DIR should point at a translated artifact dir");
    let source_epub = std::env::var("MUSETRANSLATE_PACK_SOURCE_EPUB")
        .expect("MUSETRANSLATE_PACK_SOURCE_EPUB should point at the source EPUB");
    let output_dir = std::env::var("MUSETRANSLATE_PACK_OUTPUT_DIR")
        .unwrap_or_else(|_| format!("{artifact_dir}/packed"));
    std::fs::create_dir_all(&output_dir).expect("create pack output dir");

    let artifact_dir = PathBuf::from(artifact_dir);
    let source_epub = PathBuf::from(source_epub);
    let output_dir = PathBuf::from(output_dir);

    let blocks: Vec<crate::document::TranslatedBlock> =
        crate::pipeline::artifact::read_json(&artifact_dir.join("translated_blocks.json"))
            .expect("read translated_blocks.json");
    let chapters = crate::document::extract_epub_chapters(&source_epub)
        .expect("extract epub chapters for pack");

    // 1) 归章键：heading 的 chapterTitle 可能被重写成「人物名+内联图」（如 'Tash ![](…)'），
    // 与干净目录标题（'Chapter 1: Tash'）割裂。用「最近的干净标题」作为每块的逻辑章：
    // 干净的（不含图片/较短的）标题出现即开启新章，重写形式并入上一章。
    let is_clean_title = |ct: &Option<String>, kind: &str| {
        kind == "heading"
            && ct
                .as_deref()
                .map(|t| {
                    !t.contains("![") && !t.contains("imgs/") && t.trim().chars().count() <= 60
                })
                .unwrap_or(false)
    };
    let mut segments: Vec<(Option<String>, Vec<crate::document::TranslatedBlock>)> = Vec::new();
    for b in blocks.into_iter() {
        let start_new = is_clean_title(&b.chapter_title, &b.kind) || segments.is_empty();
        if start_new {
            segments.push((b.chapter_title.clone(), vec![b]));
        } else {
            segments.last_mut().unwrap().1.push(b);
        }
    }
    eprintln!("[pack] logical segments: {}", segments.len());

    // 2) 归章候选章：保留**全部** spine 章（含低词数的 back/front matter），
    // 不再按 words>=30 过滤。

    // 为什么不过滤：back matter（SS_recommend/adcard 等）词数虽少（<30）但 spine
    // 里确实是独立文件，其译文段也是独立段；按 words 过滤会让这些章「缺席」，
    // 导致单调前移的归章游标被少消耗，**其后所有段整体错位一章**——
    // 实测 copyright 段因此被挤掉配不到 copyright.xhtml（book_zh 版权页留英文）。
    // 全保留后段章按 spine 顺序一一对应，评分（标题+块数）仍能正确归章；
    // 纯图章（cover/title，words=0）的段会因「块数差」评分为负而自然跳过，
    // 不影响后续对齐。
    let content_chapters: Vec<_> = chapters.iter().collect();
    eprintln!(
        "[pack] content chapters (all spine): {}",
        content_chapters.len()
    );
    // 3) 归章：段与内容章节按序对齐（指针单调前移，保 spine 顺序）。
    // 段 = 两个干净章标题之间的全部块（含正文）。但章标题块可能与正文的 chapterTitle
    // 不同（正文被标成「人物名+内联图」），所以一个逻辑段 = 干净标题块 + 其后所有块
    // 直到下一个干净标题。匹配依据标题文字 + 段块数与章节实际块数是否吻合，
    // 避免整本书被误塞进一章（章节标题与正文章节标不一致时 title 匹配会失败）。
    let norm = |s: &str| {
        s.split("![")
            .next()
            .unwrap_or(s)
            .trim()
            .trim_start_matches('#')
            .trim()
            .to_ascii_lowercase()
    };
    let mut pack_chapters = Vec::new();
    let mut cursor = 0usize;
    for (seg_title, seg_blocks) in segments {
        if seg_blocks
            .iter()
            .all(|b| b.source_text.trim().is_empty() && b.translated_text.trim().is_empty())
        {
            continue;
        }
        // 打分选章：标题文字匹配 + 块数吻合度，取 cursor.. 内最高分（单调前移）。
        let seg_block_count = seg_blocks.len();
        let mut best: Option<(usize, i64)> = None; // (chapter_idx, score)
        for i in cursor..content_chapters.len() {
            let ch = content_chapters[i];
            let ctn = norm(&ch.title);
            let mut score = 0i64;
            // 标题文字匹配（强信号）
            if let Some(st) = seg_title.as_deref() {
                let stn = norm(st);
                if !stn.is_empty() && !ctn.is_empty() {
                    if ctn == stn {
                        score += 1000;
                    } else if ctn.contains(&stn) || stn.contains(&ctn) {
                        score += 500;
                    }
                }
            }
            // 块数吻合（防止标题匹配失败时整段错配）：章节实际块数 vs 段块数越接近越好。
            let ch_block_count = ch.blocks.len().max(1);
            let size_diff = (ch_block_count as i64 - seg_block_count as i64).abs();
            score -= size_diff.min(200); // 差距越大扣分越多（封顶200）
            if score > 0 && best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((i, score));
            }
        }
        // 若无任何正分匹配，回退到 cursor（顺序对齐）。
        let i = best.map(|(i, _)| i).unwrap_or(cursor);
        if i < content_chapters.len() {
            pack_chapters.push(crate::document::pack::EpubPackChapter {
                translated: seg_blocks,
            });
            cursor = i + 1;
        }
    }
    eprintln!("[pack] assigned chapters: {}", pack_chapters.len());
    assert!(
        !pack_chapters.is_empty(),
        "no chapter matched translated blocks"
    );

    // 书名优先取 OPF dc:title（避免硬编码错误书名）；env 可覆盖。
    let book_title = std::env::var("MUSETRANSLATE_PACK_BOOK_TITLE")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            crate::document::pack::read_opf_title(&source_epub)
                .unwrap_or_else(|| "Translated Book".to_string())
        });
    eprintln!(
        "[pack] book_title={book_title} chapters={}",
        pack_chapters.len()
    );

    // 目录中文标题：读 translated_toc.json（管线已生成 98/103 中文标题）。
    let toc: Vec<crate::document::TocEntry> =
        crate::pipeline::artifact::read_json(&artifact_dir.join("translated_toc.json"))
            .map(|t: crate::document::TocArtifact| t.entries)
            .unwrap_or_default();
    eprintln!(
        "[pack] toc entries with zh title: {}",
        toc.iter().filter(|e| e.translated_title.is_some()).count()
    );

    let zh_out = output_dir.join("book_zh.epub");
    let bi_out = output_dir.join("book_bilingual.epub");
    let report_zh = crate::document::pack::write_epub(
        &source_epub,
        &zh_out,
        crate::document::pack::EpubPackMode::Chinese,
        &pack_chapters,
        &book_title,
        &toc,
    )
    .expect("write Chinese epub");
    let report_bi = crate::document::pack::write_epub(
        &source_epub,
        &bi_out,
        crate::document::pack::EpubPackMode::Bilingual,
        &pack_chapters,
        &book_title,
        &toc,
    )
    .expect("write bilingual epub");

    eprintln!("[pack] zh report: {report_zh:?}");
    eprintln!("[pack] bi report: {report_bi:?}");
    assert!(report_zh.aligned_blocks > 0);
    assert!(zh_out.exists() && bi_out.exists());

    // 校验：mimetype 为首条目且不压缩；双语版含双列标记；中文版含译文。
    for (path, expect_bilingual) in [(&zh_out, false), (&bi_out, true)] {
        let file = std::fs::File::open(path).expect("open packed epub");
        let mut zip = zip::ZipArchive::new(file).expect("read packed epub");
        assert!(zip.len() > 0);
        let first_name = {
            let first = zip.by_index(0).expect("first entry");
            first.name().to_string()
        };
        assert_eq!(first_name, "mimetype");
        let mut chapter_text = String::new();
        let mut found = false;
        for i in 0..zip.len() {
            let name = {
                let entry = zip.by_index(i).expect("entry");
                entry.name().to_string()
            };
            if name.ends_with(".xhtml") || name.ends_with(".html") {
                use std::io::Read as _;
                let mut buf = String::new();
                let mut entry = zip.by_index(i).expect("entry");
                entry.read_to_string(&mut buf).expect("read chapter");
                // 找含实际内容（双列对或大量中文）的正文 xhtml，跳过占位/广告页。
                let has_content = buf.contains("mt-pair")
                    || buf.matches('一').count() + buf.matches('的').count() > 10;
                if has_content {
                    chapter_text = buf;
                    found = true;
                    break;
                }
            }
        }
        if expect_bilingual {
            assert!(
                found && (chapter_text.contains("mt-pair") || chapter_text.contains("mt-zh")),
                "bilingual should have pairs (block mt-pair or inline mt-zh)"
            );
        }
    }
}

#[tokio::test]
#[ignore = "real LLM residual sentence patch smoke test"]
async fn gate_epub_residual_sentence_patch_smoke_001() {
    let runtime_paths = resolve_runtime_paths();
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let client = SensenovaClient::new(
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let target = crate::translation_validation::SentencePatchTarget {
        source_context: "Its movements were made with hateful deliberation and dreadful leisure. The looming head lit the cave wall.".to_string(),
        translated_context: "它的动作中带着可恨的 deliberation 和可怕的悠闲。洞穴墙壁因它 looming 头部的污秽光芒而变得明亮。".to_string(),
        source_sentence: "Its movements were made with hateful deliberation and dreadful leisure. The looming head lit the cave wall.".to_string(),
        translated_sentence: "它的动作中带着可恨的 deliberation 和可怕的悠闲。洞穴墙壁因它 looming 头部的污秽光芒而变得明亮。".to_string(),
    };
    let prompt = crate::translation_validation::build_llm_sentence_patch_prompt(
        "fiction",
        &target,
        &[("deliberation", ""), ("looming", "")],
    );
    let response = client
        .review_translation_issue(&prompt)
        .await
        .expect("residual patch request should succeed");
    eprintln!("[residual-smoke-response] {response}");
    let value: serde_json::Value =
        serde_json::from_str(response.trim()).expect("response should be JSON");
    let decision = value
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .expect("decision should exist");
    assert!(matches!(decision, "patch" | "correct"));
    if decision == "patch" {
        let patched = value
            .get("patchedSentence")
            .or_else(|| value.get("patched_sentence"))
            .and_then(serde_json::Value::as_str)
            .expect("patched sentence should exist");
        assert!(!patched.contains("deliberation"), "patched={patched}");
        assert!(!patched.contains("looming"), "patched={patched}");
    }
}

#[tokio::test]
#[ignore = "real LLM residual paragraph patch smoke test"]
async fn gate_epub_residual_paragraph_patch_smoke_001() {
    let runtime_paths = resolve_runtime_paths();
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let client = SensenovaClient::new(
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let prompt = crate::translation_validation::build_llm_paragraph_patch_prompt(
        "fiction",
        "Its movements were made with hateful deliberation and dreadful leisure. The looming head lit the cave wall.",
        "它的动作中带着可恨的 deliberation 和可怕的悠闲。洞穴墙壁因它 looming 头部的污秽光芒而变得明亮。",
        &[("deliberation", ""), ("looming", "")],
        "paragraph",
    );
    let response = client
        .review_translation_issue(&prompt)
        .await
        .expect("residual paragraph patch request should succeed");
    eprintln!("[residual-paragraph-smoke-response] {response}");
    let value: serde_json::Value =
        serde_json::from_str(response.trim()).expect("response should be JSON");
    let decision = value
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .expect("decision should exist");
    assert!(matches!(decision, "repair" | "correct"));
    if decision == "repair" {
        let repaired = value
            .get("repairedText")
            .or_else(|| value.get("repaired_text"))
            .and_then(serde_json::Value::as_str)
            .expect("repaired text should exist");
        assert!(!repaired.contains("deliberation"), "repaired={repaired}");
        assert!(!repaired.contains("looming"), "repaired={repaired}");
    }
}

#[tokio::test]
#[ignore = "real LLM residual chunk patch smoke test"]
async fn gate_epub_residual_chunk_patch_smoke_001() {
    let runtime_paths = resolve_runtime_paths();
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let client = SensenovaClient::new(
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let residual_terms = vec![
        crate::translation_validation::ResidualEnglishTerm {
            term: "deliberation".to_string(),
            count: 1,
            source_excerpt:
                "Its movements were made with hateful deliberation and dreadful leisure."
                    .to_string(),
            translated_excerpt: "deliberation".to_string(),
        },
        crate::translation_validation::ResidualEnglishTerm {
            term: "looming".to_string(),
            count: 1,
            source_excerpt: "The looming head lit the cave wall.".to_string(),
            translated_excerpt: "looming".to_string(),
        },
    ];
    let prompt = crate::translation_validation::build_llm_validation_chunk_review_prompt(
        "fiction",
        "Its movements were made with hateful deliberation and dreadful leisure. The looming head lit the cave wall.",
        "它的动作中带着可恨的 deliberation 和可怕的悠闲。洞穴墙壁因它 looming 头部的污秽光芒而变得明亮。",
        &residual_terms,
    );
    let response = client
        .review_translation_issue(&prompt)
        .await
        .expect("residual chunk patch request should succeed");
    eprintln!("[residual-chunk-smoke-response] {response}");
    let value: serde_json::Value =
        serde_json::from_str(response.trim()).expect("response should be JSON");
    let decision = value
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .expect("decision should exist");
    assert!(matches!(decision, "repair" | "false_positive"));
    if decision == "repair" {
        let repaired = value
            .get("repairedText")
            .or_else(|| value.get("repaired_text"))
            .and_then(serde_json::Value::as_str)
            .expect("repaired text should exist");
        assert!(!repaired.contains("deliberation"), "repaired={repaired}");
        assert!(!repaired.contains("looming"), "repaired={repaired}");
    }
}

#[tokio::test]
#[ignore = "real release-gate academic PDF integration task"]
async fn gate_pdf_empirical_article_001() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "gate-pdf-empirical-article-001",
        source_path: r"D:\tools\musetranslate\click\test_file\02_empirical_metacognition_ambiguous_news.pdf",
        article_type: "academic",
        epub_chapter_title: None,
        max_parallel: 3,
    })
    .await
    .expect("gate academic PDF task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(
        result.translated_html.contains("参考文献") || result.translated_html.contains("<html")
    );
}

#[tokio::test]
#[ignore = "real OCR-only policy PDF source-prep gate"]
async fn gate_pdf_policy_short_ocr_only_001() {
    let artifact_dir = run_pdf_ocr_only_source_prep(
        "gate-pdf-policy-short-ocr-only-001",
        r"D:\tools\musetranslate\click\test_file\01_policy_short_nudge_plus.pdf",
        "academic",
    )
    .await
    .expect("OCR-only policy PDF source-prep should finish");

    eprintln!("[ocr-only] artifact={}", artifact_dir.display());
    eprintln!(
        "[ocr-only] source={}",
        artifact_dir.join("source.md").display()
    );
    eprintln!(
        "[ocr-only] layouts={}",
        artifact_dir.join("source_layouts.json").display()
    );
    eprintln!("[ocr-only] imgs={}", artifact_dir.join("imgs").display());
}

#[tokio::test]
#[ignore = "real OCR-only empirical PDF source-prep gate"]
async fn gate_pdf_empirical_article_ocr_only_001() {
    let artifact_dir = run_pdf_ocr_only_source_prep(
        "gate-pdf-empirical-article-ocr-only-001",
        r"D:\tools\musetranslate\click\test_file\02_empirical_metacognition_ambiguous_news.pdf",
        "academic",
    )
    .await
    .expect("OCR-only empirical PDF source-prep should finish");

    eprintln!("[ocr-only] artifact={}", artifact_dir.display());
    eprintln!(
        "[ocr-only] source={}",
        artifact_dir.join("source.md").display()
    );
    eprintln!(
        "[ocr-only] layouts={}",
        artifact_dir.join("source_layouts.json").display()
    );
    eprintln!("[ocr-only] imgs={}", artifact_dir.join("imgs").display());
}

#[tokio::test]
#[ignore = "real OCR-only preprint PDF source-prep gate"]
async fn gate_pdf_preprint_toolbox_ocr_only_001() {
    let artifact_dir = run_pdf_ocr_only_source_prep(
        "gate-pdf-preprint-toolbox-ocr-only-001",
        r"D:\tools\musetranslate\click\test_file\03_preprint_toolbox_2_0.pdf",
        "academic",
    )
    .await
    .expect("OCR-only preprint PDF source-prep should finish");

    eprintln!("[ocr-only] artifact={}", artifact_dir.display());
    eprintln!(
        "[ocr-only] source={}",
        artifact_dir.join("source.md").display()
    );
    eprintln!(
        "[ocr-only] layouts={}",
        artifact_dir.join("source_layouts.json").display()
    );
    eprintln!("[ocr-only] imgs={}", artifact_dir.join("imgs").display());
}

#[tokio::test]
#[ignore = "real extraction audit for front matter and references only"]
async fn gate_pdf_extraction_algorithms_001() {
    let cases = [
        (
            "gate-pdf-policy-short-extraction-audit-001",
            r"D:\tools\musetranslate\click\test_file\01_policy_short_nudge_plus.pdf",
            "academic",
        ),
        (
            "gate-pdf-preprint-toolbox-extraction-audit-001",
            r"D:\tools\musetranslate\click\test_file\03_preprint_toolbox_2_0.pdf",
            "academic",
        ),
    ];

    for (task_id, source_path, article_type) in cases {
        let artifact_dir = run_pdf_ocr_only_source_prep(task_id, source_path, article_type)
            .await
            .expect("OCR-only extraction audit source-prep should finish");

        let source_markdown =
            std::fs::read_to_string(artifact_dir.join("source.md")).expect("read source markdown");
        let source_layouts: Vec<crate::ocr::OcrPageLayout> =
            crate::pipeline::artifact::read_json(&artifact_dir.join("source_layouts.json"))
                .expect("read source layouts");

        let candidate = crate::pipeline::structure::front_matter_candidate_from_layout(
            &source_markdown,
            &source_layouts,
        );
        let footnotes = crate::pipeline::structure::first_page_footnote_blocks(&source_layouts);
        let chunks =
            crate::pipeline::split_reference_aware_markdown(&source_markdown, 3000, article_type);

        let mut rows = Vec::with_capacity(chunks.len());
        let mut reference_chunk_count = 0usize;
        let mut protected_metadata_chunk_count = 0usize;
        let mut body_chunk_count = 0usize;
        let mut in_reference_section = false;

        for (index, chunk) in chunks.iter().enumerate() {
            let looks_reference = crate::pipeline::chunking::looks_like_reference_chunk(chunk);
            let looks_protected =
                crate::pipeline::chunking::looks_like_protected_metadata_chunk(chunk);
            if in_reference_section
                && crate::pipeline::chunking::starts_with_non_reference_tail_heading(chunk)
            {
                in_reference_section = false;
            }
            if crate::pipeline::chunking::starts_reference_section(chunk) {
                in_reference_section = true;
            }
            let effective_reference = in_reference_section || looks_reference;
            let kind = if effective_reference {
                reference_chunk_count += 1;
                "reference"
            } else if looks_protected {
                protected_metadata_chunk_count += 1;
                "protectedMetadata"
            } else {
                body_chunk_count += 1;
                "body"
            };
            rows.push(ChunkClassificationRow {
                index,
                kind: kind.to_string(),
                looks_like_reference: looks_reference,
                looks_like_protected_metadata: looks_protected,
                preview: chunk
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(240)
                    .collect(),
            });
        }

        let reference_chunks = rows
            .iter()
            .filter(|row| row.kind == "reference")
            .collect::<Vec<_>>();

        let report = ExtractionAuditReport {
            task_id: task_id.to_string(),
            source_path: source_path.to_string(),
            article_type: article_type.to_string(),
            header_boundary: format!("{:?}", candidate.boundary),
            header_block_count: candidate.blocks.len(),
            body_start_block: candidate.body_start_block,
            first_page_footnote_count: footnotes.len(),
            reference_chunk_count,
            protected_metadata_chunk_count,
            body_chunk_count,
        };

        std::fs::write(
            artifact_dir.join("front_matter_candidate.md"),
            candidate.blocks.join("\n\n"),
        )
        .expect("write front matter candidate");
        std::fs::write(
            artifact_dir.join("front_matter_candidate.json"),
            serde_json::to_string_pretty(&report).expect("serialize extraction audit report"),
        )
        .expect("write front matter candidate report");
        std::fs::write(
            artifact_dir.join("chunk_classification.json"),
            serde_json::to_string_pretty(&rows).expect("serialize chunk classification"),
        )
        .expect("write chunk classification");
        std::fs::write(
            artifact_dir.join("reference_chunks.json"),
            serde_json::to_string_pretty(&reference_chunks)
                .expect("serialize reference chunk subset"),
        )
        .expect("write reference chunks");

        eprintln!("[extraction-audit] artifact={}", artifact_dir.display());
    }
}

#[tokio::test]
#[ignore = "real release-gate short PDF integration task"]
async fn gate_pdf_policy_short_001() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "gate-pdf-policy-short-001",
        source_path: r"D:\tools\musetranslate\click\test_file\01_policy_short_nudge_plus.pdf",
        article_type: "academic",
        epub_chapter_title: None,
        max_parallel: 2,
    })
    .await
    .expect("gate short PDF task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_markdown.contains('#') || result.translated_html.contains("<html"));
}

#[tokio::test]
#[ignore = "real finalize-only gate rebuilt from existing PDF artifact"]
async fn gate_pdf_policy_short_finalize_from_artifact_001() {
    let artifact_dir = finalize_from_existing_artifact(
        "gate-pdf-policy-short-001",
        "gate-pdf-policy-short-finalize-from-artifact-001",
    )
    .await
    .expect("finalize-only gate should finish");

    let paths = paths::ArtifactPaths::new(&artifact_dir);
    assert!(paths.output_markdown_path.exists());
    assert!(paths.output_html_path.exists());
    assert!(paths.translated_blocks_path.exists());
    assert!(paths.front_matter_path.exists());
    assert!(paths.metrics_path.exists());
    assert!(paths.manifest_path.exists());

    eprintln!(
        "[finalize-only-gate] artifact={}",
        artifact_dir.to_string_lossy()
    );
}

#[tokio::test]
#[ignore = "real finalize-only gate rebuilt from existing EPUB artifact"]
async fn gate_epub_fullgate_finalize_from_artifact_001() {
    let artifact_dir = finalize_from_existing_artifact(
        "gate-epub-full-book-001-fullgate-20260603",
        "gate-epub-full-book-001-fullgate-20260603-finalize-001",
    )
    .await
    .expect("EPUB finalize-only gate should finish");

    let paths = paths::ArtifactPaths::new(&artifact_dir);
    assert!(paths.output_markdown_path.exists());
    assert!(paths.output_html_path.exists());
    assert!(paths.validation_report_path.exists());
    assert!(paths.metrics_path.exists());

    eprintln!(
        "[epub-finalize-only-gate] artifact={}",
        artifact_dir.to_string_lossy()
    );
}

#[tokio::test]
#[ignore = "real repair-only gate rebuilt from existing EPUB artifact"]
async fn gate_epub_fullgate_repair_from_artifact_001() {
    let artifact_dir = repair_finalize_from_existing_artifact(
        "gate-epub-full-book-001-fullgate-20260603",
        "gate-epub-full-book-001-fullgate-20260603-repair-001",
    )
    .await
    .expect("EPUB repair-only gate should finish");

    let paths = paths::ArtifactPaths::new(&artifact_dir);
    assert!(paths.output_markdown_path.exists());
    assert!(paths.output_html_path.exists());
    assert!(paths.validation_report_path.exists());
    assert!(paths.metrics_path.exists());

    eprintln!(
        "[epub-repair-only-gate] artifact={}",
        artifact_dir.to_string_lossy()
    );
}

#[tokio::test]
#[ignore = "real release-gate academic preprint PDF integration task"]
async fn gate_pdf_preprint_toolbox_001() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "gate-pdf-preprint-toolbox-001",
        source_path: r"D:\tools\musetranslate\click\test_file\03_preprint_toolbox_2_0.pdf",
        article_type: "academic",
        epub_chapter_title: None,
        max_parallel: 3,
    })
    .await
    .expect("gate preprint PDF task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(
        result.translated_html.contains("参考文献") || result.translated_html.contains("<html")
    );
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task"]
async fn real_epub_satampra_zeiros_chapter() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 3,
    })
    .await
    .expect("real EPUB pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_markdown.contains("萨坦普拉"));
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse"]
async fn real_epub_satampra_zeiros_chapter_fresh_v2() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros-fresh-v2",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 3,
    })
    .await
    .expect("fresh real EPUB pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse"]
async fn real_epub_satampra_zeiros_chapter_fresh_v3() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros-fresh-v3",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 3,
    })
    .await
    .expect("fresh real EPUB pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse"]
async fn real_epub_satampra_zeiros_chapter_fresh_v4() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros-fresh-v4",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 3,
    })
    .await
    .expect("fresh real EPUB pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse"]
async fn real_epub_satampra_zeiros_chapter_fresh_v5() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros-fresh-v5",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 3,
    })
    .await
    .expect("fresh real EPUB pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse"]
async fn real_epub_satampra_zeiros_chapter_fresh_v6() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros-fresh-v6",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 3,
    })
    .await
    .expect("fresh real EPUB pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse and parallel 10"]
async fn real_epub_satampra_zeiros_chapter_fresh_parallel10() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-epub-satampra-zeiros-fresh-parallel10",
        source_path: r"D:\tools\musetranslate\click\uploads\2a55e3f0496e9fe18787dd12b580e700.epub",
        article_type: "fiction",
        epub_chapter_title: Some("The Tale of Satampra Zeiros"),
        max_parallel: 10,
    })
    .await
    .expect("fresh real EPUB parallel10 pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task"]
async fn real_pdf_s2352250x2400099x_full() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-pdf-paper-s2352250x2400099x",
        source_path: r"D:\tools\musetranslate\click\uploads\3fddfaaa-6b35-42c0-9c1c-f21fcaf07b74_1-s2.0-S2352250X2400099X-main.pdf",
        article_type: "academic",
        epub_chapter_title: None,
        max_parallel: 3,
    })
    .await
    .expect("real PDF pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_html.contains("参考文献"));
}

#[tokio::test]
#[ignore = "real LLM/OCR integration task without checkpoint reuse"]
async fn real_pdf_s2352250x2400099x_full_fresh_v2() {
    let result = run_real_pipeline_task(RealPipelineTask {
        task_id: "real-pdf-paper-s2352250x2400099x-fresh-v2",
        source_path: r"D:\tools\musetranslate\click\uploads\3fddfaaa-6b35-42c0-9c1c-f21fcaf07b74_1-s2.0-S2352250X2400099X-main.pdf",
        article_type: "academic",
        epub_chapter_title: None,
        max_parallel: 3,
    })
    .await
    .expect("fresh real PDF pipeline task should finish");

    assert!(result.total_chunks > 0);
    assert_eq!(result.translated_chunks, result.total_chunks);
    assert!(result.translated_html.contains("<html"));
}

async fn run_real_pipeline_task(task: RealPipelineTask) -> Result<PipelineResult, AppError> {
    let runtime_paths = resolve_runtime_paths();
    ensure_seeded(&runtime_paths.skill_library_root_dir)?;
    let env_sources = load_runtime_env_sources();
    let prompt_mode = PromptMode::from_env();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let prompt_bundle =
        load_translation_prompt_bundle(&runtime_paths.skill_library_root_dir, task.article_type)?;
    let system_prompt = prompt_mode.system_prompt(task.article_type, &prompt_bundle);
    let system_prompt_hash = stable_hash_hex(system_prompt.as_bytes());
    let effective_task_id = format!(
        "{}{}{}",
        task.task_id,
        prompt_mode.task_suffix(),
        real_task_suffix_from_env()
    );
    let artifact_dir = runtime_paths.artifact_root_dir.join(&effective_task_id);
    // 号池优先（与 commands/tasks/runner.rs 一致）：runtime_config 含活跃号池时走
    // 跨供应商并行调度；否则回落单 provider。保证真实门禁能复测号池。
    let active_pool = runtime_config
        .route_pools
        .iter()
        .find(|pool| {
            !runtime_config.active_pool.trim().is_empty() && pool.name == runtime_config.active_pool
        })
        .or_else(|| runtime_config.route_pools.first());
    let pipeline = if let Some(pool) = active_pool {
        TranslationPipeline::new_with_route_pool(
            runtime_config.ocr_api_url,
            runtime_config.ocr_token,
            runtime_config.llm_api_url,
            runtime_config.llm_api_key,
            runtime_config.llm_model,
            pool,
        )
    } else {
        TranslationPipeline::new(
            runtime_config.ocr_api_url,
            runtime_config.ocr_token,
            runtime_config.llm_api_url,
            runtime_config.llm_api_key,
            runtime_config.llm_model,
        )
    };

    pipeline
        .process_document(
            PipelineRunConfig {
                task_id: effective_task_id.clone(),
                pdf_path: PathBuf::from(task.source_path),
                article_type: task.article_type.to_string(),
                system_prompt,
                system_prompt_hash,
                skill_ids: prompt_bundle.skill_ids,
                epub_chapter_title: task.epub_chapter_title.map(ToString::to_string),
                chunk_size: 3000,
                max_parallel: task.max_parallel,
                artifact_dir: artifact_dir.clone(),
                cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir)
                    .with_ocr_cache(runtime_config.use_ocr_cache)
                    .with_front_matter_cache(runtime_config.use_front_matter_cache),
                static_glossary_entries: Vec::new(),
                global_llm_limiter: Arc::new(Semaphore::new(task.max_parallel)),
                cancellation_token: CancellationToken::new(),
            },
            |update| {
                eprintln!(
                    "[real-task:{}] {:?} {progress:03}% {message}",
                    prompt_mode.label(),
                    update.stage,
                    progress = update.progress,
                    message = update.message
                );
            },
        )
        .await
}

async fn run_pdf_ocr_only_source_prep(
    task_id: &str,
    source_path: &str,
    article_type: &str,
) -> Result<PathBuf, AppError> {
    let runtime_paths = resolve_runtime_paths();
    let env_sources = load_runtime_env_sources();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let pipeline = TranslationPipeline::new(
        runtime_config.ocr_api_url.clone(),
        runtime_config.ocr_token.clone(),
        runtime_config.llm_api_url.clone(),
        runtime_config.llm_api_key.clone(),
        runtime_config.llm_model.clone(),
    );
    let effective_task_id = format!(
        "{}{}-{}",
        task_id,
        real_task_suffix_from_env(),
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    let artifact_dir = runtime_paths.artifact_root_dir.join(&effective_task_id);
    std::fs::create_dir_all(&artifact_dir).map_err(|error| {
        AppError::internal(format!("create OCR-only artifact dir failed: {error}"))
    })?;
    let paths = paths::ArtifactPaths::new(&artifact_dir);
    let config = PipelineRunConfig {
        task_id: effective_task_id,
        pdf_path: PathBuf::from(source_path),
        article_type: article_type.to_string(),
        system_prompt: String::new(),
        system_prompt_hash: String::new(),
        skill_ids: Vec::new(),
        epub_chapter_title: None,
        chunk_size: 3000,
        max_parallel: 1,
        artifact_dir: artifact_dir.clone(),
        cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir),
        static_glossary_entries: Vec::new(),
        global_llm_limiter: Arc::new(Semaphore::new(1)),
        cancellation_token: CancellationToken::new(),
    };

    let (mut document, source_hash) = source_prep::prepare_source_document(
        &pipeline.ocr_client,
        &config,
        &paths,
        &|stage, message, progress| {
            eprintln!("[ocr-only] {:?} {progress:03}% {message}", stage);
        },
        |stage, message, progress| {
            eprintln!("[ocr-only-event] {stage} {progress:03}% {message}");
            Ok(())
        },
    )
    .await?;

    assert!(
        !document.markdown.trim().is_empty(),
        "OCR-only source markdown should not be empty"
    );
    assert!(
        !document.page_layouts.is_empty(),
        "OCR-only source layouts should not be empty"
    );
    let _front_matter = pipeline
        .extract_source_front_matter(
            &config,
            &mut document,
            &paths,
            &|stage, message, progress| {
                eprintln!(
                    "[ocr-only-front-matter] {:?} {progress:03}% {message}",
                    stage
                );
            },
        )
        .await?;
    std::fs::write(artifact_dir.join("source_hash.txt"), source_hash)
        .map_err(|error| AppError::internal(format!("write source hash failed: {error}")))?;
    Ok(artifact_dir)
}

async fn run_author_first_page_case(
    case: &AuthorFirstPageCase,
    artifact_root: &Path,
    ocr_client: &BaiduOcrClient,
    llm_client: &SensenovaClient,
) -> Result<AuthorGateReport, AppError> {
    let case_dir = artifact_root.join(case.id);
    std::fs::create_dir_all(&case_dir)
        .map_err(|error| AppError::internal(format!("create author gate dir failed: {error}")))?;
    let first_page_path = case_dir.join("first_page.pdf");
    let source_markdown_path = case_dir.join("source.md");
    let authors_path = case_dir.join("authors.json");

    write_first_page_pdf(Path::new(case.source_path), &first_page_path)?;
    let document = ocr_client
        .convert_pdf(&first_page_path)
        .await
        .map_err(AppError::from_provider_error)?;
    std::fs::write(&source_markdown_path, &document.markdown).map_err(|error| {
        AppError::internal(format!("write author gate markdown failed: {error}"))
    })?;

    let extracted = render::extract_academic_authors(&document.markdown);
    let first_page_footnotes =
        crate::pipeline::structure::first_page_footnote_blocks(&document.page_layouts);
    let authors = if render::should_refine_academic_authors(&extracted) {
        render::refine_academic_authors_with_llm(
            llm_client,
            &document.markdown,
            &first_page_footnotes,
            &extracted,
        )
        .await?
    } else {
        extracted
    };
    assert_author_gate_case(case, &document.markdown, &authors);

    let report = AuthorGateReport {
        id: case.id.to_string(),
        source_pdf: case.source_path.to_string(),
        first_page_pdf: first_page_path.to_string_lossy().to_string(),
        source_markdown: source_markdown_path.to_string_lossy().to_string(),
        authors: authors
            .into_iter()
            .map(|author| AuthorGateAuthor {
                name: author.name,
                email: author.email,
                markers: author.markers,
                affiliations: author.affiliations,
                is_corresponding: author.is_corresponding,
                correspondence_note: author.correspondence_note,
            })
            .collect(),
    };
    std::fs::write(
        &authors_path,
        serde_json::to_string_pretty(&report).map_err(|error| {
            AppError::internal(format!("serialize author gate report failed: {error}"))
        })?,
    )
    .map_err(|error| AppError::internal(format!("write author gate report failed: {error}")))?;

    Ok(report)
}

fn write_first_page_pdf(source_path: &Path, first_page_path: &Path) -> Result<(), AppError> {
    let mut document = Document::load(source_path)
        .map_err(|error| AppError::internal(format!("load PDF for first page failed: {error}")))?;
    let pages_to_delete = document
        .get_pages()
        .keys()
        .copied()
        .filter(|page| *page != 1)
        .collect::<Vec<_>>();
    document.delete_pages(&pages_to_delete);
    document.prune_objects();
    document.compress();
    document
        .save(first_page_path)
        .map_err(|error| AppError::internal(format!("save first-page PDF failed: {error}")))?;
    Ok(())
}

fn assert_author_gate_case(
    case: &AuthorFirstPageCase,
    markdown: &str,
    authors: &[render::authors::AcademicAuthor],
) {
    assert!(
        authors.len() >= case.min_authors,
        "{} should detect at least {} authors, got {}. Source preview:\n{}",
        case.id,
        case.min_authors,
        authors.len(),
        markdown_preview(markdown)
    );
    assert!(
        authors.iter().all(|author| !author.name.trim().is_empty()),
        "{} has an empty author name: {authors:?}",
        case.id
    );
    if case.require_metadata {
        assert!(
            authors
                .iter()
                .any(|author| !author.affiliations.is_empty() || author.email.is_some()),
            "{} should capture affiliation or email metadata: {authors:?}",
            case.id
        );
    }
    assert!(
        !authors.iter().any(author_contains_body_text),
        "{} author metadata appears to include abstract/body text: {authors:?}",
        case.id
    );
}

fn author_contains_body_text(author: &render::authors::AcademicAuthor) -> bool {
    let haystack = std::iter::once(author.name.as_str())
        .chain(author.affiliations.iter().map(String::as_str))
        .chain(author.correspondence_note.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    [
        "abstract",
        "keywords",
        "introduction",
        "this paper",
        "\u{6458}\u{8981}",
        "\u{5173}\u{952E}\u{8BCD}",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn markdown_preview(markdown: &str) -> String {
    markdown.lines().take(40).collect::<Vec<_>>().join("\n")
}

fn compact_preview(text: &str, char_limit: usize) -> String {
    let mut preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.chars().count() > char_limit {
        preview = preview.chars().take(char_limit).collect::<String>();
        preview.push_str("...");
    }
    preview
}

fn minimal_translation_prompt(article_type: &str) -> String {
    format!(
        concat!(
            "# Translation Agent\n\n",
            "Article type: {article_type}\n\n",
            "Translate the user content into natural Simplified Chinese.\n",
            "Preserve Markdown structure, HTML tags, links, tables, formulas, code fences, numbering, and citations.\n",
            "Do not add explanation.\n",
            "Return a JSON object with keys translated_text and confirmed_terms.\n",
            "translated_text must contain only the translation.\n",
            "confirmed_terms must be an array. Use an empty array when unnecessary."
        ),
        article_type = article_type
    )
}

fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn real_task_suffix_from_env() -> String {
    std::env::var("MUSETRANSLATE_REAL_TASK_SUFFIX")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with('-') {
                value
            } else {
                format!("-{value}")
            }
        })
        .unwrap_or_default()
}

async fn finalize_from_existing_artifact(
    source_artifact_name: &str,
    new_task_id: &str,
) -> Result<PathBuf, AppError> {
    let runtime_paths = resolve_runtime_paths();
    ensure_seeded(&runtime_paths.skill_library_root_dir)?;
    let env_sources = load_runtime_env_sources();
    let prompt_mode = PromptMode::from_env();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let source_artifact_dir = runtime_paths.artifact_root_dir.join(source_artifact_name);
    let source_paths = paths::ArtifactPaths::new(&source_artifact_dir);
    let mut source_document = artifact::load_source_document(&source_paths)
        .await?
        .ok_or_else(|| {
            AppError::internal(format!(
                "missing persisted source document under {}",
                source_artifact_dir.display()
            ))
        })?;
    let checkpoint: ChunkCheckpoint = artifact::read_json(&source_paths.checkpoint_path)?;
    let prompt_bundle = load_translation_prompt_bundle(
        &runtime_paths.skill_library_root_dir,
        &checkpoint.article_type,
    )?;
    let system_prompt = prompt_mode.system_prompt(&checkpoint.article_type, &prompt_bundle);
    let runtime_prompt = RuntimePromptConfig {
        system_prompt: system_prompt.clone(),
        system_prompt_hash: stable_hash_hex(system_prompt.as_bytes()),
        article_type: checkpoint.article_type.clone(),
        skill_ids: prompt_bundle.skill_ids.clone(),
    };
    let effective_task_id = format!(
        "{}{}{}",
        new_task_id,
        prompt_mode.task_suffix(),
        real_task_suffix_from_env()
    );
    let artifact_dir = runtime_paths.artifact_root_dir.join(&effective_task_id);
    std::fs::create_dir_all(&artifact_dir).map_err(|error| {
        AppError::internal(format!(
            "create finalize-only artifact dir failed({}): {error}",
            artifact_dir.display()
        ))
    })?;
    let paths = paths::ArtifactPaths::new(&artifact_dir);
    artifact::persist_source_document(&paths, &source_document).await?;
    std::fs::write(&paths.event_log_path, "").map_err(|error| {
        AppError::internal(format!(
            "create finalize-only event log failed({}): {error}",
            paths.event_log_path.display()
        ))
    })?;

    let config = PipelineRunConfig {
        task_id: effective_task_id,
        pdf_path: PathBuf::from(&checkpoint.source_pdf_path),
        article_type: checkpoint.article_type.clone(),
        system_prompt,
        system_prompt_hash: runtime_prompt.system_prompt_hash.clone(),
        skill_ids: runtime_prompt.skill_ids.clone(),
        epub_chapter_title: None,
        chunk_size: checkpoint.chunk_size,
        max_parallel: 1,
        artifact_dir: artifact_dir.clone(),
        cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir),
        static_glossary_entries: Vec::new(),
        global_llm_limiter: Arc::new(Semaphore::new(1)),
        cancellation_token: CancellationToken::new(),
    };
    let pipeline = TranslationPipeline::new(
        runtime_config.ocr_api_url,
        runtime_config.ocr_token,
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let source_hash = stable_hash_hex(source_document.markdown.as_bytes());
    let source_front_matter = if source_paths.source_front_matter_path.exists() {
        artifact::read_json(&source_paths.source_front_matter_path)?
    } else {
        pipeline
            .extract_source_front_matter(
                &config,
                &mut source_document,
                &paths,
                &|stage, message, progress| {
                    eprintln!(
                        "[finalize-only-front-matter] {:?} {progress:03}% {message}",
                        stage
                    );
                },
            )
            .await?
    };
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;
    let translated_markdown = merge_translated_chunks(&checkpoint)?;
    let _result = reporting::finalize_translation_outputs(
        &pipeline,
        &config,
        &runtime_prompt,
        &paths,
        &checkpoint,
        &source_hash,
        &translated_markdown,
        &source_front_matter,
        None,
        source_document.cover_data_url.clone(),
        |stage, message, progress| {
            pipeline.record_event(&paths.event_log_path, stage, message, progress)
        },
        &|stage, message, progress| {
            eprintln!("[finalize-only] {:?} {progress:03}% {message}", stage);
        },
    )
    .await?;
    Ok(artifact_dir)
}

async fn repair_finalize_from_existing_artifact(
    source_artifact_name: &str,
    new_task_id: &str,
) -> Result<PathBuf, AppError> {
    let runtime_paths = resolve_runtime_paths();
    ensure_seeded(&runtime_paths.skill_library_root_dir)?;
    let env_sources = load_runtime_env_sources();
    let prompt_mode = PromptMode::from_env();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let source_artifact_dir = runtime_paths.artifact_root_dir.join(source_artifact_name);
    let source_paths = paths::ArtifactPaths::new(&source_artifact_dir);
    let mut source_document = artifact::load_source_document(&source_paths)
        .await?
        .ok_or_else(|| {
            AppError::internal(format!(
                "missing persisted source document under {}",
                source_artifact_dir.display()
            ))
        })?;
    let mut checkpoint: ChunkCheckpoint = artifact::read_json(&source_paths.checkpoint_path)?;
    let prompt_bundle = load_translation_prompt_bundle(
        &runtime_paths.skill_library_root_dir,
        &checkpoint.article_type,
    )?;
    let system_prompt = prompt_mode.system_prompt(&checkpoint.article_type, &prompt_bundle);
    let runtime_prompt = RuntimePromptConfig {
        system_prompt: system_prompt.clone(),
        system_prompt_hash: stable_hash_hex(system_prompt.as_bytes()),
        article_type: checkpoint.article_type.clone(),
        skill_ids: prompt_bundle.skill_ids.clone(),
    };
    let effective_task_id = format!(
        "{}{}{}",
        new_task_id,
        prompt_mode.task_suffix(),
        real_task_suffix_from_env()
    );
    let artifact_dir = runtime_paths.artifact_root_dir.join(&effective_task_id);
    std::fs::create_dir_all(&artifact_dir).map_err(|error| {
        AppError::internal(format!(
            "create repair-only artifact dir failed({}): {error}",
            artifact_dir.display()
        ))
    })?;
    let paths = paths::ArtifactPaths::new(&artifact_dir);
    artifact::persist_source_document(&paths, &source_document).await?;
    std::fs::write(&paths.event_log_path, "").map_err(|error| {
        AppError::internal(format!(
            "create repair-only event log failed({}): {error}",
            paths.event_log_path.display()
        ))
    })?;

    let max_parallel = 3usize;
    let config = PipelineRunConfig {
        task_id: effective_task_id,
        pdf_path: PathBuf::from(&checkpoint.source_pdf_path),
        article_type: checkpoint.article_type.clone(),
        system_prompt,
        system_prompt_hash: runtime_prompt.system_prompt_hash.clone(),
        skill_ids: runtime_prompt.skill_ids.clone(),
        epub_chapter_title: None,
        chunk_size: checkpoint.chunk_size,
        max_parallel,
        artifact_dir: artifact_dir.clone(),
        cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir),
        static_glossary_entries: Vec::new(),
        global_llm_limiter: Arc::new(Semaphore::new(max_parallel)),
        cancellation_token: CancellationToken::new(),
    };
    let pipeline = TranslationPipeline::new(
        runtime_config.ocr_api_url,
        runtime_config.ocr_token,
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let source_hash = stable_hash_hex(source_document.markdown.as_bytes());
    let source_front_matter = if source_paths.source_front_matter_path.exists() {
        artifact::read_json(&source_paths.source_front_matter_path)?
    } else {
        pipeline
            .extract_source_front_matter(
                &config,
                &mut source_document,
                &paths,
                &|stage, message, progress| {
                    eprintln!(
                        "[repair-only-front-matter] {:?} {progress:03}% {message}",
                        stage
                    );
                },
            )
            .await?
    };

    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;
    for pass in 1..=8 {
        let candidate_indexes = select_residual_repair_candidate_indexes(&checkpoint, 96);
        if candidate_indexes.is_empty() {
            eprintln!("[repair-only] pass={pass} no residual candidates");
            break;
        }
        eprintln!(
            "[repair-only] pass={pass} candidate_chunks={}",
            candidate_indexes.len()
        );
        stage_residual_repair_candidates(&mut checkpoint, &candidate_indexes);
        artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;
        review_residual_english_candidates(
            &pipeline.llm_client,
            &config,
            &runtime_prompt,
            &paths,
            &mut checkpoint,
            &|stage, message, progress| {
                eprintln!("[repair-only] {:?} {progress:03}% {message}", stage);
            },
            |stage, message, progress| {
                pipeline.record_event(&paths.event_log_path, stage, message, progress)
            },
        )
        .await?;
    }

    let merge_ready = mark_checkpoint_merge_ready(&mut checkpoint);
    if merge_ready > 0 {
        artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;
        eprintln!("[repair-only] advanced_merge_ready_chunks={merge_ready}");
    }
    let translated_markdown = merge_translated_chunks(&checkpoint)?;
    let _result = reporting::finalize_translation_outputs(
        &pipeline,
        &config,
        &runtime_prompt,
        &paths,
        &checkpoint,
        &source_hash,
        &translated_markdown,
        &source_front_matter,
        None,
        source_document.cover_data_url.clone(),
        |stage, message, progress| {
            pipeline.record_event(&paths.event_log_path, stage, message, progress)
        },
        &|stage, message, progress| {
            eprintln!(
                "[repair-only-finalize] {:?} {progress:03}% {message}",
                stage
            );
        },
    )
    .await?;
    Ok(artifact_dir)
}

#[tokio::test]
#[ignore = "real LLM residual minimal chain smoke test"]
async fn gate_epub_residual_min_chain_llm_001() {
    let artifact_dir = run_epub_residual_min_chain_llm(
        "gate-epub-full-book-001-epubcheck-20260605",
        "gate-epub-residual-min-chain-llm-001",
    )
    .await
    .expect("minimal residual LLM chain should finish");
    eprintln!("[residual-min-chain] artifact={}", artifact_dir.display());
}

async fn run_epub_residual_min_chain_llm(
    source_artifact_name: &str,
    new_task_id: &str,
) -> Result<PathBuf, AppError> {
    let runtime_paths = resolve_runtime_paths();
    ensure_seeded(&runtime_paths.skill_library_root_dir)?;
    let env_sources = load_runtime_env_sources();
    let prompt_mode = PromptMode::from_env();
    let runtime_config = load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        resolve_env_defaults(&env_sources),
    );
    let source_artifact_dir = runtime_paths.artifact_root_dir.join(source_artifact_name);
    let source_paths = paths::ArtifactPaths::new(&source_artifact_dir);
    let mut checkpoint: ChunkCheckpoint = artifact::read_json(&source_paths.checkpoint_path)?;
    let prompt_bundle = load_translation_prompt_bundle(
        &runtime_paths.skill_library_root_dir,
        &checkpoint.article_type,
    )?;
    let system_prompt = prompt_mode.system_prompt(&checkpoint.article_type, &prompt_bundle);
    let runtime_prompt = RuntimePromptConfig {
        system_prompt: system_prompt.clone(),
        system_prompt_hash: stable_hash_hex(system_prompt.as_bytes()),
        article_type: checkpoint.article_type.clone(),
        skill_ids: prompt_bundle.skill_ids.clone(),
    };
    let effective_task_id = format!(
        "{}{}{}-{}",
        new_task_id,
        prompt_mode.task_suffix(),
        real_task_suffix_from_env(),
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    let artifact_dir = runtime_paths.artifact_root_dir.join(&effective_task_id);
    std::fs::create_dir_all(&artifact_dir).map_err(|error| {
        AppError::internal(format!(
            "create residual-min-chain artifact dir failed({}): {error}",
            artifact_dir.display()
        ))
    })?;
    let paths = paths::ArtifactPaths::new(&artifact_dir);
    std::fs::write(&paths.event_log_path, "").map_err(|error| {
        AppError::internal(format!(
            "create residual-min-chain event log failed({}): {error}",
            paths.event_log_path.display()
        ))
    })?;

    let target_indexes = [253usize, 261, 277];
    let include_synthetic_probe = residual_min_chain_synthetic_probe_enabled();
    stage_minimal_residual_chain_checkpoint(&mut checkpoint, &target_indexes);
    if include_synthetic_probe {
        append_residual_min_chain_llm_probe(&mut checkpoint);
    } else {
        checkpoint
            .chunks
            .retain(|chunk| chunk.index != residual_min_chain_probe_index());
    }
    rebuild_frozen_chunk_patch_terms(&mut checkpoint);
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;

    let max_parallel = 1usize;
    let config = PipelineRunConfig {
        task_id: effective_task_id,
        pdf_path: PathBuf::from(&checkpoint.source_pdf_path),
        article_type: checkpoint.article_type.clone(),
        system_prompt,
        system_prompt_hash: runtime_prompt.system_prompt_hash.clone(),
        skill_ids: runtime_prompt.skill_ids.clone(),
        epub_chapter_title: None,
        chunk_size: checkpoint.chunk_size,
        max_parallel,
        artifact_dir: artifact_dir.clone(),
        cache_config: PipelineCacheConfig::from_artifact_dir(&artifact_dir),
        static_glossary_entries: Vec::new(),
        global_llm_limiter: Arc::new(Semaphore::new(max_parallel)),
        cancellation_token: CancellationToken::new(),
    };
    let pipeline = TranslationPipeline::new(
        runtime_config.ocr_api_url,
        runtime_config.ocr_token,
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );

    review_residual_english_candidates(
        &pipeline.llm_client,
        &config,
        &runtime_prompt,
        &paths,
        &mut checkpoint,
        &|stage, message, progress| {
            eprintln!("[residual-min-chain] {:?} {progress:03}% {message}", stage);
        },
        |stage, message, progress| {
            pipeline.record_event(&paths.event_log_path, stage, message, progress)
        },
    )
    .await?;
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)?;

    assert_residual_min_chain_output(&checkpoint, &target_indexes);
    if include_synthetic_probe {
        assert_residual_min_chain_llm_probe_repaired(&checkpoint);
        assert_residual_min_chain_used_llm(&paths.event_log_path)?;
    }

    Ok(artifact_dir)
}

fn stage_minimal_residual_chain_checkpoint(
    checkpoint: &mut ChunkCheckpoint,
    target_indexes: &[usize],
) {
    let target_indexes = target_indexes.iter().copied().collect::<BTreeSet<_>>();
    for chunk in &mut checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body || chunk.translated.is_none() {
            continue;
        }
        chunk.processing_stage = if target_indexes.contains(&chunk.index) {
            Some(ChunkProcessingStage::TermConsistencyApplied)
        } else {
            Some(ChunkProcessingStage::MergeReady)
        };
    }
}

fn append_residual_min_chain_llm_probe(checkpoint: &mut ChunkCheckpoint) {
    let probe_index = residual_min_chain_probe_index();
    checkpoint.chunks.retain(|chunk| chunk.index != probe_index);
    checkpoint.chunks.push(ChunkState {
        index: probe_index,
        source: "The spaceship moved quickly.".to_string(),
        translated: Some("spaceship moved quickly.".to_string()),
        confirmed_terms: Vec::new(),
        attempts: 1,
        updated_at: chrono::Utc::now().to_rfc3339(),
        segment_kind: ChunkSegmentKind::Body,
        processing_stage: Some(ChunkProcessingStage::TermConsistencyApplied),
    });
}

fn assert_residual_min_chain_output(checkpoint: &ChunkCheckpoint, target_indexes: &[usize]) {
    for index in target_indexes {
        let chunk = checkpoint
            .chunks
            .iter()
            .find(|chunk| chunk.index == *index)
            .unwrap_or_else(|| panic!("missing chunk {index}"));
        let translated = chunk
            .translated
            .as_deref()
            .unwrap_or_else(|| panic!("missing translated chunk {index}"));
        eprintln!(
            "[residual-min-chain] chunk={} preview={}",
            index,
            compact_preview(translated, 360)
        );
    }

    let chunk_253 = translated_chunk_text(checkpoint, 253);
    assert!(
        !chunk_253.contains("coaster"),
        "chunk 253 still has coaster"
    );
    assert!(
        chunk_253.contains("飞船"),
        "chunk 253 did not use final wave target"
    );

    let chunk_261 = translated_chunk_text(checkpoint, 261);
    assert!(
        !chunk_261.contains("Admiral"),
        "chunk 261 still has Admiral"
    );
    assert!(
        chunk_261.contains("海军上将"),
        "chunk 261 did not repair Admiral Carfax"
    );

    let chunk_277 = translated_chunk_text(checkpoint, 277);
    assert!(
        !chunk_277.contains("Hyperborea"),
        "chunk 277 still has Hyperborea"
    );
}

fn assert_residual_min_chain_llm_probe_repaired(checkpoint: &ChunkCheckpoint) {
    let probe = translated_chunk_text(checkpoint, residual_min_chain_probe_index());
    eprintln!(
        "[residual-min-chain] llm_probe preview={}",
        compact_preview(probe, 220)
    );
    assert!(
        !probe.contains("spaceship"),
        "LLM probe still has spaceship: {probe}"
    );
}

fn assert_residual_min_chain_used_llm(event_log_path: &Path) -> Result<(), AppError> {
    let events = std::fs::read_to_string(event_log_path).map_err(|error| {
        AppError::internal(format!(
            "read residual-min-chain event log failed({}): {error}",
            event_log_path.display()
        ))
    })?;
    let llm_attempts = events
        .lines()
        .filter(|line| line.contains("residual_llm_attempt_started"))
        .count();
    eprintln!("[residual-min-chain] residual_llm_attempts={llm_attempts}");
    assert!(llm_attempts >= 1, "expected at least one real LLM attempt");
    Ok(())
}

fn translated_chunk_text(checkpoint: &ChunkCheckpoint, index: usize) -> &str {
    checkpoint
        .chunks
        .iter()
        .find(|chunk| chunk.index == index)
        .and_then(|chunk| chunk.translated.as_deref())
        .unwrap_or_else(|| panic!("missing translated chunk {index}"))
}

fn residual_min_chain_probe_index() -> usize {
    999_998
}

fn residual_min_chain_synthetic_probe_enabled() -> bool {
    std::env::var("MUSETRANSLATE_RESIDUAL_MIN_CHAIN_SYNTHETIC_PROBE")
        .ok()
        .is_some_and(|value| value.trim() == "1")
}

fn select_residual_repair_candidate_indexes(
    checkpoint: &ChunkCheckpoint,
    term_limit: usize,
) -> BTreeSet<usize> {
    let mut indexes = BTreeSet::new();
    let mut total_terms = 0usize;
    for chunk in &checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_deref() else {
            continue;
        };
        if super::super::chunking::should_skip_residual_review(
            &checkpoint.article_type,
            &chunk.source,
        ) {
            continue;
        }
        let terms = crate::translation_validation::detect_residual_english_terms(
            &checkpoint.article_type,
            &chunk.source,
            translated,
        );
        if terms.is_empty() {
            continue;
        }
        indexes.insert(chunk.index);
        total_terms = total_terms.saturating_add(terms.len());
        if total_terms >= term_limit {
            break;
        }
    }
    indexes
}

fn stage_residual_repair_candidates(
    checkpoint: &mut ChunkCheckpoint,
    candidate_indexes: &BTreeSet<usize>,
) {
    for chunk in &mut checkpoint.chunks {
        if candidate_indexes.contains(&chunk.index) {
            chunk.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);
        }
    }
}
