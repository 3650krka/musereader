use super::{AppError, AppState};
use crate::agent_registry::{
    list_agents, preview_prompt_bundle, AgentDefinition, ReadingAgentRequest, ReadingTaskKind,
};
use crate::skill_library::{
    create_reference_entry as create_skill_reference, delete_entry as delete_skill_entry_impl,
    get_entry as get_skill_entry, list_entries as list_skill_entries,
    update_entry as update_skill_entry, SkillLibraryEntry,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptPreview {
    pub agent: String,
    pub article_type: Option<String>,
    pub system_prompt: String,
    pub fallback_prompt: Option<String>,
    pub skill_ids: Vec<String>,
    pub system_prompt_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingAgentPromptPreviewRequest {
    pub task_kind: String,
    pub book_title: Option<String>,
    pub chapter: Option<String>,
    pub excerpt: String,
    pub focus_text: Option<String>,
    pub user_note: Option<String>,
}

#[tauri::command]
pub async fn list_skill_library_entries(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SkillLibraryEntry>, AppError> {
    list_skill_entries(&state.skill_library_root_dir)
}

#[tauri::command]
pub async fn list_agents_registry() -> Result<Vec<AgentDefinition>, AppError> {
    Ok(list_agents())
}

#[tauri::command]
pub async fn get_skill_library_entry(
    skill_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<SkillLibraryEntry, AppError> {
    get_skill_entry(&state.skill_library_root_dir, &skill_id)
}

#[tauri::command]
pub async fn update_skill_library_entry(
    skill_id: String,
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<SkillLibraryEntry, AppError> {
    update_skill_entry(&state.skill_library_root_dir, &skill_id, &content)
}

/// 新增类型专用 prompt。
#[tauri::command]
pub async fn create_skill_entry(
    name: String,
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<SkillLibraryEntry, AppError> {
    create_skill_reference(&state.skill_library_root_dir, &name, &content)
}

/// 删除类型专用 prompt（系统内置项删除后 seed 还原出厂默认）。
#[tauri::command]
pub async fn delete_skill_entry(
    skill_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    delete_skill_entry_impl(&state.skill_library_root_dir, &skill_id)
}

#[tauri::command]
pub async fn preview_agent_prompt(
    agent: String,
    article_type: Option<String>,
    reading_request: Option<ReadingAgentPromptPreviewRequest>,
    state: tauri::State<'_, AppState>,
) -> Result<AgentPromptPreview, AppError> {
    let parsed_request = reading_request
        .map(parse_reading_preview_request)
        .transpose()?;
    let inject_core_skill = crate::commands::lock_mutex(&state.config, "运行配置")?.inject_core_skill;
    let bundle = preview_prompt_bundle(
        &state.skill_library_root_dir,
        &agent,
        article_type.as_deref(),
        parsed_request.as_ref(),
        inject_core_skill,
    )?;
    Ok(AgentPromptPreview {
        agent: bundle.agent,
        article_type: bundle.article_type,
        system_prompt: bundle.system_prompt,
        fallback_prompt: bundle.fallback_prompt,
        skill_ids: bundle.skill_ids,
        system_prompt_hash: bundle.system_prompt_hash,
    })
}

fn parse_reading_preview_request(
    request: ReadingAgentPromptPreviewRequest,
) -> Result<ReadingAgentRequest, AppError> {
    if request.excerpt.trim().is_empty() {
        return Err(AppError::invalid_input("阅读片段不能为空"));
    }

    Ok(ReadingAgentRequest {
        task_kind: parse_task_kind(&request.task_kind)?,
        book_title: request.book_title,
        chapter: request.chapter,
        excerpt: request.excerpt,
        focus_text: request.focus_text,
        user_note: request.user_note,
    })
}

fn parse_task_kind(value: &str) -> Result<ReadingTaskKind, AppError> {
    match value {
        "word" => Ok(ReadingTaskKind::Word),
        "sentence" => Ok(ReadingTaskKind::Sentence),
        "grammar" => Ok(ReadingTaskKind::Grammar),
        "context" => Ok(ReadingTaskKind::Context),
        "analysis" => Ok(ReadingTaskKind::Analysis),
        "notes" => Ok(ReadingTaskKind::Notes),
        _ => Err(AppError::invalid_input("不支持的阅读任务类型")),
    }
}
