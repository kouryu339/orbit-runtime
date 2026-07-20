use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use crate::agent::{AgentCluster, AgentKind, AgentPermissions, AgentRuntime};
use crate::assistant::AIAssistant;
use crate::context::{keys, AssistantContext};
use crate::conversation_state::{ConversationRequestHeaders, ConversationState};
use crate::gateway::{AgentGateway, GatewayLedger};
use crate::permission::{PermissionBroker, ToolPermissionDecision, ToolPermissionPolicy};
use crate::persistence::AgentSnapshot;
use crate::{AIAssistantConfig, Result};
use corework::cache::{Cache, CacheExt};
use corework::event::EventBus;
use corework::event_line::EventLinePolicy;
use corework::execution_unit::{ExecutionUnit, UnitType};
use corework::world::FrameworkState;

static CONVERSATION: OnceLock<Arc<Conversation>> = OnceLock::new();
static NEXT_CONVERSATION_SEQ: AtomicU64 = AtomicU64::new(1);
const CONVERSATION_EVENT_LINE: &str = "conversation";

#[derive(Debug, Clone)]
pub struct ConversationOptions {
    pub conversation_id: String,
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
    pub llm_request_headers: BTreeMap<String, String>,
    pub allow_insecure_llm_request_headers: bool,
    pub tool_permissions: ToolPermissionPolicy,
    pub assistant_config: AIAssistantConfig,
}

impl ConversationOptions {
    pub fn new(conversation_id: impl Into<String>, assistant_config: AIAssistantConfig) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            tenant_id: None,
            user_id: None,
            llm_request_headers: BTreeMap::new(),
            allow_insecure_llm_request_headers: false,
            tool_permissions: ToolPermissionPolicy::default(),
            assistant_config,
        }
    }

    pub fn from_config(config: AIAssistantConfig) -> Self {
        Self {
            conversation_id: generate_conversation_id(),
            tenant_id: None,
            user_id: None,
            llm_request_headers: BTreeMap::new(),
            allow_insecure_llm_request_headers: false,
            tool_permissions: ToolPermissionPolicy::default(),
            assistant_config: config,
        }
    }

    pub fn conversation_scope(&self) -> String {
        match self.tenant_id.as_deref().filter(|value| !value.is_empty()) {
            Some(tenant_id) => {
                format!("tenant:{}:conversation:{}", tenant_id, self.conversation_id)
            }
            None => format!("conversation:{}", self.conversation_id),
        }
    }
}

pub struct Conversation {
    conversation_id: String,
    tenant_id: Option<String>,
    user_id: Option<String>,
    llm_request_headers: BTreeMap<String, String>,
    allow_insecure_llm_request_headers: bool,
    scope_id: String,
    state: Arc<ConversationState>,
    default_agent: Arc<Mutex<AIAssistant>>,
    ledger: GatewayLedger,
    cluster: Arc<AgentCluster>,
    gateway: Arc<AgentGateway>,
    permission_broker: Arc<PermissionBroker>,
}

impl Conversation {
    pub async fn new(
        config: AIAssistantConfig,
        export_event_bus: Arc<dyn EventBus>,
    ) -> Result<Arc<Self>> {
        Self::new_with_options(ConversationOptions::from_config(config), export_event_bus).await
    }

    pub async fn new_with_options(
        options: ConversationOptions,
        export_event_bus: Arc<dyn EventBus>,
    ) -> Result<Arc<Self>> {
        let conversation_id = options.conversation_id.trim().to_string();
        if conversation_id.is_empty() {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "conversation_id must not be empty"
            )));
        }

        let tenant_id = options.tenant_id.clone().filter(|value| !value.is_empty());
        let user_id = options.user_id.clone().filter(|value| !value.is_empty());
        let llm_request_headers = options.llm_request_headers.clone();
        let allow_insecure_llm_request_headers = options.allow_insecure_llm_request_headers;
        let scope_id = options.conversation_scope();

        let framework = FrameworkState::initialize()?;
        let ledger = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            scope_id.clone(),
        ));
        ledger.create_event_line(
            CONVERSATION_EVENT_LINE,
            EventLinePolicy::subtree_publish_owner_subscribe(),
        )?;
        ledger.set_default_event_line(CONVERSATION_EVENT_LINE)?;

        let mut assistant =
            AIAssistant::new(options.assistant_config).with_parent_unit(ledger.clone());
        assistant.init().await?;
        let default_agent_id = assistant.config().agent_id.clone();
        let default_agent_name = assistant.config().name.clone();
        let state = Arc::new(ConversationState::new(
            conversation_id.clone(),
            ConversationRequestHeaders {
                headers: llm_request_headers.clone(),
                allow_insecure: allow_insecure_llm_request_headers,
            },
            default_agent_id.clone(),
        ));
        ledger.attach_shared_component(Arc::clone(&state))?;
        ledger.attach_shared_component(Arc::clone(&ledger))?;
        let permission_broker = Arc::new(PermissionBroker::new(
            conversation_id.clone(),
            options.tool_permissions.clone(),
            ledger.event_bus(),
        ));
        ledger.attach_shared_component(Arc::clone(&permission_broker))?;

        let default_runtime = Arc::new(
            AgentRuntime::from_existing_assistant(
                default_agent_id.clone(),
                default_agent_name,
                &assistant,
                AgentPermissions {
                    can_appoint: true,
                    can_dismiss: true,
                    allowed_report_targets: Vec::new(),
                    tools: Vec::new(),
                    skills: Vec::new(),
                },
            )
            .await?,
        );
        default_runtime
            .set_conversation_id(&conversation_id)
            .await?;

        let ledger_cache = ledger.cache();
        crate::agent::set_conversation_id_in_cache(&*ledger_cache, &conversation_id).await?;
        ledger_cache
            .set(
                crate::ledger::LEDGER_RESOURCE_KEY,
                &Vec::<crate::ledger::LedgerRecord>::new(),
                None,
            )
            .await?;
        ledger_cache
            .set(crate::ledger::FOCUS_RESOURCE_KEY, &default_agent_id, None)
            .await?;
        let cluster = Arc::new(AgentCluster::new(default_runtime, ledger.event_bus()));
        ledger.attach_shared_component(Arc::clone(&cluster))?;
        let gateway = Arc::new(AgentGateway::new_for_conversation(
            conversation_id.clone(),
            Arc::clone(&cluster),
            Arc::clone(&ledger),
            Arc::clone(&state),
            export_event_bus,
        ));
        gateway.install_routes().await?;

        Ok(Arc::new(Self {
            conversation_id,
            tenant_id,
            user_id,
            llm_request_headers,
            allow_insecure_llm_request_headers,
            scope_id,
            state,
            default_agent: Arc::new(Mutex::new(assistant)),
            ledger,
            cluster,
            gateway,
            permission_broker,
        }))
    }

    pub async fn init_global(
        config: AIAssistantConfig,
        event_bus: Arc<dyn EventBus>,
    ) -> Result<()> {
        if CONVERSATION.get().is_some() {
            tracing::warn!("Conversation::init_global called more than once; ignoring");
            return Ok(());
        }
        let conv = Self::new(config, event_bus).await?;
        let _ = CONVERSATION.set(conv);
        tracing::info!("Conversation global singleton initialized");
        Ok(())
    }

    pub fn global() -> Option<&'static Arc<Conversation>> {
        CONVERSATION.get()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    pub fn llm_request_headers(&self) -> &BTreeMap<String, String> {
        &self.llm_request_headers
    }

    pub fn allow_insecure_llm_request_headers(&self) -> bool {
        self.allow_insecure_llm_request_headers
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    pub fn state(&self) -> &Arc<ConversationState> {
        &self.state
    }

    pub async fn send(&self, input: &str) -> Result<()> {
        self.gateway.send(input).await
    }

    pub async fn send_with_admission(
        &self,
        input: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        self.gateway.send_with_admission(input, command_id).await
    }

    pub async fn set_focus(&self, agent_id: Option<String>) {
        self.gateway.set_focus(agent_id).await;
    }

    pub async fn set_focus_id(&self, agent_id: String) -> Result<()> {
        self.gateway.set_focus_id(agent_id).await
    }

    pub async fn focus(&self) -> Option<String> {
        self.gateway.focus().await
    }

    pub async fn focus_id(&self) -> String {
        self.gateway.focus_id().await
    }

    pub async fn request_pause(&self) -> Result<()> {
        self.permission_broker.cancel_all().await;
        self.gateway.request_pause().await
    }

    pub async fn request_pause_with_admission(
        &self,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        self.permission_broker.cancel_all().await;
        self.gateway.request_pause_with_admission(command_id).await
    }

    pub async fn resolve_tool_permission(
        &self,
        tool_call_id: &str,
        decision: ToolPermissionDecision,
    ) -> bool {
        self.permission_broker.resolve(tool_call_id, decision).await
    }

    pub async fn reset_default_agent(&self) -> Result<()> {
        let mut guard = self.default_agent.lock().await;
        guard.reset_conversation().await
    }

    pub async fn activate_skills(&self, skill_names: &[&str]) -> Result<()> {
        let mut guard = self.default_agent.lock().await;
        guard.activate_skills(skill_names).await
    }

    pub async fn register_agent(
        &self,
        config: AIAssistantConfig,
        skill_names: &[&str],
        max_thinking_rounds: u32,
        immutable_cache: &BTreeMap<String, String>,
    ) -> Result<()> {
        if self.cluster.get(&config.agent_id).await.is_some() {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "agent '{}' already exists",
                config.agent_id
            )));
        }

        let mut assistant = AIAssistant::new(config).with_parent_unit(self.ledger.clone());
        assistant.init().await?;
        assistant.activate_skills(skill_names).await?;
        let runtime = Arc::new(
            AgentRuntime::from_existing_assistant(
                assistant.config().agent_id.clone(),
                assistant.config().name.clone(),
                &assistant,
                AgentPermissions {
                    can_appoint: true,
                    can_dismiss: true,
                    allowed_report_targets: Vec::new(),
                    tools: Vec::new(),
                    skills: skill_names.iter().map(|name| (*name).to_string()).collect(),
                },
            )
            .await?,
        );
        runtime.set_conversation_id(&self.conversation_id).await?;
        let cache = runtime.sm.unit().cache();
        cache
            .set(keys::MAX_THINKING_ROUNDS, &max_thinking_rounds, None)
            .await?;
        if !immutable_cache.is_empty() {
            cache
                .set(keys::IMMUTABLE_CACHE_ENTRIES, immutable_cache, None)
                .await?;
        }
        self.cluster.register(runtime).await;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.permission_broker.cancel_all().await;
        self.cluster.shutdown().await;
    }

    pub async fn frontend_state(&self) -> crate::snapshot::ConversationState {
        crate::snapshot::SnapshotBuilder::new()
            .build(self.conversation_id.clone(), &self.cluster, None)
            .await
            .conversation_state
    }

    pub async fn restore_session(&self) -> Result<crate::persistence::RestoreResult> {
        let guard = self.default_agent.lock().await;
        let restore = guard.restore_session().await?;
        let session_id = crate::persistence::current_session_id();
        if !session_id.is_empty() {
            match crate::persistence::load_full_session(&session_id).await {
                Ok(records) => {
                    self.gateway.restore_from_persisted(records).await;
                }
                Err(e) => {
                    tracing::warn!("replay persisted session into gateway failed: {}", e);
                }
            }
            if let Err(e) = self
                .restore_agent_runtimes(&session_id, &restore.active_agents)
                .await
            {
                tracing::warn!("restore agent runtimes failed: {}", e);
            }
            self.gateway.restore_focus_from_ledger().await;
        }
        Ok(restore)
    }

    async fn restore_agent_runtimes(
        &self,
        session_id: &str,
        snapshots: &[AgentSnapshot],
    ) -> Result<()> {
        let default_agent_id = self.cluster.default_agent_id().to_string();
        for snapshot in snapshots {
            // The default agent is initialized by AIAssistant; avoid rebuilding it.
            if snapshot.id == default_agent_id || snapshot.id == crate::agent::keys::BOSS_AGENT_ID {
                continue;
            }

            // Build a temporary suspended state machine to restore and normalize cache first.
            let bootstrap_sm = Arc::new(
                crate::state_machine::build_agent_state_machine_with_initial(
                    crate::state_machine::states::SUSPENDED,
                )
                .with_parent_unit(self.ledger.clone())
                .build()
                .await
                .map_err(|e| crate::Error::StateMachine(e.to_string()))?,
            );
            let cache = bootstrap_sm.unit().cache();
            let restored =
                crate::persistence::restore_cache_snapshot(session_id, &snapshot.id, &cache)
                    .await?;
            let event_bus = bootstrap_sm.unit().event_bus();
            Self::seed_restored_agent_cache(
                &cache,
                &event_bus,
                snapshot,
                restored,
                &self.conversation_id,
            )
            .await?;

            let initial_state = if !restored && !snapshot.intent.trim().is_empty() {
                crate::state_machine::states::THINKING
            } else {
                match Self::recovery_entry_for_cache(&cache).await {
                    crate::persistence::RecoveryEntry::Thinking => {
                        crate::state_machine::states::THINKING
                    }
                    crate::persistence::RecoveryEntry::Suspended => {
                        crate::state_machine::states::SUSPENDED
                    }
                }
            };

            let sm = if initial_state == crate::state_machine::states::SUSPENDED {
                bootstrap_sm
            } else {
                Arc::new(
                    crate::state_machine::build_agent_state_machine_with_initial(initial_state)
                        .with_parent_unit(self.ledger.clone())
                        .build()
                        .await
                        .map_err(|e| crate::Error::StateMachine(e.to_string()))?,
                )
            };
            let sm_cache = sm.unit().cache();
            crate::agent::set_conversation_id_in_cache(&*sm_cache, &self.conversation_id).await?;

            sm.start()
                .await
                .map_err(|e| crate::Error::StateMachine(e.to_string()))?;

            self.cluster
                .register(Arc::new(AgentRuntime::new(
                    snapshot.id.clone(),
                    snapshot.name.clone(),
                    AgentKind::Persistent,
                    sm,
                    snapshot.permissions.clone(),
                )))
                .await;
        }
        Ok(())
    }

    async fn seed_restored_agent_cache(
        cache: &Arc<dyn Cache>,
        event_bus: &Arc<dyn corework::event::EventBus>,
        snapshot: &AgentSnapshot,
        restored_snapshot: bool,
        conversation_id: &str,
    ) -> Result<()> {
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &snapshot.id,
                None,
            )
            .await?;
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_NAME,
                &snapshot.name,
                None,
            )
            .await?;
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_CLASS,
                &Self::agent_class_key(&snapshot.class),
                None,
            )
            .await?;
        crate::agent::set_conversation_id_in_cache(&**cache, conversation_id).await?;

        if !restored_snapshot {
            cache
                .set(keys::MAIN_SKILLS, &snapshot.skill_names, None)
                .await?;
            cache
                .set(keys::IMPORTED_SKILLS, &snapshot.imported_skills, None)
                .await?;
            if !snapshot.intent.trim().is_empty() {
                AssistantContext::push_user_message_on_event_bus(
                    cache,
                    event_bus,
                    &snapshot.intent,
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Read normalized conversation history and infer the recovery entry state.
    async fn recovery_entry_for_cache(cache: &Arc<dyn Cache>) -> crate::persistence::RecoveryEntry {
        let history = AssistantContext::get_conversation(cache)
            .await
            .unwrap_or_default();
        crate::persistence::recovery_entry_from_messages(&history)
    }

    fn agent_class_key(class: &crate::agent::AgentClass) -> String {
        match class {
            crate::agent::AgentClass::OneShot => "oneshot".to_string(),
            crate::agent::AgentClass::Interactive => "interactive".to_string(),
            crate::agent::AgentClass::Scheduled { .. } => "scheduled".to_string(),
        }
    }

    pub async fn set_model(&self, model: &str) -> Result<()> {
        let guard = self.default_agent.lock().await;
        guard.set_model(model).await
    }

    pub async fn get_model(&self) -> Result<String> {
        let guard = self.default_agent.lock().await;
        guard.get_model().await
    }

    /// Switch the summary model used by `compact_history`.
    pub async fn set_summary_model(&self, model: &str) -> Result<()> {
        llm_gateway::key_store::find_by_name(model).ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("model '{}' uid not found", model))
        })?;
        let cache = self.ledger.cache();
        cache
            .set(
                crate::config_resolver::conversation_keys::CONFIG_SUMMARY_MODEL,
                &model.to_string(),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_summary_model_with_admission(
        &self,
        model: &str,
        command_id: Option<String>,
    ) -> Result<crate::gateway::AdmissionResult> {
        let admission = self
            .gateway
            .admit_with_command_id(
                &crate::admission::Command::SetConversationSummaryModel {
                    model: model.to_string(),
                },
                command_id,
            )
            .await;
        if admission.decision.is_accepted() {
            self.set_summary_model(model).await?;
        }
        Ok(admission)
    }

    /// Compact conversation history for the whole cluster or selected agents.
    pub async fn compact_history(
        &self,
        agent_ids: Vec<String>,
    ) -> Result<crate::gateway::CompactHistoryReport> {
        self.gateway.compact_history(agent_ids).await
    }

    pub async fn compact_history_with_admission(
        &self,
        agent_ids: Vec<String>,
        command_id: Option<String>,
    ) -> Result<(
        crate::gateway::AdmissionResult,
        crate::gateway::CompactHistoryReport,
    )> {
        self.gateway
            .compact_history_with_admission(agent_ids, command_id)
            .await
    }

    /// Get the configured conversation summary model name.
    pub async fn get_summary_model(&self) -> Result<String> {
        let cache = self.ledger.cache();
        let name = cache
            .get::<String>(crate::config_resolver::conversation_keys::CONFIG_SUMMARY_MODEL)
            .await?
            .unwrap_or_default();
        Ok(name)
    }

    pub async fn set_language(&self, language: &str) -> Result<String> {
        let guard = self.default_agent.lock().await;
        let normalized = guard.set_language(language).await?;
        if let Some(agent) = self.cluster.get(self.cluster.default_agent_id()).await {
            let cache = agent.sm.unit().cache();
            cache.set(keys::LANGUAGE, &normalized, None).await?;
        }
        Ok(normalized)
    }

    pub fn default_agent(&self) -> &Arc<Mutex<AIAssistant>> {
        &self.default_agent
    }

    pub fn cluster(&self) -> &Arc<AgentCluster> {
        &self.cluster
    }

    pub fn ledger(&self) -> &GatewayLedger {
        &self.ledger
    }

    pub fn gateway(&self) -> &Arc<AgentGateway> {
        &self.gateway
    }
}

fn generate_conversation_id() -> String {
    let seq = NEXT_CONVERSATION_SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = chrono::Utc::now().timestamp_millis();
    format!("conv_{millis}_{seq}")
}
