use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use ai_assistant::conversation_state::LedgerReadOptions;
use ai_assistant::ledger::LedgerRole;
use ai_assistant::ConversationManager;
use serde_json::{json, Value};
use tokio::runtime::Handle;

use super::tool_runtime::AgentTestToolRuntime;
use crate::runtime::{AgentTestRuntimeHost, RuntimeError};

pub(crate) struct AgentTestStudioServer {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for AgentTestStudioServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", port_from_url(&self.url)));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentTestStudioState {
    pub token: String,
    pub session_id: String,
    pub target_agent_id: String,
    pub target_name: String,
    pub supervisor_name: String,
    pub supervisor_conversation_id: String,
    pub runtime_handle: Handle,
    pub manager: Arc<ConversationManager>,
    pub runtime: Arc<AgentTestToolRuntime<AgentTestRuntimeHost>>,
    pub event_log: Arc<StdMutex<VecDeque<Value>>>,
}

pub(crate) fn start_agent_test_studio_server(
    state: AgentTestStudioState,
) -> Result<AgentTestStudioServer, RuntimeError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| RuntimeError::Internal(format!("bind Agent Test Studio failed: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| RuntimeError::Internal(format!("read Agent Test Studio address failed: {e}")))?
        .port();
    let url = format!("http://127.0.0.1:{port}/?token={}", state.token);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("agent-test-studio-http".to_string())
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
                            .name("agent-test-studio-http-client".to_string())
                            .spawn(move || handle_stream(&state, &stop, &mut stream));
                    }
                    Err(error) => tracing::warn!("Agent Test Studio connection failed: {error}"),
                }
            }
        })
        .map_err(|e| RuntimeError::Internal(format!("spawn Agent Test Studio failed: {e}")))?;

    Ok(AgentTestStudioServer {
        url,
        stop,
        thread: Some(thread),
    })
}

fn handle_stream(state: &AgentTestStudioState, stop: &AtomicBool, stream: &mut TcpStream) {
    let request = match read_http_request(stream) {
        Ok(request) => request,
        Err(error) => {
            let _ = write_json_response(stream, 400, json!({ "error": error }));
            return;
        }
    };
    if request.method == "GET" && request.path == "/events" {
        if authorized(state, &request) {
            let _ = write_sse(state, stop, stream, &request);
        } else {
            let _ =
                write_json_response(stream, 401, json!({ "error": "invalid agent test token" }));
        }
        return;
    }

    let response = handle_request(state, request);
    let _ = match response {
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
    Static {
        status: u16,
        content_type: &'static str,
        body: &'static [u8],
    },
    Json(u16, Value),
}

fn handle_request(state: &AgentTestStudioState, request: StudioRequest) -> StudioResponse {
    if request.method == "OPTIONS" {
        return StudioResponse::Json(204, json!({}));
    }
    if request.path == "/" || request.path == "/index.html" {
        return static_asset("/index.html")
            .unwrap_or_else(|| StudioResponse::Json(404, json!({ "error": "not found" })));
    }
    if matches!(
        request.path.as_str(),
        "/assets/index.js" | "/assets/index.css"
    ) {
        return static_asset(&request.path)
            .unwrap_or_else(|| StudioResponse::Json(404, json!({ "error": "not found" })));
    }
    if !authorized(state, &request) {
        return StudioResponse::Json(401, json!({ "error": "invalid agent test token" }));
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/context") => StudioResponse::Json(200, context_json(state)),
        ("POST", "/api/chat") => chat_response(state, &request.body),
        ("POST", "/api/pause") => pause_response(state),
        ("POST", "/api/tool-permission") => tool_permission_response(state, &request.body),
        _ if request.method == "GET" && request.path.starts_with("/api/pairs/") => {
            let pair_id = percent_decode(request.path.trim_start_matches("/api/pairs/"));
            StudioResponse::Json(200, pair_detail_json(state, &pair_id))
        }
        _ => StudioResponse::Json(404, json!({ "error": "not found" })),
    }
}

fn context_json(state: &AgentTestStudioState) -> Value {
    let messages = supervisor_messages(
        &state.runtime_handle,
        &state.manager,
        &state.supervisor_conversation_id,
        80,
    );
    let mut context = json!({
        "schema": "agent-test-studio-context/v1",
        "session_id": state.session_id,
        "conversation_id": state.supervisor_conversation_id,
        "target_agent_id": state.target_agent_id,
        "target_name": state.target_name,
        "supervisor_name": state.supervisor_name,
        "supervisor_conversation_id": state.supervisor_conversation_id,
        "supervisor_messages": messages
    });
    if let Value::Object(object) = &mut context {
        if let Value::Object(snapshot) = snapshot_json(state) {
            object.extend(snapshot);
        }
        object.insert(
            "snapshot".to_string(),
            supervisor_frontend_snapshot_json(state),
        );
    }
    context
}

fn snapshot_json(state: &AgentTestStudioState) -> Value {
    let mut snapshot = run_request_future(&state.runtime_handle, async {
        Ok(state.runtime.snapshot_json().await)
    })
    .unwrap_or_else(|error| json!({ "pairs": [], "error": error.to_string() }));
    if let Value::Object(object) = &mut snapshot {
        object.insert(
            "supervisor_messages".to_string(),
            Value::Array(supervisor_messages(
                &state.runtime_handle,
                &state.manager,
                &state.supervisor_conversation_id,
                80,
            )),
        );
        if let Value::Object(status) = supervisor_status_json(state) {
            object.extend(status);
        }
        object.insert(
            "supervisor_snapshot".to_string(),
            supervisor_frontend_snapshot_json(state),
        );
    }
    snapshot
}

fn supervisor_frontend_snapshot_json(state: &AgentTestStudioState) -> Value {
    let mut snapshot = supervisor_status_json(state);
    if let Value::Object(object) = &mut snapshot {
        object.insert(
            "ledger_records".to_string(),
            Value::Array(supervisor_ledger_records(
                &state.runtime_handle,
                &state.manager,
                &state.supervisor_conversation_id,
                200,
            )),
        );
        object.insert("revision".to_string(), Value::Number(0.into()));
    }
    snapshot
}

fn supervisor_status_json(state: &AgentTestStudioState) -> Value {
    run_request_future(&state.runtime_handle, async {
        let status = state
            .manager
            .conversation_status(&state.supervisor_conversation_id)
            .await
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        let conversation_state = if status.compacting {
            "compacting"
        } else if status.stopping {
            "stopping"
        } else {
            match status.agent_state.as_str() {
                "executing" => "executing",
                "thinking" => "thinking",
                _ => "waiting",
            }
        };
        Ok(json!({
            "supervisor_state": status.agent_state,
            "conversation_state": conversation_state
        }))
    })
    .unwrap_or_else(|error| {
        json!({
            "supervisor_state": "unknown",
            "conversation_state": "waiting",
            "supervisor_status_error": error.to_string()
        })
    })
}

fn pair_detail_json(state: &AgentTestStudioState, pair_id: &str) -> Value {
    run_request_future(
        &state.runtime_handle,
        state.runtime.pair_detail_json(pair_id),
    )
    .unwrap_or_else(|error| json!({ "pair_id": pair_id, "error": error.to_string() }))
}

fn chat_response(state: &AgentTestStudioState, body: &[u8]) -> StudioResponse {
    #[derive(serde::Deserialize)]
    struct ChatRequest {
        message: String,
    }
    let request = match serde_json::from_slice::<ChatRequest>(body) {
        Ok(request) => request,
        Err(error) => {
            return StudioResponse::Json(
                400,
                json!({ "error": format!("invalid chat JSON: {error}") }),
            )
        }
    };
    let message = request.message.trim();
    if message.is_empty() {
        return StudioResponse::Json(400, json!({ "error": "message must not be empty" }));
    }
    match send_supervisor_message(
        &state.runtime_handle,
        Arc::clone(&state.manager),
        state.supervisor_conversation_id.clone(),
        message,
    ) {
        Ok(command_id) => StudioResponse::Json(
            200,
            json!({
                "schema": "agent-test-studio-chat-result/v1",
                "command_id": command_id
            }),
        ),
        Err(error) => StudioResponse::Json(500, json!({ "error": error.to_string() })),
    }
}

fn pause_response(state: &AgentTestStudioState) -> StudioResponse {
    static NEXT_PAUSE_COMMAND: AtomicU64 = AtomicU64::new(1);
    let command_id = format!(
        "agent-test-supervisor-pause-{}",
        NEXT_PAUSE_COMMAND.fetch_add(1, Ordering::Relaxed)
    );
    let manager = Arc::clone(&state.manager);
    let conversation_id = state.supervisor_conversation_id.clone();
    match run_request_future(&state.runtime_handle, async move {
        manager
            .request_pause_with_admission(&conversation_id, Some(command_id.clone()))
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))
            .map(|admission| (command_id, admission))
    }) {
        Ok((command_id, admission)) => StudioResponse::Json(
            200,
            json!({
                "schema": "agent-test-studio-pause-result/v1",
                "command_id": command_id,
                "decision": admission.decision
            }),
        ),
        Err(error) => StudioResponse::Json(500, json!({ "error": error.to_string() })),
    }
}

fn tool_permission_response(state: &AgentTestStudioState, body: &[u8]) -> StudioResponse {
    let request = match crate::workflow_studio::parse_studio_tool_permission_request(
        body,
        &state.supervisor_conversation_id,
    ) {
        Ok(request) => request,
        Err(error) => return StudioResponse::Json(400, json!({"error": error})),
    };
    let manager = Arc::clone(&state.manager);
    let conversation_id = state.supervisor_conversation_id.clone();
    match run_request_future(&state.runtime_handle, async move {
        manager
            .resolve_tool_permission(&conversation_id, &request.tool_call_id, request.decision)
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))
    }) {
        Ok(resolved) => StudioResponse::Json(200, json!({"resolved": resolved})),
        Err(error) => StudioResponse::Json(500, json!({"error": error.to_string()})),
    }
}

fn send_supervisor_message(
    runtime_handle: &Handle,
    manager: Arc<ConversationManager>,
    conversation_id: String,
    message: &str,
) -> Result<String, RuntimeError> {
    static NEXT_CHAT_COMMAND: AtomicU64 = AtomicU64::new(1);
    let command_id = format!(
        "agent-test-supervisor-chat-{}",
        NEXT_CHAT_COMMAND.fetch_add(1, Ordering::Relaxed)
    );
    let message = message.to_string();
    let command = command_id.clone();
    let admission = run_request_future(runtime_handle, async move {
        manager
            .send_message_with_admission(&conversation_id, &message, Some(command.clone()))
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))
    })?;
    if !admission.decision.is_accepted() {
        return Err(RuntimeError::InvalidConfig(format!(
            "supervisor message was not admitted: {:?}",
            admission.decision
        )));
    }
    Ok(command_id)
}

fn supervisor_messages(
    runtime_handle: &Handle,
    manager: &Arc<ConversationManager>,
    conversation_id: &str,
    limit: usize,
) -> Vec<Value> {
    let manager = Arc::clone(manager);
    let conversation_id = conversation_id.to_string();
    run_request_future(runtime_handle, async move {
        let records = manager
            .ledger(
                &conversation_id,
                LedgerReadOptions {
                    limit,
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        Ok::<_, RuntimeError>(
            records
                .into_iter()
                .filter(|record| matches!(record.role, LedgerRole::User | LedgerRole::Assistant))
                .map(|record| {
                    json!({
                        "id": record.record_id.to_string(),
                        "role": record.role.as_str(),
                        "content": record.metadata.display_content.unwrap_or(record.content),
                        "created_at": record.created_at
                    })
                })
                .collect::<Vec<_>>(),
        )
    })
    .unwrap_or_default()
}

fn supervisor_ledger_records(
    runtime_handle: &Handle,
    manager: &Arc<ConversationManager>,
    conversation_id: &str,
    limit: usize,
) -> Vec<Value> {
    let manager = Arc::clone(manager);
    let conversation_id = conversation_id.to_string();
    run_request_future(runtime_handle, async move {
        let records = manager
            .ledger(
                &conversation_id,
                LedgerReadOptions {
                    limit,
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        Ok::<_, RuntimeError>(
            records
                .into_iter()
                .filter_map(|record| serde_json::to_value(record).ok())
                .collect::<Vec<_>>(),
        )
    })
    .unwrap_or_default()
}

fn write_sse(
    state: &AgentTestStudioState,
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
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nAccess-Control-Allow-Origin: http://127.0.0.1\r\n\r\n"
    )?;
    while !stop.load(Ordering::Relaxed) {
        let runtime_events = state
            .event_log
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| {
                        event.get("event_seq").and_then(Value::as_u64).unwrap_or(0) > since
                    })
                    .filter(|event| {
                        event
                            .get("conversation_id")
                            .and_then(Value::as_str)
                            .map(|value| value == state.supervisor_conversation_id)
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for event in runtime_events {
            let event_seq = event
                .get("event_seq")
                .and_then(Value::as_u64)
                .unwrap_or(since);
            let line = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
            write!(stream, "id: {event_seq}\ndata: {line}\n\n")?;
            since = event_seq;
        }
        let payload = snapshot_json(state);
        let business_event = json!({
            "type": "agent-test.snapshot",
            "payload": payload
        });
        let line = serde_json::to_string(&business_event).unwrap_or_else(|_| "{}".to_string());
        write!(stream, "data: {line}\n\n")?;
        stream.flush()?;
        std::thread::sleep(Duration::from_millis(900));
    }
    Ok(())
}

fn authorized(state: &AgentTestStudioState, request: &StudioRequest) -> bool {
    request.query.get("token") == Some(&state.token)
        || request
            .headers
            .get("authorization")
            .and_then(|value| value.strip_prefix("Bearer "))
            == Some(state.token.as_str())
}

fn run_request_future<F, T>(runtime_handle: &Handle, future: F) -> Result<T, RuntimeError>
where
    F: std::future::Future<Output = Result<T, RuntimeError>>,
{
    runtime_handle.block_on(future)
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
    let header_text = std::str::from_utf8(&buffer[..header_end]).map_err(|e| e.to_string())?;
    let mut lines = header_text.lines();
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

fn write_json_response(stream: &mut TcpStream, status: u16, value: Value) -> std::io::Result<()> {
    let body = if status == 204 {
        String::new()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    };
    write_http_bytes_response(
        stream,
        status,
        "application/json; charset=utf-8",
        body.as_bytes(),
    )
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
        500 => "Internal Server Error",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: http://127.0.0.1\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn static_asset(path: &str) -> Option<StudioResponse> {
    match path {
        "/index.html" => Some(StudioResponse::Static {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: include_bytes!("../../frontend/agent-test-studio/dist/index.html"),
        }),
        "/assets/index.js" => Some(StudioResponse::Static {
            status: 200,
            content_type: "text/javascript; charset=utf-8",
            body: include_bytes!("../../frontend/agent-test-studio/dist/assets/index.js"),
        }),
        "/assets/index.css" => Some(StudioResponse::Static {
            status: 200,
            content_type: "text/css; charset=utf-8",
            body: include_bytes!("../../frontend/agent-test-studio/dist/assets/index.css"),
        }),
        _ => None,
    }
}

fn port_from_url(url: &str) -> u16 {
    url.split(':')
        .nth(2)
        .and_then(|tail| tail.split('/').next())
        .and_then(|port| port.parse().ok())
        .unwrap_or(0)
}
