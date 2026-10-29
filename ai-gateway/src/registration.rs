//! Shared in-memory LLM registration for Native and browser hosts.
use serde::Deserialize;
extern crate self as llm_gateway;

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("{0}")]
    InvalidConfig(String),
    #[error("{0}")]
    Llm(String),
}
type RuntimeError = RegistrationError;

fn validate_resource_id(label: &str, value: &str) -> Result<(), RuntimeError> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Err(RuntimeError::InvalidConfig(format!("invalid {label}")));
    }
    Ok(())
}
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RuntimeLlmRegistry {
    pub id: String,
    pub current_model_uid: Option<u32>,
    pub provider_count: usize,
    pub model_count: usize,
    pub model_uids: Vec<u32>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmRegistration {
    pub schema: String,
    pub id: String,
    #[serde(alias = "currentModelUid")]
    pub current_model_uid: Option<u32>,
    #[serde(rename = "nextProviderId")]
    _legacy_next_provider_id: Option<u32>,
    #[serde(rename = "nextModelUid")]
    _legacy_next_model_uid: Option<u32>,
    pub providers: Vec<LlmRegistrationProvider>,
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
pub struct LlmRegistrationProvider {
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
    #[serde(alias = "strictToolSchema")]
    strict_tool_schema: bool,
    #[serde(alias = "enabledModels")]
    enabled_models: Vec<LlmRegistrationModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "snake_case", deny_unknown_fields)]
#[derive(Default)]
pub struct LlmRegistrationModel {
    uid: u32,
    #[serde(alias = "modelId")]
    model_id: String,
    #[serde(alias = "maxContextTokens")]
    max_context_tokens: Option<u32>,
    #[serde(default)]
    reasoning: Option<llm_gateway::reasoning::ReasoningConfig>,
}

pub fn build_llm_registry_and_config(
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
    let model_uids = config
        .providers
        .iter()
        .flat_map(|provider| provider.enabled_models.iter().map(|model| model.uid))
        .collect();
    let registry = RuntimeLlmRegistry {
        id: registration.id,
        current_model_uid,
        provider_count,
        model_count,
        model_uids,
    };
    Ok((registry, config))
}

fn llm_registration_provider_to_runtime(
    provider: LlmRegistrationProvider,
) -> Result<llm_gateway::UserProviderConfig, RuntimeError> {
    let api_key = match (provider.api_key, provider.api_key_env) {
        (Some(api_key), _) => api_key,
        (None, Some(env_name)) => {
            #[cfg(target_arch = "wasm32")]
            return Err(RuntimeError::InvalidConfig(format!(
                "api_key_env is unavailable in a browser: {env_name}"
            )));
            #[cfg(not(target_arch = "wasm32"))]
            {
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
        strict_tool_schema: provider.strict_tool_schema,
        enabled_models: provider
            .enabled_models
            .into_iter()
            .map(|model| llm_gateway::EnabledModel {
                uid: model.uid,
                model_id: model.model_id,
                max_context_tokens: model.max_context_tokens,
                reasoning: model.reasoning,
            })
            .collect(),
    })
}

pub fn validate_llm_config(config: &llm_gateway::LlmConfig) -> Result<(), RuntimeError> {
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
            if let Some(reasoning) = &model.reasoning {
                let paradigm =
                    provider
                        .api_paradigm
                        .unwrap_or_else(|| {
                            match llm_gateway::providers::resolve(&model.model_id).format {
                                llm_gateway::providers::ApiFormat::OpenAI => {
                                    llm_gateway::ApiParadigm::OpenAiChatCompletions
                                }
                                llm_gateway::providers::ApiFormat::OpenAIResponses => {
                                    llm_gateway::ApiParadigm::OpenAiResponses
                                }
                                llm_gateway::providers::ApiFormat::Anthropic => {
                                    llm_gateway::ApiParadigm::AnthropicMessages
                                }
                            }
                        });
                reasoning.validate(paradigm).map_err(|error| {
                    RuntimeError::InvalidConfig(format!("model {} reasoning: {error}", model.uid))
                })?;
            }
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

pub fn validate_current_model(
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
