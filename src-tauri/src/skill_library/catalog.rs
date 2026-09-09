use super::{
    prompt::{extract_description, extract_title},
    SkillEntryKind, SkillLibraryEntry, READING_AGENT, TRANSLATION_AGENT,
};
use crate::error::AppError;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

/// 默认提示词根目录解析。历史上只认 `CARGO_MANIFEST_DIR` 的上两级（仓库外兄弟目录），
/// 那在编译期被烘焙进二进制：除开发机外任何安装环境都不存在，技能库播种会整体失败。
/// 现按“显式覆盖 → 安装包内资源 → 仓库内资源 → 旧开发路径”顺序取第一个存在者。
pub(super) fn resolve_default_prompts_root() -> Result<PathBuf, AppError> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(override_dir) = std::env::var("MUSEREADER_PROMPTS_DIR") {
        if !override_dir.trim().is_empty() {
            candidates.push(PathBuf::from(override_dir));
        }
    }

    // 发布安装：Tauri 将 bundle.resources 释放在可执行文件旁的 resources/ 下。
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("resources").join("prompts"));
            candidates.push(exe_dir.join("prompts"));
        }
    }

    // 仓库内资源：开发与 CI 直接命中，不再依赖工作区外部目录。
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("resources").join("prompts"));

    // 兼容旧开发布局（click/prompts 为仓库兄弟目录）。
    if let Some(click_root) = manifest_dir.parent().and_then(Path::parent) {
        candidates.push(click_root.join("prompts"));
    }

    for candidate in &candidates {
        if candidate.is_dir() {
            return Ok(candidate.clone());
        }
    }

    Err(AppError::not_found(format!(
        "缺少默认 prompts 目录（已尝试：{}）",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

pub(super) fn parse_skill_id(
    skill_id: &str,
) -> Result<(String, SkillEntryKind, Option<String>), AppError> {
    let parts = skill_id.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["translation", "core"] => Ok((TRANSLATION_AGENT.to_string(), SkillEntryKind::Core, None)),
        ["translation", "fallback"] => Ok((
            TRANSLATION_AGENT.to_string(),
            SkillEntryKind::Fallback,
            None,
        )),
        ["translation", "reference", article_type] => Ok((
            TRANSLATION_AGENT.to_string(),
            SkillEntryKind::Reference,
            Some((*article_type).to_string()),
        )),
        ["reading", "core"] => Ok((READING_AGENT.to_string(), SkillEntryKind::Core, None)),
        _ => Err(AppError::invalid_input(format!(
            "未知 skill id: {skill_id}"
        ))),
    }
}

pub(super) fn load_entry(
    skill_root: &Path,
    agent: &str,
    kind: SkillEntryKind,
    article_type: Option<String>,
) -> Result<SkillLibraryEntry, AppError> {
    let path = skill_file_path(skill_root, agent, &kind, article_type.as_deref());
    let content = fs::read_to_string(&path).map_err(|error| {
        AppError::internal(format!("读取 skill 失败({}): {error}", path.display()))
    })?;
    let metadata = fs::metadata(&path).map_err(|error| {
        AppError::internal(format!(
            "读取 skill 元数据失败({}): {error}",
            path.display()
        ))
    })?;

    Ok(SkillLibraryEntry {
        id: skill_id(agent, &kind, article_type.as_deref()),
        agent: agent.to_string(),
        kind,
        article_type,
        title: extract_title(&content),
        description: extract_description(&content),
        content,
        file_path: path.to_string_lossy().to_string(),
        updated_at: metadata
            .modified()
            .ok()
            .and_then(|time| {
                chrono::DateTime::<Utc>::from_timestamp_millis(
                    time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64,
                )
            })
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
    })
}

pub(super) fn skill_file_path(
    skill_root: &Path,
    agent: &str,
    kind: &SkillEntryKind,
    article_type: Option<&str>,
) -> PathBuf {
    match (agent, kind, article_type) {
        (TRANSLATION_AGENT, SkillEntryKind::Core, _) => {
            skill_root.join("translation").join("core.md")
        }
        (TRANSLATION_AGENT, SkillEntryKind::Fallback, _) => {
            skill_root.join("translation").join("fallback.md")
        }
        (TRANSLATION_AGENT, SkillEntryKind::Reference, Some(article_type)) => skill_root
            .join("translation")
            .join("references")
            .join(format!("{article_type}.md")),
        (READING_AGENT, SkillEntryKind::Core, _) => skill_root.join("reading").join("core.md"),
        _ => skill_root.join("invalid-skill.md"),
    }
}

fn skill_id(agent: &str, kind: &SkillEntryKind, article_type: Option<&str>) -> String {
    match (agent, kind, article_type) {
        (TRANSLATION_AGENT, SkillEntryKind::Core, _) => "translation:core".to_string(),
        (TRANSLATION_AGENT, SkillEntryKind::Fallback, _) => "translation:fallback".to_string(),
        (TRANSLATION_AGENT, SkillEntryKind::Reference, Some(article_type)) => {
            format!("translation:reference:{article_type}")
        }
        (READING_AGENT, SkillEntryKind::Core, _) => "reading:core".to_string(),
        _ => "unknown".to_string(),
    }
}
