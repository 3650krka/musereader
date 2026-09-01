//! AI 阅读辅助命令：段落理解/释义 + 右侧会话问答。
//!
//! 设计要点：
//! - 复用与翻译主链路一致的 BYOK 客户端构造（号池优先，回落单 provider + fallback），
//!   会话不重复消耗全局限流额度以外的资源——仍经 `llm_limiter` 信号量统一管控。
//! - 输出走纯文本（非结构化 JSON），与翻译通道的结构化输出解耦；
//!   prompt 自带"只输出答案正文"约束。
//! - 功能开关：`MUSETRANSLATE_AI_READER=0` 可整体关闭（默认开启）。

use super::{lock_mutex, AppError, AppState};
use crate::llm::{LlmFallbackRoute, SensenovaClient};
use serde::Serialize;

const MAX_SOURCE_CHARS: usize = 4_000;
const MAX_QUESTION_CHARS: usize = 2_000;
const MAX_HISTORY_TURNS: usize = 12;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatTurn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistResponse {
    pub answer: String,
    pub model: String,
}

/// 段落助手单次动作：summarize（理解总结）/ explain（逐句释义）/ translate（重译）。
/// question 仅 ask 动作使用；history 携带会话上下文（旧→新）。
/// AI 释义流式版本：SSE 逐 chunk 经 Channel 推送，
/// 前端弹卡逐字上屏；结束后返回完整文本。流式失败时回落整段请求（on_chunk 不再触发）。
#[tauri::command]
pub async fn ai_assist_paragraph_streaming(
    action: String,
    source_text: String,
    question: Option<String>,
    target_language: Option<String>,
    on_chunk: tauri::ipc::Channel<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AiAssistResponse, AppError> {
    if crate::pipeline::env_flag_disabled("MUSETRANSLATE_AI_READER") {
        return Err(AppError::invalid_input(
            "AI 阅读辅助已关闭（MUSETRANSLATE_AI_READER=0）",
        ));
    }

    let source_text = source_text.trim().to_string();
    if source_text.is_empty() {
        return Err(AppError::invalid_input("段落内容为空"));
    }
    let source_text = truncate_chars(&source_text, MAX_SOURCE_CHARS);
    let target_language = target_language
        .filter(|lang| !lang.trim().is_empty())
        .unwrap_or_else(|| "中文".to_string());

    let prompt = match action.trim().to_ascii_lowercase().as_str() {
        "translate" => build_translate_prompt(&source_text, &target_language),
        "define" => build_define_prompt(&source_text, question.as_deref().unwrap_or("")),
        other => {
            return Err(AppError::invalid_input(format!(
                "流式动作仅支持 translate/define，收到: {other}"
            )))
        }
    };

    let (client, model) = build_ai_reader_client(&state)?;
    let _permit = acquire_llm_permit(&state).await?;

    // 先尝试 SSE 流式；失败（网关不支持/鉴权差异）回落既有整段链路。
    let streamed = client
        .complete_instruction_streaming(&prompt, |delta| {
            let _ = on_chunk.send(delta.to_string());
        })
        .await;
    let answer = match streamed {
        Ok(text) => text,
        Err(stream_error) => {
            let answer = client
                .review_translation_issue(&prompt)
                .await
                .map_err(|fallback_error| {
                    AppError::internal(format!(
                        "AI 流式与整段请求均失败：stream={stream_error:?} fallback={fallback_error:?}"
                    ))
                })?;
            answer
        }
    };
    let answer = sanitize_answer(&action, answer);
    if answer.is_empty() {
        return Err(AppError::internal("AI 阅读辅助返回空内容"));
    }
    Ok(AiAssistResponse { answer, model })
}

#[tauri::command]
pub async fn ai_assist_paragraph(
    action: String,
    source_text: String,
    question: Option<String>,
    target_language: Option<String>,
    history: Option<Vec<AiChatTurn>>,
    state: tauri::State<'_, AppState>,
) -> Result<AiAssistResponse, AppError> {
    if crate::pipeline::env_flag_disabled("MUSETRANSLATE_AI_READER") {
        return Err(AppError::invalid_input(
            "AI 阅读辅助已关闭（MUSETRANSLATE_AI_READER=0）",
        ));
    }

    let source_text = source_text.trim().to_string();
    if source_text.is_empty() {
        return Err(AppError::invalid_input("段落内容为空"));
    }
    let source_text = truncate_chars(&source_text, MAX_SOURCE_CHARS);
    let target_language = target_language
        .filter(|lang| !lang.trim().is_empty())
        .unwrap_or_else(|| "中文".to_string());
    let history = sanitize_history(history.unwrap_or_default());

    let prompt = match action.trim().to_ascii_lowercase().as_str() {
        "summarize" => build_summarize_prompt(&source_text, &target_language),
        "explain" => build_explain_prompt(&source_text, &target_language),
        "translate" => build_translate_prompt(&source_text, &target_language),
        "define" => build_define_prompt(&source_text, question.as_deref().unwrap_or("")),
        "ask" => {
            let question = question
                .map(|q| q.trim().to_string())
                .filter(|q| !q.is_empty())
                .ok_or_else(|| AppError::invalid_input("提问内容为空"))?;
            build_ask_prompt(
                &source_text,
                &truncate_chars(&question, MAX_QUESTION_CHARS),
                &target_language,
                &history,
            )
        }
        other => {
            return Err(AppError::invalid_input(format!(
                "不支持的 AI 阅读动作: {other}（应为 summarize/explain/translate/define/ask）"
            )))
        }
    };

    let (client, model) = build_ai_reader_client(&state)?;
    let _permit = acquire_llm_permit(&state).await?;
    let answer = client
        .review_translation_issue(&prompt)
        .await
        .map_err(|error| AppError::internal(format!("AI 阅读辅助请求失败: {error:?}")))?;
    let answer = sanitize_answer(&action, answer);
    if answer.is_empty() {
        return Err(AppError::internal("AI 阅读辅助返回空内容"));
    }
    Ok(AiAssistResponse { answer, model })
}

/// 输出消毒：define 动作只取第一行（防模型多输出解释过程）；其余取全文 trim。
fn sanitize_answer(action: &str, answer: String) -> String {
    let trimmed = answer.trim();
    if action.trim().eq_ignore_ascii_case("define") {
        return trimmed.lines().next().unwrap_or("").trim().to_string();
    }
    trimmed.to_string()
}

fn build_ai_reader_client(state: &AppState) -> Result<(SensenovaClient, String), AppError> {
    let config = lock_mutex(&state.config, "运行配置")?.clone();
    let model = config.llm_model.clone();
    let protocol = resolve_custom_provider_protocol(&config);
    if let Some(pool) = resolve_active_pool(&config) {
        let client = SensenovaClient::new_with_route_pool(
            config.llm_api_url.clone(),
            config.llm_api_key.clone(),
            config.llm_model.clone(),
            pool,
        )
        .with_protocol(protocol);
        return Ok((client, model));
    }
    let client = SensenovaClient::new_with_fallback_routes(
        config.llm_api_url.clone(),
        config.llm_api_key.clone(),
        config.llm_model.clone(),
        build_fallback_routes(&config),
    )
    .with_protocol(protocol);
    Ok((client, model))
}

/// 当前 llm_api_url 命中的自定义供应商协议。
fn resolve_custom_provider_protocol(config: &super::RuntimeConfig) -> crate::llm::RequestProtocol {
    let url = config.llm_api_url.trim();
    if url.is_empty() {
        return crate::llm::RequestProtocol::Auto;
    }
    config
        .custom_providers
        .iter()
        .find(|provider| provider.api_url.trim() == url)
        .map(|provider| crate::llm::RequestProtocol::from_label(&provider.protocol))
        .unwrap_or(crate::llm::RequestProtocol::Auto)
}

fn resolve_active_pool(config: &super::RuntimeConfig) -> Option<&super::RoutePoolConfig> {
    if config.route_pools.is_empty() {
        return None;
    }
    let active = config.active_pool.trim();
    if !active.is_empty() {
        if let Some(pool) = config.route_pools.iter().find(|pool| pool.name == active) {
            return Some(pool);
        }
    }
    config.route_pools.first()
}

fn build_fallback_routes(config: &super::RuntimeConfig) -> Vec<LlmFallbackRoute> {
    if config.llm_provider.trim().eq_ignore_ascii_case("nvidia") {
        return Vec::new();
    }
    if config.nvidia_api_key.trim().is_empty()
        || config.nvidia_model.trim().is_empty()
        || config.nvidia_api_url.trim().is_empty()
    {
        return Vec::new();
    }
    vec![LlmFallbackRoute {
        api_url: config.nvidia_api_url.clone(),
        api_key: config.nvidia_api_key.clone(),
        model: config.nvidia_model.clone(),
    }]
}

async fn acquire_llm_permit(
    state: &AppState,
) -> Result<tokio::sync::OwnedSemaphorePermit, AppError> {
    state
        .llm_limiter
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::internal("LLM 并发信号量已关闭"))
}

fn sanitize_history(history: Vec<AiChatTurn>) -> Vec<AiChatTurn> {
    let filtered: Vec<AiChatTurn> = history
        .into_iter()
        .filter(|turn| matches!(turn.role.as_str(), "user" | "assistant"))
        .map(|turn| AiChatTurn {
            role: turn.role,
            content: truncate_chars(turn.content.trim(), MAX_SOURCE_CHARS),
        })
        .filter(|turn| !turn.content.is_empty())
        .collect();
    filtered
        .into_iter()
        .rev()
        .take(MAX_HISTORY_TURNS)
        .rev()
        .collect()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

fn build_summarize_prompt(source: &str, target_language: &str) -> String {
    format!(
        "你是一位严谨的阅读助手。请用{target_language}对以下段落做「理解总结」：\n\
         1) 先用一句话概括主旨；\n\
         2) 再列出 2-4 个关键要点（每点一行，以「·」开头）；\n\
         3) 若段落含专业术语或隐喻，补一句通俗解释。\n\
         只输出总结正文，不要复述原文，不要输出任何前后缀说明。\n\n\
         段落原文：\n{source}"
    )
}

fn build_explain_prompt(source: &str, target_language: &str) -> String {
    format!(
        "你是一位耐心的语言教师。请用{target_language}逐句讲解以下段落的含义与语言点：\n\
         - 每句先给原句（可用 > 引用），再给{target_language}释义；\n\
         - 标出值得学习的词汇/搭配并简注；\n\
         - 保持紧凑，总长度不超过原文 2 倍。\n\
         只输出讲解正文。\n\n\
         段落原文：\n{source}"
    )
}

fn build_translate_prompt(source: &str, target_language: &str) -> String {
    format!(
        "请把以下段落翻译为{target_language}。要求：忠实原文、行文流畅、保留原文段落结构；\
         术语前后一致。只输出译文正文，不要输出任何说明。\n\n\
         段落原文：\n{source}"
    )
}

/// 语境释义（AI 词义纠正）：source=语境句，question=目标词。
/// 输出契约极严——单行「词性. 释义」，前端直接存为自定义释义覆盖词表默认释义。
fn build_define_prompt(context: &str, word: &str) -> String {
    let word = word.trim();
    format!(
        "你是一部上下文敏感的双语词典。给定句子和其中的一个英语单词或短语，\
         判断它**在这个句子里**的具体含义（多义词/习语只取语境义），用中文给出极简释义。\n\
         输出格式严格为一行：`词性. 释义`（例如 `v. 放弃；离开` 或 `短语. 着手开始`），不超过 20 个汉字；\
         不要输出音标、例句、解释过程或任何其他内容。\n\n\
         句子：{context}\n目标词：{word}"
    )
}

fn build_ask_prompt(
    source: &str,
    question: &str,
    target_language: &str,
    history: &[AiChatTurn],
) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "你是一位阅读助手，基于以下段落回答用户问题。用{target_language}回答；\
         若问题超出段落内容，可结合常识但需注明「段落未直接提及」。\
         只输出回答正文。\n\n段落原文：\n{source}\n"
    ));
    if !history.is_empty() {
        prompt.push_str("\n对话历史（旧→新）：\n");
        for turn in history {
            let role = if turn.role == "user" {
                "用户"
            } else {
                "助手"
            };
            prompt.push_str(&format!("{role}：{}\n", turn.content));
        }
    }
    prompt.push_str(&format!("\n用户当前问题：{question}"));
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_prompt_embeds_source_and_language() {
        let prompt = build_summarize_prompt("Hello world paragraph.", "中文");
        assert!(prompt.contains("Hello world paragraph."));
        assert!(prompt.contains("中文"));
        assert!(prompt.contains("只输出总结正文"));
    }

    #[test]
    fn ask_prompt_threads_history_in_order() {
        let history = vec![
            AiChatTurn {
                role: "user".to_string(),
                content: "第一段讲了什么".to_string(),
            },
            AiChatTurn {
                role: "assistant".to_string(),
                content: "讲了背景".to_string(),
            },
            AiChatTurn {
                role: "system".to_string(),
                content: "应被过滤".to_string(),
            },
        ];
        let sanitized = sanitize_history(history);
        assert_eq!(sanitized.len(), 2);
        let prompt = build_ask_prompt("para", "为什么？", "中文", &sanitized);
        let first = prompt.find("第一段讲了什么").expect("history user turn");
        let second = prompt.find("为什么？").expect("current question");
        assert!(first < second);
        assert!(!prompt.contains("应被过滤"));
    }

    #[test]
    fn truncate_chars_respects_char_boundary() {
        let text = "汉字".repeat(10);
        let truncated = truncate_chars(&text, 5);
        assert_eq!(truncated.chars().count(), 5);
    }
}
