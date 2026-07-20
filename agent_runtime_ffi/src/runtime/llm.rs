use serde::{Deserialize, Serialize};

use super::{validate_resource_id, RuntimeConfig, RuntimeError, RuntimeLlmRegistry};

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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ModelConfig {
    pub(crate) uid: u32,
    pub(crate) provider_uid: u32,
    pub(crate) model_name: String,
    #[serde(default = "default_context_window")]
    pub(crate) context_window: u32,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LlmRegistration {
    pub(crate) schema: String,
    pub(crate) id: String,
    #[serde(alias = "currentModelUid")]
    pub(crate) current_model_uid: Option<u32>,
    #[serde(rename = "nextProviderId")]
    _legacy_next_provider_id: Option<u32>,
    #[serde(rename = "nextModelUid")]
    _legacy_next_model_uid: Option<u32>,
    pub(crate) providers: Vec<LlmRegistrationProvider>,
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
pub(crate) struct LlmRegistrationProvider {
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
pub(crate) struct LlmRegistrationModel {
    uid: u32,
    #[serde(alias = "modelId")]
    model_id: String,
    #[serde(alias = "maxContextTokens")]
    max_context_tokens: Option<u32>,
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

pub(crate) fn build_llm_registry_and_config(
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

pub(crate) fn validate_current_model(
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
