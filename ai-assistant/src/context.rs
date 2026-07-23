//! Conversation context keys and canonical ledger message types.

use corework::cache::{Cache, CacheExt};
use corework::error::Result;
use corework::event::EventBus;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRef {
    pub id: String,
    pub name: String,
}

/// Canonical message stored in conversation history.
/// This intentionally stays separate from `llm_gateway::ChatMessage`: the
/// ledger preserves tool call ids, display metadata links, cache hints, and
/// legacy reasoning fields before the thinking state projects it into an LLM
/// request view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub cache_control: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<llm_gateway::ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(roles::USER, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(roles::ASSISTANT, content)
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(roles::SYSTEM, content)
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self::new(roles::TOOL, content)
    }

    pub fn tool_with_id(
        content: impl Into<String>,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        let mut message = Self::tool(content);
        message.tool_call_id = Some(tool_call_id.into());
        message.name = Some(name.into());
        message
    }

    fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
            cache_control: false,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
}

// ============================================================================
// ============================================================================

/// |---------------------|-----------------------|----------------------------------------|
pub mod keys {
    // ---- Skills ----

    pub const MAIN_SKILLS: &str = "main_skills";
    pub const SYSTEM_SKILLS: &str = "system_skills";

    /// Stable conversation-scoped text appended to the role persona.
    /// Hosts set this while constructing specialized conversations. It is part
    /// of the immutable system prefix, not a dynamic snapshot.
    pub const IMMUTABLE_ROLE_APPENDIX: &str = "immutable_role_appendix";

    /// Stable creation-time text entries appended to the immutable system prefix.
    /// The map key is only used for deterministic ordering and ownership; prompt
    /// assembly injects the values as text.
    pub const IMMUTABLE_CACHE_ENTRIES: &str = "immutable_cache_entries";

    /// Conversation-scoped agent profile registry injected by the host runtime.
    /// Profiles are configuration resources used when creating agents; they are
    /// not prompt memory and are not rendered into the model context by default.
    pub const AGENT_RESOURCE_PROFILES: &str = "agent_resource_profiles";

    pub const IMPORTED_SKILLS: &str = "imported_skills";

    // ---- Tools ----

    pub const ACTIVE_TOOLS: &str = "active_tools";

    pub const CONVERSATION: &str = "conversation";

    pub const COMPACT_CONVERSATION: &str = "compact_conversation";

    /// True while an explicit `compact_history` request is summarizing history.
    /// Frontends should show a non-chat status and keep user input disabled.
    pub const COMPACT_IN_PROGRESS: &str = "compact_in_progress";

    /// Opaque host-provided context passed through to RPC tool calls.
    /// Hosts use this for UI/runtime-owned integration choices such as browser backend mode.
    pub const TOOL_HOST_CONTEXT: &str = "tool_host_context";

    pub const PENDING_TOOLS: &str = "pending_tools";

    /// Parsed EXEC calls for lossless argument delivery. `PENDING_TOOLS`
    /// remains the audit/display and legacy recovery representation.
    pub const PENDING_STRUCTURED_TOOLS: &str = "pending_structured_tools";

    pub const PENDING_TOOL_DISPLAY_COMMANDS: &str = "pending_tool_display_commands";

    pub const PENDING_TOOL_CALLS: &str = "pending_tool_calls";
    pub const PENDING_TOOLS_WAIT_FOR_INPUT: &str = "pending_tools_wait_for_input";

    pub const PENDING_RESPONSE: &str = "pending_response";

    pub const PENDING_QUESTION: &str = "pending_question";

    pub const PENDING_VIEW: &str = "pending_view";

    pub const PENDING_RESULT: &str = "pending_result";

    pub const NEXT_STATE: &str = "next_state";

    /// Effective pre-thinking retrieval config for this agent.
    pub const RETRIEVAL_CONFIG: &str = "retrieval_config";

    /// Whether this agent may emit frontend widget tags.
    pub const FRONTEND_WIDGETS_ENABLED: &str = "frontend_widgets_enabled";

    /// AI-readable context returned by the pre-thinking retrieval hook.
    pub const RETRIEVAL_CONTEXT: &str = "retrieval_context";

    /// Hash of the query/profile/config used to produce `RETRIEVAL_CONTEXT`.
    pub const RETRIEVAL_QUERY_HASH: &str = "retrieval_query_hash";

    /// Thinking turn id that produced the current retrieval context.
    pub const RETRIEVAL_TURN_ID: &str = "retrieval_turn_id";

    /// Structured metadata for the most recent pre-thinking retrieval.
    pub const RETRIEVAL_LAST_ENTRY: &str = "retrieval_last_entry";

    /// saying output target. `thinking` means auto-continue; `suspended` means wait.
    pub const NEXT_STATE_AFTER_SAYING: &str = "next_state_after_saying";

    /// Long-running task status: running / waiting / done / paused / failed.
    pub const TASK_STATUS: &str = "task_status";

    /// Auto-continue step count. Reset when a new user message arrives.
    pub const AUTO_CONTINUE_STEPS: &str = "auto_continue_steps";

    /// Max auto-continue steps for one user task. Default: 20.
    pub const AUTO_CONTINUE_MAX_STEPS: &str = "auto_continue_max_steps";

    /// Last stop reason: done / waiting / max_steps / pause / error.
    pub const LAST_STOP_REASON: &str = "last_stop_reason";

    pub const WAITING_FOR_INPUT: &str = "waiting_for_input";

    pub const MAX_HISTORY_MESSAGES: &str = "max_history_messages";

    pub const MODEL: &str = "model";

    pub const MAX_THINKING_ROUNDS: &str = "max_thinking_rounds";

    pub const THINKING_ROUND_COUNT: &str = "thinking_round_count";

    pub const PAUSE_REQUESTED: &str = "pause_requested";

    pub const RECORDER_ACTIVE: &str = "recorder_active";

    pub const RECORDER_SAVED_SKILLS: &str = "recorder_saved_skills";

    pub const RECORDER_SAVED_TOOLS: &str = "recorder_saved_tools";

    /// Host-owned dynamic text fields for this agent in this conversation.
    pub const HOST_DYNAMIC_SNAPSHOTS: &str = "host_dynamic_snapshots";

    /// Workflow recorder draft chain; this is internal state, not host context.
    pub const RECORDER_CHAIN: &str = "recorder_chain";

    /// Monotonic turn id for the current conversation cache.
    pub const TURN_ID: &str = "turn_id";

    pub const CURRENT_PLAN: &str = "current_plan";
    pub const LANGUAGE: &str = "language";
    pub const PROMPTS_DIR: &str = "prompts_dir";
    pub const RUNTIME_DATA_DIR: &str = "runtime:data_dir";
    pub const PENDING_TOOL_CALL_IDS: &str = "pending_tool_call_ids";
    pub const PENDING_TOOL_RECOVERY_RESULTS: &str = "pending_tool_recovery_results";
    pub const DECISION_PROTOCOL: &str = "decision_protocol";
}

// ============================================================================

pub mod roles {
    pub const USER: &str = "user";
    pub const ASSISTANT: &str = "assistant";
    pub const SYSTEM: &str = "system";
    pub const TOOL: &str = "tool";
    pub const GATEWAY_MESSAGE: &str = "gateway_message";
    pub const SUMMARY: &str = "summary";
    pub const COMPACT_SUMMARY: &str = "compact_summary";
    pub const AGENT_REPORT: &str = "agent_report";
}

// ============================================================================

pub struct AssistantContext;

impl AssistantContext {
    // ========================================================================

    pub async fn current_turn_id(cache: &Arc<dyn Cache>) -> u64 {
        cache
            .get::<u64>(keys::TURN_ID)
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    pub async fn bump_turn_id(cache: &Arc<dyn Cache>) -> Result<u64> {
        let next = Self::current_turn_id(cache).await + 1;
        cache.set(keys::TURN_ID, &next, None).await?;
        Ok(next)
    }

    // ========================================================================

    pub async fn get_imported_skills(cache: &Arc<dyn Cache>) -> Result<Vec<String>> {
        Ok(cache
            .get::<Vec<String>>(keys::IMPORTED_SKILLS)
            .await?
            .unwrap_or_default())
    }

    pub async fn add_imported_skill(cache: &Arc<dyn Cache>, skill_name: String) -> Result<()> {
        let mut skills = Self::get_imported_skills(cache).await?;
        if !skills.contains(&skill_name) {
            skills.push(skill_name);
            cache.set(keys::IMPORTED_SKILLS, &skills, None).await?;
        }
        Ok(())
    }

    pub async fn all_active_skills(cache: &Arc<dyn Cache>) -> Result<Vec<String>> {
        let main: Vec<String> = cache
            .get::<Vec<String>>(keys::MAIN_SKILLS)
            .await?
            .unwrap_or_default();
        let imported = Self::get_imported_skills(cache).await?;
        let mut all = main;
        for s in imported {
            if !all.contains(&s) {
                all.push(s);
            }
        }
        Ok(all)
    }

    // ========================================================================

    pub async fn get_active_tools(cache: &Arc<dyn Cache>) -> Result<Vec<String>> {
        Ok(cache
            .get::<Vec<String>>(keys::ACTIVE_TOOLS)
            .await?
            .unwrap_or_default())
    }

    pub async fn set_active_tools(cache: &Arc<dyn Cache>, tools: Vec<String>) -> Result<()> {
        cache.set(keys::ACTIVE_TOOLS, &tools, None).await
    }

    pub async fn all_active_tools(cache: &Arc<dyn Cache>) -> Result<Vec<String>> {
        Self::get_active_tools(cache).await
    }

    pub fn all_registered_tool_names() -> Vec<String> {
        inventory::iter::<corework::ai_system::AISystemFactory>
            .into_iter()
            .map(|f| f.metadata.name.to_string())
            .collect()
    }

    // ========================================================================

    pub async fn get_conversation(cache: &Arc<dyn Cache>) -> Result<Vec<Message>> {
        Ok(cache
            .get::<Vec<Message>>(keys::CONVERSATION)
            .await?
            .unwrap_or_default())
    }

    pub async fn set_conversation(cache: &Arc<dyn Cache>, messages: &[Message]) -> Result<()> {
        let vec: Vec<Message> = messages
            .iter()
            .filter(|m| {
                !matches!(
                    m.role.as_str(),
                    "display" | "thinking_step" | "tool_step" | "interrupted" | "llm_error"
                )
            })
            .cloned()
            .collect();
        cache.set(keys::CONVERSATION, &vec, None).await
    }

    pub async fn push_message(cache: &Arc<dyn Cache>, message: Message) -> Result<()> {
        Self::push_message_with_display(cache, message, None).await
    }

    pub async fn push_message_on_event_bus(
        cache: &Arc<dyn Cache>,
        event_bus: &Arc<dyn EventBus>,
        message: Message,
    ) -> Result<()> {
        Self::push_message_with_display_on_event_bus(cache, event_bus, message, None).await
    }

    pub async fn push_message_with_display_on_event_bus(
        cache: &Arc<dyn Cache>,
        event_bus: &Arc<dyn EventBus>,
        message: Message,
        display: Option<crate::persistence::DisplayMeta>,
    ) -> Result<()> {
        Self::push_message_with_metadata_and_display_on_event_bus(
            cache,
            event_bus,
            message,
            crate::ledger::LedgerMessageMeta::default(),
            display,
        )
        .await
    }

    pub async fn push_message_with_display(
        cache: &Arc<dyn Cache>,
        message: Message,
        display: Option<crate::persistence::DisplayMeta>,
    ) -> Result<()> {
        Self::push_message_with_metadata_and_display(
            cache,
            message,
            crate::ledger::LedgerMessageMeta::default(),
            display,
        )
        .await
    }

    pub async fn push_message_with_metadata_and_display(
        cache: &Arc<dyn Cache>,
        message: Message,
        metadata: crate::ledger::LedgerMessageMeta,
        display: Option<crate::persistence::DisplayMeta>,
    ) -> Result<()> {
        let framework = corework::world::FrameworkState::initialize()?;
        let event_bus = framework.event_bus() as Arc<dyn EventBus>;
        Self::push_message_with_metadata_and_display_on_event_bus(
            cache, &event_bus, message, metadata, display,
        )
        .await
    }

    pub async fn push_message_with_metadata_and_display_on_event_bus(
        cache: &Arc<dyn Cache>,
        event_bus: &Arc<dyn EventBus>,
        message: Message,
        metadata: crate::ledger::LedgerMessageMeta,
        display: Option<crate::persistence::DisplayMeta>,
    ) -> Result<()> {
        let (source_id, source_name) = crate::agent::source_meta_from_cache(&**cache).await;
        let role = crate::ledger::LedgerRole::from_message_role(&message.role, display.as_ref());
        event_bus
            .publish(corework::event::BaseEvent::new(
                crate::events::types::AGENT_MESSAGE_PRODUCED,
                serde_json::to_value(crate::events::AgentMessageProducedPayload {
                    conversation_id: crate::ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id: source_id,
                    agent_name: source_name,
                    role,
                    content: message.content,
                    metadata,
                    display,
                    tool_call_id: message.tool_call_id,
                    tool_name: message.name,
                })?,
            ))
            .await?;
        Ok(())
    }

    pub async fn push_user_message(cache: &Arc<dyn Cache>, content: &str) -> Result<()> {
        Self::push_message(cache, Message::user(content)).await
    }

    pub async fn push_user_message_on_event_bus(
        cache: &Arc<dyn Cache>,
        event_bus: &Arc<dyn EventBus>,
        content: &str,
    ) -> Result<()> {
        Self::push_message_on_event_bus(cache, event_bus, Message::user(content)).await
    }

    pub async fn push_assistant_message(cache: &Arc<dyn Cache>, content: &str) -> Result<()> {
        Self::push_message(cache, Message::assistant(content)).await
    }

    pub async fn push_assistant_message_on_event_bus(
        cache: &Arc<dyn Cache>,
        event_bus: &Arc<dyn EventBus>,
        content: &str,
    ) -> Result<()> {
        Self::push_message_on_event_bus(cache, event_bus, Message::assistant(content)).await
    }

    pub async fn push_tool_message(cache: &Arc<dyn Cache>, content: &str) -> Result<()> {
        Self::push_message(cache, Message::tool(content)).await
    }

    pub async fn push_tool_message_with_display(
        cache: &Arc<dyn Cache>,
        content: &str,
        display: Option<crate::persistence::DisplayMeta>,
    ) -> Result<()> {
        Self::push_message_with_display(cache, Message::tool(content), display).await
    }

    pub async fn push_tool_message_with_ref_and_display(
        cache: &Arc<dyn Cache>,
        content: &str,
        tool_call: &ToolCallRef,
        display: Option<crate::persistence::DisplayMeta>,
    ) -> Result<()> {
        Self::push_message_with_display(
            cache,
            Message::tool_with_id(content, tool_call.id.clone(), tool_call.name.clone()),
            display,
        )
        .await
    }

    pub async fn clear_conversation(cache: &Arc<dyn Cache>) -> Result<()> {
        cache
            .set(keys::CONVERSATION, &Vec::<Message>::new(), None)
            .await
    }

    // ========================================================================
    // ========================================================================

    pub async fn get_compact_conversation(cache: &Arc<dyn Cache>) -> Result<Option<Vec<Message>>> {
        cache.get::<Vec<Message>>(keys::COMPACT_CONVERSATION).await
    }

    pub async fn set_compact_conversation(
        cache: &Arc<dyn Cache>,
        messages: &[Message],
    ) -> Result<()> {
        let vec = messages.to_vec();
        cache.set(keys::COMPACT_CONVERSATION, &vec, None).await
    }

    pub async fn clear_compact_conversation(cache: &Arc<dyn Cache>) -> Result<()> {
        cache
            .set(keys::COMPACT_CONVERSATION, &Vec::<Message>::new(), None)
            .await
    }

    // ========================================================================
    // ========================================================================

    pub async fn get_host_dynamic_snapshots(
        cache: &Arc<dyn Cache>,
    ) -> Result<HashMap<String, String>> {
        Ok(cache
            .get::<HashMap<String, String>>(keys::HOST_DYNAMIC_SNAPSHOTS)
            .await?
            .unwrap_or_default())
    }

    pub async fn get_immutable_cache_entries(
        cache: &Arc<dyn Cache>,
    ) -> Result<BTreeMap<String, String>> {
        Ok(cache
            .get::<BTreeMap<String, String>>(keys::IMMUTABLE_CACHE_ENTRIES)
            .await?
            .unwrap_or_default())
    }

    // ========================================================================
    // ========================================================================

    pub async fn get_current_plan(cache: &Arc<dyn Cache>) -> Result<Option<CurrentPlan>> {
        cache.get::<CurrentPlan>(keys::CURRENT_PLAN).await
    }

    pub async fn set_current_plan(cache: &Arc<dyn Cache>, plan: &CurrentPlan) -> Result<()> {
        cache.set(keys::CURRENT_PLAN, plan, None).await
    }

    pub async fn clear_current_plan(cache: &Arc<dyn Cache>) -> Result<()> {
        cache.delete(keys::CURRENT_PLAN).await
    }
}

// ============================================================================

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CurrentPlan {
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub content: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl CurrentPlan {
    pub const STATUS_ACTIVE: &'static str = "active";
    pub const STATUS_FINISHED: &'static str = "finished";

    pub fn is_active(&self) -> bool {
        self.status == Self::STATUS_ACTIVE
    }
}

// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_message_constructors() {
        let m = Message::user("hello");
        assert_eq!(m.role, "user");
        assert_eq!(m.content, "hello");

        let m = Message::assistant("hi");
        assert_eq!(m.role, "assistant");

        let m = Message::system("prompt");
        assert_eq!(m.role, "system");

        let m = Message::tool("result");
        assert_eq!(m.role, "tool");
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("Hello");
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.role, "user");
        assert_eq!(decoded.content, "Hello");
    }

    #[tokio::test]
    async fn assistant_context_push_inserts_global_ledger() {
        let _guard = crate::test_support::global_test_guard().await;
        let _ = crate::conversation::Conversation::init_global(
            crate::AIAssistantConfig::default(),
            Arc::new(corework::event::InMemoryEventBus::new()),
        )
        .await;
        let conversation = crate::conversation::Conversation::global().unwrap();
        conversation.state().replace(Vec::new()).await;

        let cache = {
            let assistant = conversation.default_agent().lock().await;
            assistant.cache().unwrap()
        };
        let event_bus = {
            let assistant = conversation.default_agent().lock().await;
            assistant.state_machine().unwrap().unit().event_bus()
        };
        AssistantContext::push_user_message_on_event_bus(
            &cache,
            &event_bus,
            "ledger insert from state context",
        )
        .await
        .unwrap();

        let records = conversation
            .state()
            .list_recent(crate::conversation_state::LedgerReadOptions::default())
            .await;

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].role, crate::ledger::LedgerRole::User);
        assert_eq!(records[0].content, "ledger insert from state context");
    }
}
