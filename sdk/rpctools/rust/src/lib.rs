use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use futures::Stream;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

pub const SCHEMA: &str = "corework-agent-tool/v1";

pub mod proto {
    tonic::include_proto!("corework.agent_tool.v1");
}

use proto::agent_tool_service_server::{AgentToolService, AgentToolServiceServer};
use proto::tool_stream_message;
use proto::{
    AiOutput as ProtoAIOutput, ExecuteRequest, HostCall, ListToolsRequest, ListToolsResponse,
    ToolDescriptor as ProtoToolDescriptor, ToolError, ToolOutputField as ProtoToolOutputField,
    ToolParameter as ProtoToolParameter, ToolStreamMessage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorCode {
    Unspecified = 0,
    Ok = 1,
    InvalidArgument = 100,
    MissingArgument = 101,
    PermissionDenied = 102,
    NotFound = 103,
    Conflict = 104,
    Internal = 200,
    Timeout = 201,
    Cancelled = 202,
    Unavailable = 203,
    HostCapabilityDenied = 300,
    HostCapabilityUnsupported = 301,
    HostCallFailed = 302,
    ProtocolError = 400,
    InvalidOutput = 401,
    SchemaMismatch = 402,
}

impl From<ToolErrorCode> for i32 {
    fn from(value: ToolErrorCode) -> Self {
        value as i32
    }
}

#[derive(Debug, Clone)]
pub struct AIOutput {
    pub result: Value,
    pub to_ai: String,
    pub error_code: ToolErrorCode,
}

#[derive(Debug, Clone)]
pub struct ToolParameter {
    pub name: String,
    pub param_type: String,
    pub required: bool,
    pub default_value: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ToolOutputField {
    pub name: String,
    pub field_type: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ToolParameter>,
    pub outputs: Vec<ToolOutputField>,
    pub destructive: bool,
    pub readonly: bool,
    pub idempotent: bool,
    pub open_world: bool,
    pub secret: bool,
    pub category: String,
    pub display_name: String,
    pub required_capabilities: Vec<String>,
}

#[derive(Debug)]
struct HostRequest {
    id: String,
    op: String,
    args: Value,
    result: oneshot::Sender<anyhow::Result<Value>>,
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub call_id: String,
    pub tool_call_id: String,
    pub idempotency_key: String,
    pub session_id: String,
    pub provider_id: String,
    pub cluster_id: String,
    pub runtime_instance_id: String,
    pub conversation_id: String,
    pub agent_id: String,
    pub turn_id: String,
    pub permissions: Vec<String>,
    pub host_context: Value,
    host_tx: mpsc::Sender<HostRequest>,
}

impl ToolContext {
    pub async fn workspace_resolve_path(&self, path: &str) -> anyhow::Result<Value> {
        self.host_call(
            "workspace.resolve_path",
            serde_json::json!({ "path": path }),
        )
        .await
    }

    pub async fn workspace_resolve_working_path(&self, path: &str) -> anyhow::Result<Value> {
        self.host_call(
            "workspace.resolve_working_path",
            serde_json::json!({ "path": path }),
        )
        .await
    }

    pub async fn workspace_create_path(&self, path: &str) -> anyhow::Result<Value> {
        self.host_call("workspace.create_path", serde_json::json!({ "path": path }))
            .await
    }

    pub async fn workspace_create_working_path(&self, path: &str) -> anyhow::Result<Value> {
        self.host_call(
            "workspace.create_working_path",
            serde_json::json!({ "path": path }),
        )
        .await
    }

    pub async fn workspace_save_as_edited(
        &self,
        source_path: &str,
        suffix: &str,
    ) -> anyhow::Result<Value> {
        self.host_call(
            "workspace.save_as_edited",
            serde_json::json!({ "source_path": source_path, "suffix": suffix }),
        )
        .await
    }

    async fn host_call(&self, op: &str, args: Value) -> anyhow::Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.host_tx
            .send(HostRequest {
                id: format!("{}-{}", self.call_id, uuid_like_id()),
                op: op.to_string(),
                args,
                result: tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("host call channel closed"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("host call result channel closed"))?
    }
}

impl ToolDescriptor {
    pub fn builder(name: impl Into<String>) -> ToolDescriptorBuilder {
        ToolDescriptorBuilder {
            descriptor: ToolDescriptor {
                name: name.into(),
                description: String::new(),
                parameters: Vec::new(),
                outputs: Vec::new(),
                destructive: false,
                readonly: false,
                idempotent: false,
                open_world: false,
                secret: false,
                category: String::new(),
                display_name: String::new(),
                required_capabilities: Vec::new(),
            },
        }
    }
}

pub struct ToolDescriptorBuilder {
    descriptor: ToolDescriptor,
}

impl ToolDescriptorBuilder {
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.descriptor.description = description.into();
        self
    }

    pub fn parameter(
        mut self,
        name: impl Into<String>,
        param_type: impl Into<String>,
        required: bool,
        default_value: Option<&str>,
        description: impl Into<String>,
    ) -> Self {
        self.descriptor.parameters.push(ToolParameter {
            name: name.into(),
            param_type: param_type.into(),
            required,
            default_value: default_value.map(ToOwned::to_owned),
            description: description.into(),
        });
        self
    }

    pub fn output(
        mut self,
        name: impl Into<String>,
        field_type: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.descriptor.outputs.push(ToolOutputField {
            name: name.into(),
            field_type: field_type.into(),
            description: description.into(),
        });
        self
    }

    pub fn readonly(mut self, value: bool) -> Self {
        self.descriptor.readonly = value;
        self
    }

    pub fn destructive(mut self, value: bool) -> Self {
        self.descriptor.destructive = value;
        self
    }

    pub fn idempotent(mut self, value: bool) -> Self {
        self.descriptor.idempotent = value;
        self
    }

    pub fn open_world(mut self, value: bool) -> Self {
        self.descriptor.open_world = value;
        self
    }

    pub fn secret(mut self, value: bool) -> Self {
        self.descriptor.secret = value;
        self
    }

    pub fn category(mut self, value: impl Into<String>) -> Self {
        self.descriptor.category = value.into();
        self
    }

    pub fn display_name(mut self, value: impl Into<String>) -> Self {
        self.descriptor.display_name = value.into();
        self
    }

    pub fn required_capability(mut self, capability: impl Into<String>) -> Self {
        self.descriptor
            .required_capabilities
            .push(capability.into());
        self
    }

    pub fn build(self) -> ToolDescriptor {
        self.descriptor
    }
}

type BoxedHandler = Arc<
    dyn Fn(ToolContext, Value) -> Pin<Box<dyn Future<Output = anyhow::Result<AIOutput>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
struct RegisteredTool {
    descriptor: ToolDescriptor,
    handler: BoxedHandler,
}

static REGISTRY: OnceLock<Mutex<HashMap<String, RegisteredTool>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, RegisteredTool>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub trait FromToolArgs: Sized + Send + 'static {
    fn from_tool_args(descriptor: &ToolDescriptor, args: Value) -> anyhow::Result<Self>;
}

impl FromToolArgs for () {
    fn from_tool_args(_descriptor: &ToolDescriptor, _args: Value) -> anyhow::Result<Self> {
        Ok(())
    }
}

impl FromToolArgs for Value {
    fn from_tool_args(_descriptor: &ToolDescriptor, args: Value) -> anyhow::Result<Self> {
        Ok(args)
    }
}

impl FromToolArgs for Option<String> {
    fn from_tool_args(descriptor: &ToolDescriptor, args: Value) -> anyhow::Result<Self> {
        let Some(first_parameter) = descriptor.parameters.first() else {
            return Ok(None);
        };
        let value = args
            .as_object()
            .and_then(|object| object.get(&first_parameter.name))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        Ok(value)
    }
}

pub fn register_tool<A, F, Fut>(descriptor: ToolDescriptor, handler: F)
where
    A: FromToolArgs,
    F: Fn(ToolContext, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<AIOutput>> + Send + 'static,
{
    let name = descriptor.name.clone();
    let descriptor_for_handler = descriptor.clone();
    let handler = Arc::new(handler);
    let boxed: BoxedHandler = Arc::new(move |ctx, args| {
        let parsed = A::from_tool_args(&descriptor_for_handler, args);
        let handler = Arc::clone(&handler);
        Box::pin(async move { handler(ctx, parsed?).await })
    });

    registry().lock().expect("tool registry poisoned").insert(
        name,
        RegisteredTool {
            descriptor,
            handler: boxed,
        },
    );
}

pub async fn serve(address: &str) -> anyhow::Result<()> {
    let address: SocketAddr = address.parse()?;
    tonic::transport::Server::builder()
        .add_service(AgentToolServiceServer::new(RustAgentToolService))
        .serve(address)
        .await?;
    Ok(())
}

#[derive(Debug, Default)]
struct RustAgentToolService;

#[tonic::async_trait]
impl AgentToolService for RustAgentToolService {
    async fn list_tools(
        &self,
        request: Request<ListToolsRequest>,
    ) -> Result<Response<ListToolsResponse>, Status> {
        let request = request.into_inner();
        if !request.accepted_schema.is_empty()
            && !request
                .accepted_schema
                .iter()
                .any(|schema| schema == SCHEMA)
        {
            return Err(Status::failed_precondition(format!(
                "unsupported schema; expected {}",
                SCHEMA
            )));
        }

        let tools = registry()
            .lock()
            .map_err(|_| Status::internal("tool registry poisoned"))?
            .values()
            .map(|tool| descriptor_to_proto(&tool.descriptor))
            .collect();

        Ok(Response::new(ListToolsResponse {
            schema: SCHEMA.to_string(),
            tools,
        }))
    }

    type ExecuteStream =
        Pin<Box<dyn Stream<Item = Result<ToolStreamMessage, Status>> + Send + 'static>>;

    async fn execute(
        &self,
        request: Request<tonic::Streaming<ToolStreamMessage>>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        let mut inbound = request.into_inner();
        let (outbound_tx, outbound_rx) = mpsc::channel(16);

        tokio::spawn(async move {
            if let Err(error) = run_execute(&mut inbound, outbound_tx.clone()).await {
                let _ = outbound_tx
                    .send(Ok(tool_error(
                        "",
                        error.to_string(),
                        ToolErrorCode::Internal,
                    )))
                    .await;
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(outbound_rx))))
    }
}

async fn run_execute(
    inbound: &mut tonic::Streaming<ToolStreamMessage>,
    outbound_tx: mpsc::Sender<Result<ToolStreamMessage, Status>>,
) -> anyhow::Result<()> {
    let first = inbound
        .message()
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing ExecuteRequest"))?;
    let call_id = first.call_id.clone();
    let Some(tool_stream_message::Message::ExecuteRequest(execute_request)) = first.message else {
        send_message(
            &outbound_tx,
            tool_error(
                &call_id,
                "first stream message must be ExecuteRequest",
                ToolErrorCode::ProtocolError,
            ),
        )
        .await?;
        return Ok(());
    };

    let Some(tool) = find_tool(&execute_request.tool_name)? else {
        send_message(
            &outbound_tx,
            tool_error(
                &call_id,
                format!("unknown tool {}", execute_request.tool_name),
                ToolErrorCode::NotFound,
            ),
        )
        .await?;
        return Ok(());
    };

    let args = request_args(&tool.descriptor, &execute_request)?;
    let (host_tx, mut host_rx) = mpsc::channel(8);
    let ctx = ToolContext {
        call_id: call_id.clone(),
        tool_call_id: execute_request.tool_call_id.clone(),
        idempotency_key: execute_request.idempotency_key.clone(),
        session_id: execute_request.session_id.clone(),
        provider_id: execute_request.provider_id.clone(),
        cluster_id: execute_request.cluster_id.clone(),
        runtime_instance_id: execute_request.runtime_instance_id.clone(),
        conversation_id: execute_request.conversation_id.clone(),
        agent_id: execute_request.agent_id.clone(),
        turn_id: execute_request.turn_id.clone(),
        permissions: execute_request.permissions.clone(),
        host_context: serde_json::from_str(&execute_request.host_context_json)
            .unwrap_or(Value::Null),
        host_tx,
    };
    let handler = Arc::clone(&tool.handler);
    let mut handler_task = tokio::spawn(async move { handler(ctx, args).await });

    loop {
        tokio::select! {
            host_request = host_rx.recv() => {
                let Some(host_request) = host_request else {
                    continue;
                };
                handle_host_request(inbound, &outbound_tx, &call_id, host_request).await?;
            }
            output = &mut handler_task => {
                match output {
                    Ok(Ok(output)) => {
                        if output.to_ai.trim().is_empty() {
                            send_message(&outbound_tx, tool_error(&call_id, "AIOutput.to_ai must be non-empty", ToolErrorCode::InvalidOutput)).await?;
                        } else {
                            send_message(&outbound_tx, ai_output(&call_id, output)?).await?;
                        }
                    }
                    Ok(Err(error)) => {
                        send_message(&outbound_tx, tool_error(&call_id, error.to_string(), ToolErrorCode::Internal)).await?;
                    }
                    Err(error) => {
                        send_message(&outbound_tx, tool_error(&call_id, error.to_string(), ToolErrorCode::Internal)).await?;
                    }
                }
                return Ok(());
            }
        }
    }
}

fn request_args(
    descriptor: &ToolDescriptor,
    execute_request: &ExecuteRequest,
) -> anyhow::Result<Value> {
    if !execute_request.args_json.trim().is_empty() {
        let args: Value = serde_json::from_str(&execute_request.args_json)
            .map_err(|e| anyhow::anyhow!("invalid args_json: {}", e))?;
        if !args.is_object() {
            return Err(anyhow::anyhow!("args_json must encode an object"));
        }
        return Ok(args);
    }

    if !execute_request.args_cli.trim().is_empty() {
        return args_from_cli(
            descriptor,
            &execute_request.tool_name,
            &execute_request.args_cli,
        );
    }

    Ok(Value::Object(serde_json::Map::new()))
}

fn args_from_cli(
    descriptor: &ToolDescriptor,
    tool_name: &str,
    args_cli: &str,
) -> anyhow::Result<Value> {
    let parsed = parse_cli_args(args_cli);
    let mut args = serde_json::Map::new();

    for parameter in &descriptor.parameters {
        if let Some(value) = parsed.get(&parameter.name) {
            args.insert(parameter.name.clone(), Value::String(value.clone()));
        } else if let Some(default_value) = &parameter.default_value {
            args.insert(parameter.name.clone(), Value::String(default_value.clone()));
        } else if parameter.required {
            return Err(anyhow::anyhow!(
                "missing required argument '{}' for tool '{}'",
                parameter.name,
                tool_name
            ));
        }
    }

    Ok(Value::Object(args))
}

fn parse_cli_args(input: &str) -> HashMap<String, String> {
    let bytes = input.as_bytes();
    let mut args = HashMap::new();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        if bytes[i] != b'-' || i + 1 >= bytes.len() || bytes[i + 1] != b'-' {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }

        i += 2;
        let key_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let key = input[key_start..i].to_string();
        if key.is_empty() {
            continue;
        }

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }

        if i >= bytes.len() || (bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-') {
            args.insert(key, "true".to_string());
            continue;
        }

        let value = if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let mut value = String::new();
            while i < bytes.len() {
                let ch = input[i..].chars().next().expect("valid char boundary");
                let ch_len = ch.len_utf8();
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    let escaped = input[i..].chars().next().expect("valid char boundary");
                    match escaped {
                        'n' if !is_path_like_parameter(&key) => value.push('\n'),
                        't' if !is_path_like_parameter(&key) => value.push('\t'),
                        'r' if !is_path_like_parameter(&key) => value.push('\r'),
                        '\\' | '"' | '\'' => value.push(escaped),
                        other => {
                            value.push('\\');
                            value.push(other);
                        }
                    }
                    i += escaped.len_utf8();
                } else {
                    value.push(ch);
                    i += ch_len;
                }
            }
            value
        } else {
            let value_start = i;
            while i < bytes.len() {
                if bytes[i].is_ascii_whitespace() {
                    let mut j = i;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len()
                        && bytes[j] == b'-'
                        && j + 1 < bytes.len()
                        && bytes[j + 1] == b'-'
                    {
                        break;
                    }
                }
                i += 1;
            }
            input[value_start..i].trim_end().to_string()
        };

        args.insert(key, value);
    }

    args
}

fn is_path_like_parameter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "path"
        || name == "paths"
        || name.ends_with("_path")
        || name.ends_with("_paths")
        || name.ends_with("path")
        || name.ends_with("paths")
        || name.contains("directory")
        || name.contains("folder")
}

async fn handle_host_request(
    inbound: &mut tonic::Streaming<ToolStreamMessage>,
    outbound_tx: &mpsc::Sender<Result<ToolStreamMessage, Status>>,
    call_id: &str,
    host_request: HostRequest,
) -> anyhow::Result<()> {
    send_message(
        outbound_tx,
        ToolStreamMessage {
            call_id: call_id.to_string(),
            message: Some(tool_stream_message::Message::HostCall(HostCall {
                id: host_request.id.clone(),
                op: host_request.op.clone(),
                args_json: serde_json::to_string(&host_request.args)?,
            })),
        },
    )
    .await?;

    let Some(message) = inbound.message().await? else {
        let _ = host_request
            .result
            .send(Err(anyhow::anyhow!("stream closed before HostResult")));
        return Ok(());
    };
    if message.call_id != call_id {
        let _ = host_request
            .result
            .send(Err(anyhow::anyhow!("HostResult call_id mismatch")));
        return Ok(());
    }
    let Some(tool_stream_message::Message::HostResult(host_result)) = message.message else {
        let _ = host_request
            .result
            .send(Err(anyhow::anyhow!("expected HostResult")));
        return Ok(());
    };
    if host_result.id != host_request.id {
        let _ = host_request
            .result
            .send(Err(anyhow::anyhow!("HostResult id mismatch")));
        return Ok(());
    }
    let value: Value = serde_json::from_str(&host_result.value_json).unwrap_or(Value::Null);
    let result = if host_result.ok {
        Ok(value)
    } else {
        Err(anyhow::anyhow!(
            "host call {} failed with code {}: {}",
            host_request.op,
            host_result.code,
            value
        ))
    };
    let _ = host_request.result.send(result);
    Ok(())
}

async fn send_message(
    outbound_tx: &mpsc::Sender<Result<ToolStreamMessage, Status>>,
    message: ToolStreamMessage,
) -> anyhow::Result<()> {
    outbound_tx
        .send(Ok(message))
        .await
        .map_err(|_| anyhow::anyhow!("response stream closed"))
}

fn find_tool(name: &str) -> anyhow::Result<Option<RegisteredTool>> {
    Ok(registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("tool registry poisoned"))?
        .get(name)
        .cloned())
}

fn descriptor_to_proto(descriptor: &ToolDescriptor) -> ProtoToolDescriptor {
    ProtoToolDescriptor {
        name: descriptor.name.clone(),
        description: descriptor.description.clone(),
        parameters: descriptor
            .parameters
            .iter()
            .map(|parameter| ProtoToolParameter {
                name: parameter.name.clone(),
                param_type: parameter.param_type.clone(),
                required: parameter.required,
                default_value: parameter.default_value.clone(),
                description: parameter.description.clone(),
            })
            .collect(),
        outputs: descriptor
            .outputs
            .iter()
            .map(|output| ProtoToolOutputField {
                name: output.name.clone(),
                field_type: output.field_type.clone(),
                description: output.description.clone(),
            })
            .collect(),
        destructive: descriptor.destructive,
        readonly: descriptor.readonly,
        idempotent: descriptor.idempotent,
        open_world: descriptor.open_world,
        secret: descriptor.secret,
        category: descriptor.category.clone(),
        display_name: descriptor.display_name.clone(),
        required_capabilities: descriptor.required_capabilities.clone(),
    }
}

fn ai_output(call_id: &str, output: AIOutput) -> anyhow::Result<ToolStreamMessage> {
    Ok(ToolStreamMessage {
        call_id: call_id.to_string(),
        message: Some(tool_stream_message::Message::AiOutput(ProtoAIOutput {
            result_json: serde_json::to_string(&output.result)?,
            to_ai: output.to_ai,
            error_code: output.error_code.into(),
        })),
    })
}

fn tool_error(
    call_id: impl Into<String>,
    message: impl Into<String>,
    code: ToolErrorCode,
) -> ToolStreamMessage {
    ToolStreamMessage {
        call_id: call_id.into(),
        message: Some(tool_stream_message::Message::Error(ToolError {
            message: message.into(),
            code: code.into(),
        })),
    }
}

fn uuid_like_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::agent_tool_service_client::AgentToolServiceClient;
    use proto::ExecuteRequest;

    #[test]
    fn args_from_cli_uses_descriptor_parameters() {
        let descriptor = ToolDescriptor::builder("ListDir")
            .parameter("path", "String", true, None, "Directory path.")
            .parameter("limit", "String", false, Some("20"), "Limit.")
            .build();

        let args = args_from_cli(
            &descriptor,
            "ListDir",
            r#"ListDir --path "C:\workspace\ai-framework\examples\python_ctypes""#,
        )
        .unwrap();

        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(r#"C:\workspace\ai-framework\examples\python_ctypes"#)
        );
        assert_eq!(args.get("limit").and_then(Value::as_str), Some("20"));
    }

    #[test]
    fn request_args_prefers_structured_json_over_lossy_cli_reparse() {
        let descriptor = ToolDescriptor::builder("ListDirectoryFiles")
            .parameter("path", "String", true, None, "Directory path.")
            .build();
        let request = ExecuteRequest {
            tool_name: "ListDirectoryFiles".to_string(),
            args_cli: r#"--path "C:\workspace\samples\nnnc""#.to_string(),
            args_json: serde_json::json!({
                "path": r"C:\workspace\samples\nnnc"
            })
            .to_string(),
            ..Default::default()
        };

        let args = request_args(&descriptor, &request).unwrap();

        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(r"C:\workspace\samples\nnnc")
        );
    }

    #[test]
    fn request_args_falls_back_to_cli_for_legacy_clients() {
        let descriptor = ToolDescriptor::builder("ListDir")
            .parameter("path", "String", true, None, "Directory path.")
            .build();
        let request = ExecuteRequest {
            tool_name: "ListDir".to_string(),
            args_cli: r#"--path "C:\workspace\audio""#.to_string(),
            args_json: String::new(),
            ..Default::default()
        };

        let args = request_args(&descriptor, &request).unwrap();

        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(r"C:\workspace\audio")
        );
    }

    #[test]
    fn legacy_cli_preserves_tab_and_carriage_return_like_path_segments() {
        let descriptor = ToolDescriptor::builder("ListDirectoryFiles")
            .parameter("path", "String", true, None, "Directory path.")
            .build();
        let request = ExecuteRequest {
            tool_name: "ListDirectoryFiles".to_string(),
            args_cli: r#"--path "C:\workspace\src-tauri\target\release\bundle\nsis""#.to_string(),
            args_json: String::new(),
            ..Default::default()
        };

        let args = request_args(&descriptor, &request).unwrap();

        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(r"C:\workspace\src-tauri\target\release\bundle\nsis")
        );
    }

    #[tokio::test]
    async fn rust_sdk_serves_list_tools_and_execute() {
        register_tool(
            ToolDescriptor::builder("RustSdkSmoke")
                .description("Rust SDK smoke tool.")
                .parameter("key", "String", false, None, "Smoke key.")
                .output("ok", "Boolean", "Whether the call worked.")
                .readonly(true)
                .idempotent(true)
                .build(),
            |ctx: ToolContext, key: Option<String>| async move {
                assert_eq!(ctx.conversation_id, "session");
                assert_eq!(ctx.agent_id, "agent-test");
                Ok(AIOutput {
                    result: serde_json::json!({ "key": key, "ok": true }),
                    to_ai: "rust sdk smoke ok".to_string(),
                    error_code: ToolErrorCode::Ok,
                })
            },
        );

        tokio::spawn(async {
            let _ = serve("127.0.0.1:50170").await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut client = AgentToolServiceClient::connect("http://127.0.0.1:50170")
            .await
            .unwrap();
        let tools = client
            .list_tools(ListToolsRequest {
                runtime_version: "test".to_string(),
                accepted_schema: vec![SCHEMA.to_string()],
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(tools.schema, SCHEMA);
        assert!(tools.tools.iter().any(|tool| tool.name == "RustSdkSmoke"));

        let request = ToolStreamMessage {
            call_id: "rust-sdk-smoke-call".to_string(),
            message: Some(tool_stream_message::Message::ExecuteRequest(
                ExecuteRequest {
                    tool_name: "RustSdkSmoke".to_string(),
                    args_cli: r#"RustSdkSmoke --key workflow:test"#.to_string(),
                    args_json: r#"{"key":"workflow:test"}"#.to_string(),
                    session_id: "session".to_string(),
                    request_id: "request".to_string(),
                    tool_call_id: "rust-sdk-smoke-call".to_string(),
                    idempotency_key: "request/rust-sdk-smoke-call".to_string(),
                    provider_id: "sdk-test".to_string(),
                    cluster_id: String::new(),
                    runtime_instance_id: String::new(),
                    conversation_id: "session".to_string(),
                    agent_id: "agent-test".to_string(),
                    turn_id: String::new(),
                    permissions: Vec::new(),
                    host_context_json: "{}".to_string(),
                },
            )),
        };
        let mut stream = client
            .execute(tokio_stream::iter(vec![request]))
            .await
            .unwrap()
            .into_inner();
        let output = stream.message().await.unwrap().unwrap();
        let Some(tool_stream_message::Message::AiOutput(output)) = output.message else {
            panic!("expected AIOutput");
        };
        assert_eq!(output.to_ai, "rust sdk smoke ok");
        assert_eq!(output.error_code, ToolErrorCode::Ok as i32);
    }
}
