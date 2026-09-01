use super::AppError;
use crate::tts::{EdgeTtsClient, TtsSynthesizeResult, TtsVoice};

/// 列出 Edge TTS 可用语音（BYOK TTS 默认 provider）。
#[tauri::command]
pub async fn list_tts_voices() -> Result<Vec<TtsVoice>, AppError> {
    EdgeTtsClient::new().list_voices().await
}

/// 合成文本为 MP3 音频（base64）。voice 如 "zh-CN-XiaoxiaoNeural"，
/// rate 如 "+0%"/"+20%"/"-10%"（None 用 Edge 默认）。
#[tauri::command]
pub async fn synthesize_tts(
    text: String,
    voice: String,
    rate: Option<String>,
) -> Result<TtsSynthesizeResult, AppError> {
    if text.trim().is_empty() {
        return Err(AppError::invalid_input("TTS 文本为空"));
    }
    if voice.trim().is_empty() {
        return Err(AppError::invalid_input("TTS 语音为空"));
    }
    EdgeTtsClient::new()
        .synthesize(&text, &voice, rate.as_deref())
        .await
}
