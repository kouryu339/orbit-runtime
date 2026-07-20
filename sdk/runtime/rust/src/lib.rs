use std::ffi::{c_char, c_int, c_uint, CStr, CString};
use std::fmt;
use std::path::{Path, PathBuf};

use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod events;
mod host;
mod registry;
pub mod release;

pub use events::{
    conversation_id_from_event, is_public_runtime_event, is_workflow_event, RuntimeDiagnostic,
    RuntimeDiagnosticLevel, RuntimeEventBus, RuntimeEventFilter, RuntimeEventPump,
    RuntimeEventPumpHandle, RuntimeEventPumpOptions, RuntimeEventSubscription,
    PUBLIC_RUNTIME_EVENT_TYPES,
};
pub use host::{
    normalize_llm_registration, read_llm_registration_path, RuntimeApp, RuntimeHostBuilder,
    RuntimeRegistration, ToolPermissionDecision, DEFAULT_EMPTY_LLM_REGISTRATION_ID,
};
pub use registry::{
    ConversationInstanceInfo, ConversationPosition, ConversationRegistry,
    ConversationRegistryAction,
};

pub const ABI_VERSION: u32 = 1;

pub const OK: c_int = 0;
pub const ERR_INVALID_ARGUMENT: c_int = 1;
pub const ERR_INVALID_HANDLE: c_int = 2;
pub const ERR_BAD_STATE: c_int = 3;
pub const ERR_TIMEOUT: c_int = 4;
pub const ERR_UNSUPPORTED: c_int = 5;
pub const ERR_RUNTIME: c_int = 100;
pub const ERR_PANIC: c_int = 101;

pub const CONVERSATION_CREATED_EVENT_TYPE: &str = "conversation:created";
pub const CONVERSATION_CLOSED_EVENT_TYPE: &str = "conversation:closed";
pub const LEDGER_DELTA_EVENT_TYPE: &str = "conversation.ledger_delta";
pub const LEDGER_DELTA_SCHEMA: &str = "agent-runtime-ledger-delta/v1";
pub const STATE_DELTA_EVENT_TYPE: &str = "conversation.state_delta";
pub const STATE_DELTA_SCHEMA: &str = "agent-runtime-state-delta/v1";
pub const FRONTEND_STATE_SNAPSHOT_EVENT_TYPE: &str = "frontend:state_snapshot";
pub const WORKFLOW_RESOURCE_CHANGED_EVENT_TYPE: &str = "workflow.resource_changed";
pub const WORKFLOW_EXECUTION_COMPLETED_EVENT_TYPE: &str = "workflow.execution_completed";

pub type AgentRuntimeHandle = u64;

type AbiVersionFn = unsafe extern "C" fn() -> u32;
type StaticStringFn = unsafe extern "C" fn() -> *const c_char;
type CreateFn = unsafe extern "C" fn(*const c_char, *mut AgentRuntimeHandle) -> c_int;
type StartFn = unsafe extern "C" fn(AgentRuntimeHandle) -> c_int;
type InvokeFn = unsafe extern "C" fn(AgentRuntimeHandle, *const c_char, *mut *mut c_char) -> c_int;
type NextEventFn = unsafe extern "C" fn(AgentRuntimeHandle, c_uint, *mut *mut c_char) -> c_int;
type ShutdownFn = unsafe extern "C" fn(AgentRuntimeHandle, c_uint) -> c_int;
type DestroyFn = unsafe extern "C" fn(AgentRuntimeHandle) -> c_int;
type FreeStringFn = unsafe extern "C" fn(*mut c_char);

#[derive(Debug, Clone)]
pub struct RuntimeError {
    code: Option<c_int>,
    message: String,
    detail: Option<Value>,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_code(code: c_int, message: impl Into<String>, detail: Option<Value>) -> Self {
        Self {
            code: Some(code),
            message: message.into(),
            detail,
        }
    }

    pub fn code(&self) -> Option<c_int> {
        self.code
    }

    pub fn detail(&self) -> Option<&Value> {
        self.detail.as_ref()
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.code, self.detail.as_ref()) {
            (Some(code), Some(detail)) => write!(formatter, "{} ({code}): {detail}", self.message),
            (Some(code), None) => write!(formatter, "{} ({code})", self.message),
            (None, Some(detail)) => write!(formatter, "{}: {detail}", self.message),
            (None, None) => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeCreateOptions {
    pub schema: String,
    pub log_level: String,
    pub language: String,
    pub restore_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
}

impl Default for RuntimeCreateOptions {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-create-options/v1".to_string(),
            log_level: "info".to_string(),
            language: "zh-CN".to_string(),
            restore_policy: "strict".to_string(),
            data_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversationSpawnOptions {
    pub schema: String,
    pub cluster_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_host_context: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<ToolPermissionPolicyOverride>,
}

impl Default for ConversationSpawnOptions {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-conversation-spawn/v1".to_string(),
            cluster_id: String::new(),
            tool_host_context: None,
            permissions: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionMode {
    Full,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPermissionPolicyOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<ToolPermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controlled_change: Option<ToolPermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive: Option<ToolPermissionMode>,
}

#[derive(Debug, Clone)]
pub struct RuntimeCommandOptions {
    pub request_id: Option<String>,
    pub command_id: Option<String>,
}

impl RuntimeCommandOptions {
    pub fn empty() -> Self {
        Self {
            request_id: None,
            command_id: None,
        }
    }
}

impl Default for RuntimeCommandOptions {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone)]
pub struct ConversationInfo {
    pub json: Value,
    pub conversation_id: String,
    pub scope_id: Option<String>,
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AdmissionResult {
    pub json: Value,
    pub command_id: Option<String>,
    pub decision: Option<String>,
    pub reason: Option<String>,
}

impl AdmissionResult {
    pub fn accepted(&self) -> bool {
        self.decision.as_deref() == Some("accepted")
    }
}

pub struct Runtime {
    _library: Library,
    handle: AgentRuntimeHandle,
    version: String,
    capabilities: Value,
    create: CreateFn,
    start: StartFn,
    invoke_fn: InvokeFn,
    next_event: NextEventFn,
    shutdown: ShutdownFn,
    destroy: DestroyFn,
    last_error: StaticStringFn,
    free_string: FreeStringFn,
    shutdown_complete: bool,
    closed: bool,
}

#[derive(Clone, Copy)]
pub struct RuntimeEventReader {
    handle: AgentRuntimeHandle,
    next_event: NextEventFn,
    last_error: StaticStringFn,
    free_string: FreeStringFn,
}

impl Runtime {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let library = unsafe { Library::new(path) }.map_err(|error| {
            RuntimeError::new(format!(
                "load runtime library '{}': {error}",
                path.display()
            ))
        })?;
        unsafe {
            let abi_version = *library
                .get::<AbiVersionFn>(b"agent_runtime_abi_version_v1\0")
                .map_err(|error| RuntimeError::new(format!("load abi_version symbol: {error}")))?;
            let version = *library
                .get::<StaticStringFn>(b"agent_runtime_version_v1\0")
                .map_err(|error| RuntimeError::new(format!("load version symbol: {error}")))?;
            let capabilities = *library
                .get::<StaticStringFn>(b"agent_runtime_capabilities_v1\0")
                .map_err(|error| RuntimeError::new(format!("load capabilities symbol: {error}")))?;
            let create = *library
                .get::<CreateFn>(b"agent_runtime_create_v1\0")
                .map_err(|error| RuntimeError::new(format!("load create symbol: {error}")))?;
            let start = *library
                .get::<StartFn>(b"agent_runtime_start_v1\0")
                .map_err(|error| RuntimeError::new(format!("load start symbol: {error}")))?;
            let invoke_fn = *library
                .get::<InvokeFn>(b"agent_runtime_invoke_v1\0")
                .map_err(|error| RuntimeError::new(format!("load invoke symbol: {error}")))?;
            let next_event = *library
                .get::<NextEventFn>(b"agent_runtime_next_event_v1\0")
                .map_err(|error| RuntimeError::new(format!("load next_event symbol: {error}")))?;
            let shutdown = *library
                .get::<ShutdownFn>(b"agent_runtime_shutdown_v1\0")
                .map_err(|error| RuntimeError::new(format!("load shutdown symbol: {error}")))?;
            let destroy = *library
                .get::<DestroyFn>(b"agent_runtime_destroy_v1\0")
                .map_err(|error| RuntimeError::new(format!("load destroy symbol: {error}")))?;
            let last_error = *library
                .get::<StaticStringFn>(b"agent_runtime_last_error_json_v1\0")
                .map_err(|error| RuntimeError::new(format!("load last_error symbol: {error}")))?;
            let free_string = *library
                .get::<FreeStringFn>(b"agent_runtime_free_string_v1\0")
                .map_err(|error| RuntimeError::new(format!("load free_string symbol: {error}")))?;

            let actual_abi = abi_version();
            if actual_abi != ABI_VERSION {
                return Err(RuntimeError::new(format!(
                    "incompatible runtime ABI: expected {ABI_VERSION}, got {actual_abi}"
                )));
            }

            let version_text = borrowed_string(version());
            let capabilities_text = borrowed_string(capabilities());
            let capabilities_json = serde_json::from_str(&capabilities_text).map_err(|error| {
                RuntimeError::new(format!("parse agent runtime capabilities JSON: {error}"))
            })?;

            Ok(Self {
                _library: library,
                handle: 0,
                version: version_text,
                capabilities: capabilities_json,
                create,
                start,
                invoke_fn,
                next_event,
                shutdown,
                destroy,
                last_error,
                free_string,
                shutdown_complete: false,
                closed: false,
            })
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn capabilities(&self) -> &Value {
        &self.capabilities
    }

    pub fn handle(&self) -> AgentRuntimeHandle {
        self.handle
    }

    pub fn supports_command(&self, command_type: &str) -> bool {
        self.capabilities
            .get("commands")
            .and_then(Value::as_array)
            .is_some_and(|commands| {
                commands
                    .iter()
                    .any(|command| command.as_str() == Some(command_type))
            })
    }

    pub fn require_command(&self, command_type: &str) -> Result<()> {
        if self.supports_command(command_type) {
            Ok(())
        } else {
            Err(RuntimeError::with_code(
                ERR_UNSUPPORTED,
                format!("runtime does not support required command '{command_type}'"),
                Some(self.capabilities.clone()),
            ))
        }
    }

    pub fn create_with_options(&mut self, options: &RuntimeCreateOptions) -> Result<()> {
        let text = serde_json::to_string(options).map_err(|error| {
            RuntimeError::new(format!("serialize runtime create options: {error}"))
        })?;
        self.create(&text)
    }

    pub fn create(&mut self, create_options_json: &str) -> Result<()> {
        let input = cstring(create_options_json.to_string())?;
        let mut handle = 0;
        let code = unsafe { (self.create)(input.as_ptr(), &mut handle) };
        self.raise_for_code(code, None)?;
        self.handle = handle;
        self.shutdown_complete = false;
        self.closed = false;
        Ok(())
    }

    pub fn start(&mut self) -> Result<()> {
        let code = unsafe { (self.start)(self.handle) };
        self.raise_for_code(code, None)
    }

    pub fn register_resources(&mut self, registration: Value) -> Result<Value> {
        self.invoke(
            "runtime.register_resources",
            json!({ "registration": registration }),
        )
    }

    pub fn register_resources_path(&mut self, path: impl AsRef<Path>) -> Result<Value> {
        self.invoke(
            "runtime.register_resources",
            json!({ "input": path.as_ref().display().to_string() }),
        )
    }

    pub fn create_workflow_draft(&mut self, resource: Value) -> Result<Value> {
        self.invoke("workflow.create", json!({ "resource": resource }))
    }

    pub fn read_workflow(&mut self, id: &str) -> Result<Value> {
        self.invoke("workflow.read", json!({ "id": id }))
    }

    pub fn register_workflow_draft(
        &mut self,
        id: &str,
        expected_revision: Option<u64>,
        name: Option<&str>,
    ) -> Result<Value> {
        self.invoke(
            "workflow.register",
            json!({ "id": id, "expected_revision": expected_revision, "name": name }),
        )
    }

    pub fn update_workflow(
        &mut self,
        resource: Value,
        expected_revision: Option<u64>,
    ) -> Result<Value> {
        self.invoke(
            "workflow.update",
            json!({ "resource": resource, "expected_revision": expected_revision }),
        )
    }

    pub fn compile_workflow_draft(&mut self, id: &str) -> Result<Value> {
        self.invoke("workflow.compile", json!({ "id": id }))
    }

    pub fn workflow_script_to_blueprint(&mut self, script: &str) -> Result<Value> {
        self.invoke(
            "workflow.convert.script_to_blueprint",
            json!({ "script": script }),
        )
    }

    pub fn workflow_blueprint_to_script(&mut self, blueprint: Value) -> Result<Value> {
        self.invoke(
            "workflow.convert.blueprint_to_script",
            json!({ "blueprint": blueprint }),
        )
    }

    pub fn delete_workflow(&mut self, id: &str, expected_revision: Option<u64>) -> Result<Value> {
        self.invoke(
            "workflow.delete",
            json!({ "id": id, "expected_revision": expected_revision }),
        )
    }

    pub fn list_workflows(&mut self, kind: Option<&str>) -> Result<Value> {
        self.invoke("workflow.list", json!({ "kind": kind }))
    }

    pub fn execute_workflow(&mut self, id: &str, inputs: Value, trace: bool) -> Result<Value> {
        self.invoke(
            "workflow.execute",
            json!({ "id": id, "inputs": inputs, "trace": trace }),
        )
    }

    pub fn test_workflow_draft(&mut self, id: &str, inputs: Value, trace: bool) -> Result<Value> {
        self.invoke(
            "workflow.execute",
            json!({ "id": id, "mode": "test", "inputs": inputs, "trace": trace }),
        )
    }

    pub fn execute_workflow_script(
        &mut self,
        script: &str,
        inputs: Value,
        trace: bool,
    ) -> Result<Value> {
        self.invoke(
            "workflow.execute_script",
            json!({ "script": script, "inputs": inputs, "trace": trace }),
        )
    }

    pub fn register_llm(&mut self, registration: Value) -> Result<Value> {
        self.invoke(
            "runtime.register_llm",
            json!({ "registration": registration }),
        )
    }

    pub fn register_llm_path(&mut self, path: impl AsRef<Path>) -> Result<Value> {
        self.invoke(
            "runtime.register_llm",
            json!({ "input": path.as_ref().display().to_string() }),
        )
    }

    pub fn register_agent_cluster(&mut self, registration: Value) -> Result<Value> {
        self.invoke(
            "runtime.register_agent_cluster",
            json!({ "registration": registration }),
        )
    }

    pub fn register_agent_cluster_path(&mut self, path: impl AsRef<Path>) -> Result<Value> {
        self.invoke(
            "runtime.register_agent_cluster",
            json!({ "input": path.as_ref().display().to_string() }),
        )
    }

    pub fn spawn_conversation(
        &mut self,
        options: ConversationSpawnOptions,
    ) -> Result<ConversationInfo> {
        let payload = serde_json::to_value(options).map_err(|error| {
            RuntimeError::new(format!("serialize conversation spawn options: {error}"))
        })?;
        self.spawn_conversation_value(payload)
    }

    pub fn spawn_conversation_value(&mut self, payload: Value) -> Result<ConversationInfo> {
        conversation_info_from_value(self.invoke("conversation.spawn", payload)?)
    }

    pub fn send_message(
        &mut self,
        conversation_id: &str,
        content: &str,
    ) -> Result<AdmissionResult> {
        let result = self.invoke(
            "conversation.send_message",
            json!({ "conversation_id": conversation_id, "content": content }),
        )?;
        Ok(admission_from_value(result))
    }

    pub fn invoke(&mut self, command_type: &str, payload: Value) -> Result<Value> {
        self.invoke_with_options(command_type, payload, RuntimeCommandOptions::default())
    }

    pub fn invoke_with_options(
        &mut self,
        command_type: &str,
        payload: Value,
        options: RuntimeCommandOptions,
    ) -> Result<Value> {
        let mut request = json!({
            "schema": "agent-runtime-command/v1",
            "type": command_type,
            "payload": payload
        });
        if let Some(request_id) = options.request_id {
            request["id"] = json!(request_id);
        }
        if let Some(command_id) = options.command_id {
            request["command_id"] = json!(command_id);
        }
        let request_text = request.to_string();
        let request = cstring(request_text)?;
        let mut output: *mut c_char = std::ptr::null_mut();
        let code = unsafe { (self.invoke_fn)(self.handle, request.as_ptr(), &mut output) };
        let envelope = unsafe { self.take_json(output) };
        let error = envelope.get("error").cloned();
        self.raise_for_code(code, error)?;
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn next_event(&self, timeout_ms: u32) -> Result<Option<Value>> {
        self.event_reader().next_event(timeout_ms)
    }

    pub fn event_reader(&self) -> RuntimeEventReader {
        RuntimeEventReader {
            handle: self.handle,
            next_event: self.next_event,
            last_error: self.last_error,
            free_string: self.free_string,
        }
    }

    pub fn shutdown(&mut self, timeout_ms: u32) -> Result<()> {
        if self.closed || self.handle == 0 {
            return Ok(());
        }
        let code = unsafe { (self.shutdown)(self.handle, timeout_ms) };
        self.raise_for_code(code, None)?;
        self.shutdown_complete = true;
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<()> {
        if self.closed || self.handle == 0 {
            return Ok(());
        }
        let code = unsafe { (self.destroy)(self.handle) };
        self.raise_for_code(code, None)?;
        self.closed = true;
        self.shutdown_complete = true;
        self.handle = 0;
        Ok(())
    }

    pub fn close(&mut self, timeout_ms: u32) -> Result<()> {
        self.shutdown(timeout_ms)?;
        self.destroy()
    }

    fn raise_for_code(&self, code: c_int, error: Option<Value>) -> Result<()> {
        if code == OK {
            return Ok(());
        }
        let detail = error.or_else(|| {
            let text = unsafe { borrowed_string((self.last_error)()) };
            serde_json::from_str(&text)
                .ok()
                .or_else(|| (!text.is_empty()).then_some(json!(text)))
        });
        let code_name = match code {
            ERR_INVALID_ARGUMENT => "INVALID_ARGUMENT",
            ERR_INVALID_HANDLE => "INVALID_HANDLE",
            ERR_BAD_STATE => "BAD_STATE",
            ERR_TIMEOUT => "TIMEOUT",
            ERR_UNSUPPORTED => "UNSUPPORTED",
            ERR_PANIC => "PANIC",
            _ => "RUNTIME",
        };
        Err(RuntimeError::with_code(
            code,
            format!("{code_name} runtime call failed"),
            detail,
        ))
    }

    unsafe fn take_json(&self, pointer: *mut c_char) -> Value {
        if pointer.is_null() {
            return Value::Null;
        }
        let text = borrowed_string(pointer);
        (self.free_string)(pointer);
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.closed || self.handle == 0 {
            return;
        }
        unsafe {
            if !self.shutdown_complete {
                let _ = (self.shutdown)(self.handle, 10_000);
            }
            let _ = (self.destroy)(self.handle);
        }
        self.closed = true;
        self.handle = 0;
    }
}

impl RuntimeEventReader {
    pub fn next_event(&self, timeout_ms: u32) -> Result<Option<Value>> {
        let mut output: *mut c_char = std::ptr::null_mut();
        let code = unsafe { (self.next_event)(self.handle, timeout_ms, &mut output) };
        if code == ERR_TIMEOUT {
            return Ok(None);
        }
        if code != OK {
            let detail = unsafe {
                let text = borrowed_string((self.last_error)());
                serde_json::from_str(&text)
                    .ok()
                    .or_else(|| (!text.is_empty()).then_some(json!(text)))
            };
            return Err(RuntimeError::with_code(
                code,
                "runtime event read failed",
                detail,
            ));
        }
        if output.is_null() {
            return Ok(None);
        }
        let text = unsafe { borrowed_string(output) };
        unsafe { (self.free_string)(output) };
        Ok(Some(
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })),
        ))
    }
}

pub fn ledger_delta_from_event(event: &Value) -> Option<&Value> {
    if event.get("type").and_then(Value::as_str) != Some(LEDGER_DELTA_EVENT_TYPE) {
        return None;
    }
    let payload = event.get("payload")?;
    (payload.get("schema").and_then(Value::as_str) == Some(LEDGER_DELTA_SCHEMA)).then_some(payload)
}

pub fn is_ledger_delta_event(event: &Value) -> bool {
    ledger_delta_from_event(event).is_some()
}

pub fn state_delta_from_event(event: &Value) -> Option<&Value> {
    if event.get("type").and_then(Value::as_str) != Some(STATE_DELTA_EVENT_TYPE) {
        return None;
    }
    let payload = event.get("payload")?;
    (payload.get("schema").and_then(Value::as_str) == Some(STATE_DELTA_SCHEMA)).then_some(payload)
}

pub fn is_state_delta_event(event: &Value) -> bool {
    state_delta_from_event(event).is_some()
}

fn conversation_info_from_value(value: Value) -> Result<ConversationInfo> {
    let conversation_id = value
        .get("conversation_id")
        .or_else(|| value.get("conversationId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            RuntimeError::new(format!(
                "conversation result missing conversation_id: {value}"
            ))
        })?;
    Ok(ConversationInfo {
        scope_id: string_field(&value, "scope_id"),
        tenant_id: string_field(&value, "tenant_id"),
        user_id: string_field(&value, "user_id"),
        created_at: string_field(&value, "created_at"),
        json: value,
        conversation_id,
    })
}

fn admission_from_value(value: Value) -> AdmissionResult {
    AdmissionResult {
        command_id: string_field(&value, "command_id"),
        decision: string_field(&value, "decision"),
        reason: string_field(&value, "reason"),
        json: value,
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

fn cstring(value: String) -> Result<CString> {
    CString::new(value).map_err(|_| RuntimeError::new("string contains an interior NUL byte"))
}

unsafe fn borrowed_string(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    CStr::from_ptr(pointer).to_string_lossy().into_owned()
}
