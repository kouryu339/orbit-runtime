//! 挂起状态 —— Agent 暂停/待输入
//! ## 进入条件
//!   用户暂停、焦点让渡、初始化待输入，或工具显式要求暂停当前 Agent。
//! ## 转移规则
//!   - `USER_INPUT` / `RESUME` → `thinking`

use corework::cache::CacheExt;
use corework::execution_unit::ExecutionUnit;
use corework::statemachine::{FnState, SimpleTransition};
use std::sync::Arc;

use super::{events, states};
use crate::context::keys;

/// 构建挂起状态
pub fn build() -> FnState {
    FnState::new(states::SUSPENDED)
        .with_description("Agent 暂停或待输入")
        .with_on_enter(|ctx| Box::pin(on_enter(ctx)))
        .with_on_exit(|ctx| Box::pin(on_exit(ctx)))
        .add_transition(
            events::USER_INPUT,
            Box::new(SimpleTransition::new(events::USER_INPUT, states::THINKING)),
        )
        .add_transition(
            events::RESUME,
            Box::new(SimpleTransition::new(events::RESUME, states::THINKING)),
        )
}

async fn on_enter(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<()> {
    let cache = sm_ctx.cache();
    cache.set(keys::WAITING_FOR_INPUT, &true, None).await?;
    let _ = super::consume_pause_if_requested(&cache).await?;
    let event_bus = sm_ctx.event_bus();
    crate::agent::publish_focus_status_for_cache(
        sm_ctx.as_ref(),
        &*cache,
        &*event_bus,
        states::SUSPENDED,
    )
    .await;

    Ok(())
}

async fn on_exit(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<()> {
    sm_ctx
        .cache()
        .set(keys::WAITING_FOR_INPUT, &false, None)
        .await?;
    Ok(())
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{events, states};
    use corework::cache::CacheExt;
    use corework::statemachine::StateMachine;

    /// 构建一个仅含 suspended + thinking 的最小状态机，用于单元测试。
    /// thinking 用空壳状态（无 on_enter / on_transition），只是转移目标。
    async fn build_test_sm() -> StateMachine {
        let thinking_stub = corework::statemachine::FnState::new(states::THINKING)
            .with_description("thinking stub");

        StateMachine::builder("test_suspended")
            .add_state(Box::new(build()))
            .add_state(Box::new(thinking_stub))
            .initial_state(states::SUSPENDED)
            .build()
            .await
            .expect("构建测试状态机失败")
    }

    // ------------------------------------------------------------------
    // 测试 1：首次进入 suspended 时静默（不写 PENDING_RESPONSE）
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_first_enter_is_silent() {
        let sm = build_test_sm().await;
        sm.start().await.expect("start 失败");

        assert_eq!(sm.current_state(), states::SUSPENDED);

        let cache = sm.unit().cache();

        // WAITING_FOR_INPUT 应为 true
        let waiting: Option<bool> = cache.get(keys::WAITING_FOR_INPUT).await.unwrap();
        assert_eq!(
            waiting,
            Some(true),
            "首次进入 suspended 应设置 WAITING_FOR_INPUT=true"
        );

        // PENDING_RESPONSE 不应有值（静默进入）
        let resp: Option<String> = cache.get(keys::PENDING_RESPONSE).await.unwrap();
        assert!(
            resp.is_none() || resp.as_deref() == Some(""),
            "首次进入 suspended 不应写 PENDING_RESPONSE，got: {:?}",
            resp
        );
    }

    // ------------------------------------------------------------------
    // 测试 2：USER_INPUT → 转移到 thinking
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_user_input_transitions_to_thinking() {
        let sm = build_test_sm().await;
        sm.start().await.expect("start 失败");

        // 模拟用户发消息
        sm.send_event(events::USER_INPUT)
            .await
            .expect("send_event USER_INPUT 失败");

        assert_eq!(
            sm.current_state(),
            states::THINKING,
            "USER_INPUT 后应转移到 thinking"
        );

        let cache = sm.unit().cache();
        let waiting: Option<bool> = cache.get(keys::WAITING_FOR_INPUT).await.unwrap();
        assert_eq!(
            waiting,
            Some(false),
            "离开 suspended 后 WAITING_FOR_INPUT 应被重置为 false"
        );
    }

    // ------------------------------------------------------------------
    // 测试 4：RESUME → 转移到 thinking
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_resume_transitions_to_thinking() {
        let sm = build_test_sm().await;
        sm.start().await.expect("start 失败");

        sm.send_event(events::RESUME).await.expect("RESUME 失败");
        assert_eq!(sm.current_state(), states::THINKING);
    }
}
