use std::collections::HashMap;
use std::sync::Arc;

use corework::event::EventBus;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use super::runtime::{AgentId, AgentRuntime};
use crate::state::{events, states};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRuntimeSnapshot {
    pub agent_id: AgentId,
    pub agent_name: String,
    pub kind: String,
    pub state: String,
    pub permissions: super::runtime::AgentPermissions,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentClusterSnapshot {
    pub default_agent_id: AgentId,
    pub active_agent_id: AgentId,
    pub agents: Vec<AgentRuntimeSnapshot>,
}

pub struct AgentCluster {
    agents: RwLock<HashMap<AgentId, Arc<AgentRuntime>>>,
    state: Arc<crate::conversation_state::ConversationState>,
    default_agent_id: AgentId,
    event_bus: Arc<dyn EventBus>,
    drivers: Mutex<DriverState>,
}

#[derive(Default)]
struct DriverState {
    shutdown: bool,
    tasks: Vec<JoinHandle<()>>,
}

impl AgentCluster {
    pub fn new(default_agent: Arc<AgentRuntime>, event_bus: Arc<dyn EventBus>) -> Self {
        let state = default_agent
            .sm
            .unit()
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .expect("default agent must inherit ConversationState");
        let default_agent_id = default_agent.id.clone();
        let mut agents = HashMap::new();
        agents.insert(default_agent.id.clone(), default_agent);
        Self {
            agents: RwLock::new(agents),
            state,
            default_agent_id,
            event_bus,
            drivers: Mutex::new(DriverState::default()),
        }
    }

    pub async fn register(&self, agent: Arc<AgentRuntime>) {
        let snapshot = agent.persistence_snapshot().await;
        agent.persist_cache_snapshot().await;
        if let Err(e) = crate::persistence::register_agent_to_session(snapshot).await {
            tracing::warn!("register agent snapshot failed: {}", e);
        }
        self.agents.write().await.insert(agent.id.clone(), agent);
    }

    #[cfg(test)]
    pub async fn register_for_test(&self, agent: Arc<AgentRuntime>) {
        self.agents.write().await.insert(agent.id.clone(), agent);
    }

    pub async fn get(&self, id: &str) -> Option<Arc<AgentRuntime>> {
        self.agents.read().await.get(id).cloned()
    }

    /// Update one host-owned dynamic text field for a specific agent.
    pub async fn set_host_dynamic_snapshot_field(
        &self,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> crate::Result<()> {
        let agent = self.get(agent_id).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("agent '{}' not found", agent_id))
        })?;
        let state = agent
            .sm
            .unit()
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                crate::Error::Other(anyhow::anyhow!(
                    "conversation state is unavailable for agent '{}'",
                    agent_id
                ))
            })?;
        let old_len = state
            .set_dynamic_snapshot_field(agent_id, field_name, text)
            .await
            .map(|value| value.len())
            .unwrap_or(0);
        let field_count = state.dynamic_snapshots(agent_id).await.len();
        tracing::info!(
            agent_id = %agent_id,
            field_name = %field_name,
            bytes = text.len(),
            previous_bytes = old_len,
            field_count = field_count,
            "host_dynamic_snapshot_field stored"
        );
        append_host_dynamic_probe_log(&format!(
            "host_dynamic_snapshot_stored agent={} field={} bytes={} previous_bytes={} field_count={} content={}",
            agent_id,
            field_name,
            text.len(),
            old_len,
            field_count,
            truncate_probe_text(text, 1200)
        ));
        if let Err(error) = self
            .event_bus
            .publish(
                corework::event::BaseEvent::new(
                    crate::events::types::AGENT_DYNAMIC_SNAPSHOT_SET,
                    serde_json::json!({
                        "conversation_id": state.conversation_id(),
                        "agent_id": agent_id,
                        "field": field_name,
                        "text": text,
                        "host_owned": true,
                        "stale_after_restore": true,
                    }),
                )
                .with_conversation_id(state.conversation_id().to_string()),
            )
            .await
        {
            tracing::warn!(
                conversation_id = %state.conversation_id(),
                agent_id = %agent_id,
                field = %field_name,
                error = %error,
                "publish dynamic snapshot state change failed"
            );
        }
        Ok(())
    }

    pub async fn list_agent_ids(&self) -> Vec<AgentId> {
        self.agents.read().await.keys().cloned().collect()
    }

    pub async fn find_by_name_or_id(&self, value: &str) -> Option<Arc<AgentRuntime>> {
        let agents = self.agents.read().await;
        agents
            .get(value)
            .cloned()
            .or_else(|| agents.values().find(|a| a.name == value).cloned())
    }

    pub async fn list(&self) -> Vec<serde_json::Value> {
        self.agents
            .read()
            .await
            .values()
            .map(|agent| {
                serde_json::json!({
                    "id": agent.id,
                    "name": agent.name,
                    "kind": format!("{:?}", agent.kind),
                    "state": agent.sm.current_state(),
                    "can_appoint": agent.permissions.can_appoint,
                    "can_dismiss": agent.permissions.can_dismiss,
                })
            })
            .collect()
    }

    pub async fn snapshot(&self) -> AgentClusterSnapshot {
        let agents = self.agents.read().await;
        AgentClusterSnapshot {
            default_agent_id: self.default_agent_id.clone(),
            active_agent_id: self.state.focus().await,
            agents: agents
                .values()
                .map(|agent| AgentRuntimeSnapshot {
                    agent_id: agent.id.clone(),
                    agent_name: agent.name.clone(),
                    kind: format!("{:?}", agent.kind),
                    state: agent.sm.current_state(),
                    permissions: agent.permissions.clone(),
                })
                .collect(),
        }
    }

    pub async fn active_agent_id(&self) -> AgentId {
        self.state.focus().await
    }

    pub fn default_agent_id(&self) -> &str {
        &self.default_agent_id
    }

    pub(crate) async fn set_active_agent_id(&self, agent_id: AgentId) -> crate::Result<()> {
        if self.get(&agent_id).await.is_none() {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "target Agent not found: {}",
                agent_id
            )));
        }
        self.state.set_focus(agent_id).await;
        Ok(())
    }

    pub async fn active_agent(&self) -> crate::Result<Arc<AgentRuntime>> {
        let id = self.active_agent_id().await;
        self.get(&id)
            .await
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("active Agent not found: {}", id)))
    }

    pub async fn send_to_active(&self, input: &str) -> crate::Result<()> {
        if self.drivers.lock().await.shutdown {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "conversation is shutting down"
            )));
        }
        let agent = self.active_agent().await?;
        tracing::info!(
            agent_id = %agent.id,
            agent_name = %agent.name,
            state = %agent.sm.current_state(),
            input_len = input.len(),
            "agent send_to_active accepted"
        );
        agent.push_user_message(input).await?;
        if agent.sm.current_state() == states::SAYING {
            agent.sm.tick().await?;
        }
        if agent.sm.current_state() == states::SAYING
            || agent.sm.current_state() == states::SUSPENDED
        {
            agent.sm.send_event(events::USER_INPUT).await?;
        }
        let driver = std::sync::Arc::clone(&agent);
        let llm_request_headers = llm_gateway::request_context::current_request_headers();
        let allow_insecure_llm_request_headers =
            llm_gateway::request_context::allow_insecure_request_headers();
        tracing::debug!(
            agent_id = %driver.id,
            state = %driver.sm.current_state(),
            "agent driver spawn"
        );
        let handle = tokio::spawn(async move {
            tracing::debug!(
                agent_id = %driver.id,
                state = %driver.sm.current_state(),
                "agent driver start"
            );
            let result = llm_gateway::request_context::scope_request_headers(
                llm_request_headers,
                allow_insecure_llm_request_headers,
                driver.drive(None),
            )
            .await;
            if let Err(error) = result {
                tracing::warn!(
                    agent_id = %driver.id,
                    state = %driver.sm.current_state(),
                    error = %error,
                    "agent drive task failed"
                );
            } else {
                tracing::debug!(
                    agent_id = %driver.id,
                    state = %driver.sm.current_state(),
                    "agent driver finished"
                );
            }
        });
        let mut drivers = self.drivers.lock().await;
        if drivers.shutdown {
            handle.abort();
        } else {
            drivers.tasks.push(handle);
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        let tasks = {
            let mut drivers = self.drivers.lock().await;
            drivers.shutdown = true;
            std::mem::take(&mut drivers.tasks)
        };
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        self.agents.write().await.clear();
    }

    pub async fn pause_active(&self) -> crate::Result<()> {
        let id = self.active_agent_id().await;
        self.pause_agent(&id).await
    }

    pub async fn pause_agent(&self, agent_id: &str) -> crate::Result<()> {
        let agent = self.get(agent_id).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("target Agent not found: {}", agent_id))
        })?;
        agent.pause().await?;
        if let Err(e) = self
            .event_bus
            .publish(corework::event::BaseEvent::new(
                crate::events::types::AGENT_SUSPENDED,
                serde_json::json!({
                    "agent_id": agent.id,
                    "agent_name": agent.name,
                    "state": crate::state::states::SUSPENDED,
                }),
            ))
            .await
        {
            tracing::warn!(
                agent_id = %agent.id,
                error = %e,
                "publish agent suspended event failed"
            );
        }
        Ok(())
    }

    pub(crate) async fn appoint_from(
        &self,
        from: &str,
        target: &str,
        message: String,
    ) -> crate::Result<Arc<AgentRuntime>> {
        let current = self.find_by_name_or_id(from).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("source Agent not found: {}", from))
        })?;
        let target_agent = self.find_by_name_or_id(target).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("target Agent not found: {}", target))
        })?;

        if !current.permissions.can_appoint && current.id != target_agent.id {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "Agent {} is not allowed to appoint another Agent",
                current.name
            )));
        }

        if current.id != target_agent.id {
            current.pause().await?;
        }

        target_agent
            .push_agent_appointment(&current.id, &current.name, &message)
            .await?;
        Ok(target_agent)
    }

    pub(crate) async fn report_to(
        &self,
        from: &str,
        target: &str,
        report_type: &str,
        report: String,
        handoff: bool,
    ) -> crate::Result<Option<Arc<AgentRuntime>>> {
        let from_agent = self.find_by_name_or_id(from).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("report source Agent not found: {}", from))
        })?;
        let target_agent = self.find_by_name_or_id(target).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("report target Agent not found: {}", target))
        })?;

        if !from_agent.permissions.allowed_report_targets.is_empty()
            && !from_agent
                .permissions
                .allowed_report_targets
                .iter()
                .any(|id| id == &target_agent.id || id == &target_agent.name)
        {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "Agent {} is not allowed to report to {}",
                from_agent.name,
                target_agent.name
            )));
        }

        target_agent
            .push_agent_report(&from_agent.id, &from_agent.name, report_type, &report)
            .await?;
        from_agent.pause().await?;

        if handoff {
            return Ok(Some(target_agent));
        }

        Ok(None)
    }

    pub(crate) async fn dismiss(&self, target: &str) -> crate::Result<Option<AgentId>> {
        let current = self.active_agent().await?;
        if !current.permissions.can_dismiss {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "Agent {} is not allowed to dismiss another Agent",
                current.name
            )));
        }

        let target_agent = self.find_by_name_or_id(target).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("target Agent not found: {}", target))
        })?;

        if target_agent.id == self.default_agent_id {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "default Agent cannot be dismissed"
            )));
        }

        let target_agent_id = target_agent.id.clone();
        let target_agent_name = target_agent.name.clone();
        self.agents.write().await.remove(&target_agent.id);

        let fallback_focus = (self.active_agent_id().await == target_agent.id)
            .then(|| self.default_agent_id.clone());

        if let Err(e) = self
            .event_bus
            .publish(corework::event::BaseEvent::new(
                crate::events::types::AGENT_CANCELED,
                serde_json::json!({
                    "agent_id": target_agent_id,
                    "agent_name": target_agent_name,
                    "state": "canceled",
                }),
            ))
            .await
        {
            tracing::warn!(
                agent_id = %target_agent_id,
                error = %e,
                "publish agent canceled event failed"
            );
        }

        Ok(fallback_focus)
    }

    pub fn event_bus(&self) -> &Arc<dyn EventBus> {
        &self.event_bus
    }
}

fn truncate_probe_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn append_host_dynamic_probe_log(line: &str) {
    use std::io::Write;
    if !runtime_context_probe_enabled() {
        return;
    }
    let path = runtime_context_probe_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", line);
    }
}

fn runtime_context_probe_enabled() -> bool {
    matches!(
        std::env::var("RUNTIME_CONTEXT_PROBE").ok().as_deref(),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

fn runtime_context_probe_file() -> std::path::PathBuf {
    std::env::var("RUNTIME_CONTEXT_PROBE_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new("data")
                .join("logs")
                .join("runtime-context-probe.log")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentKind, AgentPermissions};
    use corework::statemachine::{FnState, StateMachine};

    async fn test_agent(id: &str) -> Arc<AgentRuntime> {
        let sm = Arc::new(
            StateMachine::builder("host_dynamic_snapshot_test_agent")
                .add_state(Box::new(
                    FnState::new(crate::state::states::SUSPENDED)
                        .with_description("test suspended"),
                ))
                .initial_state(crate::state::states::SUSPENDED)
                .build()
                .await
                .unwrap(),
        );
        sm.start().await.unwrap();
        Arc::new(AgentRuntime::new(
            id.to_string(),
            "Test Agent".to_string(),
            AgentKind::Persistent,
            sm,
            AgentPermissions::default(),
        ))
    }

    #[tokio::test]
    async fn host_dynamic_snapshot_field_is_visible_to_agent_prompt_context() {
        let _guard = crate::test_support::global_test_guard().await;
        let agent = test_agent("sunwoo-support").await;
        let state = Arc::new(crate::conversation_state::ConversationState::new(
            "snapshot-test",
            Default::default(),
            "sunwoo-support",
        ));
        agent
            .sm
            .unit()
            .attach_shared_component(Arc::clone(&state))
            .unwrap();
        let cluster = AgentCluster::new(Arc::clone(&agent), agent.sm.unit().event_bus());
        let snapshot = "[Audio conversion]\n- Format: MP3\n- Queue: empty";

        cluster
            .set_host_dynamic_snapshot_field("sunwoo-support", "sunwoo:conversion_ui", snapshot)
            .await
            .unwrap();

        let stored = state.dynamic_snapshots("sunwoo-support").await;
        assert_eq!(
            stored.get("sunwoo:conversion_ui").map(String::as_str),
            Some(snapshot)
        );

        let prompt_section = crate::systems::prompt::format_host_dynamic_snapshots_section(&stored);
        assert!(prompt_section.contains("MP3"));
        assert!(prompt_section.contains("- Format: MP3"));
        assert!(!prompt_section.contains("sunwoo:conversion_ui"));
    }
}
