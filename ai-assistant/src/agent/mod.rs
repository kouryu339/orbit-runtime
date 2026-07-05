use corework::event::{BaseEvent, EventBus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub mod cluster;
pub mod runtime;
pub mod systems;
pub mod types;

pub mod keys {
    pub const ACTIVE_AGENT: &str = "agent:active";

    /// Legacy fallback agent id for migrating historical snapshots/ledgers
    /// written under the old "Boss" agent name. New code must rely on
    /// configured `agent_id` and `AgentCluster::default_agent_id()`; this
    /// constant must not be used to drive runtime behavior.
    pub const BOSS_AGENT_ID: &str = "boss";
}

pub use cluster::{AgentCluster, AgentClusterSnapshot, AgentRuntimeSnapshot};
pub use runtime::{AgentId, AgentKind, AgentPermissions, AgentRuntime};
pub use types::*;

pub fn global_conversation() -> Option<&'static Arc<crate::conversation::Conversation>> {
    crate::conversation::Conversation::global()
}

pub async fn publish_event(event_bus: &dyn EventBus, event_type: &str, payload: serde_json::Value) {
    let event = BaseEvent::new(event_type, payload);
    if let Err(e) = event_bus.publish(event).await {
        tracing::warn!(
            event_type = %event_type,
            error = %e,
            "failed to publish agent event"
        );
    }
}

pub async fn current_focus_id() -> String {
    match crate::conversation::Conversation::global() {
        Some(c) => c.focus_id().await,
        None => crate::persistence::current_default_agent_id(),
    }
}

pub async fn source_id_from_cache(cache: &dyn corework::cache::Cache) -> String {
    use corework::cache::CacheExt;
    cache
        .get::<String>(crate::state_machine::agent_keys::AGENT_ID)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::persistence::current_default_agent_id)
}

pub async fn source_meta_from_cache(cache: &dyn corework::cache::Cache) -> (String, String) {
    use corework::cache::CacheExt;
    let agent_id = source_id_from_cache(cache).await;
    let agent_name = cache
        .get::<String>(crate::state_machine::agent_keys::AGENT_NAME)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| agent_id.clone());
    (agent_id, agent_name)
}

pub async fn conversation_id_from_cache(cache: &dyn corework::cache::Cache) -> Option<String> {
    use corework::cache::CacheExt;
    cache
        .get::<String>(crate::state_machine::agent_keys::CONVERSATION_ID)
        .await
        .ok()
        .flatten()
        .filter(|id| !id.trim().is_empty())
}

pub async fn set_conversation_id_in_cache(
    cache: &dyn corework::cache::Cache,
    conversation_id: &str,
) -> corework::error::Result<()> {
    use corework::cache::CacheExt;
    cache
        .set(
            crate::state_machine::agent_keys::CONVERSATION_ID,
            &conversation_id.to_string(),
            None,
        )
        .await
}

pub async fn is_current_focus(agent_id: &str) -> bool {
    current_focus_id().await == agent_id
}

/// Notify snapshot consumers that an Agent state transition may have changed
/// the conversation-level runtime phase.
pub async fn publish_conversation_state_changed(event_bus: &dyn EventBus) {
    publish_event(
        event_bus,
        crate::events::types::CONVERSATION_STATE_CHANGED,
        serde_json::json!({}),
    )
    .await;
}

pub async fn publish_focus_status_for_cache(
    unit: &corework::execution_unit::ExecutionUnit,
    cache: &dyn corework::cache::Cache,
    event_bus: &dyn EventBus,
    state: &str,
) {
    use corework::cache::CacheExt;
    // Every state entry can affect the aggregate conversation state, including
    // transitions of a non-focused Agent in the cluster.
    publish_conversation_state_changed(event_bus).await;

    let (agent_id, agent_name) = source_meta_from_cache(cache).await;
    let is_focus = if let Some(state) =
        unit.resolve_shared_component::<crate::conversation_state::ConversationState>()
    {
        state.focus().await == agent_id
    } else {
        crate::persistence::current_default_agent_id() == agent_id
    };
    if !is_focus {
        return;
    }
    let pause_requested: bool = cache
        .get(crate::context::keys::PAUSE_REQUESTED)
        .await
        .ok()
        .flatten()
        .unwrap_or(false);
    let task_status = cache
        .get::<String>(crate::context::keys::TASK_STATUS)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            if matches!(
                state,
                crate::state::states::THINKING | crate::state::states::EXECUTING
            ) {
                "Pausing...".to_string()
            } else {
                "Pause".to_string()
            }
        });
    let stop_reason = cache
        .get::<String>(crate::context::keys::LAST_STOP_REASON)
        .await
        .ok()
        .flatten();
    let next_after_saying = cache
        .get::<String>(crate::context::keys::NEXT_STATE_AFTER_SAYING)
        .await
        .ok()
        .flatten();

    let saying_will_continue = state == crate::state::states::SAYING
        && next_after_saying.as_deref() == Some(crate::state::states::THINKING);
    let internally_busy = matches!(
        state,
        crate::state::states::THINKING | crate::state::states::EXECUTING
    ) || saying_will_continue;
    let busy = pause_requested || internally_busy;
    let pause_enabled = task_status == "running" && internally_busy && !pause_requested;
    let pause_visible = busy || task_status == "paused";
    let input_enabled = !busy;
    let status_kind = if pause_requested {
        "stopping"
    } else if task_status == "paused" || stop_reason.as_deref() == Some("pause") {
        "paused"
    } else if task_status == "waiting" {
        "waiting_user"
    } else if task_status == "done" {
        "done"
    } else if task_status == "failed" {
        "error"
    } else if busy {
        "working"
    } else {
        "idle"
    };
    let status_text = match status_kind {
        "stopping" => "stopping",
        "paused" => "paused",
        "waiting_user" => "waiting for user",
        "done" => "done",
        "error" => "error",
        "working" => "working",
        _ => "idle",
    };

    publish_event(
        event_bus,
        crate::events::types::FOCUS_STATUS_CHANGED,
        serde_json::to_value(crate::events::FocusStatusPayload {
            focused_agent_id: agent_id.clone(),
            agent_id,
            agent_name,
            status: crate::events::UiSnapshotStatus {
                kind: status_kind.to_string(),
                text: status_text.to_string(),
            },
            interaction: crate::events::UiSnapshotInteraction {
                input_enabled,
                send_enabled: input_enabled,
                pause_visible,
                pause_enabled,
                pause_label: if pause_requested {
                    "Pausing...".to_string()
                } else {
                    "Pause".to_string()
                },
                busy,
            },
        })
        .unwrap_or_else(|_| serde_json::json!({})),
    )
    .await;
}

pub async fn publish_focus_view_for_cache(
    unit: &corework::execution_unit::ExecutionUnit,
    cache: &dyn corework::cache::Cache,
    event_bus: &dyn EventBus,
    state: &str,
) {
    publish_focus_status_for_cache(unit, cache, event_bus, state).await;
}

pub async fn publish_user_facing(
    event_bus: &dyn EventBus,
    event_type: &str,
    mut payload: serde_json::Value,
    source_agent_id: &str,
) {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "agent_id".to_string(),
            serde_json::Value::String(source_agent_id.to_string()),
        );
        // Preserve one global turn id mapping per source turn.
        let original = obj.get("turn_id").and_then(|v| v.as_u64()).unwrap_or(0);
        let global = remap_turn_id(source_agent_id, original);
        obj.insert(
            "turn_id".to_string(),
            serde_json::Value::Number(global.into()),
        );
    }
    publish_event(event_bus, event_type, payload).await;
}

static GLOBAL_TURN: AtomicU64 = AtomicU64::new(0);

fn turn_remap_table() -> &'static std::sync::Mutex<Vec<((String, u64), u64)>> {
    static T: std::sync::OnceLock<std::sync::Mutex<Vec<((String, u64), u64)>>> =
        std::sync::OnceLock::new();
    T.get_or_init(|| std::sync::Mutex::new(Vec::with_capacity(64)))
}

fn remap_turn_id(agent_id: &str, original: u64) -> u64 {
    let key = (agent_id.to_string(), original);
    if let Ok(mut guard) = turn_remap_table().lock() {
        if let Some(&(_, g)) = guard.iter().rev().find(|(k, _)| *k == key) {
            return g;
        }
        let g = GLOBAL_TURN.fetch_add(1, Ordering::Relaxed) + 1;
        guard.push((key, g));
        if guard.len() > 64 {
            let drop_n = guard.len() - 64;
            guard.drain(0..drop_n);
        }
        g
    } else {
        GLOBAL_TURN.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Set active agent through the conversation gateway.
pub async fn set_active_agent(agent_id: Option<String>) {
    if let Some(conversation) = crate::conversation::Conversation::global() {
        conversation.set_focus(agent_id).await;
    }
}
