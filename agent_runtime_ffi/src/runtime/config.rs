use std::path::{Path, PathBuf};

use ai_assistant::{RetrievalConfig, SystemPromptConstraints};
use corework::prelude::RuntimeStateConfig as CoreRuntimeStateConfig;
use serde::{Deserialize, Serialize};

use super::{RpcToolEndpointConfig, RpcToolLaunchConfig, RuntimeError};

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
    pub(crate) fn from_create_options(options: RuntimeCreateOptions) -> Result<Self, RuntimeError> {
        let mut config = RuntimeConfig::default();
        config.runtime.log_level = options.log_level;
        config.runtime.language = options.language;
        config.runtime.restore_policy = options.restore_policy;
        config.runtime.data_dir = options.data_dir;
        config.runtime.llm_config_path =
            default_llm_config_path_from_data_dir(config.runtime.data_dir.as_deref());
        ai_assistant::prompt_assets::normalize_language(&config.runtime.language)
            .map_err(RuntimeError::InvalidConfig)?;
        Ok(config)
    }

    #[allow(dead_code)]
    pub(crate) fn normalize_and_validate(mut self) -> Result<Self, RuntimeError> {
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

    pub(crate) fn default_agent(&self) -> &AgentSection {
        self.agents
            .iter()
            .find(|agent| agent.is_default)
            .or_else(|| self.agents.first())
            .expect("runtime config is normalized with at least one agent")
    }

    pub(crate) fn agent_by_id_or_default(
        &self,
        agent_id: &str,
    ) -> Result<&AgentSection, RuntimeError> {
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
pub(crate) struct RuntimeCreateOptions {
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
    pub(crate) fn into_core(self) -> CoreRuntimeStateConfig {
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

fn default_llm_config_path_from_data_dir(data_dir: Option<&Path>) -> Option<PathBuf> {
    data_dir.map(|dir| {
        dir.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join("llm_config.json")
    })
}

pub(crate) fn resolve_config_storage_dir(
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

pub(crate) fn parse_config_input(input: &str) -> Result<RuntimeConfig, RuntimeError> {
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

pub(crate) fn config_parent_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

#[allow(dead_code)]
pub(crate) fn validate_runtime_config(config: &RuntimeConfig) -> Result<(), RuntimeError> {
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

pub(crate) fn validate_retrieval_config(
    path: &str,
    config: &RetrievalConfig,
) -> Result<(), RuntimeError> {
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
