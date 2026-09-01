use super::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[path = "pipeline_tests/real_pipeline.rs"]
mod real_pipeline;

#[test]
fn retry_delay_uses_fixed_five_second_steps() {
    let error = AppError::new("LLM_NETWORK_ERROR", "network failure", true);

    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 1),
        Duration::from_secs(5)
    );
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 2),
        Duration::from_secs(10)
    );
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 3),
        Duration::from_secs(15)
    );
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 4),
        Duration::from_secs(20)
    );
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 5),
        Duration::from_secs(25)
    );
}

#[test]
fn retry_delay_waits_at_least_one_minute_for_rate_limits() {
    let mut error = AppError::new("LLM_RATE_LIMITED", "rate limited", true);

    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 1),
        Duration::from_secs(60)
    );

    error.retry_after_ms = Some(90_000);
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 1),
        Duration::from_secs(90)
    );
}

fn sample_run_config(root: &std::path::Path) -> PipelineRunConfig {
    PipelineRunConfig {
        task_id: "task-1".to_string(),
        pdf_path: root.join("sample.epub"),
        article_type: "fiction".to_string(),
        system_prompt: "system prompt".to_string(),
        system_prompt_hash: "prompt-hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        epub_chapter_title: None,
        chunk_size: 120,
        max_parallel: 1,
        artifact_dir: root.to_path_buf(),
        cache_config: PipelineCacheConfig::from_artifact_dir(root),
        static_glossary_entries: Vec::new(),
        global_llm_limiter: Arc::new(Semaphore::new(1)),
        cancellation_token: CancellationToken::new(),
    }
}

fn sample_prompt_config() -> RuntimePromptConfig {
    RuntimePromptConfig {
        system_prompt: "system prompt".to_string(),
        system_prompt_hash: "prompt-hash".to_string(),
        article_type: "fiction".to_string(),
        skill_ids: vec!["translation:core".to_string()],
    }
}

#[tokio::test]
#[ignore = "real LLM front-matter extraction smoke test"]
async fn real_pdf_front_matter_llm_extracts_empirical_header() {
    let runtime_paths = crate::commands::resolve_runtime_paths();
    let env_sources = crate::commands::load_runtime_env_sources();
    let runtime_config = crate::commands::load_runtime_config(
        &runtime_paths.runtime_config_store_path,
        crate::commands::resolve_env_defaults(&env_sources),
    );
    let llm_client = SensenovaClient::new(
        runtime_config.llm_api_url,
        runtime_config.llm_api_key,
        runtime_config.llm_model,
    );
    let source_path = runtime_paths
        .artifact_root_dir
        .join("gate-pdf-empirical-article-001-latest-20260530-singlepdf-001/source.md");
    let markdown = std::fs::read_to_string(&source_path).unwrap_or_else(|error| {
        panic!(
            "read empirical source markdown failed({}): {error}",
            source_path.display()
        )
    });
    let page_layouts = vec![crate::ocr::OcrPageLayout {
        blocks: vec![crate::ocr::OcrLayoutBlock {
            label: "footnote".to_string(),
            content: "1 Neuroeconomics lab, Institut des Sciences Cognitives Marc Jeannerod (ISCMJ), CNRS UMR 5229 and Universite Claude Bernard Lyon 1, Bron, France. 2 CNRS, Universite Lumiere Lyon 2, Universite Jean-Monnet Saint-Etienne, emlyon business school, GATE, Lyon, France. 3 IZA, Bonn, Germany. 4 These authors contributed equally: Marie Claire Villeval, Jean-Claude Dreher. e-mail: dreher@isc.cnrs.fr".to_string(),
            bbox: vec![78, 1413, 1122, 1488],
            order: Some(10),
            page: Some(0),
        }],
    }];
    let candidate = structure::front_matter_candidate_from_layout(&markdown, &page_layouts);
    let extracted = structure::extract_front_matter_with_llm(&llm_client, &candidate)
        .await
        .expect("front matter extraction should return valid JSON");

    assert!(
        extracted
            .title
            .contains("Metacognition biases information seeking"),
        "unexpected title: {}",
        extracted.title
    );
    assert!(
        extracted.authors.len() >= 3,
        "expected at least 3 authors: {extracted:?}"
    );
    assert!(
        extracted
            .authors
            .iter()
            .any(|author| author.name.contains("Valentin Guigon")),
        "missing Valentin Guigon: {extracted:?}"
    );
    assert!(
        extracted.extras.iter().any(|item| item.contains("doi.org")),
        "missing DOI in extras: {extracted:?}"
    );
    assert!(
        extracted
            .authors
            .iter()
            .flat_map(|author| author.affiliations.iter())
            .any(|affiliation| affiliation.contains("CNRS")
                || affiliation.contains("Universite")
                || affiliation.contains("IZA")),
        "missing author affiliations from footnote: {extracted:?}"
    );
    assert!(
        extracted.authors.iter().any(|author| {
            author
                .email
                .as_deref()
                .is_some_and(|email| email.contains("dreher@isc.cnrs.fr"))
        }),
        "missing corresponding author email from footnote: {extracted:?}"
    );

    let output_dir = runtime_paths
        .artifact_root_dir
        .join("front-matter-llm-empirical-001");
    std::fs::create_dir_all(&output_dir).expect("front matter output dir");
    std::fs::write(
        output_dir.join("header_candidate.md"),
        candidate.blocks.join("\n\n"),
    )
    .expect("write header candidate");
    std::fs::write(
        output_dir.join("front_matter.json"),
        serde_json::to_string_pretty(&extracted).expect("front matter json"),
    )
    .expect("write front matter json");
}

#[test]
fn rebuild_gate_pdf_policy_short_report_artifacts_from_checkpoint() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifact_dir = manifest_dir.join(".runtime/artifacts/gate-pdf-policy-short-001");
    let checkpoint_path = artifact_dir.join("checkpoint.json");
    let source_markdown_path = artifact_dir.join("source.md");
    let source_toc_path = artifact_dir.join("source_toc.json");
    let translated_toc_path = artifact_dir.join("translated_toc.json");
    let translated_markdown_path = artifact_dir.join("translated.md");
    let translated_html_path = artifact_dir.join("translated.html");
    if !checkpoint_path.exists() {
        eprintln!(
            "skip gate artifact rebuild: missing {}",
            checkpoint_path.display()
        );
        return;
    }

    let checkpoint: ChunkCheckpoint =
        artifact::read_json(&checkpoint_path).expect("checkpoint should load");
    let source_markdown =
        std::fs::read_to_string(&source_markdown_path).expect("source markdown should load");
    let source_toc: crate::document::TocArtifact =
        artifact::read_json(&source_toc_path).expect("source toc should load");

    let translated_markdown =
        merge_translated_chunks(&checkpoint).expect("translated chunks should merge");
    let translated_toc = artifact::build_translated_toc_artifact(
        &source_toc,
        &source_markdown,
        &translated_markdown,
        &[],
        &[],
    );
    let final_markdown = translated_markdown.clone();
    let academic_authors = render::extract_academic_authors(&final_markdown);
    let front_matter = render::authors::AcademicFrontMatter {
        title: None,
        authors: academic_authors,
        extras: Vec::new(),
        affiliation_catalog: Vec::new(),
        extra_entries: Vec::new(),
    };
    let translated_html =
        render::render_translated_html_with_authors(&final_markdown, &front_matter);

    std::fs::write(
        &translated_toc_path,
        serde_json::to_string_pretty(&translated_toc).expect("translated toc json"),
    )
    .expect("translated toc should write");
    std::fs::write(&translated_markdown_path, final_markdown).expect("markdown should write");
    std::fs::write(&translated_html_path, translated_html).expect("html should write");
}

#[test]
fn normalize_rewritten_pdf_source_preserves_existing_pdf_tables_after_front_matter_rewrite() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-pipeline-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let artifact_dir = root.join("artifact");
    std::fs::create_dir_all(&artifact_dir).expect("artifact dir");
    let paths = crate::pipeline::paths::ArtifactPaths::new(&artifact_dir);
    let source_markdown = concat!(
        "Original title\n\n",
        "Jane Doe1, John Roe2\n\n",
        "1 Department A\n\n",
        "2 Department B\n\n",
        "## Abstract\n\n",
        "Body intro.\n\n",
        "<!-- musetranslate:pdf-table:start -->\n",
        "![PDF table 1](imgs/table_snapshot_001.jpg)\n",
        "<!-- musetranslate:pdf-table:end -->\n\n",
        "Between tables.\n\n",
        "<!-- musetranslate:pdf-table:start -->\n",
        "![PDF table 2](imgs/table_snapshot_002.jpg)\n",
        "<!-- musetranslate:pdf-table:end -->\n\n",
        "## Discussion\n\n",
        "Closing paragraph.\n"
    );
    let front_matter = render::authors::AcademicFrontMatter {
        title: Some("Normalized Title".to_string()),
        authors: vec![
            render::authors::AcademicAuthor {
                name: "Jane Doe".to_string(),
                email: None,
                markers: vec!["1".to_string()],
                affiliations: vec!["Department A".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            },
            render::authors::AcademicAuthor {
                name: "John Roe".to_string(),
                email: None,
                markers: vec!["2".to_string()],
                affiliations: vec!["Department B".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            },
        ],
        extras: Vec::new(),
        affiliation_catalog: Vec::new(),
        extra_entries: Vec::new(),
    };
    let rewritten =
        structure::rewrite_source_markdown_with_front_matter(source_markdown, &front_matter);
    let source_document = crate::ocr::OcrDocument {
        markdown: source_markdown.to_string(),
        markdown_images: Default::default(),
        page_layouts: Vec::new(),
        source_blocks: crate::document::build_source_blocks_from_markdown(source_markdown),
        epub_blocks: Vec::new(),
        toc: None,
        cover_data_url: None,
    };

    let normalized = normalize_rewritten_pdf_source(&rewritten, &source_document, &paths);

    assert!(normalized.starts_with(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START));
    assert_eq!(
        normalized.matches(layout::PDF_TABLE_START_MARKER).count(),
        2,
        "{normalized}"
    );
    let first_table = normalized
        .find("![PDF table 1](imgs/table_snapshot_001.jpg)")
        .expect("table 1");
    let second_table = normalized
        .find("![PDF table 2](imgs/table_snapshot_002.jpg)")
        .expect("table 2");
    let discussion = normalized.find("## Discussion").expect("discussion");
    assert!(first_table < second_table, "{normalized}");
    assert!(second_table < discussion, "{normalized}");
    assert!(!normalized.contains("![PDF table 3]"), "{normalized}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rebuild_gate_pdf_policy_short_block_sidecars_and_html() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifact_dir = manifest_dir.join(".runtime/artifacts/gate-pdf-policy-short-001");
    rebuild_block_sidecars_for_artifact(&artifact_dir);
}

#[test]
fn rebuild_gate_epub_full_book_block_sidecars_and_html() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifact_dir = manifest_dir.join(
        ".runtime/artifacts/gate-epub-full-book-001-epub-speed-adaptive-rpm-par5-20260531-001",
    );
    rebuild_block_sidecars_for_artifact(&artifact_dir);
}

#[test]
fn rewritten_front_matter_requires_source_blocks_rebuild_for_block_parity() {
    let original_markdown = concat!(
        "https://doi.org/10.1/example\n\n",
        "# Raw Title\n\n",
        "Jane Doe $ ^{1} $, John Roe $ ^{2,*} $\n\n",
        "Check for updates\n\n",
        "Abstract body.\n\n",
        "## From nudge to nudge plus\n\n",
        "Body section one.\n"
    );
    let front_matter = render::authors::AcademicFrontMatter {
        title: Some("Normalized Title".to_string()),
        authors: vec![
            render::authors::AcademicAuthor {
                name: "Jane Doe".to_string(),
                email: None,
                markers: vec!["1".to_string()],
                affiliations: vec!["Department A".to_string()],
                is_corresponding: false,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: None,
            },
            render::authors::AcademicAuthor {
                name: "John Roe".to_string(),
                email: Some("john@example.test".to_string()),
                markers: vec!["2".to_string(), "*".to_string()],
                affiliations: vec!["Department B".to_string()],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some("Corresponding author".to_string()),
            },
        ],
        extras: vec!["https://doi.org/10.1/example".to_string()],
        affiliation_catalog: Vec::new(),
        extra_entries: Vec::new(),
    };
    let rewritten =
        structure::rewrite_source_markdown_with_front_matter(original_markdown, &front_matter);

    let original_blocks = crate::document::build_source_blocks_from_markdown(original_markdown);
    let rebuilt_blocks = crate::document::build_source_blocks_from_markdown(&rewritten);

    assert_ne!(
        original_blocks.len(),
        rebuilt_blocks.len(),
        "front matter rewrite should materially change block layout in this regression fixture"
    );
    assert!(rebuilt_blocks
        .iter()
        .any(|block| block.markdown.trim() == "# Normalized Title"));
    assert!(rebuilt_blocks
        .iter()
        .all(|block| !block.markdown.contains("Check for updates")));
    let rebuilt_markdown = rebuilt_blocks
        .iter()
        .map(|block| block.markdown.trim())
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let normalized_rewritten = rewritten
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    assert_eq!(rebuilt_markdown, normalized_rewritten);
}

fn rebuild_block_sidecars_for_artifact(artifact_dir: &std::path::Path) {
    let source_markdown_path = artifact_dir.join("source.md");
    let translated_markdown_path = artifact_dir.join("translated.md");
    let translated_html_path = artifact_dir.join("translated.html");
    let checkpoint_path = artifact_dir.join("checkpoint.json");
    let source_blocks_path = artifact_dir.join("source_blocks.json");
    let translated_chunks_path = artifact_dir.join("translated_chunks.json");
    let translated_blocks_path = artifact_dir.join("translated_blocks.json");
    let front_matter_path = artifact_dir.join("front_matter.json");

    if !source_markdown_path.exists() || !translated_markdown_path.exists() {
        eprintln!(
            "skip block sidecar rebuild: missing {} or {}",
            source_markdown_path.display(),
            translated_markdown_path.display()
        );
        return;
    }

    let checkpoint = if checkpoint_path.exists() {
        let checkpoint: ChunkCheckpoint =
            artifact::read_json(&checkpoint_path).expect("checkpoint should load");
        let translated_chunks = artifact::build_translated_chunk_artifact(&checkpoint);
        std::fs::write(
            &translated_chunks_path,
            serde_json::to_string_pretty(&translated_chunks).expect("translated chunks json"),
        )
        .expect("translated chunks should write");
        Some(checkpoint)
    } else {
        None
    };

    let source_markdown =
        std::fs::read_to_string(&source_markdown_path).expect("source markdown should load");
    let translated_markdown = std::fs::read_to_string(&translated_markdown_path)
        .expect("translated markdown should load");
    let source_blocks = crate::document::build_source_blocks_from_markdown(&source_markdown);
    let source_block_chunk_indexes = checkpoint
        .as_ref()
        .map(|checkpoint| {
            reporting::map_source_blocks_to_chunk_indexes(
                &source_markdown,
                &source_blocks,
                checkpoint,
            )
        })
        .unwrap_or_default();
    let translated_blocks = crate::document::build_translated_blocks_with_chunk_indexes(
        &source_blocks,
        &translated_markdown,
        &source_block_chunk_indexes,
    );
    std::fs::write(
        &source_blocks_path,
        serde_json::to_string_pretty(&source_blocks).expect("source blocks json"),
    )
    .expect("source blocks should write");
    std::fs::write(
        &translated_blocks_path,
        serde_json::to_string_pretty(&translated_blocks).expect("translated blocks json"),
    )
    .expect("translated blocks should write");

    let front_matter = std::fs::read_to_string(&front_matter_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<render::authors::AcademicFrontMatter>(&raw).ok())
        .unwrap_or_default();
    let translated_html = render::render_translated_html_from_blocks_with_authors(
        &translated_markdown,
        &translated_blocks,
        &front_matter,
    )
    .unwrap_or_else(|| {
        render::render_translated_html_with_authors(&translated_markdown, &front_matter)
    });
    std::fs::write(&translated_html_path, translated_html).expect("html should write");
}

#[test]
fn rebuild_block_sidecars_from_env_artifact_dir() {
    let Ok(artifact_dir) = std::env::var("MUSETRANSLATE_REBUILD_BLOCK_SIDECAR_DIR") else {
        return;
    };
    rebuild_block_sidecars_for_artifact(std::path::Path::new(&artifact_dir));
}

#[test]
fn pipeline_event_log_stage_sequence_is_ordered_and_monotonic() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-stage-events-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let event_log_path = root.join("events.ndjson");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let sequence = [
        ("queued", "task execution started", 5),
        ("prompt_ready", "runtime prompt ready", 28),
        ("checkpoint_created", "initialized checkpoint", 32),
        ("translating", "starting translation", 40),
        ("term_consistency", "consistency applied", 91),
        ("finalized", "artifacts written", 100),
    ];

    for (stage, message, progress) in sequence {
        pipeline
            .record_event(&event_log_path, stage, message, progress)
            .expect("event should append");
    }

    let raw = std::fs::read_to_string(&event_log_path).expect("event log should exist");
    let events = raw
        .lines()
        .map(|line| serde_json::from_str::<TaskEvent>(line).expect("event line should parse"))
        .collect::<Vec<_>>();

    assert_eq!(events.len(), 6);
    assert!(events.iter().all(|event| !event.level.is_empty()));
    assert_eq!(
        events
            .iter()
            .map(|event| event.stage.as_str())
            .collect::<Vec<_>>(),
        vec![
            "queued",
            "prompt_ready",
            "checkpoint_created",
            "translating",
            "term_consistency",
            "finalized",
        ]
    );
    assert!(events
        .windows(2)
        .all(|pair| pair[1].progress >= pair[0].progress));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn finalize_outputs_persists_front_matter_artifact() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-front-matter-finalize-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_hash = "source-hash";
    let translated_markdown =
        "# Article Title\n\n## 摘要\n\n这是摘要。\n\n## 方法\n\n这是正文。".to_string();
    std::fs::write(
        &paths.source_markdown_path,
        "# Article Title\n\n## Abstract\n\nAbstract body.\n\n## Methods\n\nBody.",
    )
    .expect("source markdown should exist");
    std::fs::write(&paths.event_log_path, "").expect("event log should exist");
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: prompt.system_prompt_hash.clone(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: None,
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "Abstract body".to_string(),
            translated: Some("这是摘要。".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)
        .expect("checkpoint should exist before finalize");
    let front_matter = render::authors::AcademicFrontMatter {
        title: None,
        authors: vec![render::authors::AcademicAuthor {
            name: "Author One".to_string(),
            email: Some("author.one@example.test".to_string()),
            markers: vec!["1".to_string(), "*".to_string()],
            affiliations: vec!["Institute One".to_string()],
            is_corresponding: true,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: Vec::new(),
            correspondence_note: Some("Corresponding author".to_string()),
        }],
        extras: vec!["https://doi.org/example".to_string()],
        affiliation_catalog: Vec::new(),
        extra_entries: Vec::new(),
    };

    let result = reporting::finalize_translation_outputs(
        &pipeline,
        &config,
        &prompt,
        &paths,
        &checkpoint,
        source_hash,
        &translated_markdown,
        &front_matter,
        None,
        None,
        |_stage, _message, _progress| Ok(()),
        &|_stage, _message, _progress| {},
    )
    .await
    .expect("finalize outputs should succeed");
    assert!(std::path::Path::new(&result.output_html_path).exists());
    assert!(std::path::Path::new(&result.output_markdown_path).exists());
    assert!(paths.translated_chunks_path.exists());
    assert!(paths.translated_blocks_path.exists());
    assert!(paths.front_matter_path.exists());
    assert!(paths.source_front_matter_path.exists());
    assert!(paths.translated_front_matter_path.exists());

    let translated_chunks: Vec<TranslatedChunkArtifact> =
        artifact::read_json(&paths.translated_chunks_path)
            .expect("translated chunks artifact should load");
    assert_eq!(translated_chunks.len(), 1);
    assert_eq!(translated_chunks[0].chunk_index, 0);
    assert_eq!(translated_chunks[0].segment_kind, "Body");
    assert_eq!(translated_chunks[0].processing_stage, "Translated");
    assert_eq!(translated_chunks[0].source, "Abstract body");
    assert_eq!(
        translated_chunks[0].translated,
        checkpoint.chunks[0]
            .translated
            .as_deref()
            .expect("checkpoint chunk should have translation")
    );
    let translated_blocks: Vec<crate::document::TranslatedBlock> =
        artifact::read_json(&paths.translated_blocks_path)
            .expect("translated blocks artifact should load");
    assert_eq!(
        translated_blocks
            .iter()
            .find(|block| block.source_markdown == "Abstract body.")
            .and_then(|block| block.chunk_index),
        Some(0)
    );

    let persisted: render::authors::AcademicFrontMatter =
        artifact::read_json(&paths.front_matter_path).expect("front matter artifact should load");
    assert_eq!(persisted.authors.len(), 1);
    assert_eq!(persisted.authors[0].name, "Author One");
    assert_eq!(
        persisted.authors[0].email.as_deref(),
        Some("author.one@example.test")
    );
    assert_eq!(
        persisted.extras,
        vec!["https://doi.org/example".to_string()]
    );

    let source_persisted: render::authors::AcademicFrontMatter =
        artifact::read_json(&paths.source_front_matter_path)
            .expect("source front matter artifact should load");
    let translated_persisted: render::authors::AcademicFrontMatter =
        artifact::read_json(&paths.translated_front_matter_path)
            .expect("translated front matter artifact should load");
    assert_eq!(source_persisted.authors.len(), 1);
    assert_eq!(translated_persisted.authors.len(), 1);
    assert_eq!(
        source_persisted.extras,
        vec!["https://doi.org/example".to_string()]
    );
    assert_eq!(
        translated_persisted.extras,
        vec!["https://doi.org/example".to_string()]
    );

    let manifest: ArtifactManifest =
        artifact::read_json(&paths.manifest_path).expect("manifest should load");
    assert!(manifest
        .files
        .iter()
        .any(|file| file.kind == "translatedChunks"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn finalize_reuses_prefetched_front_matter_translation() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-front-matter-prefetch-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_hash = "source-hash";
    let translated_markdown = "# 文章标题\n\n## 摘要\n\n这是摘要。".to_string();
    std::fs::write(
        &paths.source_markdown_path,
        "# Article Title\n\n## Abstract\n\nBody.",
    )
    .expect("source markdown should exist");
    std::fs::write(&paths.event_log_path, "").expect("event log should exist");
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: prompt.system_prompt_hash.clone(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: None,
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "Body".to_string(),
            translated: Some("这是正文。".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)
        .expect("checkpoint should exist before finalize");
    let source_front_matter = render::authors::AcademicFrontMatter {
        title: Some("Evidence-Based Policy".to_string()),
        authors: vec![render::authors::AcademicAuthor {
            name: "Author One".to_string(),
            email: None,
            markers: Vec::new(),
            affiliations: vec!["Institute One".to_string()],
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

    // 预取任务：直接返回一份"已翻译"的 front matter（不触发任何 LLM 调用）。
    let prefetched = render::authors::AcademicFrontMatter {
        title: Some("循证政策".to_string()),
        ..source_front_matter.clone()
    };
    let prefetch_handle: reporting::FrontMatterPrefetch =
        tokio::spawn(async move { Ok(prefetched) });

    let result = reporting::finalize_translation_outputs(
        &pipeline,
        &config,
        &prompt,
        &paths,
        &checkpoint,
        source_hash,
        &translated_markdown,
        &source_front_matter,
        Some(prefetch_handle),
        None,
        |_stage, _message, _progress| Ok(()),
        &|_stage, _message, _progress| {},
    )
    .await
    .expect("finalize outputs should succeed");

    // 预取结果应被消费并落盘（供断点续跑复用）。
    assert!(std::path::Path::new(&result.output_markdown_path).exists());
    let persisted: render::authors::AcademicFrontMatter =
        artifact::read_json(&paths.translated_front_matter_path)
            .expect("translated front matter should persist");
    assert_eq!(persisted.title.as_deref(), Some("循证政策"));
    assert_eq!(persisted.authors.len(), 1);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn finalize_reuses_persisted_front_matter_on_resume() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-front-matter-resume-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_hash = "source-hash";
    let translated_markdown = "# 文章标题\n\n## 摘要\n\n这是摘要。".to_string();
    std::fs::write(
        &paths.source_markdown_path,
        "# Article Title\n\n## Abstract\n\nBody.",
    )
    .expect("source markdown should exist");
    std::fs::write(&paths.event_log_path, "").expect("event log should exist");
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: prompt.system_prompt_hash.clone(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: None,
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "Body".to_string(),
            translated: Some("这是正文。".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)
        .expect("checkpoint should exist before finalize");
    let source_front_matter = render::authors::AcademicFrontMatter {
        title: Some("Evidence-Based Policy".to_string()),
        authors: vec![render::authors::AcademicAuthor {
            name: "Author One".to_string(),
            email: None,
            markers: Vec::new(),
            affiliations: vec!["Institute One".to_string()],
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

    // 模拟前一次运行已落盘的译文（断点续跑场景：prefetch=None，应直接复用而非调用 LLM）。
    let persisted_translation = render::authors::AcademicFrontMatter {
        title: Some("循证政策".to_string()),
        ..source_front_matter.clone()
    };
    artifact::write_json(&paths.translated_front_matter_path, &persisted_translation)
        .await
        .expect("persisted translated front matter should be written");

    let result = reporting::finalize_translation_outputs(
        &pipeline,
        &config,
        &prompt,
        &paths,
        &checkpoint,
        source_hash,
        &translated_markdown,
        &source_front_matter,
        None,
        None,
        |_stage, _message, _progress| Ok(()),
        &|_stage, _message, _progress| {},
    )
    .await
    .expect("finalize outputs should succeed");

    // 复用的译文应进入最终产物（标题出现在面向用户的 markdown 中）。
    assert!(std::path::Path::new(&result.output_markdown_path).exists());
    let final_markdown =
        std::fs::read_to_string(&result.output_markdown_path).expect("final markdown should exist");
    assert!(final_markdown.contains("循证政策"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn epub_problem_slice_finalize_covers_titles_residuals_and_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-epub-problem-slice-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_hash = "epub-slice-source-hash";
    let source_markdown = [
        "# To the Daemon",
        "",
        "Tell me many tales.",
        "",
        "# Marooned in Andromeda",
        "",
        "Its movement had hateful deliberation. The looming head filled the cave with phosphorescence.",
        "",
        "# Appendix One: Story Notes",
        "",
        "Appendix One:",
        "Story Notes",
        "",
        "This note explains the story ending.",
        "",
        "# Abbreviations",
        "",
        "HPL and CAS are abbreviation notes.",
    ]
    .join("\n");
    std::fs::write(&paths.source_markdown_path, &source_markdown)
        .expect("source markdown should exist");
    std::fs::write(&paths.event_log_path, "").expect("event log should exist");

    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: prompt.system_prompt_hash.clone(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: Some(TranslationContext {
            proper_nouns: vec![ProperNounHint {
                source: "Daemon".to_string(),
                target: Some("恶魔".to_string()),
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 1,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            }],
            static_terms: Vec::new(),
        }),
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![
            merge_ready_chunk(
                0,
                "# To the Daemon\n\nTell me many tales.",
                "# 致魔神",
                Vec::new(),
            ),
            merge_ready_chunk(
                1,
                "Tell me many tales.",
                "# 致恶魔\n\n请告诉我许多故事。",
                Vec::new(),
            ),
            merge_ready_chunk(
                2,
                "Its movement had hateful deliberation. The looming head filled the cave with phosphorescence.",
                "它的动作中带着可恨的 deliberation。洞穴墙壁因它 looming 头部的磷光而变亮。",
                vec![ConfirmedTerm {
                    source: "phosphorescence".to_string(),
                    translation: "磷光".to_string(),
                    term_type: "technical".to_string(),
                    confidence: 92,
                    usage_role: "technical_term".to_string(),
                }],
            ),
            merge_ready_chunk(
                3,
                "# Appendix One: Story Notes\n\nAppendix One:\nStory Notes\n\n这是故事结尾的说明。",
                "# 附录一：故事注释\n\n附录一：\n故事注释\n\n这是故事结尾的说明。",
                Vec::new(),
            ),
            merge_ready_chunk(
                4,
                "# Abbreviations\n\nHPL and CAS are abbreviation notes.",
                "# Abbreviations\n\nHPL and CAS are abbreviation notes.",
                Vec::new(),
            ),
        ],
    };
    checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint)
        .expect("checkpoint should write wal snapshot");
    checkpoint::flush_checkpoint(&paths, &mut checkpoint).expect("checkpoint should flush");

    let translated_markdown =
        merge_translated_chunks(&checkpoint).expect("slice chunks should merge");
    let front_matter = render::authors::AcademicFrontMatter::default();
    let result = reporting::finalize_translation_outputs(
        &pipeline,
        &config,
        &prompt,
        &paths,
        &checkpoint,
        source_hash,
        &translated_markdown,
        &front_matter,
        None,
        None,
        |stage, message, progress| {
            pipeline.record_event(&paths.event_log_path, stage, message, progress)
        },
        &|_stage, _message, _progress| {},
    )
    .await
    .expect("finalize outputs should succeed");

    let final_markdown =
        std::fs::read_to_string(&result.output_markdown_path).expect("translated md exists");
    assert!(!final_markdown.contains("# 致魔神\n\n# 致恶魔"));
    assert!(!final_markdown.contains("# 致魔神"));
    assert!(final_markdown.contains("# 致恶魔"));
    assert_eq!(final_markdown.matches("附录一").count(), 1);

    let validation: serde_json::Value =
        artifact::read_json(&paths.validation_report_path).expect("validation should load");
    let residual_messages = validation
        .get("issues")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|issue| issue.get("message").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(residual_messages.contains("deliberation"));
    assert!(residual_messages.contains("looming"));
    assert!(!residual_messages.contains("HPL"));

    let glossary: serde_json::Value =
        artifact::read_json(&paths.glossary_path).expect("glossary should load");
    assert!(glossary
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .any(|entry| {
            entry.get("source").and_then(serde_json::Value::as_str) == Some("phosphorescence")
                && entry
                    .get("firstChunkIndex")
                    .and_then(serde_json::Value::as_u64)
                    == Some(2)
        }));

    let metrics: serde_json::Value =
        artifact::read_json(&paths.metrics_path).expect("metrics should load");
    assert!(
        metrics
            .get("residualEnglishIssueCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
            >= 2
    );

    let events_raw = std::fs::read_to_string(&paths.event_log_path).expect("events exist");
    for stage in [
        "validation_report_written",
        "glossary_start",
        "glossary_done",
        "metrics_start",
        "metrics_done",
        "manifest_start",
        "manifest_done",
    ] {
        assert!(events_raw.contains(stage), "missing event stage={stage}");
    }

    let wal_raw = std::fs::read_to_string(&paths.checkpoint_wal_path).expect("wal exists");
    assert!(wal_raw.contains("checkpoint_write_started"));
    assert!(
        wal_raw.contains("checkpoint_write_committed")
            || wal_raw.contains("checkpoint_flush_committed")
    );

    if std::env::var("MUSETRANSLATE_KEEP_EPUB_SLICE_ARTIFACT")
        .ok()
        .as_deref()
        == Some("1")
    {
        eprintln!("[epub-problem-slice] artifact={}", root.display());
    } else {
        let _ = std::fs::remove_dir_all(root);
    }
}

fn merge_ready_chunk(
    index: usize,
    source: &str,
    translated: &str,
    confirmed_terms: Vec<ConfirmedTerm>,
) -> ChunkState {
    ChunkState {
        index,
        source: source.to_string(),
        translated: Some(translated.to_string()),
        confirmed_terms,
        attempts: 1,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        segment_kind: ChunkSegmentKind::Body,
        processing_stage: Some(ChunkProcessingStage::MergeReady),
    }
}

#[test]
fn load_checkpoint_emits_chunking_stage_when_resume_is_available() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-pipeline-resume-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
    let source_hash = "source-hash";
    let now = "2026-01-01T00:00:00Z".to_string();
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: prompt.system_prompt_hash.clone(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: None,
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: now.clone(),
        updated_at: now.clone(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: split_reference_aware_markdown(
            source_markdown,
            config.chunk_size,
            &prompt.article_type,
        )
        .into_iter()
        .enumerate()
        .map(|(index, source)| ChunkState {
            index,
            source,
            translated: (index == 0).then_some("萨坦普拉路泽罗斯穿过了广场。".to_string()),
            confirmed_terms: Vec::new(),
            attempts: u8::from(index == 0),
            updated_at: now.clone(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        })
        .collect(),
    };
    apply_reference_chunk_policy(&mut checkpoint);
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)
        .expect("checkpoint should persist");
    let updates = Arc::new(Mutex::new(Vec::new()));
    let progress_updates = Arc::clone(&updates);

    let loaded = pipeline
        .load_or_initialize_checkpoint(
            &config,
            &prompt,
            &paths,
            source_markdown,
            source_hash,
            &|stage, message, progress| {
                progress_updates
                    .lock()
                    .expect("progress mutex should remain healthy")
                    .push((stage, message.to_string(), progress));
            },
        )
        .expect("checkpoint should resume");

    assert_eq!(
        loaded.chunks[0].translated.as_deref(),
        Some("萨坦普拉路泽罗斯穿过了广场。")
    );
    let updates = updates
        .lock()
        .expect("progress mutex should remain healthy");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, PipelineProgressStage::Chunking);
    assert_eq!(
        updates[0].1,
        "loaded resumable checkpoint; translated=1/1; source_hash_refreshed=false"
    );
    assert_eq!(updates[0].2, 32);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn cached_source_front_matter_is_reused_and_rewrites_source_markdown() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-front-matter-cache-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let paths = ArtifactPaths::new(&root);
    let mut source_document = crate::ocr::OcrDocument {
        markdown: [
            "https://doi.org/10.1/example",
            "",
            "# Raw Title",
            "",
            "Jane Doe $ ^{1} $, John Roe $ ^{2,*} $",
            "",
            "Check for updates",
            "",
            "Body paragraph starts here.",
        ]
        .join("\n"),
        cover_data_url: None,
        page_layouts: Vec::new(),
        markdown_images: std::collections::HashMap::new(),
        toc: None,
        epub_blocks: Vec::new(),
        source_blocks: crate::document::build_source_blocks_from_markdown(
            &[
                "https://doi.org/10.1/example",
                "",
                "# Raw Title",
                "",
                "Jane Doe $ ^{1} $, John Roe $ ^{2,*} $",
                "",
                "Check for updates",
                "",
                "Body paragraph starts here.",
            ]
            .join("\n"),
        ),
    };
    let cached = render::authors::AcademicFrontMatter {
        title: Some("Normalized Title".to_string()),
        authors: vec![render::authors::AcademicAuthor {
            name: "Jane Doe".to_string(),
            email: None,
            markers: vec!["1".to_string()],
            affiliations: vec!["Department A".to_string()],
            is_corresponding: false,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: Vec::new(),
            correspondence_note: None,
        }],
        extras: vec!["https://doi.org/10.1/example".to_string()],
        affiliation_catalog: Vec::new(),
        extra_entries: Vec::new(),
    };
    artifact::write_json(&paths.source_front_matter_path, &cached)
        .await
        .expect("write cached front matter");

    let config = sample_run_config(&root);
    let extracted = pipeline
        .extract_source_front_matter(
            &config,
            &mut source_document,
            &paths,
            &|_stage, _message, _progress| {},
        )
        .await
        .expect("front matter should reuse cache");

    assert_eq!(extracted.title.as_deref(), Some("Normalized Title"));
    assert!(source_document.markdown.contains("Normalized Title"));
    assert!(!source_document.markdown.contains("Check for updates"));
    let rebuilt_blocks =
        crate::document::build_source_blocks_from_markdown(&source_document.markdown);
    assert_eq!(source_document.source_blocks, rebuilt_blocks);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_checkpoint_emits_rebuild_then_created_sequence_for_unreadable_state() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-pipeline-rebuild-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
    std::fs::write(&paths.checkpoint_path, "{broken json")
        .expect("broken checkpoint should be written");
    let updates = Arc::new(Mutex::new(Vec::new()));
    let progress_updates = Arc::clone(&updates);

    let loaded = pipeline
        .load_or_initialize_checkpoint(
            &config,
            &prompt,
            &paths,
            source_markdown,
            "source-hash",
            &|stage, message, progress| {
                progress_updates
                    .lock()
                    .expect("progress mutex should remain healthy")
                    .push((stage, message.to_string(), progress));
            },
        )
        .expect("unreadable checkpoint should rebuild");

    assert!(!loaded.chunks.is_empty());
    let updates = updates
        .lock()
        .expect("progress mutex should remain healthy");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].0, PipelineProgressStage::Chunking);
    assert_eq!(
        updates[0].1,
        "checkpoint unreadable, rebuilding from source"
    );
    assert_eq!(updates[0].2, 31);
    assert_eq!(updates[1].0, PipelineProgressStage::Chunking);
    assert!(updates[1].1.starts_with("initialized checkpoint with "));
    assert_eq!(updates[1].2, 32);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_checkpoint_records_pollution_cleanup_before_resume_ready() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-pipeline-resume-pollution-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_markdown = [
        "Hyperborea stood beyond the northern sea, where the wind carried old names from the icebound coast. The harbor bells rang through the frost.",
        "",
        "Queen Cunambria guarded the ancient tower above the frozen harbor, watching every ship that crossed the grey water at dusk.",
    ]
    .join("\n\n");
    let source_hash = "source-hash";
    let source_chunks =
        split_reference_aware_markdown(&source_markdown, config.chunk_size, &prompt.article_type);
    assert!(
        source_chunks.len() >= 2,
        "test source should split into at least two chunks"
    );
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: prompt.system_prompt_hash.clone(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: None,
        source_hash: source_hash.to_string(),
        chunk_size: config.chunk_size,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:01Z".to_string(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: source_chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| ChunkState {
                index,
                source,
                translated: match index {
                    0 => Some("\u{fffd}\u{fffd}\u{fffd}\u{fffd}损坏文本".to_string()),
                    1 => Some("库纳姆布里亚女王守望着冰封港口上方的古塔。".to_string()),
                    _ => None,
                },
                confirmed_terms: Vec::new(),
                attempts: if index < 2 { 1 } else { 0 },
                updated_at: "2026-01-01T00:00:01Z".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            })
            .collect(),
    };
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)
        .expect("checkpoint should persist");
    let updates = Arc::new(Mutex::new(Vec::new()));
    let progress_updates = Arc::clone(&updates);

    let loaded = pipeline
        .load_or_initialize_checkpoint(
            &config,
            &prompt,
            &paths,
            &source_markdown,
            source_hash,
            &|stage, message, progress| {
                progress_updates
                    .lock()
                    .expect("progress mutex should remain healthy")
                    .push((stage, message.to_string(), progress));
            },
        )
        .expect("checkpoint should resume");

    assert_eq!(loaded.chunks[0].translated, None);
    assert_eq!(
        loaded.chunks[1].translated.as_deref(),
        Some("库纳姆布里亚女王守望着冰封港口上方的古塔。")
    );
    let updates = updates
        .lock()
        .expect("progress mutex should remain healthy");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, PipelineProgressStage::Chunking);
    assert_eq!(
        updates[0].1,
        "loaded resumable checkpoint; translated=1/3; source_hash_refreshed=false"
    );
    assert_eq!(updates[0].2, 32);

    let raw = std::fs::read_to_string(&paths.event_log_path).expect("event log should exist");
    let events = raw
        .lines()
        .map(|line| serde_json::from_str::<TaskEvent>(line).expect("event line should parse"))
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .map(|event| event.stage.as_str())
            .collect::<Vec<_>>(),
        vec![
            "checkpoint_pollution_rebuild",
            "resume_ready",
            "resume_audit_warning"
        ]
    );
    assert_eq!(
        events[0].message,
        "invalidated 1 polluted chunks before resume"
    );
    assert_eq!(
        events[1].message,
        "loaded resumable checkpoint; translated=1/3; source_hash_refreshed=false"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_checkpoint_rebuild_due_to_resume_mismatch_does_not_emit_resume_ready() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-pipeline-resume-mismatch-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("temp dir should exist");
    let pipeline = TranslationPipeline::new(
        "http://ocr.local".to_string(),
        "token".to_string(),
        "http://llm.local".to_string(),
        "key".to_string(),
        "model".to_string(),
    );
    let config = sample_run_config(&root);
    let prompt = sample_prompt_config();
    let paths = ArtifactPaths::new(&root);
    let source_markdown = "Satampra Zeiros crossed the square.\n\nQueen Cunambria answered.";
    let now = "2026-01-01T00:00:00Z".to_string();
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: prompt.article_type.clone(),
        system_prompt_hash: "stale-prompt-hash".to_string(),
        skill_ids: prompt.skill_ids.clone(),
        translation_context: None,
        source_hash: "source-hash".to_string(),
        chunk_size: config.chunk_size,
        created_at: now.clone(),
        updated_at: now.clone(),
        source_markdown_path: paths.source_markdown_path.to_string_lossy().to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: split_reference_aware_markdown(
            source_markdown,
            config.chunk_size,
            &prompt.article_type,
        )
        .into_iter()
        .enumerate()
        .map(|(index, source)| ChunkState {
            index,
            source,
            translated: Some(format!("stale translation {index}")),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: now.clone(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        })
        .collect(),
    };
    apply_reference_chunk_policy(&mut checkpoint);
    artifact::persist_checkpoint(&paths.checkpoint_path, &checkpoint)
        .expect("checkpoint should persist");
    let updates = Arc::new(Mutex::new(Vec::new()));
    let progress_updates = Arc::clone(&updates);

    let loaded = pipeline
        .load_or_initialize_checkpoint(
            &config,
            &prompt,
            &paths,
            source_markdown,
            "source-hash",
            &|stage, message, progress| {
                progress_updates
                    .lock()
                    .expect("progress mutex should remain healthy")
                    .push((stage, message.to_string(), progress));
            },
        )
        .expect("checkpoint should rebuild when resume contract mismatches");

    assert!(loaded.chunks.iter().all(|chunk| chunk.translated.is_none()));
    let updates = updates
        .lock()
        .expect("progress mutex should remain healthy");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, PipelineProgressStage::Chunking);
    assert!(updates[0].1.starts_with("initialized checkpoint with "));
    assert_eq!(updates[0].2, 32);

    let raw = std::fs::read_to_string(&paths.event_log_path).expect("event log should exist");
    let events = raw
        .lines()
        .map(|line| serde_json::from_str::<TaskEvent>(line).expect("event line should parse"))
        .collect::<Vec<_>>();
    // resume 行为硬化后：checkpoint_created 之后还会记录 resume_audit 审计事件（只报告不自动修）；
    // 本用例 invalidated 后处于全 pending，审计判定存在不一致，故为 warning 级。
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].stage, "checkpoint_created");
    assert!(events[0]
        .message
        .starts_with("initialized checkpoint with "));
    assert_eq!(events[1].stage, "resume_audit_warning");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn retry_delay_keeps_same_schedule_for_non_rate_limit_errors() {
    let error = AppError::new("LLM_TIMEOUT", "timeout", true);

    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 1),
        Duration::from_secs(5)
    );
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 3),
        Duration::from_secs(15)
    );
    assert_eq!(
        compute_retry_delay(&error, Duration::from_secs(2), 5),
        Duration::from_secs(25)
    );
}

#[test]
fn parses_repaired_translation_from_fenced_json() {
    let review = "```json\n{\"decision\":\"repair\",\"repairedText\":\"repaired body\"}\n```";

    assert_eq!(
        parse_repaired_translation(review).as_deref(),
        Some("repaired body")
    );
}

#[test]
fn validation_flags_residual_english_words() {
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "a vague fear of the monstrous jungle".to_string(),
            translated: Some("对这 monstrous 丛林泛起隐约的恐惧".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };

    let report = artifact::build_validation_report("test", &checkpoint);

    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "RESIDUAL_ENGLISH_WORD"));
}

#[test]
fn validation_summary_exposes_stage_error_signals() {
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: Some(TranslationContext {
            proper_nouns: vec![ProperNounHint {
                source: "Satampra Zeiros".to_string(),
                target: Some("萨坦普拉路泽罗斯".to_string()),
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 1,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            }],
            static_terms: vec![
                StaticGlossaryEntry {
                    source: "Satampra Zeiros".to_string(),
                    target: "Satampra-target".to_string(),
                    scope: "global".to_string(),
                    enforcement: "strict".to_string(),
                    notes: String::new(),
                },
                StaticGlossaryEntry {
                    source: "monstrous".to_string(),
                    target: "monstrous-target".to_string(),
                    scope: "global".to_string(),
                    enforcement: "contextual".to_string(),
                    notes: String::new(),
                },
            ],
        }),
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:02Z".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![
            ChunkState {
                index: 0,
                source: "# Chapter\n\nSatampra Zeiros entered the city.".to_string(),
                translated: Some("# Chapter\n\n泽罗斯进入了城市。".to_string()),
                confirmed_terms: Vec::new(),
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            },
            ChunkState {
                index: 1,
                source: "a vague fear of the monstrous jungle".to_string(),
                translated: Some("对这 monstrous 丛林泛起隐约的恐惧".to_string()),
                confirmed_terms: Vec::new(),
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            },
            ChunkState {
                index: 2,
                source: "An untranslated section.".to_string(),
                translated: None,
                confirmed_terms: Vec::new(),
                attempts: 0,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            },
        ],
    };

    let report = artifact::build_validation_report("test", &checkpoint);
    let issue_codes = report
        .issues
        .iter()
        .map(|issue| issue.code.as_str())
        .collect::<HashSet<_>>();

    assert_eq!(report.summary.untranslated_chunk_count, 1);
    assert_eq!(report.summary.markdown_heading_count, 1);
    assert!(report.summary.issue_count >= 2);
    assert!(issue_codes.contains("RESIDUAL_ENGLISH_WORD"));
    assert!(issue_codes.contains("MISSING_CANONICAL_PROPER_NOUN_TRANSLATION"));
}

#[test]
fn metrics_artifact_quantifies_quality_effects() {
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: Some(TranslationContext {
            proper_nouns: vec![
                ProperNounHint {
                    source: "Satampra Zeiros".to_string(),
                    target: Some("萨坦普拉路泽罗斯".to_string()),
                    enforcement: ProperNounEnforcement::Strict,
                    occurrences: 1,
                    first_chunk_index: 0,
                    chunk_indexes: vec![0],
                },
                ProperNounHint {
                    source: "Martian".to_string(),
                    target: Some("火星人".to_string()),
                    enforcement: ProperNounEnforcement::Contextual,
                    occurrences: 1,
                    first_chunk_index: 1,
                    chunk_indexes: vec![1],
                },
                ProperNounHint {
                    source: "Queen Cunambria".to_string(),
                    target: None,
                    enforcement: ProperNounEnforcement::Strict,
                    occurrences: 1,
                    first_chunk_index: 2,
                    chunk_indexes: vec![2],
                },
            ],
            static_terms: vec![
                StaticGlossaryEntry {
                    source: "Satampra Zeiros".to_string(),
                    target: "Satampra-target".to_string(),
                    scope: "global".to_string(),
                    enforcement: "strict".to_string(),
                    notes: String::new(),
                },
                StaticGlossaryEntry {
                    source: "monstrous".to_string(),
                    target: "monstrous-target".to_string(),
                    scope: "global".to_string(),
                    enforcement: "contextual".to_string(),
                    notes: String::new(),
                },
            ],
        }),
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:04Z".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![
            ChunkState {
                index: 0,
                source: "Satampra Zeiros entered the city.".to_string(),
                translated: Some("泽罗斯进入了城市。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "Satampra Zeiros".to_string(),
                    translation: "泽罗斯".to_string(),
                    term_type: "person".to_string(),
                    confidence: 90,
                    usage_role: "person_name".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::ResidualReviewed),
            },
            ChunkState {
                index: 1,
                source: "The Martian rulers watched the monstrous sky.".to_string(),
                translated: Some("火星统治者注视着 monstrous 天空。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "Martian".to_string(),
                    translation: "火星人".to_string(),
                    term_type: "group".to_string(),
                    confidence: 88,
                    usage_role: "group_name".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::TermConsistencyApplied),
            },
            ChunkState {
                index: 2,
                source: "Satampra Zeiros met Queen Cunambria.".to_string(),
                translated: Some("萨坦普拉路泽罗斯遇见了库纳姆布里亚女王。".to_string()),
                confirmed_terms: vec![ConfirmedTerm {
                    source: "Satampra Zeiros".to_string(),
                    translation: "萨坦普拉路泽罗斯".to_string(),
                    term_type: "person".to_string(),
                    confidence: 93,
                    usage_role: "person_name".to_string(),
                }],
                attempts: 1,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: Some(ChunkProcessingStage::MergeReady),
            },
        ],
    };

    let validation = artifact::build_validation_report("test", &checkpoint);
    let glossary = artifact::build_glossary_artifact("fiction", &checkpoint);
    let metrics = artifact::build_metrics_artifact(
        &checkpoint,
        &validation,
        &glossary,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("metrics");

    assert_eq!(metrics.total_chunks, 3);
    assert_eq!(metrics.translated_chunks, 3);
    assert_eq!(metrics.chunks_with_confirmed_terms, 3);
    assert_eq!(metrics.confirmed_term_count, 3);
    assert!(metrics.strict_glossary_entries >= 1);
    assert!(metrics.contextual_glossary_entries >= 1);
    assert_eq!(metrics.strict_proper_noun_hint_count, 2);
    assert_eq!(metrics.contextual_proper_noun_hint_count, 1);
    assert_eq!(metrics.missing_proper_noun_target_count, 1);
    assert_eq!(metrics.duplicate_proper_noun_source_count, 0);
    assert_eq!(metrics.multi_translation_confirmed_term_count, 1);
    assert!(metrics.validation_issue_count >= 1);
    assert_eq!(metrics.residual_english_issue_count, 1);
    assert_eq!(metrics.residual_review_candidate_chunk_count, 1);
    assert_eq!(metrics.residual_review_candidate_term_count, 1);
    assert_eq!(metrics.residual_review_candidate_limit, 96);
    assert!(!metrics.residual_review_hit_candidate_limit);
    assert_eq!(metrics.strict_consistency_issue_count, 1);
    assert_eq!(metrics.elapsed_ms, Some(4_000));
}

#[test]
fn academic_references_are_preserved_and_not_reviewed() {
    let markdown = "# Paper\n\nBody paragraph.\n\n## References\n\nSmith, J. (2020). Test article.\n\nJones, A. (2021). Another paper.";
    let chunks = split_reference_aware_markdown(markdown, 90, "academic");

    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.pdf".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 90,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| ChunkState {
                index,
                source,
                translated: None,
                confirmed_terms: Vec::new(),
                attempts: 0,
                updated_at: "now".to_string(),
                segment_kind: ChunkSegmentKind::Body,
                processing_stage: None,
            })
            .collect(),
    };

    apply_reference_chunk_policy(&mut checkpoint);

    assert!(checkpoint
        .chunks
        .iter()
        .any(|chunk| chunk.segment_kind == ChunkSegmentKind::Reference));
}

#[test]
fn checkpoint_atomic_write_preserves_utf8_json() {
    let path = std::env::temp_dir().join(format!(
        "musetranslate-checkpoint-{}.json",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.pdf".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 90,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "源文本".to_string(),
            translated: Some("译文".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };

    artifact::persist_checkpoint(&path, &checkpoint).expect("checkpoint should write");
    let loaded: ChunkCheckpoint = artifact::read_json(&path).expect("checkpoint should read");

    assert_eq!(loaded.chunks[0].translated.as_deref(), Some("译文"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn dirty_checkpoint_write_records_write_metrics() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-checkpoint-write-metrics-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let paths = ArtifactPaths::new(&root);
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.pdf".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 90,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "源文本".to_string(),
            translated: Some("译文".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };

    checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint)
        .expect("dirty checkpoint should write");

    assert_eq!(checkpoint.write_metrics.write_count, 1);
    assert_eq!(checkpoint.write_metrics.pending_write_count, 0);
    assert_eq!(checkpoint.write_metrics.last_flush_write_count, 1);
    let loaded: ChunkCheckpoint =
        artifact::read_json(&paths.checkpoint_path).expect("checkpoint should read");
    assert_eq!(loaded.chunks[0].translated.as_deref(), Some("译文"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn dirty_checkpoint_write_appends_wal_events() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-checkpoint-wal-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let paths = ArtifactPaths::new(&root);
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.pdf".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 90,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "婧愭枃鏈?".to_string(),
            translated: Some("璇戞枃".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };

    checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint)
        .expect("dirty checkpoint should write and append wal");

    let wal_raw =
        std::fs::read_to_string(&paths.checkpoint_wal_path).expect("checkpoint wal should exist");
    assert!(wal_raw.contains("checkpoint_write_started"));
    assert!(
        wal_raw.contains("checkpoint_write_committed")
            || wal_raw.contains("checkpoint_flush_committed")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn checkpoint_flush_interval_batches_noncritical_writes_but_flushes_on_demand() {
    let root = std::env::temp_dir().join(format!(
        "musetranslate-checkpoint-batch-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let paths = ArtifactPaths::new(&root);
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.pdf".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 90,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 0,
            source: "source".to_string(),
            translated: Some("translated".to_string()),
            confirmed_terms: Vec::new(),
            attempts: 1,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };

    checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint).expect("first write");
    checkpoint.chunks[0].translated = Some("translated 2".to_string());
    checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint).expect("second write");
    checkpoint.chunks[0].translated = Some("translated 3".to_string());
    checkpoint::persist_dirty_checkpoint(&paths, &mut checkpoint).expect("third write");

    assert_eq!(checkpoint.write_metrics.write_count, 3);
    assert_eq!(checkpoint.write_metrics.pending_write_count, 1);
    assert_eq!(checkpoint.write_metrics.last_flush_write_count, 2);

    checkpoint::flush_checkpoint(&paths, &mut checkpoint).expect("explicit flush");
    assert_eq!(checkpoint.write_metrics.pending_write_count, 0);
    assert_eq!(checkpoint.write_metrics.last_flush_write_count, 3);

    let loaded: ChunkCheckpoint =
        artifact::read_json(&paths.checkpoint_path).expect("checkpoint should read");
    assert_eq!(loaded.chunks[0].translated.as_deref(), Some("translated 3"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn chunk_context_prioritizes_exact_strict_hits() {
    let context = TranslationContext {
        proper_nouns: vec![
            ProperNounHint {
                source: "Hyperborea".to_string(),
                target: Some("许珀耳玻瑞亚".to_string()),
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 2,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            },
            ProperNounHint {
                source: "Tirouv Ompallios".to_string(),
                target: Some("提罗乌·翁帕利奥斯".to_string()),
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 4,
                first_chunk_index: 0,
                chunk_indexes: vec![1],
            },
        ],
        static_terms: Vec::new(),
    };

    let rendered = format_translation_context_for_chunk(
        &context,
        "fiction",
        1,
        "Tirouv Ompallios walked alone.",
    );

    assert!(rendered.contains("Strict glossary hits in this chunk:"));
    assert!(rendered.contains("Tirouv Ompallios => MUST USE EXACTLY 提罗乌·翁帕利奥斯"));
    assert!(!rendered.contains("Hyperborea => MUST USE EXACTLY"));
    assert!(!rendered.contains("- Hyperborea ->"));
}

#[test]
fn algorithmic_hints_are_scoped_to_originating_chunks() {
    let context = TranslationContext {
        proper_nouns: vec![ProperNounHint {
            source: "Queen Cunambria".to_string(),
            target: None,
            enforcement: ProperNounEnforcement::Strict,
            occurrences: 1,
            first_chunk_index: 0,
            chunk_indexes: vec![0],
        }],
        static_terms: Vec::new(),
    };

    let origin_rendered = format_translation_context_for_chunk(
        &context,
        "fiction",
        0,
        "Queen Cunambria entered the hall.",
    );
    let later_rendered = format_translation_context_for_chunk(
        &context,
        "fiction",
        1,
        "Queen Cunambria returned at dusk.",
    );

    assert!(origin_rendered.contains("Algorithmic proper-noun candidates in this chunk:"));
    assert!(origin_rendered.contains("Queen Cunambria -> likely proper noun"));
    assert!(!later_rendered.contains("Queen Cunambria"));
}

#[test]
fn confirmed_glossary_hints_still_match_later_source_mentions() {
    let context = TranslationContext {
        proper_nouns: vec![ProperNounHint {
            source: "Queen Cunambria".to_string(),
            target: Some("库南布里亚女王".to_string()),
            enforcement: ProperNounEnforcement::Strict,
            occurrences: 2,
            first_chunk_index: 0,
            chunk_indexes: vec![0],
        }],
        static_terms: Vec::new(),
    };

    let rendered = format_translation_context_for_chunk(
        &context,
        "fiction",
        7,
        "Queen Cunambria returned at dusk.",
    );

    assert!(rendered.contains("Strict glossary hits in this chunk:"));
    assert!(rendered.contains("Queen Cunambria => MUST USE EXACTLY 库南布里亚女王"));
}

#[test]
fn confirmed_terms_prune_matching_algorithmic_hints_for_current_chunk_only() {
    let mut context = TranslationContext {
        proper_nouns: vec![
            ProperNounHint {
                source: "Queen Cunambria".to_string(),
                target: None,
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 1,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            },
            ProperNounHint {
                source: "Queen Cunambria".to_string(),
                target: None,
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 1,
                first_chunk_index: 1,
                chunk_indexes: vec![1],
            },
            ProperNounHint {
                source: "Hyperborea".to_string(),
                target: Some("许珀耳玻瑞亚".to_string()),
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 2,
                first_chunk_index: 0,
                chunk_indexes: vec![0],
            },
        ],
        static_terms: Vec::new(),
    };
    let confirmed_terms = vec![ConfirmedTerm {
        source: "Queen Cunambria".to_string(),
        translation: "库南布里亚女王".to_string(),
        term_type: "person".to_string(),
        confidence: 95,
        usage_role: "person_name".to_string(),
    }];

    let pruned = crate::pipeline::context::prune_confirmed_algorithmic_hints(
        &mut context,
        0,
        &confirmed_terms,
    );

    assert_eq!(pruned, 1);
    assert!(context.proper_nouns.iter().any(|hint| {
        hint.source == "Queen Cunambria" && hint.target.is_none() && hint.chunk_indexes == vec![1]
    }));
    assert!(context
        .proper_nouns
        .iter()
        .any(|hint| hint.source == "Hyperborea" && hint.target.is_some()));
}

#[test]
fn source_hint_matching_uses_phrase_boundaries() {
    assert!(source_mentions_hint(
        "Tirouv Ompallios walked alone.",
        "Tirouv Ompallios"
    ));
    assert!(source_mentions_hint(
        "White Sybil of Polarion spoke at dusk.",
        "Polarion"
    ));
    assert!(source_mentions_hint(
        "Hyperborean rulers watched the sky.",
        "Hyperborea"
    ));
    assert!(source_mentions_hint(
        "the chinese envoy arrived at dawn.",
        "china"
    ));
    assert!(source_mentions_hint(
        "Tirouv's oath still bound the harbor.",
        "Tirouv Ompallios"
    ));
}

#[test]
fn strict_hit_terms_can_be_synthesized_when_text_is_correct() {
    let strict_hits = vec![StrictChunkHit {
        source: "Tirouv Ompallios".to_string(),
        target: "提罗乌·翁帕利奥斯".to_string(),
        usage_role: "person_name".to_string(),
        term_type: "person".to_string(),
    }];
    let mut confirmed_terms = Vec::new();

    synthesize_missing_strict_hit_terms(
        &strict_hits,
        "提罗乌·翁帕利奥斯走进了神殿。",
        &mut confirmed_terms,
    );

    assert_eq!(confirmed_terms.len(), 1);
    assert_eq!(confirmed_terms[0].source, "Tirouv Ompallios");
    assert_eq!(confirmed_terms[0].translation, "提罗乌·翁帕利奥斯");
}

#[test]
fn strict_hit_aliases_are_repaired_before_validation() {
    let strict_hits = vec![StrictChunkHit {
        source: "Tirouv Ompallios".to_string(),
        target: "提罗乌·翁帕利奥斯".to_string(),
        usage_role: "person_name".to_string(),
        term_type: "person".to_string(),
    }];
    let confirmed_terms = vec![ConfirmedTerm {
        source: "Tirouv Ompallios".to_string(),
        translation: "蒂罗夫·翁帕利奥斯".to_string(),
        term_type: "person".to_string(),
        confidence: 95,
        usage_role: "person_name".to_string(),
    }];

    let repaired = repair_strict_hit_aliases(
        "蒂罗夫·翁帕利奥斯走进了神殿。",
        &confirmed_terms,
        &strict_hits,
    );

    assert!(repaired.contains("提罗乌·翁帕利奥斯"));
    assert!(!repaired.contains("蒂罗夫·翁帕利奥斯"));
}

#[test]
fn canonical_proper_noun_repair_preserves_markdown_paths_and_longer_tokens() {
    let context = TranslationContext {
        proper_nouns: vec![ProperNounHint {
            source: "Hyperborea".to_string(),
            target: Some("许珀耳玻瑞亚".to_string()),
            enforcement: ProperNounEnforcement::Strict,
            occurrences: 2,
            first_chunk_index: 0,
            chunk_indexes: vec![0],
        }],
        static_terms: Vec::new(),
    };

    let repaired = crate::pipeline::context::repair_canonical_proper_nouns(
        "Hyperborea 仍在。![](imgs/Hyperborea-map.png) Hyperborean rulers watched.",
        Some(&context),
    );

    assert!(repaired.contains("许珀耳玻瑞亚 仍在。"));
    assert!(repaired.contains("imgs/Hyperborea-map.png"));
    assert!(repaired.contains("Hyperborean rulers"));
}

#[test]
fn final_context_repairs_fix_parallel_strict_hit_drift() {
    let mut checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: Some(TranslationContext {
            proper_nouns: vec![ProperNounHint {
                source: "Hyperborea".to_string(),
                target: Some("鏋佸寳涔嬪湴".to_string()),
                enforcement: ProperNounEnforcement::Strict,
                occurrences: 2,
                first_chunk_index: 3,
                chunk_indexes: vec![7],
            }],
            static_terms: Vec::new(),
        }),
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ChunkState {
            index: 7,
            source: "Such temples are rare in Hyperborea nowadays.".to_string(),
            translated: Some("这样的神庙在 Hyperborea 已很罕见。".to_string()),
            confirmed_terms: vec![ConfirmedTerm {
                source: "Hyperborea".to_string(),
                translation: "Hyperborea".to_string(),
                term_type: "location".to_string(),
                confidence: 90,
                usage_role: "place_name".to_string(),
            }],
            attempts: 1,
            updated_at: "now".to_string(),
            segment_kind: ChunkSegmentKind::Body,
            processing_stage: None,
        }],
    };

    let repaired = repair_body_chunks_with_final_context(&mut checkpoint);

    assert_eq!(repaired, 1);
    let chunk = &checkpoint.chunks[0];
    assert!(chunk
        .translated
        .as_deref()
        .is_some_and(|text| text.contains("鏋佸寳涔嬪湴")));
    assert_eq!(chunk.confirmed_terms[0].translation, "鏋佸寳涔嬪湴");
}

#[test]
fn pending_indexes_include_only_untranslated_body_chunks() {
    let mut pending_with_stale_text =
        chunk_state(4, "正文三", Some("旧译文三"), ChunkSegmentKind::Body);
    pending_with_stale_text.processing_stage = Some(ChunkProcessingStage::Pending);
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![
            chunk_state(0, "正文一", None, ChunkSegmentKind::Body),
            chunk_state(1, "References", None, ChunkSegmentKind::Reference),
            chunk_state(
                2,
                "<!-- Authors -->",
                None,
                ChunkSegmentKind::ProtectedMetadata,
            ),
            chunk_state(3, "正文二", Some("译文二"), ChunkSegmentKind::Body),
            pending_with_stale_text,
        ],
    };

    assert_eq!(collect_pending_indexes(&checkpoint), vec![0, 4]);
}

#[test]
fn stage_gate_detects_term_consistency_work_only_for_translated_body_chunks() {
    let mut translated = chunk_state(0, "正文", Some("译文"), ChunkSegmentKind::Body);
    translated.processing_stage = Some(ChunkProcessingStage::Translated);
    let mut residual_done = chunk_state(1, "正文二", Some("译文二"), ChunkSegmentKind::Body);
    residual_done.processing_stage = Some(ChunkProcessingStage::ResidualReviewed);
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![translated, residual_done],
    };

    assert!(needs_term_consistency(&checkpoint));
    assert!(!needs_residual_review(&checkpoint));
}

#[test]
fn stage_gate_detects_residual_review_only_for_consistency_complete_chunks() {
    let mut consistent = chunk_state(0, "正文", Some("译文"), ChunkSegmentKind::Body);
    consistent.processing_stage = Some(ChunkProcessingStage::TermConsistencyApplied);
    let mut merge_ready = chunk_state(1, "正文二", Some("译文二"), ChunkSegmentKind::Body);
    merge_ready.processing_stage = Some(ChunkProcessingStage::MergeReady);
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![consistent, merge_ready],
    };

    assert!(!needs_term_consistency(&checkpoint));
    assert!(needs_residual_review(&checkpoint));
}

#[test]
fn stage_gate_skips_finished_quality_stages_on_resume() {
    let mut merge_ready = chunk_state(0, "正文", Some("译文"), ChunkSegmentKind::Body);
    merge_ready.processing_stage = Some(ChunkProcessingStage::MergeReady);
    let mut protected = chunk_state(
        1,
        "References",
        Some("References"),
        ChunkSegmentKind::Reference,
    );
    protected.processing_stage = Some(ChunkProcessingStage::ProtectedPassthrough);
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![merge_ready, protected],
    };

    assert!(!needs_term_consistency(&checkpoint));
    assert!(!needs_residual_review(&checkpoint));
    assert!(!needs_merge_ready_advance(&checkpoint));
}

#[test]
fn merge_requires_translated_reference_and_metadata_chunks() {
    let mut body = chunk_state(0, "正文", Some("译文"), ChunkSegmentKind::Body);
    body.processing_stage = Some(ChunkProcessingStage::MergeReady);
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![
            body,
            chunk_state(1, "References", None, ChunkSegmentKind::Reference),
        ],
    };

    let error = merge_translated_chunks(&checkpoint).expect_err("reference text is still required");

    assert!(error.message.contains("missing translated chunk"));
}

#[test]
fn merge_rejects_body_chunks_before_quality_stages_complete() {
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "academic".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![chunk_state(0, "正文", Some("译文"), ChunkSegmentKind::Body)],
    };

    let error = merge_translated_chunks(&checkpoint).expect_err("body chunk needs stage gating");

    assert!(error.message.contains("not merge-ready"));
}

#[test]
fn merge_keeps_source_text_for_translation_failed_chunks() {
    let mut failed = chunk_state(1, "Failed block source", None, ChunkSegmentKind::Body);
    failed.processing_stage = Some(ChunkProcessingStage::TranslationFailed);
    let mut ok_chunk = chunk_state(0, "Source block", Some("翻译块"), ChunkSegmentKind::Body);
    ok_chunk.processing_stage = Some(ChunkProcessingStage::MergeReady);
    let checkpoint = ChunkCheckpoint {
        version: 1,
        task_id: "test".to_string(),
        source_pdf_path: "sample.epub".to_string(),
        article_type: "fiction".to_string(),
        system_prompt_hash: "hash".to_string(),
        skill_ids: vec!["translation:core".to_string()],
        translation_context: None,
        source_hash: "hash".to_string(),
        chunk_size: 1000,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
        source_markdown_path: "source.md".to_string(),
        write_metrics: Default::default(),
        delta_sync: Default::default(),
        chunks: vec![ok_chunk, failed],
    };

    // 失败块不毁整篇：产物照常生成，失败块保留原文。
    let merged = merge_translated_chunks(&checkpoint).expect("failed chunks should not block merge");
    assert!(merged.contains("翻译块"));
    assert!(merged.contains("Failed block source"));

    // 失败清单正确归因。
    let failed_indexes = failed_body_chunk_indexes(&checkpoint);
    assert_eq!(failed_indexes, vec![1]);
}

#[tokio::test]
async fn pipeline_can_build_with_limiter_and_cancellation() {
    let _limiter = Arc::new(Semaphore::new(3));
    let token = CancellationToken::new();
    assert!(ensure_not_cancelled(&token).is_ok());
}

fn chunk_state(
    index: usize,
    source: &str,
    translated: Option<&str>,
    segment_kind: ChunkSegmentKind,
) -> ChunkState {
    ChunkState {
        index,
        source: source.to_string(),
        translated: translated.map(str::to_string),
        confirmed_terms: Vec::new(),
        attempts: u8::from(translated.is_some()),
        updated_at: "now".to_string(),
        segment_kind,
        processing_stage: None,
    }
}

#[test]
fn static_glossary_hits_are_rendered_in_translation_context() {
    let context = TranslationContext {
        proper_nouns: Vec::new(),
        static_terms: vec![StaticGlossaryEntry {
            source: "deliberation".to_string(),
            target: "审议".to_string(),
            scope: "global".to_string(),
            enforcement: "strict".to_string(),
            notes: String::new(),
        }],
    };

    let rendered = format_translation_context_for_chunk(
        &context,
        "academic",
        2,
        "The deliberation reshaped the outcome.",
    );

    assert!(rendered.contains("Static glossary hits in this chunk:"));
    assert!(rendered.contains("deliberation => MUST USE EXACTLY 审议"));
}

use std::time::Duration;
