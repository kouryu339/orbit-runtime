//! 通用异步重试器
//!
//! - 仅重试 [`ApiError::Retryable`]
//! - 致命错误（[`ApiError::Fatal`] / 旧 variant）立即返回
//! - 指数退避 + jitter，可被 `Retry-After` header 覆盖
//! - 重试用尽时通过 [`fatalize_retry_exhausted`] 升级为 Fatal
//!
//! ## 使用
//!
//! ```ignore
//! let resp = retry_with_backoff(
//!     RetryPolicy::default(),
//!     "deepseek-chat",
//!     |attempt| async move {
//!         do_one_request(attempt).await
//!     }
//! ).await?;
//! ```

use std::future::Future;
use std::time::Duration;

use crate::classify::fatalize_retry_exhausted;
use crate::error::{ApiError, Result};

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// 最大尝试次数（含首次）。3 = 首次 + 2 次重试
    pub max_attempts: u32,
    /// 基础退避时长（attempt=0 时的等待）
    pub base_delay: Duration,
    /// 最大退避时长（封顶）
    pub max_delay: Duration,
    /// jitter 上限（毫秒）
    pub jitter_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(20),
            jitter_ms: 250,
        }
    }
}

impl RetryPolicy {
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }
}

/// 计算第 `attempt` 次重试前的等待时长
/// `attempt` 从 0 开始（即第一次失败后等多久重试）
fn backoff(policy: &RetryPolicy, attempt: u32, retry_after: Option<Duration>) -> Duration {
    if let Some(ra) = retry_after {
        return ra.min(policy.max_delay);
    }
    let exp = policy.base_delay.saturating_mul(1 << attempt.min(6));
    let capped = exp.min(policy.max_delay);
    // 简单 jitter（不依赖 rand crate，用纳秒时间戳低位）
    let jitter = if policy.jitter_ms > 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        Duration::from_millis(nanos % policy.jitter_ms)
    } else {
        Duration::ZERO
    };
    capped + jitter
}

/// 执行带重试的异步操作
///
/// `f(attempt)` 中 attempt 从 0 开始，便于回调内做日志：第 0 次=首次，第 1 次=第一次重试...
pub async fn retry_with_backoff<F, Fut, T>(policy: RetryPolicy, model: &str, mut f: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt: u32 = 0;
    loop {
        match f(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                // 致命 / 取消 / 旧 variant 立即返回
                if !e.is_retryable() {
                    return Err(e);
                }
                let next_attempt = attempt + 1;
                if next_attempt >= policy.max_attempts {
                    // 重试用尽 → 升级为 Fatal
                    let mut e = e;
                    if let ApiError::Retryable {
                        ref mut attempts, ..
                    } = e
                    {
                        *attempts = next_attempt;
                    }
                    return Err(fatalize_retry_exhausted(e, model));
                }
                // 计算退避
                let retry_after = match &e {
                    ApiError::Retryable { retry_after, .. } => *retry_after,
                    _ => None,
                };
                let status = match &e {
                    ApiError::Retryable { status, .. } => *status,
                    _ => None,
                };
                let delay = backoff(&policy, attempt, retry_after);
                tracing::warn!(
                    target: "ai_gateway::retry",
                    attempt = attempt + 1,
                    max = policy.max_attempts,
                    status = ?status,
                    delay_ms = delay.as_millis() as u64,
                    model = %model,
                    error = %e,
                    "上游可重试错误，{}ms 后重试",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                attempt = next_attempt;
            }
        }
    }
}
