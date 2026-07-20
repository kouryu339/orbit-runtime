use std::sync::Arc;

use corework::cache::CacheExt;
use corework::statemachine::StateMachine;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};

use crate::context::{keys, AssistantContext};
use crate::state::{events, states};

pub type AgentId = String;

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
    pub name: String,
    pub kind: AgentKind,
    pub sm: Arc<StateMachine>,
    pub permissions: AgentPermissions,
}

impl AgentRuntime {
    pub fn new(
        id: AgentId,
        name: String,
        kind: AgentKind,
        sm: Arc<StateMachine>,
        permissions: AgentPermissions,
    ) -> Self {
        Self {
            id,
            name,
            kind,
            sm,
            permissions,
        }
    }

    pub async fn push_user_message(&self, input: &str) -> crate::Result<()> {
        if !input.is_empty() {
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
        let cache = self.sm.unit().cache();
        let state = self.sm.current_state();
        let task_status: Option<String> = cache.get(keys::TASK_STATUS).await?;
        let busy = state == states::THINKING
            || state == states::EXECUTING
            || task_status.as_deref() == Some("running");
        tracing::info!(target = "ai_assistant::pause", state = %state, busy, "pause requested");
        // 先发一次 stopping 快照（pause_requested=true 但 LLM/工具还未真正停下），
        // 让前端立刻把按钮切成"正在暂停"，不必等到状态机真正进入 suspended。
        if busy {
            cache.set(keys::PAUSE_REQUESTED, &true, None).await?;
            let event_bus = self.sm.unit().event_bus();
            crate::agent::publish_focus_status_for_cache(
                self.sm.unit().as_ref(),
                &*cache,
                &*event_bus,
                &state,
            )
            .await;
        }
        if busy {
            crate::state::request_pause(&cache, None).await?;
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

        if self.sm.current_state() == states::SAYING || self.sm.current_state() == states::SUSPENDED
        {
            self.sm.send_event(events::USER_INPUT).await?;
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
