use crate::context::{keys, AssistantContext};
use corework::cache::CacheExt;
use corework::execution_unit::ExecutionUnit;
use corework::statemachine::{FnState, SimpleTransition};
use std::sync::Arc;

use super::{agent_keys, events, states};

pub fn build() -> FnState {
    FnState::new(states::SAYING)
        .with_description("统一承载提问、结果与普通回复")
        .with_on_enter(|ctx| Box::pin(on_enter(ctx)))
        .with_on_transition(|ctx| Box::pin(on_transition(ctx)))
        .add_transition(
            events::USER_INPUT,
            Box::new(SimpleTransition::new(events::USER_INPUT, states::THINKING)),
        )
}

async fn on_enter(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<()> {
    let cache = sm_ctx.cache();
    let event_bus = sm_ctx.event_bus();
    let next_after_saying: String = cache
        .get(keys::NEXT_STATE_AFTER_SAYING)
        .await?
        .unwrap_or_else(|| states::SUSPENDED.to_string());
    let waiting_for_input = next_after_saying != states::THINKING;

    cache
        .set(keys::WAITING_FOR_INPUT, &waiting_for_input, None)
        .await?;

    let pending_question: Option<String> = cache.get(keys::PENDING_QUESTION).await?;
    if let Some(question) = pending_question {
        cache.delete(keys::PENDING_QUESTION).await?;
        cache.delete(keys::PENDING_VIEW).await?;
        upsert_last_assistant_message(&cache, &event_bus, &question, &question).await?;
        cache.set(keys::PENDING_RESPONSE, &question, None).await?;
    } else if let Some(result_text) = cache.get::<String>(keys::PENDING_RESULT).await? {
        let is_agent: bool = cache
            .get::<String>(agent_keys::AGENT_CLASS)
            .await?
            .is_some();
        let display_text = if is_agent {
            result_text.clone()
        } else {
            format!(
                "{}\n\n还有什么需要帮助的吗？如果对结果不满意，可以告诉我重新处理。",
                result_text
            )
        };

        upsert_last_assistant_message(&cache, &event_bus, &result_text, &display_text).await?;
        cache.delete(keys::PENDING_VIEW).await?;

        cache
            .set(keys::PENDING_RESPONSE, &display_text, None)
            .await?;
        cache.delete(keys::PENDING_RESULT).await?;
    } else if cache.get::<String>(keys::PENDING_RESPONSE).await?.is_some() {
        cache.delete(keys::PENDING_VIEW).await?;
    }

    cache.set(keys::THINKING_ROUND_COUNT, &0u32, None).await?;

    let resp: String = cache
        .get::<String>(keys::PENDING_RESPONSE)
        .await?
        .unwrap_or_default();
    if !resp.is_empty() {
        let src = crate::agent::source_id_from_cache(&*cache).await;
        let turn_id = AssistantContext::current_turn_id(&cache).await;
        crate::agent::publish_user_facing(
            &*event_bus,
            crate::events::types::SAYING,
            serde_json::json!({
                "content": resp,
                "turn_id": turn_id,
            }),
            &src,
        )
        .await;
        if waiting_for_input {
            crate::agent::publish_user_facing(
                &*event_bus,
                crate::events::types::TURN_DONE,
                serde_json::json!({ "turn_id": turn_id }),
                &src,
            )
            .await;
        }
    }
    crate::agent::publish_focus_status_for_cache(
        sm_ctx.as_ref(),
        &*cache,
        &*event_bus,
        states::SAYING,
    )
    .await;

    Ok(())
}

async fn on_transition(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<Option<String>> {
    let cache = sm_ctx.cache();
    let next: String = cache
        .get(keys::NEXT_STATE_AFTER_SAYING)
        .await?
        .unwrap_or_else(|| states::SUSPENDED.to_string());
    cache.delete(keys::NEXT_STATE_AFTER_SAYING).await?;
    Ok(Some(next))
}

async fn upsert_last_assistant_message(
    cache: &Arc<dyn corework::cache::Cache>,
    event_bus: &Arc<dyn corework::event::EventBus>,
    original: &str,
    replacement: &str,
) -> corework::error::Result<()> {
    if original == replacement {
        return Ok(());
    }
    AssistantContext::push_assistant_message_on_event_bus(cache, event_bus, replacement).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_machine::states;
    use crate::views::ViewPayload;
    use corework::cache::CacheExt;
    use corework::statemachine::{FnState, StateMachine};

    async fn build_test_sm() -> StateMachine {
        let thinking_stub = FnState::new(states::THINKING).with_description("thinking stub");
        let suspended_stub = FnState::new(states::SUSPENDED).with_description("suspended stub");

        StateMachine::builder("test_saying")
            .add_state(Box::new(build()))
            .add_state(Box::new(thinking_stub))
            .add_state(Box::new(suspended_stub))
            .initial_state(states::SAYING)
            .build()
            .await
            .expect("build test state machine")
    }

    #[tokio::test]
    async fn test_stale_pending_view_is_cleared_for_plain_response() {
        let sm = build_test_sm().await;
        let cache = sm.unit().cache();

        cache
            .set(keys::PENDING_RESPONSE, &"plain reply".to_string(), None)
            .await
            .expect("set pending response");
        cache
            .set(
                keys::PENDING_VIEW,
                &ViewPayload::new("[select:single | label=\"old\" | options=\"A,B\"]"),
                None,
            )
            .await
            .expect("set pending view");

        sm.start().await.expect("start");

        let pending_view: Option<ViewPayload> = cache.get(keys::PENDING_VIEW).await.unwrap();
        assert!(pending_view.is_none());

        let pending_response: Option<String> = cache.get(keys::PENDING_RESPONSE).await.unwrap();
        assert_eq!(pending_response.as_deref(), Some("plain reply"));
    }

    #[tokio::test]
    async fn test_saying_can_auto_continue_to_thinking() {
        let sm = build_test_sm().await;
        let cache = sm.unit().cache();

        cache
            .set(
                keys::NEXT_STATE_AFTER_SAYING,
                &states::THINKING.to_string(),
                None,
            )
            .await
            .expect("set next state");

        sm.start().await.expect("start");

        let waiting_for_input: Option<bool> = cache.get(keys::WAITING_FOR_INPUT).await.unwrap();
        assert_eq!(waiting_for_input, Some(false));

        sm.tick().await.expect("tick");
        assert_eq!(sm.current_state(), states::THINKING);

        let next_after_saying: Option<String> =
            cache.get(keys::NEXT_STATE_AFTER_SAYING).await.unwrap();
        assert!(next_after_saying.is_none());
    }

    #[tokio::test]
    async fn test_saying_defaults_to_suspended_when_waiting_for_input() {
        let sm = build_test_sm().await;
        let cache = sm.unit().cache();

        cache
            .set(keys::PENDING_RESPONSE, &"plain reply".to_string(), None)
            .await
            .expect("set pending response");

        sm.start().await.expect("start");

        let waiting_for_input: Option<bool> = cache.get(keys::WAITING_FOR_INPUT).await.unwrap();
        assert_eq!(waiting_for_input, Some(true));

        sm.tick().await.expect("tick");
        assert_eq!(sm.current_state(), states::SUSPENDED);

        let next_after_saying: Option<String> =
            cache.get(keys::NEXT_STATE_AFTER_SAYING).await.unwrap();
        assert!(next_after_saying.is_none());
    }
}
