use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};

use chrono::{DateTime, Utc};
use corework::cache::{Cache, CacheExt};
use corework::event::EventBus;
use corework::execution_unit::{ExecutionUnit, UnitType};
use corework::world::FrameworkState;
use tokio::sync::{Mutex, RwLock};

use crate::conversation::{Conversation, ConversationOptions};
use crate::Result;

const MANAGER_SCOPE_ID: &str = "agent-runtime:conversation-manager";
const CONVERSATION_INDEX_RESOURCE: &str = "manager:conversation_index";
const RUNTIME_STATUS_RESOURCE: &str = "manager:runtime_status";

pub struct ConversationManager {
    unit: Arc<ExecutionUnit>,
    conversations: RwLock<HashMap<String, Arc<ConversationRuntime>>>,
    index_refresh: Mutex<()>,
    runtime_llm_request_headers: Arc<StdRwLock<BTreeMap<String, String>>>,
}

struct ConversationRuntime {
    id: String,
    tenant_id: Option<String>,
    user_id: Option<String>,
    created_at: DateTime<Utc>,
    conversation: Arc<Conversation>,
    llm_request_headers: BTreeMap<String, String>,
    runtime_llm_request_headers: Arc<StdRwLock<BTreeMap<String, String>>>,
    allow_insecure_llm_request_headers: bool,
    command_gate: ConversationCommandGate,
}

struct ConversationCommandGate {
    lock: Mutex<()>,
    closing: AtomicBool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationInfo {
    pub conversation_id: String,
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
    pub scope_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationRuntimeStatus {
    pub active_agent_id: String,
    pub agent_state: String,
    pub compacting: bool,
    pub stopping: bool,
}

impl ConversationManager {
    pub async fn new(framework: FrameworkState) -> Result<Self> {
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            MANAGER_SCOPE_ID,
        ));
        let cache = unit.cache();
        cache
            .set(
                CONVERSATION_INDEX_RESOURCE,
                &Vec::<ConversationInfo>::new(),
                None,
            )
            .await?;
        cache
            .set(RUNTIME_STATUS_RESOURCE, &"ready".to_string(), None)
            .await?;

        Ok(Self {
            unit,
            conversations: RwLock::new(HashMap::new()),
            index_refresh: Mutex::new(()),
            runtime_llm_request_headers: Arc::new(StdRwLock::new(BTreeMap::new())),
        })
    }

    pub fn unit(&self) -> &Arc<ExecutionUnit> {
        &self.unit
    }

    pub async fn create_conversation(
        &self,
        options: ConversationOptions,
        export_event_bus: Arc<dyn EventBus>,
    ) -> Result<ConversationInfo> {
        let conversation_id = options.conversation_id.trim().to_string();
        if conversation_id.is_empty() {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "conversation_id must not be empty"
            )));
        }

        {
            let conversations = self.conversations.read().await;
            if conversations.contains_key(&conversation_id) {
                return Err(crate::Error::Other(anyhow::anyhow!(
                    "conversation '{}' already exists",
                    conversation_id
                )));
            }
        }

        let conversation = Conversation::new_with_options(options, export_event_bus).await?;
        let runtime = Arc::new(ConversationRuntime::new(
            conversation,
            Arc::clone(&self.runtime_llm_request_headers),
        ));

        let mut conversations = self.conversations.write().await;
        if conversations.contains_key(&conversation_id) {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "conversation '{}' already exists",
                conversation_id
            )));
        }
        conversations.insert(conversation_id, Arc::clone(&runtime));
        drop(conversations);

        runtime.publish_llm_request_header_scope().await;
        self.refresh_index().await?;
        Ok(runtime.info())
    }

    async fn get(&self, conversation_id: &str) -> Option<Arc<ConversationRuntime>> {
        self.conversations
            .read()
            .await
            .get(conversation_id)
            .cloned()
    }

    pub async fn list(&self) -> Vec<ConversationInfo> {
        self.conversations
            .read()
            .await
            .values()
            .map(|runtime| runtime.info())
            .collect()
    }

    pub async fn close(&self, conversation_id: &str) -> Result<bool> {
        let runtime = self.get(conversation_id).await;
        let Some(runtime) = runtime else {
            return Ok(false);
        };

        let _guard = runtime.command_gate.begin_close().await;
        let removed = self.conversations.write().await.remove(conversation_id);
        if removed.is_some() {
            runtime.conversation().shutdown().await;
            self.refresh_index().await?;
        }
        Ok(removed.is_some())
    }

    pub async fn set_runtime_llm_request_headers(&self, headers: BTreeMap<String, String>) {
        match self.runtime_llm_request_headers.write() {
            Ok(mut guard) => {
                *guard = headers;
            }
            Err(_) => {
                tracing::warn!("runtime LLM request header lock is poisoned; update skipped");
            }
        }

        let runtimes = self
            .conversations
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for runtime in runtimes {
            runtime.publish_llm_request_header_scope().await;
        }
    }

    pub async fn send_message(&self, conversation_id: &str, content: &str) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        runtime.send_message(content).await
    }

    pub async fn send_message_with_admission(
        &self,
        conversation_id: &str,
        content: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        self.require_runtime(conversation_id)
            .await?
            .send_message_with_admission(content, command_id)
            .await
    }

    pub async fn request_pause_with_admission(
        &self,
        conversation_id: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        self.require_runtime(conversation_id)
            .await?
            .request_pause_with_admission(command_id)
            .await
    }

    pub async fn request_pause(&self, conversation_id: &str) -> Result<()> {
        self.require_runtime(conversation_id)
            .await?
            .request_pause()
            .await
    }

    pub async fn resolve_tool_permission(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        decision: crate::permission::ToolPermissionDecision,
    ) -> Result<bool> {
        self.require_runtime(conversation_id)
            .await?
            .resolve_tool_permission(tool_call_id, decision)
            .await
    }

    pub async fn set_focus(&self, conversation_id: &str, agent_id: String) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        runtime.conversation.set_focus_id(agent_id).await
    }

    pub async fn set_language(&self, conversation_id: &str, language: &str) -> Result<String> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        runtime.conversation.set_language(language).await
    }

    pub async fn set_summary_model(&self, conversation_id: &str, model: &str) -> Result<()> {
        self.require_runtime(conversation_id)
            .await?
            .set_summary_model(model)
            .await
    }

    pub async fn set_summary_model_with_admission(
        &self,
        conversation_id: &str,
        model: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        self.require_runtime(conversation_id)
            .await?
            .set_summary_model_with_admission(model, command_id)
            .await
    }

    pub async fn compact_history(
        &self,
        conversation_id: &str,
        agent_ids: Vec<String>,
    ) -> Result<crate::gateway::CompactHistoryReport> {
        self.require_runtime(conversation_id)
            .await?
            .compact_history(agent_ids)
            .await
    }

    pub async fn compact_history_with_admission(
        &self,
        conversation_id: &str,
        agent_ids: Vec<String>,
        command_id: Option<String>,
    ) -> Result<(
        crate::gateway::AdmissionResult,
        crate::gateway::CompactHistoryReport,
    )> {
        self.require_runtime(conversation_id)
            .await?
            .compact_history_with_admission(agent_ids, command_id)
            .await
    }

    pub async fn set_host_dynamic_snapshot_field(
        &self,
        conversation_id: &str,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> Result<()> {
        self.require_runtime(conversation_id)
            .await?
            .set_host_dynamic_snapshot_field(agent_id, field_name, text)
            .await
    }

    pub async fn frontend_state(
        &self,
        conversation_id: &str,
    ) -> Result<crate::snapshot::ConversationState> {
        self.require_runtime(conversation_id)
            .await?
            .frontend_state()
            .await
    }

    pub async fn ledger(
        &self,
        conversation_id: &str,
        options: crate::conversation_state::LedgerReadOptions,
    ) -> Result<Vec<crate::ledger::LedgerRecord>> {
        let runtime = self.require_runtime(conversation_id).await?;
        Ok(runtime.conversation.state().list_recent(options).await)
    }

    pub async fn replace_ledger(
        &self,
        conversation_id: &str,
        records: Vec<crate::ledger::LedgerRecord>,
    ) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        runtime.conversation.state().replace(records).await;
        runtime
            .conversation
            .gateway()
            .publish_ledger_snapshot()
            .await;
        Ok(())
    }

    pub async fn cluster_snapshot(
        &self,
        conversation_id: &str,
    ) -> Result<crate::agent::AgentClusterSnapshot> {
        let runtime = self.require_runtime(conversation_id).await?;
        Ok(runtime.conversation.cluster().snapshot().await)
    }

    pub async fn activate_skills(&self, conversation_id: &str, skill_names: &[&str]) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        runtime.conversation.activate_skills(skill_names).await
    }

    pub async fn register_agent(
        &self,
        conversation_id: &str,
        config: crate::AIAssistantConfig,
        skill_names: &[&str],
        max_thinking_rounds: u32,
        immutable_cache: &BTreeMap<String, String>,
    ) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        runtime
            .conversation
            .register_agent(config, skill_names, max_thinking_rounds, immutable_cache)
            .await
    }

    pub async fn default_agent_cache(&self, conversation_id: &str) -> Result<Arc<dyn Cache>> {
        let runtime = self.require_runtime(conversation_id).await?;
        let assistant = runtime.conversation.default_agent().lock().await;
        assistant.cache().ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!(
                "default agent cache is unavailable for conversation '{}'",
                conversation_id
            ))
        })
    }

    pub async fn agent_cache(
        &self,
        conversation_id: &str,
        agent_id: &str,
    ) -> Result<Arc<dyn Cache>> {
        let runtime = self.require_runtime(conversation_id).await?;
        let agent = runtime
            .conversation
            .cluster()
            .get(agent_id)
            .await
            .ok_or_else(|| {
                crate::Error::Other(anyhow::anyhow!(
                    "agent '{}' is unavailable for conversation '{}'",
                    agent_id,
                    conversation_id
                ))
            })?;
        Ok(agent.sm.unit().cache())
    }

    pub async fn agent_immutable_cache_entries(
        &self,
        conversation_id: &str,
        agent_id: &str,
    ) -> Result<BTreeMap<String, String>> {
        let runtime = self.require_runtime(conversation_id).await?;
        let agent = runtime
            .conversation
            .cluster()
            .get(agent_id)
            .await
            .ok_or_else(|| {
                crate::Error::Other(anyhow::anyhow!(
                    "agent '{}' is unavailable for conversation '{}'",
                    agent_id,
                    conversation_id
                ))
            })?;
        Ok(
            crate::context::AssistantContext::get_immutable_cache_entries(&agent.sm.unit().cache())
                .await?,
        )
    }

    pub async fn host_dynamic_snapshots(
        &self,
        conversation_id: &str,
        agent_id: &str,
    ) -> Result<HashMap<String, String>> {
        let runtime = self.require_runtime(conversation_id).await?;
        Ok(runtime
            .conversation
            .state()
            .dynamic_snapshots(agent_id)
            .await)
    }

    pub async fn agent_tasks(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::conversation_state::AgentTaskEntry>> {
        let runtime = self.require_runtime(conversation_id).await?;
        Ok(runtime.conversation.state().agent_tasks().await)
    }

    pub async fn upsert_agent_task(
        &self,
        conversation_id: &str,
        entry: crate::conversation_state::AgentTaskEntry,
    ) -> Result<crate::conversation_state::AgentTaskEntry> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        Ok(runtime.conversation.state().upsert_agent_task(entry).await)
    }

    pub async fn restore_agent_execution_entry(
        &self,
        conversation_id: &str,
        agent_id: &str,
        tools: Vec<String>,
        call_ids: Vec<String>,
        recovery_results: BTreeMap<String, crate::decision::ToolResult>,
    ) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        let agent = runtime
            .conversation
            .cluster()
            .get(agent_id)
            .await
            .ok_or_else(|| {
                crate::Error::Other(anyhow::anyhow!(
                    "agent '{}' is unavailable for conversation '{}'",
                    agent_id,
                    conversation_id
                ))
            })?;
        let cache = agent.sm.unit().cache();
        cache
            .set(crate::context::keys::PENDING_TOOLS, &tools, None)
            .await?;
        cache
            .set(crate::context::keys::PENDING_TOOL_CALL_IDS, &call_ids, None)
            .await?;
        cache
            .set(
                crate::context::keys::PENDING_TOOL_DISPLAY_COMMANDS,
                &tools,
                None,
            )
            .await?;
        cache
            .set(
                crate::context::keys::PENDING_TOOLS_WAIT_FOR_INPUT,
                &false,
                None,
            )
            .await?;
        cache
            .set(
                crate::context::keys::PENDING_TOOL_RECOVERY_RESULTS,
                &recovery_results,
                None,
            )
            .await?;
        cache
            .set(
                crate::context::keys::TASK_STATUS,
                &"running".to_string(),
                None,
            )
            .await?;
        agent
            .sm
            .recover_enter_state(crate::state::states::EXECUTING)
            .await
            .map_err(|error| crate::Error::StateMachine(error.to_string()))?;
        Ok(())
    }

    pub async fn restore_agent_state_entry(
        &self,
        conversation_id: &str,
        agent_id: &str,
        state: &str,
    ) -> Result<()> {
        let runtime = self.require_runtime(conversation_id).await?;
        let _guard = runtime.lock_command().await?;
        let agent = runtime
            .conversation
            .cluster()
            .get(agent_id)
            .await
            .ok_or_else(|| {
                crate::Error::Other(anyhow::anyhow!(
                    "agent '{}' is unavailable for conversation '{}'",
                    agent_id,
                    conversation_id
                ))
            })?;
        agent
            .sm
            .force_state(state)
            .await
            .map_err(|error| crate::Error::StateMachine(error.to_string()))?;
        Ok(())
    }

    pub async fn conversation_status(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationRuntimeStatus> {
        let runtime = self.require_runtime(conversation_id).await?;
        let cluster = runtime.conversation.cluster();
        let active_agent_id = cluster.active_agent_id().await;
        let agent = cluster.get(&active_agent_id).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!(
                "active agent '{}' is unavailable for conversation '{}'",
                active_agent_id,
                conversation_id
            ))
        })?;
        let cache = agent.sm.unit().cache();
        Ok(ConversationRuntimeStatus {
            active_agent_id,
            agent_state: agent.sm.current_state(),
            compacting: cache
                .get::<bool>(crate::context::keys::COMPACT_IN_PROGRESS)
                .await?
                .unwrap_or(false),
            stopping: cache
                .get::<bool>(crate::context::keys::PAUSE_REQUESTED)
                .await?
                .unwrap_or(false),
        })
    }

    async fn require_runtime(&self, conversation_id: &str) -> Result<Arc<ConversationRuntime>> {
        self.get(conversation_id).await.ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!(
                "conversation '{}' not found",
                conversation_id
            ))
        })
    }

    async fn refresh_index(&self) -> Result<()> {
        let _refresh = self.index_refresh.lock().await;
        let index = self.list().await;
        self.unit
            .cache()
            .set(CONVERSATION_INDEX_RESOURCE, &index, None)
            .await?;
        Ok(())
    }
}

impl ConversationRuntime {
    fn new(
        conversation: Arc<Conversation>,
        runtime_llm_request_headers: Arc<StdRwLock<BTreeMap<String, String>>>,
    ) -> Self {
        Self {
            id: conversation.conversation_id().to_string(),
            tenant_id: conversation.tenant_id().map(ToOwned::to_owned),
            user_id: conversation.user_id().map(ToOwned::to_owned),
            created_at: Utc::now(),
            llm_request_headers: conversation.llm_request_headers().clone(),
            runtime_llm_request_headers,
            allow_insecure_llm_request_headers: conversation.allow_insecure_llm_request_headers(),
            conversation,
            command_gate: ConversationCommandGate::new(),
        }
    }

    pub fn conversation(&self) -> &Arc<Conversation> {
        &self.conversation
    }

    pub fn info(&self) -> ConversationInfo {
        ConversationInfo {
            conversation_id: self.id.clone(),
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            scope_id: self.conversation.scope_id().to_string(),
            created_at: self.created_at,
        }
    }

    pub async fn send_message(&self, content: &str) -> Result<()> {
        let _guard = self.lock_command().await?;
        let headers = self.effective_llm_request_headers();
        llm_gateway::request_context::scope_request_headers(
            headers,
            self.allow_insecure_llm_request_headers,
            self.conversation.send(content),
        )
        .await
    }

    pub async fn send_message_with_admission(
        &self,
        content: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        let _guard = self.lock_command().await?;
        let headers = self.effective_llm_request_headers();
        llm_gateway::request_context::scope_request_headers(
            headers,
            self.allow_insecure_llm_request_headers,
            self.conversation.send_with_admission(content, command_id),
        )
        .await
    }

    pub async fn request_pause(&self) -> Result<()> {
        self.command_gate.ensure_open(&self.id)?;
        self.conversation.request_pause().await
    }

    pub async fn resolve_tool_permission(
        &self,
        tool_call_id: &str,
        decision: crate::permission::ToolPermissionDecision,
    ) -> Result<bool> {
        self.command_gate.ensure_open(&self.id)?;
        Ok(self
            .conversation
            .resolve_tool_permission(tool_call_id, decision)
            .await)
    }

    pub async fn request_pause_with_admission(
        &self,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        self.command_gate.ensure_open(&self.id)?;
        self.conversation
            .request_pause_with_admission(command_id)
            .await
    }

    pub async fn set_summary_model_with_admission(
        &self,
        model: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        let _guard = self.lock_command().await?;
        self.conversation
            .set_summary_model_with_admission(model, command_id)
            .await
    }

    pub async fn compact_history_with_admission(
        &self,
        agent_ids: Vec<String>,
        command_id: Option<String>,
    ) -> Result<(
        crate::gateway::AdmissionResult,
        crate::gateway::CompactHistoryReport,
    )> {
        let _guard = self.lock_command().await?;
        let headers = self.effective_llm_request_headers();
        llm_gateway::request_context::scope_request_headers(
            headers,
            self.allow_insecure_llm_request_headers,
            self.conversation
                .compact_history_with_admission(agent_ids, command_id),
        )
        .await
    }

    pub async fn set_summary_model(&self, model: &str) -> Result<()> {
        let _guard = self.lock_command().await?;
        self.conversation.set_summary_model(model).await
    }

    pub async fn compact_history(
        &self,
        agent_ids: Vec<String>,
    ) -> Result<crate::gateway::CompactHistoryReport> {
        let _guard = self.lock_command().await?;
        let headers = self.effective_llm_request_headers();
        llm_gateway::request_context::scope_request_headers(
            headers,
            self.allow_insecure_llm_request_headers,
            self.conversation.compact_history(agent_ids),
        )
        .await
    }

    pub async fn frontend_state(&self) -> Result<crate::snapshot::ConversationState> {
        self.command_gate.ensure_open(&self.id)?;
        Ok(self.conversation.frontend_state().await)
    }

    pub async fn set_host_dynamic_snapshot_field(
        &self,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> Result<()> {
        let _guard = self.lock_command().await?;
        self.conversation
            .cluster()
            .set_host_dynamic_snapshot_field(agent_id, field_name, text)
            .await
    }

    async fn lock_command(&self) -> Result<tokio::sync::MutexGuard<'_, ()>> {
        self.command_gate.lock(&self.id).await
    }

    fn effective_llm_request_headers(&self) -> BTreeMap<String, String> {
        let mut headers = self.llm_request_headers.clone();
        match self.runtime_llm_request_headers.read() {
            Ok(runtime_headers) => {
                for (name, value) in runtime_headers.iter() {
                    headers.insert(name.clone(), value.clone());
                }
            }
            Err(_) => {
                tracing::warn!(
                    "runtime LLM request header lock is poisoned; using conversation headers only"
                );
            }
        }
        headers
    }

    pub async fn publish_llm_request_header_scope(&self) {
        self.conversation
            .state()
            .set_request_headers(
                self.effective_llm_request_headers(),
                self.allow_insecure_llm_request_headers,
            )
            .await;
    }
}

impl ConversationCommandGate {
    fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            closing: AtomicBool::new(false),
        }
    }

    async fn lock(&self, conversation_id: &str) -> Result<tokio::sync::MutexGuard<'_, ()>> {
        let guard = self.lock.lock().await;
        self.ensure_open(conversation_id)?;
        Ok(guard)
    }

    fn ensure_open(&self, conversation_id: &str) -> Result<()> {
        if self.closing.load(Ordering::Acquire) {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "conversation '{}' is closing",
                conversation_id
            )));
        }
        Ok(())
    }

    async fn begin_close(&self) -> tokio::sync::MutexGuard<'_, ()> {
        let guard = self.lock.lock().await;
        self.closing.store(true, Ordering::Release);
        guard
    }
}

#[cfg(test)]
mod tests {
    use super::ConversationCommandGate;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn same_conversation_commands_are_serialized() {
        let gate = Arc::new(ConversationCommandGate::new());
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..8 {
            let gate = Arc::clone(&gate);
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            tasks.push(tokio::spawn(async move {
                let _guard = gate.lock("conversation-a").await.unwrap();
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_conversations_use_independent_command_gates() {
        let gate_a = Arc::new(ConversationCommandGate::new());
        let gate_b = Arc::new(ConversationCommandGate::new());
        let barrier = Arc::new(Barrier::new(2));

        let task_a = {
            let gate = Arc::clone(&gate_a);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                let _guard = gate.lock("conversation-a").await.unwrap();
                barrier.wait().await;
            })
        };
        let task_b = {
            let gate = Arc::clone(&gate_b);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                let _guard = gate.lock("conversation-b").await.unwrap();
                barrier.wait().await;
            })
        };

        tokio::time::timeout(Duration::from_secs(1), async {
            task_a.await.unwrap();
            task_b.await.unwrap();
        })
        .await
        .expect("different conversation gates should not block each other");
    }

    #[tokio::test]
    async fn closing_conversation_rejects_new_commands() {
        let gate = ConversationCommandGate::new();
        let close_guard = gate.begin_close().await;
        drop(close_guard);

        let error = gate.lock("conversation-a").await.unwrap_err();
        assert!(error.to_string().contains("is closing"));
    }
}
