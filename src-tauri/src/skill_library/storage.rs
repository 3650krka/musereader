use super::{
    catalog::skill_file_path, SkillEntryKind, ARTICLE_TYPES, READING_AGENT, READING_CORE_SKILL,
    TRANSLATION_AGENT, TRANSLATION_FALLBACK_SKILL,
};
use crate::error::AppError;
use std::fs;
use std::path::Path;

pub(super) fn seed_translation_skills(
    skill_root: &Path,
    prompts_root: &Path,
) -> Result<(), AppError> {
    let target_core = skill_file_path(skill_root, TRANSLATION_AGENT, &SkillEntryKind::Core, None);
    seed_file_if_missing(&target_core, &prompts_root.join("SKILL.md"))?;

    let target_fallback = skill_file_path(
        skill_root,
        TRANSLATION_AGENT,
        &SkillEntryKind::Fallback,
        None,
    );
    seed_inline_if_missing(&target_fallback, TRANSLATION_FALLBACK_SKILL)?;

    for article_type in ARTICLE_TYPES {
        let source = prompts_root
            .join("references")
            .join(format!("{article_type}.md"));
        let target = skill_file_path(
            skill_root,
            TRANSLATION_AGENT,
            &SkillEntryKind::Reference,
            Some(article_type),
        );
        seed_file_if_missing(&target, &source)?;
    }
    Ok(())
}

pub(super) fn seed_reading_skills(skill_root: &Path) -> Result<(), AppError> {
    let target = skill_file_path(skill_root, READING_AGENT, &SkillEntryKind::Core, None);
    seed_inline_if_missing(&target, READING_CORE_SKILL)
}

fn seed_file_if_missing(target: &Path, source: &Path) -> Result<(), AppError> {
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| AppError::internal(format!("创建 skill 目录失败: {error}")))?;
    }
    let content = fs::read_to_string(source).map_err(|error| {
        AppError::internal(format!(
            "读取默认 skill 失败({}): {error}",
            source.display()
        ))
    })?;
    fs::write(target, content.as_bytes()).map_err(|error| {
        AppError::internal(format!(
            "写入默认 skill 失败({}): {error}",
            target.display()
        ))
    })
}

fn seed_inline_if_missing(target: &Path, content: &str) -> Result<(), AppError> {
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| AppError::internal(format!("创建 skill 目录失败: {error}")))?;
    }
    fs::write(target, content.as_bytes()).map_err(|error| {
        AppError::internal(format!(
            "写入默认 skill 失败({}): {error}",
            target.display()
        ))
    })
}
