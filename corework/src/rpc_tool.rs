//! Runtime RPC tool support.
//!
//! This module intentionally separates the tool registry/executor model from
//! the transport. The first transport implemented here is a tiny JSON-lines
//! protocol used for local validation; a tonic/gRPC client can implement the
//! same `RpcToolClient` trait later.

use crate::ai_system::{AIOutput, SimpleArgs};
use crate::error::{FrameworkError, Result};
use crate::orchestration::Context;
use crate::rpc_proto::v1::{
    agent_tool_service_client::AgentToolServiceClient, tool_stream_message,
    AiOutput as ProtoAIOutput, ExecuteRequest, HostResult as ProtoHostResult, ListToolsRequest,
    ListToolsResponse, ToolDescriptor, ToolErrorCode, ToolStreamMessage,
};
use crate::workflow::dynamic_node::DynamicExecute;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub const RPC_TOOL_SCHEMA_V1: &str = "corework-agent-tool/v1";
pub const CAPABILITY_WORKSPACE_RESOLVE_PATH: &str = "workspace.resolve_path";
pub const CAPABILITY_WORKSPACE_RESOLVE_WORKING_PATH: &str = "workspace.resolve_working_path";
pub const CAPABILITY_WORKSPACE_CREATE_PATH: &str = "workspace.create_path";
pub const CAPABILITY_WORKSPACE_CREATE_WORKING_PATH: &str = "workspace.create_working_path";
pub const CAPABILITY_WORKSPACE_SAVE_AS_EDITED: &str = "workspace.save_as_edited";
pub const RPC_TOOL_SERVICE_V1: &str = "corework.agent_tool.v1.AgentToolService";
pub const RPC_TOOL_EXECUTE_METHOD: &str = "Execute";
const TOOL_HOST_CONTEXT_KEY: &str = "tool_host_context";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAIParameter {
    pub name: String,
    pub param_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAIOutputField {
    pub name: String,
    pub field_type: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeToolMetadata {
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_rpc_tool_kind")]
    pub tool_kind: String,
    #[serde(default)]
    pub parameters: Vec<RuntimeAIParameter>,
    #[serde(default)]
    pub outputs: Vec<RuntimeAIOutputField>,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub open_world: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub endpoint_id: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub method: String,
}

impl RuntimeToolMetadata {
    pub fn display_name_or_name(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.name
        } else {
            &self.display_name
        }
    }
}

fn default_rpc_tool_kind() -> String {
    "rpc".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEndpointInfo {
    pub endpoint_id: String,
    pub address: String,
    pub timeout_ms: u64,
}

#[derive(Default)]
pub struct RpcEndpointRegistry {
    endpoints: RwLock<HashMap<String, RpcEndpointInfo>>,
}

impl RpcEndpointRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, endpoint: RpcEndpointInfo) -> Result<()> {
        let mut endpoints = self.endpoints.write();
        if endpoints.contains_key(&endpoint.endpoint_id) {
            return Err(FrameworkError::InvalidOperation(format!(
                "RPC endpoint '{}' already registered",
                endpoint.endpoint_id
            )));
        }
        endpoints.insert(endpoint.endpoint_id.clone(), endpoint);
        Ok(())
    }

    pub fn get(&self, endpoint_id: &str) -> Option<RpcEndpointInfo> {
        self.endpoints.read().get(endpoint_id).cloned()
    }
}

#[derive(Default)]
pub struct RuntimeToolRegistry {
    tools: RwLock<HashMap<String, RuntimeToolMetadata>>,
}

impl RuntimeToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, tool: RuntimeToolMetadata) -> Result<()> {
        validate_runtime_tool_metadata(&tool)?;
        let mut tools = self.tools.write();
        if tools.contains_key(&tool.name) {
            return Err(FrameworkError::InvalidOperation(format!(
                "Runtime tool '{}' already registered",
                tool.name
            )));
        }
        tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<RuntimeToolMetadata> {
        self.tools.read().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.read().keys().cloned().collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolRequest {
    pub tool_name: String,
    pub args_cli: String,
    pub args_json: Value,
    pub call_id: String,
    #[serde(default)]
    pub tool_call_id: String,
    #[serde(default)]
    pub idempotency_key: String,
    pub session_id: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub cluster_id: String,
    #[serde(default)]
    pub runtime_instance_id: String,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub host_context: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAIOutput {
    #[serde(default)]
    pub result: Value,
    pub to_ai: String,
    pub error_code: i32,
}

impl RemoteAIOutput {
    pub fn into_checked_ai_output(self, tool_name: &str) -> Result<AIOutput> {
        let output = normalize_remote_ai_output(tool_name, self);
        validate_remote_ai_output(tool_name, &output)?;
        let code = ToolErrorCode::try_from(output.error_code).map_err(|_| {
            FrameworkError::InvalidData(format!(
                "Remote tool '{}' returned unknown ToolErrorCode {}",
                tool_name, output.error_code
            ))
        })?;
        if code == ToolErrorCode::Unspecified {
            return Err(FrameworkError::InvalidData(format!(
                "Remote tool '{}' returned TOOL_ERROR_CODE_UNSPECIFIED",
                tool_name
            )));
        }
        let local_error_code = if code == ToolErrorCode::Ok {
            0
        } else {
            output.error_code
        };

        Ok(AIOutput {
            result: output.result,
            to_ai: output.to_ai,
            error_code: local_error_code,
        })
    }
}

fn normalize_remote_ai_output(tool_name: &str, mut output: RemoteAIOutput) -> RemoteAIOutput {
    let remote_to_ai_is_empty = output.to_ai.trim().is_empty();
    output.to_ai =
        synthesize_rpc_to_ai(tool_name, output.error_code, &output.to_ai, &output.result);
    if remote_to_ai_is_empty
        && (output.error_code == ToolErrorCode::Ok as i32
            || output.error_code == ToolErrorCode::Unspecified as i32)
    {
        output.error_code = ToolErrorCode::InvalidOutput as i32;
    }
    output
}

fn synthesize_rpc_to_ai(
    tool_name: &str,
    error_code: i32,
    remote_to_ai: &str,
    result: &Value,
) -> String {
    let detail = rpc_to_ai_detail(remote_to_ai);
    let result_summary = rpc_result_summary(result);
    let context = join_rpc_context(detail.as_deref(), result_summary.as_deref());

    if error_code == ToolErrorCode::Ok as i32 {
        if remote_to_ai.trim().is_empty() {
            if let Some(result_summary) = result_summary {
                return format!(
                    "RPC tool '{}' reported success, but did not provide a to_ai summary. {}",
                    tool_name, result_summary
                );
            }
            return format!(
                "RPC tool '{}' reported success, but did not provide a result summary or structured result.",
                tool_name
            );
        }
        if let Some(context) = context {
            return format!(
                "RPC tool '{}' completed successfully. {}",
                tool_name, context
            );
        }
        return format!(
            "RPC tool '{}' reported success, but did not provide a result summary or structured result.",
            tool_name
        );
    }

    if error_code == ToolErrorCode::MissingArgument as i32 {
        return format_rpc_failure(tool_name, error_code, "missing required argument", context);
    }
    if error_code == ToolErrorCode::InvalidArgument as i32 {
        return format_rpc_failure(tool_name, error_code, "invalid argument", context);
    }
    if error_code == ToolErrorCode::Unspecified as i32 {
        return format!(
            "RPC tool '{}' returned an unspecified error code. {}",
            tool_name,
            context.unwrap_or_else(|| "No explanation was provided.".to_string())
        );
    }

    format_rpc_failure(tool_name, error_code, "execution failed", context)
}

fn format_rpc_failure(
    tool_name: &str,
    error_code: i32,
    label: &str,
    context: Option<String>,
) -> String {
    match context {
        Some(context) => format!(
            "RPC tool '{}' {} (error_code={}): {}",
            tool_name, label, error_code, context
        ),
        None => format!(
            "RPC tool '{}' {} (error_code={}), but did not provide an explanation.",
            tool_name, label, error_code
        ),
    }
}

fn rpc_to_ai_detail(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(format!("Remote summary: {}", summarize_rpc_value(&value)));
    }
    if let Some((prefix, value)) = extract_embedded_json_value(trimmed) {
        let prefix = strip_json_intro_line(prefix);
        let summary = summarize_rpc_value(&value);
        if prefix.is_empty() {
            return Some(format!("Remote summary: {}", summary));
        }
        return Some(format!(
            "Remote message: {}; structured summary: {}",
            truncate_chars(&prefix, 600),
            summary
        ));
    }
    Some(format!("Remote message: {}", truncate_chars(trimmed, 600)))
}

fn rpc_result_summary(result: &Value) -> Option<String> {
    if result.is_null() {
        None
    } else {
        Some(format!("Result summary: {}", summarize_rpc_value(result)))
    }
}

fn join_rpc_context(detail: Option<&str>, result_summary: Option<&str>) -> Option<String> {
    match (detail, result_summary) {
        (Some(detail), Some(result_summary)) => Some(format!("{detail} {result_summary}")),
        (Some(detail), None) => Some(detail.to_string()),
        (None, Some(result_summary)) => Some(result_summary.to_string()),
        (None, None) => None,
    }
}

fn extract_embedded_json_value(text: &str) -> Option<(&str, Value)> {
    for (idx, ch) in text.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }
        let candidate = &text[idx..];
        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
            return Some((&text[..idx], value));
        }
    }
    None
}

fn strip_json_intro_line(text: &str) -> String {
    let mut lines = text.lines().collect::<Vec<_>>();
    if lines
        .last()
        .is_some_and(|line| line.to_ascii_lowercase().contains("json"))
    {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

fn summarize_rpc_value(value: &Value) -> String {
    const MAX_ITEMS: usize = 12;
    const MAX_TEXT_CHARS: usize = 600;

    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => truncate_chars(v.trim(), MAX_TEXT_CHARS),
        Value::Array(values) => {
            if values.is_empty() {
                return "empty list".to_string();
            }
            let shown = values
                .iter()
                .take(MAX_ITEMS)
                .enumerate()
                .map(|(idx, value)| format!("{}: {}", idx + 1, summarize_rpc_value(value)))
                .collect::<Vec<_>>()
                .join("; ");
            if values.len() > MAX_ITEMS {
                format!("{shown}; ... {} more item(s)", values.len() - MAX_ITEMS)
            } else {
                shown
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                return "empty object".to_string();
            }
            if let Some(entries) = map.get("entries").and_then(Value::as_array) {
                return summarize_rpc_entries_object(map, entries);
            }
            let shown = map
                .iter()
                .take(MAX_ITEMS)
                .map(|(key, value)| format!("{}: {}", key, summarize_rpc_value(value)))
                .collect::<Vec<_>>()
                .join("; ");
            if map.len() > MAX_ITEMS {
                format!("{shown}; ... {} more field(s)", map.len() - MAX_ITEMS)
            } else {
                shown
            }
        }
    }
}

fn summarize_rpc_entries_object(map: &serde_json::Map<String, Value>, entries: &[Value]) -> String {
    const MAX_ENTRIES: usize = 12;

    let mut parts = Vec::new();
    for key in ["path", "directory", "base", "root", "total", "count"] {
        if let Some(value) = map.get(key) {
            parts.push(format!("{}: {}", key, summarize_rpc_value(value)));
        }
    }
    parts.push(format!("entries: {} item(s)", entries.len()));
    if !entries.is_empty() {
        let shown = entries
            .iter()
            .take(MAX_ENTRIES)
            .enumerate()
            .map(|(idx, value)| format!("{}: {}", idx + 1, summarize_rpc_value(value)))
            .collect::<Vec<_>>()
            .join("; ");
        if entries.len() > MAX_ENTRIES {
            parts.push(format!(
                "shown entries: {}; ... {} more item(s)",
                shown,
                entries.len() - MAX_ENTRIES
            ));
        } else {
            parts.push(format!("shown entries: {}", shown));
        }
    }
    parts.join("; ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn rpc_protocol_ai_output(tool_name: &str, message: impl Into<String>) -> AIOutput {
    let message = message.into();
    AIOutput::error(
        ToolErrorCode::ProtocolError as i32,
        format!("RPC tool '{}' protocol error: {}", tool_name, message),
    )
}

pub fn validate_rpc_endpoint_info(endpoint: &RpcEndpointInfo) -> Result<()> {
    if endpoint.endpoint_id.trim().is_empty() {
        return Err(FrameworkError::InvalidData(
            "RPC endpoint_id must not be empty".to_string(),
        ));
    }
    if endpoint.address.trim().is_empty() {
        return Err(FrameworkError::InvalidData(format!(
            "RPC endpoint '{}' address must not be empty",
            endpoint.endpoint_id
        )));
    }
    Ok(())
}

pub fn validate_runtime_tool_metadata(tool: &RuntimeToolMetadata) -> Result<()> {
    if tool.name.trim().is_empty() {
        return Err(FrameworkError::InvalidData(
            "Runtime tool name must not be empty".to_string(),
        ));
    }

    validate_unique_named_items(
        &tool.name,
        "parameter",
        tool.parameters.iter().map(|p| p.name.as_str()),
    )?;
    validate_unique_named_items(
        &tool.name,
        "output",
        tool.outputs.iter().map(|o| o.name.as_str()),
    )?;

    for capability in &tool.required_capabilities {
        if is_workspace_capability(capability) {
            continue;
        }
        return Err(FrameworkError::InvalidData(format!(
            "Runtime tool '{}' declares unsupported capability '{}'",
            tool.name, capability
        )));
    }

    Ok(())
}

fn ensure_tool_declares_host_capability(
    tool: &RuntimeToolMetadata,
    op: &str,
    _key: Option<&str>,
) -> Result<()> {
    if is_workspace_capability(op) {
        if tool
            .required_capabilities
            .iter()
            .any(|capability| capability == op)
        {
            return Ok(());
        }
        return Err(FrameworkError::InvalidOperation(format!(
            "Runtime tool '{}' is not allowed to call host capability '{}'",
            tool.name, op
        )));
    }

    Err(FrameworkError::InvalidOperation(format!(
        "Unsupported host capability '{}'",
        op
    )))
}

fn is_supported_host_capability(op: &str) -> bool {
    is_workspace_capability(op)
}

fn is_workspace_capability(capability: &str) -> bool {
    matches!(
        capability,
        CAPABILITY_WORKSPACE_RESOLVE_PATH
            | CAPABILITY_WORKSPACE_RESOLVE_WORKING_PATH
            | CAPABILITY_WORKSPACE_CREATE_PATH
            | CAPABILITY_WORKSPACE_CREATE_WORKING_PATH
            | CAPABILITY_WORKSPACE_SAVE_AS_EDITED
    )
}

pub fn validate_runtime_tool_for_endpoint(
    endpoint: &RpcEndpointInfo,
    tool: &RuntimeToolMetadata,
) -> Result<()> {
    validate_rpc_endpoint_info(endpoint)?;
    validate_runtime_tool_metadata(tool)?;

    if tool.endpoint_id.trim().is_empty() {
        return Err(FrameworkError::InvalidData(format!(
            "Runtime tool '{}' endpoint_id must not be empty",
            tool.name
        )));
    }
    if tool.endpoint_id != endpoint.endpoint_id {
        return Err(FrameworkError::InvalidData(format!(
            "Runtime tool '{}' endpoint_id '{}' does not match endpoint '{}'",
            tool.name, tool.endpoint_id, endpoint.endpoint_id
        )));
    }

    Ok(())
}

pub fn validate_runtime_tool_set_for_endpoint(
    endpoint: &RpcEndpointInfo,
    tools: &[RuntimeToolMetadata],
) -> Result<()> {
    validate_rpc_endpoint_info(endpoint)?;
    let mut names = HashSet::new();
    for tool in tools {
        validate_runtime_tool_for_endpoint(endpoint, tool)?;
        if !names.insert(tool.name.clone()) {
            return Err(FrameworkError::InvalidData(format!(
                "Runtime tool '{}' is duplicated in endpoint '{}'",
                tool.name, endpoint.endpoint_id
            )));
        }
    }
    Ok(())
}

pub fn validate_remote_ai_output(tool_name: &str, output: &RemoteAIOutput) -> Result<()> {
    if output.to_ai.trim().is_empty() {
        return Err(FrameworkError::InvalidData(format!(
            "Remote tool '{}' returned invalid AIOutput: to_ai is empty",
            tool_name
        )));
    }
    Ok(())
}

pub fn runtime_tool_metadata_from_descriptor(
    endpoint_id: &str,
    descriptor: ToolDescriptor,
) -> RuntimeToolMetadata {
    let display_name = if descriptor.display_name.trim().is_empty() {
        descriptor.name.clone()
    } else {
        descriptor.display_name
    };
    RuntimeToolMetadata {
        name: descriptor.name,
        display_name,
        description: descriptor.description,
        tool_kind: "rpc".to_string(),
        parameters: descriptor
            .parameters
            .into_iter()
            .map(|p| RuntimeAIParameter {
                name: p.name,
                param_type: p.param_type,
                required: p.required,
                default_value: p.default_value,
                description: p.description,
            })
            .collect(),
        outputs: descriptor
            .outputs
            .into_iter()
            .map(|o| RuntimeAIOutputField {
                name: o.name,
                field_type: o.field_type,
                description: o.description,
            })
            .collect(),
        destructive: descriptor.destructive,
        readonly: descriptor.readonly,
        idempotent: descriptor.idempotent,
        open_world: descriptor.open_world,
        secret: descriptor.secret,
        required_capabilities: descriptor.required_capabilities,
        endpoint_id: endpoint_id.to_string(),
        service: RPC_TOOL_SERVICE_V1.to_string(),
        method: RPC_TOOL_EXECUTE_METHOD.to_string(),
    }
}

pub fn runtime_tools_from_list_tools_response(
    endpoint: &RpcEndpointInfo,
    response: ListToolsResponse,
) -> Result<Vec<RuntimeToolMetadata>> {
    validate_rpc_endpoint_info(endpoint)?;
    if response.schema != RPC_TOOL_SCHEMA_V1 {
        return Err(FrameworkError::InvalidData(format!(
            "RPC endpoint '{}' returned unsupported tool schema '{}'",
            endpoint.endpoint_id, response.schema
        )));
    }

    let tools: Vec<RuntimeToolMetadata> = response
        .tools
        .into_iter()
        .map(|descriptor| runtime_tool_metadata_from_descriptor(&endpoint.endpoint_id, descriptor))
        .collect();
    validate_runtime_tool_set_for_endpoint(endpoint, &tools)?;
    Ok(tools)
}

#[derive(Debug, Clone)]
pub struct GrpcRpcToolDiscoveryClient {
    pub runtime_version: String,
}

impl Default for GrpcRpcToolDiscoveryClient {
    fn default() -> Self {
        Self {
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl GrpcRpcToolDiscoveryClient {
    pub fn new(runtime_version: impl Into<String>) -> Self {
        Self {
            runtime_version: runtime_version.into(),
        }
    }

    pub async fn list_tools(&self, endpoint: RpcEndpointInfo) -> Result<Vec<RuntimeToolMetadata>> {
        validate_rpc_endpoint_info(&endpoint)?;
        let timeout = Duration::from_millis(endpoint.timeout_ms.max(1));
        let fut = async {
            let address = grpc_endpoint_uri(&endpoint.address);
            let mut client = AgentToolServiceClient::connect(address.clone())
                .await
                .map_err(|e| {
                    FrameworkError::SystemError(format!(
                        "Failed to connect gRPC RPC endpoint '{}': {}",
                        address, e
                    ))
                })?;

            let response = client
                .list_tools(ListToolsRequest {
                    runtime_version: self.runtime_version.clone(),
                    accepted_schema: vec![RPC_TOOL_SCHEMA_V1.to_string()],
                })
                .await
                .map_err(|e| {
                    FrameworkError::SystemError(format!(
                        "Failed to list tools from RPC endpoint '{}': {}",
                        endpoint.endpoint_id, e
                    ))
                })?
                .into_inner();

            runtime_tools_from_list_tools_response(&endpoint, response)
        };

        tokio::time::timeout(timeout, fut).await.map_err(|_| {
            FrameworkError::SystemError(format!(
                "RPC endpoint '{}' ListTools timed out after {}ms",
                endpoint.endpoint_id, endpoint.timeout_ms
            ))
        })?
    }
}

fn grpc_endpoint_uri(address: &str) -> String {
    if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{}", address)
    }
}

fn validate_unique_named_items<'a>(
    tool_name: &str,
    item_kind: &str,
    names: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let mut seen = HashSet::new();
    for name in names {
        if name.trim().is_empty() {
            return Err(FrameworkError::InvalidData(format!(
                "Runtime tool '{}' has an empty {} name",
                tool_name, item_kind
            )));
        }
        if !seen.insert(name.to_string()) {
            return Err(FrameworkError::InvalidData(format!(
                "Runtime tool '{}' has duplicate {} '{}'",
                tool_name, item_kind, name
            )));
        }
    }
    Ok(())
}

#[async_trait]
pub trait RpcToolClient: Send + Sync {
    async fn execute(
        &self,
        endpoint: RpcEndpointInfo,
        metadata: RuntimeToolMetadata,
        request: AgentToolRequest,
        ctx: &Context,
    ) -> Result<AIOutput>;
}

pub struct RpcStubSystem {
    pub tool_name: String,
    pub endpoints: Arc<RpcEndpointRegistry>,
    pub tools: Arc<RuntimeToolRegistry>,
    pub client: Arc<dyn RpcToolClient>,
}

impl RpcStubSystem {
    pub fn new(
        tool_name: impl Into<String>,
        endpoints: Arc<RpcEndpointRegistry>,
        tools: Arc<RuntimeToolRegistry>,
        client: Arc<dyn RpcToolClient>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            endpoints,
            tools,
            client,
        }
    }
}

#[async_trait]
impl DynamicExecute for RpcStubSystem {
    async fn execute_dynamic(
        &self,
        mut input: HashMap<String, Value>,
        ctx: &Context,
    ) -> Result<Value> {
        let metadata = self.tools.get(&self.tool_name).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "Runtime tool '{}' is not registered",
                self.tool_name
            ))
        })?;
        let endpoint = self.endpoints.get(&metadata.endpoint_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "RPC endpoint '{}' for tool '{}' is not registered",
                metadata.endpoint_id, self.tool_name
            ))
        })?;

        let tool_call_id = input
            .remove("__tool_call_id")
            .and_then(|value| value.as_str().map(ToOwned::to_owned));
        let args_cli = input
            .get("command")
            .or_else(|| input.get("input"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let args_json = build_rpc_args_json(&metadata, input, &args_cli)?;
        let call_id = uuid::Uuid::new_v4().to_string();
        let tool_call_id = tool_call_id.unwrap_or_else(|| call_id.clone());
        let conversation_id = resolve_rpc_tool_conversation_id(ctx).await?;
        let agent_id = resolve_rpc_tool_agent_id(ctx).await?;
        let host_context = resolve_rpc_tool_host_context(ctx).await?;
        let request = AgentToolRequest {
            tool_name: self.tool_name.clone(),
            args_cli,
            args_json,
            call_id: call_id.clone(),
            tool_call_id: tool_call_id.clone(),
            idempotency_key: tool_call_id,
            session_id: ctx.request_id.clone(),
            provider_id: endpoint.endpoint_id.clone(),
            cluster_id: String::new(),
            runtime_instance_id: String::new(),
            conversation_id,
            agent_id,
            turn_id: String::new(),
            permissions: metadata.required_capabilities.clone(),
            host_context,
        };

        let output = self
            .client
            .execute(endpoint, metadata, request, ctx)
            .await
            .map_err(|e| FrameworkError::SystemError(format!("RPC tool failed: {}", e)))?;

        serde_json::to_value(output).map_err(FrameworkError::SerializationError)
    }
}

async fn resolve_rpc_tool_host_context(ctx: &Context) -> Result<Value> {
    for key in [
        TOOL_HOST_CONTEXT_KEY,
        "host_context",
        "runtime:tool_host_context",
        "runtime:host_context",
    ] {
        if let Some(value) = ctx.cache.get_raw(key).await? {
            if value.is_null() {
                continue;
            }
            return Ok(value);
        }
    }
    Ok(Value::Object(serde_json::Map::new()))
}

async fn resolve_rpc_tool_conversation_id(ctx: &Context) -> Result<String> {
    for key in [
        "conversation_id",
        "parent_conversation_id",
        "runtime:conversation_id",
        "scope:conversation_id",
        "conversation:id",
    ] {
        if let Some(value) = ctx.cache.get_raw(key).await? {
            if let Some(conversation_id) = value.as_str().map(str::trim) {
                if !conversation_id.is_empty() {
                    return Ok(conversation_id.to_string());
                }
            }
            if let Some(conversation_id) = value
                .as_object()
                .and_then(|object| object.get("conversation_id").or_else(|| object.get("id")))
                .and_then(|value| value.as_str())
                .map(str::trim)
            {
                if !conversation_id.is_empty() {
                    return Ok(conversation_id.to_string());
                }
            }
        }
    }

    Ok(String::new())
}

async fn resolve_rpc_tool_agent_id(ctx: &Context) -> Result<String> {
    for key in ["agent_id", "runtime:agent_id", "scope:agent_id", "agent:id"] {
        if let Some(value) = ctx.cache.get_raw(key).await? {
            if let Some(agent_id) = value.as_str().map(str::trim) {
                if !agent_id.is_empty() {
                    return Ok(agent_id.to_string());
                }
            }
            if let Some(agent_id) = value
                .as_object()
                .and_then(|object| object.get("agent_id").or_else(|| object.get("id")))
                .and_then(|value| value.as_str())
                .map(str::trim)
            {
                if !agent_id.is_empty() {
                    return Ok(agent_id.to_string());
                }
            }
        }
    }

    Ok(String::new())
}

#[derive(Default)]
pub struct JsonLineRpcToolClient;

#[derive(Default)]
pub struct JsonLineRpcToolDiscoveryClient;

fn build_rpc_args_json(
    metadata: &RuntimeToolMetadata,
    input: HashMap<String, Value>,
    args_cli: &str,
) -> Result<Value> {
    let mut args_json = serde_json::Map::new();

    if !args_cli.trim().is_empty() {
        let parsed = SimpleArgs::parse(args_cli)?;
        for parameter in &metadata.parameters {
            let raw = parsed
                .get(&parameter.name)
                .map(str::to_string)
                .or_else(|| parameter.default_value.clone());
            if let Some(raw) = raw {
                args_json.insert(
                    parameter.name.clone(),
                    coerce_rpc_arg_value(parameter, &raw)?,
                );
            }
        }
    }

    for (key, value) in input {
        if key == "input" || key == "command" {
            continue;
        }
        if let Some(parameter) = metadata.parameters.iter().find(|p| p.name == key) {
            let value = match value {
                Value::String(raw) => coerce_rpc_arg_value(parameter, &raw)?,
                other => other,
            };
            args_json.insert(key, value);
        } else {
            args_json.insert(key, value);
        }
    }

    Ok(Value::Object(args_json))
}

fn coerce_rpc_arg_value(parameter: &RuntimeAIParameter, raw: &str) -> Result<Value> {
    let ty = parameter.param_type.trim().to_ascii_lowercase();
    match ty.as_str() {
        "number" | "integer" | "int" | "int64" | "long" => {
            if let Ok(value) = raw.parse::<i64>() {
                Ok(Value::Number(value.into()))
            } else if let Ok(value) = raw.parse::<f64>() {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| invalid_rpc_arg(parameter, raw))
            } else {
                Err(invalid_rpc_arg(parameter, raw))
            }
        }
        "boolean" | "bool" => raw.parse::<bool>().map(Value::Bool).or_else(|_| {
            match raw.trim().to_ascii_lowercase().as_str() {
                "1" | "yes" | "y" => Ok(Value::Bool(true)),
                "0" | "no" | "n" => Ok(Value::Bool(false)),
                _ => Err(invalid_rpc_arg(parameter, raw)),
            }
        }),
        "stringarray" | "string[]" | "array<string>" => {
            if raw.trim().is_empty() {
                return Ok(Value::Array(Vec::new()));
            }
            if let Ok(Value::Array(values)) = serde_json::from_str::<Value>(raw) {
                return Ok(Value::Array(values));
            }
            Ok(Value::Array(
                raw.split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| Value::String(value.to_string()))
                    .collect(),
            ))
        }
        _ => Ok(Value::String(raw.to_string())),
    }
}

fn invalid_rpc_arg(parameter: &RuntimeAIParameter, raw: &str) -> FrameworkError {
    FrameworkError::InvalidData(format!(
        "Invalid value '{}' for RPC tool parameter '{}' of type '{}'",
        raw, parameter.name, parameter.param_type
    ))
}

#[derive(Default)]
pub struct GrpcRpcToolClient;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WireMessage {
    #[serde(rename = "list_tools")]
    ListTools,
    #[serde(rename = "tools")]
    Tools {
        schema: String,
        tools: Vec<RuntimeToolMetadata>,
    },
    #[serde(rename = "execute")]
    Execute {
        request: AgentToolRequest,
        metadata: RuntimeToolMetadata,
    },
    #[serde(rename = "host_call")]
    HostCall { id: String, op: String, args: Value },
    #[serde(rename = "host_result")]
    HostResult { id: String, ok: bool, value: Value },
    #[serde(rename = "ai_output")]
    AiOutput { output: RemoteAIOutput },
    #[serde(rename = "error")]
    Error { message: String },
}

impl JsonLineRpcToolDiscoveryClient {
    pub async fn list_tools(&self, endpoint: RpcEndpointInfo) -> Result<Vec<RuntimeToolMetadata>> {
        validate_rpc_endpoint_info(&endpoint)?;
        let timeout = Duration::from_millis(endpoint.timeout_ms.max(1));
        let fut = async {
            let stream = TcpStream::connect(&endpoint.address).await.map_err(|e| {
                FrameworkError::SystemError(format!(
                    "Failed to connect RPC endpoint '{}': {}",
                    endpoint.address, e
                ))
            })?;
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            write_wire(&mut writer, &WireMessage::ListTools).await?;

            let mut line = String::new();
            let read = reader.read_line(&mut line).await.map_err(|e| {
                FrameworkError::SystemError(format!("Failed to read RPC tool list: {}", e))
            })?;
            if read == 0 {
                return Err(FrameworkError::SystemError(
                    "RPC endpoint closed connection before returning tools".to_string(),
                ));
            }

            match serde_json::from_str::<WireMessage>(line.trim_end())
                .map_err(FrameworkError::SerializationError)?
            {
                WireMessage::Tools { schema, mut tools } => {
                    if schema != RPC_TOOL_SCHEMA_V1 {
                        return Err(FrameworkError::InvalidData(format!(
                            "RPC endpoint '{}' returned unsupported tool schema '{}'",
                            endpoint.endpoint_id, schema
                        )));
                    }
                    for tool in &mut tools {
                        tool.endpoint_id = endpoint.endpoint_id.clone();
                        tool.tool_kind = "rpc".to_string();
                        tool.service = RPC_TOOL_SERVICE_V1.to_string();
                        tool.method = RPC_TOOL_EXECUTE_METHOD.to_string();
                    }
                    validate_runtime_tool_set_for_endpoint(&endpoint, &tools)?;
                    Ok(tools)
                }
                WireMessage::Error { message } => Err(FrameworkError::InvalidData(format!(
                    "RPC endpoint '{}' failed to list tools: {}",
                    endpoint.endpoint_id, message
                ))),
                _ => Err(FrameworkError::InvalidData(
                    "Unexpected RPC wire message while listing tools".to_string(),
                )),
            }
        };

        tokio::time::timeout(timeout, fut).await.map_err(|_| {
            FrameworkError::SystemError(format!(
                "RPC endpoint '{}' list_tools timed out after {}ms",
                endpoint.endpoint_id, endpoint.timeout_ms
            ))
        })?
    }
}

#[async_trait]
impl RpcToolClient for JsonLineRpcToolClient {
    async fn execute(
        &self,
        endpoint: RpcEndpointInfo,
        metadata: RuntimeToolMetadata,
        request: AgentToolRequest,
        ctx: &Context,
    ) -> Result<AIOutput> {
        let timeout = Duration::from_millis(endpoint.timeout_ms.max(1));
        let fut = async {
            let stream = TcpStream::connect(&endpoint.address).await.map_err(|e| {
                FrameworkError::SystemError(format!(
                    "Failed to connect RPC endpoint '{}': {}",
                    endpoint.address, e
                ))
            })?;
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);

            write_wire(
                &mut writer,
                &WireMessage::Execute {
                    request,
                    metadata: metadata.clone(),
                },
            )
            .await?;

            loop {
                let mut line = String::new();
                let read = reader.read_line(&mut line).await.map_err(|e| {
                    FrameworkError::SystemError(format!("Failed to read RPC response: {}", e))
                })?;
                if read == 0 {
                    return Err(FrameworkError::SystemError(
                        "RPC endpoint closed connection before AIOutput".to_string(),
                    ));
                }

                let msg: WireMessage = serde_json::from_str(line.trim_end())
                    .map_err(FrameworkError::SerializationError)?;
                match msg {
                    WireMessage::HostCall { id, op, args } => {
                        let result = handle_host_call(ctx, &endpoint, &metadata, &op, args).await;
                        let (ok, value) = match result {
                            Ok(value) => (true, value),
                            Err(e) => (false, Value::String(e.to_string())),
                        };
                        write_wire(&mut writer, &WireMessage::HostResult { id, ok, value }).await?;
                    }
                    WireMessage::AiOutput { output } => {
                        return output.into_checked_ai_output(&metadata.name);
                    }
                    WireMessage::Error { message } => {
                        return Ok(rpc_protocol_ai_output(&metadata.name, message));
                    }
                    WireMessage::ListTools
                    | WireMessage::Tools { .. }
                    | WireMessage::Execute { .. }
                    | WireMessage::HostResult { .. } => {
                        return Err(FrameworkError::InvalidData(
                            "Unexpected RPC wire message".to_string(),
                        ));
                    }
                }
            }
        };

        tokio::time::timeout(timeout, fut).await.map_err(|_| {
            FrameworkError::SystemError(format!(
                "RPC endpoint '{}' timed out after {}ms",
                endpoint.endpoint_id, endpoint.timeout_ms
            ))
        })?
    }
}

#[async_trait]
impl RpcToolClient for GrpcRpcToolClient {
    async fn execute(
        &self,
        endpoint: RpcEndpointInfo,
        metadata: RuntimeToolMetadata,
        request: AgentToolRequest,
        ctx: &Context,
    ) -> Result<AIOutput> {
        validate_runtime_tool_for_endpoint(&endpoint, &metadata)?;
        let timeout = Duration::from_millis(endpoint.timeout_ms.max(1));
        let fut = async {
            let address = grpc_endpoint_uri(&endpoint.address);
            let mut client = AgentToolServiceClient::connect(address.clone())
                .await
                .map_err(|e| {
                    FrameworkError::SystemError(format!(
                        "Failed to connect gRPC RPC endpoint '{}': {}",
                        address, e
                    ))
                })?;

            let (tx, rx) = mpsc::channel::<ToolStreamMessage>(8);
            tx.send(ToolStreamMessage {
                call_id: request.call_id.clone(),
                message: Some(tool_stream_message::Message::ExecuteRequest(
                    ExecuteRequest {
                        tool_name: request.tool_name.clone(),
                        args_cli: request.args_cli.clone(),
                        args_json: serde_json::to_string(&request.args_json)
                            .map_err(FrameworkError::SerializationError)?,
                        session_id: request.session_id.clone(),
                        request_id: ctx.request_id.clone(),
                        tool_call_id: request.tool_call_id.clone(),
                        idempotency_key: request.idempotency_key.clone(),
                        provider_id: request.provider_id.clone(),
                        cluster_id: request.cluster_id.clone(),
                        runtime_instance_id: request.runtime_instance_id.clone(),
                        conversation_id: request.conversation_id.clone(),
                        agent_id: request.agent_id.clone(),
                        turn_id: request.turn_id.clone(),
                        permissions: request.permissions.clone(),
                        host_context_json: serde_json::to_string(&request.host_context)
                            .map_err(FrameworkError::SerializationError)?,
                    },
                )),
            })
            .await
            .map_err(|e| {
                FrameworkError::SystemError(format!("Failed to send gRPC ExecuteRequest: {}", e))
            })?;

            let mut stream = client
                .execute(ReceiverStream::new(rx))
                .await
                .map_err(|e| {
                    FrameworkError::SystemError(format!(
                        "Failed to execute RPC tool '{}': {}",
                        metadata.name, e
                    ))
                })?
                .into_inner();

            while let Some(message) = stream.message().await.map_err(|e| {
                FrameworkError::SystemError(format!(
                    "Failed to read RPC stream for tool '{}': {}",
                    metadata.name, e
                ))
            })? {
                if message.call_id != request.call_id {
                    return Err(FrameworkError::InvalidData(format!(
                        "RPC tool '{}' returned stream message for call_id '{}' while '{}' was expected",
                        metadata.name, message.call_id, request.call_id
                    )));
                }

                match message.message {
                    Some(tool_stream_message::Message::HostCall(host_call)) => {
                        if host_call.id.trim().is_empty() {
                            return Err(FrameworkError::InvalidData(format!(
                                "RPC tool '{}' returned HostCall with empty id",
                                metadata.name
                            )));
                        }
                        let args: Value = serde_json::from_str(&host_call.args_json)
                            .map_err(FrameworkError::SerializationError)?;
                        let result =
                            handle_host_call(ctx, &endpoint, &metadata, &host_call.op, args).await;
                        let (ok, value_json, code) = match result {
                            Ok(value) => (
                                true,
                                serde_json::to_string(&value)
                                    .map_err(FrameworkError::SerializationError)?,
                                ToolErrorCode::Ok as i32,
                            ),
                            Err(e) => (
                                false,
                                serde_json::to_string(&e.to_string())
                                    .map_err(FrameworkError::SerializationError)?,
                                host_call_error_code(&host_call.op, &e) as i32,
                            ),
                        };
                        tx.send(ToolStreamMessage {
                            call_id: request.call_id.clone(),
                            message: Some(tool_stream_message::Message::HostResult(
                                ProtoHostResult {
                                    id: host_call.id,
                                    ok,
                                    value_json,
                                    code,
                                },
                            )),
                        })
                        .await
                        .map_err(|e| {
                            FrameworkError::SystemError(format!(
                                "Failed to send gRPC HostResult: {}",
                                e
                            ))
                        })?;
                    }
                    Some(tool_stream_message::Message::AiOutput(output)) => {
                        return proto_ai_output_into_ai_output(&metadata.name, output);
                    }
                    Some(tool_stream_message::Message::Error(error)) => {
                        return Err(FrameworkError::SystemError(format!(
                            "RPC tool '{}' returned protocol error {:?}: {}",
                            metadata.name,
                            ToolErrorCode::try_from(error.code)
                                .unwrap_or(ToolErrorCode::Unspecified),
                            error.message
                        )));
                    }
                    Some(tool_stream_message::Message::Log(log)) => {
                        tracing::info!(
                            target: "corework::rpc_tool",
                            endpoint_id = %endpoint.endpoint_id,
                            tool = %metadata.name,
                            level = %log.level,
                            "{}",
                            log.message
                        );
                    }
                    Some(
                        tool_stream_message::Message::ExecuteRequest(_)
                        | tool_stream_message::Message::HostResult(_),
                    )
                    | None => {
                        return Err(FrameworkError::InvalidData(
                            "Unexpected gRPC RPC stream message".to_string(),
                        ));
                    }
                }
            }

            Err(FrameworkError::SystemError(format!(
                "RPC endpoint closed gRPC stream before AIOutput for tool '{}'",
                metadata.name
            )))
        };

        tokio::time::timeout(timeout, fut).await.map_err(|_| {
            FrameworkError::SystemError(format!(
                "RPC endpoint '{}' timed out after {}ms",
                endpoint.endpoint_id, endpoint.timeout_ms
            ))
        })?
    }
}

fn proto_ai_output_into_ai_output(tool_name: &str, output: ProtoAIOutput) -> Result<AIOutput> {
    let result = serde_json::from_str(&output.result_json).map_err(|e| {
        FrameworkError::InvalidData(format!(
            "Remote tool '{}' returned invalid result_json: {}",
            tool_name, e
        ))
    })?;
    let normalized = normalize_remote_ai_output(
        tool_name,
        RemoteAIOutput {
            result,
            to_ai: output.to_ai,
            error_code: output.error_code,
        },
    );
    let code = ToolErrorCode::try_from(normalized.error_code).map_err(|_| {
        FrameworkError::InvalidData(format!(
            "Remote tool '{}' returned unknown ToolErrorCode {}",
            tool_name, normalized.error_code
        ))
    })?;
    if code == ToolErrorCode::Unspecified {
        return Err(FrameworkError::InvalidData(format!(
            "Remote tool '{}' returned TOOL_ERROR_CODE_UNSPECIFIED",
            tool_name
        )));
    }
    let local_error_code = if code == ToolErrorCode::Ok {
        0
    } else {
        normalized.error_code
    };

    Ok(AIOutput {
        result: normalized.result,
        to_ai: normalized.to_ai,
        error_code: local_error_code,
    })
}

fn host_call_error_code(op: &str, error: &FrameworkError) -> ToolErrorCode {
    match error {
        FrameworkError::InvalidOperation(message) if message.contains("not allowed") => {
            ToolErrorCode::HostCapabilityDenied
        }
        _ if !is_supported_host_capability(op) => ToolErrorCode::HostCapabilityUnsupported,
        _ => ToolErrorCode::HostCallFailed,
    }
}

async fn write_wire<W>(writer: &mut W, msg: &WireMessage) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut line = serde_json::to_vec(msg).map_err(FrameworkError::SerializationError)?;
    line.push(b'\n');
    writer
        .write_all(&line)
        .await
        .map_err(|e| FrameworkError::SystemError(format!("Failed to write RPC message: {}", e)))?;
    writer
        .flush()
        .await
        .map_err(|e| FrameworkError::SystemError(format!("Failed to flush RPC message: {}", e)))
}

async fn handle_host_call(
    ctx: &Context,
    _endpoint: &RpcEndpointInfo,
    metadata: &RuntimeToolMetadata,
    op: &str,
    args: Value,
) -> Result<Value> {
    match op {
        CAPABILITY_WORKSPACE_RESOLVE_PATH | CAPABILITY_WORKSPACE_RESOLVE_WORKING_PATH => {
            ensure_tool_declares_host_capability(metadata, op, None)?;
            let path = required_string(&args, "path")?;
            let (working_path, source_path) =
                crate::workspace::resolve_working_path(&path, &ctx.cache)
                    .await
                    .map_err(FrameworkError::SystemError)?;
            Ok(serde_json::json!({
                "working_path": working_path,
                "source_path": source_path
            }))
        }
        CAPABILITY_WORKSPACE_CREATE_PATH | CAPABILITY_WORKSPACE_CREATE_WORKING_PATH => {
            ensure_tool_declares_host_capability(metadata, op, None)?;
            let path = required_string(&args, "path")?;
            let (working_path, source_path) =
                crate::workspace::create_working_path(&path, &ctx.cache)
                    .await
                    .map_err(FrameworkError::SystemError)?;
            Ok(serde_json::json!({
                "working_path": working_path,
                "source_path": source_path
            }))
        }
        CAPABILITY_WORKSPACE_SAVE_AS_EDITED => {
            ensure_tool_declares_host_capability(metadata, op, None)?;
            let source_path = required_string(&args, "source_path")?;
            let suffix = required_string(&args, "suffix")?;
            let saved_path = crate::workspace::save_as_edited(&source_path, &ctx.cache, &suffix)
                .await
                .map_err(FrameworkError::SystemError)?;
            Ok(serde_json::json!({ "saved_path": saved_path }))
        }
        other => Err(FrameworkError::InvalidOperation(format!(
            "Unsupported host capability '{}'",
            other
        ))),
    }
}

fn required_string(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| {
            FrameworkError::InvalidData(format!(
                "Host capability argument '{}' must be a string",
                key
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;
    use crate::event::InMemoryEventBus;
    use crate::monitoring::NoopTelemetry;
    use crate::rpc_proto::v1::{ToolOutputField, ToolParameter};

    fn test_endpoint() -> RpcEndpointInfo {
        RpcEndpointInfo {
            endpoint_id: "test".to_string(),
            address: "127.0.0.1:58081".to_string(),
            timeout_ms: 1000,
        }
    }

    fn test_tool() -> RuntimeToolMetadata {
        RuntimeToolMetadata {
            name: "Probe".to_string(),
            display_name: "Probe".to_string(),
            description: "Probe workspace capability.".to_string(),
            tool_kind: "rpc".to_string(),
            parameters: vec![RuntimeAIParameter {
                name: "key".to_string(),
                param_type: "String".to_string(),
                required: true,
                default_value: None,
                description: "Snapshot key.".to_string(),
            }],
            outputs: vec![RuntimeAIOutputField {
                name: "value".to_string(),
                field_type: "String".to_string(),
                description: "Snapshot value.".to_string(),
            }],
            destructive: false,
            readonly: true,
            idempotent: true,
            open_world: false,
            secret: false,
            required_capabilities: vec![CAPABILITY_WORKSPACE_RESOLVE_PATH.to_string()],
            endpoint_id: "test".to_string(),
            service: "corework-agent-tool/v1".to_string(),
            method: "Execute".to_string(),
        }
    }

    #[tokio::test]
    async fn json_line_discovery_reads_sdk_registered_tools() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert!(matches!(
                serde_json::from_str::<WireMessage>(line.trim_end()).unwrap(),
                WireMessage::ListTools
            ));

            let mut tool = test_tool();
            tool.name = "FrontendNavigate".to_string();
            tool.display_name = "Navigate Frontend".to_string();
            tool.endpoint_id.clear();
            tool.service.clear();
            tool.method.clear();
            write_wire(
                &mut writer,
                &WireMessage::Tools {
                    schema: RPC_TOOL_SCHEMA_V1.to_string(),
                    tools: vec![tool],
                },
            )
            .await
            .unwrap();
        });

        let endpoint = RpcEndpointInfo {
            endpoint_id: "frontend-tools".to_string(),
            address: address.to_string(),
            timeout_ms: 1000,
        };
        let tools = JsonLineRpcToolDiscoveryClient
            .list_tools(endpoint)
            .await
            .unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "FrontendNavigate");
        assert_eq!(tools[0].endpoint_id, "frontend-tools");
        assert_eq!(tools[0].service, RPC_TOOL_SERVICE_V1);
        assert_eq!(tools[0].method, RPC_TOOL_EXECUTE_METHOD);
        server.await.unwrap();
    }

    #[test]
    fn runtime_tool_display_name_falls_back_to_name() {
        let mut tool = test_tool();
        tool.display_name.clear();

        assert_eq!(tool.display_name_or_name(), "Probe");

        tool.display_name = "Remote Probe".to_string();
        assert_eq!(tool.display_name_or_name(), "Remote Probe");
    }

    #[tokio::test]
    async fn host_capability_rejects_removed_snapshot_operations() {
        let cache = Arc::new(InMemoryCache::new());
        let event_bus = Arc::new(InMemoryEventBus::new());
        let telemetry = Arc::new(NoopTelemetry);
        let ctx = Context::new(cache, event_bus, telemetry);
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities.clear();

        assert!(handle_host_call(
            &ctx,
            &endpoint,
            &tool,
            "snapshot.get",
            serde_json::json!({ "key": "rpc:snapshot" }),
        )
        .await
        .is_err());
        assert!(handle_host_call(
            &ctx,
            &endpoint,
            &tool,
            "snapshot.put",
            serde_json::json!({ "key": "rpc:snapshot", "value": "blocked" }),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn host_capability_can_resolve_workspace_paths() {
        let cache = Arc::new(InMemoryCache::new());
        let event_bus = Arc::new(InMemoryEventBus::new());
        let telemetry = Arc::new(NoopTelemetry);
        let ctx = Context::new(cache, event_bus, telemetry);
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities = vec![CAPABILITY_WORKSPACE_RESOLVE_PATH.to_string()];

        let temp_root = std::env::temp_dir().join(format!(
            "corework-rpc-workspace-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp_root).unwrap();
        let previous_local_appdata = std::env::var("LOCALAPPDATA").ok();
        std::env::set_var("LOCALAPPDATA", &temp_root);

        let source_path = temp_root.join("source.pptx");
        std::fs::write(&source_path, b"pptx smoke").unwrap();
        let source_path = source_path.to_string_lossy().to_string();

        let value = handle_host_call(
            &ctx,
            &endpoint,
            &tool,
            CAPABILITY_WORKSPACE_RESOLVE_PATH,
            serde_json::json!({ "path": source_path }),
        )
        .await
        .unwrap();

        if let Some(previous) = previous_local_appdata {
            std::env::set_var("LOCALAPPDATA", previous);
        } else {
            std::env::remove_var("LOCALAPPDATA");
        }

        let working_path = value
            .get("working_path")
            .and_then(Value::as_str)
            .expect("working_path");
        let returned_source = value
            .get("source_path")
            .and_then(Value::as_str)
            .expect("source_path");
        assert!(std::path::Path::new(working_path).exists());
        assert_eq!(returned_source, source_path);
    }

    #[tokio::test]
    async fn host_capability_rejects_undeclared_workspace_access() {
        let cache = Arc::new(InMemoryCache::new());
        let event_bus = Arc::new(InMemoryEventBus::new());
        let telemetry = Arc::new(NoopTelemetry);
        let ctx = Context::new(cache, event_bus, telemetry);
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities.clear();

        let result = handle_host_call(
            &ctx,
            &endpoint,
            &tool,
            CAPABILITY_WORKSPACE_RESOLVE_PATH,
            serde_json::json!({ "path": "D:/docs/source.pptx" }),
        )
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn runtime_tool_validation_accepts_workspace_capability() {
        validate_runtime_tool_for_endpoint(&test_endpoint(), &test_tool()).unwrap();
    }

    #[test]
    fn runtime_tool_validation_rejects_duplicate_parameter_names() {
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.parameters.push(RuntimeAIParameter {
            name: "key".to_string(),
            param_type: "String".to_string(),
            required: false,
            default_value: None,
            description: String::new(),
        });

        assert!(validate_runtime_tool_for_endpoint(&endpoint, &tool).is_err());
    }

    #[test]
    fn runtime_tool_validation_rejects_unknown_capability() {
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities = vec!["browser.read:rpc:".to_string()];

        assert!(validate_runtime_tool_for_endpoint(&endpoint, &tool).is_err());
    }

    #[test]
    fn runtime_tool_validation_rejects_removed_snapshot_capability() {
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities = vec!["snapshot.get:browser:".to_string()];

        assert!(validate_runtime_tool_for_endpoint(&endpoint, &tool).is_err());
    }

    #[test]
    fn runtime_tool_validation_rejects_removed_snapshot_capability_variant() {
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities = vec!["snapshot.get:rpc:".to_string()];

        assert!(validate_runtime_tool_for_endpoint(&endpoint, &tool).is_err());
    }

    #[test]
    fn runtime_tool_validation_accepts_workspace_capability_without_snapshot_boundary() {
        let endpoint = test_endpoint();
        let mut tool = test_tool();
        tool.required_capabilities = vec![CAPABILITY_WORKSPACE_RESOLVE_PATH.to_string()];

        validate_runtime_tool_for_endpoint(&endpoint, &tool).unwrap();
    }

    #[test]
    fn runtime_tool_set_validation_rejects_duplicate_tool_names() {
        let endpoint = test_endpoint();
        let tools = vec![test_tool(), test_tool()];

        assert!(validate_runtime_tool_set_for_endpoint(&endpoint, &tools).is_err());
    }

    #[test]
    fn list_tools_response_conversion_maps_descriptor_to_runtime_metadata() {
        let endpoint = test_endpoint();
        let response = ListToolsResponse {
            schema: RPC_TOOL_SCHEMA_V1.to_string(),
            tools: vec![ToolDescriptor {
                name: "RemoteProbe".to_string(),
                description: "Remote probe.".to_string(),
                parameters: vec![ToolParameter {
                    name: "key".to_string(),
                    param_type: "String".to_string(),
                    required: true,
                    default_value: Some("rpc:default".to_string()),
                    description: "Snapshot key.".to_string(),
                }],
                outputs: vec![ToolOutputField {
                    name: "value".to_string(),
                    field_type: "String".to_string(),
                    description: "Snapshot value.".to_string(),
                }],
                destructive: false,
                readonly: true,
                idempotent: true,
                open_world: false,
                secret: false,
                category: "debug".to_string(),
                display_name: "Remote Probe".to_string(),
                required_capabilities: vec![CAPABILITY_WORKSPACE_RESOLVE_PATH.to_string()],
            }],
        };

        let tools = runtime_tools_from_list_tools_response(&endpoint, response).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "RemoteProbe");
        assert_eq!(tools[0].display_name, "Remote Probe");
        assert_eq!(tools[0].endpoint_id, "test");
        assert_eq!(tools[0].service, RPC_TOOL_SERVICE_V1);
        assert_eq!(tools[0].method, RPC_TOOL_EXECUTE_METHOD);
        assert_eq!(
            tools[0].parameters[0].default_value.as_deref(),
            Some("rpc:default")
        );
    }

    #[test]
    fn list_tools_response_conversion_rejects_schema_mismatch() {
        let endpoint = test_endpoint();
        let response = ListToolsResponse {
            schema: "other-schema/v1".to_string(),
            tools: vec![],
        };

        assert!(runtime_tools_from_list_tools_response(&endpoint, response).is_err());
    }

    #[test]
    fn grpc_endpoint_uri_adds_http_scheme_for_host_port_addresses() {
        assert_eq!(
            grpc_endpoint_uri("127.0.0.1:50051"),
            "http://127.0.0.1:50051"
        );
        assert_eq!(
            grpc_endpoint_uri("http://localhost:50051"),
            "http://localhost:50051"
        );
    }

    #[test]
    fn agent_tool_request_uses_host_context_for_host_defined_metadata() {
        let request = AgentToolRequest {
            tool_name: "RemoteProbe".to_string(),
            args_cli: String::new(),
            args_json: serde_json::json!({ "key": "rpc:value" }),
            call_id: "call-1".to_string(),
            tool_call_id: "tool-call-1".to_string(),
            idempotency_key: "conv-1/turn-1/tool-call-1".to_string(),
            session_id: "conv-1".to_string(),
            provider_id: "provider-1".to_string(),
            cluster_id: "cluster-1".to_string(),
            runtime_instance_id: "runtime-1".to_string(),
            conversation_id: "conv-1".to_string(),
            agent_id: "agent-1".to_string(),
            turn_id: "turn-1".to_string(),
            permissions: vec![CAPABILITY_WORKSPACE_RESOLVE_PATH.to_string()],
            host_context: serde_json::json!({ "subject": "opaque" }),
        };

        let value = serde_json::to_value(request).unwrap();
        assert!(value.get("host_context").is_some());
    }

    #[test]
    fn rpc_args_json_expands_cli_args_using_tool_metadata() {
        let mut tool = test_tool();
        tool.parameters = vec![
            RuntimeAIParameter {
                name: "user_id".to_string(),
                param_type: "Number".to_string(),
                required: true,
                default_value: None,
                description: String::new(),
            },
            RuntimeAIParameter {
                name: "email_verified".to_string(),
                param_type: "Boolean".to_string(),
                required: false,
                default_value: Some("false".to_string()),
                description: String::new(),
            },
            RuntimeAIParameter {
                name: "tags".to_string(),
                param_type: "StringArray".to_string(),
                required: false,
                default_value: None,
                description: String::new(),
            },
        ];

        let args = build_rpc_args_json(&tool, HashMap::new(), "--user_id 1005 --tags a,b").unwrap();

        assert_eq!(
            args,
            serde_json::json!({
                "user_id": 1005,
                "email_verified": false,
                "tags": ["a", "b"]
            })
        );
    }

    #[test]
    fn rpc_args_json_preserves_structured_inputs_over_cli_args() {
        let mut tool = test_tool();
        tool.parameters = vec![RuntimeAIParameter {
            name: "user_id".to_string(),
            param_type: "Number".to_string(),
            required: true,
            default_value: None,
            description: String::new(),
        }];
        let mut input = HashMap::new();
        input.insert(
            "input".to_string(),
            Value::String("--user_id 1005".to_string()),
        );
        input.insert("user_id".to_string(), serde_json::json!(1006));

        let args = build_rpc_args_json(&tool, input, "--user_id 1005").unwrap();

        assert_eq!(args, serde_json::json!({ "user_id": 1006 }));
    }

    #[test]
    fn rpc_args_json_coerces_structured_string_inputs_using_metadata() {
        let mut tool = test_tool();
        tool.parameters = vec![RuntimeAIParameter {
            name: "user_id".to_string(),
            param_type: "Number".to_string(),
            required: true,
            default_value: None,
            description: String::new(),
        }];
        let mut input = HashMap::new();
        input.insert("user_id".to_string(), Value::String("1005".to_string()));

        let args = build_rpc_args_json(&tool, input, "").unwrap();

        assert_eq!(args, serde_json::json!({ "user_id": 1005 }));
    }

    #[tokio::test]
    async fn remote_ai_output_synthesizes_to_ai_when_remote_omits_it() {
        let output = RemoteAIOutput {
            result: Value::Null,
            to_ai: "   ".to_string(),
            error_code: ToolErrorCode::Ok as i32,
        };

        let output = output.into_checked_ai_output("BadTool").unwrap();
        assert_eq!(output.error_code, ToolErrorCode::InvalidOutput as i32);
        assert!(output.to_ai.contains("BadTool"));
    }

    #[test]
    fn remote_ai_output_maps_protocol_ok_to_local_success() {
        let output = RemoteAIOutput {
            result: serde_json::json!({ "ok": true }),
            to_ai: "done".to_string(),
            error_code: ToolErrorCode::Ok as i32,
        };

        let output = output.into_checked_ai_output("GoodTool").unwrap();

        assert_eq!(output.error_code, 0);
        assert!(output.to_ai.contains("GoodTool"));
        assert!(output.to_ai.contains("completed successfully"));
        assert!(output.to_ai.contains("Remote message: done"));
        assert!(output.to_ai.contains("Result summary: ok: true"));
        assert_ne!(output.to_ai, "done");
    }

    #[test]
    fn remote_ai_output_rewrites_json_to_ai_as_plain_summary() {
        let output = RemoteAIOutput {
            result: Value::Null,
            to_ai: r#"{"status":"accepted","path":"D:\\AudioOut","count":2}"#.to_string(),
            error_code: ToolErrorCode::Ok as i32,
        };

        let output = output.into_checked_ai_output("FrontendAction").unwrap();

        assert_eq!(output.error_code, 0);
        assert!(output.to_ai.contains("FrontendAction"));
        assert!(output.to_ai.contains("completed successfully"));
        assert!(output.to_ai.contains("Remote summary:"));
        assert!(output.to_ai.contains("status: accepted"));
        assert!(output.to_ai.contains(r"path: D:\AudioOut"));
        assert!(output.to_ai.contains("count: 2"));
        assert!(!output.to_ai.contains('{'));
        assert!(!output.to_ai.contains('}'));
    }

    #[tokio::test]
    async fn remote_ai_output_empty_to_ai_summarizes_result_without_raw_json() {
        let output = RemoteAIOutput {
            result: serde_json::json!({
                "status": "accepted",
                "message": "AddFileToConvertQueue done",
            }),
            to_ai: " ".to_string(),
            error_code: ToolErrorCode::Ok as i32,
        };

        let output = output.into_checked_ai_output("FrontendAction").unwrap();

        assert_eq!(output.error_code, ToolErrorCode::InvalidOutput as i32);
        assert!(output.to_ai.contains("FrontendAction"));
        assert!(output.to_ai.contains("reported success"));
        assert!(output.to_ai.contains("Result summary:"));
        assert!(output.to_ai.contains("status: accepted"));
        assert!(output.to_ai.contains("message: AddFileToConvertQueue done"));
        assert!(!output.to_ai.contains("{\"status\""));
    }

    #[test]
    fn remote_ai_output_error_json_to_ai_keeps_failure_explanation() {
        let output = RemoteAIOutput {
            result: Value::Null,
            to_ai: r#"{"error":"file not found","path":"D:\\missing.mp4"}"#.to_string(),
            error_code: ToolErrorCode::NotFound as i32,
        };

        let output = output
            .into_checked_ai_output("AddFileToConvertQueue")
            .unwrap();

        assert_eq!(output.error_code, ToolErrorCode::NotFound as i32);
        assert!(output.to_ai.contains("AddFileToConvertQueue"));
        assert!(output.to_ai.contains("execution failed"));
        assert!(output.to_ai.contains("error_code="));
        assert!(output.to_ai.contains("Remote summary:"));
        assert!(output.to_ai.contains("file not found"));
        assert!(output.to_ai.contains(r"path: D:\missing.mp4"));
        assert!(!output.to_ai.contains('{'));
    }

    #[tokio::test]
    async fn remote_ai_output_empty_error_to_ai_keeps_error_code_and_result_summary() {
        let output = RemoteAIOutput {
            result: serde_json::json!({
                "message": "frontend rejected action",
                "status": "rejected",
            }),
            to_ai: " ".to_string(),
            error_code: ToolErrorCode::Unavailable as i32,
        };

        let output = output
            .into_checked_ai_output("AddFileToConvertQueue")
            .unwrap();

        assert_eq!(output.error_code, ToolErrorCode::Unavailable as i32);
        assert!(output.to_ai.contains("execution failed"));
        assert!(output.to_ai.contains("error_code="));
        assert!(output.to_ai.contains("AddFileToConvertQueue"));
        assert!(output.to_ai.contains("frontend rejected action"));
        assert!(output.to_ai.contains("status: rejected"));
    }

    #[test]
    fn remote_ai_output_parameter_error_gets_synthesized_explanation() {
        let output = RemoteAIOutput {
            result: Value::Null,
            to_ai: "path is required".to_string(),
            error_code: ToolErrorCode::MissingArgument as i32,
        };

        let output = output
            .into_checked_ai_output("AddFileToConvertQueue")
            .unwrap();

        assert_eq!(output.error_code, ToolErrorCode::MissingArgument as i32);
        assert!(output.to_ai.contains("AddFileToConvertQueue"));
        assert!(output.to_ai.contains("missing required argument"));
        assert!(output.to_ai.contains("Remote message: path is required"));
        assert_ne!(output.to_ai, "path is required");
    }
}
