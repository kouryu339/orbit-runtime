use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ConversationSpawnRequest {
    pub(super) schema: String,
    #[serde(alias = "clusterId")]
    pub(super) cluster_id: String,
    #[serde(
        default,
        alias = "hostContext",
        alias = "toolHostContext",
        alias = "tool_host_context"
    )]
    pub(super) tool_host_context: Option<Value>,
    #[serde(default, alias = "toolPermissions", alias = "tool_permissions")]
    pub(super) permissions: Option<ConversationToolPermissionOverride>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ConversationToolPermissionOverride {
    pub(super) read_only: Option<ai_assistant::ToolPermissionMode>,
    pub(super) controlled_change: Option<ai_assistant::ToolPermissionMode>,
    pub(super) destructive: Option<ai_assistant::ToolPermissionMode>,
}

impl ConversationToolPermissionOverride {
    pub(super) fn apply_to(
        &self,
        base: &ai_assistant::ToolPermissionPolicy,
    ) -> ai_assistant::ToolPermissionPolicy {
        ai_assistant::ToolPermissionPolicy {
            read_only: self.read_only.unwrap_or(base.read_only),
            controlled_change: self.controlled_change.unwrap_or(base.controlled_change),
            destructive: self.destructive.unwrap_or(base.destructive),
        }
    }
}

impl Default for ConversationSpawnRequest {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-conversation-spawn/v1".to_string(),
            cluster_id: String::new(),
            tool_host_context: None,
            permissions: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ConversationInitPlan {
    pub(super) model_uid: Option<u32>,
    pub(super) max_thinking_rounds: Option<u32>,
    pub(super) agent_profiles: BTreeMap<String, ResourceAgentProfileConfig>,
    pub(super) tool_host_context: Option<Value>,
    pub(super) lifecycle_cluster_id: Option<String>,
    pub(super) lifecycle_cluster_description: Option<String>,
    pub(super) additional_agents: Vec<ConversationAgentInit>,
    pub(super) tool_permissions: Option<ai_assistant::ToolPermissionPolicy>,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationAgentInit {
    pub(super) config: AIAssistantConfig,
    pub(super) skills: Vec<String>,
    pub(super) max_thinking_rounds: u32,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationInstanceMetadata {
    pub(super) cluster_id: String,
    pub(super) cluster_description: String,
    pub(super) log_path: Option<PathBuf>,
    pub(super) log_policy: ConversationLogPolicy,
}

pub(super) struct ConversationOwnerLease {
    stop_renewer: watch::Sender<bool>,
    renewer: tokio::task::JoinHandle<()>,
    key: String,
    owner: String,
}

impl ConversationOwnerLease {
    pub(super) async fn stop_and_release(
        self,
        backend: Arc<dyn RuntimeCoordinationBackend>,
    ) -> Result<(), RuntimeError> {
        let _ = self.stop_renewer.send(true);
        let _ = self.renewer.await;
        backend.release_lease(&self.key, &self.owner).await
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct ConversationOptionsInput {
    pub(super) conversation_id: Option<String>,
    #[serde(alias = "external_id")]
    pub(super) _external_id: Option<String>,
    pub(super) tenant_id: Option<String>,
    pub(super) user_id: Option<String>,
    #[serde(
        alias = "llm_headers",
        alias = "request_headers",
        alias = "llmRequestHeaders",
        alias = "requestHeaders",
        default
    )]
    pub(super) llm_request_headers: Option<BTreeMap<String, String>>,
    #[serde(
        alias = "allow_insecure_llm_headers",
        alias = "allowInsecureLlmHeaders",
        alias = "allowInsecureLlmRequestHeaders",
        default
    )]
    pub(super) allow_insecure_llm_request_headers: bool,
}

pub(super) fn parse_conversation_spawn_request(
    input: &str,
) -> Result<ConversationSpawnRequest, RuntimeError> {
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

pub(super) async fn apply_conversation_init_plan(
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

pub(super) fn parse_conversation_options(
    input: &str,
) -> Result<ConversationOptionsInput, RuntimeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(ConversationOptionsInput::default());
    }
    serde_json::from_str::<ConversationOptionsInput>(trimmed)
        .map_err(|e| RuntimeError::InvalidConfig(format!("parse conversation options failed: {e}")))
}

pub(super) fn parse_ai_auth_context_headers(
    input: &str,
) -> Result<BTreeMap<String, String>, RuntimeError> {
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

pub(super) fn conversation_index_key(cluster_id: &str) -> String {
    format!("runtime:{}:conversations:index", cluster_id)
}

pub(super) fn conversation_metadata_key(cluster_id: &str, conversation_id: &str) -> String {
    format!(
        "runtime:{}:conversation:{}:metadata",
        cluster_id, conversation_id
    )
}

fn conversation_catalog_lease_key(cluster_id: &str) -> String {
    format!("runtime:{}:conversations:catalog", cluster_id)
}

pub(super) fn conversation_owner_lease_key(cluster_id: &str, conversation_id: &str) -> String {
    format!(
        "runtime:{}:conversation:{}:owner",
        cluster_id, conversation_id
    )
}

pub(super) async fn load_conversation_index(
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

pub(super) async fn record_conversation_created(
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

pub(super) async fn publish_conversation_created_event(
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

pub(super) async fn publish_conversation_closed_event(
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

pub(super) async fn record_conversation_closed(
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

pub(super) async fn acquire_conversation_owner_lease(
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
