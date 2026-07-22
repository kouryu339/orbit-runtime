use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::Duration;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::Mutex as StdMutex,
};

use ai_assistant::{
    AIAssistantConfig, AssistantContext, ConversationInfo, ConversationManager,
    ConversationOptions, RetrievalConfig, SkillManager, SystemPromptConstraints,
};
use async_trait::async_trait;
use corework::ai_system::AISystemFactory;
use corework::cache::CacheExt;
use corework::event::{BaseEvent, EventBus, EventHandler, InMemoryEventBus};
use corework::prelude::{
    CacheBackendConfig as CoreCacheBackendConfig, CacheConfig, FrameworkState,
};
use corework::rpc_tool::RuntimeToolMetadata;
use corework::workflow::blueprint_json::BlueprintJson;
use corework::workflow::dynamic_node::DynamicExecute;
use corework::workflow::WorkflowsModule;
use llm_gateway::key_store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc, watch, RwLock as TokioRwLock};

use crate::{
    agent_test_studio::conclusion::PairConclusionHost,
    agent_test_studio::pair_builder::{PairConversationFactory, PairConversationSpec},
    agent_test_studio::pair_runtime::PairMessageSender,
    agent_test_studio::role_contract::{TargetToolContract, TargetWhiteboxContract},
    agent_test_studio::tool_runtime::AgentTestToolRuntime,
    ffi_error_code_internal, ffi_error_code_invalid_config, ffi_error_code_llm_failed,
    ffi_error_code_rpc_failed,
};

mod agent_cluster;
mod agent_test_host;
mod config;
mod conversation;
mod conversation_operations;
mod coordination;
mod events;
mod llm;
mod provider_management;
mod recovery;
mod resource_definitions;
mod resources;
mod rpc;
mod tool_definitions;
mod workflow_node_definitions;
mod workflow_operations;

pub(crate) use workflow_node_definitions::workflow_node_definition_values;

use agent_cluster::*;
pub use agent_cluster::{RuntimeAgentCluster, RuntimeAgentDefinition};
pub(crate) use agent_test_host::AgentTestRuntimeHost;
pub use config::RuntimeConfig;
pub(crate) use config::{
    config_parent_dir, parse_config_input, resolve_config_storage_dir, validate_retrieval_config,
    AgentSection, RuntimeSection,
};
#[cfg(test)]
pub(crate) use config::{validate_runtime_config, PersistenceMode, RuntimeStateBackendConfig};
use conversation::*;
pub(crate) use coordination::{
    lease_renew_interval, run_lease_renewer, LocalRuntimeCoordinationBackend,
    LocalRuntimeSequenceBackend, LocalRuntimeStateStore, RuntimeCoordinationBackend,
    RuntimeSequenceBackend, RuntimeStateStore, LOCAL_LEASE_RENEW_INTERVAL_MS, LOCAL_LEASE_TTL_MS,
};
use events::*;
use llm::*;
use recovery::*;
use resources::*;
#[allow(unused_imports)]
pub use resources::{
    ConversationLogPolicy, ResourceAgentProfileConfig, ResourceAgentsConfig, ResourceDataConfig,
    ResourceRegistration, ResourceRpcEndpointConfig, ResourceSkillsConfig, ResourceWorkflowsConfig,
    RuntimeResourceRegistry, RuntimeRpcEndpointPool,
};
use rpc::*;
pub use rpc::{RpcToolEndpointConfig, RpcToolLaunchConfig};

#[cfg(test)]
use crate::agent_test_studio::tools::{
    ADVERSARY_CONCLUDE, ADVERSARY_CREATE, ADVERSARY_DESTROY, ADVERSARY_INSPECT,
};

const CONVERSATION_CREATED_EVENT: &str = "runtime:conversation_created";
const CONVERSATION_CLOSED_EVENT: &str = "runtime:conversation_closed";
const WORKFLOW_RESOURCE_CHANGED_EVENT: &str = "workflow.resource_changed";
const WORKFLOW_EXECUTION_COMPLETED_EVENT: &str = "workflow.execution_completed";

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("runtime has not been started")]
    NotStarted,
    #[error("LLM configuration failed: {0}")]
    Llm(String),
    #[error("RPC failed: {0}")]
    Rpc(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl RuntimeError {
    pub fn code(&self) -> i32 {
        match self {
            Self::InvalidConfig(_) => ffi_error_code_invalid_config(),
            Self::Llm(_) => ffi_error_code_llm_failed(),
            Self::Rpc(_) => ffi_error_code_rpc_failed(),
            Self::NotStarted | Self::Internal(_) => ffi_error_code_internal(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversationSnapshotConsistency {
    Stable,
    BestEffort,
}

#[derive(Debug, Clone, Copy)]
struct ConversationSnapshotExportOptions {
    consistency: ConversationSnapshotConsistency,
}

impl Default for ConversationSnapshotExportOptions {
    fn default() -> Self {
        Self {
            consistency: ConversationSnapshotConsistency::Stable,
        }
    }
}

impl ConversationSnapshotExportOptions {
    fn parse(input: &str) -> Result<Self, RuntimeError> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed == "{}" {
            return Ok(Self::default());
        }
        let value: Value = serde_json::from_str(trimmed).map_err(|error| {
            RuntimeError::InvalidConfig(format!(
                "parse conversation snapshot options failed: {error}"
            ))
        })?;
        let Value::Object(options) = value else {
            return Err(RuntimeError::InvalidConfig(
                "conversation snapshot options must be an object".to_string(),
            ));
        };
        let consistency = match options.get("consistency").and_then(Value::as_str) {
            None | Some("stable") => ConversationSnapshotConsistency::Stable,
            Some("best_effort") => ConversationSnapshotConsistency::BestEffort,
            Some(other) => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation snapshot consistency '{other}' is not supported; use 'stable' or 'best_effort'"
                )))
            }
        };
        Ok(Self { consistency })
    }

    fn stable_required(self) -> bool {
        self.consistency == ConversationSnapshotConsistency::Stable
    }

    fn as_str(self) -> &'static str {
        match self.consistency {
            ConversationSnapshotConsistency::Stable => "stable",
            ConversationSnapshotConsistency::BestEffort => "best_effort",
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeRegistries {
    pub resources: Option<RuntimeResourceRegistry>,
    pub llm: Option<RuntimeLlmRegistry>,
    pub agent_clusters: BTreeMap<String, RuntimeAgentCluster>,
    pub frozen: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RuntimeLlmRegistry {
    pub id: String,
    pub current_model_uid: Option<u32>,
    pub provider_count: usize,
    pub model_count: usize,
}

pub struct RuntimeFacade {
    rt: Runtime,
    config: RuntimeConfig,
    event_bus: Arc<InMemoryEventBus>,
    state_store: Arc<dyn RuntimeStateStore>,
    sequence_backend: Arc<dyn RuntimeSequenceBackend>,
    coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    started: bool,
    conversation_manager: Option<Arc<ConversationManager>>,
    conversation_owner_leases: Arc<StdMutex<HashMap<String, ConversationOwnerLease>>>,
    conversation_instances: HashMap<String, ConversationInstanceMetadata>,
    agent_test_runtime: Arc<TokioRwLock<Option<Arc<AgentTestToolRuntime<AgentTestRuntimeHost>>>>>,
    agent_test_supervisor_conversation_id: Option<String>,
    agent_test_studio_server: Option<crate::agent_test_studio::server::AgentTestStudioServer>,
    workflow_module: Option<Arc<WorkflowsModule>>,
    workflow_studio_server: Option<crate::workflow_studio::WorkflowStudioServer>,
    workflow_studio_conversation_id: Option<String>,
    workflow_studio_projector_name: Option<String>,
    runtime_tools: Vec<RuntimeToolMetadata>,
    sidecar_children: Vec<ManagedSidecar>,
    event_sender: Arc<Mutex<Option<std_mpsc::Sender<String>>>>,
    event_log: Arc<StdMutex<VecDeque<Value>>>,
    provider_bundle: Option<ProviderBundle>,
    llm_config: llm_gateway::LlmConfig,
    ai_auth_context_headers: BTreeMap<String, String>,
    registries: RuntimeRegistries,
    builtin_clusters: Option<BuiltinClusterConfigs>,
}

impl Drop for RuntimeFacade {
    fn drop(&mut self) {
        if self.started {
            if let Err(error) = self.shutdown() {
                tracing::warn!(%error, "runtime facade drop cleanup failed");
            }
        }
    }
}

impl RuntimeFacade {
    #[allow(dead_code)]
    pub fn create_default() -> Result<Self, RuntimeError> {
        Self::create_inner(RuntimeConfig::default())
    }

    pub fn create(config_input: &str) -> Result<Self, RuntimeError> {
        let config = parse_config_input(config_input)?;
        Self::create_inner(config)
    }

    fn create_inner(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        configure_diagnostics(&config);
        ai_assistant::persistence::set_auto_file_persistence_enabled(
            config.runtime.persistence.auto_file_persistence_enabled(),
        );
        let llm_config = load_fixed_llm_config(&config)?;
        let rt = Builder::new_multi_thread()
            .enable_all()
            .thread_name("agent-runtime")
            .build()
            .map_err(|e| RuntimeError::Internal(format!("create tokio runtime failed: {e}")))?;

        let mut facade = Self {
            rt,
            config,
            event_bus: Arc::new(InMemoryEventBus::new()),
            state_store: Arc::new(LocalRuntimeStateStore::default()),
            sequence_backend: Arc::new(LocalRuntimeSequenceBackend::default()),
            coordination_backend: Arc::new(LocalRuntimeCoordinationBackend::default()),
            started: false,
            conversation_manager: None,
            conversation_owner_leases: Arc::new(StdMutex::new(HashMap::new())),
            conversation_instances: HashMap::new(),
            agent_test_runtime: Arc::new(TokioRwLock::new(None)),
            agent_test_supervisor_conversation_id: None,
            agent_test_studio_server: None,
            workflow_module: None,
            workflow_studio_server: None,
            workflow_studio_conversation_id: None,
            workflow_studio_projector_name: None,
            runtime_tools: Vec::new(),
            sidecar_children: Vec::new(),
            event_sender: Arc::new(Mutex::new(None)),
            event_log: Arc::new(StdMutex::new(VecDeque::new())),
            provider_bundle: None,
            llm_config,
            ai_auth_context_headers: BTreeMap::new(),
            registries: RuntimeRegistries::default(),
            builtin_clusters: None,
        };
        facade.sync_provider_bundle();
        Ok(facade)
    }

    fn event_metadata(&self) -> RuntimeEventMetadata {
        RuntimeEventMetadata::from_runtime(&self.config.runtime)
    }

    fn require_conversation_owner(&self, conversation_id: &str) -> Result<(), RuntimeError> {
        if !self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .contains_key(conversation_id)
        {
            tracing::warn!(
                conversation_id = %conversation_id,
                runtime_instance_id = %self.config.runtime.runtime_instance_id,
                "runtime owner lease check failed: conversation not owned by this instance"
            );
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is not owned by this runtime instance",
                conversation_id
            )));
        }
        let backend = Arc::clone(&self.coordination_backend);
        let owner = self.config.runtime.runtime_instance_id.clone();
        let cluster_id = self
            .conversation_instances
            .get(conversation_id)
            .map(|metadata| metadata.cluster_id.as_str())
            .unwrap_or(&self.config.runtime.cluster_id);
        let key = conversation_owner_lease_key(cluster_id, conversation_id);
        let ttl_ms = LOCAL_LEASE_TTL_MS;
        tracing::debug!(
            conversation_id = %conversation_id,
            owner = %owner,
            lease_key = %key,
            ttl_ms = ttl_ms,
            "runtime owner lease renew start"
        );
        let owned = self
            .rt
            .block_on(async move { backend.renew_lease(&key, &owner, ttl_ms).await })?;
        if owned {
            tracing::debug!(
                conversation_id = %conversation_id,
                runtime_instance_id = %self.config.runtime.runtime_instance_id,
                "runtime owner lease renew ok"
            );
            Ok(())
        } else {
            tracing::warn!(
                conversation_id = %conversation_id,
                runtime_instance_id = %self.config.runtime.runtime_instance_id,
                "runtime owner lease renew lost ownership"
            );
            Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' owner lease is no longer held by this runtime instance",
                conversation_id
            )))
        }
    }

    pub fn register_resources_json(&mut self, resources_json: &str) -> Result<(), RuntimeError> {
        let registration: ResourceRegistration =
            serde_json::from_str(resources_json).map_err(|error| {
                RuntimeError::InvalidConfig(format!("parse resource registration failed: {error}"))
            })?;
        let base_dir = self.config.config_dir.clone();
        self.register_resources_with_base(registration, base_dir.as_deref())
    }

    pub fn register_resources_file(&mut self, resources_path: &str) -> Result<(), RuntimeError> {
        let trimmed = resources_path.trim();
        if trimmed.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "resource registration file path must not be empty".to_string(),
            ));
        }
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return Err(RuntimeError::InvalidConfig(
                "resource registration file path must not be inline JSON".to_string(),
            ));
        }
        let path = PathBuf::from(trimmed);
        let content = fs::read_to_string(&path).map_err(|error| {
            RuntimeError::InvalidConfig(format!(
                "read resource registration file failed: {}: {error}",
                path.display()
            ))
        })?;
        let registration: ResourceRegistration =
            serde_json::from_str(&content).map_err(|error| {
                RuntimeError::InvalidConfig(format!("parse resource registration failed: {error}"))
            })?;
        let base_dir = config_parent_dir(&path);
        self.register_resources_with_base(registration, Some(&base_dir))
    }

    pub fn register_resources_input(&mut self, resources_input: &str) -> Result<(), RuntimeError> {
        let trimmed = resources_input.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            self.register_resources_json(trimmed)
        } else {
            self.register_resources_file(trimmed)
        }
    }

    #[allow(dead_code)]
    pub fn registered_resources(&self) -> Option<&RuntimeResourceRegistry> {
        self.registries.resources.as_ref()
    }

    pub fn register_llm_json(&mut self, llm_json: &str) -> Result<(), RuntimeError> {
        let registration: LlmRegistration = serde_json::from_str(llm_json).map_err(|error| {
            RuntimeError::InvalidConfig(format!("parse llm registration failed: {error}"))
        })?;
        self.register_llm(registration)?;
        self.persist_provider_config_text(llm_json)
    }

    pub fn register_llm_file(&mut self, llm_path: &str) -> Result<(), RuntimeError> {
        let trimmed = llm_path.trim();
        if trimmed.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "llm registration file path must not be empty".to_string(),
            ));
        }
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return Err(RuntimeError::InvalidConfig(
                "llm registration file path must not be inline JSON".to_string(),
            ));
        }
        let path = PathBuf::from(trimmed);
        let content = fs::read_to_string(&path).map_err(|error| {
            RuntimeError::InvalidConfig(format!(
                "read llm registration file failed: {}: {error}",
                path.display()
            ))
        })?;
        let registration: LlmRegistration = serde_json::from_str(&content).map_err(|error| {
            RuntimeError::InvalidConfig(format!("parse llm registration failed: {error}"))
        })?;
        self.register_llm(registration)?;
        self.persist_provider_config_text(&content)
    }

    pub fn register_llm_input(&mut self, llm_input: &str) -> Result<(), RuntimeError> {
        let trimmed = llm_input.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            self.register_llm_json(trimmed)
        } else {
            self.register_llm_file(trimmed)
        }
    }

    pub fn reload_llm_input(&mut self, llm_input: &str) -> Result<(), RuntimeError> {
        let content = read_json_or_file(llm_input)?;
        if let Ok(registration) = serde_json::from_str::<LlmRegistration>(&content) {
            let (registry, config) = build_llm_registry_and_config(registration)?;
            validate_llm_config(&config)?;
            validate_current_model(config.current_model_uid, &config)?;
            llm_gateway::build_index_and_resolver(config.clone());
            if let Some(uid) = config.current_model_uid {
                key_store::set_current(uid);
            }
            self.llm_config = config;
            self.sync_provider_bundle();
            self.persist_llm_config()?;
            self.registries.llm = Some(registry);
            return Ok(());
        }
        self.configure_providers(&content)
    }

    fn register_llm(&mut self, registration: LlmRegistration) -> Result<(), RuntimeError> {
        if self.started || self.registries.frozen {
            return Err(RuntimeError::InvalidConfig(
                "llm registration is frozen after runtime start".to_string(),
            ));
        }
        let (registry, config) = build_llm_registry_and_config(registration)?;
        validate_llm_config(&config)?;
        validate_current_model(config.current_model_uid, &config)?;
        llm_gateway::build_index_and_resolver(config.clone());
        if let Some(uid) = config.current_model_uid {
            key_store::set_current(uid);
        }
        self.llm_config = config;
        self.sync_provider_bundle();
        self.persist_llm_config()?;
        self.registries.llm = Some(registry);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn registered_llm(&self) -> Option<&RuntimeLlmRegistry> {
        self.registries.llm.as_ref()
    }

    pub fn register_agent_cluster_json(&mut self, cluster_json: &str) -> Result<(), RuntimeError> {
        let registration: AgentClusterRegistration =
            serde_json::from_str(cluster_json).map_err(|error| {
                RuntimeError::InvalidConfig(format!(
                    "parse agent cluster registration failed: {error}"
                ))
            })?;
        self.register_agent_cluster(registration)
    }

    pub fn register_agent_cluster_file(&mut self, cluster_path: &str) -> Result<(), RuntimeError> {
        let trimmed = cluster_path.trim();
        if trimmed.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "agent cluster registration file path must not be empty".to_string(),
            ));
        }
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return Err(RuntimeError::InvalidConfig(
                "agent cluster registration file path must not be inline JSON".to_string(),
            ));
        }
        let path = PathBuf::from(trimmed);
        let content = fs::read_to_string(&path).map_err(|error| {
            RuntimeError::InvalidConfig(format!(
                "read agent cluster registration file failed: {}: {error}",
                path.display()
            ))
        })?;
        let registration: AgentClusterRegistration =
            serde_json::from_str(&content).map_err(|error| {
                RuntimeError::InvalidConfig(format!(
                    "parse agent cluster registration failed: {error}"
                ))
            })?;
        self.register_agent_cluster(registration)
    }

    pub fn register_agent_cluster_input(
        &mut self,
        cluster_input: &str,
    ) -> Result<(), RuntimeError> {
        let trimmed = cluster_input.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            self.register_agent_cluster_json(trimmed)
        } else {
            self.register_agent_cluster_file(trimmed)
        }
    }

    fn register_agent_cluster(
        &mut self,
        registration: AgentClusterRegistration,
    ) -> Result<(), RuntimeError> {
        if self.started || self.registries.frozen {
            return Err(RuntimeError::InvalidConfig(
                "agent cluster registration is frozen after runtime start".to_string(),
            ));
        }
        let cluster = build_agent_cluster_registry(registration, &self.registries)?;
        if self.registries.agent_clusters.contains_key(&cluster.id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' is duplicated",
                cluster.id
            )));
        }
        self.registries
            .agent_clusters
            .insert(cluster.id.clone(), cluster);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn register_resources(
        &mut self,
        registration: ResourceRegistration,
    ) -> Result<(), RuntimeError> {
        let base_dir = self.config.config_dir.clone();
        self.register_resources_with_base(registration, base_dir.as_deref())
    }

    fn register_resources_with_base(
        &mut self,
        registration: ResourceRegistration,
        base_dir: Option<&Path>,
    ) -> Result<(), RuntimeError> {
        if self.started || self.registries.frozen {
            return Err(RuntimeError::InvalidConfig(
                "resource registration is frozen after runtime start".to_string(),
            ));
        }
        let registry = self
            .rt
            .block_on(build_resource_registry(registration, base_dir))?;

        self.config.runtime.skills_dir = Some(registry.skills_root_dir.clone());
        if let Some(data_dir) = registry.data_dir.clone() {
            self.config.runtime.data_dir = Some(data_dir);
        }
        if let Some(workflows_root_dir) = registry.workflows_root_dir.clone() {
            self.config.workflow.auto_load_dir = Some(workflows_root_dir);
        }
        self.config.rpc_tools = registry.rpc_pool.endpoints().cloned().collect();
        self.registries.resources = Some(registry);
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), RuntimeError> {
        if self.started {
            return Ok(());
        }
        self.builtin_clusters = Some(build_builtin_cluster_configs(&self.registries)?);
        self.registries.frozen = true;

        let mut config_for_skill_validation = self.config.clone();
        config_for_skill_validation.agents.extend(
            self.registries
                .agent_clusters
                .values()
                .flat_map(|cluster| cluster.agents.iter())
                .map(runtime_agent_definition_to_agent_section),
        );
        if let Some(builtin) = self.builtin_clusters.as_ref() {
            config_for_skill_validation.agents.extend(
                [
                    &builtin.workflow_editor,
                    &builtin.agent_test_supervisor,
                    &builtin.agent_test_adversary,
                ]
                .into_iter()
                .flat_map(|cluster| cluster.agents.iter())
                .map(runtime_agent_definition_to_agent_section),
            );
        }
        self.rt
            .block_on(async move { validate_agent_skills(&config_for_skill_validation).await })?;

        ai_assistant::prompt_assets::set_prompts_dir(
            ai_assistant::prompt_assets::default_prompts_dir(),
        );
        ai_assistant::prompt_assets::set_language(&self.config.runtime.language)
            .map_err(RuntimeError::InvalidConfig)?;
        let event_bus = Arc::clone(&self.event_bus);
        let agent_test_runtime = Arc::clone(&self.agent_test_runtime);
        let event_metadata = self.event_metadata();
        let event_sender = Arc::clone(&self.event_sender);
        let event_log = Arc::clone(&self.event_log);
        let rpc_tools = self.config.rpc_tools.clone();
        let mut retrieval_configs = self
            .config
            .agents
            .iter()
            .filter_map(|agent| agent.retrieval.clone())
            .collect::<Vec<_>>();
        retrieval_configs.extend(
            self.registries
                .agent_clusters
                .values()
                .flat_map(|cluster| cluster.agents.iter())
                .filter_map(|agent| agent.retrieval.clone()),
        );
        if let Some(resources) = self.registries.resources.as_ref() {
            retrieval_configs.extend(
                resources
                    .agent_profiles
                    .values()
                    .filter_map(|profile| profile.retrieval.clone()),
            );
        }
        let retrieval_endpoints = self.config.rpc_tools.clone();
        let retrieval_endpoint_ids = retrieval_configs
            .iter()
            .filter(|retrieval| retrieval.enabled)
            .filter_map(|retrieval| retrieval.endpoint_id.as_deref())
            .map(str::trim)
            .filter(|endpoint_id| !endpoint_id.is_empty())
            .collect::<BTreeSet<_>>();
        let rpc_tools = rpc_tools
            .into_iter()
            .filter(|endpoint| !retrieval_endpoint_ids.contains(endpoint.endpoint_id.as_str()))
            .collect::<Vec<_>>();
        let core_cache_backend_config = CoreCacheBackendConfig::Memory(CacheConfig::default());
        let core_runtime_state_config = self.config.runtime.state_backend.into_core();
        let coordination_backend: Arc<dyn RuntimeCoordinationBackend> =
            Arc::new(LocalRuntimeCoordinationBackend::default());
        let workflow_auto_load_dir = self.config.workflow.auto_load_dir.clone();
        let parent_workflow_registry = self
            .registries
            .resources
            .as_ref()
            .map(|resources| resources.workflow_registry.clone())
            .unwrap_or_default();

        let (
            manager,
            workflow_module,
            sidecar_children,
            runtime_tools,
            state_store,
            sequence_backend,
        ) = self.rt.block_on(async move {
            let framework = FrameworkState::initialize_with_runtime_state_config(
                core_cache_backend_config,
                core_runtime_state_config,
            )
            .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            tracing::info!(
                backend = ?framework.runtime_state().backend_kind(),
                "runtime state backend initialized"
            );
            framework
                .world()
                .set_resource(PARENT_WORKFLOW_REGISTRY, &parent_workflow_registry, None)
                .map_err(|e| {
                    RuntimeError::Internal(format!("install parent workflow registry failed: {e}"))
                })?;
            let state_store: Arc<dyn RuntimeStateStore> =
                Arc::new(LocalRuntimeStateStore::default());
            let sequence_backend: Arc<dyn RuntimeSequenceBackend> =
                Arc::new(LocalRuntimeSequenceBackend::default());
            let projector = Arc::new(HostEventProjector::runtime(
                Arc::clone(&sequence_backend),
                event_metadata.clone(),
            ));
            let runtime_bus: Arc<dyn EventBus> = event_bus.clone();
            let (agent_test_event_tx, mut agent_test_event_rx) = mpsc::unbounded_channel();
            install_event_forwarders(
                runtime_bus,
                event_sender.clone(),
                projector.clone(),
                event_log.clone(),
                agent_test_event_tx,
            )
            .await?;
            tokio::spawn(async move {
                while let Some(envelope) = agent_test_event_rx.recv().await {
                    let runtime = agent_test_runtime.read().await.clone();
                    if let Some(runtime) = runtime {
                        if let Err(error) = runtime.ingest(&envelope).await {
                            tracing::warn!("agent test relay event failed: {}", error);
                        }
                    }
                }
            });

            let workflow_module =
                install_workflow_runtime_from_config(workflow_auto_load_dir, event_bus.clone())?;
            let (sidecar_children, mut runtime_tools) =
                install_rpc_tools_from_config(rpc_tools).await?;
            runtime_tools.extend(install_retrieval_system_from_config(
                &retrieval_configs,
                &retrieval_endpoints,
            )?);
            let manager = Arc::new(
                ConversationManager::new(framework)
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?,
            );

            Ok::<
                (
                    Arc<ConversationManager>,
                    Arc<WorkflowsModule>,
                    Vec<ManagedSidecar>,
                    Vec<RuntimeToolMetadata>,
                    Arc<dyn RuntimeStateStore>,
                    Arc<dyn RuntimeSequenceBackend>,
                ),
                RuntimeError,
            >((
                manager,
                workflow_module,
                sidecar_children,
                runtime_tools,
                state_store,
                sequence_backend,
            ))
        })?;

        self.rt.block_on(
            manager.set_runtime_llm_request_headers(self.ai_auth_context_headers.clone()),
        );
        self.conversation_manager = Some(manager);
        self.workflow_module = Some(workflow_module);
        self.sidecar_children = sidecar_children;
        self.runtime_tools = runtime_tools;
        self.state_store = state_store;
        self.sequence_backend = sequence_backend;
        self.coordination_backend = coordination_backend;
        self.started = true;
        Ok(())
    }

    pub fn open_workflow_studio(&mut self, options_json: &str) -> Result<String, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let options = crate::workflow_studio::parse_workflow_studio_options(options_json)?;
        let target_agent =
            resolve_studio_target_agent(&self.config, &self.registries, &options.agent_id)?;
        let workflows = self
            .workflow_module
            .as_ref()
            .cloned()
            .ok_or(RuntimeError::NotStarted)?;

        let session_id = crate::workflow_studio::next_studio_session_id();
        if let Some(projector_name) = self.workflow_studio_projector_name.take() {
            self.rt
                .block_on(self.event_bus.unsubscribe(
                    corework::workflow::workflows::WORKFLOW_RESOURCE_CHANGED_EVENT,
                    &projector_name,
                ))
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        if let Some(previous_conversation_id) = self.workflow_studio_conversation_id.take() {
            if let Err(error) = self.close_conversation(&previous_conversation_id) {
                tracing::warn!(
                    conversation_id = %previous_conversation_id,
                    "close previous Workflow Studio conversation failed: {error}"
                );
            }
        }
        let editor_conversation_id = format!("{session_id}_editor");
        let mut editor_cluster = self
            .builtin_clusters
            .as_ref()
            .ok_or(RuntimeError::NotStarted)?
            .workflow_editor
            .clone();
        editor_cluster.permissions =
            crate::workflow_studio::workflow_studio_tool_permission_policy(
                &options.tool_execution_policy,
            )?;
        let editor_agent = cluster_focus_agent_section(&editor_cluster)?;
        let node_capabilities =
            crate::workflow_studio::collect_workflow_studio_node_capabilities(&self.runtime_tools);
        self.create_conversation_from_cluster_config(
            &json!({
                "schema": "agent-runtime-conversation-options/v1",
                "conversation_id": editor_conversation_id,
                "tenant_id": "workflow-studio",
                "user_id": "workflow-studio-editor"
            })
            .to_string(),
            &editor_cluster,
        )?;
        let editor_session = Arc::new(corework::workflow::workflows::WorkflowEditorSession::new(
            self.runtime_tools.clone(),
        ));
        let manager = self.manager()?;
        self.rt.block_on(async {
            manager
                .attach_shared_component(&editor_conversation_id, Arc::clone(&editor_session))
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))
        })?;
        self.configure_workflow_editor_conversation(
            &editor_conversation_id,
            &editor_agent,
            &target_agent,
            &node_capabilities,
        )?;
        let projector_name = format!("WorkflowStudioSnapshotProjector:{session_id}");
        let projector: Arc<dyn EventHandler> = Arc::new(
            crate::workflow_studio::WorkflowStudioSnapshotProjector::new(
                projector_name.clone(),
                Arc::clone(&workflows),
                Arc::clone(&editor_session),
                self.manager()?,
                editor_conversation_id.clone(),
                editor_agent.id.clone(),
            ),
        );
        self.rt
            .block_on(self.event_bus.subscribe(
                corework::workflow::workflows::WORKFLOW_RESOURCE_CHANGED_EVENT.to_string(),
                projector,
            ))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let token = crate::workflow_studio::next_studio_token();
        let chat_bridge = crate::workflow_studio::WorkflowStudioChatBridge {
            runtime_handle: self.rt.handle().clone(),
            manager: self.manager()?,
            coordination_backend: Arc::clone(&self.coordination_backend),
            cluster_id: editor_cluster.id.clone(),
            runtime_instance_id: self.config.runtime.runtime_instance_id.clone(),
        };
        let state = crate::workflow_studio::WorkflowStudioState {
            session_id: session_id.clone(),
            token: token.clone(),
            editor_conversation_id: editor_conversation_id.clone(),
            chat_bridge,
            editor_agent_id: editor_agent.id.clone(),
            editor_agent_name: editor_agent.name.clone(),
            editor_agent_role: editor_agent.role.clone(),
            config: self.config.clone(),
            runtime_tools: self.runtime_tools.clone(),
            node_capabilities,
            workflows: Arc::clone(&workflows),
            editor_session,
            readonly: options.readonly,
            workflow_name: options.workflow_name.clone(),
            tool_execution_policy: options.tool_execution_policy.clone(),
            event_log: Arc::clone(&self.event_log),
        };
        let server = crate::workflow_studio::start_workflow_studio_server(state)?;
        let result = json!({
            "schema": "agent-runtime-workflow-studio-open-result/v1",
            "url": server.url.clone(),
            "session_id": session_id,
            "editor_conversation_id": editor_conversation_id,
            "editor_agent_id": editor_agent.id,
            "workflows_dir": server.workflows_dir.to_string_lossy(),
            "readonly": options.readonly,
            "open_browser": options.open_browser,
            "tool_execution_policy": options.tool_execution_policy
        })
        .to_string();
        self.workflow_studio_server = Some(server);
        self.workflow_studio_conversation_id = Some(editor_conversation_id.clone());
        self.workflow_studio_projector_name = Some(projector_name);
        Ok(result)
    }

    pub fn open_agent_test_studio(&mut self, options_json: &str) -> Result<String, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        #[derive(Deserialize)]
        struct Options {
            #[serde(default)]
            agent_id: String,
            #[serde(default)]
            developer_brief: String,
        }
        let options = if options_json.trim().is_empty() {
            Options {
                agent_id: String::new(),
                developer_brief: String::new(),
            }
        } else {
            serde_json::from_str::<Options>(options_json).map_err(|error| {
                RuntimeError::InvalidConfig(format!(
                    "parse Agent Test Studio options failed: {error}"
                ))
            })?
        };
        let target_agent =
            resolve_studio_target_agent(&self.config, &self.registries, &options.agent_id)?;
        self.agent_test_studio_server = None;
        let previous_runtime = self
            .rt
            .block_on(async { self.agent_test_runtime.write().await.take() });
        self.rt
            .block_on(crate::agent_test_studio::systems::clear_runtime());
        if let Some(previous_runtime) = previous_runtime {
            self.rt.block_on(previous_runtime.shutdown())?;
        }
        if let Some(previous) = self.agent_test_supervisor_conversation_id.take() {
            if let Err(error) = self.close_conversation(&previous) {
                tracing::warn!(
                    conversation_id = %previous,
                    "close previous Agent Test supervisor failed: {error}"
                );
            }
        }

        static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
        let session_id = format!(
            "agent-test-session-{:04}",
            NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
        );
        let supervisor_conversation_id = format!("{session_id}:supervisor");
        let builtin_clusters = self
            .builtin_clusters
            .as_ref()
            .ok_or(RuntimeError::NotStarted)?
            .clone();
        let supervisor_agent =
            cluster_focus_agent_section(&builtin_clusters.agent_test_supervisor)?;
        self.create_conversation_from_cluster_config(
            &json!({
                "schema": "agent-runtime-conversation-options/v1",
                "conversation_id": supervisor_conversation_id,
                "tenant_id": "agent-test-studio",
                "user_id": "agent-test-supervisor"
            })
            .to_string(),
            &builtin_clusters.agent_test_supervisor,
        )?;

        let contract = self.rt.block_on(build_target_whitebox_contract(
            &target_agent,
            options.developer_brief,
            &self.runtime_tools,
        ))?;
        let appendix = format!(
            "# Immutable Target Whitebox Contract\n\n{}",
            serde_json::to_string_pretty(&contract)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?
        );
        if let Err(error) =
            self.set_conversation_immutable_role_appendix(&supervisor_conversation_id, &appendix)
        {
            let _ = self.close_conversation(&supervisor_conversation_id);
            return Err(error);
        }

        let host = AgentTestRuntimeHost {
            config: self.config.clone(),
            target_agent: target_agent.clone(),
            adversary_cluster: builtin_clusters.agent_test_adversary,
            manager: self.manager()?,
            export_event_bus: self.event_bus.clone(),
            state_store: Arc::clone(&self.state_store),
            coordination_backend: Arc::clone(&self.coordination_backend),
            conversation_owner_leases: Arc::clone(&self.conversation_owner_leases),
            logs_dir: self
                .registries
                .resources
                .as_ref()
                .and_then(|resources| resources.logs_dir.clone()),
            conversation_log_policy: self
                .registries
                .resources
                .as_ref()
                .map(|resources| resources.conversation_log_policy.clone())
                .unwrap_or_default(),
            conversation_logs: Arc::new(StdMutex::new(HashMap::new())),
        };
        let tool_runtime = Arc::new(AgentTestToolRuntime::new(target_agent.id.clone(), host));
        self.rt
            .block_on(crate::agent_test_studio::systems::bind_runtime(Arc::clone(
                &tool_runtime,
            )));
        let token = crate::workflow_studio::next_studio_token();
        let server_state = crate::agent_test_studio::server::AgentTestStudioState {
            token,
            session_id: session_id.clone(),
            target_agent_id: target_agent.id.clone(),
            target_name: target_agent.name.clone(),
            supervisor_name: supervisor_agent.name.clone(),
            supervisor_conversation_id: supervisor_conversation_id.clone(),
            runtime_handle: self.rt.handle().clone(),
            manager: self.manager()?,
            runtime: Arc::clone(&tool_runtime),
            event_log: Arc::clone(&self.event_log),
        };
        let server =
            crate::agent_test_studio::server::start_agent_test_studio_server(server_state)?;
        self.rt.block_on(async {
            *self.agent_test_runtime.write().await = Some(tool_runtime);
        });
        self.agent_test_supervisor_conversation_id = Some(supervisor_conversation_id.clone());

        let result = json!({
            "schema": "agent-runtime-agent-test-studio-open-result/v1",
            "url": server.url,
            "session_id": session_id,
            "supervisor_conversation_id": supervisor_conversation_id,
            "supervisor_agent_id": supervisor_agent.id,
            "target_agent_id": target_agent.id
        })
        .to_string();
        self.agent_test_studio_server = Some(server);
        Ok(result)
    }

    pub fn set_event_sender(&mut self, sender: std_mpsc::Sender<String>) {
        match self.event_sender.lock() {
            Ok(mut guard) => {
                *guard = Some(sender);
            }
            Err(error) => {
                tracing::warn!(error = %error, "event sender mutex poisoned; event sender not installed");
            }
        }
    }

    pub fn shutdown(&mut self) -> Result<(), RuntimeError> {
        self.workflow_studio_server = None;
        self.agent_test_studio_server = None;

        let agent_test_runtime = self
            .rt
            .block_on(async { self.agent_test_runtime.write().await.take() });
        self.rt
            .block_on(crate::agent_test_studio::systems::clear_runtime());
        if let Some(agent_test_runtime) = agent_test_runtime {
            if let Err(error) = self.rt.block_on(agent_test_runtime.shutdown()) {
                tracing::warn!("shutdown Agent Test runtime failed: {error}");
            }
        }

        let conversation_ids = if let Some(manager) = self.conversation_manager.clone() {
            self.rt
                .block_on(async move { manager.list().await })
                .into_iter()
                .map(|info| info.conversation_id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut first_error = None;
        for conversation_id in conversation_ids {
            if let Err(error) = self.close_conversation(&conversation_id) {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    "close conversation during runtime shutdown failed: {error}"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        let remaining_leases = {
            let mut leases = self
                .conversation_owner_leases
                .lock()
                .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?;
            std::mem::take(&mut *leases)
        };
        for (conversation_id, lease) in remaining_leases {
            if let Err(error) = self
                .rt
                .block_on(lease.stop_and_release(Arc::clone(&self.coordination_backend)))
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    "release remaining conversation lease failed: {error}"
                );
            }
        }

        if let Some(projector_name) = self.workflow_studio_projector_name.take() {
            if let Err(error) = self.rt.block_on(self.event_bus.unsubscribe(
                corework::workflow::workflows::WORKFLOW_RESOURCE_CHANGED_EVENT,
                &projector_name,
            )) {
                tracing::warn!(%error, "unsubscribe Workflow Studio snapshot projector failed");
            }
        }
        self.workflow_studio_conversation_id = None;
        self.agent_test_supervisor_conversation_id = None;
        self.conversation_instances.clear();
        self.sidecar_children.clear();
        self.runtime_tools.clear();
        self.workflow_module = None;
        self.conversation_manager = None;
        self.started = false;
        if let Ok(mut sender) = self.event_sender.lock() {
            *sender = None;
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn assistant_config_for_agent(&self, agent: &AgentSection) -> AIAssistantConfig {
        assistant_config_for_runtime(&self.config, agent)
    }

    fn active_skill_names_for_agent(&self, agent: &AgentSection) -> Vec<String> {
        active_skill_names(agent)
    }

    fn configure_workflow_editor_conversation(
        &self,
        editor_conversation_id: &str,
        editor_agent: &AgentSection,
        target_agent: &AgentSection,
        node_capabilities: &[Value],
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = editor_conversation_id.to_string();
        let editor_agent_id = editor_agent.id.clone();
        let node_capabilities = node_capabilities.to_vec();
        let mut target_skill_refs = Vec::new();
        for skill in self.active_skill_names_for_agent(target_agent) {
            if !target_skill_refs.contains(&skill) {
                target_skill_refs.push(skill);
            }
        }
        let target_agent_context = json!({
            "schema": "workflow-studio-target-agent/v1",
            "id": target_agent.id,
            "name": target_agent.name,
            "role": target_agent.role,
            "features": target_agent.features,
            "skill_refs": target_skill_refs,
            "is_default": target_agent.is_default,
        });
        let editor_callable_node_tools: Vec<String> = node_capabilities
            .iter()
            .filter(|capability| {
                capability
                    .get("editor_callable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .filter_map(|capability| capability.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        let editor_tools = json!([
            "listWorkflows",
            "readWorkflow",
            "createWorkflowDraft",
            "updateWorkflow",
            "compileWorkflow",
            "testWorkflow",
            "registerWorkflow",
            "deleteWorkflow",
            "executeWorkflow",
            "executeWorkflowScript",
            "searchSkillRefs"
        ]);
        let studio_context = json!({
            "schema": "workflow-editor-context/v1",
            "editor_agent_id": editor_agent.id,
            "target_agent": target_agent_context,
            "language": self.config.runtime.language,
            "workflows_dir": self
                .workflow_module
                .as_ref()
                .map(|module| module.workflows_dir().to_string_lossy().to_string())
                .unwrap_or_default(),
            "editor_tools": editor_tools,
            "node_capabilities": node_capabilities,
            "tool_execution_policy": "Studio internal editing tools are the authoritative way to mutate drafts; node-capable tools are available for probing and workflow execution."
        })
        .to_string();

        self.rt.block_on(async move {
            let target_tool_names =
                if let Some(skill_manager) = ai_assistant::skills::systems::SKILL_MANAGER.get() {
                    let refs: Vec<&str> = target_skill_refs.iter().map(String::as_str).collect();
                    let mut manager = skill_manager.write().await;
                    let _ = manager.load_many(&refs).await;
                    manager.collect_tools_for_skills(&refs)
                } else {
                    Vec::new()
                };

            let cache = manager
                .default_agent_cache(&conversation_id)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let mut active_tools = AssistantContext::get_active_tools(&cache)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            for tool in editor_callable_node_tools {
                if !active_tools.contains(&tool) {
                    active_tools.push(tool);
                }
            }
            for tool in &target_tool_names {
                if !active_tools.contains(tool) {
                    active_tools.push(tool.clone());
                }
            }
            active_tools.sort();
            active_tools.dedup();
            AssistantContext::set_active_tools(&cache, active_tools)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set(
                    "workflow_studio.reference_skill_names",
                    &target_skill_refs,
                    None,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set("workflow_studio.target_agent", &target_agent_context, None)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set(
                    "workflow_studio.target_skill_refs",
                    &target_skill_refs,
                    None,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set(
                    "workflow_studio.target_tool_names",
                    &target_tool_names,
                    None,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set("workflow_studio.editor_context", &studio_context, None)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set("workflow_studio.editor_tools", &editor_tools, None)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set(
                    "workflow_studio.node_capabilities",
                    &node_capabilities,
                    None,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &editor_agent_id,
                    "workflow_studio.editor_context",
                    &studio_context,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &editor_agent_id,
                    "workflow_studio.editor_tools",
                    &serde_json::to_string(&editor_tools)
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &editor_agent_id,
                    "workflow_studio.target_agent",
                    &serde_json::to_string(&target_agent_context)
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &editor_agent_id,
                    "workflow_studio.target_skill_refs",
                    &serde_json::to_string(&target_skill_refs)
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &editor_agent_id,
                    "workflow_studio.target_tool_names",
                    &serde_json::to_string(&target_tool_names)
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &editor_agent_id,
                    "workflow_studio.node_capabilities",
                    &serde_json::to_string(&node_capabilities)
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?,
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            Ok::<(), RuntimeError>(())
        })
    }

    fn manager(&self) -> Result<Arc<ConversationManager>, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        self.conversation_manager
            .clone()
            .ok_or(RuntimeError::NotStarted)
    }
}

#[async_trait]
impl PairConversationFactory for RuntimeFacade {
    async fn create_target_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError> {
        let agent = self.config.agent_by_id_or_default(&spec.agent_id)?.clone();
        let skills = self.active_skill_names_for_agent(&agent);
        self.create_internal_conversation(
            &json!({
                "schema": "agent-runtime-conversation-options/v1",
                "conversation_id": spec.conversation_id,
            })
            .to_string(),
            &agent,
            skills,
            "agent-test-target",
            "Agent Test target conversation",
        )?;
        Ok(())
    }

    async fn create_adversary_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError> {
        let adversary_cluster = self
            .builtin_clusters
            .as_ref()
            .ok_or(RuntimeError::NotStarted)?
            .agent_test_adversary
            .clone();
        let create_result = self.create_conversation_from_cluster_config(
            &json!({
                "schema": "agent-runtime-conversation-options/v1",
                "conversation_id": spec.conversation_id,
            })
            .to_string(),
            &adversary_cluster,
        );
        create_result?;

        let appendix = spec.immutable_role_appendix.as_deref().ok_or_else(|| {
            RuntimeError::InvalidConfig(
                "adversary conversation requires an immutable role appendix".to_string(),
            )
        })?;
        if let Err(error) =
            self.set_conversation_immutable_role_appendix(&spec.conversation_id, appendix)
        {
            let _ = RuntimeFacade::close_conversation(self, &spec.conversation_id);
            return Err(error);
        }
        Ok(())
    }

    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        RuntimeFacade::close_conversation(self, conversation_id)
    }
}

#[async_trait]
impl PairMessageSender for RuntimeFacade {
    async fn send_message_with_admission(
        &mut self,
        conversation_id: &str,
        content: &str,
    ) -> Result<bool, RuntimeError> {
        static NEXT_RELAY_COMMAND: AtomicU64 = AtomicU64::new(1);
        let sequence = NEXT_RELAY_COMMAND.fetch_add(1, Ordering::Relaxed);
        let admission = RuntimeFacade::send_message_with_admission(
            self,
            conversation_id,
            content,
            format!("agent-test-relay-{sequence}"),
        )?;
        Ok(admission.decision.is_accepted())
    }
}

#[async_trait]
impl PairConclusionHost for RuntimeFacade {
    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        RuntimeFacade::close_conversation(self, conversation_id)
    }

    async fn publish_agent_test_event(
        &mut self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), RuntimeError> {
        self.event_bus
            .publish(BaseEvent::new(event_type, payload))
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))
    }
}

fn configure_diagnostics(config: &RuntimeConfig) {
    let data_dir =
        config.runtime.data_dir.clone().unwrap_or_else(|| {
            resolve_config_storage_dir(config.config_dir.as_ref(), None, "data")
        });
    let log_path = data_dir.join("logs").join("agent-runtime.log");
    llm_gateway::diagnostics::set_log_file_path(&log_path);
    llm_gateway::diagnostics::append_line(format!(
        "[agent-runtime] diagnostics log initialized path={}",
        log_path.display()
    ));
}

fn non_empty_arg(value: &str, name: &str) -> Result<String, RuntimeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "{name} must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}

fn assistant_config_for_runtime(config: &RuntimeConfig, agent: &AgentSection) -> AIAssistantConfig {
    AIAssistantConfig {
        agent_id: agent.id.clone(),
        name: agent.name.clone(),
        model: agent.model.clone(),
        max_tokens: agent.max_tokens,
        skills_dir: config
            .runtime
            .skills_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("skills")),
        data_dir: config.runtime.data_dir.clone(),
        prompts_dir: None,
        language: config.runtime.language.clone(),
        retrieval: agent.retrieval.clone().unwrap_or_default(),
        system_prompt_constraints: effective_system_prompt_constraints(
            agent.frontend_widgets_enabled,
            &agent.system_prompt_constraints,
        ),
        frontend_widgets_enabled: effective_frontend_widgets_enabled(
            agent.frontend_widgets_enabled,
            &agent.system_prompt_constraints,
        ),
        system_skills: agent.system_skills.clone(),
    }
}

fn active_skill_names(agent: &AgentSection) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(role) = &agent.role {
        if !role.trim().is_empty() {
            names.push(role.clone());
        }
    }
    names.extend(
        agent
            .features
            .iter()
            .filter(|name| !name.trim().is_empty())
            .cloned(),
    );
    for skill in agent.system_skills.values() {
        if !skill.trim().is_empty() && !names.contains(skill) {
            names.push(skill.clone());
        }
    }
    names
}

async fn set_immutable_role_appendix(
    manager: &ConversationManager,
    conversation_id: &str,
    appendix: &str,
) -> Result<(), RuntimeError> {
    let appendix = appendix.trim();
    if appendix.is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "immutable role appendix must not be empty".to_string(),
        ));
    }
    let cache = manager
        .default_agent_cache(conversation_id)
        .await
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    if cache
        .get::<String>(ai_assistant::context::keys::IMMUTABLE_ROLE_APPENDIX)
        .await
        .map_err(|error| RuntimeError::Internal(error.to_string()))?
        .is_some()
    {
        return Err(RuntimeError::InvalidConfig(format!(
            "conversation '{}' immutable role appendix is already set",
            conversation_id
        )));
    }
    cache
        .set(
            ai_assistant::context::keys::IMMUTABLE_ROLE_APPENDIX,
            &appendix.to_string(),
            None,
        )
        .await
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

fn validate_registered_agent_retrieval(
    cluster_id: &str,
    agent_id: &str,
    retrieval: Option<&RetrievalConfig>,
    resources: Option<&RuntimeResourceRegistry>,
) -> Result<(), RuntimeError> {
    let Some(retrieval) = retrieval else {
        return Ok(());
    };
    let path = format!(
        "agent cluster '{}'.agents['{}'].retrieval",
        cluster_id, agent_id
    );
    validate_retrieval_config(&path, retrieval)?;
    if !retrieval.enabled {
        return Ok(());
    }
    let endpoint_id = retrieval
        .endpoint_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RuntimeError::InvalidConfig(format!(
                "{path}.endpoint_id must be configured when retrieval is enabled"
            ))
        })?;
    let resources = resources.ok_or_else(|| {
        RuntimeError::InvalidConfig(format!("{path} requires registered resources"))
    })?;
    let endpoint = resources.rpc_pool.get(endpoint_id).ok_or_else(|| {
        RuntimeError::InvalidConfig(format!(
            "{path} references unregistered endpoint '{}'",
            endpoint_id
        ))
    })?;
    if endpoint.protocol != "json-lines" {
        return Err(RuntimeError::InvalidConfig(format!(
            "{path} endpoint '{}' uses protocol '{}'; retrieval endpoints must use 'json-lines'",
            endpoint_id, endpoint.protocol
        )));
    }
    Ok(())
}

async fn validate_agent_skills(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    let skills_dir = config.runtime.skills_dir.as_ref().ok_or_else(|| {
        RuntimeError::InvalidConfig("runtime.skills_dir must not be empty".to_string())
    })?;
    let mut manager = SkillManager::from_directory(skills_dir)
        .await
        .map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "load skills directory failed from {}: {e}",
                skills_dir.display()
            ))
        })?;

    for agent in &config.agents {
        if let Some(role) = agent
            .role
            .as_deref()
            .map(str::trim)
            .filter(|role| !role.is_empty())
        {
            let skill = manager.load(role).await.map_err(|e| {
                RuntimeError::InvalidConfig(format!(
                    "agent '{}' role skill '{}' failed to load from {}: {e}",
                    agent.id,
                    role,
                    skills_dir.display()
                ))
            })?;
            if !skill.metadata.is_role() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "agent '{}' role skill '{}' must have kind=role",
                    agent.id, role
                )));
            }
            if skill.instructions.trim().is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "agent '{}' role skill '{}' must include non-empty instructions",
                    agent.id, role
                )));
            }
        }

        for (state, skill_name) in &agent.system_skills {
            if state != "thinking" {
                return Err(RuntimeError::InvalidConfig(format!(
                    "agent '{}' system_skills state '{}' is unsupported; only 'thinking' is configurable",
                    agent.id, state
                )));
            }
            let skill_name = skill_name.trim();
            if skill_name.is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "agent '{}' system_skills.thinking must not be empty",
                    agent.id
                )));
            }
            let skill = manager.load(skill_name).await.map_err(|error| {
                RuntimeError::InvalidConfig(format!(
                    "agent '{}' thinking system skill '{}' failed to load from {}: {error}",
                    agent.id,
                    skill_name,
                    skills_dir.display()
                ))
            })?;
            if !skill.metadata.system_layer || skill.metadata.is_role() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "agent '{}' thinking system skill '{}' must have system_layer=true and must not be kind=role",
                    agent.id, skill_name
                )));
            }
            if skill.instructions.trim().is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "agent '{}' thinking system skill '{}' must include non-empty instructions",
                    agent.id, skill_name
                )));
            }
        }
    }

    Ok(())
}

fn read_json_or_file(input: &str) -> Result<String, RuntimeError> {
    let trimmed = input.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Ok(trimmed.to_string());
    }

    fs::read_to_string(trimmed)
        .map_err(|e| RuntimeError::InvalidConfig(format!("read json file failed: {trimmed}: {e}")))
}

#[cfg(test)]
mod tests;
