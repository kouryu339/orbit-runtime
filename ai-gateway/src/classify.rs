//! HTTP response error classification.

use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::StatusCode;
use serde_json::Value;

use crate::error::{ApiError, FatalError, FatalKind};

pub fn classify_http_error(
    status: StatusCode,
    body: &str,
    headers: &HeaderMap,
    model: &str,
) -> ApiError {
    let code = status.as_u16();
    let upstream = extract_error_msg(body);

    match code {
        429 => {
            if is_quota_error(&upstream, body) {
                return ApiError::Fatal(FatalError {
                    kind: FatalKind::QuotaExceeded,
                    status: Some(429),
                    upstream_msg: upstream.clone(),
                    user_message: "API 余额或配额不足。".into(),
                    suggestion: Some("到服务商控制台检查用量、账单或充值状态。".into()),
                    retryable_by_user: false,
                    attempts: 0,
                    model: Some(model.into()),
                });
            }
            return ApiError::Retryable {
                status: Some(429),
                msg: upstream,
                retry_after: parse_retry_after(headers),
                attempts: 0,
            };
        }
        408 | 500..=599 => {
            return ApiError::Retryable {
                status: Some(code),
                msg: upstream,
                retry_after: parse_retry_after(headers),
                attempts: 0,
            };
        }
        _ => {}
    }

    let (kind, user_message, suggestion, retryable_by_user) = match code {
        401 | 403 => (
            FatalKind::AuthFailed,
            "API Key 无效、过期，或没有访问该模型的权限。".to_string(),
            Some("检查或更新对应服务商的 API Key。".into()),
            false,
        ),
        404 => (
            FatalKind::ModelNotFound,
            format!("模型 [{model}] 不存在，或当前 Key 没有访问权限。"),
            Some("确认模型名拼写，或向服务商申请该模型权限。".into()),
            false,
        ),
        402 => (
            FatalKind::QuotaExceeded,
            "API 余额不足。".to_string(),
            Some("到服务商控制台检查账单或充值。".into()),
            false,
        ),
        400 if is_context_too_long(&upstream, body) => (
            FatalKind::ContextTooLong,
            "对话上下文超过模型最大长度限制。".to_string(),
            Some("开启新会话、压缩历史，或切换到更长上下文模型。".into()),
            false,
        ),
        400 if is_content_filter(&upstream, body) => (
            FatalKind::ContentFiltered,
            "内容被上游安全策略拦截。".to_string(),
            Some("调整提示词措辞后重试。".into()),
            true,
        ),
        400 => (
            FatalKind::BadRequest,
            format!("请求被上游拒绝：{}", truncate(&upstream, 120)),
            Some("检查模型名、参数格式、消息 role 或工具调用格式。".into()),
            true,
        ),
        _ => (
            FatalKind::Unknown,
            format!("HTTP {}: {}", code, truncate(&upstream, 120)),
            None,
            true,
        ),
    };

    ApiError::Fatal(FatalError {
        kind,
        status: Some(code),
        upstream_msg: upstream,
        user_message,
        suggestion,
        retryable_by_user,
        attempts: 0,
        model: Some(model.into()),
    })
}

pub fn classify_network_error(err: &reqwest::Error) -> ApiError {
    ApiError::Retryable {
        status: None,
        msg: err.to_string(),
        retry_after: None,
        attempts: 0,
    }
}

pub fn fatalize_retry_exhausted(err: ApiError, model: &str) -> ApiError {
    match err {
        ApiError::Retryable {
            status,
            msg,
            attempts,
            ..
        } => {
            let (kind, user_message, suggestion) = match status {
                Some(429) => (
                    FatalKind::RateLimitExhausted,
                    format!("上游 API 限流，已重试 {attempts} 次仍失败。"),
                    Some("等待 1-2 分钟，或切换到备用 Key/模型。".into()),
                ),
                Some(s) if (500..=599).contains(&s) => (
                    FatalKind::UpstreamDown,
                    format!("上游服务异常 (HTTP {s})，已重试 {attempts} 次仍失败。"),
                    Some("稍后再试；若持续失败请查看服务商状态页。".into()),
                ),
                None => (
                    FatalKind::Timeout,
                    format!("网络超时或连接失败，已重试 {attempts} 次。"),
                    Some("检查本地网络、代理或服务商连通性。".into()),
                ),
                Some(s) => (
                    FatalKind::UpstreamDown,
                    format!("上游错误 (HTTP {s})，已重试 {attempts} 次。"),
                    None,
                ),
            };
            ApiError::Fatal(FatalError {
                kind,
                status,
                upstream_msg: msg,
                user_message,
                suggestion,
                retryable_by_user: true,
                attempts,
                model: Some(model.into()),
            })
        }
        other => other,
    }
}

pub async fn json_response_or_error(
    resp: reqwest::Response,
    model: &str,
) -> crate::error::Result<Value> {
    let status = resp.status();
    if !status.is_success() {
        let headers = resp.headers().clone();
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_http_error(status, &body, &headers, model));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| ApiError::LlmFailed(format!("解析上游响应失败: {e}")))
}

fn extract_error_msg(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(s) = v.pointer("/error/message").and_then(|x| x.as_str()) {
            return s.to_string();
        }
        if let Some(s) = v.pointer("/error_msg").and_then(|x| x.as_str()) {
            return s.to_string();
        }
        if let Some(s) = v.pointer("/message").and_then(|x| x.as_str()) {
            return s.to_string();
        }
        if let Some(s) = v.pointer("/code").and_then(|x| x.as_str()) {
            return s.to_string();
        }
        if let Some(s) = v.get("error").and_then(|x| x.as_str()) {
            return s.to_string();
        }
    }
    truncate(body, 200)
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let h = headers.get(RETRY_AFTER)?.to_str().ok()?;
    if let Ok(secs) = h.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs.min(300)));
    }
    None
}

fn is_quota_error(extracted: &str, body: &str) -> bool {
    let s = extracted.to_ascii_lowercase();
    let b = body.to_ascii_lowercase();
    let kws = [
        "quota",
        "billing",
        "insufficient",
        "balance",
        "insufficient_quota",
        "payment",
        "余额",
        "配额",
        "欠费",
    ];
    kws.iter().any(|k| s.contains(k) || b.contains(k))
}

fn is_context_too_long(extracted: &str, body: &str) -> bool {
    let s = extracted.to_ascii_lowercase();
    let b = body.to_ascii_lowercase();
    let kws = [
        "context_length",
        "context length",
        "maximum context",
        "max_tokens",
        "too long",
        "context window",
        "maximum length",
        "上下文",
        "超过最大",
    ];
    kws.iter().any(|k| s.contains(k) || b.contains(k))
}

fn is_content_filter(extracted: &str, body: &str) -> bool {
    let s = extracted.to_ascii_lowercase();
    let b = body.to_ascii_lowercase();
    let kws = [
        "content_filter",
        "content policy",
        "safety",
        "data_inspection_failed",
        "risk_control",
        "sensitive",
        "moderation",
        "敏感",
        "审核",
        "违规",
    ];
    kws.iter().any(|k| s.contains(k) || b.contains(k))
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
