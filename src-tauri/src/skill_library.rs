use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use catalog::{load_entry, parse_skill_id, resolve_default_prompts_root, skill_file_path};
use prompt::{build_translation_system_prompt, remove_frontmatter, stable_hash_hex};
use storage::{seed_reading_skills, seed_translation_skills};

mod catalog;
mod prompt;
mod storage;

pub const TRANSLATION_AGENT: &str = "translation";
pub const READING_AGENT: &str = "reading";

const ARTICLE_TYPES: &[&str] = &[
    "academic",
    "fiction",
    "nonfiction",
    "textbook",
    "business",
    "children",
    "blog",
    "news",
    "tech_doc",
    "legal",
    "general",
];

const READING_CORE_SKILL: &str = r#"# 阅读 Agent 核心规则

## 目标
- 服务阅读，不抢夺阅读
- 优先帮助用户理解文本、记住文本、回到文本
- 所有输出必须可回溯到当前书籍、章节、句段或用户笔记

## 行为约束
- 不擅自扩写世界观，不虚构原文没有的信息
- 解释以短句和分层结构为主，避免大段说教
- 在词汇、句法、语境、主题、结构之间切换时，明确当前层级
- 当信息不足时，优先提示“基于当前片段推断”

## 默认能力
- 句子精讲
- 生词学习
- 语法拆解
- 语境释义
- 阅读分析
- 笔记串联
"#;

const TRANSLATION_FALLBACK_SKILL: &str = r#"你是 Markdown 漏翻修复助手。
把输入段落翻译成自然、准确、简洁的中文。
保留 Markdown 结构、HTML 标签、公式、表格、编号、引用格式。
只返回修复后的段落，不要解释。"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillEntryKind {
    Core,
    Reference,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLibraryEntry {
    pub id: String,
    pub agent: String,
    pub kind: SkillEntryKind,
    pub article_type: Option<String>,
    pub title: String,
    pub description: String,
    pub content: String,
    pub file_path: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationPromptBundle {
    pub agent: String,
    pub article_type: String,
    pub system_prompt: String,
    pub core_prompt: String,
    pub reference_prompt: String,
    pub runtime_prompt: String,
    pub fallback_prompt: String,
    pub skill_ids: Vec<String>,
    pub system_prompt_hash: String,
}

pub fn supported_article_types() -> &'static [&'static str] {
    ARTICLE_TYPES
}

pub fn ensure_seeded(skill_root: &Path) -> Result<(), AppError> {
    fs::create_dir_all(skill_root)
        .map_err(|error| AppError::internal(format!("创建 skill 目录失败: {error}")))?;

    let default_prompts_root = resolve_default_prompts_root()?;
    seed_translation_skills(skill_root, &default_prompts_root)?;
    seed_reading_skills(skill_root)?;
    Ok(())
}

pub fn list_entries(skill_root: &Path) -> Result<Vec<SkillLibraryEntry>, AppError> {
    ensure_seeded(skill_root)?;
    let mut entries = Vec::new();
    entries.push(load_entry(
        skill_root,
        TRANSLATION_AGENT,
        SkillEntryKind::Core,
        None,
    )?);
    entries.push(load_entry(
        skill_root,
        TRANSLATION_AGENT,
        SkillEntryKind::Fallback,
        None,
    )?);
    // 类型专用 prompt：以 references/ 目录实际文件为准
    let references_dir = skill_root.join("translation").join("references");
    let mut reference_types: Vec<String> = if references_dir.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(&references_dir)
            .map(|dir| {
                dir.filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().to_string();
                        name.strip_suffix(".md").map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    } else {
        Vec::new()
    };
    if reference_types.is_empty() {
        reference_types = ARTICLE_TYPES.iter().map(|t| (*t).to_string()).collect();
    }
    for article_type in &reference_types {
        entries.push(load_entry(
            skill_root,
            TRANSLATION_AGENT,
            SkillEntryKind::Reference,
            Some(article_type.clone()),
        )?);
    }
    entries.push(load_entry(
        skill_root,
        READING_AGENT,
        SkillEntryKind::Core,
        None,
    )?);
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(entries)
}

pub fn get_entry(skill_root: &Path, skill_id: &str) -> Result<SkillLibraryEntry, AppError> {
    ensure_seeded(skill_root)?;
    let (agent, kind, article_type) = parse_skill_id(skill_id)?;
    load_entry(skill_root, &agent, kind, article_type)
}

pub fn update_entry(
    skill_root: &Path,
    skill_id: &str,
    content: &str,
) -> Result<SkillLibraryEntry, AppError> {
    ensure_seeded(skill_root)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("skill 内容不能为空"));
    }
    let (agent, kind, article_type) = parse_skill_id(skill_id)?;
    let path = skill_file_path(skill_root, &agent, &kind, article_type.as_deref());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| AppError::internal(format!("创建 skill 子目录失败: {error}")))?;
    }
    fs::write(&path, trimmed.as_bytes())
        .map_err(|error| AppError::internal(format!("写入 skill 文件失败: {error}")))?;
    load_entry(skill_root, &agent, kind, article_type)
}

/// 测试便捷包装：默认注入核心翻译 skill。
#[cfg(test)]
pub fn load_translation_prompt_bundle(
    skill_root: &Path,
    article_type: &str,
) -> Result<TranslationPromptBundle, AppError> {
    load_translation_prompt_bundle_with_core(skill_root, article_type, true)
}

/// inject_core_skill=false 时跳过核心翻译 skill（全景翻译 skill），
/// 仅保留文章类型专用规则 + runtime 约束；类型 prompt 始终注入。
pub fn load_translation_prompt_bundle_with_core(
    skill_root: &Path,
    article_type: &str,
    inject_core_skill: bool,
) -> Result<TranslationPromptBundle, AppError> {
    validate_article_type(article_type)?;
    ensure_seeded(skill_root)?;

    let core = load_entry(skill_root, TRANSLATION_AGENT, SkillEntryKind::Core, None)?;
    let reference = load_entry(
        skill_root,
        TRANSLATION_AGENT,
        SkillEntryKind::Reference,
        Some(article_type.to_string()),
    )?;
    let fallback = load_entry(
        skill_root,
        TRANSLATION_AGENT,
        SkillEntryKind::Fallback,
        None,
    )?;

    let core_prompt = if inject_core_skill {
        remove_frontmatter(&core.content)
    } else {
        String::new()
    };
    let reference_prompt = reference.content.trim().to_string();
    let runtime_prompt = append_runtime_translation_rules(
        build_translation_system_prompt(article_type, &core_prompt, &reference_prompt),
        article_type,
    );
    let system_prompt = runtime_prompt.clone();

    let mut skill_ids = vec![reference.id, fallback.id];
    if inject_core_skill {
        skill_ids.insert(0, core.id);
    }

    Ok(TranslationPromptBundle {
        agent: TRANSLATION_AGENT.to_string(),
        article_type: article_type.to_string(),
        system_prompt_hash: stable_hash_hex(system_prompt.as_bytes()),
        system_prompt,
        core_prompt,
        reference_prompt,
        runtime_prompt,
        fallback_prompt: fallback.content.trim().to_string(),
        skill_ids,
    })
}

fn append_runtime_translation_rules(mut prompt: String, article_type: &str) -> String {
    let rules = crate::translation_validation::runtime_translation_rules(article_type);
    let Some(main_rules) = rules.main_translation else {
        return prompt;
    };
    prompt.push_str("\n\n## Runtime Translation Rules\n\n");
    prompt.push_str(main_rules);
    if let Some(confirmed_terms) = rules.confirmed_terms {
        prompt.push_str("\n");
        prompt.push_str(confirmed_terms);
    }
    prompt
}

/// 新增类型专用 prompt：slug 化名称写入 references/{name}.md。
pub fn create_reference_entry(
    skill_root: &Path,
    name: &str,
    content: &str,
) -> Result<SkillLibraryEntry, AppError> {
    ensure_seeded(skill_root)?;
    let slug: String = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        return Err(AppError::invalid_input("prompt 名称为空"));
    }
    if slug.len() > 40 {
        return Err(AppError::invalid_input("prompt 名称过长（限 40 字符）"));
    }
    let path = skill_file_path(skill_root, TRANSLATION_AGENT, &SkillEntryKind::Reference, Some(slug));
    if path.exists() {
        return Err(AppError::conflict("同名 prompt 已存在"));
    }
    let content = if content.trim().is_empty() {
        format!("# {}

（在此编写该类型的翻译规则与风格约束）", slug)
    } else {
        content.trim().to_string()
    };
    fs::write(&path, content.as_bytes())
        .map_err(|error| AppError::internal(format!("写入 prompt 失败: {error}")))?;
    load_entry(skill_root, TRANSLATION_AGENT, SkillEntryKind::Reference, Some(slug.to_string()))
}

/// 删除 prompt/skill：系统内置项删除后由 seed 机制还原为出厂默认，
/// 用户自建项直接移除。
pub fn delete_entry(skill_root: &Path, skill_id: &str) -> Result<(), AppError> {
    ensure_seeded(skill_root)?;
    let (agent, kind, article_type) = parse_skill_id(skill_id)?;
    if matches!(kind, SkillEntryKind::Core | SkillEntryKind::Fallback) {
        return Err(AppError::invalid_input(
            "核心/回退 skill 不可删除（仅可编辑；删除需求请用重置）",
        ));
    }
    let path = skill_file_path(skill_root, &agent, &kind, article_type.as_deref());
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| AppError::internal(format!("删除 prompt 失败: {error}")))?;
    }
    Ok(())
}

pub fn load_reading_agent_prompt(skill_root: &Path) -> Result<String, AppError> {
    ensure_seeded(skill_root)?;
    let entry = load_entry(skill_root, READING_AGENT, SkillEntryKind::Core, None)?;
    Ok(entry.content)
}

fn validate_article_type(article_type: &str) -> Result<(), AppError> {
    if ARTICLE_TYPES.contains(&article_type) {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!(
            "不支持的文章类型: {article_type}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn strips_frontmatter_from_skill_body() {
        let input = "---\nname: demo\n---\n# Title\n\nBody";
        assert_eq!(remove_frontmatter(input), "# Title\n\nBody");
    }

    #[test]
    fn builds_translation_prompt_bundle_from_seeded_library() {
        let unique = format!(
            "musetranslate-skill-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        ensure_seeded(&root).expect("seed should succeed");

        let bundle = load_translation_prompt_bundle(&root, "fiction").expect("bundle should load");

        assert_eq!(bundle.agent, TRANSLATION_AGENT);
        assert_eq!(bundle.article_type, "fiction");
        assert!(bundle.system_prompt.contains("核心翻译 Skill"));
        assert!(bundle.system_prompt.contains("小说翻译提示"));
        assert!(bundle
            .skill_ids
            .iter()
            .any(|id| id == "translation:reference:fiction"));
        assert!(bundle.system_prompt.contains("Runtime Translation Rules"));
        assert!(bundle.system_prompt.contains("invented creature names"));
        assert!(bundle
            .system_prompt
            .contains("do not keep the original English in parentheses"));
        assert!(!bundle.system_prompt_hash.is_empty());
    }

    #[test]
    fn bundle_without_core_skill_drops_core_id_and_content() {
        let unique = format!(
            "musetranslate-skill-test-{}-nocore",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        ensure_seeded(&root).expect("seed should succeed");

        let bundle = load_translation_prompt_bundle_with_core(&root, "fiction", false)
            .expect("bundle should load");
        assert!(!bundle.skill_ids.iter().any(|id| id == "translation:core"));
        assert!(bundle.skill_ids.iter().any(|id| id == "translation:reference:fiction"));
        assert!(bundle.core_prompt.is_empty());

        let with_core = load_translation_prompt_bundle(&root, "fiction").expect("bundle");
        assert!(with_core.skill_ids.iter().any(|id| id == "translation:core"));
        // 两种 bundle 的 hash 必须不同（core 内容参与哈希）
        assert_ne!(bundle.system_prompt_hash, with_core.system_prompt_hash);
    }

    #[test]
    fn academic_translation_prompt_does_not_include_fiction_runtime_rules() {
        let unique = format!(
            "musetranslate-skill-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let root = std::env::temp_dir().join(unique);
        ensure_seeded(&root).expect("seed should succeed");

        let bundle = load_translation_prompt_bundle(&root, "academic").expect("bundle should load");

        assert!(!bundle.system_prompt.contains("invented creature names"));
        assert!(!bundle.system_prompt.contains("Runtime Translation Rules"));
    }
}
