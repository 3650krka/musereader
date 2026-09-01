use super::*;
use chrono::Utc;

pub(crate) fn build_manifest(
    config: &PipelineRunConfig,
    runtime_prompt: &RuntimePromptConfig,
    paths: &ArtifactPaths,
    checkpoint: &ChunkCheckpoint,
    validation_report: &ValidationReport,
    glossary: &GlossaryArtifact,
) -> Result<ArtifactManifest, AppError> {
    let now = Utc::now().to_rfc3339();
    let translated_chunks = checkpoint
        .chunks
        .iter()
        .filter(|chunk| chunk.translated.as_ref().is_some())
        .count();

    Ok(ArtifactManifest {
        version: 1,
        task_id: config.task_id.clone(),
        source_pdf_path: config.pdf_path.to_string_lossy().to_string(),
        article_type: runtime_prompt.article_type.clone(),
        system_prompt_hash: runtime_prompt.system_prompt_hash.clone(),
        skill_ids: checkpoint.skill_ids.clone(),
        translation_context: checkpoint.translation_context.clone(),
        glossary: Some(glossary.clone()),
        source_hash: checkpoint.source_hash.clone(),
        created_at: checkpoint.created_at.clone(),
        updated_at: now,
        total_chunks: checkpoint.chunks.len(),
        translated_chunks,
        validation_summary: validation_report.summary.clone(),
        files: vec![
            build_artifact_file("sourceMarkdown", &paths.source_markdown_path)?,
            build_optional_artifact_file(
                "sourceRawOcrMarkdown",
                &paths.source_raw_ocr_markdown_path,
            )?,
            build_optional_artifact_file(
                "sourceJsonRebuiltMarkdown",
                &paths.source_json_rebuilt_markdown_path,
            )?,
            build_optional_artifact_file(
                "sourceMdCleanupMarkdown",
                &paths.source_md_cleanup_markdown_path,
            )?,
            build_optional_artifact_file("sourceOcrLayoutRaw", &paths.source_ocr_layout_raw_path)?,
            build_optional_artifact_file(
                "sourceOcrStructureRaw",
                &paths.source_ocr_structure_raw_path,
            )?,
            build_optional_artifact_file("sourceLayouts", &paths.source_layouts_path)?,
            build_optional_artifact_file("sourceToc", &paths.source_toc_path)?,
            build_optional_artifact_file("sourceBlocks", &paths.source_blocks_path)?,
            build_optional_artifact_file("sourceImagesMap", &paths.source_images_map_path)?,
            build_artifact_file("translatedMarkdown", &paths.output_markdown_path)?,
            build_artifact_file("translatedHtml", &paths.output_html_path)?,
            build_optional_artifact_file("translatedEpub", &paths.translated_epub_path)?,
            build_optional_artifact_file("bilingualEpub", &paths.bilingual_epub_path)?,
            build_optional_artifact_file("translatedChunks", &paths.translated_chunks_path)?,
            build_optional_artifact_file("translatedBlocks", &paths.translated_blocks_path)?,
            build_optional_artifact_file("translatedToc", &paths.translated_toc_path)?,
            build_artifact_file("checkpoint", &paths.checkpoint_path)?,
            build_artifact_file("eventLog", &paths.event_log_path)?,
            build_artifact_file("validationReport", &paths.validation_report_path)?,
            build_artifact_file("glossary", &paths.glossary_path)?,
            build_artifact_file("metrics", &paths.metrics_path)?,
            build_optional_artifact_file("llmRouteProfiles", &paths.llm_route_profile_path)?,
        ]
        .into_iter()
        .flatten()
        .collect(),
    })
}

fn build_artifact_file(kind: &str, path: &Path) -> Result<Option<ArtifactFile>, AppError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::internal(format!(
            "read artifact metadata failed({}): {error}",
            path.display()
        ))
    })?;
    Ok(Some(ArtifactFile {
        kind: kind.to_string(),
        path: path.to_string_lossy().to_string(),
        bytes: metadata.len(),
    }))
}

fn build_optional_artifact_file(kind: &str, path: &Path) -> Result<Option<ArtifactFile>, AppError> {
    if !path.exists() {
        return Ok(None);
    }
    build_artifact_file(kind, path)
}
