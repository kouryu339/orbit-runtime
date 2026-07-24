//! Stable C ABI for the Agent Runtime.
//!
//! ABI 1 deliberately exposes lifecycle and transport primitives only. Runtime
//! features are versioned JSON commands so adding a feature does not add a C
//! symbol.

#![allow(
    clippy::field_reassign_with_default,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::too_many_arguments
)]

mod agent_test_studio;
mod runtime;
mod workflow_studio;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use runtime::{RuntimeError, RuntimeFacade};
use serde_json::{json, Map, Value};

pub type AgentRuntimeHandle = u64;

const AGENT_RUNTIME_ABI_VERSION: u32 = 1;
const AGENT_RUNTIME_INVALID_HANDLE: AgentRuntimeHandle = 0;

const AGENT_RUNTIME_OK: c_int = 0;
const AGENT_RUNTIME_ERR_INVALID_ARGUMENT: c_int = 1;
const AGENT_RUNTIME_ERR_INVALID_HANDLE: c_int = 2;
const AGENT_RUNTIME_ERR_BAD_STATE: c_int = 3;
const AGENT_RUNTIME_ERR_TIMEOUT: c_int = 4;
const AGENT_RUNTIME_ERR_UNSUPPORTED: c_int = 5;
const AGENT_RUNTIME_ERR_RUNTIME: c_int = 100;
const AGENT_RUNTIME_ERR_PANIC: c_int = 101;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static NEXT_COMMAND: AtomicU64 = AtomicU64::new(1);
static HANDLES: OnceLock<Mutex<HashMap<AgentRuntimeHandle, Arc<HandleEntry>>>> = OnceLock::new();

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Open,
    Closing,
    Closed,
}

#[derive(Debug)]
struct Lifecycle {
    state: LifecycleState,
    active_calls: usize,
    shutdown_started: bool,
    shutdown_error: Option<String>,
}

struct HandleEntry {
    facade: RwLock<RuntimeFacade>,
    events: Mutex<Receiver<String>>,
    lifecycle: Mutex<Lifecycle>,
    lifecycle_changed: Condvar,
}

struct CallGuard {
    entry: Arc<HandleEntry>,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if let Ok(mut lifecycle) = self.entry.lifecycle.lock() {
            lifecycle.active_calls = lifecycle.active_calls.saturating_sub(1);
            self.entry.lifecycle_changed.notify_all();
        }
    }
}

#[derive(Debug)]
struct FfiError {
    code: c_int,
    kind: &'static str,
    message: String,
}

impl FfiError {
    fn new(code: c_int, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            kind,
            message: message.into(),
        }
    }

    fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(
            AGENT_RUNTIME_ERR_INVALID_ARGUMENT,
            "invalid_argument",
            message,
        )
    }

    fn invalid_handle() -> Self {
        Self::new(
            AGENT_RUNTIME_ERR_INVALID_HANDLE,
            "invalid_handle",
            "invalid runtime handle",
        )
    }

    fn bad_state(message: impl Into<String>) -> Self {
        Self::new(AGENT_RUNTIME_ERR_BAD_STATE, "bad_state", message)
    }

    fn timeout(message: impl Into<String>) -> Self {
        Self::new(AGENT_RUNTIME_ERR_TIMEOUT, "timeout", message)
    }

    fn unsupported(message: impl Into<String>) -> Self {
        Self::new(AGENT_RUNTIME_ERR_UNSUPPORTED, "unsupported", message)
    }

    fn panic() -> Self {
        Self::new(
            AGENT_RUNTIME_ERR_PANIC,
            "panic",
            "panic was caught at the FFI boundary",
        )
    }

    fn to_json(&self) -> Value {
        json!({
            "schema": "agent-runtime-error/v1",
            "code": self.code,
            "kind": self.kind,
            "message": self.message,
        })
    }
}

impl From<RuntimeError> for FfiError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfig(message) => Self::invalid_argument(message),
            RuntimeError::NotStarted => Self::bad_state("runtime has not been started"),
            RuntimeError::Llm(message) => Self::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "llm_failed",
                format!("LLM configuration failed: {message}"),
            ),
            RuntimeError::Rpc(message) => Self::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "rpc_failed",
                format!("RPC failed: {message}"),
            ),
            RuntimeError::Internal(message) => {
                Self::new(AGENT_RUNTIME_ERR_RUNTIME, "internal", message)
            }
        }
    }
}

fn handles() -> &'static Mutex<HashMap<AgentRuntimeHandle, Arc<HandleEntry>>> {
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_last_error(error: &FfiError) {
    let text = error.to_json().to_string().replace('\0', "\\0");
    let value = CString::new(text).unwrap_or_else(|_| {
        CString::new(
            r#"{"schema":"agent-runtime-error/v1","code":100,"kind":"internal","message":"failed to encode error"}"#,
        )
        .expect("static error JSON contains no NUL")
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(value));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

fn cstr_arg(ptr: *const c_char, name: &str) -> Result<String, FfiError> {
    if ptr.is_null() {
        return Err(FfiError::invalid_argument(format!("{name} is NULL")));
    }
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|error| FfiError::invalid_argument(format!("{name} is not valid UTF-8: {error}")))?
        .to_owned();
    Ok(value)
}

fn write_owned_string(out: *mut *mut c_char, value: String) -> Result<(), FfiError> {
    if out.is_null() {
        return Err(FfiError::invalid_argument("output string pointer is NULL"));
    }
    let value = CString::new(value.replace('\0', "\\0")).map_err(|_| {
        FfiError::new(
            AGENT_RUNTIME_ERR_RUNTIME,
            "internal",
            "encode result failed",
        )
    })?;
    unsafe {
        *out = value.into_raw();
    }
    Ok(())
}

fn clear_out_string(out: *mut *mut c_char) {
    if !out.is_null() {
        unsafe {
            *out = ptr::null_mut();
        }
    }
}

fn lookup_handle(handle: AgentRuntimeHandle) -> Result<Arc<HandleEntry>, FfiError> {
    if handle == AGENT_RUNTIME_INVALID_HANDLE {
        return Err(FfiError::invalid_handle());
    }
    handles()
        .lock()
        .map_err(|_| {
            FfiError::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "internal",
                "handle registry poisoned",
            )
        })?
        .get(&handle)
        .cloned()
        .ok_or_else(FfiError::invalid_handle)
}

fn acquire_call(handle: AgentRuntimeHandle) -> Result<CallGuard, FfiError> {
    let entry = lookup_handle(handle)?;
    {
        let mut lifecycle = entry.lifecycle.lock().map_err(|_| {
            FfiError::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "internal",
                "lifecycle lock poisoned",
            )
        })?;
        match lifecycle.state {
            LifecycleState::Open => lifecycle.active_calls += 1,
            LifecycleState::Closing => {
                return Err(FfiError::bad_state("runtime is shutting down"));
            }
            LifecycleState::Closed => {
                return Err(FfiError::bad_state("runtime is shut down"));
            }
        }
    }
    Ok(CallGuard { entry })
}

fn acquire_event_reader(handle: AgentRuntimeHandle) -> Result<Arc<HandleEntry>, FfiError> {
    // Event polling is the host's drain path. It remains available during
    // Closing/Closed so shutdown-produced ledger deltas and close events can be
    // consumed before destroy removes the handle.
    lookup_handle(handle)
}

fn next_command_id() -> String {
    let sequence = NEXT_COMMAND.fetch_add(1, Ordering::Relaxed);
    format!("ffi_cmd_{sequence}")
}

fn request_payload(request: &Value) -> Result<&Map<String, Value>, FfiError> {
    match request.get("payload") {
        None | Some(Value::Null) => {
            static EMPTY: OnceLock<Map<String, Value>> = OnceLock::new();
            Ok(EMPTY.get_or_init(Map::new))
        }
        Some(Value::Object(payload)) => Ok(payload),
        Some(_) => Err(FfiError::invalid_argument(
            "command payload must be a JSON object",
        )),
    }
}

fn required_string(payload: &Map<String, Value>, name: &str) -> Result<String, FfiError> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| FfiError::invalid_argument(format!("payload.{name} is required")))
}

fn required_string_with_aliases(
    payload: &Map<String, Value>,
    name: &str,
    aliases: &[&str],
) -> Result<String, FfiError> {
    std::iter::once(name)
        .chain(aliases.iter().copied())
        .find_map(|candidate| {
            payload
                .get(candidate)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
        .map(str::to_owned)
        .ok_or_else(|| FfiError::invalid_argument(format!("payload.{name} is required")))
}

fn optional_object_json(payload: &Map<String, Value>, name: &str) -> Result<String, FfiError> {
    match payload.get(name) {
        None | Some(Value::Null) => Ok("{}".to_string()),
        Some(Value::Object(value)) => Ok(Value::Object(value.clone()).to_string()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(FfiError::invalid_argument(format!(
            "payload.{name} must be an object or JSON string"
        ))),
    }
}

fn optional_inputs(payload: &Map<String, Value>) -> Result<HashMap<String, Value>, FfiError> {
    match payload.get("inputs") {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(Value::Object(inputs)) => Ok(inputs.clone().into_iter().collect()),
        Some(_) => Err(FfiError::invalid_argument(
            "payload.inputs must be an object",
        )),
    }
}

fn optional_workflow_execution_context(
    payload: &Map<String, Value>,
) -> Result<corework::workflow::workflows::WorkflowExecutionContext, FfiError> {
    let optional_string = |name: &str, alias: &str| -> Result<Option<String>, FfiError> {
        match payload.get(name).or_else(|| payload.get(alias)) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) if !value.trim().is_empty() => {
                Ok(Some(value.trim().to_string()))
            }
            Some(Value::String(_)) => Err(FfiError::invalid_argument(format!(
                "payload.{name} must not be empty when provided"
            ))),
            Some(_) => Err(FfiError::invalid_argument(format!(
                "payload.{name} must be a string"
            ))),
        }
    };
    let conversation_id = optional_string("conversation_id", "conversationId")?;
    let agent_id = optional_string("agent_id", "agentId")?;
    if conversation_id.is_some() != agent_id.is_some() {
        return Err(FfiError::invalid_argument(
            "payload.conversation_id and payload.agent_id must be provided together",
        ));
    }
    Ok(corework::workflow::workflows::WorkflowExecutionContext {
        conversation_id,
        agent_id,
    })
}

fn optional_bool(payload: &Map<String, Value>, name: &str) -> Result<bool, FfiError> {
    match payload.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(FfiError::invalid_argument(format!(
            "payload.{name} must be a boolean"
        ))),
    }
}

fn optional_u64(payload: &Map<String, Value>, name: &str) -> Result<Option<u64>, FfiError> {
    match payload.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_u64().map(Some).ok_or_else(|| {
            FfiError::invalid_argument(format!("payload.{name} must be an unsigned integer"))
        }),
        Some(_) => Err(FfiError::invalid_argument(format!(
            "payload.{name} must be an unsigned integer"
        ))),
    }
}

fn workflow_resource(payload: &Map<String, Value>) -> Result<Value, FfiError> {
    match payload.get("resource") {
        Some(Value::Object(resource)) => Ok(Value::Object(resource.clone())),
        Some(_) => Err(FfiError::invalid_argument(
            "payload.resource must be an object",
        )),
        None => Ok(Value::Object(payload.clone())),
    }
}

fn registration_input(payload: &Map<String, Value>) -> Result<String, FfiError> {
    if let Some(input) = payload.get("input") {
        return match input {
            Value::String(value) => Ok(value.clone()),
            Value::Object(_) | Value::Array(_) => Ok(input.to_string()),
            _ => Err(FfiError::invalid_argument(
                "payload.input must be a file path, JSON string, object, or array",
            )),
        };
    }
    if let Some(registration) = payload.get("registration") {
        return match registration {
            Value::String(value) => Ok(value.clone()),
            Value::Object(_) | Value::Array(_) => Ok(registration.to_string()),
            _ => Err(FfiError::invalid_argument(
                "payload.registration must be a JSON string, object, or array",
            )),
        };
    }
    Err(FfiError::invalid_argument(
        "payload.input or payload.registration is required",
    ))
}

fn admission_json(admission: ai_assistant::gateway::AdmissionResult) -> Value {
    let (decision, reason) = match admission.decision {
        ai_assistant::admission::Decision::Accepted { .. } => ("accepted", None),
        ai_assistant::admission::Decision::Rejected { reason } => ("rejected", Some(reason)),
    };
    json!({
        "command_id": admission.command_id,
        "decision": decision,
        "reason": reason,
    })
}

fn parse_json_or_string(value: String) -> Value {
    serde_json::from_str(&value).unwrap_or(Value::String(value))
}

fn invoke_command(
    facade: &mut RuntimeFacade,
    command_type: &str,
    payload: &Map<String, Value>,
    command_id: &str,
) -> Result<Value, FfiError> {
    let result = match command_type {
        "runtime.register_resources" => {
            facade.register_resources_input(&registration_input(payload)?)?;
            json!({})
        }
        "runtime.register_llm" => {
            facade.register_llm_input(&registration_input(payload)?)?;
            json!({})
        }
        "runtime.reload_llm" => {
            facade.reload_llm_input(&registration_input(payload)?)?;
            json!({})
        }
        "runtime.register_agent_cluster" => {
            facade.register_agent_cluster_input(&registration_input(payload)?)?;
            json!({})
        }
        "runtime.set_auth_context" => {
            let context = payload
                .get("context")
                .cloned()
                .unwrap_or_else(|| Value::Object(payload.clone()));
            facade.set_ai_auth_context(&context.to_string())?;
            json!({})
        }
        "runtime.configure_providers" => {
            facade.configure_providers(&registration_input(payload)?)?;
            json!({})
        }
        "runtime.get_provider_definitions" => parse_json_or_string(facade.provider_definitions()?),
        "runtime.get_tool_definitions" => parse_json_or_string(facade.tool_definitions()?),
        "runtime.get_workflow_node_definitions" => {
            parse_json_or_string(facade.workflow_node_definitions()?)
        }
        "runtime.get_agent_cluster_definitions" => {
            parse_json_or_string(facade.agent_cluster_definitions()?)
        }
        "runtime.get_rpc_endpoint_definitions" => {
            parse_json_or_string(facade.rpc_endpoint_definitions()?)
        }
        "runtime.set_current_model" => {
            let model_uid = payload
                .get("model_uid")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| FfiError::invalid_argument("payload.model_uid must be a uint32"))?;
            facade.set_current_model(model_uid)?;
            json!({})
        }
        "runtime.set_language" => {
            facade.set_language(&required_string(payload, "language")?)?;
            json!({})
        }
        "workflow.create" => facade.create_workflow_draft(&workflow_resource(payload)?)?,
        "workflow.read" => facade.read_workflow_resource(&required_string(payload, "id")?)?,
        "workflow.register" => facade.register_workflow_draft(
            &required_string(payload, "id")?,
            optional_u64(payload, "expected_revision")?,
            payload.get("name").and_then(Value::as_str),
        )?,
        "workflow.update" => facade.update_workflow_resource(
            &workflow_resource(payload)?,
            optional_u64(payload, "expected_revision")?,
        )?,
        "workflow.compile" => facade.compile_workflow_draft(&required_string(payload, "id")?)?,
        "workflow.delete" => facade.delete_workflow_resource(
            &required_string(payload, "id")?,
            optional_u64(payload, "expected_revision")?,
        )?,
        "workflow.list" => {
            let kind = match payload.get("kind").and_then(Value::as_str) {
                None => None,
                Some("draft") => Some(corework::workflow::workflows::WorkflowResourceKind::Draft),
                Some("registered") => {
                    Some(corework::workflow::workflows::WorkflowResourceKind::Registered)
                }
                Some(other) => {
                    return Err(FfiError::invalid_argument(format!(
                        "payload.kind '{}' is invalid; use 'draft' or 'registered'",
                        other
                    )))
                }
            };
            facade.list_workflow_resources(kind)?
        }
        "workflow.convert.script_to_blueprint" => {
            facade.workflow_script_to_blueprint(&required_string(payload, "script")?)?
        }
        "workflow.convert.blueprint_to_script" => {
            let blueprint = payload
                .get("blueprint")
                .ok_or_else(|| FfiError::invalid_argument("payload.blueprint must be an object"))?;
            if !blueprint.is_object() {
                return Err(FfiError::invalid_argument(
                    "payload.blueprint must be an object",
                ));
            }
            facade.workflow_blueprint_to_script(blueprint)?
        }
        "workflow.execute" => {
            let execution_context = optional_workflow_execution_context(payload)?;
            facade.execute_workflow_resource(
                &required_string(payload, "id")?,
                payload.get("mode").and_then(Value::as_str),
                optional_inputs(payload)?,
                optional_bool(payload, "trace")?,
                &execution_context,
            )?
        }
        "workflow.execute_script" => {
            let execution_context = optional_workflow_execution_context(payload)?;
            facade.execute_workflow_script(
                &required_string(payload, "script")?,
                optional_inputs(payload)?,
                optional_bool(payload, "trace")?,
                &execution_context,
            )?
        }
        "conversation.spawn" => {
            let spawn = payload
                .get("spawn")
                .cloned()
                .unwrap_or_else(|| Value::Object(payload.clone()));
            let info = if spawn.get("cluster_id").is_some() {
                facade.spawn_conversation(&spawn.to_string())?
            } else {
                facade.create_conversation(&spawn.to_string())?
            };
            json!({
                "conversation_id": info.conversation_id,
                "scope_id": info.scope_id,
                "tenant_id": info.tenant_id,
                "user_id": info.user_id,
                "created_at": info.created_at,
            })
        }
        "conversation.spawn_from_snapshot" => {
            let spawn = payload
                .get("spawn")
                .ok_or_else(|| FfiError::invalid_argument("payload.spawn is required"))?;
            let snapshot = payload
                .get("snapshot")
                .ok_or_else(|| FfiError::invalid_argument("payload.snapshot is required"))?;
            let info = facade
                .spawn_conversation_from_snapshot(&spawn.to_string(), &snapshot.to_string())?;
            json!({
                "conversation_id": info.conversation_id,
                "scope_id": info.scope_id,
                "tenant_id": info.tenant_id,
                "user_id": info.user_id,
                "created_at": info.created_at,
                "restored": true,
            })
        }
        "conversation.send_message" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            let content = required_string(payload, "content")?;
            let admission = facade.send_message_with_admission(
                &conversation_id,
                &content,
                command_id.to_owned(),
            )?;
            admission_json(admission)
        }
        "conversation.pause" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            let admission = facade
                .pause_conversation_with_admission(&conversation_id, command_id.to_owned())?;
            admission_json(admission)
        }
        "conversation.close" => {
            facade.close_conversation(&required_string(payload, "conversation_id")?)?;
            json!({})
        }
        "conversation.export_snapshot" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            let options = optional_object_json(payload, "options")?;
            parse_json_or_string(facade.export_conversation_snapshot(&conversation_id, &options)?)
        }
        "conversation.agent_tasks" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            parse_json_or_string(facade.agent_tasks_json(&conversation_id)?)
        }
        "conversation.materialize" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            let options = optional_object_json(payload, "options")?;
            let info = facade.materialize_conversation(&conversation_id, &options)?;
            json!({
                "conversation_id": info.conversation_id,
                "scope_id": info.scope_id,
                "tenant_id": info.tenant_id,
                "user_id": info.user_id,
                "created_at": info.created_at,
                "state_loaded": true,
            })
        }
        "conversation.import_snapshot" => {
            let snapshot = payload
                .get("snapshot")
                .ok_or_else(|| FfiError::invalid_argument("payload.snapshot is required"))?;
            let options = optional_object_json(payload, "options")?;
            facade.import_conversation_snapshot(&snapshot.to_string(), &options)?;
            json!({})
        }
        "conversation.set_dynamic_snapshot" => {
            facade.set_agent_dynamic_snapshot_field(
                &required_string(payload, "conversation_id")?,
                &required_string(payload, "agent_id")?,
                &required_string(payload, "field_name")?,
                payload
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| FfiError::invalid_argument("payload.text is required"))?,
            )?;
            json!({})
        }
        "conversation.resolve_tool_permission" => {
            let decision = match required_string(payload, "decision")?.as_str() {
                "allow" => ai_assistant::ToolPermissionDecision::Allow,
                "deny" => ai_assistant::ToolPermissionDecision::Deny,
                other => {
                    return Err(FfiError::invalid_argument(format!(
                        "payload.decision '{}' is invalid; use 'allow' or 'deny'",
                        other
                    )))
                }
            };
            let resolved = facade.resolve_tool_permission(
                &required_string_with_aliases(payload, "conversation_id", &["conversationId"])?,
                &required_string_with_aliases(payload, "tool_call_id", &["toolCallId"])?,
                decision,
            )?;
            json!({ "resolved": resolved })
        }
        "conversation.set_summary_model" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            let model_name = required_string(payload, "model_name")?;
            let admission = facade.set_conversation_summary_model_with_admission(
                &conversation_id,
                &model_name,
                command_id.to_owned(),
            )?;
            admission_json(admission)
        }
        "conversation.compact_history" => {
            let conversation_id = required_string(payload, "conversation_id")?;
            let agent_ids = match payload.get("agent_ids") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => {
                    serde_json::from_value::<Vec<String>>(value.clone()).map_err(|error| {
                        FfiError::invalid_argument(format!(
                            "payload.agent_ids must be a string array: {error}"
                        ))
                    })?
                }
            };
            let (admission, report) = facade.compact_conversation_history_with_admission(
                &conversation_id,
                agent_ids,
                command_id.to_owned(),
            )?;
            json!({
                "admission": admission_json(admission),
                "report": parse_json_or_string(report),
            })
        }
        "studio.open_workflow" => {
            let options = optional_object_json(payload, "options")?;
            parse_json_or_string(facade.open_workflow_studio(&options)?)
        }
        "studio.open_agent_test" => {
            let options = optional_object_json(payload, "options")?;
            parse_json_or_string(facade.open_agent_test_studio(&options)?)
        }
        "runtime.export_snapshot" => parse_json_or_string(facade.snapshot()?),
        _ => {
            return Err(FfiError::unsupported(format!(
                "unsupported command type '{command_type}'"
            )));
        }
    };
    Ok(result)
}

fn result_envelope(request_id: Option<&str>, command_id: &str, result: Value) -> String {
    json!({
        "schema": "agent-runtime-result/v1",
        "id": request_id,
        "command_id": command_id,
        "ok": true,
        "result": result,
    })
    .to_string()
}

fn error_envelope(request_id: Option<&str>, command_id: &str, error: &FfiError) -> String {
    json!({
        "schema": "agent-runtime-result/v1",
        "id": request_id,
        "command_id": command_id,
        "ok": false,
        "error": error.to_json(),
    })
    .to_string()
}

fn finish_status(result: Result<(), FfiError>) -> c_int {
    match result {
        Ok(()) => {
            clear_last_error();
            AGENT_RUNTIME_OK
        }
        Err(error) => {
            let code = error.code;
            set_last_error(&error);
            code
        }
    }
}

#[no_mangle]
pub extern "C" fn agent_runtime_abi_version_v1() -> u32 {
    AGENT_RUNTIME_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn agent_runtime_version_v1() -> *const c_char {
    static VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    VERSION.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn agent_runtime_capabilities_v1() -> *const c_char {
    static CAPABILITIES: OnceLock<CString> = OnceLock::new();
    CAPABILITIES
        .get_or_init(|| {
            CString::new(r#"{"schema":"agent-runtime-capabilities/v1","abi":{"major":1,"minor":3},"commands":["runtime.register_resources","runtime.register_llm","runtime.reload_llm","runtime.register_agent_cluster","runtime.set_auth_context","runtime.configure_providers","runtime.get_provider_definitions","runtime.get_tool_definitions","runtime.get_workflow_node_definitions","runtime.get_agent_cluster_definitions","runtime.get_rpc_endpoint_definitions","runtime.set_current_model","runtime.set_language","runtime.export_snapshot","workflow.create","workflow.read","workflow.register","workflow.update","workflow.compile","workflow.delete","workflow.list","workflow.convert.script_to_blueprint","workflow.convert.blueprint_to_script","workflow.execute","workflow.execute_script","conversation.spawn","conversation.spawn_from_snapshot","conversation.send_message","conversation.pause","conversation.close","conversation.export_snapshot","conversation.agent_tasks","conversation.materialize","conversation.import_snapshot","conversation.set_dynamic_snapshot","conversation.resolve_tool_permission","conversation.set_summary_model","conversation.compact_history","studio.open_workflow","studio.open_agent_test"],"events":{"transport":"pull","schema":"agent-runtime-event/v1","types":["conversation:created","conversation:closed","conversation.ledger_delta","conversation.state_delta","frontend:state_snapshot","workflow.resource_changed","workflow.execution_completed"],"lines":{"workflow":{"selector":"event_line=workflow","aggregate_id":"payload.workflow_id","conversation_scoped":false}},"drain_after_shutdown":true},"ledger":{"delta_schema":"agent-runtime-ledger-delta/v1","ops":["append"],"idempotency":["conversation_id","record_id"]},"state":{"delta_schema":"agent-runtime-state-delta/v1","ops":["focus.set","dynamic_snapshot.set","agent_task.upsert","agent_skills.set","agent_plan.set"],"agent_scoped":["dynamic_snapshot.set","agent_skills.set","agent_plan.set"],"conversation_scoped":["focus.set","agent_task.upsert"],"host_owned":["dynamic_snapshot.set"]},"shutdown":{"timeout":true,"retryable":true},"threading":{"handle_calls":"safe_serialized","callbacks":false}}"#)
                .expect("static capabilities JSON contains no NUL")
        })
        .as_ptr()
}

#[no_mangle]
pub extern "C" fn agent_runtime_create_v1(
    create_options_json: *const c_char,
    out_handle: *mut AgentRuntimeHandle,
) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_handle.is_null() {
            return Err(FfiError::invalid_argument("out_handle is NULL"));
        }
        unsafe {
            *out_handle = AGENT_RUNTIME_INVALID_HANDLE;
        }
        let config = cstr_arg(create_options_json, "create_options_json")?;
        let mut facade = RuntimeFacade::create(&config).map_err(FfiError::from)?;
        let (event_sender, event_receiver) = mpsc::channel();
        facade.set_event_sender(event_sender);
        let entry = Arc::new(HandleEntry {
            facade: RwLock::new(facade),
            events: Mutex::new(event_receiver),
            lifecycle: Mutex::new(Lifecycle {
                state: LifecycleState::Open,
                active_calls: 0,
                shutdown_started: false,
                shutdown_error: None,
            }),
            lifecycle_changed: Condvar::new(),
        });
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        if handle == AGENT_RUNTIME_INVALID_HANDLE {
            return Err(FfiError::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "internal",
                "runtime handle space exhausted",
            ));
        }
        handles()
            .lock()
            .map_err(|_| {
                FfiError::new(
                    AGENT_RUNTIME_ERR_RUNTIME,
                    "internal",
                    "handle registry poisoned",
                )
            })?
            .insert(handle, entry);
        unsafe {
            *out_handle = handle;
        }
        Ok(())
    }));
    finish_status(result.unwrap_or_else(|_| Err(FfiError::panic())))
}

#[no_mangle]
pub extern "C" fn agent_runtime_start_v1(handle: AgentRuntimeHandle) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let guard = acquire_call(handle)?;
        let mut facade = guard.entry.facade.write().map_err(|_| {
            FfiError::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "internal",
                "runtime lock poisoned",
            )
        })?;
        facade.start().map_err(FfiError::from)
    }));
    finish_status(result.unwrap_or_else(|_| Err(FfiError::panic())))
}

#[no_mangle]
pub extern "C" fn agent_runtime_invoke_v1(
    handle: AgentRuntimeHandle,
    request_json: *const c_char,
    out_response_json: *mut *mut c_char,
) -> c_int {
    clear_out_string(out_response_json);
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_response_json.is_null() {
            return Err(FfiError::invalid_argument("out_response_json is NULL"));
        }
        let request_text = cstr_arg(request_json, "request_json")?;
        let request: Value = serde_json::from_str(&request_text).map_err(|error| {
            FfiError::invalid_argument(format!("request_json is invalid JSON: {error}"))
        })?;
        if request.get("schema").and_then(Value::as_str) != Some("agent-runtime-command/v1") {
            return Err(FfiError::invalid_argument(
                "request schema must be 'agent-runtime-command/v1'",
            ));
        }
        let request_id = request.get("id").and_then(Value::as_str);
        let command_type = request
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| FfiError::invalid_argument("request type is required"))?;
        let payload = request_payload(&request)?;
        let command_id = request
            .get("command_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(next_command_id);
        let guard = acquire_call(handle)?;
        let command_result = {
            let mut facade = guard.entry.facade.write().map_err(|_| {
                FfiError::new(
                    AGENT_RUNTIME_ERR_RUNTIME,
                    "internal",
                    "runtime lock poisoned",
                )
            })?;
            invoke_command(&mut facade, command_type, payload, &command_id)
        };
        match command_result {
            Ok(value) => {
                write_owned_string(
                    out_response_json,
                    result_envelope(request_id, &command_id, value),
                )?;
                Ok(())
            }
            Err(error) => {
                let envelope = error_envelope(request_id, &command_id, &error);
                let _ = write_owned_string(out_response_json, envelope);
                Err(error)
            }
        }
    }));
    finish_status(result.unwrap_or_else(|_| Err(FfiError::panic())))
}

#[no_mangle]
pub extern "C" fn agent_runtime_next_event_v1(
    handle: AgentRuntimeHandle,
    timeout_ms: u32,
    out_event_json: *mut *mut c_char,
) -> c_int {
    clear_out_string(out_event_json);
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_event_json.is_null() {
            return Err(FfiError::invalid_argument("out_event_json is NULL"));
        }
        let entry = acquire_event_reader(handle)?;
        let receiver = entry.events.lock().map_err(|_| {
            FfiError::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "internal",
                "event queue lock poisoned",
            )
        })?;
        let event = if timeout_ms == 0 {
            match receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => {
                    return Err(FfiError::timeout("no event is currently available"));
                }
                Err(TryRecvError::Disconnected) => {
                    return Err(FfiError::bad_state("event queue is closed"));
                }
            }
        } else {
            match receiver.recv_timeout(Duration::from_millis(timeout_ms.into())) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(FfiError::timeout("timed out waiting for an event"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(FfiError::bad_state("event queue is closed"));
                }
            }
        };
        write_owned_string(out_event_json, event)
    }));
    finish_status(result.unwrap_or_else(|_| Err(FfiError::panic())))
}

#[no_mangle]
pub extern "C" fn agent_runtime_shutdown_v1(handle: AgentRuntimeHandle, timeout_ms: u32) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let entry = lookup_handle(handle)?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.into());
        let mut lifecycle = entry.lifecycle.lock().map_err(|_| {
            FfiError::new(
                AGENT_RUNTIME_ERR_RUNTIME,
                "internal",
                "lifecycle lock poisoned",
            )
        })?;
        if lifecycle.state == LifecycleState::Closed {
            return Ok(());
        }
        lifecycle.state = LifecycleState::Closing;
        loop {
            if let Some(message) = lifecycle.shutdown_error.clone() {
                return Err(FfiError::new(
                    AGENT_RUNTIME_ERR_RUNTIME,
                    "shutdown_failed",
                    message,
                ));
            }
            if lifecycle.state == LifecycleState::Closed {
                return Ok(());
            }
            if lifecycle.active_calls == 0 && !lifecycle.shutdown_started {
                lifecycle.shutdown_started = true;
                let worker_entry = Arc::clone(&entry);
                std::thread::spawn(move || {
                    let shutdown_result = worker_entry
                        .facade
                        .write()
                        .map_err(|_| "runtime lock poisoned".to_string())
                        .and_then(|mut facade| {
                            facade.shutdown().map_err(|error| error.to_string())
                        });
                    if let Ok(mut lifecycle) = worker_entry.lifecycle.lock() {
                        match shutdown_result {
                            Ok(()) => lifecycle.state = LifecycleState::Closed,
                            Err(message) => lifecycle.shutdown_error = Some(message),
                        }
                        worker_entry.lifecycle_changed.notify_all();
                    }
                });
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(FfiError::timeout("runtime shutdown is still in progress"));
            }
            let wait = deadline.saturating_duration_since(now);
            let (next, status) = entry
                .lifecycle_changed
                .wait_timeout(lifecycle, wait)
                .map_err(|_| {
                    FfiError::new(
                        AGENT_RUNTIME_ERR_RUNTIME,
                        "internal",
                        "lifecycle lock poisoned",
                    )
                })?;
            lifecycle = next;
            if status.timed_out() && lifecycle.state != LifecycleState::Closed {
                return Err(FfiError::timeout("runtime shutdown is still in progress"));
            }
        }
    }));
    finish_status(result.unwrap_or_else(|_| Err(FfiError::panic())))
}

#[no_mangle]
pub extern "C" fn agent_runtime_destroy_v1(handle: AgentRuntimeHandle) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let entry = lookup_handle(handle)?;
        {
            let lifecycle = entry.lifecycle.lock().map_err(|_| {
                FfiError::new(
                    AGENT_RUNTIME_ERR_RUNTIME,
                    "internal",
                    "lifecycle lock poisoned",
                )
            })?;
            if lifecycle.state != LifecycleState::Closed {
                return Err(FfiError::bad_state(
                    "agent_runtime_shutdown_v1 must complete before destroy",
                ));
            }
            if lifecycle.active_calls != 0 {
                return Err(FfiError::bad_state("runtime still has active FFI calls"));
            }
        }
        handles()
            .lock()
            .map_err(|_| {
                FfiError::new(
                    AGENT_RUNTIME_ERR_RUNTIME,
                    "internal",
                    "handle registry poisoned",
                )
            })?
            .remove(&handle)
            .ok_or_else(FfiError::invalid_handle)?;
        Ok(())
    }));
    finish_status(result.unwrap_or_else(|_| Err(FfiError::panic())))
}

#[no_mangle]
pub extern "C" fn agent_runtime_last_error_json_v1() -> *const c_char {
    LAST_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|value| value.as_ptr())
            .unwrap_or(ptr::null())
    })
}

#[no_mangle]
pub extern "C" fn agent_runtime_free_string_v1(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(CString::from_raw(value));
    }));
}

pub(crate) fn ffi_error_code_invalid_config() -> c_int {
    AGENT_RUNTIME_ERR_INVALID_ARGUMENT
}

pub(crate) fn ffi_error_code_llm_failed() -> c_int {
    AGENT_RUNTIME_ERR_RUNTIME
}

pub(crate) fn ffi_error_code_rpc_failed() -> c_int {
    AGENT_RUNTIME_ERR_RUNTIME
}

pub(crate) fn ffi_error_code_internal() -> c_int {
    AGENT_RUNTIME_ERR_RUNTIME
}

#[cfg(test)]
mod abi_tests {
    use super::*;

    #[test]
    fn permission_identifiers_accept_canonical_and_legacy_names() {
        let canonical = json!({
            "conversation_id": "conversation-1",
            "tool_call_id": "call-1"
        });
        let legacy = json!({
            "conversationId": "conversation-2",
            "toolCallId": "call-2"
        });

        for (payload, conversation_id, tool_call_id) in [
            (canonical, "conversation-1", "call-1"),
            (legacy, "conversation-2", "call-2"),
        ] {
            let payload = payload.as_object().unwrap();
            assert_eq!(
                required_string_with_aliases(payload, "conversation_id", &["conversationId"])
                    .unwrap(),
                conversation_id
            );
            assert_eq!(
                required_string_with_aliases(payload, "tool_call_id", &["toolCallId"]).unwrap(),
                tool_call_id
            );
        }
    }

    #[test]
    fn workflow_execution_context_requires_a_complete_identity_pair() {
        let canonical = json!({
            "conversation_id": "conversation-1",
            "agent_id": "agent-1"
        });
        let legacy = json!({
            "conversationId": "conversation-2",
            "agentId": "agent-2"
        });
        for payload in [canonical, legacy] {
            let context =
                optional_workflow_execution_context(payload.as_object().unwrap()).unwrap();
            assert!(context.conversation_id.is_some());
            assert!(context.agent_id.is_some());
        }

        let incomplete = json!({"conversation_id": "conversation-1"});
        let error =
            optional_workflow_execution_context(incomplete.as_object().unwrap()).unwrap_err();
        assert!(error
            .message
            .contains("conversation_id and payload.agent_id must be provided together"));

        let invalid = json!({"conversation_id": 7, "agent_id": "agent-1"});
        let error = optional_workflow_execution_context(invalid.as_object().unwrap()).unwrap_err();
        assert!(error
            .message
            .contains("payload.conversation_id must be a string"));
    }

    #[test]
    fn registration_input_accepts_structured_json() {
        let payload = json!({
            "input": {
                "schema": "agent-runtime-llm-registration/v1",
                "providers": []
            }
        });
        let input = registration_input(payload.as_object().unwrap()).unwrap();
        let value: Value = serde_json::from_str(&input).unwrap();
        assert_eq!(value["schema"], "agent-runtime-llm-registration/v1");
    }

    #[test]
    fn abi_metadata_is_self_describing() {
        assert_eq!(agent_runtime_abi_version_v1(), 1);
        let capabilities = unsafe { CStr::from_ptr(agent_runtime_capabilities_v1()) }
            .to_str()
            .unwrap();
        let value: Value = serde_json::from_str(capabilities).unwrap();
        assert_eq!(value["abi"]["major"], 1);
        assert_eq!(value["abi"]["minor"], 3);
        assert_eq!(value["events"]["transport"], "pull");
        assert_eq!(
            value["events"]["lines"]["workflow"]["conversation_scoped"],
            false
        );
        assert_eq!(
            value["events"]["lines"]["workflow"]["aggregate_id"],
            "payload.workflow_id"
        );
        let event_types = value["events"]["types"].as_array().unwrap();
        assert!(!event_types.iter().any(|event| event == "llm_usage"));
        assert!(!event_types.iter().any(|event| event == "llm_error"));
        assert!(event_types
            .iter()
            .any(|event| event == "conversation.state_delta"));
        assert!(!event_types
            .iter()
            .any(|event| event == "tool:permission-requested"));
        assert!(!event_types
            .iter()
            .any(|event| event == "tool:permission-resolved"));
        assert_eq!(value["threading"]["callbacks"], false);
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "runtime.reload_llm"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "runtime.get_tool_definitions"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "runtime.get_workflow_node_definitions"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "runtime.get_agent_cluster_definitions"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "runtime.get_rpc_endpoint_definitions"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "conversation.agent_tasks"));
        for command in [
            "workflow.create",
            "workflow.read",
            "workflow.register",
            "workflow.update",
            "workflow.compile",
            "workflow.delete",
            "workflow.list",
            "workflow.convert.script_to_blueprint",
            "workflow.convert.blueprint_to_script",
            "workflow.execute",
            "workflow.execute_script",
        ] {
            assert!(value["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| candidate == command));
        }
    }

    #[test]
    fn invalid_handle_returns_structured_error() {
        assert_eq!(
            agent_runtime_start_v1(AgentRuntimeHandle::MAX),
            AGENT_RUNTIME_ERR_INVALID_HANDLE
        );
        let error = unsafe { CStr::from_ptr(agent_runtime_last_error_json_v1()) }
            .to_str()
            .unwrap();
        let value: Value = serde_json::from_str(error).unwrap();
        assert_eq!(value["kind"], "invalid_handle");
    }

    #[test]
    fn error_envelope_uses_runtime_owned_string_contract() {
        let error = FfiError::unsupported("unsupported command type 'not.supported'");
        let envelope = error_envelope(None, "ffi_cmd_test", &error);
        let mut response = ptr::null_mut();
        write_owned_string(&mut response, envelope).unwrap();
        assert!(!response.is_null());
        let decoded: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(response) }.to_str().unwrap()).unwrap();
        assert_eq!(decoded["ok"], false);
        assert_eq!(decoded["error"]["kind"], "unsupported");
        agent_runtime_free_string_v1(response);
    }

    #[test]
    fn next_event_remains_readable_while_handle_is_closing() {
        let config = r#"{"schema":"agent-runtime-create-options/v1","log_level":"info","language":"zh","restore_policy":"strict"}"#;
        let mut facade = runtime::RuntimeFacade::create(config).unwrap();
        let (event_sender, event_receiver) = mpsc::channel();
        facade.set_event_sender(event_sender.clone());
        let entry = Arc::new(HandleEntry {
            facade: RwLock::new(facade),
            events: Mutex::new(event_receiver),
            lifecycle: Mutex::new(Lifecycle {
                state: LifecycleState::Closing,
                active_calls: 0,
                shutdown_started: true,
                shutdown_error: None,
            }),
            lifecycle_changed: Condvar::new(),
        });
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        handles().lock().unwrap().insert(handle, entry);
        event_sender
            .send(r#"{"type":"conversation.ledger_delta"}"#.to_string())
            .unwrap();

        let mut event = ptr::null_mut();
        assert_eq!(
            agent_runtime_next_event_v1(handle, 0, &mut event),
            AGENT_RUNTIME_OK
        );
        let text = unsafe { CStr::from_ptr(event) }.to_str().unwrap();
        assert_eq!(text, r#"{"type":"conversation.ledger_delta"}"#);
        agent_runtime_free_string_v1(event);
        handles().lock().unwrap().remove(&handle);
    }
}
