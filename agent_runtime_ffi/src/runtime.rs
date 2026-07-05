use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
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
    RuntimeStateConfig as CoreRuntimeStateConfig,
};
use corework::rpc_tool::{
    validate_runtime_tool_for_endpoint, GrpcRpcToolClient, GrpcRpcToolDiscoveryClient,
    JsonLineRpcToolClient, JsonLineRpcToolDiscoveryClient, RpcEndpointInfo, RpcEndpointRegistry,
    RpcStubSystem, RpcToolClient, RuntimeToolMetadata, RuntimeToolRegistry,
};
use corework::workflow::blueprint_json::BlueprintJson;
use corework::workflow::dynamic_node::DynamicExecute;
use corework::workflow::workflows::script_tools::{ParentWorkflowEntry, PARENT_WORKFLOW_REGISTRY};
use corework::workflow::WorkflowsModule;
use llm_gateway::key_store;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc, watch, RwLock as TokioRwLock};
use tokio::time::sleep;

use crate::{
    agent_test_studio::conclusion::PairConclusionHost,
    agent_test_studio::pair_builder::{PairConversationFactory, PairConversationSpec},
    agent_test_studio::pair_runtime::PairMessageSender,
    agent_test_studio::role_contract::{TargetToolContract, TargetWhiteboxContract},
    agent_test_studio::tool_runtime::AgentTestToolRuntime,
    ffi_error_code_internal, ffi_error_code_invalid_config, ffi_error_code_llm_failed,
    ffi_error_code_rpc_failed,
};

mod events;
mod resources;

use events::*;
use resources::*;

#[cfg(test)]
use crate::agent_test_studio::tools::{
    ADVERSARY_CONCLUDE, ADVERSARY_CREATE, ADVERSARY_DESTROY, ADVERSARY_INSPECT,
};

pub(crate) const LOCAL_LEASE_TTL_MS: u64 = 30_000;
pub(crate) const LOCAL_LEASE_RENEW_INTERVAL_MS: u64 = 10_000;
const CONVERSATION_CREATED_EVENT: &str = "runtime:conversation_created";
const CONVERSATION_CLOSED_EVENT: &str = "runtime:conversation_closed";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub schema: String,
    #[serde(skip)]
    pub config_dir: Option<PathBuf>,
    pub runtime: RuntimeSection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentSection>,
    pub workflow: WorkflowSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval: Option<RetrievalConfig>,
    pub rpc_tools: Vec<RpcToolEndpointConfig>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-config/v1".to_string(),
            config_dir: None,
            runtime: RuntimeSection::default(),
            agents: vec![AgentSection::default()],
            agent: None,
            workflow: WorkflowSection::default(),
            retrieval: None,
            rpc_tools: Vec::new(),
        }
    }
}

impl RuntimeConfig {
    fn from_create_options(options: RuntimeCreateOptions) -> Result<Self, RuntimeError> {
        let mut config = RuntimeConfig::default();
        config.runtime.log_level = options.log_level;
        config.runtime.language = options.language;
        config.runtime.restore_policy = options.restore_policy;
        config.runtime.data_dir = options.data_dir;
        ai_assistant::prompt_assets::normalize_language(&config.runtime.language)
            .map_err(RuntimeError::InvalidConfig)?;
        Ok(config)
    }

    #[allow(dead_code)]
    fn normalize_and_validate(mut self) -> Result<Self, RuntimeError> {
        if self.schema.trim().is_empty() {
            self.schema = "agent-runtime-config/v1".to_string();
        }
        if self.schema != "agent-runtime-config/v1" {
            return Err(RuntimeError::InvalidConfig(format!(
                "runtime config schema '{}' is not supported",
                self.schema
            )));
        }

        if self.agents.is_empty() {
            if let Some(mut legacy_agent) = self.agent.take() {
                legacy_agent.is_default = true;
                if self.runtime.skills_dir.is_none()
                    && legacy_agent.skills_dir != PathBuf::from("skills")
                {
                    self.runtime.skills_dir = Some(legacy_agent.skills_dir.clone());
                }
                self.agents.push(legacy_agent);
            }
        }
        if self.agents.is_empty() {
            self.agents.push(AgentSection::default());
        }
        self.agent = None;
        self.normalize_storage_paths();

        validate_runtime_config(&self)?;
        Ok(self)
    }

    #[allow(dead_code)]
    fn normalize_storage_paths(&mut self) {
        self.runtime.data_dir = Some(resolve_config_storage_dir(
            self.config_dir.as_ref(),
            self.runtime.data_dir.as_deref(),
            "data",
        ));
        self.runtime.skills_dir = Some(resolve_explicit_or_default_dir(
            self.config_dir.as_ref(),
            self.runtime.skills_dir.as_deref(),
            "skills",
        ));
        self.runtime.prompts_dir = None;
        self.runtime.llm_config_path = Some(resolve_config_storage_file(
            self.config_dir.as_ref(),
            self.runtime.llm_config_dir.as_deref(),
            "llm_config.json",
        ));
        self.workflow.auto_load_dir = Some(resolve_config_storage_dir(
            self.config_dir.as_ref(),
            self.workflow.auto_load_dir.as_deref(),
            "workflows",
        ));
    }

    fn default_agent(&self) -> &AgentSection {
        self.agents
            .iter()
            .find(|agent| agent.is_default)
            .or_else(|| self.agents.first())
            .expect("runtime config is normalized with at least one agent")
    }

    fn agent_by_id_or_default(&self, agent_id: &str) -> Result<&AgentSection, RuntimeError> {
        let requested = agent_id.trim();
        if !requested.is_empty() {
            if let Some(agent) = self.agents.iter().find(|agent| agent.id == requested) {
                return Ok(agent);
            }
            return Err(RuntimeError::InvalidConfig(format!(
                "agent_id '{}' was not found in runtime config",
                requested
            )));
        }
        Ok(self.default_agent())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RuntimeCreateOptions {
    schema: String,
    log_level: String,
    language: String,
    restore_policy: RestorePolicy,
    data_dir: Option<PathBuf>,
}

impl Default for RuntimeCreateOptions {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-create-options/v1".to_string(),
            log_level: "info".to_string(),
            language: "zh".to_string(),
            restore_policy: RestorePolicy::Strict,
            data_dir: None,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStateBackendConfig {
    Map,
    Ecs,
    Hybrid,
}

impl Default for RuntimeStateBackendConfig {
    fn default() -> Self {
        Self::Map
    }
}

impl RuntimeStateBackendConfig {
    fn into_core(self) -> CoreRuntimeStateConfig {
        match self {
            Self::Map => CoreRuntimeStateConfig::map(),
            Self::Ecs => CoreRuntimeStateConfig::ecs(),
            Self::Hybrid => CoreRuntimeStateConfig::hybrid(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeSection {
    pub log_level: String,
    pub state_backend: RuntimeStateBackendConfig,
    pub data_dir: Option<PathBuf>,
    pub skills_dir: Option<PathBuf>,
    pub prompts_dir: Option<PathBuf>,
    pub language: String,
    #[serde(alias = "llm_config_path")]
    pub llm_config_dir: Option<PathBuf>,
    #[serde(skip)]
    pub llm_config_path: Option<PathBuf>,
    pub cluster_id: String,
    pub runtime_profile_id: String,
    pub cluster_fingerprint: Option<String>,
    pub runtime_instance_id: String,
    pub persistence: PersistenceConfig,
    pub restore_policy: RestorePolicy,
    pub max_thinking_rounds: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval: Option<RetrievalConfig>,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            state_backend: RuntimeStateBackendConfig::default(),
            data_dir: None,
            skills_dir: None,
            prompts_dir: None,
            language: "zh".to_string(),
            llm_config_dir: None,
            llm_config_path: None,
            cluster_id: "default".to_string(),
            runtime_profile_id: "default-v1".to_string(),
            cluster_fingerprint: None,
            runtime_instance_id: "local".to_string(),
            persistence: PersistenceConfig::default(),
            restore_policy: RestorePolicy::default(),
            max_thinking_rounds: 0,
            retrieval: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PersistenceConfig {
    Legacy(bool),
    Profile(PersistenceSection),
}

impl PersistenceConfig {
    pub fn mode(&self) -> PersistenceMode {
        match self {
            Self::Legacy(true) => PersistenceMode::LocalFiles,
            Self::Legacy(false) => PersistenceMode::HostManaged,
            Self::Profile(section) => section.mode,
        }
    }

    pub fn auto_file_persistence_enabled(&self) -> bool {
        self.mode() == PersistenceMode::LocalFiles
    }
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self::Profile(PersistenceSection::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistenceSection {
    pub mode: PersistenceMode,
}

impl Default for PersistenceSection {
    fn default() -> Self {
        Self {
            mode: PersistenceMode::HostManaged,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceMode {
    HostManaged,
    LocalFiles,
}

impl Default for PersistenceMode {
    fn default() -> Self {
        Self::HostManaged
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestorePolicy {
    Strict,
    Compatible,
    UnsafeForce,
}

impl Default for RestorePolicy {
    fn default() -> Self {
        Self::Strict
    }
}

#[allow(dead_code)]
#[async_trait]
trait RuntimeStateStore: Send + Sync {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>, RuntimeError>;
    async fn put_json(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl_ms: Option<u64>,
    ) -> Result<(), RuntimeError>;
    async fn delete(&self, key: &str) -> Result<(), RuntimeError>;
}

#[derive(Default)]
struct LocalRuntimeStateStore {
    values: StdMutex<HashMap<String, serde_json::Value>>,
}

#[async_trait]
impl RuntimeStateStore for LocalRuntimeStateStore {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>, RuntimeError> {
        self.values
            .lock()
            .map_err(|_| RuntimeError::Internal("local state store mutex poisoned".to_string()))
            .map(|values| values.get(key).cloned())
    }

    async fn put_json(
        &self,
        key: &str,
        value: serde_json::Value,
        _ttl_ms: Option<u64>,
    ) -> Result<(), RuntimeError> {
        self.values
            .lock()
            .map_err(|_| RuntimeError::Internal("local state store mutex poisoned".to_string()))?
            .insert(key.to_string(), value);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), RuntimeError> {
        self.values
            .lock()
            .map_err(|_| RuntimeError::Internal("local state store mutex poisoned".to_string()))?
            .remove(key);
        Ok(())
    }
}

#[allow(dead_code)]
#[async_trait]
pub(crate) trait RuntimeCoordinationBackend: Send + Sync {
    async fn acquire_lease(
        &self,
        key: &str,
        owner: &str,
        ttl_ms: u64,
    ) -> Result<bool, RuntimeError>;
    async fn renew_lease(&self, key: &str, owner: &str, ttl_ms: u64) -> Result<bool, RuntimeError>;
    async fn release_lease(&self, key: &str, owner: &str) -> Result<(), RuntimeError>;
}

#[async_trait]
trait RuntimeSequenceBackend: Send + Sync {
    async fn next_global_event_seq(&self) -> Result<u64, RuntimeError>;
    async fn next_conversation_event_seq(&self, conversation_id: &str)
        -> Result<u64, RuntimeError>;
}

#[derive(Default)]
struct LocalRuntimeSequenceBackend {
    global_event_seq: StdMutex<u64>,
    conversation_event_seq: StdMutex<HashMap<String, u64>>,
}

#[async_trait]
impl RuntimeSequenceBackend for LocalRuntimeSequenceBackend {
    async fn next_global_event_seq(&self) -> Result<u64, RuntimeError> {
        let mut seq = self
            .global_event_seq
            .lock()
            .map_err(|_| RuntimeError::Internal("local event seq mutex poisoned".to_string()))?;
        *seq += 1;
        Ok(*seq)
    }

    async fn next_conversation_event_seq(
        &self,
        conversation_id: &str,
    ) -> Result<u64, RuntimeError> {
        let mut seqs = self.conversation_event_seq.lock().map_err(|_| {
            RuntimeError::Internal("local conversation event seq mutex poisoned".to_string())
        })?;
        let seq = seqs.entry(conversation_id.to_string()).or_insert(0);
        *seq += 1;
        Ok(*seq)
    }
}

#[derive(Default)]
struct LocalRuntimeCoordinationBackend {
    leases: StdMutex<HashMap<String, (String, Instant)>>,
}

#[async_trait]
impl RuntimeCoordinationBackend for LocalRuntimeCoordinationBackend {
    async fn acquire_lease(
        &self,
        key: &str,
        owner: &str,
        ttl_ms: u64,
    ) -> Result<bool, RuntimeError> {
        let now = Instant::now();
        let expires_at = now + Duration::from_millis(ttl_ms.max(1));
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| RuntimeError::Internal("local lease mutex poisoned".to_string()))?;
        if let Some((current_owner, current_expires_at)) = leases.get(key) {
            if current_owner != owner && *current_expires_at > now {
                return Ok(false);
            }
        }
        leases.insert(key.to_string(), (owner.to_string(), expires_at));
        Ok(true)
    }

    async fn renew_lease(&self, key: &str, owner: &str, ttl_ms: u64) -> Result<bool, RuntimeError> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| RuntimeError::Internal("local lease mutex poisoned".to_string()))?;
        match leases.get_mut(key) {
            Some((current_owner, expires_at)) if current_owner == owner => {
                *expires_at = Instant::now() + Duration::from_millis(ttl_ms.max(1));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn release_lease(&self, key: &str, owner: &str) -> Result<(), RuntimeError> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| RuntimeError::Internal("local lease mutex poisoned".to_string()))?;
        if leases
            .get(key)
            .map(|(current_owner, _)| current_owner == owner)
            .unwrap_or(false)
        {
            leases.remove(key);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkflowSection {
    pub auto_load_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSection {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub model: String,
    pub max_tokens: usize,
    pub skills_dir: PathBuf,
    pub role: Option<String>,
    pub features: Vec<String>,
    pub retrieval: Option<RetrievalConfig>,
    #[serde(alias = "systemPromptConstraints", default)]
    pub system_prompt_constraints: SystemPromptConstraints,
    #[serde(default = "default_true")]
    pub frontend_widgets_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RpcToolEndpointConfig {
    pub endpoint_id: String,
    pub address: String,
    pub protocol: String,
    pub launch: Option<RpcToolLaunchConfig>,
    pub timeout_ms: u64,
    pub tools: Vec<RuntimeToolMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcToolLaunchConfig {
    pub kind: String,
    pub program: Option<PathBuf>,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub env: std::collections::BTreeMap<String, String>,
    pub startup_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
}

impl Default for RpcToolLaunchConfig {
    fn default() -> Self {
        Self {
            kind: "external".to_string(),
            program: None,
            args: Vec::new(),
            working_dir: None,
            env: std::collections::BTreeMap::new(),
            startup_timeout_ms: 10_000,
            shutdown_timeout_ms: 3_000,
        }
    }
}

impl Default for RpcToolEndpointConfig {
    fn default() -> Self {
        Self {
            endpoint_id: String::new(),
            address: String::new(),
            protocol: "grpc".to_string(),
            launch: None,
            timeout_ms: 30_000,
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceRegistration {
    pub schema: String,
    pub id: String,
    pub skills: ResourceSkillsConfig,
    pub workflows: ResourceWorkflowsConfig,
    pub data: ResourceDataConfig,
    pub agents: ResourceAgentsConfig,
    pub rpc_endpoints: Vec<ResourceRpcEndpointConfig>,
}

impl Default for ResourceRegistration {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-resource-registration/v1".to_string(),
            id: String::new(),
            skills: ResourceSkillsConfig::default(),
            workflows: ResourceWorkflowsConfig::default(),
            data: ResourceDataConfig::default(),
            agents: ResourceAgentsConfig::default(),
            rpc_endpoints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceSkillsConfig {
    pub root_dir: PathBuf,
    pub builtin_system: bool,
}

impl Default for ResourceSkillsConfig {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::from("skills"),
            builtin_system: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceWorkflowsConfig {
    pub root_dir: Option<PathBuf>,
    pub registry_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceDataConfig {
    pub data_dir: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
    pub conversation_logs: ConversationLogPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceAgentsConfig {
    pub profiles: Vec<ResourceAgentProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceAgentProfileConfig {
    pub id: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub features: Vec<String>,
    #[serde(alias = "modelUid")]
    pub model_uid: Option<u32>,
    pub retrieval: Option<RetrievalConfig>,
    #[serde(alias = "systemPromptConstraints", default)]
    pub system_prompt_constraints: SystemPromptConstraints,
    #[serde(alias = "frontendWidgetsEnabled", default = "default_true")]
    pub frontend_widgets_enabled: bool,
}

impl Default for ResourceAgentProfileConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            role: None,
            features: Vec::new(),
            model_uid: None,
            retrieval: None,
            system_prompt_constraints: SystemPromptConstraints::default(),
            frontend_widgets_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConversationLogPolicy {
    pub retention_days: u64,
    pub max_files_per_cluster: usize,
    pub max_file_bytes: u64,
}

impl Default for ConversationLogPolicy {
    fn default() -> Self {
        Self {
            retention_days: 14,
            max_files_per_cluster: 1_000,
            max_file_bytes: 10 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceRpcEndpointConfig {
    pub id: String,
    pub protocol: String,
    pub endpoint: String,
    pub launch: Option<RpcToolLaunchConfig>,
    pub timeout_ms: u64,
}

impl Default for ResourceRpcEndpointConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            protocol: "grpc".to_string(),
            endpoint: String::new(),
            launch: None,
            timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RuntimeResourceRegistry {
    pub id: String,
    pub skills_root_dir: PathBuf,
    pub builtin_system_skills: bool,
    pub workflows_root_dir: Option<PathBuf>,
    pub workflow_registry_id: Option<String>,
    pub workflow_registry: Vec<ParentWorkflowEntry>,
    pub data_dir: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
    pub conversation_log_policy: ConversationLogPolicy,
    pub agent_profiles: BTreeMap<String, ResourceAgentProfileConfig>,
    pub rpc_pool: RuntimeRpcEndpointPool,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeRpcEndpointPool {
    endpoints: BTreeMap<String, RpcToolEndpointConfig>,
}

impl RuntimeRpcEndpointPool {
    fn insert(&mut self, endpoint: RpcToolEndpointConfig) -> Result<(), RuntimeError> {
        if self.endpoints.contains_key(&endpoint.endpoint_id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "resource rpc endpoint '{}' is duplicated",
                endpoint.endpoint_id
            )));
        }
        self.endpoints
            .insert(endpoint.endpoint_id.clone(), endpoint);
        Ok(())
    }

    pub fn get(&self, endpoint_id: &str) -> Option<&RpcToolEndpointConfig> {
        self.endpoints.get(endpoint_id)
    }

    pub fn endpoints(&self) -> impl Iterator<Item = &RpcToolEndpointConfig> {
        self.endpoints.values()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }
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

#[derive(Debug, Clone)]
pub struct RuntimeAgentCluster {
    pub id: String,
    pub description: String,
    pub focus_agent_id: String,
    pub agents: Vec<RuntimeAgentDefinition>,
    pub max_thinking_rounds: u32,
    pub permissions: ai_assistant::ToolPermissionPolicy,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentDefinition {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub features: Vec<String>,
    pub model_uid: u32,
    pub retrieval: Option<RetrievalConfig>,
    pub system_prompt_constraints: SystemPromptConstraints,
    pub frontend_widgets_enabled: bool,
}

#[derive(Debug, Clone)]
struct BuiltinClusterConfigs {
    workflow_editor: RuntimeAgentCluster,
    agent_test_supervisor: RuntimeAgentCluster,
    agent_test_adversary: RuntimeAgentCluster,
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            id: "boss".to_string(),
            name: "Default Agent".to_string(),
            is_default: true,
            model: "default".to_string(),
            max_tokens: 4096,
            skills_dir: PathBuf::from("skills"),
            role: None,
            features: Vec::new(),
            retrieval: None,
            system_prompt_constraints: SystemPromptConstraints::default(),
            frontend_widgets_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
struct ProviderBundle {
    providers: Vec<ProviderConfig>,
    models: Vec<ModelConfig>,
    current_model_uid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderDefinitions {
    schema: &'static str,
    providers: Vec<ProviderDefinition>,
    models: Vec<ModelConfig>,
    current_model_uid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderDefinition {
    uid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_paradigm: Option<llm_gateway::ApiParadigm>,
    prompt_cache_control: bool,
    api_key_set: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ProviderConfig {
    uid: u32,
    #[serde(default)]
    name: Option<String>,
    api_key: String,
    #[serde(default)]
    base_url: String,
    #[serde(alias = "apiParadigm", alias = "api_paradigm", default)]
    api_paradigm: Option<llm_gateway::ApiParadigm>,
    #[serde(alias = "promptCacheControl", default)]
    prompt_cache_control: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ModelConfig {
    uid: u32,
    provider_uid: u32,
    model_name: String,
    #[serde(default = "default_context_window")]
    context_window: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct ProviderConfigV1 {
    schema: String,
    providers: Vec<HostUserProviderConfig>,
    #[serde(alias = "currentModelUid")]
    current_model_uid: Option<u32>,
}

impl Default for ProviderConfigV1 {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-provider-config/v1".to_string(),
            providers: Vec::new(),
            current_model_uid: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LlmRegistration {
    schema: String,
    id: String,
    #[serde(alias = "currentModelUid")]
    current_model_uid: Option<u32>,
    #[serde(rename = "nextProviderId")]
    _legacy_next_provider_id: Option<u32>,
    #[serde(rename = "nextModelUid")]
    _legacy_next_model_uid: Option<u32>,
    providers: Vec<LlmRegistrationProvider>,
}

impl Default for LlmRegistration {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-llm-registration/v1".to_string(),
            id: String::new(),
            current_model_uid: None,
            _legacy_next_provider_id: None,
            _legacy_next_model_uid: None,
            providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "snake_case", deny_unknown_fields)]
#[derive(Default)]
struct LlmRegistrationProvider {
    id: u32,
    name: String,
    #[serde(rename = "type")]
    builtin_type: String,
    #[serde(alias = "apiKey")]
    api_key: Option<String>,
    #[serde(alias = "apiKeyEnv")]
    api_key_env: Option<String>,
    #[serde(alias = "baseUrl")]
    base_url: String,
    #[serde(alias = "apiParadigm")]
    api_paradigm: Option<llm_gateway::ApiParadigm>,
    #[serde(alias = "promptCacheControl")]
    prompt_cache_control: bool,
    #[serde(alias = "enabledModels")]
    enabled_models: Vec<LlmRegistrationModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "snake_case", deny_unknown_fields)]
#[derive(Default)]
struct LlmRegistrationModel {
    uid: u32,
    #[serde(alias = "modelId")]
    model_id: String,
    #[serde(alias = "maxContextTokens")]
    max_context_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AgentClusterRegistration {
    schema: String,
    id: String,
    description: String,
    #[serde(alias = "focusAgentId")]
    focus_agent_id: Option<String>,
    agents: Vec<AgentClusterAgentRegistration>,
    #[serde(alias = "maxThinkingRounds")]
    max_thinking_rounds: u32,
    permissions: ai_assistant::ToolPermissionPolicy,
}

impl Default for AgentClusterRegistration {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-agent-cluster-registration/v1".to_string(),
            id: String::new(),
            description: String::new(),
            focus_agent_id: None,
            agents: Vec::new(),
            max_thinking_rounds: 0,
            permissions: ai_assistant::ToolPermissionPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AgentClusterAgentRegistration {
    id: String,
    profile: Option<String>,
    name: String,
    role: Option<String>,
    features: Vec<String>,
    #[serde(alias = "modelUid")]
    model_uid: Option<u32>,
    retrieval: Option<RetrievalConfig>,
    #[serde(alias = "systemPromptConstraints", default)]
    system_prompt_constraints: SystemPromptConstraints,
    #[serde(alias = "frontendWidgetsEnabled", default = "default_true")]
    frontend_widgets_enabled: bool,
}

impl Default for AgentClusterAgentRegistration {
    fn default() -> Self {
        Self {
            id: String::new(),
            profile: None,
            name: String::new(),
            role: None,
            features: Vec::new(),
            model_uid: None,
            retrieval: None,
            system_prompt_constraints: SystemPromptConstraints::default(),
            frontend_widgets_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConversationSpawnRequest {
    schema: String,
    #[serde(alias = "clusterId")]
    cluster_id: String,
    #[serde(
        default,
        alias = "hostContext",
        alias = "toolHostContext",
        alias = "tool_host_context"
    )]
    tool_host_context: Option<Value>,
}

impl Default for ConversationSpawnRequest {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-conversation-spawn/v1".to_string(),
            cluster_id: String::new(),
            tool_host_context: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct HostEnabledModel {
    uid: u32,
    #[serde(alias = "modelId")]
    model_id: String,
    #[serde(alias = "maxContextTokens", default)]
    max_context_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct HostUserProviderConfig {
    id: u32,
    name: String,
    #[serde(rename = "type")]
    builtin_type: String,
    #[serde(alias = "apiKey")]
    api_key: String,
    #[serde(alias = "baseUrl", default)]
    base_url: String,
    #[serde(alias = "apiParadigm", default)]
    api_paradigm: Option<llm_gateway::ApiParadigm>,
    #[serde(alias = "promptCacheControl", default)]
    prompt_cache_control: bool,
    #[serde(alias = "enabledModels", default)]
    enabled_models: Vec<HostEnabledModel>,
}

impl From<HostEnabledModel> for llm_gateway::EnabledModel {
    fn from(model: HostEnabledModel) -> Self {
        Self {
            uid: model.uid,
            model_id: model.model_id,
            max_context_tokens: model.max_context_tokens,
        }
    }
}

impl From<HostUserProviderConfig> for llm_gateway::UserProviderConfig {
    fn from(provider: HostUserProviderConfig) -> Self {
        Self {
            id: provider.id,
            name: provider.name,
            builtin_type: provider.builtin_type,
            api_key: provider.api_key,
            base_url: provider.base_url,
            api_paradigm: provider.api_paradigm,
            prompt_cache_control: provider.prompt_cache_control,
            enabled_models: provider
                .enabled_models
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

fn default_context_window() -> u32 {
    8192
}

#[derive(Debug, Clone, Default)]
struct ConversationInitPlan {
    model_uid: Option<u32>,
    max_thinking_rounds: Option<u32>,
    agent_profiles: BTreeMap<String, ResourceAgentProfileConfig>,
    tool_host_context: Option<Value>,
    lifecycle_cluster_id: Option<String>,
    lifecycle_cluster_description: Option<String>,
    additional_agents: Vec<ConversationAgentInit>,
    tool_permissions: Option<ai_assistant::ToolPermissionPolicy>,
}

#[derive(Debug, Clone)]
struct ConversationAgentInit {
    config: AIAssistantConfig,
    skills: Vec<String>,
    max_thinking_rounds: u32,
}

#[derive(Debug, Clone)]
struct ConversationInstanceMetadata {
    cluster_id: String,
    cluster_description: String,
    log_path: Option<PathBuf>,
    log_policy: ConversationLogPolicy,
}

struct ManagedSidecar {
    child: Child,
    shutdown_timeout_ms: u64,
}

impl Drop for ManagedSidecar {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_millis(self.shutdown_timeout_ms);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.wait();
    }
}

struct ConversationOwnerLease {
    stop_renewer: watch::Sender<bool>,
    renewer: tokio::task::JoinHandle<()>,
    key: String,
    owner: String,
}

impl ConversationOwnerLease {
    async fn stop_and_release(
        self,
        backend: Arc<dyn RuntimeCoordinationBackend>,
    ) -> Result<(), RuntimeError> {
        let _ = self.stop_renewer.send(true);
        let _ = self.renewer.await;
        backend.release_lease(&self.key, &self.owner).await
    }
}

#[derive(Clone)]
pub(crate) struct AgentTestRuntimeHost {
    config: RuntimeConfig,
    adversary_cluster: RuntimeAgentCluster,
    manager: Arc<ConversationManager>,
    export_event_bus: Arc<dyn EventBus>,
    state_store: Arc<dyn RuntimeStateStore>,
    coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    conversation_owner_leases: Arc<StdMutex<HashMap<String, ConversationOwnerLease>>>,
    logs_dir: Option<PathBuf>,
    conversation_log_policy: ConversationLogPolicy,
    conversation_logs: Arc<StdMutex<HashMap<String, ConversationInstanceMetadata>>>,
}

impl AgentTestRuntimeHost {
    async fn create_cluster_conversation(
        &self,
        spec: &PairConversationSpec,
        cluster: &RuntimeAgentCluster,
        agent_id: &str,
    ) -> Result<(), RuntimeError> {
        let registered_agent = cluster
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "built-in cluster '{}' agent '{}' was not found",
                    cluster.id, agent_id
                ))
            })?;
        let agent = runtime_agent_definition_to_agent_section(registered_agent);
        let skills = active_skill_names(&agent);
        tracing::info!(
            conversation_id = %spec.conversation_id,
            agent_id = %agent.id,
            skills = ?skills,
            "creating agent test conversation"
        );
        let assistant_config = assistant_config_for_runtime(&self.config, &agent);
        let mut options = ConversationOptions::new(spec.conversation_id.clone(), assistant_config);
        options.tenant_id = Some("agent-test-studio".to_string());
        options.user_id = Some(spec.agent_id.clone());
        options.tool_permissions = cluster.permissions.clone();
        if self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .contains_key(&spec.conversation_id)
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is already active in this runtime instance",
                spec.conversation_id
            )));
        }

        let runtime_cluster_id = self.config.runtime.cluster_id.clone();
        let lifecycle_cluster_id = cluster.id.clone();
        let lifecycle_cluster_description = cluster.description.clone();
        let runtime_instance_id = self.config.runtime.runtime_instance_id.clone();
        let conversation_id = spec.conversation_id.clone();
        let Some(owner_lease) = acquire_conversation_owner_lease(
            Arc::clone(&self.coordination_backend),
            runtime_cluster_id.clone(),
            conversation_id.clone(),
            runtime_instance_id.clone(),
            LOCAL_LEASE_TTL_MS,
            lease_renew_interval(LOCAL_LEASE_TTL_MS, LOCAL_LEASE_RENEW_INTERVAL_MS),
        )
        .await?
        else {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is already owned by another runtime instance",
                conversation_id
            )));
        };

        let result = async {
            let info = self
                .manager
                .create_conversation(options, Arc::clone(&self.export_event_bus))
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            tracing::info!(
                conversation_id = %conversation_id,
                "created agent test conversation runtime"
            );
            if !skills.is_empty() {
                let refs: Vec<&str> = skills.iter().map(String::as_str).collect();
                self.manager
                    .activate_skills(&conversation_id, &refs)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                tracing::info!(
                    conversation_id = %conversation_id,
                    "activated agent test conversation skills"
                );
            }
            if let Some(appendix) = spec.immutable_role_appendix.as_deref() {
                set_immutable_role_appendix(&self.manager, &conversation_id, appendix).await?;
            }
            record_conversation_created(
                Arc::clone(&self.state_store),
                Arc::clone(&self.coordination_backend),
                runtime_cluster_id,
                runtime_instance_id,
                LOCAL_LEASE_TTL_MS,
                info.clone(),
            )
            .await?;
            publish_conversation_created_event(
                Arc::clone(&self.export_event_bus),
                &info,
                &lifecycle_cluster_id,
                &lifecycle_cluster_description,
            )
            .await
        }
        .await;

        if let Err(error) = result {
            let _ = self.manager.close(&conversation_id).await;
            let _ = owner_lease
                .stop_and_release(Arc::clone(&self.coordination_backend))
                .await;
            return Err(error);
        }
        self.conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .insert(conversation_id, owner_lease);
        let log_path = create_conversation_log_path(
            self.logs_dir.as_deref(),
            &lifecycle_cluster_id,
            &spec.conversation_id,
            chrono::Utc::now(),
            &self.conversation_log_policy,
        );
        let metadata = ConversationInstanceMetadata {
            cluster_id: lifecycle_cluster_id,
            cluster_description: lifecycle_cluster_description,
            log_path,
            log_policy: self.conversation_log_policy.clone(),
        };
        append_conversation_log_path(
            metadata.log_path.as_deref(),
            &self.config.runtime.runtime_instance_id,
            &metadata.cluster_id,
            &spec.conversation_id,
            "conversation_created",
            Value::Object(Default::default()),
            metadata.log_policy.max_file_bytes,
        );
        self.conversation_logs
            .lock()
            .map_err(|_| RuntimeError::Internal("conversation log lock poisoned".to_string()))?
            .insert(spec.conversation_id.clone(), metadata);
        Ok(())
    }

    async fn create_business_target_conversation(
        &self,
        spec: &PairConversationSpec,
        agent: &AgentSection,
    ) -> Result<(), RuntimeError> {
        let cluster = RuntimeAgentCluster {
            id: "agent-test-target".to_string(),
            description: "Isolated business target conversation for Agent Test".to_string(),
            focus_agent_id: agent.id.clone(),
            agents: vec![RuntimeAgentDefinition {
                id: agent.id.clone(),
                name: agent.name.clone(),
                role: agent.role.clone(),
                features: agent.features.clone(),
                model_uid: key_store::current().unwrap_or(0),
                retrieval: agent.retrieval.clone(),
                system_prompt_constraints: effective_system_prompt_constraints(
                    agent.frontend_widgets_enabled,
                    &agent.system_prompt_constraints,
                ),
                frontend_widgets_enabled: effective_frontend_widgets_enabled(
                    agent.frontend_widgets_enabled,
                    &agent.system_prompt_constraints,
                ),
            }],
            max_thinking_rounds: 0,
            permissions: ai_assistant::ToolPermissionPolicy::default(),
        };
        self.create_cluster_conversation(spec, &cluster, &agent.id)
            .await
    }

    async fn close(&self, conversation_id: &str) -> Result<(), RuntimeError> {
        let owner_lease = self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .remove(conversation_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is not owned by this runtime instance",
                    conversation_id
                ))
            })?;
        let removed = self
            .manager
            .close(conversation_id)
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        if removed {
            let metadata = self
                .conversation_logs
                .lock()
                .map_err(|_| RuntimeError::Internal("conversation log lock poisoned".to_string()))?
                .remove(conversation_id);
            record_conversation_closed(
                Arc::clone(&self.state_store),
                Arc::clone(&self.coordination_backend),
                self.config.runtime.cluster_id.clone(),
                self.config.runtime.runtime_instance_id.clone(),
                LOCAL_LEASE_TTL_MS,
                conversation_id.to_string(),
            )
            .await?;
            let lifecycle_cluster_id = metadata
                .as_ref()
                .map(|metadata| metadata.cluster_id.as_str())
                .unwrap_or("agent-test");
            let lifecycle_cluster_description = metadata
                .as_ref()
                .map(|metadata| metadata.cluster_description.as_str())
                .unwrap_or("Agent Test conversation");
            publish_conversation_closed_event(
                Arc::clone(&self.export_event_bus),
                conversation_id,
                lifecycle_cluster_id,
                lifecycle_cluster_description,
            )
            .await?;
            append_conversation_log_path(
                metadata
                    .as_ref()
                    .and_then(|metadata| metadata.log_path.as_deref()),
                &self.config.runtime.runtime_instance_id,
                lifecycle_cluster_id,
                conversation_id,
                "conversation_closed",
                Value::Object(Default::default()),
                metadata
                    .as_ref()
                    .map(|metadata| metadata.log_policy.max_file_bytes)
                    .unwrap_or(ConversationLogPolicy::default().max_file_bytes),
            );
        }
        owner_lease
            .stop_and_release(Arc::clone(&self.coordination_backend))
            .await
    }
}

#[async_trait]
impl PairConversationFactory for AgentTestRuntimeHost {
    async fn create_target_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError> {
        let agent = self.config.agent_by_id_or_default(&spec.agent_id)?.clone();
        self.create_business_target_conversation(spec, &agent).await
    }

    async fn create_adversary_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError> {
        let focus_agent_id = self.adversary_cluster.focus_agent_id.clone();
        let mut spec = spec.clone();
        spec.agent_id = focus_agent_id.clone();
        self.create_cluster_conversation(&spec, &self.adversary_cluster, &focus_agent_id)
            .await
    }

    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        self.close(conversation_id).await
    }
}

#[async_trait]
impl PairMessageSender for AgentTestRuntimeHost {
    async fn send_message_with_admission(
        &mut self,
        conversation_id: &str,
        content: &str,
    ) -> Result<bool, RuntimeError> {
        let host = self.clone();
        let conversation_id = conversation_id.to_string();
        let content = content.to_string();
        tokio::spawn(async move {
            if let Err(error) = host
                .send_relay_message_with_admission(&conversation_id, &content)
                .await
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    "agent test relay send failed: {error}"
                );
                let _ = host
                    .export_event_bus
                    .publish(BaseEvent::new(
                        crate::agent_test_studio::pair_runtime::RELAY_SEND_FAILED_EVENT,
                        json!({
                            "conversation_id": conversation_id,
                            "failure": error.to_string()
                        }),
                    ))
                    .await;
            }
        });
        Ok(true)
    }
}

impl AgentTestRuntimeHost {
    async fn send_relay_message_with_admission(
        &self,
        conversation_id: &str,
        content: &str,
    ) -> Result<(), RuntimeError> {
        static NEXT_RELAY_COMMAND: AtomicU64 = AtomicU64::new(1);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.config.runtime.cluster_id, conversation_id
        );
        let lease_owner = self.config.runtime.runtime_instance_id.clone();
        let acquired = self
            .coordination_backend
            .acquire_lease(&lease_key, &lease_owner, LOCAL_LEASE_TTL_MS)
            .await?;
        if !acquired {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' turn lease is held by another runtime",
                conversation_id
            )));
        }
        let sequence = NEXT_RELAY_COMMAND.fetch_add(1, Ordering::Relaxed);
        let admission = self
            .manager
            .send_message_with_admission(
                conversation_id,
                content,
                Some(format!("agent-test-relay-{sequence}")),
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()));
        let release = self
            .coordination_backend
            .release_lease(&lease_key, &lease_owner)
            .await;
        match (admission, release) {
            (Ok(admission), Ok(())) if admission.decision.is_accepted() => Ok(()),
            (Ok(_), Ok(())) => Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' rejected the relay message admission",
                conversation_id
            ))),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(release_error)) => {
                tracing::warn!("release agent test turn lease failed: {}", release_error);
                Err(error)
            }
        }
    }
}

#[async_trait]
impl PairConclusionHost for AgentTestRuntimeHost {
    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        self.close(conversation_id).await
    }

    async fn publish_agent_test_event(
        &mut self,
        event_type: &str,
        payload: Value,
    ) -> Result<(), RuntimeError> {
        self.export_event_bus
            .publish(BaseEvent::new(event_type, payload))
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))
    }
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
        self.register_llm(registration)
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
        self.register_llm(registration)
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

        let config_for_skill_validation = self.config.clone();
        self.rt.block_on(async move {
            validate_default_agent_role_skill(&config_for_skill_validation).await
        })?;

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

            let workflow_module = install_workflow_runtime_from_config(workflow_auto_load_dir)?;
            let (sidecar_children, runtime_tools) =
                install_rpc_tools_from_config(rpc_tools).await?;
            install_retrieval_system_from_config(&retrieval_configs, &retrieval_endpoints)?;
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
        let workflows = self
            .workflow_module
            .as_ref()
            .cloned()
            .ok_or(RuntimeError::NotStarted)?;

        let session_id = crate::workflow_studio::next_studio_session_id();
        if let Some(previous_conversation_id) = self.workflow_studio_conversation_id.take() {
            if let Err(error) = self.close_conversation(&previous_conversation_id) {
                tracing::warn!(
                    conversation_id = %previous_conversation_id,
                    "close previous Workflow Studio conversation failed: {error}"
                );
            }
        }
        let editor_conversation_id = format!("{session_id}_editor");
        let target_agent = self
            .config
            .agent_by_id_or_default(&options.agent_id)?
            .clone();
        let editor_cluster = self
            .builtin_clusters
            .as_ref()
            .ok_or(RuntimeError::NotStarted)?
            .workflow_editor
            .clone();
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
        self.configure_workflow_editor_conversation(
            &editor_conversation_id,
            &editor_agent,
            &target_agent,
            &node_capabilities,
        )?;
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
        let target_agent = self
            .config
            .agent_by_id_or_default(&options.agent_id)?
            .clone();
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

    pub fn configure_providers(&mut self, providers_input: &str) -> Result<(), RuntimeError> {
        let content = read_json_or_file(providers_input)?;

        if let Ok(config_v1) = serde_json::from_str::<ProviderConfigV1>(&content) {
            if config_v1.schema == "agent-runtime-provider-config/v1" {
                let current_model_uid = config_v1.current_model_uid;
                let config = llm_gateway::LlmConfig {
                    providers: config_v1.providers.into_iter().map(Into::into).collect(),
                    current_model_uid,
                };
                validate_llm_config(&config)?;
                validate_current_model(current_model_uid, &config)?;
                llm_gateway::build_index_and_resolver(config.clone());
                if let Some(uid) = current_model_uid {
                    key_store::set_current(uid);
                }
                self.llm_config = config;
                self.sync_provider_bundle();
                if let Some(bundle) = self.provider_bundle.as_mut() {
                    bundle.current_model_uid = current_model_uid;
                }
                self.persist_provider_config_text(&content)?;
                return Ok(());
            }
        }

        if let Ok(config) = serde_json::from_str::<llm_gateway::LlmConfig>(&content) {
            validate_llm_config(&config)?;
            llm_gateway::build_index_and_resolver(config.clone());
            self.llm_config = config;
            self.sync_provider_bundle();
            self.persist_llm_config()?;
            return Ok(());
        }

        let bundle: ProviderBundle = serde_json::from_str(&content).map_err(|e| {
            RuntimeError::InvalidConfig(format!("parse providers_json failed: {e}"))
        })?;

        let provider_map: std::collections::HashMap<u32, ProviderConfig> = bundle
            .providers
            .clone()
            .into_iter()
            .map(|provider| (provider.uid, provider))
            .collect();
        for provider in provider_map.values() {
            if provider.prompt_cache_control
                && provider.api_paradigm != Some(llm_gateway::ApiParadigm::AnthropicMessages)
            {
                return Err(RuntimeError::InvalidConfig(format!(
                    "providers['{}'].prompt_cache_control requires api_paradigm 'anthropic_messages'",
                    provider.uid
                )));
            }
        }

        let entries = bundle.models.clone().into_iter().map(|model| {
            (
                model.uid,
                key_store::ModelEntry {
                    model_name: model.model_name,
                    provider_uid: model.provider_uid,
                    context_window: model.context_window,
                },
            )
        });
        key_store::reload(entries);

        key_store::set_resolver(move |provider_uid| {
            provider_map
                .get(&provider_uid)
                .map(|provider| key_store::ProviderRuntimeConfig {
                    api_key: provider.api_key.clone(),
                    base_url: provider.base_url.clone(),
                    api_paradigm: provider.api_paradigm,
                    prompt_cache_control: provider.prompt_cache_control,
                })
        });

        if let Some(uid) = bundle.current_model_uid {
            if !bundle.models.iter().any(|model| model.uid == uid) {
                return Err(RuntimeError::Llm(format!(
                    "current_model_uid {uid} is not configured"
                )));
            }
            key_store::set_current(uid);
        }

        self.provider_bundle = Some(bundle);
        self.persist_provider_config_text(&content)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add_providers_typed(&mut self, providers_json: &str) -> Result<(), RuntimeError> {
        let providers = parse_provider_array(providers_json)?;

        for provider in providers {
            if let Some(existing) = self
                .llm_config
                .providers
                .iter_mut()
                .find(|p| p.id == provider.id)
            {
                for model in provider.enabled_models {
                    if !existing.enabled_models.iter().any(|m| m.uid == model.uid) {
                        existing.enabled_models.push(model);
                    }
                }
                // Keep provider display metadata in sync with imported host config.
                existing.name = provider.name;
                existing.api_key = provider.api_key;
                if !provider.base_url.is_empty() {
                    existing.base_url = provider.base_url;
                }
                existing.api_paradigm = provider.api_paradigm;
                existing.prompt_cache_control = provider.prompt_cache_control;
            } else {
                self.llm_config.providers.push(provider);
            }
        }

        validate_llm_config(&self.llm_config)?;
        llm_gateway::build_index_and_resolver(self.llm_config.clone());
        self.sync_provider_bundle();
        self.persist_llm_config()?;

        Ok(())
    }

    fn sync_provider_bundle(&mut self) {
        let bundle = self.provider_bundle.get_or_insert_with(Default::default);
        bundle.providers = self
            .llm_config
            .providers
            .iter()
            .map(|p| ProviderConfig {
                uid: p.id,
                name: Some(p.name.clone()),
                api_key: p.api_key.clone(),
                base_url: p.base_url.clone(),
                api_paradigm: p.api_paradigm,
                prompt_cache_control: p.prompt_cache_control,
            })
            .collect();
        bundle.models = self
            .llm_config
            .providers
            .iter()
            .flat_map(|p| {
                p.enabled_models.iter().map(move |m| ModelConfig {
                    uid: m.uid,
                    provider_uid: p.id,
                    model_name: m.model_id.clone(),
                    context_window: 0,
                })
            })
            .collect();
        bundle.current_model_uid = self
            .llm_config
            .current_model_uid
            .or_else(key_store::current);
    }

    pub fn provider_definitions(&self) -> Result<String, RuntimeError> {
        let Some(bundle) = &self.provider_bundle else {
            return Ok(json!({
                "schema": "agent-runtime-provider-definitions/v1",
                "providers": [],
                "models": [],
                "current_model_uid": null
            })
            .to_string());
        };

        let definitions = ProviderDefinitions {
            schema: "agent-runtime-provider-definitions/v1",
            providers: bundle
                .providers
                .iter()
                .map(|provider| ProviderDefinition {
                    uid: provider.uid,
                    name: provider.name.clone(),
                    base_url: provider.base_url.clone(),
                    api_paradigm: provider.api_paradigm,
                    prompt_cache_control: provider.prompt_cache_control,
                    api_key_set: !provider.api_key.is_empty(),
                })
                .collect(),
            models: bundle.models.clone(),
            current_model_uid: bundle.current_model_uid,
        };

        serde_json::to_string(&definitions).map_err(|e| {
            RuntimeError::Internal(format!("serialize provider definitions failed: {e}"))
        })
    }

    pub fn set_current_model(&mut self, model_uid: u32) -> Result<(), RuntimeError> {
        if key_store::get(model_uid).is_none() {
            return Err(RuntimeError::Llm(format!(
                "model uid {model_uid} is not configured"
            )));
        }
        key_store::set_current(model_uid);
        self.llm_config.current_model_uid = Some(model_uid);
        if let Some(bundle) = self.provider_bundle.as_mut() {
            bundle.current_model_uid = Some(model_uid);
        }
        self.persist_current_model_selection(model_uid)?;
        Ok(())
    }

    pub fn set_language(&mut self, language: &str) -> Result<(), RuntimeError> {
        let normalized = ai_assistant::prompt_assets::set_language(language)
            .map_err(RuntimeError::InvalidConfig)?;
        self.config.runtime.language = normalized.clone();
        if let Some(manager) = self.conversation_manager.clone() {
            self.rt.block_on(async move {
                for info in manager.list().await {
                    manager
                        .set_language(&info.conversation_id, &normalized)
                        .await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                }
                Ok::<(), RuntimeError>(())
            })?;
        }
        Ok(())
    }

    fn persist_provider_config_text(&self, content: &str) -> Result<(), RuntimeError> {
        let Some(path) = self.config.runtime.llm_config_path.clone() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                RuntimeError::InvalidConfig(format!(
                    "create provider config dir failed {}: {e}",
                    parent.display()
                ))
            })?;
        }
        fs::write(&path, content).map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "write provider config failed {}: {e}",
                path.display()
            ))
        })
    }

    fn persist_llm_config(&self) -> Result<(), RuntimeError> {
        let content = serde_json::to_string_pretty(&self.llm_config).map_err(|e| {
            RuntimeError::Internal(format!("serialize provider config failed: {e}"))
        })?;
        self.persist_provider_config_text(&content)
    }

    fn persist_current_model_selection(&self, model_uid: u32) -> Result<(), RuntimeError> {
        let Some(path) = self.config.runtime.llm_config_path.clone() else {
            return Ok(());
        };
        if !path.exists() {
            return self.persist_llm_config();
        }

        let content = fs::read_to_string(&path).map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "read provider config failed {}: {e}",
                path.display()
            ))
        })?;
        let mut value: Value = serde_json::from_str(&content).map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "parse provider config failed {}: {e}",
                path.display()
            ))
        })?;

        let Some(object) = value.as_object_mut() else {
            return self.persist_llm_config();
        };
        let field = if object.contains_key("currentModelUid")
            && !object.contains_key("current_model_uid")
        {
            "currentModelUid"
        } else {
            "current_model_uid"
        };
        object.insert(field.to_string(), json!(model_uid));

        let content = serde_json::to_string_pretty(&value).map_err(|e| {
            RuntimeError::Internal(format!("serialize provider config failed: {e}"))
        })?;
        self.persist_provider_config_text(&content)
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

    pub fn set_ai_auth_context(&mut self, context_json: &str) -> Result<(), RuntimeError> {
        let headers = parse_ai_auth_context_headers(context_json)?;
        self.ai_auth_context_headers = headers.clone();
        if let Some(manager) = self.conversation_manager.clone() {
            self.rt
                .block_on(manager.set_runtime_llm_request_headers(headers));
        }
        Ok(())
    }

    pub fn create_conversation(
        &mut self,
        options_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let agent = self.config.default_agent().clone();
        let skills = self.active_skill_names_for_agent(&agent);
        self.create_conversation_with_agent_and_skills(options_json, &agent, skills)
    }

    pub fn spawn_conversation(
        &mut self,
        spawn_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let request = parse_conversation_spawn_request(spawn_json)?;
        let cluster = self
            .registries
            .agent_clusters
            .get(&request.cluster_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "agent cluster '{}' is not registered",
                    request.cluster_id
                ))
            })?;
        let focus_agent = cluster
            .agents
            .iter()
            .find(|agent| agent.id == cluster.focus_agent_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "agent cluster '{}' focus_agent_id '{}' was not found",
                    cluster.id, cluster.focus_agent_id
                ))
            })?;
        let agent = runtime_agent_definition_to_agent_section(&focus_agent);
        let skills = active_skill_names(&agent);
        let mut additional_agents = Vec::new();
        for registered_agent in cluster
            .agents
            .iter()
            .filter(|registered_agent| registered_agent.id != cluster.focus_agent_id)
        {
            let section = runtime_agent_definition_to_agent_section(registered_agent);
            additional_agents.push(ConversationAgentInit {
                config: self.assistant_config_for_agent(&section),
                skills: active_skill_names(&section),
                max_thinking_rounds: cluster.max_thinking_rounds,
            });
        }
        let init = ConversationInitPlan {
            model_uid: (focus_agent.model_uid != 0).then_some(focus_agent.model_uid),
            max_thinking_rounds: Some(cluster.max_thinking_rounds),
            tool_host_context: request.tool_host_context.clone(),
            lifecycle_cluster_id: Some(cluster.id.clone()),
            lifecycle_cluster_description: Some(cluster.description.clone()),
            additional_agents,
            ..ConversationInitPlan::default()
        };
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        self.create_conversation_from_parts(
            ConversationOptionsInput::default(),
            &agent,
            skills,
            init,
        )
    }

    pub fn spawn_conversation_from_snapshot(
        &mut self,
        spawn_json: &str,
        snapshot_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let snapshot = parse_conversation_snapshot(snapshot_json)?;
        let info = self.spawn_conversation(spawn_json)?;
        let state_deltas = snapshot_state_deltas(&snapshot);
        let recovery =
            match self.replace_conversation_ledger(&info.conversation_id, snapshot.ledger) {
                Ok(recovery) => recovery,
                Err(error) => {
                    let _ = self.close_conversation(&info.conversation_id);
                    return Err(error);
                }
            };
        if let Err(error) =
            self.apply_conversation_state_deltas(&info.conversation_id, state_deltas)
        {
            let _ = self.close_conversation(&info.conversation_id);
            return Err(error);
        }
        if let Err(error) = self.apply_conversation_recovery(&info.conversation_id, recovery) {
            let _ = self.close_conversation(&info.conversation_id);
            return Err(error);
        }
        Ok(info)
    }

    #[allow(dead_code)]
    pub(crate) fn registered_cluster_description(&self, cluster_id: &str) -> Option<&str> {
        self.registries
            .agent_clusters
            .get(cluster_id)
            .map(|cluster| cluster.description.as_str())
    }

    fn create_conversation_with_agent_and_skills(
        &mut self,
        options_json: &str,
        agent: &AgentSection,
        skills: Vec<String>,
    ) -> Result<ConversationInfo, RuntimeError> {
        let options_input = parse_conversation_options(options_json)?;
        self.create_conversation_from_parts(
            options_input,
            agent,
            skills,
            ConversationInitPlan::default(),
        )
    }

    fn create_internal_conversation(
        &mut self,
        options_json: &str,
        agent: &AgentSection,
        skills: Vec<String>,
        cluster_id: &str,
        cluster_description: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let options_input = parse_conversation_options(options_json)?;
        self.create_conversation_from_parts(
            options_input,
            agent,
            skills,
            ConversationInitPlan {
                lifecycle_cluster_id: Some(cluster_id.to_string()),
                lifecycle_cluster_description: Some(cluster_description.to_string()),
                ..ConversationInitPlan::default()
            },
        )
    }

    fn create_conversation_from_cluster_config(
        &mut self,
        options_json: &str,
        cluster: &RuntimeAgentCluster,
    ) -> Result<ConversationInfo, RuntimeError> {
        let focus_agent = cluster
            .agents
            .iter()
            .find(|agent| agent.id == cluster.focus_agent_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "built-in cluster '{}' focus agent '{}' was not found",
                    cluster.id, cluster.focus_agent_id
                ))
            })?;
        let agent = runtime_agent_definition_to_agent_section(focus_agent);
        let options_input = parse_conversation_options(options_json)?;
        self.create_conversation_from_parts(
            options_input,
            &agent,
            active_skill_names(&agent),
            ConversationInitPlan {
                model_uid: (focus_agent.model_uid != 0).then_some(focus_agent.model_uid),
                max_thinking_rounds: Some(cluster.max_thinking_rounds),
                tool_permissions: Some(cluster.permissions.clone()),
                lifecycle_cluster_id: Some(cluster.id.clone()),
                lifecycle_cluster_description: Some(cluster.description.clone()),
                additional_agents: Vec::new(),
                ..ConversationInitPlan::default()
            },
        )
    }

    fn create_conversation_from_parts(
        &mut self,
        options_input: ConversationOptionsInput,
        agent: &AgentSection,
        skills: Vec<String>,
        init: ConversationInitPlan,
    ) -> Result<ConversationInfo, RuntimeError> {
        let manager = self.manager()?;
        let assistant_config = self.assistant_config_for_agent(agent);
        let mut init = init;
        if init.agent_profiles.is_empty() {
            if let Some(resources) = &self.registries.resources {
                init.agent_profiles = resources.agent_profiles.clone();
            }
        }
        let mut options = match options_input.conversation_id {
            Some(id) if !id.trim().is_empty() => ConversationOptions::new(id, assistant_config),
            _ => ConversationOptions::from_config(assistant_config),
        };
        options.tenant_id = options_input
            .tenant_id
            .filter(|value| !value.trim().is_empty());
        options.user_id = options_input
            .user_id
            .filter(|value| !value.trim().is_empty());
        options.llm_request_headers = options_input.llm_request_headers.unwrap_or_default();
        options.allow_insecure_llm_request_headers =
            options_input.allow_insecure_llm_request_headers;
        if let Some(policy) = init.tool_permissions.clone() {
            options.tool_permissions = policy;
        }
        let requested_conversation_id = options.conversation_id.clone();
        if self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .contains_key(&requested_conversation_id)
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is already active in this runtime instance",
                requested_conversation_id
            )));
        }

        let export_event_bus: Arc<dyn EventBus> = self.event_bus.clone();
        let state_store = Arc::clone(&self.state_store);
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let cluster_id = init
            .lifecycle_cluster_id
            .clone()
            .unwrap_or_else(|| self.config.runtime.cluster_id.clone());
        let runtime_instance_id = self.config.runtime.runtime_instance_id.clone();
        let catalog_lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let owner_lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let owner_lease_renew_interval =
            lease_renew_interval(owner_lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let instance_init = init.clone();

        let (info, owner_lease) = self.rt.block_on(async move {
            let conversation_id = options.conversation_id.clone();
            let Some(owner_lease) = acquire_conversation_owner_lease(
                Arc::clone(&coordination_backend),
                cluster_id.clone(),
                conversation_id.clone(),
                runtime_instance_id.clone(),
                owner_lease_ttl_ms,
                owner_lease_renew_interval,
            )
            .await?
            else {
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is already owned by another runtime instance",
                    conversation_id
                )));
            };

            let result = async {
                let info = manager
                    .create_conversation(options, Arc::clone(&export_event_bus))
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;

                if !skills.is_empty() {
                    let refs: Vec<&str> = skills.iter().map(String::as_str).collect();
                    manager
                        .activate_skills(&conversation_id, &refs)
                        .await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                }

                apply_conversation_init_plan(&manager, &conversation_id, &init).await?;
                for additional_agent in &init.additional_agents {
                    let refs: Vec<&str> =
                        additional_agent.skills.iter().map(String::as_str).collect();
                    manager
                        .register_agent(
                            &conversation_id,
                            additional_agent.config.clone(),
                            &refs,
                            additional_agent.max_thinking_rounds,
                            &BTreeMap::new(),
                        )
                        .await
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                }

                let lifecycle_cluster_id = init
                    .lifecycle_cluster_id
                    .clone()
                    .unwrap_or_else(|| cluster_id.clone());
                record_conversation_created(
                    Arc::clone(&state_store),
                    Arc::clone(&coordination_backend),
                    cluster_id.clone(),
                    runtime_instance_id.clone(),
                    catalog_lease_ttl_ms,
                    info.clone(),
                )
                .await?;
                publish_conversation_created_event(
                    export_event_bus,
                    &info,
                    &lifecycle_cluster_id,
                    init.lifecycle_cluster_description.as_deref().unwrap_or(""),
                )
                .await?;

                Ok::<ConversationInfo, RuntimeError>(info)
            }
            .await;

            match result {
                Ok(info) => Ok((info, owner_lease)),
                Err(error) => {
                    let _ = manager.close(&conversation_id).await;
                    let _ = owner_lease
                        .stop_and_release(Arc::clone(&coordination_backend))
                        .await;
                    Err(error)
                }
            }
        })?;

        self.conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .insert(info.conversation_id.clone(), owner_lease);
        let lifecycle_cluster_id = instance_init
            .lifecycle_cluster_id
            .clone()
            .unwrap_or_else(|| self.config.runtime.cluster_id.clone());
        let cluster_description = instance_init
            .lifecycle_cluster_description
            .clone()
            .unwrap_or_default();
        let log_path = self.create_conversation_log(
            &lifecycle_cluster_id,
            &info.conversation_id,
            info.created_at,
        );
        let log_policy = self.conversation_log_policy();
        self.conversation_instances.insert(
            info.conversation_id.clone(),
            ConversationInstanceMetadata {
                cluster_id: lifecycle_cluster_id,
                cluster_description,
                log_path,
                log_policy,
            },
        );
        self.append_conversation_log(
            &info.conversation_id,
            "conversation_created",
            json!({ "scope_id": info.scope_id }),
        );
        Ok(info)
    }

    fn set_conversation_immutable_role_appendix(
        &self,
        conversation_id: &str,
        appendix: &str,
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = conversation_id.to_string();
        let appendix = appendix.trim().to_string();
        if appendix.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "immutable role appendix must not be empty".to_string(),
            ));
        }
        self.rt.block_on(async move {
            let cache = manager
                .default_agent_cache(&conversation_id)
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
                    &appendix,
                    None,
                )
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))
        })
    }

    pub fn materialize_conversation(
        &mut self,
        conversation_id: &str,
        options_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let mut options = if options_json.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<serde_json::Value>(options_json).map_err(|e| {
                RuntimeError::InvalidConfig(format!("parse materialize options failed: {e}"))
            })?
        };
        if !options.is_object() {
            return Err(RuntimeError::InvalidConfig(
                "materialize options must be a JSON object".to_string(),
            ));
        }
        options["conversation_id"] = serde_json::Value::String(conversation_id);
        if options.get("schema").is_none() {
            options["schema"] =
                serde_json::Value::String("agent-runtime-conversation-options/v1".to_string());
        }
        self.create_conversation(&options.to_string())
    }

    #[allow(dead_code)]
    pub fn send_message(&self, conversation_id: &str, content: &str) -> Result<(), RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "send_message",
            json!({ "content_bytes": content.len() }),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        let content = content.to_string();
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let lease_owner = self.config.runtime.runtime_instance_id.clone();
        let lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let lease_renew_interval =
            lease_renew_interval(lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.conversation_instances
                .get(&conversation_id)
                .map(|metadata| metadata.cluster_id.as_str())
                .unwrap_or(&self.config.runtime.cluster_id),
            conversation_id
        );
        self.rt.block_on(async move {
            let runtime_conversation_id = conversation_id.clone();
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                ttl_ms = lease_ttl_ms,
                content_len = content.len(),
                "runtime turn lease acquire start"
            );
            let acquired = coordination_backend
                .acquire_lease(&lease_key, &lease_owner, lease_ttl_ms)
                .await?;
            if !acquired {
                tracing::warn!(
                    conversation_id = %runtime_conversation_id,
                    lease_owner = %lease_owner,
                    lease_key = %lease_key,
                    "runtime turn lease acquire rejected"
                );
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' turn lease is held by another runtime",
                    runtime_conversation_id
                )));
            }
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                "runtime turn lease acquire ok"
            );

            let (stop_renewer, renewer_stop_rx) = watch::channel(false);
            let renewer = tokio::spawn(run_lease_renewer(
                Arc::clone(&coordination_backend),
                lease_key.clone(),
                lease_owner.clone(),
                lease_ttl_ms,
                lease_renew_interval,
                renewer_stop_rx,
            ));
            let send_result = manager
                .send_message(&conversation_id, &content)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()));
            let _ = stop_renewer.send(true);
            let _ = renewer.await;
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                send_ok = send_result.is_ok(),
                "runtime turn lease release start"
            );
            let release_result = coordination_backend
                .release_lease(&lease_key, &lease_owner)
                .await;
            match (send_result, release_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!("release conversation turn lease failed: {}", release_error);
                    Err(error)
                }
            }
        })
    }

    pub fn send_message_with_admission(
        &self,
        conversation_id: &str,
        content: &str,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "send_message_with_admission",
            json!({ "command_id": command_id.clone(), "content_bytes": content.len() }),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        let content = content.to_string();
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let lease_owner = self.config.runtime.runtime_instance_id.clone();
        let lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let lease_renew_interval =
            lease_renew_interval(lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.conversation_instances
                .get(&conversation_id)
                .map(|metadata| metadata.cluster_id.as_str())
                .unwrap_or(&self.config.runtime.cluster_id),
            conversation_id
        );
        self.rt.block_on(async move {
            let runtime_conversation_id = conversation_id.clone();
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                ttl_ms = lease_ttl_ms,
                content_len = content.len(),
                "runtime turn lease acquire start"
            );
            let acquired = coordination_backend
                .acquire_lease(&lease_key, &lease_owner, lease_ttl_ms)
                .await?;
            if !acquired {
                tracing::warn!(
                    conversation_id = %runtime_conversation_id,
                    lease_owner = %lease_owner,
                    lease_key = %lease_key,
                    "runtime turn lease acquire rejected"
                );
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' turn lease is held by another runtime",
                    runtime_conversation_id
                )));
            }
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                "runtime turn lease acquire ok"
            );

            let (stop_renewer, renewer_stop_rx) = watch::channel(false);
            let renewer = tokio::spawn(run_lease_renewer(
                Arc::clone(&coordination_backend),
                lease_key.clone(),
                lease_owner.clone(),
                lease_ttl_ms,
                lease_renew_interval,
                renewer_stop_rx,
            ));
            let send_result = manager
                .send_message_with_admission(&conversation_id, &content, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()));
            let _ = stop_renewer.send(true);
            let _ = renewer.await;
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                send_ok = send_result.is_ok(),
                "runtime turn lease release start"
            );
            let release_result = coordination_backend
                .release_lease(&lease_key, &lease_owner)
                .await;
            match (send_result, release_result) {
                (Ok(admission), Ok(())) => Ok(admission),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!("release conversation turn lease failed: {}", release_error);
                    Err(error)
                }
            }
        })
    }

    /// Updates one host-owned plain-text dynamic context field for one agent.
    ///
    /// Thinking reads every field in the current agent's table on entry. This
    /// operation deliberately does not expose replacement of the complete table
    /// and is not available through RPC tool HostCall.
    pub fn set_agent_dynamic_snapshot_field(
        &self,
        conversation_id: &str,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> Result<(), RuntimeError> {
        llm_gateway::diagnostics::append_line(format!(
            "[agent-runtime] set_agent_dynamic_snapshot_field requested conversation_id={} agent_id={} field={} bytes={}",
            conversation_id,
            agent_id,
            field_name,
            text.len()
        ));
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let agent_id = non_empty_arg(agent_id, "agent_id")?;
        let field_name = non_empty_arg(field_name, "field_name")?;
        self.require_conversation_owner(&conversation_id)?;
        let text = text.to_string();
        self.rt.block_on(async move {
            llm_gateway::diagnostics::append_line(format!(
                "[agent-runtime] set_agent_dynamic_snapshot_field applying conversation_id={} agent_id={} field={} bytes={}",
                conversation_id,
                agent_id,
                field_name,
                text.len()
            ));
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &agent_id,
                    &field_name,
                    &text,
                )
                .await
                .map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;
            llm_gateway::diagnostics::append_line(format!(
                "[agent-runtime] set_agent_dynamic_snapshot_field applied conversation_id={} agent_id={} field={}",
                conversation_id,
                agent_id,
                field_name
            ));
            Ok(())
        })
    }

    pub fn resolve_tool_permission(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        decision: ai_assistant::ToolPermissionDecision,
    ) -> Result<bool, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let tool_call_id = non_empty_arg(tool_call_id, "tool_call_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .resolve_tool_permission(&conversation_id, &tool_call_id, decision)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))
        })
    }

    fn create_conversation_log(
        &self,
        cluster_id: &str,
        conversation_id: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Option<PathBuf> {
        let resources = self.registries.resources.as_ref();
        let logs_dir = resources
            .and_then(|resources| resources.logs_dir.clone())
            .or_else(|| {
                self.config
                    .runtime
                    .data_dir
                    .as_ref()
                    .map(|data_dir| data_dir.join("logs"))
            });
        let policy = resources
            .map(|resources| resources.conversation_log_policy.clone())
            .unwrap_or_default();
        create_conversation_log_path(
            logs_dir.as_deref(),
            cluster_id,
            conversation_id,
            created_at,
            &policy,
        )
    }

    fn conversation_log_policy(&self) -> ConversationLogPolicy {
        self.registries
            .resources
            .as_ref()
            .map(|resources| resources.conversation_log_policy.clone())
            .unwrap_or_default()
    }

    fn append_conversation_log(&self, conversation_id: &str, event: &str, details: Value) {
        let Some(metadata) = self.conversation_instances.get(conversation_id) else {
            return;
        };
        let Some(path) = metadata.log_path.as_ref() else {
            return;
        };
        append_conversation_log_path(
            Some(path),
            &self.config.runtime.runtime_instance_id,
            &metadata.cluster_id,
            conversation_id,
            event,
            details,
            metadata.log_policy.max_file_bytes,
        );
    }

    pub fn set_conversation_summary_model_with_admission(
        &self,
        conversation_id: &str,
        model_name: &str,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let model_name = non_empty_arg(model_name, "model_name")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .set_summary_model_with_admission(&conversation_id, &model_name, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    /// Set the summarization model used by `compact_history`.
    ///
    /// This only updates the conversation-level summary model; it does not
    /// change the active reasoning model for future assistant turns.
    #[allow(dead_code)]
    pub fn set_conversation_summary_model(
        &self,
        conversation_id: &str,
        model_name: &str,
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let model_name = non_empty_arg(model_name, "model_name")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .set_summary_model(&conversation_id, &model_name)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    pub fn compact_conversation_history_with_admission(
        &self,
        conversation_id: &str,
        agent_ids: Vec<String>,
        command_id: String,
    ) -> Result<(ai_assistant::gateway::AdmissionResult, String), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let (admission, report) = manager
                .compact_history_with_admission(&conversation_id, agent_ids, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let report_json = serde_json::to_string(&report).map_err(|e| {
                RuntimeError::Internal(format!("serialize compact history report failed: {e}"))
            })?;
            Ok((admission, report_json))
        })
    }

    /// Compact conversation history and return the serialized per-agent report.
    ///
    /// An empty `agent_ids` list compacts the whole cluster; otherwise only the
    /// named agents are compacted.
    #[allow(dead_code)]
    pub fn compact_conversation_history(
        &self,
        conversation_id: &str,
        agent_ids: Vec<String>,
    ) -> Result<String, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let report = manager
                .compact_history(&conversation_id, agent_ids)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            serde_json::to_string(&report).map_err(|e| {
                RuntimeError::Internal(format!("serialize compact history report failed: {e}"))
            })
        })
    }

    #[allow(dead_code)]
    pub fn pause_conversation(&self, conversation_id: &str) -> Result<(), RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "pause_conversation",
            Value::Object(Default::default()),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .request_pause(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    pub fn pause_conversation_with_admission(
        &self,
        conversation_id: &str,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "pause_conversation_with_admission",
            json!({ "command_id": command_id.clone() }),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .request_pause_with_admission(&conversation_id, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    pub fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.append_conversation_log(
            &conversation_id,
            "conversation_closing",
            Value::Object(Default::default()),
        );
        let owner_lease = self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .remove(&conversation_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is not owned by this runtime instance",
                    conversation_id
                ))
            })?;
        let state_store = Arc::clone(&self.state_store);
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let metadata = self
            .conversation_instances
            .get(&conversation_id)
            .cloned()
            .unwrap_or_else(|| ConversationInstanceMetadata {
                cluster_id: self.config.runtime.cluster_id.clone(),
                cluster_description: String::new(),
                log_path: None,
                log_policy: self.conversation_log_policy(),
            });
        let cluster_id = metadata.cluster_id.clone();
        let cluster_description = metadata.cluster_description.clone();
        let export_event_bus: Arc<dyn EventBus> = self.event_bus.clone();
        let runtime_instance_id = self.config.runtime.runtime_instance_id.clone();
        let catalog_lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let closing_conversation_id = conversation_id.clone();
        let close_result = self.rt.block_on(async move {
            let close_result = async {
                let removed = manager
                    .close(&closing_conversation_id)
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                if removed {
                    record_conversation_closed(
                        state_store,
                        Arc::clone(&coordination_backend),
                        cluster_id.clone(),
                        runtime_instance_id,
                        catalog_lease_ttl_ms,
                        closing_conversation_id.clone(),
                    )
                    .await?;
                    publish_conversation_closed_event(
                        export_event_bus,
                        &closing_conversation_id,
                        &cluster_id,
                        &cluster_description,
                    )
                    .await?;
                }
                Ok::<(), RuntimeError>(())
            }
            .await;
            let release_result = owner_lease
                .stop_and_release(Arc::clone(&coordination_backend))
                .await;
            match (close_result, release_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!("release conversation owner lease failed: {}", release_error);
                    Err(error)
                }
            }
        });
        self.append_conversation_log(
            &conversation_id,
            "conversation_closed",
            json!({ "success": close_result.is_ok() }),
        );
        self.conversation_instances.remove(&conversation_id);
        close_result
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

    pub fn snapshot(&self) -> Result<String, RuntimeError> {
        let state_store = Arc::clone(&self.state_store);
        let cluster_id = self.config.runtime.cluster_id.clone();
        let conversations = if self.started {
            self.rt
                .block_on(async move { load_conversation_index(&state_store, &cluster_id).await })?
        } else if let Some(manager) = self.conversation_manager.clone() {
            self.rt.block_on(async move { manager.list().await })
        } else {
            Vec::new()
        };
        Ok(json!({
            "schema": "agent-runtime-snapshot/v1",
            "started": self.started,
            "config": self.config,
            "conversations": conversations,
            "note": "per-conversation snapshot export is not wired in v0.1 shell"
        })
        .to_string())
    }

    pub fn export_conversation_snapshot(
        &self,
        conversation_id: &str,
        options_json: &str,
    ) -> Result<String, RuntimeError> {
        let options = ConversationSnapshotExportOptions::parse(options_json)?;
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let conversation_state = manager
                .frontend_state(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let is_stable =
                conversation_state == ai_assistant::snapshot::ConversationState::Waiting;
            if options.stable_required() && !is_stable {
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation_not_waiting: conversation '{}' is in state '{}'",
                    conversation_id,
                    serde_json::to_value(conversation_state)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_else(|| "unknown".to_string())
                )));
            }
            let ledger = manager
                .ledger(
                    &conversation_id,
                    ai_assistant::conversation_state::LedgerReadOptions::default(),
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let cluster = manager
                .cluster_snapshot(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let tasks = manager
                .agent_tasks(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let info = manager
                .list()
                .await
                .into_iter()
                .find(|info| info.conversation_id == conversation_id)
                .ok_or_else(|| {
                    RuntimeError::InvalidConfig(format!(
                        "conversation '{}' not found",
                        conversation_id
                    ))
                })?;
            Ok(json!({
                "schema": "agent-runtime-conversation-snapshot/v1",
                "conversation_id": conversation_id,
                "tenant_id": info.tenant_id,
                "user_id": info.user_id,
                "consistency": options.as_str(),
                "stable": is_stable,
                "conversation_state": conversation_state,
                "ledger": ledger,
                "runtime": cluster,
                "tasks": tasks,
                "exported_at_unix_ms": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis())
                    .unwrap_or(0)
            })
            .to_string())
        })
    }

    pub fn agent_tasks_json(&self, conversation_id: &str) -> Result<String, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let tasks = manager
                .agent_tasks(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            Ok(json!({
                "schema": "agent-runtime-agent-tasks/v1",
                "conversation_id": conversation_id,
                "tasks": tasks,
            })
            .to_string())
        })
    }

    pub fn import_conversation_snapshot(
        &mut self,
        snapshot_json: &str,
        options_json: &str,
    ) -> Result<(), RuntimeError> {
        let options = parse_conversation_options(options_json)?;
        let target_conversation_id = non_empty_arg(
            options.conversation_id.as_deref().unwrap_or_default(),
            "options.conversation_id",
        )?;
        let snapshot = parse_conversation_snapshot(snapshot_json)?;
        let state_deltas = snapshot_state_deltas(&snapshot);
        let recovery =
            self.replace_conversation_ledger(&target_conversation_id, snapshot.ledger)?;
        self.apply_conversation_state_deltas(&target_conversation_id, state_deltas)?;
        self.apply_conversation_recovery(&target_conversation_id, recovery)
    }

    fn replace_conversation_ledger(
        &mut self,
        target_conversation_id: &str,
        mut records: Vec<ai_assistant::ledger::LedgerRecord>,
    ) -> Result<RestoredConversationRecovery, RuntimeError> {
        let manager = self.manager()?;
        let target_conversation_id =
            non_empty_arg(target_conversation_id, "target_conversation_id")?;
        self.require_conversation_owner(&target_conversation_id)?;
        records.sort_by_key(|record| record.record_id);
        for (index, record) in records.iter_mut().enumerate() {
            record.conversation_id = target_conversation_id.clone();
            record.record_id = index as u64 + 1;
        }
        let recovery = restored_conversation_recovery(&records);
        self.rt.block_on(async move {
            manager
                .replace_ledger(&target_conversation_id, records)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            Ok(recovery)
        })
    }

    fn apply_conversation_state_deltas(
        &mut self,
        target_conversation_id: &str,
        state_deltas: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        if state_deltas.is_empty() {
            return Ok(());
        }
        let manager = self.manager()?;
        let target_conversation_id =
            non_empty_arg(target_conversation_id, "target_conversation_id")?;
        self.require_conversation_owner(&target_conversation_id)?;
        self.rt.block_on(async move {
            for delta in state_deltas {
                apply_conversation_state_delta(&manager, &target_conversation_id, delta).await?;
            }
            Ok(())
        })
    }

    fn apply_conversation_recovery(
        &mut self,
        target_conversation_id: &str,
        recovery: RestoredConversationRecovery,
    ) -> Result<(), RuntimeError> {
        if recovery.entry_states.is_empty() && recovery.execution_plans.is_empty() {
            return Ok(());
        }
        let manager = self.manager()?;
        let target_conversation_id =
            non_empty_arg(target_conversation_id, "target_conversation_id")?;
        self.require_conversation_owner(&target_conversation_id)?;
        self.rt.block_on(async move {
            for (agent_id, state) in recovery.entry_states {
                manager
                    .restore_agent_state_entry(&target_conversation_id, &agent_id, &state)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            for plan in recovery.execution_plans {
                manager
                    .restore_agent_execution_entry(
                        &target_conversation_id,
                        &plan.agent_id,
                        plan.tools,
                        plan.call_ids,
                        plan.recovery_results,
                    )
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            Ok(())
        })
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
            "openWorkflowDraft",
            "updateCurrentWorkflowDraft",
            "compileWorkflowScript",
            "testWorkflow",
            "readWorkflow",
            "saveWorkflow",
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

fn build_llm_registry_and_config(
    mut registration: LlmRegistration,
) -> Result<(RuntimeLlmRegistry, llm_gateway::LlmConfig), RuntimeError> {
    if registration.schema.trim().is_empty() {
        registration.schema = "agent-runtime-llm-registration/v1".to_string();
    }
    if registration.schema != "agent-runtime-llm-registration/v1" {
        return Err(RuntimeError::InvalidConfig(format!(
            "llm registration schema '{}' is not supported",
            registration.schema
        )));
    }
    if registration.id.trim().is_empty() {
        registration.id = "default-llm".to_string();
    }
    validate_resource_id("llm registration id", &registration.id)?;

    let current_model_uid = registration.current_model_uid;
    let provider_count = registration.providers.len();
    let mut model_count = 0usize;
    let providers = registration
        .providers
        .into_iter()
        .map(|provider| {
            model_count += provider.enabled_models.len();
            llm_registration_provider_to_runtime(provider)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let config = llm_gateway::LlmConfig {
        providers,
        current_model_uid,
    };
    validate_llm_config(&config)?;
    let registry = RuntimeLlmRegistry {
        id: registration.id,
        current_model_uid,
        provider_count,
        model_count,
    };
    Ok((registry, config))
}

fn llm_registration_provider_to_runtime(
    provider: LlmRegistrationProvider,
) -> Result<llm_gateway::UserProviderConfig, RuntimeError> {
    let api_key = match (provider.api_key, provider.api_key_env) {
        (Some(api_key), _) => api_key,
        (None, Some(env_name)) => {
            let env_name = env_name.trim();
            if env_name.is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "llm provider '{}' api_key_env must not be empty",
                    provider.id
                )));
            }
            std::env::var(env_name).map_err(|error| {
                RuntimeError::InvalidConfig(format!(
                    "read api_key_env '{}' for llm provider '{}' failed: {error}",
                    env_name, provider.id
                ))
            })?
        }
        (None, None) => String::new(),
    };

    Ok(llm_gateway::UserProviderConfig {
        id: provider.id,
        name: provider.name,
        builtin_type: provider.builtin_type,
        api_key,
        base_url: provider.base_url,
        api_paradigm: provider.api_paradigm,
        prompt_cache_control: provider.prompt_cache_control,
        enabled_models: provider
            .enabled_models
            .into_iter()
            .map(|model| llm_gateway::EnabledModel {
                uid: model.uid,
                model_id: model.model_id,
                max_context_tokens: model.max_context_tokens,
            })
            .collect(),
    })
}

fn build_agent_cluster_registry(
    mut registration: AgentClusterRegistration,
    registries: &RuntimeRegistries,
) -> Result<RuntimeAgentCluster, RuntimeError> {
    if registration.schema.trim().is_empty() {
        registration.schema = "agent-runtime-agent-cluster-registration/v1".to_string();
    }
    if registration.schema != "agent-runtime-agent-cluster-registration/v1" {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster registration schema '{}' is not supported",
            registration.schema
        )));
    }
    validate_resource_id("agent cluster id", &registration.id)?;
    registration.description = registration.description.trim().to_string();
    if registration.description.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' description must not be empty",
            registration.id
        )));
    }
    if registration.agents.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' agents must not be empty",
            registration.id
        )));
    }
    let default_model_uid = registries
        .llm
        .as_ref()
        .and_then(|llm| llm.current_model_uid);
    let requested_focus_agent_id = registration
        .focus_agent_id
        .take()
        .filter(|value| !value.trim().is_empty());
    let mut ids = std::collections::HashSet::new();
    let mut agents = Vec::new();
    let mut focus_aliases = Vec::<(String, String)>::new();
    for mut agent in registration.agents {
        let explicit_profile = agent
            .profile
            .as_deref()
            .map(str::trim)
            .filter(|profile| !profile.is_empty());
        let profile = if let Some(profile_ref) = explicit_profile {
            let profile =
                resolve_registered_agent_profile(registries, profile_ref).ok_or_else(|| {
                    RuntimeError::InvalidConfig(format!(
                        "agent cluster '{}' references unknown agent profile '{}'",
                        registration.id, profile_ref
                    ))
                })?;
            Some(profile)
        } else {
            resolve_registered_agent_profile(registries, &agent.name)
                .or_else(|| resolve_registered_agent_profile(registries, &agent.id))
        };
        let profile_id = profile.map(|profile| profile.id.clone());
        if let Some(profile) = profile {
            apply_registered_agent_profile(&registration.id, &mut agent, profile)?;
        }
        let id = agent.id.trim().to_string();
        validate_resource_id("agent id", &id)?;
        if !ids.insert(id.clone()) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' agent id '{}' is duplicated",
                registration.id, id
            )));
        }
        let model_uid = agent.model_uid.or(default_model_uid).unwrap_or(0);
        if model_uid != 0 && key_store::get(model_uid).is_none() {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' agent '{}' model_uid {} is not registered",
                registration.id, id, model_uid
            )));
        }
        validate_registered_agent_retrieval(
            &registration.id,
            &id,
            agent.retrieval.as_ref(),
            registries.resources.as_ref(),
        )?;
        let features = normalize_string_list(&format!("agent '{}'.features", id), agent.features)?;
        let role = agent
            .role
            .map(|role| role.trim().to_string())
            .filter(|role| !role.is_empty());
        let name = if agent.name.trim().is_empty() {
            id.clone()
        } else {
            agent.name.trim().to_string()
        };
        focus_aliases.push((id.clone(), id.clone()));
        focus_aliases.push((name.clone(), id.clone()));
        if let Some(profile_id) = profile_id {
            focus_aliases.push((profile_id, id.clone()));
        }
        agents.push(RuntimeAgentDefinition {
            id,
            name,
            role,
            features,
            model_uid,
            retrieval: agent.retrieval,
            system_prompt_constraints: effective_system_prompt_constraints(
                agent.frontend_widgets_enabled,
                &agent.system_prompt_constraints,
            ),
            frontend_widgets_enabled: effective_frontend_widgets_enabled(
                agent.frontend_widgets_enabled,
                &agent.system_prompt_constraints,
            ),
        });
    }
    let requested_focus_agent_id = requested_focus_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let focus_agent_id = match requested_focus_agent_id {
        Some(focus) => resolve_cluster_focus_agent_id(&registration.id, focus, &focus_aliases)?,
        None => agents[0].id.clone(),
    };
    if !ids.contains(focus_agent_id.trim()) {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' focus_agent_id '{}' was not found",
            registration.id, focus_agent_id
        )));
    }
    Ok(RuntimeAgentCluster {
        id: registration.id,
        description: registration.description,
        focus_agent_id: focus_agent_id.trim().to_string(),
        agents,
        max_thinking_rounds: registration.max_thinking_rounds,
        permissions: registration.permissions,
    })
}

fn resolve_cluster_focus_agent_id(
    cluster_id: &str,
    focus: &str,
    focus_aliases: &[(String, String)],
) -> Result<String, RuntimeError> {
    let mut matches = focus_aliases
        .iter()
        .filter(|(alias, _)| alias == focus)
        .map(|(_, agent_id)| agent_id.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [agent_id] => Ok(agent_id.clone()),
        [] => Ok(focus.to_string()),
        _ => Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' focus_agent_id '{}' matches multiple agents; use a concrete agent id",
            cluster_id, focus
        ))),
    }
}

fn resolve_registered_agent_profile<'a>(
    registries: &'a RuntimeRegistries,
    name: &str,
) -> Option<&'a ResourceAgentProfileConfig> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let resources = registries.resources.as_ref()?;
    resources.agent_profiles.get(name).or_else(|| {
        resources.agent_profiles.values().find(|profile| {
            profile
                .name
                .as_deref()
                .map(str::trim)
                .is_some_and(|profile_name| profile_name == name)
        })
    })
}

fn apply_registered_agent_profile(
    cluster_id: &str,
    agent: &mut AgentClusterAgentRegistration,
    profile: &ResourceAgentProfileConfig,
) -> Result<(), RuntimeError> {
    if agent.id.trim().is_empty() {
        agent.id = profile.id.clone();
    }
    let explicit_name = agent.name.trim().to_string();
    if explicit_name.is_empty() || explicit_name == profile.id {
        agent.name = profile.name.clone().unwrap_or_else(|| profile.id.clone());
    }
    if agent
        .role
        .as_ref()
        .map(|role| role.trim().is_empty())
        .unwrap_or(true)
    {
        agent.role = profile.role.clone();
    }
    if agent.features.is_empty() {
        agent.features = profile.features.clone();
    } else {
        let mut features = profile.features.clone();
        for feature in &agent.features {
            if !features.iter().any(|existing| existing == feature) {
                features.push(feature.clone());
            }
        }
        agent.features = features;
    }
    if agent.model_uid.is_none() {
        agent.model_uid = profile.model_uid;
    }
    if agent.retrieval.is_none() {
        agent.retrieval = profile.retrieval.clone();
    }
    if agent
        .system_prompt_constraints
        .frontend_widgets_enabled
        .is_none()
    {
        agent.system_prompt_constraints.frontend_widgets_enabled =
            profile.system_prompt_constraints.frontend_widgets_enabled;
    }
    if agent.frontend_widgets_enabled {
        agent.frontend_widgets_enabled = profile.frontend_widgets_enabled;
    }
    let _ = cluster_id;
    Ok(())
}

fn build_builtin_cluster_configs(
    registries: &RuntimeRegistries,
) -> Result<BuiltinClusterConfigs, RuntimeError> {
    let builtin_model_uid = registries
        .llm
        .as_ref()
        .and_then(|llm| llm.current_model_uid);
    let cluster = |id: &str, description: &str, agent_id: &str, agent_name: &str, role: &str| {
        let registration = AgentClusterRegistration {
            id: id.to_string(),
            description: description.to_string(),
            focus_agent_id: Some(agent_id.to_string()),
            agents: vec![AgentClusterAgentRegistration {
                id: agent_id.to_string(),
                name: agent_name.to_string(),
                role: Some(role.to_string()),
                model_uid: builtin_model_uid,
                ..AgentClusterAgentRegistration::default()
            }],
            ..AgentClusterRegistration::default()
        };
        if builtin_model_uid.is_some() {
            build_agent_cluster_registry(registration, registries)
        } else {
            // Legacy runtime configs may start without an LLM registration.
            // Keep the built-in cluster formal and defer model resolution until use.
            Ok(RuntimeAgentCluster {
                id: registration.id,
                description: registration.description,
                focus_agent_id: agent_id.to_string(),
                agents: vec![RuntimeAgentDefinition {
                    id: agent_id.to_string(),
                    name: agent_name.to_string(),
                    role: Some(role.to_string()),
                    features: Vec::new(),
                    model_uid: 0,
                    retrieval: None,
                    system_prompt_constraints: SystemPromptConstraints {
                        frontend_widgets_enabled: Some(true),
                    },
                    frontend_widgets_enabled: true,
                }],
                max_thinking_rounds: 0,
                permissions: registration.permissions,
            })
        }
    };

    Ok(BuiltinClusterConfigs {
        workflow_editor: cluster(
            "workflow-studio",
            "Workflow Studio editor conversation",
            "workflow-studio-editor",
            "Workflow Studio Editor",
            crate::workflow_studio::WORKFLOW_EDITOR_ROLE_SKILL,
        )?,
        agent_test_supervisor: cluster(
            "agent-test-supervisor",
            "Agent Test Studio supervisor conversation",
            "agent-test-supervisor",
            "Agent Test Supervisor",
            "agent_test_supervisor",
        )?,
        agent_test_adversary: cluster(
            "agent-test-adversary",
            "Adversarial actor conversation for Agent Test",
            "agent-test-adversary",
            "Agent Test Adversary",
            "agent_test_adversary",
        )?,
    })
}

fn cluster_focus_agent_section(
    cluster: &RuntimeAgentCluster,
) -> Result<AgentSection, RuntimeError> {
    let agent = cluster
        .agents
        .iter()
        .find(|agent| agent.id == cluster.focus_agent_id)
        .ok_or_else(|| {
            RuntimeError::InvalidConfig(format!(
                "built-in cluster '{}' focus agent '{}' was not found",
                cluster.id, cluster.focus_agent_id
            ))
        })?;
    Ok(runtime_agent_definition_to_agent_section(agent))
}

fn normalize_string_list(label: &str, values: Vec<String>) -> Result<Vec<String>, RuntimeError> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        validate_resource_id(label, &value)?;
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

fn parse_conversation_spawn_request(input: &str) -> Result<ConversationSpawnRequest, RuntimeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "conversation spawn request must not be empty".to_string(),
        ));
    }
    let mut request: ConversationSpawnRequest = serde_json::from_str(trimmed).map_err(|e| {
        RuntimeError::InvalidConfig(format!("parse conversation spawn failed: {e}"))
    })?;
    if request.schema.trim().is_empty() {
        request.schema = "agent-runtime-conversation-spawn/v1".to_string();
    }
    if request.schema != "agent-runtime-conversation-spawn/v1" {
        return Err(RuntimeError::InvalidConfig(format!(
            "conversation spawn schema '{}' is not supported",
            request.schema
        )));
    }
    validate_resource_id("conversation spawn cluster_id", &request.cluster_id)?;
    Ok(request)
}

fn effective_frontend_widgets_enabled(
    legacy_frontend_widgets_enabled: bool,
    constraints: &SystemPromptConstraints,
) -> bool {
    constraints
        .frontend_widgets_enabled
        .unwrap_or(legacy_frontend_widgets_enabled)
}

fn effective_system_prompt_constraints(
    legacy_frontend_widgets_enabled: bool,
    constraints: &SystemPromptConstraints,
) -> SystemPromptConstraints {
    SystemPromptConstraints {
        frontend_widgets_enabled: Some(effective_frontend_widgets_enabled(
            legacy_frontend_widgets_enabled,
            constraints,
        )),
    }
}

fn runtime_agent_definition_to_agent_section(agent: &RuntimeAgentDefinition) -> AgentSection {
    let model = if agent.model_uid == 0 {
        "default".to_string()
    } else {
        key_store::get(agent.model_uid)
            .map(|entry| entry.model_name)
            .unwrap_or_else(|| agent.model_uid.to_string())
    };
    AgentSection {
        id: agent.id.clone(),
        name: agent.name.clone(),
        is_default: true,
        model,
        max_tokens: 4096,
        skills_dir: PathBuf::from("skills"),
        role: agent.role.clone(),
        features: agent.features.clone(),
        retrieval: agent.retrieval.clone(),
        system_prompt_constraints: agent.system_prompt_constraints.clone(),
        frontend_widgets_enabled: agent.frontend_widgets_enabled,
    }
}

async fn apply_conversation_init_plan(
    manager: &ConversationManager,
    conversation_id: &str,
    init: &ConversationInitPlan,
) -> Result<(), RuntimeError> {
    if init.model_uid.is_none()
        && init.max_thinking_rounds.is_none()
        && init.agent_profiles.is_empty()
        && init.tool_host_context.is_none()
    {
        return Ok(());
    }
    let cache = manager
        .default_agent_cache(conversation_id)
        .await
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    if let Some(model_uid) = init.model_uid {
        let model_name = key_store::get(model_uid)
            .map(|entry| entry.model_name)
            .ok_or_else(|| RuntimeError::Llm(format!("model uid {model_uid} is not configured")))?;
        cache
            .set(ai_assistant::context::keys::MODEL, &model_name, None)
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    }
    if let Some(max_thinking_rounds) = init.max_thinking_rounds {
        cache
            .set(
                ai_assistant::context::keys::MAX_THINKING_ROUNDS,
                &max_thinking_rounds,
                None,
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    }
    if !init.agent_profiles.is_empty() {
        cache
            .set(
                ai_assistant::context::keys::AGENT_RESOURCE_PROFILES,
                &init.agent_profiles,
                None,
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    }
    if let Some(tool_host_context) = &init.tool_host_context {
        cache
            .set(
                ai_assistant::context::keys::TOOL_HOST_CONTEXT,
                tool_host_context,
                None,
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct RestoredConversationRecovery {
    entry_states: Vec<(String, String)>,
    execution_plans: Vec<RestoredExecutionPlan>,
}

#[derive(Debug, Clone)]
struct RestoredExecutionPlan {
    agent_id: String,
    tools: Vec<String>,
    call_ids: Vec<String>,
    recovery_results: BTreeMap<String, ai_assistant::ToolResult>,
}

#[derive(Debug, Clone)]
struct OpenToolCall {
    call_id: String,
    tool_name: Option<String>,
    tool_command: Option<String>,
    agent_id: String,
    turn_id: Option<u64>,
    source: OpenToolCallSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenToolCallSource {
    AssistantDeclared,
    Started,
}

fn restored_conversation_recovery(
    records: &[ai_assistant::ledger::LedgerRecord],
) -> RestoredConversationRecovery {
    let execution_plans = restored_execution_plans(records);
    if !execution_plans.is_empty() {
        return RestoredConversationRecovery {
            entry_states: Vec::new(),
            execution_plans,
        };
    }

    let entry_states = restored_entry_states(records);
    RestoredConversationRecovery {
        entry_states,
        execution_plans,
    }
}

fn restored_entry_states(records: &[ai_assistant::ledger::LedgerRecord]) -> Vec<(String, String)> {
    let Some(last) = records.last() else {
        return Vec::new();
    };
    let state = match last.role {
        ai_assistant::ledger::LedgerRole::User => Some(ai_assistant::state::states::THINKING),
        ai_assistant::ledger::LedgerRole::Assistant => Some(ai_assistant::state::states::SUSPENDED),
        ai_assistant::ledger::LedgerRole::Tool => Some(ai_assistant::state::states::THINKING),
        ai_assistant::ledger::LedgerRole::GatewayMessage if is_terminal_tool_record(last) => {
            Some(ai_assistant::state::states::THINKING)
        }
        _ => None,
    };
    state
        .map(|state| vec![(last.agent_id.clone(), state.to_string())])
        .unwrap_or_default()
}

fn restored_execution_plans(
    records: &[ai_assistant::ledger::LedgerRecord],
) -> Vec<RestoredExecutionPlan> {
    let mut open_calls = BTreeMap::<String, OpenToolCall>::new();
    for record in records.iter() {
        remember_assistant_declared_tool_calls(&mut open_calls, record);
        update_open_tool_calls(&mut open_calls, record);
    }
    if !open_calls.is_empty() {
        return execution_plans_from_open_calls(open_calls);
    }

    let Some(last) = records.last() else {
        return Vec::new();
    };
    if last.role == ai_assistant::ledger::LedgerRole::Assistant {
        if let Ok(tool_calls) = ai_assistant::runtime::parser::parse_tool_calls(&last.content) {
            if !tool_calls.is_empty() {
                let tools = tool_calls
                    .iter()
                    .map(|call| call.to_legacy_command())
                    .collect::<Vec<_>>();
                let mut call_ids = last
                    .metadata
                    .extra
                    .get("tool_call_ids")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                while call_ids.len() < tools.len() {
                    call_ids.push(format!("recovered:{}:{}", last.record_id, call_ids.len()));
                }
                call_ids.truncate(tools.len());
                return vec![RestoredExecutionPlan {
                    agent_id: last.agent_id.clone(),
                    tools,
                    call_ids,
                    recovery_results: BTreeMap::new(),
                }];
            }
        }
    }

    Vec::new()
}

fn execution_plans_from_open_calls(
    open_calls: BTreeMap<String, OpenToolCall>,
) -> Vec<RestoredExecutionPlan> {
    let mut grouped = BTreeMap::<String, RestoredExecutionPlan>::new();
    for open_call in open_calls.into_values() {
        let Some(command) = open_call.tool_command.clone() else {
            continue;
        };
        let entry = grouped
            .entry(open_call.agent_id.clone())
            .or_insert_with(|| RestoredExecutionPlan {
                agent_id: open_call.agent_id.clone(),
                tools: Vec::new(),
                call_ids: Vec::new(),
                recovery_results: BTreeMap::new(),
            });
        entry.tools.push(command.clone());
        entry.call_ids.push(open_call.call_id.clone());
        if recovery_tool_effect(open_call.tool_name.as_deref()) != "read_only" {
            entry.recovery_results.insert(
                open_call.call_id.clone(),
                recovery_tool_result(&open_call, &command),
            );
        }
    }
    grouped
        .into_values()
        .filter(|plan| !plan.tools.is_empty())
        .collect()
}

#[cfg(test)]
fn repair_restored_ledger(
    records: &mut Vec<ai_assistant::ledger::LedgerRecord>,
    target_conversation_id: &str,
) {
    let mut next_record_id = records
        .iter()
        .map(|record| record.record_id)
        .max()
        .unwrap_or(0)
        + 1;
    for plan in restored_execution_plans(records) {
        for (index, call_id) in plan.call_ids.iter().enumerate() {
            let command = plan.tools.get(index).cloned().unwrap_or_default();
            let result = plan
                .recovery_results
                .get(call_id)
                .cloned()
                .unwrap_or_else(|| {
                    let (tool_name, _) = ai_assistant::tool_runner::parse_tool_command(&command);
                    recovery_tool_result(
                        &OpenToolCall {
                            call_id: call_id.clone(),
                            tool_name: Some(tool_name.to_string()),
                            tool_command: Some(command.clone()),
                            agent_id: plan.agent_id.clone(),
                            turn_id: None,
                            source: OpenToolCallSource::AssistantDeclared,
                        },
                        &command,
                    )
                });
            let (tool_name, _) = ai_assistant::tool_runner::parse_tool_command(&result.command);
            let mut metadata = ai_assistant::ledger::LedgerMessageMeta::default();
            metadata.subtype =
                Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FAILED.to_string());
            metadata.tool_name = Some(tool_name.to_string());
            metadata.tool_command = Some(result.command.clone());
            metadata.success = Some(false);
            metadata.collapsed = Some(true);
            metadata.extra.insert("kind".to_string(), json!("tool"));
            metadata
                .extra
                .insert("status".to_string(), json!("recovery_interrupted"));
            metadata.extra.insert("call_id".to_string(), json!(call_id));
            if let Some(object) = result.result.as_object() {
                for key in ["recovery_kind", "effect", "source"] {
                    if let Some(value) = object.get(key) {
                        metadata.extra.insert(key.to_string(), value.clone());
                    }
                }
            }
            records.push(ai_assistant::ledger::LedgerRecord {
                record_id: next_record_id,
                conversation_id: target_conversation_id.to_string(),
                agent_id: plan.agent_id.clone(),
                agent_name: plan.agent_id.clone(),
                role: ai_assistant::ledger::LedgerRole::Tool,
                content: result.to_ai.clone(),
                metadata,
                created_at: chrono::Local::now().to_rfc3339(),
            });
            next_record_id += 1;
        }
    }
}

fn remember_assistant_declared_tool_calls(
    open_calls: &mut BTreeMap<String, OpenToolCall>,
    record: &ai_assistant::ledger::LedgerRecord,
) {
    if record.role != ai_assistant::ledger::LedgerRole::Assistant {
        return;
    }
    let Some(call_ids) = record
        .metadata
        .extra
        .get("tool_call_ids")
        .and_then(Value::as_array)
    else {
        return;
    };
    let commands = ai_assistant::runtime::parser::parse_tool_calls(&record.content)
        .map(|calls| {
            calls
                .into_iter()
                .map(|call| call.to_legacy_command())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (index, call_id) in call_ids.iter().filter_map(Value::as_str).enumerate() {
        let command = commands.get(index).cloned();
        let tool_name = command
            .as_deref()
            .map(ai_assistant::tool_runner::parse_tool_command)
            .map(|(name, _)| name.to_string());
        open_calls
            .entry(call_id.to_string())
            .or_insert(OpenToolCall {
                call_id: call_id.to_string(),
                tool_name,
                tool_command: command,
                agent_id: record.agent_id.clone(),
                turn_id: record.metadata.extra.get("turn_id").and_then(Value::as_u64),
                source: OpenToolCallSource::AssistantDeclared,
            });
    }
}

fn update_open_tool_calls(
    open_calls: &mut BTreeMap<String, OpenToolCall>,
    record: &ai_assistant::ledger::LedgerRecord,
) {
    let Some(call_id) = record
        .metadata
        .extra
        .get("call_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    if is_terminal_tool_record(record) {
        open_calls.remove(&call_id);
        return;
    }
    if is_started_tool_record(record) {
        open_calls.insert(
            call_id.clone(),
            OpenToolCall {
                call_id,
                tool_name: record.metadata.tool_name.clone(),
                tool_command: record.metadata.tool_command.clone(),
                agent_id: record.agent_id.clone(),
                turn_id: record.metadata.extra.get("turn_id").and_then(Value::as_u64),
                source: OpenToolCallSource::Started,
            },
        );
    }
}

fn is_started_tool_record(record: &ai_assistant::ledger::LedgerRecord) -> bool {
    record.metadata.subtype.as_deref()
        == Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED)
        || record.metadata.extra.get("status").and_then(Value::as_str) == Some("running")
}

fn is_terminal_tool_record(record: &ai_assistant::ledger::LedgerRecord) -> bool {
    matches!(
        record.metadata.subtype.as_deref(),
        Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FINISHED)
            | Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FAILED)
    ) || matches!(
        record.metadata.extra.get("status").and_then(Value::as_str),
        Some("success") | Some("error") | Some("failed") | Some("canceled")
    )
}

fn recovery_tool_effect(tool_name: Option<&str>) -> &'static str {
    let Some(tool_name) = tool_name else {
        return "unknown";
    };
    match ai_assistant::tool_runner::permission_metadata(tool_name).map(|metadata| metadata.effect)
    {
        Ok(ai_assistant::ToolEffect::ReadOnly) => "read_only",
        Ok(ai_assistant::ToolEffect::ControlledChange) => "controlled_change",
        Ok(ai_assistant::ToolEffect::Destructive) => "destructive",
        Err(_) => "unknown",
    }
}

fn recovery_tool_result(open_call: &OpenToolCall, command: &str) -> ai_assistant::ToolResult {
    let effect = recovery_tool_effect(open_call.tool_name.as_deref());
    ai_assistant::ToolResult {
        command: command.to_string(),
        success: false,
        to_ai: recovery_tool_to_ai(open_call, effect),
        error_code: 1,
        result: json!({
            "status": "recovery_interrupted",
            "recovery_kind": "unfinished_tool_call",
            "call_id": open_call.call_id,
            "effect": effect,
            "source": match open_call.source {
                OpenToolCallSource::AssistantDeclared => "assistant_declared",
                OpenToolCallSource::Started => "started",
            },
            "turn_id": open_call.turn_id,
        }),
    }
}

fn recovery_tool_to_ai(open_call: &OpenToolCall, effect: &str) -> String {
    let tool_name = open_call.tool_name.as_deref().unwrap_or("unknown");
    let command = recovery_command_line(open_call.tool_command.as_deref().unwrap_or(""));
    if effect == "read_only" {
        return format!(
            "The runtime was interrupted before this tool call produced a closed result.\n\
tool_call_id: {}\n\
tool_name: {}\n\
effect: read_only\n\
This tool is classified as read-only, so it may be run again if the current state still needs a fresh observation.\n\
{}",
            open_call.call_id, tool_name, command
        );
    }
    format!(
        "The runtime was interrupted before this tool call produced a closed result.\n\
tool_call_id: {}\n\
tool_name: {}\n\
effect: {}\n\
Do not assume the operation succeeded, and do not directly repeat the same operation. If it may have side effects, first verify the external system state or report the interruption and re-plan.\n\
For child agents, report the uncertainty to the parent agent instead of asking the user directly.\n\
{}",
        open_call.call_id, tool_name, effect, command
    )
}

fn recovery_command_line(command: &str) -> String {
    if command.trim().is_empty() {
        String::new()
    } else {
        format!("command: {command}")
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ConversationOptionsInput {
    conversation_id: Option<String>,
    #[serde(alias = "external_id")]
    _external_id: Option<String>,
    tenant_id: Option<String>,
    user_id: Option<String>,
    #[serde(
        alias = "llm_headers",
        alias = "request_headers",
        alias = "llmRequestHeaders",
        alias = "requestHeaders",
        default
    )]
    llm_request_headers: Option<BTreeMap<String, String>>,
    #[serde(
        alias = "allow_insecure_llm_headers",
        alias = "allowInsecureLlmHeaders",
        alias = "allowInsecureLlmRequestHeaders",
        default
    )]
    allow_insecure_llm_request_headers: bool,
}

#[derive(Debug, Deserialize)]
struct ConversationSnapshotImport {
    #[allow(dead_code)]
    schema: Option<String>,
    #[allow(dead_code)]
    conversation_id: Option<String>,
    #[serde(default)]
    ledger: Vec<ai_assistant::ledger::LedgerRecord>,
    #[serde(
        default,
        alias = "state_delta",
        alias = "stateDeltas",
        deserialize_with = "deserialize_state_deltas"
    )]
    state_deltas: Vec<Value>,
    #[serde(default)]
    state: Option<Value>,
    #[allow(dead_code)]
    runtime: Option<serde_json::Value>,
}

fn deserialize_state_deltas<'de, D>(deserializer: D) -> Result<Vec<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?.unwrap_or(Value::Null);
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => Ok(items),
        Value::Object(_) => Ok(vec![value]),
        _ => Err(de::Error::custom("state_deltas must be an object or array")),
    }
}

fn snapshot_state_deltas(snapshot: &ConversationSnapshotImport) -> Vec<Value> {
    let mut deltas = snapshot.state_deltas.clone();
    if let Some(state) = &snapshot.state {
        if let Some(items) = state.get("deltas").and_then(Value::as_array) {
            deltas.extend(items.iter().cloned());
        }
        if let Some(items) = state.get("state_deltas").and_then(Value::as_array) {
            deltas.extend(items.iter().cloned());
        }
    }
    deltas
}

fn parse_conversation_snapshot(
    snapshot_json: &str,
) -> Result<ConversationSnapshotImport, RuntimeError> {
    let snapshot: ConversationSnapshotImport =
        serde_json::from_str(snapshot_json).map_err(|e| {
            RuntimeError::InvalidConfig(format!("parse conversation snapshot failed: {e}"))
        })?;
    if snapshot.schema.as_deref() != Some("agent-runtime-conversation-snapshot/v1") {
        return Err(RuntimeError::InvalidConfig(
            "conversation snapshot schema must be 'agent-runtime-conversation-snapshot/v1'"
                .to_string(),
        ));
    }
    Ok(snapshot)
}

async fn apply_conversation_state_delta(
    manager: &ConversationManager,
    conversation_id: &str,
    delta: Value,
) -> Result<(), RuntimeError> {
    let op = match delta.get("op").and_then(Value::as_str) {
        Some(op) if !op.trim().is_empty() => op,
        _ => return Ok(()),
    };

    match op {
        "focus.set" => {
            let Some(agent_id) = delta
                .get("focus_agent_id")
                .or_else(|| delta.get("agent_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            manager
                .set_focus(conversation_id, agent_id.to_string())
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        "dynamic_snapshot.set" => {
            let Some(agent_id) = delta
                .get("agent_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let Some(field) = delta
                .get("field")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let text = delta
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            manager
                .set_host_dynamic_snapshot_field(conversation_id, agent_id, field, text)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        "agent_task.upsert" => {
            let task_value = delta.get("task").cloned().unwrap_or_else(|| delta.clone());
            match serde_json::from_value::<ai_assistant::conversation_state::AgentTaskEntry>(
                task_value,
            ) {
                Ok(task) => {
                    manager
                        .upsert_agent_task(conversation_id, task)
                        .await
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                }
                Err(error) => {
                    tracing::warn!("skip invalid restored agent_task.upsert delta: {}", error);
                }
            }
        }
        "agent_skills.set" => {
            let Some(agent_id) = delta
                .get("agent_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let cache = match manager.agent_cache(conversation_id, agent_id).await {
                Ok(cache) => cache,
                Err(error) => {
                    tracing::warn!(
                        "skip restored agent_skills.set for unavailable agent '{}': {}",
                        agent_id,
                        error
                    );
                    return Ok(());
                }
            };
            if let Some(main_skills) = string_array_field(&delta, "main_skills") {
                cache
                    .set(ai_assistant::context::keys::MAIN_SKILLS, &main_skills, None)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            if let Some(imported_skills) = string_array_field(&delta, "imported_skills") {
                cache
                    .set(
                        ai_assistant::context::keys::IMPORTED_SKILLS,
                        &imported_skills,
                        None,
                    )
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            if let Some(active_tools) = string_array_field(&delta, "active_tools") {
                AssistantContext::set_active_tools(&cache, active_tools)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
        }
        "agent_plan.set" => {
            let Some(agent_id) = delta
                .get("agent_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let Some(plan_value) = delta.get("plan").cloned() else {
                return Ok(());
            };
            let plan =
                match serde_json::from_value::<ai_assistant::context::CurrentPlan>(plan_value) {
                    Ok(plan) => plan,
                    Err(error) => {
                        tracing::warn!("skip invalid restored agent_plan.set delta: {}", error);
                        return Ok(());
                    }
                };
            let cache = match manager.agent_cache(conversation_id, agent_id).await {
                Ok(cache) => cache,
                Err(error) => {
                    tracing::warn!(
                        "skip restored agent_plan.set for unavailable agent '{}': {}",
                        agent_id,
                        error
                    );
                    return Ok(());
                }
            };
            AssistantContext::set_current_plan(&cache, &plan)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        _ => {}
    }
    Ok(())
}

fn string_array_field(value: &Value, field: &str) -> Option<Vec<String>> {
    value.get(field).and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
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

fn parse_conversation_options(input: &str) -> Result<ConversationOptionsInput, RuntimeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(ConversationOptionsInput::default());
    }
    serde_json::from_str::<ConversationOptionsInput>(trimmed)
        .map_err(|e| RuntimeError::InvalidConfig(format!("parse conversation options failed: {e}")))
}

fn parse_ai_auth_context_headers(input: &str) -> Result<BTreeMap<String, String>, RuntimeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(BTreeMap::new());
    }

    let value: Value = serde_json::from_str(trimmed)
        .map_err(|e| RuntimeError::InvalidConfig(format!("parse ai auth context failed: {e}")))?;
    let Value::Object(object) = value else {
        return Err(RuntimeError::InvalidConfig(
            "ai auth context must be a JSON object".to_string(),
        ));
    };

    if let Some(headers_value) = object.get("headers") {
        if headers_value.is_null() {
            return Ok(BTreeMap::new());
        }
        if let Value::Object(headers) = headers_value {
            return header_map_from_json_object(headers);
        }
        return Err(RuntimeError::InvalidConfig(
            "ai auth context headers must be a JSON object".to_string(),
        ));
    }

    if object.contains_key("access_token")
        || object.contains_key("accessToken")
        || object.contains_key("app_meta")
        || object.contains_key("appMeta")
    {
        return headers_from_auth_fields(&object);
    }

    header_map_from_json_object(&object)
}

fn headers_from_auth_fields(
    object: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut headers = BTreeMap::new();
    let access_token = object
        .get("access_token")
        .or_else(|| object.get("accessToken"))
        .and_then(header_value_from_json);
    if let Some(token) = access_token {
        let authorization = if token
            .get(..7)
            .map(|prefix| prefix.eq_ignore_ascii_case("bearer "))
            .unwrap_or(false)
        {
            token
        } else {
            format!("Bearer {token}")
        };
        headers.insert("Authorization".to_string(), authorization);
    }

    let app_meta = object
        .get("app_meta")
        .or_else(|| object.get("appMeta"))
        .and_then(header_value_from_json);
    if let Some(app_meta) = app_meta {
        headers.insert("X-App-Meta".to_string(), app_meta);
    }

    Ok(headers)
}

fn header_map_from_json_object(
    object: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, RuntimeError> {
    let mut headers = BTreeMap::new();
    for (name, value) in object {
        if let Some(value) = header_value_from_json(value) {
            headers.insert(name.clone(), value);
        }
    }
    Ok(headers)
}

fn header_value_from_json(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        }
        other => Some(other.to_string()),
    }
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

pub(crate) fn lease_renew_interval(lock_ttl_ms: u64, renew_interval_ms: u64) -> Duration {
    let interval_ms = if renew_interval_ms > 0 {
        renew_interval_ms
    } else {
        (lock_ttl_ms / 3).max(1)
    };
    Duration::from_millis(interval_ms.min(lock_ttl_ms.max(1)).max(1))
}

fn conversation_index_key(cluster_id: &str) -> String {
    format!("runtime:{}:conversations:index", cluster_id)
}

fn conversation_metadata_key(cluster_id: &str, conversation_id: &str) -> String {
    format!(
        "runtime:{}:conversation:{}:metadata",
        cluster_id, conversation_id
    )
}

fn conversation_catalog_lease_key(cluster_id: &str) -> String {
    format!("runtime:{}:conversations:catalog", cluster_id)
}

fn conversation_owner_lease_key(cluster_id: &str, conversation_id: &str) -> String {
    format!(
        "runtime:{}:conversation:{}:owner",
        cluster_id, conversation_id
    )
}

async fn load_conversation_index(
    state_store: &Arc<dyn RuntimeStateStore>,
    cluster_id: &str,
) -> Result<Vec<ConversationInfo>, RuntimeError> {
    let Some(value) = state_store
        .get_json(&conversation_index_key(cluster_id))
        .await?
    else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value)
        .map_err(|e| RuntimeError::Internal(format!("parse conversation index failed: {e}")))
}

async fn with_conversation_catalog_lease<F, Fut, T>(
    coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    cluster_id: String,
    owner: String,
    ttl_ms: u64,
    operation: F,
) -> Result<T, RuntimeError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, RuntimeError>>,
{
    let lease_key = conversation_catalog_lease_key(&cluster_id);
    let acquired = coordination_backend
        .acquire_lease(&lease_key, &owner, ttl_ms)
        .await?;
    if !acquired {
        return Err(RuntimeError::Internal(
            "conversation catalog lease is held by another runtime".to_string(),
        ));
    }

    let result = operation().await;
    let release_result = coordination_backend.release_lease(&lease_key, &owner).await;
    match (result, release_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(release_error)) => {
            tracing::warn!(
                "release conversation catalog lease failed: {}",
                release_error
            );
            Err(error)
        }
    }
}

async fn record_conversation_created(
    state_store: Arc<dyn RuntimeStateStore>,
    coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    cluster_id: String,
    owner: String,
    ttl_ms: u64,
    info: ConversationInfo,
) -> Result<(), RuntimeError> {
    with_conversation_catalog_lease(
        coordination_backend,
        cluster_id.clone(),
        owner,
        ttl_ms,
        || async {
            let metadata_key = conversation_metadata_key(&cluster_id, &info.conversation_id);
            state_store
                .put_json(
                    &metadata_key,
                    serde_json::to_value(&info).map_err(|e| {
                        RuntimeError::Internal(format!(
                            "serialize conversation metadata failed: {e}"
                        ))
                    })?,
                    None,
                )
                .await?;

            let mut index = load_conversation_index(&state_store, &cluster_id).await?;
            index.retain(|entry| entry.conversation_id != info.conversation_id);
            index.push(info);
            index.sort_by(|left, right| left.conversation_id.cmp(&right.conversation_id));
            state_store
                .put_json(
                    &conversation_index_key(&cluster_id),
                    serde_json::to_value(index).map_err(|e| {
                        RuntimeError::Internal(format!("serialize conversation index failed: {e}"))
                    })?,
                    None,
                )
                .await
        },
    )
    .await
}

async fn publish_conversation_created_event(
    event_bus: Arc<dyn EventBus>,
    info: &ConversationInfo,
    cluster_id: &str,
    cluster_description: &str,
) -> Result<(), RuntimeError> {
    event_bus
        .publish(BaseEvent::new(
            CONVERSATION_CREATED_EVENT,
            json!({
                "conversation_id": info.conversation_id,
                "scope_id": info.scope_id,
                "tenant_id": info.tenant_id,
                "user_id": info.user_id,
                "cluster_id": cluster_id,
                "cluster_description": cluster_description,
                "created_at": info.created_at,
            }),
        ))
        .await
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

async fn publish_conversation_closed_event(
    event_bus: Arc<dyn EventBus>,
    conversation_id: &str,
    cluster_id: &str,
    cluster_description: &str,
) -> Result<(), RuntimeError> {
    event_bus
        .publish(BaseEvent::new(
            CONVERSATION_CLOSED_EVENT,
            json!({
                "conversation_id": conversation_id,
                "cluster_id": cluster_id,
                "cluster_description": cluster_description,
                "closed_at": chrono::Utc::now(),
            }),
        ))
        .await
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

async fn record_conversation_closed(
    state_store: Arc<dyn RuntimeStateStore>,
    coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    cluster_id: String,
    owner: String,
    ttl_ms: u64,
    conversation_id: String,
) -> Result<(), RuntimeError> {
    with_conversation_catalog_lease(
        coordination_backend,
        cluster_id.clone(),
        owner,
        ttl_ms,
        || async {
            state_store
                .delete(&conversation_metadata_key(&cluster_id, &conversation_id))
                .await?;
            let mut index = load_conversation_index(&state_store, &cluster_id).await?;
            index.retain(|entry| entry.conversation_id != conversation_id);
            state_store
                .put_json(
                    &conversation_index_key(&cluster_id),
                    serde_json::to_value(index).map_err(|e| {
                        RuntimeError::Internal(format!("serialize conversation index failed: {e}"))
                    })?,
                    None,
                )
                .await
        },
    )
    .await
}

async fn acquire_conversation_owner_lease(
    backend: Arc<dyn RuntimeCoordinationBackend>,
    cluster_id: String,
    conversation_id: String,
    owner: String,
    ttl_ms: u64,
    renew_interval: Duration,
) -> Result<Option<ConversationOwnerLease>, RuntimeError> {
    let owner_lease_key = conversation_owner_lease_key(&cluster_id, &conversation_id);
    let owner_acquired = backend
        .acquire_lease(&owner_lease_key, &owner, ttl_ms)
        .await?;
    if !owner_acquired {
        return Ok(None);
    }

    let (stop_renewer, renewer_stop_rx) = watch::channel(false);
    let renewer = tokio::spawn(run_lease_renewer(
        Arc::clone(&backend),
        owner_lease_key.clone(),
        owner.clone(),
        ttl_ms,
        renew_interval,
        renewer_stop_rx,
    ));
    Ok(Some(ConversationOwnerLease {
        stop_renewer,
        renewer,
        key: owner_lease_key,
        owner,
    }))
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

pub(crate) async fn run_lease_renewer(
    backend: Arc<dyn RuntimeCoordinationBackend>,
    key: String,
    owner: String,
    ttl_ms: u64,
    interval: Duration,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
            _ = sleep(interval) => {
                match backend.renew_lease(&key, &owner, ttl_ms).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!("conversation turn lease renewal lost ownership");
                        return;
                    }
                    Err(error) => {
                        tracing::warn!("conversation turn lease renewal failed: {}", error);
                        return;
                    }
                }
            }
        }
    }
}

fn resolve_config_base(config_dir: Option<&PathBuf>, path: Option<&Path>) -> PathBuf {
    let path = path.filter(|path| !path.as_os_str().is_empty());
    let Some(path) = path else {
        return config_dir.cloned().unwrap_or_else(|| PathBuf::from("."));
    };
    if path.is_absolute() {
        return path.to_path_buf();
    }
    config_dir
        .map(|dir| dir.join(path))
        .unwrap_or_else(|| path.to_path_buf())
}

fn resolve_config_storage_dir(
    config_dir: Option<&PathBuf>,
    path: Option<&Path>,
    fixed_dir: &str,
) -> PathBuf {
    resolve_config_base(config_dir, path).join(fixed_dir)
}

#[allow(dead_code)]
fn resolve_explicit_or_default_dir(
    config_dir: Option<&PathBuf>,
    path: Option<&Path>,
    default_dir: &str,
) -> PathBuf {
    match path.filter(|path| !path.as_os_str().is_empty()) {
        Some(path) => resolve_config_base(config_dir, Some(path)),
        None => resolve_config_base(config_dir, None).join(default_dir),
    }
}

#[allow(dead_code)]
fn resolve_config_storage_file(
    config_dir: Option<&PathBuf>,
    path: Option<&Path>,
    fixed_file: &str,
) -> PathBuf {
    let base = resolve_config_base(config_dir, path);
    if base
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return base;
    }
    base.join(fixed_file)
}

fn parse_config_input(input: &str) -> Result<RuntimeConfig, RuntimeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(RuntimeConfig::default());
    }
    if !trimmed.starts_with('{') {
        return Err(RuntimeError::InvalidConfig(
            "runtime create input must be empty or inline agent-runtime-create-options/v1 JSON; config file paths are not supported".to_string(),
        ));
    }
    let mut options: RuntimeCreateOptions = serde_json::from_str(trimmed).map_err(|error| {
        RuntimeError::InvalidConfig(format!("parse runtime create options failed: {error}"))
    })?;
    if options.schema.trim().is_empty() {
        options.schema = "agent-runtime-create-options/v1".to_string();
    }
    if options.schema != "agent-runtime-create-options/v1" {
        return Err(RuntimeError::InvalidConfig(format!(
            "runtime create options schema '{}' is not supported",
            options.schema
        )));
    }
    RuntimeConfig::from_create_options(options)
}

fn config_parent_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn load_fixed_llm_config(config: &RuntimeConfig) -> Result<llm_gateway::LlmConfig, RuntimeError> {
    let Some(config_path) = config.runtime.llm_config_path.as_ref() else {
        let config = llm_gateway::LlmConfig::default();
        llm_gateway::build_index_and_resolver(config.clone());
        return Ok(config);
    };
    let config = if config_path.exists() {
        llm_gateway::load_config_from_path(config_path)
    } else {
        Ok(llm_gateway::LlmConfig::default())
    }
    .map_err(|e| {
        RuntimeError::InvalidConfig(format!(
            "load llm_config.json failed from {}: {e}",
            config_path.display()
        ))
    })?;
    validate_llm_config(&config)?;
    llm_gateway::build_index_and_resolver(config.clone());
    Ok(config)
}

#[allow(dead_code)]
fn validate_runtime_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    if config
        .runtime
        .skills_dir
        .as_ref()
        .is_none_or(|path| path.as_os_str().is_empty())
    {
        return Err(RuntimeError::InvalidConfig(
            "runtime.skills_dir must not be empty".to_string(),
        ));
    }

    ai_assistant::prompt_assets::normalize_language(&config.runtime.language)
        .map_err(RuntimeError::InvalidConfig)?;

    if config.runtime.cluster_id.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "runtime.cluster_id must not be empty".to_string(),
        ));
    }
    if config.runtime.runtime_profile_id.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "runtime.runtime_profile_id must not be empty".to_string(),
        ));
    }
    if config.runtime.runtime_instance_id.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "runtime.runtime_instance_id must not be empty".to_string(),
        ));
    }
    let mut agent_ids = std::collections::HashSet::new();
    let mut default_count = 0usize;
    for agent in &config.agents {
        if agent.id.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "agents[].id must not be empty".to_string(),
            ));
        }
        if !agent_ids.insert(agent.id.clone()) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agents[].id '{}' is duplicated",
                agent.id
            )));
        }
        if agent.name.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(format!(
                "agents['{}'].name must not be empty",
                agent.id
            )));
        }
        if agent.is_default {
            default_count += 1;
        }
        if matches!(&agent.role, Some(role) if role.trim().is_empty()) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agents['{}'].role must not be empty when provided",
                agent.id
            )));
        }
        if agent.features.iter().any(|name| name.trim().is_empty()) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agents['{}'].features must not contain empty strings",
                agent.id
            )));
        }
    }
    if default_count != 1 {
        return Err(RuntimeError::InvalidConfig(format!(
            "exactly one agent must have is_default=true, got {default_count}"
        )));
    }
    if config.retrieval.is_some() {
        return Err(RuntimeError::InvalidConfig(
            "top-level retrieval is no longer supported; configure agents[].retrieval".to_string(),
        ));
    }
    if config.runtime.retrieval.is_some() {
        return Err(RuntimeError::InvalidConfig(
            "runtime.retrieval is no longer supported; configure agents[].retrieval".to_string(),
        ));
    }

    let mut endpoint_ids = std::collections::HashSet::new();
    for endpoint in &config.rpc_tools {
        if endpoint.endpoint_id.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "rpc_tools[].endpoint_id must not be empty".to_string(),
            ));
        }
        if !endpoint_ids.insert(endpoint.endpoint_id.clone()) {
            return Err(RuntimeError::InvalidConfig(format!(
                "rpc_tools[].endpoint_id '{}' is duplicated",
                endpoint.endpoint_id
            )));
        }
        if endpoint.address.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(format!(
                "rpc_tools['{}'].address must not be empty",
                endpoint.endpoint_id
            )));
        }
        match endpoint.protocol.as_str() {
            "grpc" => {
                if !endpoint.tools.is_empty() {
                    return Err(RuntimeError::InvalidConfig(format!(
                        "rpc_tools['{}'] uses protocol=grpc; tools must be discovered by ListTools",
                        endpoint.endpoint_id
                    )));
                }
            }
            "json-lines" => {}
            other => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "rpc_tools['{}'].protocol '{}' is not supported",
                    endpoint.endpoint_id, other
                )));
            }
        }

        let launch = endpoint
            .launch
            .clone()
            .unwrap_or_else(RpcToolLaunchConfig::default);
        match launch.kind.as_str() {
            "external" => {}
            "process" => {
                let Some(program) = launch.program.as_ref() else {
                    return Err(RuntimeError::InvalidConfig(format!(
                        "rpc_tools['{}'].launch.program is required when launch.kind=process",
                        endpoint.endpoint_id
                    )));
                };
                if program.as_os_str().is_empty() {
                    return Err(RuntimeError::InvalidConfig(format!(
                        "rpc_tools['{}'].launch.program must not be empty",
                        endpoint.endpoint_id
                    )));
                }
            }
            other => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "rpc_tools['{}'].launch.kind '{}' is not supported",
                    endpoint.endpoint_id, other
                )));
            }
        }
    }

    for agent in &config.agents {
        let Some(retrieval) = agent.retrieval.as_ref() else {
            continue;
        };
        let path = format!("agents['{}'].retrieval", agent.id);
        validate_retrieval_config(&path, retrieval)?;
        if !retrieval.enabled {
            continue;
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
        let endpoint = config
            .rpc_tools
            .iter()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .ok_or_else(|| {
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
    }

    Ok(())
}

fn validate_retrieval_config(path: &str, config: &RetrievalConfig) -> Result<(), RuntimeError> {
    if !config.enabled {
        return Ok(());
    }
    if config.tool_name.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "{path}.tool_name must not be empty when retrieval is enabled"
        )));
    }
    if config.mode != "before_thinking" {
        return Err(RuntimeError::InvalidConfig(format!(
            "{path}.mode '{}' is not supported; use 'before_thinking'",
            config.mode
        )));
    }
    if config.trigger != "first_thinking_per_user_turn" {
        return Err(RuntimeError::InvalidConfig(format!(
            "{path}.trigger '{}' is not supported; use 'first_thinking_per_user_turn'",
            config.trigger
        )));
    }
    if config.inject_as != "dynamic_context" {
        return Err(RuntimeError::InvalidConfig(format!(
            "{path}.inject_as '{}' is not supported; use 'dynamic_context'",
            config.inject_as
        )));
    }
    Ok(())
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

async fn validate_default_agent_role_skill(config: &RuntimeConfig) -> Result<(), RuntimeError> {
    let agent = config.default_agent();
    let Some(role) = agent
        .role
        .as_deref()
        .map(str::trim)
        .filter(|role| !role.is_empty())
    else {
        return Ok(());
    };
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
    let skill = manager.load(role).await.map_err(|e| {
        RuntimeError::InvalidConfig(format!(
            "default agent '{}' role skill '{}' failed to load from {}: {e}",
            agent.id,
            role,
            skills_dir.display()
        ))
    })?;

    if !skill.metadata.is_role() {
        return Err(RuntimeError::InvalidConfig(format!(
            "default agent '{}' role skill '{}' must have kind=role",
            agent.id, role
        )));
    }
    if skill.instructions.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "default agent '{}' role skill '{}' must include non-empty instructions",
            agent.id, role
        )));
    }

    Ok(())
}

#[allow(dead_code)]
fn parse_provider_array(input: &str) -> Result<Vec<llm_gateway::UserProviderConfig>, RuntimeError> {
    if let Ok(providers) = serde_json::from_str::<Vec<HostUserProviderConfig>>(input) {
        return Ok(providers.into_iter().map(Into::into).collect());
    }
    serde_json::from_str::<Vec<llm_gateway::UserProviderConfig>>(input)
        .map_err(|e| RuntimeError::InvalidConfig(format!("parse providers_json failed: {e}")))
}

fn validate_llm_config(config: &llm_gateway::LlmConfig) -> Result<(), RuntimeError> {
    let mut provider_ids = std::collections::HashSet::new();
    let mut model_uids = std::collections::HashSet::new();
    for provider in &config.providers {
        if provider.id == 0 {
            return Err(RuntimeError::InvalidConfig(
                "providers[].id must not be 0".to_string(),
            ));
        }
        if !provider_ids.insert(provider.id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "providers[].id '{}' is duplicated",
                provider.id
            )));
        }
        if provider.name.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(format!(
                "providers['{}'].name must not be empty",
                provider.id
            )));
        }
        if provider.builtin_type.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(format!(
                "providers['{}'].type must not be empty",
                provider.id
            )));
        }
        if provider.prompt_cache_control
            && provider.api_paradigm != Some(llm_gateway::ApiParadigm::AnthropicMessages)
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "providers['{}'].prompt_cache_control requires api_paradigm 'anthropic_messages'",
                provider.id
            )));
        }
        for model in &provider.enabled_models {
            if model.uid == 0 {
                return Err(RuntimeError::InvalidConfig(format!(
                    "providers['{}'].enabled_models[].uid must not be 0",
                    provider.id
                )));
            }
            if !model_uids.insert(model.uid) {
                return Err(RuntimeError::InvalidConfig(format!(
                    "enabled model uid '{}' is duplicated",
                    model.uid
                )));
            }
            if model.model_id.trim().is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "providers['{}'].enabled_models['{}'].model_id must not be empty",
                    provider.id, model.uid
                )));
            }
        }
    }
    Ok(())
}

fn validate_current_model(
    current_model_uid: Option<u32>,
    config: &llm_gateway::LlmConfig,
) -> Result<(), RuntimeError> {
    let Some(uid) = current_model_uid else {
        return Ok(());
    };
    if config
        .providers
        .iter()
        .flat_map(|provider| provider.enabled_models.iter())
        .any(|model| model.uid == uid)
    {
        return Ok(());
    }
    Err(RuntimeError::Llm(format!(
        "current_model_uid {uid} is not configured"
    )))
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
