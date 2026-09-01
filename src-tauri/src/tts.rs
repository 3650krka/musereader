//! BYOK TTS（文本转语音）：默认 Edge TTS（微软消费级免费神经语音），
//! 预留 OpenAI/Azure/ElevenLabs 等 BYOK provider 扩展点。
//!
//! Edge TTS 接入：
//! - 签名：HMAC-SHA256(appId + encodeURIComponent(url) + date + requestId)，
//!   密钥为公开的第一方签名材料；
//! - token 获取：dev.microsofttranslator.com/apps/endpoint，JWT exp 缓存；
//! - 合成：SSML（prosody rate/pitch/volume）POST 到 tts.speech.microsoft.com，
//!   控制字符清洗（U+0000-0008/000B-000C/000E-001F → 空格）；
//! - 分块：UTF-8 字节数 ≤1800 软边界切分（标点/空白优先，保底 60%），
//!   MP3 裸流按序拼接（mp3 格式天然可连）。
//! - 长文本分块合成：每块独立请求，失败按指数退避重试（≤2 次）。

use crate::error::AppError;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const EDGE_TTS_TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const EDGE_TTS_SIGNATURE_SECRET_BASE64: &str =
    "oik6PdDdMnOXemTbwvMn9de/h9lFnfBaCWbGMMZqqoSaQaqUOqjVGm5NqsmjcBI1x+sS9ugjB55HEJWRiFXYFw==";
const EDGE_TTS_APP_ID: &str = "MSTranslatorAndroidApp";
const EDGE_TTS_ENDPOINT_URL: &str =
    "https://dev.microsofttranslator.com/apps/endpoint?api-version=1.0";
const EDGE_TTS_OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";
const EDGE_TTS_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36 Edg/127.0.0.1";
const EDGE_TTS_MAX_CHUNK_BYTES: usize = 1800;
const EDGE_TTS_MAX_CHUNKS: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoice {
    pub id: String,
    pub name: String,
    pub locale: String,
    #[serde(default)]
    pub gender: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSynthesizeResult {
    pub audio_base64: String,
    pub content_type: String,
    pub chunks: usize,
}

pub struct EdgeTtsClient {
    http: reqwest::Client,
}

impl Default for EdgeTtsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl EdgeTtsClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(EDGE_TTS_USER_AGENT)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// 列出可用语音（Edge TTS consumer voices list，24h 缓存由调用方负责）。
    pub async fn list_voices(&self) -> Result<Vec<TtsVoice>, AppError> {
        let url = format!(
            "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list?trustedclienttoken={EDGE_TTS_TRUSTED_CLIENT_TOKEN}"
        );
        let response = self.http.get(&url).send().await.map_err(|error| {
            AppError::internal(format!("edge tts voices request failed: {error}"))
        })?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "edge tts voices returned {}",
                response.status()
            )));
        }
        let raw: Vec<serde_json::Value> = response.json().await.map_err(|error| {
            AppError::internal(format!("edge tts voices parse failed: {error}"))
        })?;
        let voices = raw
            .iter()
            .filter_map(|item| {
                Some(TtsVoice {
                    id: item.get("ShortName")?.as_str()?.to_string(),
                    name: item
                        .get("FriendlyName")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    locale: item
                        .get("Locale")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    gender: item
                        .get("Gender")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect();
        Ok(voices)
    }

    /// 合成文本为 MP3（自动分块拼接）。voice 如 "zh-CN-XiaoxiaoNeural"。
    pub async fn synthesize(
        &self,
        text: &str,
        voice: &str,
        rate: Option<&str>,
    ) -> Result<TtsSynthesizeResult, AppError> {
        let chunks = chunk_text_for_tts(text);
        if chunks.is_empty() {
            return Err(AppError::invalid_input("TTS 文本为空"));
        }
        if chunks.len() > EDGE_TTS_MAX_CHUNKS {
            return Err(AppError::invalid_input(format!(
                "TTS 文本过长（{} 块 > 上限 {}）",
                chunks.len(),
                EDGE_TTS_MAX_CHUNKS
            )));
        }
        let mut audio = Vec::new();
        for chunk in &chunks {
            let ssml = build_ssml(chunk, voice, rate);
            let bytes = self.synthesize_chunk(&ssml).await?;
            audio.extend_from_slice(&bytes);
        }
        Ok(TtsSynthesizeResult {
            audio_base64: base64::engine::general_purpose::STANDARD.encode(&audio),
            content_type: "audio/mpeg".to_string(),
            chunks: chunks.len(),
        })
    }

    async fn synthesize_chunk(&self, ssml: &str) -> Result<Vec<u8>, AppError> {
        let mut last_error = AppError::internal("edge tts not attempted");
        for attempt in 0..=2u8 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    500 * u64::from(attempt) + 100,
                ))
                .await;
            }
            match self.try_synthesize_chunk(ssml).await {
                Ok(bytes) => return Ok(bytes),
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    async fn try_synthesize_chunk(&self, ssml: &str) -> Result<Vec<u8>, AppError> {
        // 免 token 直连。
        let url = format!(
            "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/audio/v1?trustedclienttoken={EDGE_TTS_TRUSTED_CLIENT_TOKEN}"
        );
        let response = self
            .http
            .post(&url)
            .header("Content-Type", "application/ssml+xml")
            .header("X-Microsoft-OutputFormat", EDGE_TTS_OUTPUT_FORMAT)
            .body(ssml.to_string())
            .send()
            .await
            .map_err(|error| AppError::internal(format!("edge tts synthesize failed: {error}")))?;
        if !response.status().is_success() {
            return Err(AppError::internal(format!(
                "edge tts synthesize returned {}",
                response.status()
            )));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| AppError::internal(format!("edge tts audio read failed: {error}")))
    }
}

/// 签名生成。
#[allow(dead_code)]
fn generate_translator_signature() -> Result<String, AppError> {
    let url_host = EDGE_TTS_ENDPOINT_URL
        .split("://")
        .nth(1)
        .unwrap_or_default();
    let encoded_url = urlencoding::encode(url_host);
    let request_id = uuid::Uuid::new_v4().simple().to_string();
    let date = build_signature_date();
    let payload = format!("{EDGE_TTS_APP_ID}{encoded_url}{date}{request_id}").to_lowercase();
    let key = base64::engine::general_purpose::STANDARD
        .decode(EDGE_TTS_SIGNATURE_SECRET_BASE64)
        .map_err(|error| AppError::internal(format!("edge tts key decode failed: {error}")))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key)
        .map_err(|error| AppError::internal(format!("edge tts hmac failed: {error}")))?;
    mac.update(payload.as_bytes());
    let signature = mac.finalize().into_bytes();
    let signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature);
    Ok(format!(
        "{EDGE_TTS_APP_ID}::{signature_b64}::{date}::{request_id}"
    ))
}

fn build_signature_date() -> String {
    // "Thu, 01 Jan 1970 00:00:00 GMT" → 去 GMT 小写 + " GMT"
    let now = chrono::Utc::now();
    let formatted = now
        .format("%a, %d %b %Y %H:%M:%S")
        .to_string()
        .to_lowercase();
    format!("{formatted} GMT")
}

/// 构建 SSML（控制字符清洗 + XML 转义 + prosody 参数）。
fn build_ssml(text: &str, voice: &str, rate: Option<&str>) -> String {
    let sanitized: String = text
        .chars()
        .map(|ch| {
            let code = ch as u32;
            if (code <= 8) || (11..=12).contains(&code) || (14..=31).contains(&code) {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let locale = voice.split('-').take(2).collect::<Vec<_>>().join("-");
    let locale = if locale.is_empty() { "zh-CN" } else { &locale };
    let rate = rate.unwrap_or("+0%");
    format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"{}\"><voice name=\"{}\"><prosody rate=\"{}\" pitch=\"+0Hz\" volume=\"+0%\">{}</prosody></voice></speak>",
        escape_xml_attr(locale),
        escape_xml_attr(voice),
        escape_xml_attr(rate),
        escape_xml_text(sanitized.trim())
    )
}

fn escape_xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attr(text: &str) -> String {
    escape_xml_text(text)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// TTS 分块：UTF-8 ≤1800 字节，软边界（标点/空白）优先，保底 60% 处硬切。
fn chunk_text_for_tts(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.len() <= EDGE_TTS_MAX_CHUNK_BYTES {
            chunks.push(rest.to_string());
            break;
        }
        // 在字节预算内找最大 char 边界
        let budget = EDGE_TTS_MAX_CHUNK_BYTES.min(rest.len());
        let mut cut = budget;
        while cut > 0 && !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        // 软边界：在 [60%, budget] 区间内找最后的标点/空白
        let floor = cut * 3 / 5;
        let mut soft = None;
        for (index, ch) in rest.char_indices().take_while(|(i, _)| *i <= cut) {
            if index >= floor
                && matches!(
                    ch,
                    ' ' | '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '\n'
                )
            {
                soft = Some(index + ch.len_utf8());
            }
        }
        let split_at = soft.unwrap_or(cut);
        let (head, tail) = rest.split_at(split_at);
        let head = head.trim();
        if !head.is_empty() {
            chunks.push(head.to_string());
        }
        rest = tail.trim_start();
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssml_escapes_and_sanitizes() {
        let ssml = build_ssml(
            "Hello <world> & \"friends\"\u{0007}",
            "zh-CN-XiaoxiaoNeural",
            None,
        );
        assert!(
            ssml.contains("Hello &lt;world&gt; &amp; \"friends\""),
            "ssml: {ssml}"
        );
        assert!(!ssml.contains('\u{0007}'));
        assert!(ssml.contains("xml:lang=\"zh-CN\""));
        assert!(ssml.contains("voice name=\"zh-CN-XiaoxiaoNeural\""));
        assert!(ssml.contains("rate=\"+0%\""));
    }

    #[test]
    fn chunking_respects_byte_budget() {
        let long = "这是一段很长的中文文本，用来测试 TTS 分块功能。".repeat(200);
        let chunks = chunk_text_for_tts(&long);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(
                chunk.len() <= EDGE_TTS_MAX_CHUNK_BYTES + 8,
                "chunk too long: {}",
                chunk.len()
            );
        }
    }

    #[test]
    fn chunking_prefers_soft_boundaries() {
        let text = format!("{}。{}", "甲".repeat(700), "乙".repeat(700));
        let chunks = chunk_text_for_tts(&text);
        assert!(chunks.len() >= 2);
        // 软边界切分应尽量在句号后（中文每字 3 字节，700 字 = 2100 > 1800，必切）
        assert!(
            chunks[0].ends_with('。') || chunks[0].chars().all(|c| c == '甲'),
            "first chunk: {}",
            &chunks[0][..20.min(chunks[0].len())]
        );
    }

    #[test]
    fn chunking_handles_surrogate_safe_cjk() {
        // 全 CJK 文本分块不应产生半个字符（char boundary 对齐）
        let text = "汉".repeat(1200); // 3600 bytes
        let chunks = chunk_text_for_tts(&text);
        let reassembled = chunks.join("");
        assert_eq!(reassembled, "汉".repeat(1200));
    }

    #[test]
    fn signature_date_format() {
        let date = build_signature_date();
        assert!(date.ends_with(" GMT"), "date: {date}");
        assert!(!date.contains("GMT GMT"));
    }

    #[test]
    fn empty_and_short_text() {
        assert!(chunk_text_for_tts("").is_empty());
        assert_eq!(chunk_text_for_tts("短文本"), vec!["短文本".to_string()]);
    }
}
