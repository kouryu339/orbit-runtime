//! 通义千问 VL（视觉语言模型）API

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use corework::buns_system;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;

use crate::error::ApiError;
use crate::providers::{resolve_vlm, resolve_vlm_for_video, VlmApiStyle};
use crate::types::{TokenUsage, VlmResponse};

/// 默认模型（千问性能最强的视觉理解模型）
pub const DEFAULT_MODEL: &str = "qwen3.5-plus";

// ============================================================================
// IO 类型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallQwenVlInput {
    /// 图片本地路径
    pub image_path: String,
    /// 提示文本
    pub prompt: String,
    /// 模型名称（默认 "qwen-vl-max-latest"）
    #[serde(default)]
    pub model: Option<String>,
    /// 采样温度 [0, 2)
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    /// 最大输出 token 数
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallQwenVlOutput {
    pub response: VlmResponse,
}

// ============================================================================
// System
// ============================================================================

#[buns_system(
    "CallQwenVl",
    description = "调用通义千问 VL 多模态 API，传入图片与文本提示，返回模型回复",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = true
)]
pub struct CallQwenVlSystem;

#[async_trait]
impl SystemOperation for CallQwenVlSystem {
    type Input = CallQwenVlInput;
    type Output = CallQwenVlOutput;
    type Error = FrameworkError;

    fn name(&self) -> &str {
        "CallQwenVl"
    }

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let response = call_qwen_vl(
            &input.image_path,
            &input.prompt,
            input.model.as_deref(),
            input.temperature,
            input.top_p,
            input.max_tokens,
        )
        .await
        .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
        Ok(CallQwenVlOutput { response })
    }
}

// ============================================================================
// 纯函数实现
// ============================================================================

/// 调用通义千问 VL API（从环境变量读取凭证）
///
/// - `model`: 模型名，None 时使用 `DEFAULT_MODEL`
/// - `temperature` / `top_p` / `max_tokens`: 可选采样参数
pub async fn call_qwen_vl(
    image_path: &str,
    prompt: &str,
    model: Option<&str>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u32>,
) -> crate::error::Result<VlmResponse> {
    let api_key = std::env::var("DASHSCOPE_API_KEY")
        .map_err(|_| ApiError::VlmFailed("未设置 DASHSCOPE_API_KEY 环境变量".into()))?;

    let image_data =
        std::fs::read(image_path).map_err(|e| ApiError::VlmFailed(format!("读取图片失败: {e}")))?;
    let image_base64 = general_purpose::STANDARD.encode(&image_data);

    let client = Client::new();
    let url =
        "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";

    let mut params = json!({});
    if let Some(t) = temperature {
        params["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        params["top_p"] = json!(p);
    }
    if let Some(m) = max_tokens {
        params["max_tokens"] = json!(m);
    }

    let body = json!({
        "model": model.unwrap_or(DEFAULT_MODEL),
        "input": {
            "messages": [{
                "role": "user",
                "content": [
                    { "image": format!("data:image/jpeg;base64,{}", image_base64) },
                    { "text": prompt }
                ]
            }]
        },
        "parameters": params
    });

    let resp: Value = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::VlmFailed(format!("调用通义千问 VL 失败: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::VlmFailed(format!("解析 VL 响应失败: {e}")))?;

    // 错误检查
    if let Some(code) = resp.get("code") {
        if code.as_str() != Some("Success") && !code.as_str().unwrap_or("").is_empty() {
            return Err(ApiError::VlmFailed(format!(
                "通义千问 VL API 错误: {}",
                resp.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("未知错误")
            )));
        }
    }

    let content = resp["output"]["choices"][0]["message"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| ApiError::VlmFailed("VL 响应格式错误".into()))?
        .to_string();

    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(VlmResponse { content, tokens })
}

// ============================================================================
// ============================================================================

///
/// - `qwen-vl*`        → 通义千问 VL（DASHSCOPE_API_KEY）
/// - `doubao-vision*`  → 豆包视觉（ARK_API_KEY）
/// - `MiniMax-VL*`     → MiniMax VL（MINIMAX_API_KEY）
/// - `gpt-4o*`         → GPT-4o Vision（OPENAI_API_KEY）
/// - 其他/空           → fallback 通义千问 VL
pub async fn call_vlm(
    image_path: &str,
    prompt: &str,
    system_message: Option<&str>,
    model: Option<&str>,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
) -> crate::error::Result<VlmResponse> {
    let provider = resolve_vlm(model.unwrap_or(""));

    let api_key = if provider.env_key.is_empty() {
        String::new()
    } else {
        std::env::var(provider.env_key)
            .map_err(|_| ApiError::VlmFailed(format!("未设置 {} 环境变量", provider.env_key)))?
    };

    let model_name = model.unwrap_or(provider.default_model);

    match provider.api_style {
        VlmApiStyle::QwenNative => {
            // 通义千问 VL 原生端点（忽略 system_message，使用 prompt）
            call_qwen_vl(
                image_path,
                prompt,
                Some(model_name),
                temperature,
                None,
                max_tokens,
            )
            .await
        }
        VlmApiStyle::OpenAIVision => {
            crate::openai_compat::call_inner_vision(
                image_path,
                prompt,
                system_message,
                model_name,
                provider.base_url,
                &api_key,
                temperature,
                max_tokens,
            )
            .await
        }
    }
}

// ============================================================================
// 视频理解（原生视频输入，无需提取音轨）
// ============================================================================

/// 调用支持原生视频输入的视觉语言模型
///
/// - `qwen-vl*`         → 通义千问 VL（DashScope，`video` base64 字段）
/// - `doubao-vision*`   → 豆包视觉（ARK，`video_url` content block）
/// - `gemini-*`         → Gemini（Google OpenAI 兼容端点，`video_url` content block）
/// - 空/其他            → 自动选第一个支持视频的提供商（默认 qwen-vl-max-latest）
///
/// 若选定提供商不支持视频，返回错误（调用方可 fallback 到 ffmpeg+ASR 路线）。
pub async fn call_video_vlm(
    video_path: &str,
    prompt: &str,
    model: Option<&str>,
    max_tokens: Option<u32>,
) -> crate::error::Result<VlmResponse> {
    let provider = resolve_vlm_for_video(model.unwrap_or(""))
        .map_err(|e| ApiError::VlmFailed(e.to_string()))?;

    let api_key = if provider.env_key.is_empty() {
        String::new()
    } else {
        std::env::var(provider.env_key)
            .map_err(|_| ApiError::VlmFailed(format!("未设置 {} 环境变量", provider.env_key)))?
    };

    let model_name = model.unwrap_or(provider.default_model);

    match provider.api_style {
        VlmApiStyle::QwenNative => {
            call_qwen_vl_video(video_path, prompt, model_name, &api_key, max_tokens).await
        }
        VlmApiStyle::OpenAIVision => {
            crate::openai_compat::call_inner_vision_video(
                video_path,
                prompt,
                None,
                model_name,
                provider.base_url,
                &api_key,
                None,
                max_tokens,
            )
            .await
        }
    }
}

/// 通义千问 VL 原生视频理解（DashScope multimodal-generation，`video` base64 字段）
async fn call_qwen_vl_video(
    video_path: &str,
    prompt: &str,
    model: &str,
    api_key: &str,
    max_tokens: Option<u32>,
) -> crate::error::Result<VlmResponse> {
    use base64::{engine::general_purpose, Engine as _};

    let video_data = tokio::fs::read(video_path)
        .await
        .map_err(|e| ApiError::VlmFailed(format!("读取视频文件失败 [{}]: {e}", video_path)))?;
    let video_base64 = general_purpose::STANDARD.encode(&video_data);

    let mime = detect_video_mime(video_path);
    let data_url = format!("data:{};base64,{}", mime, video_base64);

    let client = Client::new();
    let url =
        "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";

    let mut params = serde_json::json!({});
    if let Some(m) = max_tokens {
        params["max_tokens"] = serde_json::json!(m);
    }

    let body = serde_json::json!({
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": [
                    { "video": data_url },
                    { "text": prompt }
                ]
            }]
        },
        "parameters": params
    });

    tracing::debug!("QwenVL 视频理解请求: model={}, video={}", model, video_path);

    let resp: serde_json::Value = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::VlmFailed(format!("QwenVL 视频 HTTP 请求失败: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::VlmFailed(format!("解析 QwenVL 视频响应失败: {e}")))?;

    if let Some(code) = resp.get("code") {
        if code.as_str() != Some("Success") && !code.as_str().unwrap_or("").is_empty() {
            return Err(ApiError::VlmFailed(format!(
                "通义千问 VL 视频 API 错误: {}",
                resp.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("未知错误")
            )));
        }
    }

    let content = resp["output"]["choices"][0]["message"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| ApiError::VlmFailed("QwenVL 视频响应格式错误".into()))?
        .to_string();

    let tokens = resp.get("usage").map(|u| TokenUsage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
    });

    Ok(VlmResponse { content, tokens })
}

/// 根据文件扩展名推断视频 MIME 类型
fn detect_video_mime(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("flv") => "video/x-flv",
        Some("ts") => "video/mp2t",
        _ => "video/mp4",
    }
}
