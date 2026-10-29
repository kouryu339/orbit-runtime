use serde::{Deserialize, Serialize};

use super::{RuntimeConfig, RuntimeError, RuntimeLlmRegistry};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub(crate) struct ProviderBundle {
    pub(crate) providers: Vec<ProviderConfig>,
    pub(crate) models: Vec<ModelConfig>,
    pub(crate) current_model_uid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderDefinitions {
    pub(crate) schema: &'static str,
    pub(crate) providers: Vec<ProviderDefinition>,
    pub(crate) models: Vec<ModelConfig>,
    pub(crate) current_model_uid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderDefinition {
    pub(crate) uid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    pub(crate) base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) api_paradigm: Option<llm_gateway::ApiParadigm>,
    pub(crate) prompt_cache_control: bool,
    pub(crate) strict_tool_schema: bool,
    pub(crate) api_key_set: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProviderConfig {
    pub(crate) uid: u32,
    #[serde(default)]
    pub(crate) name: Option<String>,
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) base_url: String,
    #[serde(alias = "apiParadigm", alias = "api_paradigm", default)]
    pub(crate) api_paradigm: Option<llm_gateway::ApiParadigm>,
    #[serde(alias = "promptCacheControl", default)]
    pub(crate) prompt_cache_control: bool,
    #[serde(alias = "strictToolSchema", default)]
    pub(crate) strict_tool_schema: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ModelConfig {
    pub(crate) uid: u32,
    pub(crate) provider_uid: u32,
    pub(crate) model_name: String,
    #[serde(default = "default_context_window")]
    pub(crate) context_window: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<llm_gateway::reasoning::ReasoningConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct ProviderConfigV1 {
    pub(crate) schema: String,
    pub(crate) providers: Vec<HostUserProviderConfig>,
    #[serde(alias = "currentModelUid")]
    pub(crate) current_model_uid: Option<u32>,
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

pub(crate) use llm_gateway::registration::LlmRegistration;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct HostEnabledModel {
    uid: u32,
    #[serde(alias = "modelId")]
    model_id: String,
    #[serde(alias = "maxContextTokens", default)]
    max_context_tokens: Option<u32>,
    #[serde(default)]
    reasoning: Option<llm_gateway::reasoning::ReasoningConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct HostUserProviderConfig {
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
    #[serde(alias = "strictToolSchema", default)]
    strict_tool_schema: bool,
    #[serde(alias = "enabledModels", default)]
    enabled_models: Vec<HostEnabledModel>,
}

impl From<HostEnabledModel> for llm_gateway::EnabledModel {
    fn from(model: HostEnabledModel) -> Self {
        Self {
            uid: model.uid,
            model_id: model.model_id,
            max_context_tokens: model.max_context_tokens,
            reasoning: model.reasoning,
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
            strict_tool_schema: provider.strict_tool_schema,
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

pub(crate) fn build_llm_registry_and_config(
    registration: LlmRegistration,
) -> Result<(RuntimeLlmRegistry, llm_gateway::LlmConfig), RuntimeError> {
    llm_gateway::registration::build_llm_registry_and_config(registration)
        .map_err(registration_error)
}

#[allow(dead_code)]
pub(crate) fn parse_provider_array(
    input: &str,
) -> Result<Vec<llm_gateway::UserProviderConfig>, RuntimeError> {
    if let Ok(providers) = serde_json::from_str::<Vec<HostUserProviderConfig>>(input) {
        return Ok(providers.into_iter().map(Into::into).collect());
    }
    serde_json::from_str::<Vec<llm_gateway::UserProviderConfig>>(input)
        .map_err(|e| RuntimeError::InvalidConfig(format!("parse providers_json failed: {e}")))
}

pub(crate) fn load_fixed_llm_config(
    config: &RuntimeConfig,
) -> Result<llm_gateway::LlmConfig, RuntimeError> {
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

pub(crate) fn validate_llm_config(config: &llm_gateway::LlmConfig) -> Result<(), RuntimeError> {
    llm_gateway::registration::validate_llm_config(config).map_err(registration_error)
}
pub(crate) fn validate_current_model(
    current: Option<u32>,
    config: &llm_gateway::LlmConfig,
) -> Result<(), RuntimeError> {
    llm_gateway::registration::validate_current_model(current, config).map_err(registration_error)
}
fn registration_error(error: llm_gateway::registration::RegistrationError) -> RuntimeError {
    match error {
        llm_gateway::registration::RegistrationError::InvalidConfig(message) => {
            RuntimeError::InvalidConfig(message)
        }
        llm_gateway::registration::RegistrationError::Llm(message) => RuntimeError::Llm(message),
    }
}
