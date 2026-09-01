use crate::error::AppError;
use crate::skill_library::{
    load_reading_agent_prompt, load_translation_prompt_bundle_with_core, READING_AGENT,
    TRANSLATION_AGENT,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentCapability {
    TranslateDocument,
    ExplainSentence,
    ExplainWord,
    ExplainGrammar,
    ExplainContext,
    AnalyzeReading,
    LinkNotes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: String,
    pub title: String,
    pub description: String,
    pub article_type_required: bool,
    pub default_skill_ids: Vec<String>,
    pub capabilities: Vec<AgentCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptBundle {
    pub agent: String,
    pub article_type: Option<String>,
    pub system_prompt: String,
    pub fallback_prompt: Option<String>,
    pub skill_ids: Vec<String>,
    pub system_prompt_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingTaskKind {
    Word,
    Sentence,
    Grammar,
    Context,
    Analysis,
    Notes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingAgentRequest {
    pub task_kind: ReadingTaskKind,
    pub book_title: Option<String>,
    pub chapter: Option<String>,
    pub excerpt: String,
    pub focus_text: Option<String>,
    pub user_note: Option<String>,
}

pub fn list_agents() -> Vec<AgentDefinition> {
    vec![
        AgentDefinition {
            id: TRANSLATION_AGENT.to_string(),
            title: "翻译".to_string(),
            description: "书籍导入、分块翻译、HTML 生成".to_string(),
            article_type_required: true,
            default_skill_ids: vec![
                "translation:core".to_string(),
                "translation:fallback".to_string(),
            ],
            capabilities: vec![AgentCapability::TranslateDocument],
        },
        AgentDefinition {
            id: READING_AGENT.to_string(),
            title: "阅读学习".to_string(),
            description: "生词、句法、语境、分析、笔记串联".to_string(),
            article_type_required: false,
            default_skill_ids: vec!["reading:core".to_string()],
            capabilities: vec![
                AgentCapability::ExplainWord,
                AgentCapability::ExplainSentence,
                AgentCapability::ExplainGrammar,
                AgentCapability::ExplainContext,
                AgentCapability::AnalyzeReading,
                AgentCapability::LinkNotes,
            ],
        },
    ]
}

pub fn preview_prompt_bundle(
    skill_root: &Path,
    agent: &str,
    article_type: Option<&str>,
    request: Option<&ReadingAgentRequest>,
    inject_core_skill: bool,
) -> Result<AgentPromptBundle, AppError> {
    match agent {
        TRANSLATION_AGENT => {
            let bundle = load_translation_prompt_bundle_with_core(
                skill_root,
                article_type.unwrap_or("academic"),
                inject_core_skill,
            )?;
            Ok(AgentPromptBundle {
                agent: bundle.agent,
                article_type: Some(bundle.article_type),
                system_prompt: bundle.system_prompt,
                fallback_prompt: Some(bundle.fallback_prompt),
                skill_ids: bundle.skill_ids,
                system_prompt_hash: bundle.system_prompt_hash,
            })
        }
        READING_AGENT => {
            let core_prompt = load_reading_agent_prompt(skill_root)?;
            let system_prompt = compose_reading_prompt(&core_prompt, request);
            Ok(AgentPromptBundle {
                agent: READING_AGENT.to_string(),
                article_type: None,
                fallback_prompt: None,
                skill_ids: vec!["reading:core".to_string()],
                system_prompt_hash: stable_hash_hex(system_prompt.as_bytes()),
                system_prompt,
            })
        }
        _ => Err(AppError::invalid_input("不支持的 agent")),
    }
}

fn compose_reading_prompt(core_prompt: &str, request: Option<&ReadingAgentRequest>) -> String {
    let mut sections = vec![
        "# 阅读 Agent 系统提示".to_string(),
        String::new(),
        core_prompt.trim().to_string(),
    ];

    if let Some(request) = request {
        append_reading_request_sections(&mut sections, request);
    }

    sections.join("\n")
}

fn append_reading_request_sections(sections: &mut Vec<String>, request: &ReadingAgentRequest) {
    sections.push(String::new());
    sections.push("## 当前任务".to_string());
    sections.push(format!(
        "任务类型: {}",
        reading_task_kind_label(&request.task_kind)
    ));
    push_optional_field(sections, "书名", request.book_title.as_deref());
    push_optional_field(sections, "章节", request.chapter.as_deref());
    push_optional_field(sections, "聚焦文本", request.focus_text.as_deref());
    push_optional_field(sections, "用户备注", request.user_note.as_deref());
    sections.push(String::new());
    sections.push("## 当前片段".to_string());
    sections.push(request.excerpt.trim().to_string());
}

fn push_optional_field(sections: &mut Vec<String>, label: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    sections.push(format!("{label}: {value}"));
}

fn reading_task_kind_label(kind: &ReadingTaskKind) -> &'static str {
    match kind {
        ReadingTaskKind::Word => "生词学习",
        ReadingTaskKind::Sentence => "句子精讲",
        ReadingTaskKind::Grammar => "句法分析",
        ReadingTaskKind::Context => "语境理解",
        ReadingTaskKind::Analysis => "阅读分析",
        ReadingTaskKind::Notes => "笔记串联",
    }
}

fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
