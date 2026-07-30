use std::sync::Arc;

use corework::cache::CacheExt;
use corework::statemachine::StateMachine;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::context::{keys, AssistantContext};
use crate::state::{events, states};

pub type AgentId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DelegatedTaskInputDisposition {
    QueuedRunning,
    QueuedPaused,
    Resume,
}

impl DelegatedTaskInputDisposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::QueuedRunning => "queued_running",
            Self::QueuedPaused => "queued_paused",
            Self::Resume => "resumed",
        }
    }
}

/// Defines when an agent pause request may close an in-flight tool call.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentPauseMode {
    /// Preserve the current tool result and suspend at the next state-machine boundary.
    #[default]
    WaitForTool,
    /// Stop waiting for the current tool and synthesize an indeterminate result.
    DetachTool,
}

impl AgentPauseMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WaitForTool => "wait_for_tool",
            Self::DetachTool => "detach_tool",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "wait_for_tool" => Ok(Self::WaitForTool),
            "detach_tool" => Ok(Self::DetachTool),
            other => Err(format!(
                "unsupported pause mode '{other}'; expected wait_for_tool or detach_tool"
            )),
        }
    }
}

#[derive(Default)]
struct AgentExecutionControlState {
    tool_generation: u64,
    active_tool: Option<(u64, CancellationToken)>,
    detach_requested: bool,
    thinking_generation: u64,
    active_thinking: Option<(u64, CancellationToken)>,
    wait_generation: u64,
    active_wait: Option<(u64, CancellationToken)>,
    user_input_wait_wakeup_pending: bool,
}

/// Per-agent control plane for the currently executing tool batch.
///
/// The token only closes the runtime's wait. External side effects may still
/// complete, so callers must treat the synthesized result as indeterminate.
#[derive(Default)]
pub(crate) struct AgentExecutionControl {
    state: std::sync::Mutex<AgentExecutionControlState>,
}

pub(crate) struct ToolExecutionLease {
    control: Arc<AgentExecutionControl>,
    generation: u64,
    token: CancellationToken,
}

pub(crate) struct ThinkingExecutionLease {
    control: Arc<AgentExecutionControl>,
    generation: u64,
    token: CancellationToken,
}

pub(crate) struct AgentWaitLease {
    control: Arc<AgentExecutionControl>,
    generation: u64,
    token: CancellationToken,
}

impl AgentExecutionControl {
    pub(crate) fn begin_tool_batch(self: &Arc<Self>) -> ToolExecutionLease {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.tool_generation = state.tool_generation.saturating_add(1);
        let generation = state.tool_generation;
        let token = CancellationToken::new();
        if state.detach_requested {
            token.cancel();
        }
        state.active_tool = Some((generation, token.clone()));
        ToolExecutionLease {
            control: Arc::clone(self),
            generation,
            token,
        }
    }

    fn request_detach(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.detach_requested = true;
        if let Some((_, token)) = state.active_tool.as_ref() {
            token.cancel();
        }
    }

    pub(crate) fn begin_thinking_request(self: &Arc<Self>) -> ThinkingExecutionLease {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.thinking_generation = state.thinking_generation.saturating_add(1);
        let generation = state.thinking_generation;
        let token = CancellationToken::new();
        state.active_thinking = Some((generation, token.clone()));
        ThinkingExecutionLease {
            control: Arc::clone(self),
            generation,
            token,
        }
    }

    fn request_thinking_cancel(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, token)) = state.active_thinking.as_ref() {
            token.cancel();
        }
    }

    pub(crate) fn begin_agent_wait(self: &Arc<Self>) -> AgentWaitLease {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.wait_generation = state.wait_generation.saturating_add(1);
        let generation = state.wait_generation;
        let token = CancellationToken::new();
        if state.user_input_wait_wakeup_pending {
            token.cancel();
            state.user_input_wait_wakeup_pending = false;
        }
        state.active_wait = Some((generation, token.clone()));
        AgentWaitLease {
            control: Arc::clone(self),
            generation,
            token,
        }
    }

    fn notify_user_input_during_execution(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.user_input_wait_wakeup_pending = true;
        if let Some((_, token)) = state.active_wait.as_ref() {
            token.cancel();
        }
    }

    fn clear_pause_request(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.detach_requested = false;
    }
}

impl ToolExecutionLease {
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl ThinkingExecutionLease {
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl AgentWaitLease {
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for ToolExecutionLease {
    fn drop(&mut self) {
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .active_tool
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.generation)
        {
            state.active_tool = None;
        }
    }
}

impl Drop for ThinkingExecutionLease {
    fn drop(&mut self) {
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .active_thinking
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.generation)
        {
            state.active_thinking = None;
        }
    }
}

impl Drop for AgentWaitLease {
    fn drop(&mut self) {
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .active_wait
            .as_ref()
            .is_some_and(|(generation, _)| *generation == self.generation)
        {
            state.active_wait = None;
            state.user_input_wait_wakeup_pending = false;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentKind {
    Persistent,
    OneShot,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentPermissions {
    pub can_appoint: bool,
    pub can_dismiss: bool,
    pub allowed_report_targets: Vec<AgentId>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}

pub struct AgentRuntime {
    pub id: AgentId,
    /// Stable registry entry used to construct this conversation-scoped instance.
    /// Runtime snapshots persist this reference, not a copy of the registered
    /// profile, permissions, tools, or model configuration.
    pub definition_id: String,
    pub name: String,
    pub kind: AgentKind,
    pub sm: Arc<StateMachine>,
    pub permissions: AgentPermissions,
    execution_control: Arc<AgentExecutionControl>,
    dispatch_gate: tokio::sync::Mutex<()>,
}

impl AgentRuntime {
    pub fn new(
        id: AgentId,
        name: String,
        kind: AgentKind,
        sm: Arc<StateMachine>,
        permissions: AgentPermissions,
    ) -> Self {
        let definition_id = id.clone();
        Self::new_with_definition_id(id, definition_id, name, kind, sm, permissions)
    }

    pub fn new_with_definition_id(
        id: AgentId,
        definition_id: String,
        name: String,
        kind: AgentKind,
        sm: Arc<StateMachine>,
        permissions: AgentPermissions,
    ) -> Self {
        let execution_control = Arc::new(AgentExecutionControl::default());
        sm.unit()
            .attach_shared_component(Arc::clone(&execution_control))
            .expect("agent execution control must be attached exactly once");
        if let Some(conversation_state) = sm
            .unit()
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
        {
            let interrupt_source: Arc<dyn corework::wait_control::WaitInterruptSource> = Arc::new(
                super::wait_interrupt::DelegatedTaskWaitInterruptSource::new(
                    conversation_state,
                    id.clone(),
                ),
            );
            sm.unit()
                .attach_shared_component(Arc::new(
                    corework::wait_control::WaitInterruptSourceHandle::new(interrupt_source),
                ))
                .expect("agent wait interrupt source must be attached exactly once");
        }
        Self {
            id,
            definition_id,
            name,
            kind,
            sm,
            permissions,
            execution_control,
            dispatch_gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Serializes external input admission and internal wake decisions for this Agent.
    /// The guard is intentionally short lived and must not be held while `drive` runs.
    pub(crate) async fn lock_dispatch(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.dispatch_gate.lock().await
    }

    pub async fn push_user_message(&self, input: &str) -> crate::Result<()> {
        if !input.is_empty() {
            if self.sm.current_state() == states::EXECUTING {
                self.execution_control.notify_user_input_during_execution();
            }
            self.execution_control.clear_pause_request();
            let cache = self.sm.unit().cache();
            cache
                .set(keys::TASK_STATUS, &"running".to_string(), None)
                .await?;
            cache.set(keys::AUTO_CONTINUE_STEPS, &0u32, None).await?;
            cache.delete(keys::LAST_STOP_REASON).await?;
            cache.delete(keys::NEXT_STATE_AFTER_SAYING).await?;
            let event_bus = self.sm.unit().event_bus();
            AssistantContext::push_user_message_on_event_bus(&cache, &event_bus, input).await?;
        }
        Ok(())
    }

    pub(crate) async fn push_delegated_task_input(
        &self,
        task_id: &str,
        request_id: &str,
        task_revision: u64,
        question: &str,
        answer: &str,
        from_agent_id: &str,
        from_agent_name: &str,
    ) -> crate::Result<DelegatedTaskInputDisposition> {
        let content = format!(
            "[Delegated task input]\nTask: {task_id}\nRequest: {request_id}\nTask revision: {task_revision}\nQuestion: {question}\nAnswer: {answer}"
        );
        self.push_delegated_task_message(
            task_id,
            "request_id",
            request_id,
            content,
            crate::ledger::GATEWAY_SUBTYPE_AGENT_TASK_INPUT_RESPONDED,
            "task_input_response",
            from_agent_id,
            from_agent_name,
        )
        .await
    }

    pub(crate) async fn push_delegated_task_update(
        &self,
        task_id: &str,
        update_id: &str,
        task_revision: u64,
        instruction: &str,
        objective: Option<&str>,
        acceptance: Option<&[String]>,
        from_agent_id: &str,
        from_agent_name: &str,
    ) -> crate::Result<DelegatedTaskInputDisposition> {
        let mut content = format!(
            "[Delegated task update]\nTask: {task_id}\nUpdate: {update_id}\nTask revision: {task_revision}\nInstruction: {instruction}"
        );
        if let Some(objective) = objective {
            content.push_str(&format!("\nUpdated objective: {objective}"));
        }
        if let Some(acceptance) = acceptance {
            content.push_str(&format!("\nUpdated acceptance: {}", acceptance.join(", ")));
        }
        self.push_delegated_task_message(
            task_id,
            "update_id",
            update_id,
            content,
            crate::ledger::GATEWAY_SUBTYPE_AGENT_TASK_UPDATED,
            "task_updated",
            from_agent_id,
            from_agent_name,
        )
        .await
    }

    async fn push_delegated_task_message(
        &self,
        task_id: &str,
        correlation_key: &str,
        correlation_id: &str,
        content: String,
        subtype: &str,
        reason: &str,
        from_agent_id: &str,
        from_agent_name: &str,
    ) -> crate::Result<DelegatedTaskInputDisposition> {
        let cache = self.sm.unit().cache();
        let mut metadata = crate::ledger::LedgerMessageMeta {
            subtype: Some(subtype.to_string()),
            from_agent_id: Some(from_agent_id.to_string()),
            to_agent_id: Some(self.id.clone()),
            reason: Some(reason.to_string()),
            ..Default::default()
        };
        metadata.extra.insert(
            "task_id".to_string(),
            serde_json::Value::String(task_id.to_string()),
        );
        metadata.extra.insert(
            correlation_key.to_string(),
            serde_json::Value::String(correlation_id.to_string()),
        );
        metadata.extra.insert(
            "from_agent_name".to_string(),
            serde_json::Value::String(from_agent_name.to_string()),
        );
        let event_bus = self.sm.unit().event_bus();
        AssistantContext::push_message_with_metadata_and_display_on_event_bus(
            &cache,
            &event_bus,
            crate::context::Message::user(content),
            metadata,
            None,
        )
        .await?;

        let task_status: Option<String> = cache.get(keys::TASK_STATUS).await?;
        let pause_requested = cache
            .get::<bool>(keys::PAUSE_REQUESTED)
            .await?
            .unwrap_or(false);
        if pause_requested || task_status.as_deref() == Some("paused") {
            self.persist_cache_snapshot().await;
            return Ok(DelegatedTaskInputDisposition::QueuedPaused);
        }

        match self.sm.current_state().as_str() {
            states::SUSPENDED => {
                cache
                    .set(keys::TASK_STATUS, &"running".to_string(), None)
                    .await?;
                cache.delete(keys::LAST_STOP_REASON).await?;
                cache.delete(keys::NEXT_STATE_AFTER_SAYING).await?;
                self.sm.send_event(events::USER_INPUT).await?;
                self.persist_cache_snapshot().await;
                Ok(DelegatedTaskInputDisposition::Resume)
            }
            states::SAYING => {
                cache
                    .set(
                        keys::NEXT_STATE_AFTER_SAYING,
                        &states::THINKING.to_string(),
                        None,
                    )
                    .await?;
                self.persist_cache_snapshot().await;
                Ok(DelegatedTaskInputDisposition::QueuedRunning)
            }
            _ => {
                self.persist_cache_snapshot().await;
                Ok(DelegatedTaskInputDisposition::QueuedRunning)
            }
        }
    }

    pub async fn from_existing_assistant(
        id: AgentId,
        name: String,
        assistant: &crate::assistant::AIAssistant,
        permissions: AgentPermissions,
    ) -> crate::Result<Self> {
        let sm = assistant
            .state_machine()
            .ok_or_else(|| crate::Error::StateMachine("默认 Agent 状态机未初始化".to_string()))?;

        let cache = sm.unit().cache();
        cache
            .set(crate::state::agent_keys::AGENT_ID, &id, None)
            .await?;
        cache
            .set(crate::state::agent_keys::AGENT_NAME, &name, None)
            .await?;
        cache
            .set(
                crate::state::agent_keys::AGENT_CLASS,
                &"persistent".to_string(),
                None,
            )
            .await?;

        Ok(Self::new(id, name, AgentKind::Persistent, sm, permissions))
    }

    pub async fn set_conversation_id(&self, conversation_id: &str) -> crate::Result<()> {
        let cache = self.sm.unit().cache();
        crate::agent::set_conversation_id_in_cache(&*cache, conversation_id)
            .await
            .map_err(|e| crate::Error::Other(anyhow::anyhow!(e.to_string())))
    }

    pub async fn push_agent_appointment(
        &self,
        from_id: &str,
        from_name: &str,
        message: &str,
    ) -> crate::Result<()> {
        let cache = self.sm.unit().cache();
        let content = if message.trim().is_empty() {
            format!("Agent appointment from {}.", from_name)
        } else {
            format!("Agent appointment from {}:\n{}", from_name, message)
        };
        let mut metadata = crate::ledger::LedgerMessageMeta::default();
        metadata.subtype = Some(crate::ledger::GATEWAY_SUBTYPE_AGENT_APPOINTMENT.to_string());
        metadata.from_agent_id = Some(from_id.to_string());
        metadata.to_agent_id = Some(self.id.clone());
        metadata.reason = Some("appoint".to_string());
        metadata.extra.insert(
            "from_agent_name".to_string(),
            serde_json::Value::String(from_name.to_string()),
        );
        metadata.extra.insert(
            "to_agent_name".to_string(),
            serde_json::Value::String(self.name.clone()),
        );
        let event_bus = self.sm.unit().event_bus();
        AssistantContext::push_message_with_metadata_and_display_on_event_bus(
            &cache,
            &event_bus,
            crate::context::Message::user(content),
            metadata,
            None,
        )
        .await?;
        self.persist_cache_snapshot().await;
        Ok(())
    }

    pub async fn push_agent_report(
        &self,
        from_id: &str,
        from_name: &str,
        report_type: &str,
        report: &str,
    ) -> crate::Result<()> {
        let cache = self.sm.unit().cache();
        let text = format!("[Agent 复命: {}]\n{}", from_name, report);
        let display = crate::persistence::DisplayMeta {
            display_role: "agent_report".to_string(),
            tool_name: None,
            tool_command: None,
            success: None,
            reasoning: None,
            decision: None,
            tools: Vec::new(),
            agent_name: Some(from_name.to_string()),
        };
        let mut metadata = crate::ledger::LedgerMessageMeta::default();
        metadata.subtype = Some(crate::ledger::GATEWAY_SUBTYPE_AGENT_REPORT.to_string());
        metadata.from_agent_id = Some(from_id.to_string());
        metadata.to_agent_id = Some(self.id.clone());
        metadata.reason = Some(report_type.to_string());
        metadata.extra.insert(
            "from_agent_name".to_string(),
            serde_json::Value::String(from_name.to_string()),
        );
        metadata.extra.insert(
            "to_agent_name".to_string(),
            serde_json::Value::String(self.name.clone()),
        );
        let event_bus = self.sm.unit().event_bus();
        AssistantContext::push_message_with_metadata_and_display_on_event_bus(
            &cache,
            &event_bus,
            crate::context::Message {
                role: crate::context::roles::AGENT_REPORT.to_string(),
                content: text,
                cache_control: false,
                tool_call_id: None,
                name: None,
                tool_calls: None,
                reasoning_content: None,
            },
            metadata,
            Some(display),
        )
        .await?;
        self.persist_cache_snapshot().await;
        Ok(())
    }

    pub async fn pause(&self) -> crate::Result<()> {
        self.pause_with_mode(AgentPauseMode::WaitForTool).await
    }

    pub async fn pause_with_mode(&self, mode: AgentPauseMode) -> crate::Result<()> {
        let cache = self.sm.unit().cache();
        let state = self.sm.current_state();
        let task_status: Option<String> = cache.get(keys::TASK_STATUS).await?;
        let busy = state == states::THINKING
            || state == states::EXECUTING
            || task_status.as_deref() == Some("running");
        tracing::info!(
            target = "ai_assistant::pause",
            state = %state,
            busy,
            mode = mode.as_str(),
            "pause requested"
        );
        // 先发一次 stopping 快照（pause_requested=true 但 LLM/工具还未真正停下），
        // 让前端立刻把按钮切成"正在暂停"，不必等到状态机真正进入 suspended。
        if busy {
            cache.set(keys::PAUSE_REQUESTED, &true, None).await?;
            cache
                .set(keys::PAUSE_MODE, &mode.as_str().to_string(), None)
                .await?;
            let event_bus = self.sm.unit().event_bus();
            crate::agent::publish_focus_status_for_cache(
                self.sm.unit().as_ref(),
                &*cache,
                &*event_bus,
                &state,
            )
            .await;
        }
        if state == states::EXECUTING && mode == AgentPauseMode::DetachTool {
            self.execution_control.request_detach();
        }
        if state == states::THINKING {
            self.execution_control.request_thinking_cancel();
        }
        if busy {
            // The per-agent control above interrupts only this runtime. The
            // pause flag is consumed at the next state boundary.
        } else {
            crate::state::request_pause(&cache, Some(self.sm.as_ref())).await?;
        }
        let event_bus = self.sm.unit().event_bus();
        crate::agent::publish_focus_status_for_cache(
            self.sm.unit().as_ref(),
            &*cache,
            &*event_bus,
            &state,
        )
        .await;
        self.persist_cache_snapshot().await;
        Ok(())
    }

    pub async fn drive(&self, input: Option<&str>) -> crate::Result<()> {
        if let Some(input) = input {
            self.push_user_message(input).await?;
        }

        if self.sm.current_state() == states::SAYING {
            self.sm.tick().await?;
        }

        if self.sm.current_state() == states::SAYING {
            self.execution_control.clear_pause_request();
            self.sm.send_event(events::USER_INPUT).await?;
        } else if self.sm.current_state() == states::SUSPENDED {
            self.execution_control.clear_pause_request();
            // Internal wake Systems enqueue RESUME so the driver, rather than
            // the caller's task, owns the transition into Thinking.
            self.sm.tick().await?;
            if self.sm.current_state() == states::SUSPENDED {
                self.sm.send_event(events::USER_INPUT).await?;
            }
        }

        let mut spins = 0u32;
        loop {
            let cur = self.sm.current_state();
            if cur == states::SUSPENDED {
                break;
            }
            if spins >= 6000 {
                tracing::warn!(
                    "AgentRuntime {} driver: 60s 仍未稳到终态，当前状态={}",
                    self.id,
                    cur
                );
                break;
            }
            self.sm.tick().await?;
            spins += 1;
            tokio::task::yield_now().await;
        }

        self.persist_cache_snapshot().await;
        Ok(())
    }

    pub async fn pending_response(&self) -> crate::Result<String> {
        let cache = self.sm.unit().cache();
        Ok(cache
            .get::<String>(keys::PENDING_RESPONSE)
            .await?
            .unwrap_or_default())
    }

    pub async fn persistence_snapshot(&self) -> crate::persistence::AgentSnapshot {
        let cache = self.sm.unit().cache();
        let class_key = cache
            .get::<String>(crate::state::agent_keys::AGENT_CLASS)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "interactive".to_string());
        let skill_names = cache
            .get::<Vec<String>>(keys::MAIN_SKILLS)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let imported_skills = cache
            .get::<Vec<String>>(keys::IMPORTED_SKILLS)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let conversation_id = crate::agent::conversation_id_from_cache(&*cache)
            .await
            .unwrap_or_else(|| crate::ledger::DEFAULT_CONVERSATION_ID.to_string());
        let intent = crate::systems::ledger::QueryAgentLlmExecutionSnapshotSystem
            .execute(
                crate::systems::ledger::QueryAgentContextInput {
                    conversation_id,
                    agent_id: self.id.clone(),
                },
                &self.sm.unit().create_context(),
            )
            .await
            .ok()
            .and_then(|snapshot| {
                snapshot
                    .messages
                    .into_iter()
                    .find(|message| message.role == crate::context::roles::USER)
                    .map(|message| message.content)
            })
            .unwrap_or_default();

        crate::persistence::AgentSnapshot {
            id: self.id.clone(),
            name: self.name.clone(),
            class: crate::agent::AgentClass::from_str(&class_key),
            status: self.persistence_status(),
            intent,
            skill_names,
            imported_skills,
            permissions: self.permissions.clone(),
        }
    }

    pub async fn persist_cache_snapshot(&self) {
        let session_id = crate::persistence::current_session_id();
        if session_id.is_empty() {
            return;
        }
        if let Err(e) =
            crate::persistence::save_cache_snapshot(&session_id, &self.id, &self.sm.unit().cache())
                .await
        {
            tracing::warn!("save agent cache snapshot failed for {}: {}", self.id, e);
        }
    }

    fn persistence_status(&self) -> crate::agent::AgentStatus {
        match self.sm.current_state().as_str() {
            states::SUSPENDED | states::SAYING => crate::agent::AgentStatus::Idle,
            _ => crate::agent::AgentStatus::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_mode_parser_is_strict_and_backward_compatible() {
        assert_eq!(
            AgentPauseMode::parse("").unwrap(),
            AgentPauseMode::WaitForTool
        );
        assert_eq!(
            AgentPauseMode::parse("wait_for_tool").unwrap(),
            AgentPauseMode::WaitForTool
        );
        assert_eq!(
            AgentPauseMode::parse("detach_tool").unwrap(),
            AgentPauseMode::DetachTool
        );
        assert!(AgentPauseMode::parse("force").is_err());
    }

    #[test]
    fn detach_request_cancels_only_the_active_agent_tool_batch() {
        let first = Arc::new(AgentExecutionControl::default());
        let second = Arc::new(AgentExecutionControl::default());
        let first_lease = first.begin_tool_batch();
        let second_lease = second.begin_tool_batch();

        first.request_detach();

        assert!(first_lease.token().is_cancelled());
        assert!(!second_lease.token().is_cancelled());
    }

    #[test]
    fn detach_request_before_batch_start_is_not_lost() {
        let control = Arc::new(AgentExecutionControl::default());
        control.request_detach();

        let lease = control.begin_tool_batch();

        assert!(lease.token().is_cancelled());
    }

    #[test]
    fn thinking_cancel_is_scoped_to_the_target_agent() {
        let first = Arc::new(AgentExecutionControl::default());
        let second = Arc::new(AgentExecutionControl::default());
        let first_lease = first.begin_thinking_request();
        let second_lease = second.begin_thinking_request();

        first.request_thinking_cancel();

        assert!(first_lease.token().is_cancelled());
        assert!(!second_lease.token().is_cancelled());
    }

    #[test]
    fn user_input_wakes_only_the_current_agents_wait() {
        let first = Arc::new(AgentExecutionControl::default());
        let second = Arc::new(AgentExecutionControl::default());
        let first_lease = first.begin_agent_wait();
        let second_lease = second.begin_agent_wait();

        first.notify_user_input_during_execution();

        assert!(first_lease.token().is_cancelled());
        assert!(!second_lease.token().is_cancelled());
    }

    #[test]
    fn user_input_before_wait_registration_is_not_lost() {
        let control = Arc::new(AgentExecutionControl::default());
        control.notify_user_input_during_execution();

        let lease = control.begin_agent_wait();

        assert!(lease.token().is_cancelled());
    }
}
