//! AI助手状态实现
//! 每个状态在独立文件中实现，通过 `build()` 返回配置好回调的 `FnState`。
//! 状态转移由各 `on_transition` 读取 `NEXT_STATE` cache 键完成路由。

pub mod executing;
pub mod saying;
pub mod suspended;
pub mod thinking;

use crate::context::keys;
use corework::cache::CacheExt;

/// 检查并消费暂停信号。若已请求暂停，清理中间状态并返回 `true`。
/// 调用方应在返回 `true` 后将状态机转入 `suspended`（不是 `saying`）。
/// `saying` 承载 AI 的自然停稳，`suspended` 承载用户主动暂停。

/// 静默暂停：不向对话历史插入任何消息，不设置 PENDING_RESPONSE。
/// 前端表现为"哪里停就停哪里"，用户输入框重新可用即可。

pub(crate) async fn consume_pause_if_requested(
    cache: &std::sync::Arc<dyn corework::cache::Cache>,
) -> corework::error::Result<bool> {
    let requested: bool = cache.get(keys::PAUSE_REQUESTED).await?.unwrap_or(false);
    if !requested {
        return Ok(false);
    }

    tracing::info!("检测到用户暂停请求，消费信号并准备转入 suspended");
    cache.set(keys::PAUSE_REQUESTED, &false, None).await?;
    cache
        .set(keys::TASK_STATUS, &"paused".to_string(), None)
        .await?;
    cache
        .set(keys::LAST_STOP_REASON, &"pause".to_string(), None)
        .await?;

    // 立即取消正在进行的 LLM HTTP 请求（核心：让 tokio::select! 赢得竞争）
    thinking::request_cancel();

    cache.delete(keys::PENDING_TOOLS).await?;
    cache.delete(keys::PENDING_STRUCTURED_TOOLS).await?;
    cache.delete(keys::PENDING_TOOL_CALLS).await?;
    cache.delete(keys::PENDING_TOOL_CALL_IDS).await?;
    cache.delete(keys::PENDING_TOOL_DISPLAY_COMMANDS).await?;
    cache.delete(keys::PENDING_TOOLS_WAIT_FOR_INPUT).await?;
    cache.delete(keys::NEXT_STATE).await?;

    // 静默暂停：不插入任何消息，不设置 PENDING_RESPONSE。
    // 前端只需看到 isLoading 变 false、输入框重新可用。

    Ok(true)
}

pub(crate) async fn request_pause(
    cache: &std::sync::Arc<dyn corework::cache::Cache>,
    sm: Option<&corework::statemachine::StateMachine>,
) -> corework::error::Result<()> {
    if let Some(sm) = sm {
        let current = sm.current_state();
        if current == states::SAYING {
            let next_after_saying: Option<String> =
                cache.get(keys::NEXT_STATE_AFTER_SAYING).await?;
            if next_after_saying.as_deref() != Some(states::THINKING) {
                return Ok(());
            }
            cache
                .set(
                    keys::NEXT_STATE_AFTER_SAYING,
                    &states::SUSPENDED.to_string(),
                    None,
                )
                .await?;
        } else if current != states::THINKING && current != states::EXECUTING {
            return Ok(());
        }
    }

    let requested: bool = cache.get(keys::PAUSE_REQUESTED).await?.unwrap_or(false);
    if !requested {
        cache.set(keys::PAUSE_REQUESTED, &true, None).await?;
    }

    if let Some(sm) = sm {
        let current = sm.current_state();
        if current == states::THINKING {
            thinking::request_cancel();
        }
    } else {
        thinking::request_cancel();
    }

    Ok(())
}

/// 状态名常量
pub mod states {
    pub const SAYING: &str = "saying";
    /// 思考 —— 调用 LLM 获取决策
    pub const THINKING: &str = "thinking";
    pub const EXECUTING: &str = "executing";
    /// Agent 挂起 —— 暂停、等待输入或焦点让渡后的停靠点
    pub const SUSPENDED: &str = "suspended";
}

/// 事件名常量
pub mod events {
    /// 用户发来了消息 → saying/suspended → thinking
    pub const USER_INPUT: &str = "user_input";
    /// 历史兼容事件，当前主链路不再依赖
    pub const FINISH: &str = "finish";
    pub const PAUSE: &str = "pause";
    pub const RESUME: &str = "resume";
}

/// 子 Agent cache key 常量（存在各自 ScopedCache 中）
pub mod agent_keys {
    /// Agent 类型标记 — `String`（"oneshot" / "interactive" / "scheduled"）
    pub const AGENT_CLASS: &str = "agent_class";
    /// Agent 名称 — `String`
    pub const AGENT_NAME: &str = "agent_name";
    /// Agent ID — `String`（供事件归属、焦点切换和复命工具使用）
    pub const AGENT_ID: &str = "agent_id";
    /// Parent conversation id visible to this agent's scoped cache.
    pub const CONVERSATION_ID: &str = "conversation_id";
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::cache::CacheExt;
    use corework::statemachine::{FnState, StateMachine};

    async fn test_cache() -> std::sync::Arc<dyn corework::cache::Cache> {
        let sm = StateMachine::builder("test_pause_cleanup")
            .add_state(Box::new(FnState::new(states::SUSPENDED)))
            .initial_state(states::SUSPENDED)
            .build()
            .await
            .expect("build state machine");
        sm.unit().cache()
    }

    #[tokio::test]
    async fn pause_clears_all_pending_tool_execution_metadata() {
        let cache = test_cache().await;
        cache.set(keys::PAUSE_REQUESTED, &true, None).await.unwrap();
        cache
            .set(
                keys::PENDING_TOOLS,
                &vec!["EXEC ReadFile".to_string()],
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                keys::PENDING_STRUCTURED_TOOLS,
                &vec![crate::decision_line::ParsedToolCall {
                    name: "ReadFile".to_string(),
                    params: vec![],
                }],
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                keys::PENDING_TOOL_CALLS,
                &vec![crate::context::ToolCallRef {
                    id: "old-call".to_string(),
                    name: "ReadFile".to_string(),
                }],
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                keys::PENDING_TOOL_CALL_IDS,
                &vec!["old-call".to_string()],
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                keys::PENDING_TOOL_DISPLAY_COMMANDS,
                &vec!["EXEC ReadFile".to_string()],
                None,
            )
            .await
            .unwrap();
        cache
            .set(keys::PENDING_TOOLS_WAIT_FOR_INPUT, &true, None)
            .await
            .unwrap();

        assert!(consume_pause_if_requested(&cache).await.unwrap());
        assert!(cache
            .get::<Vec<String>>(keys::PENDING_TOOLS)
            .await
            .unwrap()
            .is_none());
        assert!(cache
            .get::<Vec<crate::decision_line::ParsedToolCall>>(keys::PENDING_STRUCTURED_TOOLS)
            .await
            .unwrap()
            .is_none());
        assert!(cache
            .get::<Vec<crate::context::ToolCallRef>>(keys::PENDING_TOOL_CALLS)
            .await
            .unwrap()
            .is_none());
        assert!(cache
            .get::<Vec<String>>(keys::PENDING_TOOL_CALL_IDS)
            .await
            .unwrap()
            .is_none());
        assert!(cache
            .get::<Vec<String>>(keys::PENDING_TOOL_DISPLAY_COMMANDS)
            .await
            .unwrap()
            .is_none());
        assert!(cache
            .get::<bool>(keys::PENDING_TOOLS_WAIT_FOR_INPUT)
            .await
            .unwrap()
            .is_none());
    }
}
