//! Structured API errors for the AI gateway.
//!
//! New code should prefer `Retryable`, `Fatal`, and `Cancelled`. The legacy
//! `*Failed(String)` variants are kept for compatibility with older call sites
//! and for local parse/format errors that do not come from an HTTP response.

use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("OCR 调用失败: {0}")]
    OcrFailed(String),

    #[error("VLM 调用失败: {0}")]
    VlmFailed(String),

    #[error("LLM 调用失败: {0}")]
    LlmFailed(String),

    #[error("DeepSeek 调用失败: {0}")]
    DeepSeekFailed(String),

    #[error("ASR 转录失败: {0}")]
    AsrFailed(String),

    #[error("视频处理失败: {0}")]
    VideoFailed(String),

    #[error("请求已被用户取消")]
    Cancelled,

    #[error("可重试错误 [{}]: {msg}", status.map(|s| s.to_string()).unwrap_or_else(|| "network".into()))]
    Retryable {
        status: Option<u16>,
        msg: String,
        retry_after: Option<Duration>,
        attempts: u32,
    },

    #[error("{}", .0.user_message)]
    Fatal(FatalError),
}

#[derive(Debug, Clone)]
pub struct FatalError {
    pub kind: FatalKind,
    pub status: Option<u16>,
    pub upstream_msg: String,
    pub user_message: String,
    pub suggestion: Option<String>,
    pub retryable_by_user: bool,
    pub attempts: u32,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FatalKind {
    RateLimitExhausted,
    AuthFailed,
    BadRequest,
    ContextTooLong,
    ContentFiltered,
    ModelNotFound,
    QuotaExceeded,
    UpstreamDown,
    Timeout,
    ConfigError,
    Unknown,
}

impl FatalError {
    pub fn config_error(msg: impl Into<String>) -> Self {
        let m = msg.into();
        Self {
            kind: FatalKind::ConfigError,
            status: None,
            upstream_msg: m.clone(),
            user_message: format!("网关配置错误：{m}"),
            suggestion: Some("检查 API Key / 模型配置是否正确".into()),
            retryable_by_user: false,
            attempts: 0,
            model: None,
        }
    }
}

impl ApiError {
    pub fn user_message(&self) -> String {
        match self {
            ApiError::Fatal(f) => f.user_message.clone(),
            ApiError::Retryable {
                status,
                msg,
                attempts,
                ..
            } => format!(
                "上游暂时不可用（已重试 {} 次{}）：{}",
                attempts,
                status.map(|s| format!("，HTTP {}", s)).unwrap_or_default(),
                truncate(msg, 120)
            ),
            ApiError::Cancelled => "请求已取消".into(),
            ApiError::OcrFailed(s)
            | ApiError::VlmFailed(s)
            | ApiError::LlmFailed(s)
            | ApiError::DeepSeekFailed(s)
            | ApiError::AsrFailed(s)
            | ApiError::VideoFailed(s) => truncate(s, 200),
        }
    }

    pub fn suggestion(&self) -> Option<String> {
        match self {
            ApiError::Fatal(f) => f.suggestion.clone(),
            ApiError::Retryable { .. } => Some("稍后再试或检查网络".into()),
            _ => None,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            ApiError::Fatal(f) => match f.kind {
                FatalKind::RateLimitExhausted => "rate_limit_exhausted",
                FatalKind::AuthFailed => "auth_failed",
                FatalKind::BadRequest => "bad_request",
                FatalKind::ContextTooLong => "context_too_long",
                FatalKind::ContentFiltered => "content_filtered",
                FatalKind::ModelNotFound => "model_not_found",
                FatalKind::QuotaExceeded => "quota_exceeded",
                FatalKind::UpstreamDown => "upstream_down",
                FatalKind::Timeout => "timeout",
                FatalKind::ConfigError => "config_error",
                FatalKind::Unknown => "unknown",
            },
            ApiError::Retryable { .. } => "retryable",
            ApiError::Cancelled => "cancelled",
            ApiError::OcrFailed(_) => "ocr_failed",
            ApiError::VlmFailed(_) => "vlm_failed",
            ApiError::LlmFailed(_) => "llm_failed",
            ApiError::DeepSeekFailed(_) => "deepseek_failed",
            ApiError::AsrFailed(_) => "asr_failed",
            ApiError::VideoFailed(_) => "video_failed",
        }
    }

    pub fn is_fatal(&self) -> bool {
        matches!(self, ApiError::Fatal(_))
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, ApiError::Retryable { .. })
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, ApiError::Cancelled)
    }

    pub fn retryable_by_user(&self) -> bool {
        match self {
            ApiError::Fatal(f) => f.retryable_by_user,
            ApiError::Retryable { .. } => true,
            ApiError::Cancelled => true,
            _ => true,
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
