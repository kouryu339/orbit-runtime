use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex as StdMutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ai_assistant::{ConversationManager, SkillManager};
use async_trait::async_trait;
use corework::event::{BaseEvent, EventHandler};
use corework::rpc_tool::RuntimeToolMetadata;
use corework::workflow::registry::NodeRegistry;
use corework::workflow::workflows::WorkflowExecutionContext;
use corework::workflow::WorkflowsModule;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::runtime::{Builder, Handle};
use tokio::sync::watch;

use crate::runtime::{
    lease_renew_interval, run_lease_renewer, RuntimeConfig, RuntimeCoordinationBackend,
    RuntimeError, LOCAL_LEASE_RENEW_INTERVAL_MS, LOCAL_LEASE_TTL_MS,
};

pub(crate) const WORKFLOW_EDITOR_ROLE_SKILL: &str = "workflow_editor";

pub(crate) struct WorkflowStudioSnapshotProjector {
    name: String,
    workflows: Arc<WorkflowsModule>,
    editor_session: Arc<corework::workflow::workflows::WorkflowEditorSession>,
    manager: Arc<ConversationManager>,
    conversation_id: String,
    agent_id: String,
}

impl WorkflowStudioSnapshotProjector {
    pub(crate) fn new(
        name: String,
        workflows: Arc<WorkflowsModule>,
        editor_session: Arc<corework::workflow::workflows::WorkflowEditorSession>,
        manager: Arc<ConversationManager>,
        conversation_id: String,
        agent_id: String,
    ) -> Self {
        Self {
            name,
            workflows,
            editor_session,
            manager,
            conversation_id,
            agent_id,
        }
    }

    async fn set_snapshot(&self, field: &str, value: String) -> corework::error::Result<()> {
        self.manager
            .set_host_dynamic_snapshot_field(&self.conversation_id, &self.agent_id, field, &value)
            .await
            .map_err(|error| corework::error::FrameworkError::SystemError(error.to_string()))
    }

    async fn project(&self, event: &BaseEvent) -> corework::error::Result<()> {
        let changed_id = event.payload.get("workflow_id").and_then(Value::as_str);
        if let Some(selection) = self.editor_session.selection() {
            if changed_id == Some(selection.workflow_id.as_str()) {
                if event.payload.get("operation").and_then(Value::as_str) == Some("deleted") {
                    self.editor_session.clear();
                    self.set_snapshot("workflow_studio.current_resource", String::new())
                        .await?;
                    self.set_snapshot("workflow_studio.current_draft", String::new())
                        .await?;
                } else {
                    let resource = self
                        .workflows
                        .read_workflow_resource(&selection.workflow_id)?;
                    self.editor_session
                        .select(resource.summary.id.clone(), resource.summary.revision);
                    self.set_snapshot(
                        "workflow_studio.current_resource",
                        serde_json::to_string(&resource)
                            .map_err(corework::error::FrameworkError::SerializationError)?,
                    )
                    .await?;
                    self.set_snapshot(
                        "workflow_studio.current_draft",
                        resource.script.unwrap_or_default(),
                    )
                    .await?;
                }
            }
        }

        let workflows = self.workflows.list_workflow_catalog(None)?;
        self.set_snapshot(
            "workflow_studio.workflows",
            serde_json::to_string(&json!({
                "schema": "workflow-studio-workflows/v2",
                "workflows_dir": self.workflows.workflows_dir().to_string_lossy(),
                "selection": self.editor_session.selection(),
                "workflows": workflows
            }))
            .map_err(corework::error::FrameworkError::SerializationError)?,
        )
        .await
    }
}

#[async_trait]
impl EventHandler for WorkflowStudioSnapshotProjector {
    async fn handle(&self, event: &BaseEvent) -> corework::error::Result<()> {
        self.project(event).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

pub(crate) struct WorkflowStudioServer {
    pub(crate) workflows_dir: PathBuf,
    pub(crate) url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WorkflowStudioServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", studio_port_from_url(&self.url)));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
pub(crate) struct WorkflowStudioChatBridge {
    pub(crate) runtime_handle: Handle,
    pub(crate) manager: Arc<ConversationManager>,
    pub(crate) coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    pub(crate) cluster_id: String,
    pub(crate) runtime_instance_id: String,
}

impl WorkflowStudioChatBridge {
    fn send_message_with_admission(
        &self,
        conversation_id: String,
        content: String,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        let manager = Arc::clone(&self.manager);
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let lease_owner = self.runtime_instance_id.clone();
        let lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let lease_renew_interval =
            lease_renew_interval(lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.cluster_id, conversation_id
        );
        self.runtime_handle.block_on(async move {
            let runtime_conversation_id = conversation_id.clone();
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                ttl_ms = lease_ttl_ms,
                content_len = content.len(),
                "Workflow Studio turn lease acquire start"
            );
            let acquired = coordination_backend
                .acquire_lease(&lease_key, &lease_owner, lease_ttl_ms)
                .await?;
            if !acquired {
                tracing::warn!(
                    conversation_id = %runtime_conversation_id,
                    lease_owner = %lease_owner,
                    lease_key = %lease_key,
                    "Workflow Studio turn lease acquire rejected"
                );
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' turn lease is held by another runtime",
                    runtime_conversation_id
                )));
            }

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
            let release_result = coordination_backend
                .release_lease(&lease_key, &lease_owner)
                .await;
            match (send_result, release_result) {
                (Ok(admission), Ok(())) => Ok(admission),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!(
                        "release Workflow Studio turn lease failed: {}",
                        release_error
                    );
                    Err(error)
                }
            }
        })
    }

    fn request_pause_with_admission(
        &self,
        conversation_id: String,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        let manager = Arc::clone(&self.manager);
        self.runtime_handle.block_on(async move {
            manager
                .request_pause_with_admission(&conversation_id, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    fn resolve_tool_permission(
        &self,
        conversation_id: String,
        tool_call_id: String,
        decision: ai_assistant::ToolPermissionDecision,
    ) -> Result<bool, RuntimeError> {
        let manager = Arc::clone(&self.manager);
        self.runtime_handle.block_on(async move {
            manager
                .resolve_tool_permission(&conversation_id, &tool_call_id, decision)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct WorkflowStudioOpenOptions {
    #[serde(alias = "agent_id")]
    pub(crate) agent_id: String,
    pub(crate) workflow_name: String,
    pub(crate) readonly: bool,
    pub(crate) open_browser: bool,
    pub(crate) tool_execution_policy: String,
}

impl Default for WorkflowStudioOpenOptions {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            workflow_name: String::new(),
            readonly: false,
            open_browser: false,
            tool_execution_policy: "open_readonly_confirm_destructive".to_string(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct WorkflowStudioState {
    pub(crate) session_id: String,
    pub(crate) token: String,
    pub(crate) editor_conversation_id: String,
    pub(crate) chat_bridge: WorkflowStudioChatBridge,
    pub(crate) editor_agent_id: String,
    pub(crate) editor_agent_name: String,
    pub(crate) editor_agent_role: Option<String>,
    pub(crate) config: RuntimeConfig,
    pub(crate) runtime_tools: Vec<RuntimeToolMetadata>,
    pub(crate) node_capabilities: Vec<Value>,
    pub(crate) workflows: Arc<WorkflowsModule>,
    pub(crate) editor_session: Arc<corework::workflow::workflows::WorkflowEditorSession>,
    pub(crate) readonly: bool,
    pub(crate) workflow_name: String,
    pub(crate) tool_execution_policy: String,
    pub(crate) event_log: Arc<StdMutex<VecDeque<Value>>>,
}

pub(crate) fn collect_workflow_studio_node_capabilities(
    runtime_tools: &[RuntimeToolMetadata],
) -> Vec<Value> {
    crate::runtime::workflow_node_definition_values(runtime_tools)
}

pub(crate) fn parse_workflow_studio_options(
    options_json: &str,
) -> Result<WorkflowStudioOpenOptions, RuntimeError> {
    let options = if options_json.trim().is_empty() {
        WorkflowStudioOpenOptions::default()
    } else {
        serde_json::from_str(options_json).map_err(|e| {
            RuntimeError::InvalidConfig(format!("parse workflow studio options_json failed: {e}"))
        })?
    };
    workflow_studio_tool_permission_policy(&options.tool_execution_policy)?;
    Ok(options)
}

pub(crate) fn workflow_studio_tool_permission_policy(
    policy: &str,
) -> Result<ai_assistant::ToolPermissionPolicy, RuntimeError> {
    use ai_assistant::{ToolPermissionMode, ToolPermissionPolicy};

    match policy {
        "open_readonly_confirm_destructive" => Ok(ToolPermissionPolicy {
            read_only: ToolPermissionMode::Full,
            controlled_change: ToolPermissionMode::Full,
            destructive: ToolPermissionMode::Ask,
        }),
        "open_all" => Ok(ToolPermissionPolicy::default()),
        other => Err(RuntimeError::InvalidConfig(format!(
            "workflow studio tool_execution_policy '{}' is unsupported; use 'open_readonly_confirm_destructive' or 'open_all'",
            other
        ))),
    }
}

pub(crate) struct StudioToolPermissionRequest {
    pub(crate) tool_call_id: String,
    pub(crate) decision: ai_assistant::ToolPermissionDecision,
}

pub(crate) fn parse_studio_tool_permission_request(
    body: &[u8],
    expected_conversation_id: &str,
) -> Result<StudioToolPermissionRequest, String> {
    #[derive(Deserialize)]
    struct Input {
        conversation_id: String,
        tool_call_id: String,
        decision: String,
    }

    let input = serde_json::from_slice::<Input>(body)
        .map_err(|error| format!("invalid tool permission JSON: {error}"))?;
    if input.conversation_id != expected_conversation_id {
        return Err("conversation_id does not belong to this Studio session".to_string());
    }
    if input.tool_call_id.trim().is_empty() {
        return Err("tool_call_id must not be empty".to_string());
    }
    let decision = match input.decision.as_str() {
        "allow" => ai_assistant::ToolPermissionDecision::Allow,
        "deny" => ai_assistant::ToolPermissionDecision::Deny,
        _ => return Err("decision must be 'allow' or 'deny'".to_string()),
    };
    Ok(StudioToolPermissionRequest {
        tool_call_id: input.tool_call_id,
        decision,
    })
}

pub(crate) fn next_studio_session_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("studio_{millis}_{seq}")
}

pub(crate) fn next_studio_token() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{seq:x}")
}

fn next_studio_command_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("studio_send_message_{millis}_{seq}")
}

pub(crate) fn start_workflow_studio_server(
    state: WorkflowStudioState,
) -> Result<WorkflowStudioServer, RuntimeError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| RuntimeError::Internal(format!("bind Workflow Studio failed: {e}")))?;
    listener.set_nonblocking(false).map_err(|e| {
        RuntimeError::Internal(format!("configure Workflow Studio socket failed: {e}"))
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| RuntimeError::Internal(format!("read Workflow Studio address failed: {e}")))?
        .port();
    let url = format!("http://127.0.0.1:{port}/?token={}", state.token);
    let workflows_dir = state.workflows.workflows_dir().clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("workflow-studio-http".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }
                match stream {
                    Ok(mut stream) => {
                        let state = state.clone();
                        let stop = Arc::clone(&stop_thread);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let _ = std::thread::Builder::new()
                            .name("workflow-studio-http-client".to_string())
                            .spawn(move || {
                                handle_workflow_studio_stream(&state, &stop, &mut stream)
                            });
                    }
                    Err(e) => {
                        tracing::warn!("Workflow Studio connection failed: {e}");
                    }
                }
            }
        })
        .map_err(|e| RuntimeError::Internal(format!("spawn Workflow Studio failed: {e}")))?;

    Ok(WorkflowStudioServer {
        workflows_dir,
        url,
        stop,
        thread: Some(thread),
    })
}

fn studio_port_from_url(url: &str) -> u16 {
    url.split(':')
        .nth(2)
        .and_then(|tail| tail.split('/').next())
        .and_then(|port| port.parse().ok())
        .unwrap_or(0)
}

fn handle_workflow_studio_stream(
    state: &WorkflowStudioState,
    stop: &AtomicBool,
    stream: &mut TcpStream,
) {
    let request = match read_http_request(stream) {
        Ok(request) => request,
        Err(e) => {
            let _ = write_json_response(stream, 400, json!({"error": e}));
            return;
        }
    };
    if request.method == "GET" && request.path == "/events" {
        if studio_request_authorized(state, &request) {
            let _ = write_workflow_studio_sse(state, stop, stream, &request);
        } else {
            let _ = write_json_response(
                stream,
                401,
                json!({"error": "invalid workflow studio token"}),
            );
        }
        return;
    }
    let response = handle_workflow_studio_request(state, request);
    let _ = match response {
        StudioResponse::Html(html) => {
            write_http_response(stream, 200, "text/html; charset=utf-8", html)
        }
        StudioResponse::Static {
            status,
            content_type,
            body,
        } => write_http_bytes_response(stream, status, content_type, body),
        StudioResponse::Json(status, value) => write_json_response(stream, status, value),
    };
}

struct StudioRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

enum StudioResponse {
    Html(String),
    Static {
        status: u16,
        content_type: &'static str,
        body: &'static [u8],
    },
    Json(u16, Value),
}

fn read_http_request(stream: &mut TcpStream) -> Result<StudioRequest, String> {
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 4096];
    let mut header_end = None;
    while header_end.is_none() && buffer.len() < 1024 * 1024 {
        let read = stream.read(&mut temp).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..read]);
        header_end = buffer.windows(4).position(|w| w == b"\r\n\r\n");
    }
    let header_end = header_end.ok_or_else(|| "incomplete HTTP request".to_string())?;
    let header_bytes = &buffer[..header_end];
    let headers_text = std::str::from_utf8(header_bytes).map_err(|e| e.to_string())?;
    let mut lines = headers_text.lines();
    let start = lines
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut start_parts = start.split_whitespace();
    let method = start_parts.next().unwrap_or("").to_string();
    let target = start_parts.next().unwrap_or("/");
    let (path, query) = split_target(target);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut temp).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..read]);
    }
    let body = buffer
        .get(body_start..body_start + content_length.min(buffer.len().saturating_sub(body_start)))
        .unwrap_or(&[])
        .to_vec();
    Ok(StudioRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, query_text) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in query_text.split('&').filter(|part| !part.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(percent_decode(key), percent_decode(value));
    }
    (percent_decode(path), query)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(hex);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn handle_workflow_studio_request(
    state: &WorkflowStudioState,
    request: StudioRequest,
) -> StudioResponse {
    if request.method == "OPTIONS" {
        return StudioResponse::Json(204, json!({}));
    }
    if request.path == "/" || request.path == "/index.html" {
        return workflow_studio_static_asset("/index.html")
            .unwrap_or_else(|| StudioResponse::Html(workflow_studio_html()));
    }
    if matches!(
        request.path.as_str(),
        "/assets/index.js" | "/assets/index.css"
    ) {
        return workflow_studio_static_asset(&request.path)
            .unwrap_or_else(|| StudioResponse::Json(404, json!({"error": "not found"})));
    }
    if !studio_request_authorized(state, &request) {
        return StudioResponse::Json(401, json!({"error": "invalid workflow studio token"}));
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/context") => StudioResponse::Json(200, studio_context_json(state)),
        ("GET", "/api/tools") => StudioResponse::Json(200, studio_tools_json(state)),
        ("GET", "/api/skills") => StudioResponse::Json(200, studio_skills_json(state)),
        ("GET", "/api/workflows") => StudioResponse::Json(200, studio_workflows_json(state)),
        ("POST", "/api/compile") => {
            StudioResponse::Json(200, studio_compile_json(state, &request.body))
        }
        ("POST", "/api/decompile") => studio_decompile_response(state, &request.body),
        ("POST", "/api/nodes/instantiate") => {
            StudioResponse::Json(200, studio_instantiate_node_json(&request.body))
        }
        ("POST", "/api/normalize") => {
            StudioResponse::Json(200, studio_normalize_json(state, &request.body))
        }
        ("POST", "/api/layout") => StudioResponse::Json(200, studio_layout_json(&request.body)),
        ("POST", "/api/save") => StudioResponse::Json(200, studio_save_json(state, &request.body)),
        ("POST", "/api/run") => StudioResponse::Json(200, studio_run_json(state, &request.body)),
        ("POST", "/api/studio-state") => studio_state_snapshot_response(state, &request.body),
        ("POST", "/api/skill-refs/search") => {
            StudioResponse::Json(200, studio_skill_refs_search_json(state, &request.body))
        }
        ("POST", "/api/skill-patch/propose") => {
            StudioResponse::Json(200, studio_skill_patch_propose_json(state, &request.body))
        }
        ("POST", "/api/skill-patch/apply") => {
            StudioResponse::Json(200, studio_skill_patch_apply_json(state, &request.body))
        }
        ("POST", "/api/chat") => studio_chat_response(state, &request.body),
        ("POST", "/api/pause") => studio_pause_response(state, &request.body),
        ("POST", "/api/tool-permission") => studio_tool_permission_response(state, &request.body),
        _ if request.path.starts_with("/api/skills/") => {
            let name = request.path.trim_start_matches("/api/skills/");
            StudioResponse::Json(200, studio_skill_detail_json(state, name))
        }
        _ if request.path.starts_with("/api/workflows/") => {
            let name = request.path.trim_start_matches("/api/workflows/");
            StudioResponse::Json(200, studio_workflow_detail_json(state, name))
        }
        _ => StudioResponse::Json(404, json!({"error": "not found"})),
    }
}

fn write_workflow_studio_sse(
    state: &WorkflowStudioState,
    stop: &AtomicBool,
    stream: &mut TcpStream,
    request: &StudioRequest,
) -> std::io::Result<()> {
    let mut since = request
        .query
        .get("since")
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| {
            request
                .headers
                .get("last-event-id")
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or(0);
    stream.set_write_timeout(None)?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n"
    )?;
    stream.flush()?;
    let mut last_heartbeat = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let events = state
            .event_log
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| {
                        event.get("event_seq").and_then(Value::as_u64).unwrap_or(0) > since
                    })
                    .filter(|event| {
                        let is_editor_conversation = event
                            .get("conversation_id")
                            .and_then(Value::as_str)
                            .is_some_and(|value| value == state.editor_conversation_id);
                        let is_workflow_line =
                            event.get("event_line").and_then(Value::as_str) == Some("workflow");
                        is_editor_conversation || is_workflow_line
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for event in events {
            let event_seq = event
                .get("event_seq")
                .and_then(Value::as_u64)
                .unwrap_or(since);
            let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
            write!(stream, "id: {event_seq}\ndata: {data}\n\n")?;
            since = event_seq;
            last_heartbeat = Instant::now();
        }
        if last_heartbeat.elapsed() >= Duration::from_secs(15) {
            write!(stream, ": keep-alive\n\n")?;
            last_heartbeat = Instant::now();
        }
        stream.flush()?;
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn studio_request_authorized(state: &WorkflowStudioState, request: &StudioRequest) -> bool {
    request
        .query
        .get("token")
        .map(|value| value == &state.token)
        .unwrap_or(false)
        || request
            .headers
            .get("authorization")
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(|value| value == state.token)
            .unwrap_or(false)
}

fn studio_context_json(state: &WorkflowStudioState) -> Value {
    let snapshot = state
        .event_log
        .lock()
        .ok()
        .and_then(|events| {
            events.iter().rev().find_map(|event| {
                let is_snapshot =
                    event.get("type").and_then(Value::as_str) == Some("frontend:state_snapshot");
                let is_editor = event.get("conversation_id").and_then(Value::as_str)
                    == Some(state.editor_conversation_id.as_str());
                (is_snapshot && is_editor)
                    .then(|| event.get("payload").cloned())
                    .flatten()
            })
        })
        .unwrap_or_else(|| {
            json!({
                "conversation_id": state.editor_conversation_id,
                "revision": 0,
                "conversation_state": "waiting",
                "ledger_records": []
            })
        });
    json!({
        "schema": "workflow-studio-context/v1",
        "session_id": state.session_id,
        "conversation_id": state.editor_conversation_id,
        "snapshot": snapshot,
        "editor_conversation_id": state.editor_conversation_id,
        "editor_agent": {
            "id": state.editor_agent_id,
            "name": state.editor_agent_name,
            "role": state.editor_agent_role,
        },
        "runtime": {
            "language": state.config.runtime.language,
            "cluster_id": state.config.runtime.cluster_id,
            "runtime_profile_id": state.config.runtime.runtime_profile_id,
            "runtime_instance_id": state.config.runtime.runtime_instance_id,
        },
        "workflows_dir": state.workflows.workflows_dir().to_string_lossy(),
        "workflow_name": state.workflow_name,
        "node_capabilities": state.node_capabilities,
        "readonly": state.readonly,
        "tool_execution_policy": state.tool_execution_policy
    })
}

fn studio_chat_response(state: &WorkflowStudioState, body: &[u8]) -> StudioResponse {
    let parsed = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(e) => {
            return StudioResponse::Json(400, json!({"error": format!("invalid JSON body: {e}")}));
        }
    };
    let Some(object) = parsed.as_object() else {
        return StudioResponse::Json(400, json!({"error": "body must be a JSON object"}));
    };
    let content = object
        .get("message")
        .or_else(|| object.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return StudioResponse::Json(400, json!({"error": "message must not be empty"}));
    }
    let command_id = object
        .get("command_id")
        .or_else(|| object.get("commandId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(next_studio_command_id);

    match state.chat_bridge.send_message_with_admission(
        state.editor_conversation_id.clone(),
        content,
        command_id,
    ) {
        Ok(admission) => {
            let decision = serde_json::to_value(&admission.decision)
                .unwrap_or_else(|_| json!({"decision": "unknown"}));
            StudioResponse::Json(
                200,
                json!({
                    "schema": "workflow-studio-chat-result/v1",
                    "conversation_id": state.editor_conversation_id,
                    "command_id": admission.command_id,
                    "decision": decision
                }),
            )
        }
        Err(e) => StudioResponse::Json(500, json!({"error": e.to_string()})),
    }
}

fn studio_pause_response(state: &WorkflowStudioState, body: &[u8]) -> StudioResponse {
    let parsed = serde_json::from_slice::<Value>(body).unwrap_or_else(|_| json!({}));
    let command_id = parsed
        .get("command_id")
        .or_else(|| parsed.get("commandId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(next_studio_command_id);

    match state
        .chat_bridge
        .request_pause_with_admission(state.editor_conversation_id.clone(), command_id)
    {
        Ok(admission) => StudioResponse::Json(
            200,
            json!({
                "schema": "workflow-studio-pause-result/v1",
                "conversation_id": state.editor_conversation_id,
                "command_id": admission.command_id,
                "decision": admission.decision
            }),
        ),
        Err(e) => StudioResponse::Json(500, json!({"error": e.to_string()})),
    }
}

fn studio_tool_permission_response(state: &WorkflowStudioState, body: &[u8]) -> StudioResponse {
    let request = match parse_studio_tool_permission_request(body, &state.editor_conversation_id) {
        Ok(request) => request,
        Err(error) => return StudioResponse::Json(400, json!({"error": error})),
    };
    match state.chat_bridge.resolve_tool_permission(
        state.editor_conversation_id.clone(),
        request.tool_call_id,
        request.decision,
    ) {
        Ok(resolved) => StudioResponse::Json(200, json!({"resolved": resolved})),
        Err(error) => StudioResponse::Json(500, json!({"error": error.to_string()})),
    }
}

fn studio_state_snapshot_response(state: &WorkflowStudioState, body: &[u8]) -> StudioResponse {
    let _input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => {
            return StudioResponse::Json(400, json!({"error": format!("invalid JSON body: {e}")}));
        }
    };
    let workflow_snapshot = studio_workflows_json(state);
    let workflow_snapshot = match serde_json::to_string(&workflow_snapshot) {
        Ok(value) => value,
        Err(error) => {
            return StudioResponse::Json(500, json!({"error": error.to_string()}));
        }
    };
    if let Err(error) = set_workflow_studio_snapshot_field(
        state,
        "workflow_studio.workflows",
        workflow_snapshot.clone(),
    ) {
        return StudioResponse::Json(500, json!({"error": error.to_string()}));
    }
    let mut current_resource_bytes = 0;
    let mut current_script_bytes = 0;
    if let Some(selection) = state.editor_session.selection() {
        if let Ok(resource) = state
            .workflows
            .read_workflow_resource(&selection.workflow_id)
        {
            let resource_json = serde_json::to_string(&resource).unwrap_or_default();
            current_resource_bytes = resource_json.len();
            current_script_bytes = resource.script.as_deref().map(str::len).unwrap_or(0);
            if let Err(error) = set_workflow_studio_snapshot_field(
                state,
                "workflow_studio.current_resource",
                resource_json,
            ) {
                return StudioResponse::Json(500, json!({"error": error.to_string()}));
            }
            if let Some(script) = resource.script {
                if let Err(error) = set_workflow_studio_current_draft(state, script) {
                    return StudioResponse::Json(500, json!({"error": error.to_string()}));
                }
            }
        }
    }
    StudioResponse::Json(
        200,
        json!({
            "schema": "workflow-studio-state-snapshot-result/v2",
            "fields": {
                "workflow_studio.current_resource": current_resource_bytes,
                "workflow_studio.current_draft": current_script_bytes,
                "workflow_studio.workflows": workflow_snapshot.len()
            }
        }),
    )
}

fn set_workflow_studio_snapshot_field(
    state: &WorkflowStudioState,
    field: &str,
    value: String,
) -> Result<(), RuntimeError> {
    let manager = Arc::clone(&state.chat_bridge.manager);
    let conversation_id = state.editor_conversation_id.clone();
    let editor_agent_id = state.editor_agent_id.clone();
    let field = field.to_string();
    state.chat_bridge.runtime_handle.block_on(async move {
        manager
            .set_host_dynamic_snapshot_field(&conversation_id, &editor_agent_id, &field, &value)
            .await
            .map_err(|e| RuntimeError::Internal(e.to_string()))
    })
}

fn set_workflow_studio_current_draft(
    state: &WorkflowStudioState,
    snapshot_text: String,
) -> Result<(), RuntimeError> {
    set_workflow_studio_snapshot_field(state, "workflow_studio.current_draft", snapshot_text)
}

fn studio_tools_json(state: &WorkflowStudioState) -> Value {
    json!({
        "schema": "workflow-studio-tools/v1",
        "rpc_tools": state.runtime_tools,
        "node_capabilities": state.node_capabilities
    })
}

fn studio_skills_json(state: &WorkflowStudioState) -> Value {
    let Some(skills_dir) = state.config.runtime.skills_dir.clone() else {
        return json!({"error": "runtime.skills_dir is not configured"});
    };
    let result = run_short_tokio(async move { SkillManager::from_directory(&skills_dir).await });
    match result {
        Ok(manager) => {
            let skills: Vec<Value> = manager
                .all_metadata()
                .into_iter()
                .filter(|meta| !meta.system_layer)
                .map(|meta| {
                    json!({
                        "name": meta.name,
                        "kind": meta.kind,
                        "description": meta.description,
                        "tools": meta.tools,
                        "workflows": meta.workflows
                    })
                })
                .collect();
            json!({"schema": "workflow-studio-skills/v1", "skills": skills})
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn studio_skill_detail_json(state: &WorkflowStudioState, name: &str) -> Value {
    let Some(skills_dir) = state.config.runtime.skills_dir.clone() else {
        return json!({"error": "runtime.skills_dir is not configured"});
    };
    let name = name.to_string();
    let result = run_short_tokio(async move {
        let mut manager = SkillManager::from_directory(&skills_dir).await?;
        manager.load(&name).await.cloned()
    });
    match result {
        Ok(skill) => json!({
            "schema": "workflow-studio-skill-detail/v1",
            "skill": skill
        }),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn studio_workflows_json(state: &WorkflowStudioState) -> Value {
    let _ = state.workflows.scan_local_dir();
    let workflows = state
        .workflows
        .list_workflow_catalog(None)
        .unwrap_or_default();
    json!({
        "schema": "workflow-studio-workflows/v2",
        "workflows_dir": state.workflows.workflows_dir().to_string_lossy(),
        "selection": state.editor_session.selection(),
        "workflows": workflows
    })
}

fn studio_workflow_detail_json(state: &WorkflowStudioState, id: &str) -> Value {
    match state.workflows.read_workflow_resource(id) {
        Ok(workflow) => {
            state
                .editor_session
                .select(workflow.summary.id.clone(), workflow.summary.revision);
            let blueprint = workflow.blueprint.clone();
            json!({
                "schema": "workflow-studio-workflow-detail/v2",
                "resource": workflow,
                "blueprint": blueprint
            })
        }
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn studio_compile_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let script = input.get("script").and_then(Value::as_str).unwrap_or("");
    if script.trim().is_empty() {
        return json!({"error": "script must not be empty"});
    }
    match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
        script,
        &state.runtime_tools,
    ) {
        Ok(blueprint) => json!({
            "schema": "workflow-studio-compile-result/v1",
            "blueprint": blueprint
        }),
        Err(e) => json!({
            "error": "script compile failed",
            "line": e.line,
            "message": e.message
        }),
    }
}

fn studio_decompile_response(_state: &WorkflowStudioState, body: &[u8]) -> StudioResponse {
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => {
            return StudioResponse::Json(400, json!({"error": format!("invalid JSON body: {e}")}));
        }
    };
    let blueprint_value = match input.get("blueprint").cloned() {
        Some(value) => value,
        None => return StudioResponse::Json(400, json!({"error": "blueprint is required"})),
    };
    let blueprint =
        match corework::workflow::blueprint_json::BlueprintJson::from_json_value(blueprint_value) {
            Ok(value) => value,
            Err(e) => {
                return StudioResponse::Json(
                    400,
                    json!({"error": format!("invalid BlueprintJson: {e}")}),
                );
            }
        };
    let script = match corework::workflow::chain_decompiler::decompile_chain(&blueprint) {
        Ok(value) => value,
        Err(e) => {
            return StudioResponse::Json(
                400,
                json!({"error": "blueprint decompile failed", "message": e.message}),
            );
        }
    };
    if script.len() > 64 * 1024 {
        return StudioResponse::Json(400, json!({"error": "decompiled script is too large"}));
    }

    StudioResponse::Json(
        200,
        json!({
            "schema": "workflow-studio-decompile-result/v1",
            "bytes": script.len(),
            "script": script
        }),
    )
}

fn studio_instantiate_node_json(body: &[u8]) -> Value {
    use corework::workflow::blueprint_json::{BlueprintNodeJson, NodePin, NodePosition, NodeSize};
    use corework::workflow::registry::PinKind;

    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let node_type = input
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let temporary_id = input
        .get("temporary_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if node_type.is_empty() || temporary_id.is_empty() {
        return json!({"error": "node_type and temporary_id are required"});
    }
    let Some(meta) = NodeRegistry::get(node_type) else {
        return json!({"error": format!("node type is not registered: {node_type}")});
    };
    let pins = meta
        .pins
        .iter()
        .map(|pin| NodePin {
            name: pin.name.to_string(),
            kind: match pin.kind {
                PinKind::ExecInput => "ExecInput",
                PinKind::ExecOutput => "ExecOutput",
                PinKind::DataInput => "DataInput",
                PinKind::DataOutput => "DataOutput",
            }
            .to_string(),
            data_type: pin.data_type.to_string(),
            description: pin.description.to_string(),
            default_value: pin
                .default_value
                .and_then(|value| serde_json::from_str(value).ok()),
            resolved_type: None,
            split_config: None,
        })
        .collect::<Vec<_>>();
    let x = input
        .get("position")
        .and_then(|position| position.get("x"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let y = input
        .get("position")
        .and_then(|position| position.get("y"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let node = BlueprintNodeJson {
        id: temporary_id.to_string(),
        node_type: node_type.to_string(),
        position: NodePosition { x, y },
        size: NodeSize::from_pins(&pins),
        pins,
        properties: HashMap::from([("studio".to_string(), json!({"temporary": true}))]),
        display_name: Some(meta.display_name.to_string()),
        comment: None,
    };
    json!({
        "schema": "workflow-studio-node-instance/v1",
        "node": node
    })
}

fn studio_normalize_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    use corework::workflow::blueprint_json::BlueprintJson;

    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let Some(blueprint_value) = input.get("blueprint").cloned() else {
        return json!({"error": "blueprint is required"});
    };
    let candidate = match BlueprintJson::from_json_value(blueprint_value) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid BlueprintJson: {e}")}),
    };
    let temporary_ids = input
        .get("temporary_ids")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let script = match corework::workflow::chain_decompiler::decompile_chain(&candidate) {
        Ok(value) => value,
        Err(e) => {
            return json!({
                "error": "blueprint normalize decompile failed",
                "message": e.message
            });
        }
    };
    let mut normalized =
        match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
            &script,
            &state.runtime_tools,
        ) {
            Ok(value) => value,
            Err(e) => {
                return json!({
                    "error": "blueprint normalize compile failed",
                    "line": e.line,
                    "message": e.message
                });
            }
        };
    normalized.metadata = candidate.metadata.clone();
    normalized.variables = candidate.variables.clone();
    normalized.comments = candidate.comments.clone();

    let old_by_id = candidate
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<HashMap<_, _>>();
    let old_by_step = candidate
        .nodes
        .iter()
        .filter_map(|node| workflow_studio_source_step(node).map(|step| (step, node)))
        .collect::<HashMap<_, _>>();
    let mut used_old_ids = HashSet::new();
    let mut id_remap = HashMap::new();
    for node in &mut normalized.nodes {
        let source = old_by_id.get(&node.id).copied().or_else(|| {
            workflow_studio_source_step(node).and_then(|step| old_by_step.get(&step).copied())
        });
        if let Some(source) = source {
            migrate_workflow_studio_visuals(source, node);
            used_old_ids.insert(source.id.clone());
            id_remap.insert(source.id.clone(), node.id.clone());
        }
    }

    let mut temporary_id_remap = HashMap::new();
    for temporary_id in temporary_ids {
        let Some(source) = old_by_id.get(&temporary_id).copied() else {
            continue;
        };
        let Some(target) = normalized.nodes.iter_mut().find(|node| {
            node.node_type == source.node_type
                && !used_old_ids.contains(&node.id)
                && !id_remap.values().any(|mapped| mapped == &node.id)
        }) else {
            continue;
        };
        migrate_workflow_studio_visuals(source, target);
        temporary_id_remap.insert(temporary_id, target.id.clone());
    }
    json!({
        "schema": "workflow-studio-normalize-result/v1",
        "script": script,
        "blueprint": normalized,
        "id_remap": id_remap,
        "temporary_id_remap": temporary_id_remap
    })
}

fn workflow_studio_source_step(
    node: &corework::workflow::blueprint_json::BlueprintNodeJson,
) -> Option<String> {
    node.properties
        .get("source_script")?
        .get("step")?
        .as_str()
        .map(str::to_string)
}

fn migrate_workflow_studio_visuals(
    source: &corework::workflow::blueprint_json::BlueprintNodeJson,
    target: &mut corework::workflow::blueprint_json::BlueprintNodeJson,
) {
    target.position = source.position.clone();
    target.size = source.size.clone();
    target.display_name = source.display_name.clone().or(target.display_name.clone());
    target.comment = source.comment.clone();
    if let Some(layout) = source.properties.get("layout") {
        target
            .properties
            .insert("layout".to_string(), layout.clone());
    }
}

fn studio_layout_json(body: &[u8]) -> Value {
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let mode = input
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("all")
        .to_string();
    let blueprint_value = match input.get("blueprint").cloned() {
        Some(value) => value,
        None => return json!({"error": "blueprint is required"}),
    };
    let blueprint =
        match corework::workflow::blueprint_json::BlueprintJson::from_json_value(blueprint_value) {
            Ok(value) => value,
            Err(e) => return json!({"error": format!("invalid BlueprintJson: {e}")}),
        };
    let blueprint = layout_workflow_studio_blueprint(blueprint, &mode);
    json!({
        "schema": "workflow-studio-layout-result/v1",
        "algorithm": "dag-layered-pure-groups-v1",
        "mode": mode,
        "blueprint": blueprint
    })
}

fn layout_workflow_studio_blueprint(
    mut blueprint: corework::workflow::blueprint_json::BlueprintJson,
    mode: &str,
) -> corework::workflow::blueprint_json::BlueprintJson {
    use corework::workflow::blueprint_json::{NodePosition, NodeSize};

    const MARGIN_X: f64 = 80.0;
    const MARGIN_Y: f64 = 70.0;
    const GAP_X: f64 = 120.0;
    const GAP_Y: f64 = 46.0;
    const PURE_GAP_X: f64 = 66.0;
    const PURE_GAP_Y: f64 = 20.0;

    let node_count = blueprint.nodes.len();
    let node_index: HashMap<String, usize> = blueprint
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| (node.id.clone(), idx))
        .collect();
    let is_pure: HashMap<String, bool> = blueprint
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.clone(),
                node.pins
                    .iter()
                    .all(|pin| pin.kind != "ExecInput" && pin.kind != "ExecOutput"),
            )
        })
        .collect();

    for node in &mut blueprint.nodes {
        if node.size.width <= 0.0
            || node.size.height <= 0.0
            || layout_property_string(node, "size_source").as_deref() != Some("user")
        {
            node.size = estimate_studio_node_size(node);
            set_layout_property(node, "size_source", "auto");
        }
    }

    let exec_edges: Vec<(String, String)> = blueprint
        .connections
        .iter()
        .filter(|conn| conn.connection_type == "Exec")
        .map(|conn| (conn.source_node.clone(), conn.target_node.clone()))
        .collect();
    let data_edges: Vec<(String, String)> = blueprint
        .connections
        .iter()
        .filter(|conn| conn.connection_type != "Exec")
        .map(|conn| (conn.source_node.clone(), conn.target_node.clone()))
        .collect();

    let mut layer: HashMap<String, i32> = HashMap::new();
    for node in &blueprint.nodes {
        if node.node_type == "StartNode" || !is_pure.get(&node.id).copied().unwrap_or(false) {
            layer.insert(node.id.clone(), 0);
        }
    }
    for _ in 0..node_count.max(1) {
        let mut changed = false;
        for (source, target) in &exec_edges {
            let Some(source_layer) = layer.get(source).copied() else {
                continue;
            };
            let next_layer = source_layer + 1;
            if next_layer > layer.get(target).copied().unwrap_or(0) {
                layer.insert(target.clone(), next_layer);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let last_exec_layer = layer.values().copied().max().unwrap_or(0);
    for node in &blueprint.nodes {
        if node.node_type == "EndNode" {
            layer.insert(node.id.clone(), last_exec_layer.max(1));
        }
    }

    let mut pure_min_layer: HashMap<String, i32> = HashMap::new();
    let mut pure_max_layer: HashMap<String, i32> = HashMap::new();
    for (source, target) in &data_edges {
        let source_is_pure = is_pure.get(source).copied().unwrap_or(false);
        let target_is_pure = is_pure.get(target).copied().unwrap_or(false);
        if !source_is_pure && target_is_pure {
            let producer_layer = layer.get(source).copied().unwrap_or(0);
            pure_min_layer
                .entry(target.clone())
                .and_modify(|value| *value = (*value).max(producer_layer + 1))
                .or_insert(producer_layer + 1);
        }
    }
    for _ in 0..node_count.max(1) {
        let mut changed = false;
        for (source, target) in &data_edges {
            let source_is_pure = is_pure.get(source).copied().unwrap_or(false);
            if !source_is_pure {
                continue;
            }
            let target_layer = if is_pure.get(target).copied().unwrap_or(false) {
                pure_max_layer
                    .get(target)
                    .copied()
                    .or_else(|| layer.get(target).copied())
            } else {
                layer.get(target).copied()
            };
            if let Some(target_layer) = target_layer {
                let next_max = (target_layer - 1).max(0);
                if next_max < pure_max_layer.get(source).copied().unwrap_or(i32::MAX) {
                    pure_max_layer.insert(source.clone(), next_max);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for node in &blueprint.nodes {
        if is_pure.get(&node.id).copied().unwrap_or(false) {
            let min_layer = pure_min_layer.get(&node.id).copied().unwrap_or(0);
            let max_layer = pure_max_layer
                .get(&node.id)
                .copied()
                .unwrap_or(last_exec_layer.saturating_sub(1));
            let assigned_layer = if max_layer >= min_layer {
                max_layer
            } else {
                min_layer
            };
            layer.insert(node.id.clone(), assigned_layer.max(0));
        }
    }

    let mut layers: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for node in &blueprint.nodes {
        layers
            .entry(layer.get(&node.id).copied().unwrap_or(0))
            .or_default()
            .push(node.id.clone());
    }
    for ids in layers.values_mut() {
        ids.sort_by(|a, b| {
            let a_node = &blueprint.nodes[node_index[a]];
            let b_node = &blueprint.nodes[node_index[b]];
            (
                is_pure.get(a).copied().unwrap_or(false),
                source_step_sort_key(a_node),
                node_index[a],
            )
                .cmp(&(
                    is_pure.get(b).copied().unwrap_or(false),
                    source_step_sort_key(b_node),
                    node_index[b],
                ))
        });
    }

    let mut layer_x = BTreeMap::new();
    let mut cursor_x = MARGIN_X;
    for (layer_id, ids) in &layers {
        layer_x.insert(*layer_id, cursor_x);
        let max_width = ids
            .iter()
            .filter_map(|id| node_index.get(id))
            .map(|idx| blueprint.nodes[*idx].size.width)
            .fold(240.0, f64::max);
        cursor_x += max_width + GAP_X;
    }

    let mut positions: HashMap<String, NodePosition> = HashMap::new();
    for (layer_id, ids) in &layers {
        let x = layer_x.get(layer_id).copied().unwrap_or(MARGIN_X);
        let mut y = MARGIN_Y;
        for id in ids {
            let idx = node_index[id];
            if is_pure.get(id).copied().unwrap_or(false) {
                continue;
            }
            positions.insert(id.clone(), NodePosition { x, y });
            y += blueprint.nodes[idx].size.height + GAP_Y;
        }
    }

    let mut pure_children: HashMap<String, Vec<String>> = HashMap::new();
    let mut pure_consumers: HashMap<String, Vec<String>> = HashMap::new();
    for (source, target) in &data_edges {
        if is_pure.get(source).copied().unwrap_or(false)
            && is_pure.get(target).copied().unwrap_or(false)
        {
            pure_children
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
        if is_pure.get(source).copied().unwrap_or(false) {
            pure_consumers
                .entry(source.clone())
                .or_default()
                .push(target.clone());
        }
    }

    let mut placed_pure = HashSet::new();
    let mut consumers: Vec<String> = blueprint
        .nodes
        .iter()
        .filter(|node| !is_pure.get(&node.id).copied().unwrap_or(false))
        .map(|node| node.id.clone())
        .collect();
    consumers.sort_by_key(|id| (layer.get(id).copied().unwrap_or(0), node_index[id]));
    for consumer_id in consumers {
        let Some(consumer_position) = positions.get(&consumer_id).cloned() else {
            continue;
        };
        let roots: Vec<String> = data_edges
            .iter()
            .filter(|(_source, target)| target == &consumer_id)
            .map(|(source, _target)| source.clone())
            .filter(|source| is_pure.get(source).copied().unwrap_or(false))
            .collect();
        let mut y = consumer_position.y;
        for root in roots {
            layout_pure_expression_tree(
                &root,
                1,
                &mut y,
                &mut positions,
                &mut placed_pure,
                &pure_children,
                &blueprint,
                &node_index,
                &consumer_position,
                PURE_GAP_X,
                PURE_GAP_Y,
            );
        }
    }
    for node in &blueprint.nodes {
        if !is_pure.get(&node.id).copied().unwrap_or(false) || placed_pure.contains(&node.id) {
            continue;
        }
        let consumers = pure_consumers.get(&node.id).cloned().unwrap_or_default();
        let avg_y = if consumers.is_empty() {
            MARGIN_Y
        } else {
            let total = consumers
                .iter()
                .filter_map(|id| positions.get(id))
                .map(|position| position.y)
                .sum::<f64>();
            total / consumers.len() as f64
        };
        let node_layer = layer.get(&node.id).copied().unwrap_or(0);
        positions.insert(
            node.id.clone(),
            NodePosition {
                x: layer_x.get(&node_layer).copied().unwrap_or(MARGIN_X),
                y: avg_y,
            },
        );
    }

    let mut by_layer: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for node in &blueprint.nodes {
        by_layer
            .entry(layer.get(&node.id).copied().unwrap_or(0))
            .or_default()
            .push(node.id.clone());
    }
    for ids in by_layer.values_mut() {
        ids.sort_by(|a, b| {
            positions
                .get(a)
                .map(|p| p.y)
                .unwrap_or(MARGIN_Y)
                .partial_cmp(&positions.get(b).map(|p| p.y).unwrap_or(MARGIN_Y))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut next_y = MARGIN_Y;
        for id in ids {
            let idx = node_index[id];
            let position = positions.entry(id.clone()).or_insert(NodePosition {
                x: MARGIN_X,
                y: next_y,
            });
            if position.y < next_y {
                position.y = next_y;
            }
            next_y = position.y + blueprint.nodes[idx].size.height + GAP_Y.min(PURE_GAP_Y + 18.0);
        }
    }

    let min_x = positions
        .values()
        .map(|p| p.x)
        .fold(MARGIN_X, f64::min)
        .min(MARGIN_X);
    let min_y = positions
        .values()
        .map(|p| p.y)
        .fold(MARGIN_Y, f64::min)
        .min(MARGIN_Y);
    let shift_x = if min_x < MARGIN_X {
        MARGIN_X - min_x
    } else {
        0.0
    };
    let shift_y = if min_y < MARGIN_Y {
        MARGIN_Y - min_y
    } else {
        0.0
    };

    for node in &mut blueprint.nodes {
        let should_layout = mode == "all"
            || layout_property_string(node, "position_source").as_deref() != Some("user")
            || node.position.x == 0.0 && node.position.y == 0.0;
        if !should_layout {
            continue;
        }
        if let Some(position) = positions.get(&node.id) {
            node.position = NodePosition {
                x: position.x + shift_x,
                y: position.y + shift_y,
            };
            set_layout_property(node, "position_source", "auto");
            set_layout_property(node, "algorithm", "dag-layered-pure-groups-v1");
        }
        if node.size.width <= 0.0 || node.size.height <= 0.0 {
            node.size = NodeSize::from_pins(&node.pins);
        }
    }

    blueprint
}

fn layout_pure_expression_tree(
    node_id: &str,
    depth: usize,
    cursor_y: &mut f64,
    positions: &mut HashMap<String, corework::workflow::blueprint_json::NodePosition>,
    placed: &mut HashSet<String>,
    pure_children: &HashMap<String, Vec<String>>,
    blueprint: &corework::workflow::blueprint_json::BlueprintJson,
    node_index: &HashMap<String, usize>,
    consumer_position: &corework::workflow::blueprint_json::NodePosition,
    gap_x: f64,
    gap_y: f64,
) {
    if !placed.insert(node_id.to_string()) {
        return;
    }
    let Some(idx) = node_index.get(node_id).copied() else {
        return;
    };
    let node = &blueprint.nodes[idx];
    if let Some(children) = pure_children.get(node_id) {
        let mut sorted = children.clone();
        sorted.sort_by_key(|id| node_index.get(id).copied().unwrap_or(usize::MAX));
        for child in sorted {
            layout_pure_expression_tree(
                &child,
                depth + 1,
                cursor_y,
                positions,
                placed,
                pure_children,
                blueprint,
                node_index,
                consumer_position,
                gap_x,
                gap_y,
            );
        }
    }
    positions.insert(
        node_id.to_string(),
        corework::workflow::blueprint_json::NodePosition {
            x: consumer_position.x - depth as f64 * (node.size.width + gap_x),
            y: *cursor_y,
        },
    );
    *cursor_y += node.size.height + gap_y;
}

fn estimate_studio_node_size(
    node: &corework::workflow::blueprint_json::BlueprintNodeJson,
) -> corework::workflow::blueprint_json::NodeSize {
    let is_pure = node
        .pins
        .iter()
        .all(|pin| pin.kind != "ExecInput" && pin.kind != "ExecOutput");
    let input_pins: Vec<_> = node
        .pins
        .iter()
        .filter(|pin| pin.kind == "ExecInput" || pin.kind == "DataInput")
        .collect();
    let output_pins: Vec<_> = node
        .pins
        .iter()
        .filter(|pin| pin.kind == "ExecOutput" || pin.kind == "DataOutput")
        .collect();
    let max_label = node
        .pins
        .iter()
        .map(|pin| pin.name.len() + pin.data_type.len().min(14))
        .chain(std::iter::once(
            node.display_name
                .as_deref()
                .unwrap_or(&node.node_type)
                .len(),
        ))
        .max()
        .unwrap_or(12)
        .min(30) as f64;
    let base_width: f64 = if is_pure {
        210.0
    } else if node.node_type.contains("Rpc") || node.node_type.contains("Tool") {
        320.0
    } else if node.node_type.contains("Branch")
        || node.node_type.contains("Loop")
        || node.node_type.contains("Start")
        || node.node_type.contains("End")
    {
        250.0
    } else {
        285.0
    };
    let width = base_width.max(150.0 + max_label * 7.0).min(380.0);
    let left_height: f64 = input_pins
        .iter()
        .map(|pin| estimate_pin_row_height(pin))
        .sum();
    let right_height: f64 = output_pins
        .iter()
        .map(|pin| estimate_pin_row_height(pin))
        .sum();
    let height = (42.0 + left_height.max(right_height) + 16.0).max(76.0);
    corework::workflow::blueprint_json::NodeSize { width, height }
}

fn estimate_pin_row_height(pin: &corework::workflow::blueprint_json::NodePin) -> f64 {
    let preview_len = pin
        .default_value
        .as_ref()
        .map(|value| match value {
            Value::String(text) => text.len(),
            other => other.to_string().len(),
        })
        .unwrap_or(0);
    let extra_lines = if preview_len <= 24 {
        0
    } else {
        ((preview_len - 24) / 36 + 1).min(2)
    };
    24.0 + extra_lines as f64 * 18.0
}

fn layout_property_string(
    node: &corework::workflow::blueprint_json::BlueprintNodeJson,
    key: &str,
) -> Option<String> {
    node.properties
        .get("layout")
        .and_then(Value::as_object)
        .and_then(|layout| layout.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn set_layout_property(
    node: &mut corework::workflow::blueprint_json::BlueprintNodeJson,
    key: &str,
    value: &str,
) {
    let mut layout = node
        .properties
        .get("layout")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    layout.insert(key.to_string(), Value::String(value.to_string()));
    node.properties
        .insert("layout".to_string(), Value::Object(layout));
}

fn source_step_sort_key(node: &corework::workflow::blueprint_json::BlueprintNodeJson) -> String {
    node.properties
        .get("source_script")
        .and_then(Value::as_object)
        .and_then(|source| source.get("step"))
        .and_then(Value::as_str)
        .map(|step| {
            step.split('.')
                .map(|part| format!("{:0>6}", part))
                .collect::<Vec<_>>()
                .join(".")
        })
        .unwrap_or_else(|| "999999".to_string())
}

fn studio_save_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    if state.readonly {
        return json!({"error": "Workflow Studio session is readonly"});
    }
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let blueprint_value = input
        .get("blueprint")
        .cloned()
        .unwrap_or_else(|| input.clone());
    let requested_workflow_id = input
        .get("workflow_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let expected_revision = input.get("expected_revision").and_then(Value::as_u64);
    let blueprint =
        match corework::workflow::blueprint_json::BlueprintJson::from_json_value(blueprint_value) {
            Ok(value) => value,
            Err(e) => return json!({"error": format!("invalid BlueprintJson: {e}")}),
        };
    let script = match corework::workflow::chain_decompiler::decompile_chain(&blueprint) {
        Ok(script) => script,
        Err(error) => return json!({"error": error.to_string()}),
    };
    let workflows = Arc::clone(&state.workflows);
    let session = Arc::clone(&state.editor_session);
    let selection = session.selection();
    let result = run_short_tokio(async move {
        match selection {
            Some(selection) => {
                if requested_workflow_id
                    .as_deref()
                    .is_some_and(|requested| requested != selection.workflow_id.as_str())
                {
                    return Err(corework::error::FrameworkError::InvalidOperation(format!(
                        "selected Workflow '{}' does not match requested '{}'",
                        selection.workflow_id,
                        requested_workflow_id.as_deref().unwrap_or_default()
                    )));
                }
                let current = workflows.read_workflow_resource(&selection.workflow_id)?;
                let expected_revision = expected_revision.or(Some(selection.revision));
                let mut blueprint = blueprint;
                blueprint.metadata.id = current.summary.id.clone();
                blueprint.metadata.name = current.summary.name.clone();
                blueprint.metadata.description = current.summary.description.clone();
                match current.summary.kind {
                    corework::workflow::workflows::WorkflowResourceKind::Draft => {
                        workflows
                            .update_draft_resource(
                                &current.summary.id,
                                expected_revision,
                                &current.summary.name,
                                &current.summary.description,
                                Some(script),
                                Some(blueprint),
                                corework::workflow::workflows::WorkflowValidation::valid(),
                            )
                            .await
                    }
                    corework::workflow::workflows::WorkflowResourceKind::Registered => {
                        workflows
                            .update_registered_resource(&blueprint, expected_revision)
                            .await
                    }
                }
            }
            None => {
                let requested_id = (!blueprint.metadata.id.trim().is_empty())
                    .then(|| blueprint.metadata.id.clone());
                let name = blueprint.metadata.name.clone();
                let description = blueprint.metadata.description.clone();
                workflows
                    .create_draft_resource(
                        requested_id.as_deref(),
                        &name,
                        &description,
                        Some(script),
                        Some(blueprint),
                        corework::workflow::workflows::WorkflowValidation::valid(),
                    )
                    .await
            }
        }
    });
    match result {
        Ok(workflow) => {
            state
                .editor_session
                .select(workflow.summary.id.clone(), workflow.summary.revision);
            let _ = studio_state_snapshot_response(state, b"{}");
            json!({
                "schema": "workflow-studio-save-result/v2",
                "resource": workflow
            })
        }
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn studio_run_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let inputs = input
        .get("inputs")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<HashMap<String, Value>>()
        })
        .unwrap_or_default();
    let trace = input.get("trace").and_then(Value::as_bool).unwrap_or(true);
    let selection = match state.editor_session.selection() {
        Some(selection) => selection,
        None => {
            publish_studio_execution_event(
                state,
                json!({
                    "source": "workflow_studio",
                    "status": "failed",
                    "code": 400,
                    "duration_ms": 0,
                    "error": "no Workflow resource is selected"
                }),
            );
            return json!({
                "error": "no Workflow resource is selected; create or open and save a resource before running"
            });
        }
    };
    let workflows = Arc::clone(&state.workflows);
    let workflow_id = selection.workflow_id.clone();
    let execution_context = WorkflowExecutionContext::new(
        state.editor_conversation_id.clone(),
        state.editor_agent_id.clone(),
    );
    let started = std::time::Instant::now();
    let result = run_short_tokio(async move {
        let resource = workflows.read_workflow_resource(&workflow_id)?;
        let outcome = match resource.summary.kind {
            corework::workflow::workflows::WorkflowResourceKind::Draft => {
                let blueprint = workflows.draft_blueprint(&workflow_id)?;
                workflows
                    .execute_from_blueprint_outcome_with_context(
                        blueprint,
                        inputs,
                        &execution_context,
                    )
                    .await?
            }
            corework::workflow::workflows::WorkflowResourceKind::Registered => {
                workflows
                    .execute_registered_outcome_with_context(
                        &workflow_id,
                        inputs,
                        &execution_context,
                    )
                    .await?
            }
        };
        Ok((resource.summary, outcome))
    });
    match result {
        Ok((summary, outcome)) => {
            let duration_ms = started.elapsed().as_millis();
            let execution_error = outcome.error.clone();
            let event_workflow_id = summary.id.clone();
            let event_revision = summary.revision;
            publish_studio_execution_event(
                state,
                json!({
                    "source": "workflow_studio",
                    "workflow_id": event_workflow_id,
                    "revision": event_revision,
                    "status": if execution_error.is_none() { "succeeded" } else { "failed" },
                    "code": if execution_error.is_none() { 0 } else { -1 },
                    "duration_ms": duration_ms,
                    "error": execution_error
                }),
            );
            let outputs = outcome
                .report
                .outputs
                .into_iter()
                .map(|(key, value)| (key, value.json_value().clone()))
                .collect::<serde_json::Map<String, Value>>();
            json!({
                "schema": "workflow-studio-run-result/v2",
                "workflow_id": summary.id,
                "revision": summary.revision,
                "code": if outcome.error.is_none() { 0 } else { -1 },
                "error": outcome.error,
                "outputs": outputs,
                "trace": if trace { outcome.report.trace } else { None }
            })
        }
        Err(e) => {
            publish_studio_execution_event(
                state,
                json!({
                    "source": "workflow_studio",
                    "workflow_id": selection.workflow_id,
                    "revision": selection.revision,
                    "status": "failed",
                    "code": 400,
                    "duration_ms": started.elapsed().as_millis(),
                    "error": e.to_string()
                }),
            );
            json!({"error": e.to_string()})
        }
    }
}

fn publish_studio_execution_event(state: &WorkflowStudioState, payload: Value) {
    let workflows = Arc::clone(&state.workflows);
    let _ = run_short_tokio(async move {
        workflows.publish_execution_event(payload).await;
        Ok(())
    });
}

fn studio_skill_refs_search_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty() {
        return json!({"error": "query must not be empty"});
    }
    let context_paragraphs = input
        .get("context_paragraphs")
        .and_then(Value::as_u64)
        .unwrap_or(2)
        .min(5) as usize;
    let max_results = input
        .get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .min(32) as usize;

    match load_parent_visible_skills(state) {
        Ok(skills) => {
            let matches =
                search_skill_refs_in_skills(&skills, &query, context_paragraphs, max_results);
            json!({
                "schema": "workflow-studio-skill-ref-search-result/v1",
                "query": query,
                "matches": matches
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn studio_skill_patch_propose_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let workflow_name = input
        .get("workflow_name")
        .and_then(Value::as_str)
        .or_else(|| {
            input
                .get("blueprint")
                .and_then(|bp| bp.get("metadata"))
                .and_then(|meta| meta.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or(state.workflow_name.as_str())
        .trim();
    if workflow_name.is_empty() {
        return json!({"error": "workflow_name is required"});
    }
    let workflow_id = input
        .get("workflow_id")
        .and_then(Value::as_str)
        .or_else(|| {
            input
                .get("blueprint")
                .and_then(|blueprint| blueprint.get("metadata"))
                .and_then(|metadata| metadata.get("id"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(workflow_name);
    let skill_name = input
        .get("skill_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let Some(skill_name) = skill_name else {
        return json!({"error": "skill_name is required"});
    };

    let inputs = input
        .get("inputs")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            input
                .get("blueprint")
                .and_then(|bp| bp.get("metadata"))
                .and_then(|meta| meta.get("inputs"))
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default();
    let input_names: Vec<String> = inputs
        .iter()
        .filter_map(|value| {
            value.as_str().map(str::to_string).or_else(|| {
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .collect();
    let section_markdown = build_workflow_skill_section(workflow_name, workflow_id, &input_names);
    json!({
        "schema": "workflow-studio-skill-patch-proposal/v1",
        "skill_name": skill_name,
        "mode": "append",
        "section_markdown": section_markdown,
        "diff_preview": format!("--- SKILL.md\n+++ SKILL.md\n@@ append\n{}", section_markdown.lines().map(|line| format!("+{line}")).collect::<Vec<_>>().join("\n"))
    })
}

fn studio_skill_patch_apply_json(state: &WorkflowStudioState, body: &[u8]) -> Value {
    if state.readonly {
        return json!({"error": "Workflow Studio session is readonly"});
    }
    let input: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return json!({"error": format!("invalid JSON body: {e}")}),
    };
    let skill_name = input
        .get("skill_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let section_markdown = input
        .get("section_markdown")
        .or_else(|| input.get("patch_text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if skill_name.is_empty() {
        return json!({"error": "skill_name is required"});
    }
    if section_markdown.is_empty() {
        return json!({"error": "section_markdown is required"});
    }
    match apply_skill_append_patch(state, &skill_name, &section_markdown) {
        Ok(path) => json!({
            "schema": "workflow-studio-skill-patch-apply-result/v1",
            "applied": true,
            "skill_name": skill_name,
            "path": path.to_string_lossy()
        }),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn load_parent_visible_skills(
    state: &WorkflowStudioState,
) -> Result<Vec<ai_assistant::Skill>, RuntimeError> {
    let Some(skills_dir) = state.config.runtime.skills_dir.clone() else {
        return Err(RuntimeError::InvalidConfig(
            "runtime.skills_dir is not configured".to_string(),
        ));
    };
    run_short_tokio(async move {
        let mut manager = SkillManager::from_directory(&skills_dir).await?;
        let mut names = manager
            .all_metadata()
            .into_iter()
            .filter(|meta| !meta.system_layer)
            .map(|meta| meta.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        let mut skills = Vec::new();
        for name in names {
            if let Ok(skill) = manager.load(&name).await {
                skills.push(skill.clone());
            }
        }
        Ok(skills)
    })
}

fn search_skill_refs_in_skills(
    skills: &[ai_assistant::Skill],
    query: &str,
    context_paragraphs: usize,
    max_results: usize,
) -> Vec<Value> {
    let needle = query.to_ascii_lowercase();
    let mut matches = Vec::new();
    for skill in skills {
        if matches.len() >= max_results {
            break;
        }
        let metadata_text = serde_json::to_string(&skill.metadata).unwrap_or_default();
        let metadata_matched = metadata_text.to_ascii_lowercase().contains(&needle);
        if !metadata_matched {
            continue;
        }
        let paragraphs = split_skill_paragraphs(&skill.instructions);
        let mut body_matched = false;
        for (index, paragraph) in paragraphs.iter().enumerate() {
            if matches.len() >= max_results {
                break;
            }
            if !paragraph.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            body_matched = true;
            matches.push(json!({
                "skill": skill.metadata.name,
                "path": skill_path_string(skill),
                "metadata_matched": true,
                "body_matched": true,
                "heading": nearest_heading(&paragraphs, index),
                "paragraph": paragraph,
                "before": context_paragraph_text(&paragraphs, index, context_paragraphs, true),
                "after": context_paragraph_text(&paragraphs, index, context_paragraphs, false),
                "note": null
            }));
        }
        if !body_matched && matches.len() < max_results {
            matches.push(json!({
                "skill": skill.metadata.name,
                "path": skill_path_string(skill),
                "metadata_matched": true,
                "body_matched": false,
                "heading": null,
                "paragraph": "",
                "before": "",
                "after": "",
                "note": "skill metadata references the query, but body has no explicit paragraph match"
            }));
        }
    }
    matches
}

fn split_skill_paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(str::to_string)
        .collect()
}

fn nearest_heading(paragraphs: &[String], index: usize) -> Option<String> {
    paragraphs
        .iter()
        .take(index + 1)
        .rev()
        .find(|paragraph| paragraph.trim_start().starts_with('#'))
        .cloned()
}

fn context_paragraph_text(
    paragraphs: &[String],
    index: usize,
    count: usize,
    before: bool,
) -> String {
    if count == 0 {
        return String::new();
    }
    if before {
        let start = index.saturating_sub(count);
        paragraphs[start..index].join("\n\n")
    } else {
        let end = (index + 1 + count).min(paragraphs.len());
        paragraphs[index + 1..end].join("\n\n")
    }
}

fn skill_path_string(skill: &ai_assistant::Skill) -> String {
    skill
        .base_path
        .as_ref()
        .map(|path| path.join("SKILL.md").to_string_lossy().to_string())
        .unwrap_or_default()
}

fn build_workflow_skill_section(
    workflow_name: &str,
    workflow_id: &str,
    inputs: &[String],
) -> String {
    let mut section = String::new();
    section.push_str(&format!("\n\n## Workflow: {workflow_name}\n\n"));
    section.push_str(
        "When the user needs this workflow, execute the Registered resource by stable id.\n\n",
    );
    section.push_str(&format!("- Workflow id: `{workflow_id}`\n"));
    section.push_str("- Execution tool: `executeWorkflow`\n");
    if !inputs.is_empty() {
        section.push_str("- Inputs:\n");
        for input in inputs {
            section.push_str(&format!("  - `--input.{input}`\n"));
        }
    }
    section.push_str("\nExample:\n\n```text\n");
    section.push_str(&format!(
        "EXEC executeWorkflow --workflow_id \"{workflow_id}\""
    ));
    for input in inputs {
        section.push_str(&format!(" --input.{input} \"...\""));
    }
    section.push_str("\n```\n");
    section
}

fn apply_skill_append_patch(
    state: &WorkflowStudioState,
    skill_name: &str,
    section_markdown: &str,
) -> Result<PathBuf, RuntimeError> {
    let skills = load_parent_visible_skills(state)?;
    let skill = skills
        .into_iter()
        .find(|skill| skill.metadata.name == skill_name)
        .ok_or_else(|| {
            RuntimeError::InvalidConfig(format!(
                "skill '{skill_name}' is not visible to parent agent"
            ))
        })?;
    let path = skill
        .base_path
        .ok_or_else(|| RuntimeError::InvalidConfig("skill has no base_path".to_string()))?
        .join("SKILL.md");
    let mut content = fs::read_to_string(&path)
        .map_err(|e| RuntimeError::Internal(format!("read skill file failed: {e}")))?;
    if content.contains(section_markdown.trim()) {
        return Ok(path);
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(section_markdown.trim_start());
    content.push('\n');
    fs::write(&path, content)
        .map_err(|e| RuntimeError::Internal(format!("write skill file failed: {e}")))?;
    Ok(path)
}

fn run_short_tokio<F, T>(future: F) -> Result<T, RuntimeError>
where
    F: std::future::Future<Output = corework::error::Result<T>>,
{
    let rt = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            RuntimeError::Internal(format!("create Studio request runtime failed: {e}"))
        })?;
    rt.block_on(future)
        .map_err(|e| RuntimeError::Internal(e.to_string()))
}

fn write_json_response(stream: &mut TcpStream, status: u16, value: Value) -> std::io::Result<()> {
    let body = if status == 204 {
        String::new()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    };
    write_http_response(stream, status, "application/json; charset=utf-8", body)
}

fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: String,
) -> std::io::Result<()> {
    write_http_bytes_response(stream, status, content_type, body.as_bytes())
}

fn write_http_bytes_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        501 => "Not Implemented",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: http://127.0.0.1\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn workflow_studio_static_asset(path: &str) -> Option<StudioResponse> {
    match path {
        "/index.html" => Some(StudioResponse::Static {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: include_bytes!("../frontend/workflow-studio/dist/index.html"),
        }),
        "/assets/index.js" => Some(StudioResponse::Static {
            status: 200,
            content_type: "text/javascript; charset=utf-8",
            body: include_bytes!("../frontend/workflow-studio/dist/assets/index.js"),
        }),
        "/assets/index.css" => Some(StudioResponse::Static {
            status: 200,
            content_type: "text/css; charset=utf-8",
            body: include_bytes!("../frontend/workflow-studio/dist/assets/index.css"),
        }),
        _ => None,
    }
}

pub(crate) fn workflow_studio_html() -> String {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Workflow Studio</title>
  <style>
    :root {
      color-scheme: light;
      font-family: "Aptos", "Segoe UI Variable", system-ui, sans-serif;
      --bg: #f4f0e6; --panel: #fffdf8; --panel-2: #f7f2e8; --line: #d7cec0;
      --line-soft: rgba(83, 103, 98, .16); --text: #263a37; --muted: #6b7d78;
      --faint: #91a09a; --accent: #167d71; --accent-2: #b67522; --bad: #b84e48;
      --ok: #2f8561; --canvas: #fbf8f0; --shadow: 0 18px 48px rgba(81, 69, 48, .16);
    }
    * { box-sizing: border-box; }
    body { margin: 0; min-width: 0; background: var(--bg); color: var(--text); }
    body::before { content: ""; position: fixed; inset: 0; pointer-events: none; background: radial-gradient(circle at 18% -10%, rgba(22,125,113,.10), transparent 30rem), radial-gradient(circle at 92% 16%, rgba(182,117,34,.08), transparent 24rem); }
    button, input, textarea { font: inherit; }
    button { cursor: pointer; }
    button:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .topbar { position: relative; display: flex; height: 64px; align-items: center; justify-content: space-between; gap: 18px; border-bottom: 1px solid var(--line); background: rgba(255,253,248,.88); padding: 0 18px; backdrop-filter: blur(18px); }
    .brand { display: flex; align-items: center; gap: 12px; }
    .brand-mark { display: grid; width: 34px; height: 34px; place-items: center; border: 1px solid rgba(101,215,199,.5); border-radius: 10px; color: var(--accent); background: rgba(101,215,199,.08); font: 700 13px ui-monospace, Consolas, monospace; }
    h1 { margin: 0; font-size: 16px; letter-spacing: .04em; }
    .eyebrow { margin: 2px 0 0; color: var(--muted); font: 11px ui-monospace, Consolas, monospace; letter-spacing: .08em; text-transform: uppercase; }
    .top-actions, .toolbar, .tabs, .status-row { display: flex; align-items: center; gap: 8px; }
    .pill { border: 1px solid var(--line); border-radius: 999px; padding: 5px 9px; color: var(--muted); background: rgba(255,253,248,.82); font-size: 11px; }
    .pill.ok { border-color: rgba(125,211,157,.35); color: var(--ok); }
    .shell { position: relative; display: grid; grid-template-columns: 268px minmax(0, 1fr) 342px; height: calc(100vh - 64px); }
    .rail, .assistant { min-width: 0; overflow: hidden; background: rgba(255,253,248,.94); }
    .rail { display: grid; grid-template-rows: auto minmax(0, 1fr); border-right: 1px solid var(--line); }
    .assistant { display: grid; grid-template-rows: auto minmax(0, 1fr) auto auto; border-left: 1px solid var(--line); }
    .rail-head, .assistant-head, .pane-head { display: flex; min-height: 48px; align-items: center; justify-content: space-between; gap: 8px; border-bottom: 1px solid var(--line-soft); padding: 10px 12px; }
    h2, h3 { margin: 0; font-size: 12px; letter-spacing: .08em; text-transform: uppercase; }
    .rail-body, .message-list { min-height: 0; overflow: auto; padding: 10px; }
    .search { width: 100%; border: 1px solid var(--line); border-radius: 8px; background: #fffefa; color: var(--text); padding: 9px 10px; font-size: 12px; }
    .section-label { margin: 15px 2px 7px; color: var(--faint); font: 700 10px ui-monospace, Consolas, monospace; letter-spacing: .13em; text-transform: uppercase; }
    .group { margin-top: 8px; }
    .group-title, .subgroup-title { display: flex; cursor: pointer; list-style: none; align-items: center; justify-content: space-between; gap: 8px; border-radius: 7px; transition: 150ms ease; }
    .group-title::-webkit-details-marker, .subgroup-title::-webkit-details-marker { display: none; }
    .group-title { margin: 0; padding: 7px 6px; color: var(--accent); font: 700 11px ui-monospace, Consolas, monospace; letter-spacing: .08em; text-transform: uppercase; }
    .subgroup-title { margin: 2px 0; padding: 5px 6px; color: var(--muted); font: 700 10px ui-monospace, Consolas, monospace; letter-spacing: .07em; text-transform: uppercase; }
    .group-title:hover, .subgroup-title:hover { background: rgba(22,125,113,.07); }
    .tree-label::before { display: inline-block; width: 14px; color: var(--faint); content: "›"; font-size: 16px; line-height: 10px; transform: rotate(0); transition: transform 150ms ease; }
    details[open] > summary .tree-label::before { transform: rotate(90deg); }
    .group-count { color: var(--faint); font-size: 10px; }
    .subgroup { margin-left: 10px; }
    .subgroup-items { margin-left: 10px; border-left: 1px solid var(--line-soft); padding-left: 4px; }
    .item { display: block; width: 100%; border: 1px solid transparent; border-radius: 8px; background: transparent; color: var(--text); padding: 9px; text-align: left; transition: 160ms ease; }
    .item + .item { margin-top: 3px; }
    .item:hover, .item.active { border-color: rgba(101,215,199,.26); background: rgba(101,215,199,.07); }
    .item-title { display: block; overflow: hidden; color: #2a403d; font-size: 12px; font-weight: 650; text-overflow: ellipsis; white-space: nowrap; }
    .item-meta { display: block; margin-top: 4px; color: var(--muted); font: 10px ui-monospace, Consolas, monospace; }
    .workbench { display: grid; min-width: 0; grid-template-rows: 48px minmax(0, 1fr) 214px; background: var(--canvas); }
    .pane-head { background: rgba(255,253,248,.94); }
    .button { min-height: 32px; border: 1px solid var(--line); border-radius: 7px; background: rgba(255,253,248,.94); color: var(--muted); padding: 6px 10px; font-size: 12px; font-weight: 650; transition: 150ms ease; }
    .button:hover { border-color: rgba(101,215,199,.5); color: var(--text); }
    .button.primary { border-color: rgba(22,125,113,.7); background: var(--accent); color: #f7fffd; }
    .button:disabled { cursor: not-allowed; opacity: .45; }
    .tab { border: 0; border-radius: 6px; background: transparent; color: var(--muted); padding: 6px 8px; font-size: 12px; }
    .tab.active { background: rgba(101,215,199,.11); color: var(--accent); }
    .canvas-wrap { position: relative; overflow: hidden; background-color: var(--canvas); background-image: radial-gradient(rgba(112,145,158,.23) 1px, transparent 1px); background-size: 22px 22px; }
    #graph { width: 100%; height: 100%; }
    .empty-canvas { position: absolute; inset: 0; display: grid; place-items: center; color: var(--faint); text-align: center; pointer-events: none; }
    .empty-canvas b { display: block; margin-bottom: 7px; color: var(--muted); font-size: 14px; }
    .node { cursor: grab; touch-action: none; }
    .node.dragging { cursor: grabbing; }
    .node rect { fill: #fffefa; stroke: #86a39d; stroke-width: 1.2; }
    .node.rpc rect { stroke: #c4924d; }
    .node.selected rect, .node.dragging rect { stroke: var(--accent); stroke-width: 2; filter: drop-shadow(0 6px 14px rgba(101,215,199,.2)); }
    .node text { fill: #263a37; font-size: 12px; pointer-events: none; }
    .node .node-sub { fill: #718781; font: 10px ui-monospace, Consolas, monospace; }
    .node .node-head { fill: rgba(101,215,199,.08); stroke: none; }
    .node .pin { fill: #617b76; font: 10px ui-monospace, Consolas, monospace; }
    .node .pin.exec { fill: var(--accent); }
    .node .pin.data { fill: #9b6d2d; }
    .node .pin-hotspot { cursor: crosshair; fill: transparent; pointer-events: all; }
    .node .pin-text { fill: #445a56; font: 10px ui-monospace, Consolas, monospace; }
    .node .pin-preview { fill: #81928e; font: 9px ui-monospace, Consolas, monospace; }
    .wire { fill: none; stroke: #78928d; stroke-width: 1.4; marker-end: url(#arrow); }
    .wire.data { stroke: #b7833b; stroke-dasharray: 5 4; }
    .wire.preview { stroke: var(--accent); stroke-width: 2; stroke-dasharray: 6 4; marker-end: none; pointer-events: none; }
    .wire.preview.data { stroke: #b7833b; }
    .bottom { display: grid; grid-template-rows: 38px minmax(0, 1fr); border-top: 1px solid var(--line); background: #fffefa; }
    .bottom-head { display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid var(--line-soft); padding: 4px 10px; }
    .bottom-body { min-height: 0; }
    textarea.code, pre.output { width: 100%; height: 100%; margin: 0; overflow: auto; border: 0; background: #fffefa; color: #3c514e; padding: 12px 14px; font: 12px/1.6 ui-monospace, "Cascadia Code", Consolas, monospace; resize: none; }
    pre.output { display: none; white-space: pre-wrap; }
    .message-list { min-height: 0; }
    .message { margin-bottom: 12px; border-radius: 10px; padding: 10px 11px; font-size: 12px; line-height: 1.55; white-space: pre-wrap; }
    .message.user { margin-left: 34px; background: rgba(22,125,113,.10); color: #31524d; }
    .message.system { margin-right: 20px; border-left: 2px solid var(--accent-2); border-radius: 0 8px 8px 0; background: rgba(182,117,34,.07); color: #536863; }
    .composer { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 7px; border-top: 1px solid var(--line-soft); padding: 10px; }
    .composer textarea { min-height: 58px; max-height: 120px; border: 1px solid var(--line); border-radius: 8px; background: #fffefa; color: var(--text); padding: 9px; font-size: 12px; resize: vertical; }
    .context { border-top: 1px solid var(--line-soft); padding: 9px 11px; color: var(--muted); font: 10px/1.6 ui-monospace, Consolas, monospace; }
    .trace { display: none; position: absolute; right: 14px; bottom: 14px; width: min(440px, 70%); max-height: 45%; overflow: auto; border: 1px solid var(--line); border-radius: 10px; background: rgba(255,253,248,.97); box-shadow: var(--shadow); padding: 10px; }
    .trace.open { display: block; }
    .trace-row { border-left: 2px solid var(--ok); padding: 6px 8px; color: var(--muted); font-size: 11px; }
    .trace-row.failed { border-color: var(--bad); }
    .trace-row + .trace-row { margin-top: 5px; }
    @media (max-width: 900px) {
      .topbar { height: auto; min-height: 64px; flex-wrap: wrap; padding: 12px; }
      .top-actions { min-width: 0; flex-wrap: wrap; }
      .pill { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .shell { display: grid; height: auto; grid-template-columns: minmax(0, 1fr); grid-template-rows: 236px minmax(590px, 72vh) minmax(360px, 56vh); }
      .rail { border-right: 0; border-bottom: 1px solid var(--line); }
      .assistant { border-top: 1px solid var(--line); border-left: 0; }
      .workbench { grid-template-rows: 48px minmax(320px, 1fr) 214px; }
    }
    @media (prefers-reduced-motion: reduce) { *, *::before, *::after { transition-duration: .01ms !important; animation-duration: .01ms !important; } }
  </style>
</head>
<body>
  <header class="topbar">
    <div class="brand">
      <div class="brand-mark">WF</div>
      <div><h1>Workflow Studio</h1><p class="eyebrow">local orchestration workbench</p></div>
    </div>
    <div class="top-actions">
      <span class="pill" id="policy">policy: loading</span>
      <span class="pill ok" id="session">connecting</span>
    </div>
  </header>
  <main class="shell">
    <aside class="rail">
      <div class="rail-head"><h2>Registry</h2><span class="pill" id="nodeCount">0 nodes</span></div>
      <div class="rail-body">
        <input class="search" id="search" placeholder="Filter workflows or nodes" aria-label="Filter workflows or nodes" />
        <div class="section-label">Persisted workflows</div><div id="workflows"></div>
        <div class="section-label">Graph node catalog</div><div id="nodes"></div>
      </div>
    </aside>
    <section class="workbench">
      <div class="pane-head">
        <div class="tabs"><button class="tab active" data-mode="professional">Professional</button><button class="tab" data-mode="normal">Normal</button></div>
        <div class="toolbar"><button class="button" id="traceToggle">Trace</button><button class="button" id="layout">Auto layout</button><button class="button" id="compile">Compile</button><button class="button" id="save">Save</button><button class="button primary" id="run">Run workflow</button></div>
      </div>
      <div class="canvas-wrap">
        <svg id="graph" role="img" aria-label="Workflow graph preview"></svg>
        <div class="empty-canvas" id="emptyCanvas"><div><b>No blueprint loaded</b>Compile a script or open a persisted workflow.</div></div>
        <div class="trace" id="trace"></div>
      </div>
      <div class="bottom">
        <div class="bottom-head"><div class="tabs"><button class="tab active" data-bottom="script">Script</button><button class="tab" data-bottom="output">Output</button></div><span class="pill" id="blueprintStatus">draft empty</span></div>
        <div class="bottom-body">
          <textarea class="code" id="script" spellcheck="false">workflow demo
start -> end</textarea>
          <pre class="output" id="output"></pre>
        </div>
      </div>
    </section>
    <aside class="assistant">
      <div class="assistant-head"><div><h2>Editor agent</h2><p class="eyebrow">workflow-studio-editor</p></div><span class="pill ok" id="agentState">ready</span></div>
      <div class="message-list" id="messages"><div class="message system">Ask the independent editor agent to inspect workflows, compile scripts, or explain graph nodes. Conversation updates and tool activity appear here as Runtime events arrive.</div></div>
      <form class="composer" id="composer"><textarea id="chat" placeholder="Ask the editor agent…" aria-label="Message editor agent"></textarea><button class="button primary" id="send" type="submit">Send</button></form>
      <div class="context" id="context">Loading editor context…</div>
    </aside>
  </main>
  <script>
    const token = new URLSearchParams(location.search).get('token') || '';
    const state = { blueprint: null, workflows: [], nodes: [], mode: 'professional', selected: null, drag: null, connect: null, trace: null, since: 0, seen: new Set(), snapshotTimer: null, snapshotRevision: 0, lastSnapshotKey: '' };
    const $ = id => document.getElementById(id);
    const escapeHtml = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
    async function api(path, options = {}) {
      const res = await fetch(path + (path.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(token), {
        ...options,
        headers: { 'Content-Type': 'application/json', ...(options.headers || {}) }
      });
      const value = await res.json();
      if (!res.ok || value.error) throw new Error(value.error || `Request failed: ${res.status}`);
      return value;
    }
    function draftSnapshotKey() {
      return state.blueprint ? JSON.stringify(state.blueprint) : '';
    }
    async function decompileCurrentDraft(reason = 'draft_changed') {
      if (!state.blueprint?.nodes?.length) return null;
      const key = draftSnapshotKey();
      if (key === state.lastSnapshotKey) return null;
      const out = await api('/api/decompile', { method: 'POST', body: JSON.stringify({ blueprint: state.blueprint, reason }) });
      state.lastSnapshotKey = key;
      if (typeof out.script === 'string') $('script').value = out.script;
      return out;
    }
    function scheduleStudioSnapshot(reason = 'draft_changed') {
      if (state.snapshotTimer) clearTimeout(state.snapshotTimer);
      state.snapshotTimer = setTimeout(() => {
        state.snapshotTimer = null;
        decompileCurrentDraft(reason).catch(error => console.warn('Workflow draft decompile failed', error));
      }, 350);
    }
    async function flushStudioSnapshot(reason = 'before_chat') {
      if (state.snapshotTimer) {
        clearTimeout(state.snapshotTimer);
        state.snapshotTimer = null;
      }
      try { return await decompileCurrentDraft(reason); }
      catch (error) { console.warn('Workflow draft decompile failed', error); return null; }
    }
    async function refresh() {
      const context = await api('/api/context');
      $('session').textContent = context.session_id || 'connected';
      $('policy').textContent = `policy: ${context.tool_execution_policy || 'default'}`;
      $('context').textContent = `${context.editor_agent?.id || 'workflow-studio-editor'}\n${context.workflows_dir || ''}\n${context.runtime?.runtime_instance_id || ''}`;
      state.nodes = context.node_capabilities || [];
      $('nodeCount').textContent = `${state.nodes.length} nodes`;
      const workflows = await api('/api/workflows'); state.workflows = workflows.workflows || [];
      const tools = await api('/api/tools');
      state.nodes = tools.node_capabilities || state.nodes;
      renderRegistry();
      scheduleStudioSnapshot('context_loaded');
    }
    function renderRegistry() {
      const q = $('search').value.trim().toLowerCase();
      const workflows = state.workflows.filter(w => `${w.name} ${w.id} ${w.kind}`.toLowerCase().includes(q));
      $('workflows').innerHTML = workflows.map(w => `<button class="item" data-workflow="${escapeHtml(w.id)}"><span class="item-title">${escapeHtml(w.name || w.id)}</span><span class="item-meta">${escapeHtml(w.kind || 'draft')} r${Number(w.revision || 0)}</span></button>`).join('') || '<span class="item-meta">No Workflow resources</span>';
      const nodes = state.nodes.filter(n => `${n.name} ${n.display_name} ${n.category}`.toLowerCase().includes(q));
      const groups = new Map();
      nodes.forEach(node => {
        const category = String(node.category || 'Other');
        const categoryRoot = category.split('/')[0].trim().toLowerCase();
        const identity = `${node.name || ''} ${node.display_name || ''}`.toLowerCase();
        const source = String(node.source || 'local').toLowerCase();
        let group = 'Local tools';
        if (identity.includes('setvar') || identity.includes('set variable')) group = 'Set var';
        else if (categoryRoot === 'control flow') group = 'Flow control';
        else if (source === 'rpc') group = 'RPC tools';
        else if (['math', 'constants', 'array', 'logic', 'string', 'data', 'variable'].includes(categoryRoot)) group = 'Data processing';
        const subgroup = category.split('/').map(part => part.trim()).filter(Boolean).join(' / ') || 'Other';
        if (!groups.has(group)) groups.set(group, new Map());
        const subgroups = groups.get(group);
        if (!subgroups.has(subgroup)) subgroups.set(subgroup, []);
        subgroups.get(subgroup).push(node);
      });
      const order = ['Data processing', 'Flow control', 'Set var', 'Local tools', 'RPC tools'];
      $('nodes').innerHTML = order.filter(group => groups.has(group)).map(group => {
        const subgroups = groups.get(group);
        const count = Array.from(subgroups.values()).reduce((total, items) => total + items.length, 0);
        const content = Array.from(subgroups.entries()).map(([subgroup, items]) => `
          <details class="subgroup" ${q ? 'open' : ''}>
            <summary class="subgroup-title"><span class="tree-label">${escapeHtml(subgroup)}</span><span class="group-count">${items.length}</span></summary>
            <div class="subgroup-items">${items.map(n => `<button class="item" title="${escapeHtml(n.description)}"><span class="item-title">${escapeHtml(n.display_name || n.name)}</span><span class="item-meta">${escapeHtml(n.source || 'local')} · ${escapeHtml(n.category || 'node')}</span></button>`).join('')}</div>
          </details>
        `).join('');
        return `<details class="group" open><summary class="group-title"><span class="tree-label">${escapeHtml(group)}</span><span class="group-count">${count}</span></summary>${content}</details>`;
      }).join('') || '<span class="item-meta">No matching nodes</span>';
      document.querySelectorAll('[data-workflow]').forEach(button => button.onclick = () => openWorkflow(button.dataset.workflow));
    }
    async function openWorkflow(workflowId) {
      try { const out = await api('/api/workflows/' + encodeURIComponent(workflowId)); state.currentWorkflowId = workflowId; setBlueprint(out.blueprint); setOutput(out); } catch (error) { setOutput({ error: error.message }); }
    }
    function setBlueprint(blueprint) {
      state.blueprint = blueprint || null; state.selected = null;
      const count = blueprint?.nodes?.length || 0;
      $('blueprintStatus').textContent = count ? `${count} nodes · ${blueprint.connections?.length || 0} wires` : 'draft empty';
      $('emptyCanvas').style.display = count ? 'none' : 'grid';
      renderGraph();
      scheduleStudioSnapshot('blueprint_changed');
    }
    function nodeSize(node) {
      const s = node.size || {};
      const inputCount = (node.pins || []).filter(p => p.kind === 'ExecInput' || p.kind === 'DataInput').length;
      const outputCount = (node.pins || []).filter(p => p.kind === 'ExecOutput' || p.kind === 'DataOutput').length;
      return { width: Number.isFinite(s.width) && s.width > 0 ? s.width : 260, height: Number.isFinite(s.height) && s.height > 0 ? s.height : Math.max(76, 58 + Math.max(inputCount, outputCount) * 24) };
    }
    function nodePosition(node, index) {
      const p = node.position || {};
      if (Number.isFinite(p.x) && Number.isFinite(p.y) && (p.x !== 0 || p.y !== 0)) return { x: p.x, y: p.y };
      return { x: 80 + (index % 4) * 320, y: 70 + Math.floor(index / 4) * 160 };
    }
    function graphPoint(event) {
      const svg = $('graph'), point = svg.createSVGPoint();
      point.x = event.clientX; point.y = event.clientY;
      const matrix = svg.getScreenCTM();
      return matrix ? point.matrixTransform(matrix.inverse()) : { x: event.clientX, y: event.clientY };
    }
    function nodeById(id) {
      return state.blueprint?.nodes?.find(node => node.id === id) || null;
    }
    function setNodeLayoutProperty(node, key, value) {
      node.properties = node.properties || {};
      const layout = node.properties.layout && typeof node.properties.layout === 'object' && !Array.isArray(node.properties.layout) ? node.properties.layout : {};
      layout[key] = value;
      node.properties.layout = layout;
    }
    function startNodeDrag(event, nodeId) {
      if (event.button != null && event.button !== 0) return;
      const node = nodeById(nodeId); if (!node) return;
      const pos = nodePosition(node, 0), point = graphPoint(event);
      state.selected = nodeId;
      state.drag = { nodeId, dx: point.x - pos.x, dy: point.y - pos.y, moved: false };
      event.currentTarget.classList.add('dragging');
      event.currentTarget.setPointerCapture?.(event.pointerId);
      event.preventDefault();
      renderGraph();
    }
    function moveNodeDrag(event) {
      if (!state.drag) return;
      const node = nodeById(state.drag.nodeId); if (!node) return;
      const point = graphPoint(event);
      node.position = { x: Math.max(20, Math.round(point.x - state.drag.dx)), y: Math.max(20, Math.round(point.y - state.drag.dy)) };
      setNodeLayoutProperty(node, 'position_source', 'user');
      state.drag.moved = true;
      renderGraph();
    }
    function endNodeDrag() {
      if (!state.drag) return;
      const moved = state.drag.moved;
      state.drag = null;
      renderGraph();
      if (moved) scheduleStudioSnapshot('node_dragged');
      else scheduleStudioSnapshot('selection_changed');
    }
    function pinByName(node, pinName) {
      return (node?.pins || []).find(pin => pin.name === pinName) || null;
    }
    function pinSide(pin) {
      return String(pin?.kind || '').endsWith('Output') ? 'right' : 'left';
    }
    function pinConnectionType(pin) {
      return String(pin?.kind || '').startsWith('Exec') ? 'Exec' : 'Data';
    }
    function canConnectPins(aNode, aPin, bNode, bPin) {
      if (!aNode || !aPin || !bNode || !bPin || aNode.id === bNode.id) return false;
      const aIsOut = String(aPin.kind || '').endsWith('Output'), bIsOut = String(bPin.kind || '').endsWith('Output');
      if (aIsOut === bIsOut) return false;
      return pinConnectionType(aPin) === pinConnectionType(bPin);
    }
    function normalizeConnectionPins(aNode, aPin, bNode, bPin) {
      return String(aPin.kind || '').endsWith('Output')
        ? { sourceNode: aNode, sourcePin: aPin, targetNode: bNode, targetPin: bPin }
        : { sourceNode: bNode, sourcePin: bPin, targetNode: aNode, targetPin: aPin };
    }
    function connectionId(conn) {
      return `conn_${conn.source_node}_${conn.source_pin}_${conn.target_node}_${conn.target_pin}_${Date.now().toString(36)}`;
    }
    function addConnection(aNode, aPin, bNode, bPin) {
      if (!canConnectPins(aNode, aPin, bNode, bPin)) return false;
      const pins = normalizeConnectionPins(aNode, aPin, bNode, bPin);
      const type = pinConnectionType(pins.sourcePin);
      const next = {
        source_node: pins.sourceNode.id,
        source_pin: pins.sourcePin.name,
        target_node: pins.targetNode.id,
        target_pin: pins.targetPin.name,
        connection_type: type
      };
      const exact = conn => conn.source_node === next.source_node && conn.source_pin === next.source_pin && conn.target_node === next.target_node && conn.target_pin === next.target_pin && (conn.connection_type || 'Data') === type;
      state.blueprint.connections = (state.blueprint.connections || []).filter(conn => {
        if (exact(conn)) return false;
        return !(type === 'Exec' && conn.connection_type === 'Exec' && conn.source_node === next.source_node && conn.source_pin === next.source_pin);
      });
      next.id = connectionId(next);
      state.blueprint.connections.push(next);
      $('blueprintStatus').textContent = `${state.blueprint.nodes?.length || 0} nodes 路 ${state.blueprint.connections?.length || 0} wires`;
      return true;
    }
    function startPinConnect(event) {
      if (event.button != null && event.button !== 0) return;
      const node = nodeById(event.currentTarget.dataset.node);
      const pin = pinByName(node, event.currentTarget.dataset.pin);
      if (!node || !pin) return;
      const pos = nodePosition(node, 0);
      const anchor = pinAnchor(node, pos, pin.name, pinSide(pin));
      state.connect = { nodeId: node.id, pinName: pin.name, pointer: anchor };
      event.currentTarget.setPointerCapture?.(event.pointerId);
      event.preventDefault();
      event.stopPropagation();
      renderGraph();
    }
    function movePinConnect(event) {
      if (!state.connect) return;
      state.connect.pointer = graphPoint(event);
      renderGraph();
    }
    function endPinConnect(event) {
      if (!state.connect) return;
      const sourceNode = nodeById(state.connect.nodeId);
      const sourcePin = pinByName(sourceNode, state.connect.pinName);
      const target = document.elementFromPoint(event.clientX, event.clientY)?.closest?.('[data-pin]');
      let changed = false;
      if (target) {
        const targetNode = nodeById(target.dataset.node);
        const targetPin = pinByName(targetNode, target.dataset.pin);
        changed = addConnection(sourceNode, sourcePin, targetNode, targetPin);
      }
      state.connect = null;
      renderGraph();
      if (changed) scheduleStudioSnapshot('connection_changed');
    }
    function truncateText(value, max = 28) {
      const text = String(value ?? '').replace(/\s+/g, ' ');
      return text.length > max ? text.slice(0, max - 1) + '…' : text;
    }
    function pinPreview(pin) {
      if (pin.default_value == null || pin.kind !== 'DataInput') return '';
      const raw = typeof pin.default_value === 'string' ? `"${pin.default_value}"` : JSON.stringify(pin.default_value);
      return truncateText(raw, 24);
    }
    function pinRows(node, side) {
      const pins = node.pins || [];
      const kinds = side === 'left' ? ['ExecInput', 'DataInput'] : ['ExecOutput', 'DataOutput'];
      return pins.filter(pin => kinds.includes(pin.kind));
    }
    function pinAnchor(node, pos, pinName, side) {
      const size = nodeSize(node), pins = pinRows(node, side);
      const index = Math.max(0, pins.findIndex(pin => pin.name === pinName));
      const rowY = pos.y + 50 + index * 24;
      return { x: side === 'left' ? pos.x : pos.x + size.width, y: rowY };
    }
    function renderPins(node, side, size) {
      const pins = pinRows(node, side);
      const x = side === 'left' ? 12 : size.width - 12;
      const textX = side === 'left' ? 24 : size.width - 24;
      const anchor = side === 'left' ? 'start' : 'end';
      return pins.map((pin, index) => {
        const y = 50 + index * 24, isExec = String(pin.kind).startsWith('Exec');
        const label = `${pin.name}${pin.data_type ? ':' + truncateText(pin.data_type, 10) : ''}`;
        const preview = side === 'left' ? pinPreview(pin) : '';
        return `<g><circle class="pin ${isExec ? 'exec' : 'data'}" cx="${x}" cy="${y - 3}" r="${isExec ? 4 : 3.4}"/><circle class="pin-hotspot" data-node="${escapeHtml(node.id)}" data-pin="${escapeHtml(pin.name)}" cx="${x}" cy="${y - 3}" r="10"/><text class="pin-text" x="${textX}" y="${y}" text-anchor="${anchor}">${escapeHtml(truncateText(label, 22))}</text>${preview ? `<text class="pin-preview" x="${textX}" y="${y + 12}" text-anchor="${anchor}">${escapeHtml(preview)}</text>` : ''}</g>`;
      }).join('');
    }
    function renderGraph() {
      const svg = $('graph'), bp = state.blueprint;
      if (!bp?.nodes?.length) { svg.innerHTML = ''; return; }
      const positions = new Map(bp.nodes.map((n, i) => [n.id, nodePosition(n, i)]));
      const nodeMap = new Map(bp.nodes.map(n => [n.id, n]));
      const wires = (bp.connections || []).map(c => {
        const source = nodeMap.get(c.source_node), target = nodeMap.get(c.target_node);
        const a = positions.get(c.source_node), b = positions.get(c.target_node); if (!source || !target || !a || !b) return '';
        const start = pinAnchor(source, a, c.source_pin, 'right'), end = pinAnchor(target, b, c.target_pin, 'left');
        const x1=start.x,y1=start.y,x2=end.x,y2=end.y, bend=Math.max(46,Math.abs(x2-x1)*.42);
        return `<path class="wire ${c.connection_type === 'Exec' ? '' : 'data'}" d="M${x1} ${y1} C${x1+bend} ${y1},${x2-bend} ${y2},${x2} ${y2}"/>`;
      }).join('');
      let previewWire = '';
      if (state.connect) {
        const source = nodeMap.get(state.connect.nodeId), pos = positions.get(state.connect.nodeId), pin = pinByName(source, state.connect.pinName);
        if (source && pos && pin && state.connect.pointer) {
          const start = pinAnchor(source, pos, pin.name, pinSide(pin)), end = state.connect.pointer;
          const x1=start.x,y1=start.y,x2=end.x,y2=end.y,bend=Math.max(46,Math.abs(x2-x1)*.42);
          previewWire = `<path class="wire preview ${pinConnectionType(pin) === 'Exec' ? '' : 'data'}" d="M${x1} ${y1} C${x1+bend} ${y1},${x2-bend} ${y2},${x2} ${y2}"/>`;
        }
      }
      const nodes = bp.nodes.map((n, i) => {
        const p=positions.get(n.id), size=nodeSize(n), simple=state.mode==='normal', title=simple ? (n.display_name || n.node_type) : n.node_type;
        const subtitle=simple ? `${n.pins?.length || 0} pins` : (n.display_name || `${n.pins?.length || 0} pins`);
        return `<g class="node ${String(n.node_type).toLowerCase().includes('rpc') ? 'rpc' : ''} ${state.selected===n.id ? 'selected' : ''} ${state.drag?.nodeId===n.id ? 'dragging' : ''}" data-node="${escapeHtml(n.id)}" transform="translate(${p.x} ${p.y})"><rect width="${size.width}" height="${size.height}" rx="8"/><rect class="node-head" width="${size.width}" height="36" rx="8"/><text x="12" y="22">${escapeHtml(truncateText(title, 32))}</text><text class="node-sub" x="12" y="34">${escapeHtml(truncateText(subtitle, 38))}</text>${renderPins(n, 'left', size)}${renderPins(n, 'right', size)}</g>`;
      }).join('');
      const extents = bp.nodes.map((n, i) => { const p = positions.get(n.id), s = nodeSize(n); return { x: p.x + s.width, y: p.y + s.height }; });
      const maxX = Math.max(900, ...extents.map(p => p.x + 120));
      const maxY = Math.max(620, ...extents.map(p => p.y + 120));
      svg.setAttribute('viewBox', `0 0 ${maxX} ${maxY}`);
      svg.innerHTML = `<defs><marker id="arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill='rgb(120, 146, 141)'/></marker></defs>${wires}${previewWire}${nodes}`;
      svg.querySelectorAll('[data-node]').forEach(node => node.onpointerdown = event => startNodeDrag(event, node.dataset.node));
      svg.querySelectorAll('[data-pin]').forEach(pin => pin.onpointerdown = startPinConnect);
    }
    function setOutput(value) { $('output').textContent = JSON.stringify(value, null, 2); showBottom('output'); }
    function showBottom(name) {
      document.querySelectorAll('[data-bottom]').forEach(b => b.classList.toggle('active', b.dataset.bottom === name));
      $('script').style.display = name === 'script' ? 'block' : 'none'; $('output').style.display = name === 'output' ? 'block' : 'none';
    }
    $('compile').onclick = async () => {
      try { const out=await api('/api/compile',{method:'POST',body:JSON.stringify({script:$('script').value})}); setBlueprint(out.blueprint); setOutput(out); } catch(error) { setOutput({error:error.message}); }
    };
    $('layout').onclick = async () => {
      try { if(!state.blueprint) return; const out=await api('/api/layout',{method:'POST',body:JSON.stringify({blueprint:state.blueprint,mode:'all'})}); setBlueprint(out.blueprint); setOutput({schema:out.schema,algorithm:out.algorithm,mode:out.mode}); } catch(error) { setOutput({error:error.message}); }
    };
    $('save').onclick = async () => {
      try { const out=await api('/api/save',{method:'POST',body:JSON.stringify({blueprint:state.blueprint})}); setOutput(out); await refresh(); } catch(error) { setOutput({error:error.message}); }
    };
    $('run').onclick = async () => {
      try { const out=await api('/api/run',{method:'POST',body:JSON.stringify({blueprint:state.blueprint,inputs:{},trace:true})}); state.trace=out.trace; renderTrace(); setOutput(out); scheduleStudioSnapshot('trace_changed'); } catch(error) { setOutput({error:error.message}); }
    };
    function renderTrace() {
      const nodes=state.trace?.nodes || [];
      $('trace').innerHTML = `<h3>Execution trace</h3>${nodes.map(n=>`<div class="trace-row ${n.status==='Failed'?'failed':''}"><b>${escapeHtml(n.node_name)}</b><br>${escapeHtml(n.status)}${n.duration_ms == null ? '' : ` · ${n.duration_ms} ms`}</div>`).join('') || '<p class="item-meta">No trace recorded yet.</p>'}`;
    }
    $('traceToggle').onclick = () => { renderTrace(); $('trace').classList.toggle('open'); };
    function appendMessage(kind, text, key) {
      if (key && state.seen.has(key)) return;
      if (key) state.seen.add(key);
      $('messages').insertAdjacentHTML('beforeend', `<div class="message ${kind}">${escapeHtml(text)}</div>`);
      $('messages').scrollTop = $('messages').scrollHeight;
    }
    function runtimeEventRecords(event) {
      const payload = event.payload || {};
      if (event.type === 'conversation.ledger_delta') {
        if (Array.isArray(payload.records)) return payload.records;
        return payload.record ? [payload.record] : [];
      }
      if (event.type === 'frontend:state_snapshot') {
        if (Array.isArray(payload.ledger_records)) return payload.ledger_records;
        if (Array.isArray(payload.ledger_delta?.records)) return payload.ledger_delta.records;
        return payload.ledger_delta?.record ? [payload.ledger_delta.record] : [];
      }
      return [];
    }
    function recordKey(record) {
      return String(record.record_id || record.id || `${record.role}:${record.created_at || ''}:${record.content || record.text || ''}`);
    }
    function appendLedgerRecord(record) {
      if (!record) return;
      const key = recordKey(record);
      const subtype = record.metadata?.subtype;
      const content = record.content || record.text || record.metadata?.display_content || '';
      const toolName = record.metadata?.tool_name || record.metadata?.extra?.tool_name || 'tool';
      if (record.role === 'assistant') appendMessage('system', content, key);
      else if (record.role === 'user') appendMessage('user', content, key);
      else if (subtype === 'tool_call_started') appendMessage('system', `Running: ${toolName}\n${content}`, key);
      else if (subtype === 'tool_call_finished') appendMessage('system', `Completed: ${toolName}\n${content}`, key);
      else if (subtype === 'tool_call_failed') appendMessage('system', `Failed: ${toolName}\n${content}`, key);
      else if (subtype === 'tool_call_permission_requested') appendMessage('system', `Waiting permission: ${toolName}\n${content}`, key);
      else if (subtype === 'tool_call_permission_resolved') appendMessage('system', `Permission resolved: ${toolName}\n${content}`, key);
      else if (subtype === 'llm_usage') appendMessage('system', `LLM usage\n${content || JSON.stringify(record.metadata?.extra || {}, null, 2)}`, key);
      else if (subtype === 'llm_error') appendMessage('system', `LLM error\n${content || JSON.stringify(record.metadata?.extra || {}, null, 2)}`, key);
      else if (content) appendMessage('system', content, key);
    }
    function applyRuntimeEvent(event) {
      state.since = Math.max(state.since, Number(event.event_seq || 0));
      if (event.event_line === 'workflow') {
        const payload = event.payload || {};
        refresh().then(() => {
          if (payload.operation !== 'deleted' && payload.workflow_id === state.currentWorkflowId) {
            return openWorkflow(payload.workflow_id);
          }
        }).catch(error => console.warn('Failed to refresh Workflow resources', error));
        return;
      }
      const payload = event.payload || {};
      if (event.type === 'frontend:state_snapshot') {
        $('agentState').textContent = payload.conversation_state || 'waiting';
      } else if (event.type === 'conversation.state_delta') {
        $('agentState').textContent = payload.conversation_state || payload.state || 'waiting';
      }
      for (const record of runtimeEventRecords(event)) {
        appendLedgerRecord(record);
      }
    }
    function connectEvents() {
      const source = new EventSource('/events?token=' + encodeURIComponent(token) + '&since=' + state.since);
      source.onopen = () => { $('agentState').textContent = 'ready'; };
      source.onerror = () => { $('agentState').textContent = 'reconnecting'; };
      source.onmessage = message => {
        if (!message.data) return;
        try { applyRuntimeEvent(JSON.parse(message.data)); }
        catch (error) { console.warn('Failed to parse Workflow Studio SSE event', error, message.data); }
      };
      return source;
    }
    $('search').oninput = renderRegistry;
    document.querySelectorAll('[data-mode]').forEach(button => button.onclick=()=>{ state.mode=button.dataset.mode; document.querySelectorAll('[data-mode]').forEach(b=>b.classList.toggle('active',b===button)); renderGraph(); });
    document.querySelectorAll('[data-bottom]').forEach(button => button.onclick=()=>showBottom(button.dataset.bottom));
    addEventListener('pointermove', event => { moveNodeDrag(event); movePinConnect(event); });
    addEventListener('pointerup', event => { endNodeDrag(); endPinConnect(event); });
    addEventListener('pointercancel', event => { endNodeDrag(); endPinConnect(event); });
    $('composer').onsubmit = async event => {
      event.preventDefault(); const text=$('chat').value.trim(); if(!text)return;
      appendMessage('user',text); $('chat').value=''; $('agentState').textContent='sending';
      try { await flushStudioSnapshot('before_chat'); const out=await api('/api/chat',{method:'POST',body:JSON.stringify({message:text})}); appendMessage('system',`Message admitted.\n${out.command_id || ''}`); $('agentState').textContent='admitted'; }
      catch(error){ appendMessage('system',`Send failed: ${error.message}`); $('agentState').textContent='error'; }
      $('messages').scrollTop=$('messages').scrollHeight;
    };
    const eventSource = connectEvents();
    addEventListener('beforeunload', () => eventSource.close());
    refresh().catch(error => setOutput({ error: error.message }));
  </script>
</body>
</html>"#
        .to_string()
}

#[cfg(test)]
mod tool_permission_tests {
    use super::*;

    #[test]
    fn default_studio_policy_asks_for_destructive_tools() {
        let policy =
            workflow_studio_tool_permission_policy("open_readonly_confirm_destructive").unwrap();
        assert_eq!(policy.read_only, ai_assistant::ToolPermissionMode::Full);
        assert_eq!(
            policy.controlled_change,
            ai_assistant::ToolPermissionMode::Full
        );
        assert_eq!(policy.destructive, ai_assistant::ToolPermissionMode::Ask);
    }

    #[test]
    fn parses_studio_tool_permission_decisions() {
        let request = parse_studio_tool_permission_request(
            br#"{"conversation_id":"studio-1","tool_call_id":"call-1","decision":"allow"}"#,
            "studio-1",
        )
        .unwrap();
        assert_eq!(request.tool_call_id, "call-1");
        assert_eq!(
            request.decision,
            ai_assistant::ToolPermissionDecision::Allow
        );

        let request = parse_studio_tool_permission_request(
            br#"{"conversation_id":"studio-1","tool_call_id":"call-2","decision":"deny"}"#,
            "studio-1",
        )
        .unwrap();
        assert_eq!(request.decision, ai_assistant::ToolPermissionDecision::Deny);
    }

    #[test]
    fn rejects_tool_permission_for_another_studio_conversation() {
        let error = parse_studio_tool_permission_request(
            br#"{"conversation_id":"studio-2","tool_call_id":"call-1","decision":"allow"}"#,
            "studio-1",
        )
        .err()
        .unwrap();
        assert!(error.contains("does not belong"));
    }
}
