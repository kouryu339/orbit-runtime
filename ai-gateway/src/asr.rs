//! ASR（自动语音识别）API
//!
//! - `qwen-audio*`  → 阿里 DashScope qwen-audio-turbo（DASHSCOPE_API_KEY）
//! - `doubao-asr*`  → 字节豆包 ASR（ARK_API_KEY）
//! - `whisper-*`    → OpenAI Whisper（OPENAI_API_KEY）
//! - 其他/空         → fallback qwen-audio-turbo

use std::sync::OnceLock;
use std::time::Instant;

use reqwest::Client;
use serde_json::Value;

use crate::error::ApiError;
use crate::providers::{resolve_asr, AsrApiStyle};
use crate::types::{AsrResponse, AsrSegment, TokenUsage};

// 复用全局 HTTP Client，避免每次调用重建连接池
static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

fn get_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(Client::new)
}

// ============================================================================
// 公开入口
// ============================================================================

/// 调用 ASR 接口转录音频文件
///
/// # 参数
/// - `media_path` — 本地音频文件路径（支持 mp3/wav/m4a/ogg/flac）
/// - `model`      — 模型名（None → qwen-audio-turbo）
/// - `language`   — 语言代码（None → 自动检测）
/// - `prompt`     — 提示词（可提高特定词识别率，部分提供商支持）
pub async fn call_asr(
    media_path: &str,
    model: Option<&str>,
    language: Option<&str>,
    prompt: Option<&str>,
) -> crate::error::Result<AsrResponse> {
    let provider = resolve_asr(model.unwrap_or(""));

    let api_key = if provider.env_key.is_empty() {
        String::new()
    } else {
        std::env::var(provider.env_key)
            .map_err(|_| ApiError::AsrFailed(format!("未设置 {} 环境变量", provider.env_key)))?
    };

    let model_name = model.unwrap_or(provider.default_model);

    // QwenAudio 忽略 prompt（服务端不支持），WhisperCompat 附加 prompt
    let effective_prompt = match provider.api_style {
        AsrApiStyle::QwenAudio => None,
        AsrApiStyle::WhisperCompat => prompt,
    };

    call_transcription_inner(
        media_path,
        model_name,
        &api_key,
        provider.base_url,
        language,
        effective_prompt,
    )
    .await
}

// ============================================================================
// ============================================================================

/// multipart/form-data 上传音频文件并调用转录接口
///
/// 两种 API 风格（DashScope qwen-audio / OpenAI Whisper）HTTP 格式相同，
async fn call_transcription_inner(
    media_path: &str,
    model: &str,
    api_key: &str,
    base_url: &str,
    language: Option<&str>,
    prompt: Option<&str>,
) -> crate::error::Result<AsrResponse> {
    let start = Instant::now();

    // 读取媒体文件
    let file_bytes = tokio::fs::read(media_path)
        .await
        .map_err(|e| ApiError::AsrFailed(format!("读取音频文件失败 [{}]: {e}", media_path)))?;

    let file_name = std::path::Path::new(media_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.mp3")
        .to_string();

    let mime = detect_audio_mime(media_path);
    let url = format!("{}/audio/transcriptions", base_url);

    // 构建 multipart 表单
    let file_part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str(mime)
        .map_err(|e| ApiError::AsrFailed(format!("设置 MIME 失败: {e}")))?;

    let mut form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("response_format", "verbose_json")
        .part("file", file_part);

    if let Some(lang) = language {
        if !lang.is_empty() {
            form = form.text("language", lang.to_string());
        }
    }
    if let Some(p) = prompt {
        if !p.is_empty() {
            form = form.text("prompt", p.to_string());
        }
    }

    tracing::debug!("ASR 请求: model={}, file={}", model, media_path);

    let resp: Value = get_client()
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| ApiError::AsrFailed(format!("ASR HTTP 请求失败 [{}]: {e}", base_url)))?
        .json()
        .await
        .map_err(|e| ApiError::AsrFailed(format!("解析 ASR 响应失败: {e}")))?;

    parse_whisper_verbose_response(resp, start)
}

// ============================================================================
// 公共响应解析
// ============================================================================

fn parse_whisper_verbose_response(
    resp: Value,
    start: Instant,
) -> crate::error::Result<AsrResponse> {
    // 错误检查
    if let Some(err_obj) = resp.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(ApiError::AsrFailed(format!("ASR API 错误: {}", msg)));
    }

    let text = resp["text"]
        .as_str()
        .ok_or_else(|| ApiError::AsrFailed("ASR 响应缺少 text 字段".into()))?
        .to_string();

    let language = resp["language"].as_str().map(|s| s.to_string());

    // 解析分段（verbose_json 模式下有 segments）
    let segments: Option<Vec<AsrSegment>> = resp
        .get("segments")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|seg| {
                    let seg_text = seg["text"].as_str()?.to_string();
                    // start/end 字段缺失或格式不对时跳过该分段（而不是输出 0ms）
                    let start_sec = seg["start"].as_f64()?;
                    let end_sec = seg["end"].as_f64()?;
                    Some(AsrSegment {
                        start_ms: (start_sec * 1000.0) as u64,
                        end_ms: (end_sec * 1000.0) as u64,
                        text: seg_text,
                    })
                })
                .collect()
        })
        .filter(|v: &Vec<AsrSegment>| !v.is_empty());

    // token 用量（部分提供商返回）
    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
    });

    let elapsed_ms = start.elapsed().as_millis() as u64;

    Ok(AsrResponse {
        text,
        language,
        segments,
        elapsed_ms: Some(elapsed_ms),
        tokens,
    })
}

// ============================================================================
// 工具函数
// ============================================================================

/// 根据文件扩展名推断 MIME 类型
fn detect_audio_mime(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("webm") => "audio/webm",
        Some("opus") => "audio/opus",
        Some("aac") => "audio/aac",
        _ => "audio/mpeg",
    }
}
