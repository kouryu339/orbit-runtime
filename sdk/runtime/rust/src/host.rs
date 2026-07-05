use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{
    admission_from_value, conversation_info_from_value, AdmissionResult, ConversationInfo,
    ConversationSpawnOptions, Result, Runtime, RuntimeCreateOptions, RuntimeDiagnostic,
    RuntimeDiagnosticLevel, RuntimeError, RuntimeEventBus, RuntimeEventPump,
    RuntimeEventPumpHandle, RuntimeEventPumpOptions, RuntimeEventSubscription,
};

pub const DEFAULT_EMPTY_LLM_REGISTRATION_ID: &str = "runtime-host-llm";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPermissionDecision {
    Allow,
    Deny,
}

impl ToolPermissionDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone)]
pub enum RuntimeRegistration {
    Path(PathBuf),
    Value(Value),
}

impl RuntimeRegistration {
    fn register_resources(&self, runtime: &mut Runtime) -> Result<Value> {
        match self {
            Self::Path(path) => runtime.register_resources_path(path),
            Self::Value(value) => runtime.register_resources(value.clone()),
        }
    }

    fn register_agent_cluster(&self, runtime: &mut Runtime) -> Result<Value> {
        match self {
            Self::Path(path) => runtime.register_agent_cluster_path(path),
            Self::Value(value) => runtime.register_agent_cluster(value.clone()),
        }
    }
}

pub struct RuntimeHostBuilder {
    library_path: PathBuf,
    create_options: RuntimeCreateOptions,
    resources: Option<RuntimeRegistration>,
    llm: Option<RuntimeRegistration>,
    empty_llm_registration_id: String,
    agent_cluster: Option<RuntimeRegistration>,
    start_event_pump: bool,
    event_pump_options: RuntimeEventPumpOptions,
    diagnostics: Option<mpsc::Sender<RuntimeDiagnostic>>,
    slow_invoke_threshold: Option<Duration>,
}

impl RuntimeHostBuilder {
    pub fn new(library_path: impl Into<PathBuf>) -> Self {
        Self {
            library_path: library_path.into(),
            create_options: RuntimeCreateOptions::default(),
            resources: None,
            llm: None,
            empty_llm_registration_id: DEFAULT_EMPTY_LLM_REGISTRATION_ID.to_string(),
            agent_cluster: None,
            start_event_pump: true,
            event_pump_options: RuntimeEventPumpOptions::default(),
            diagnostics: None,
            slow_invoke_threshold: Some(Duration::from_millis(250)),
        }
    }

    pub fn create_options(mut self, options: RuntimeCreateOptions) -> Self {
        self.create_options = options;
        self
    }

    pub fn resources_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.resources = Some(RuntimeRegistration::Path(path.into()));
        self
    }

    pub fn resources(mut self, registration: Value) -> Self {
        self.resources = Some(RuntimeRegistration::Value(registration));
        self
    }

    pub fn llm_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.llm = Some(RuntimeRegistration::Path(path.into()));
        self
    }

    pub fn llm(mut self, registration: Value) -> Self {
        self.llm = Some(RuntimeRegistration::Value(registration));
        self
    }

    pub fn empty_llm_registration_id(mut self, id: impl Into<String>) -> Self {
        self.empty_llm_registration_id = id.into();
        self
    }

    pub fn agent_cluster_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.agent_cluster = Some(RuntimeRegistration::Path(path.into()));
        self
    }

    pub fn agent_cluster(mut self, registration: Value) -> Self {
        self.agent_cluster = Some(RuntimeRegistration::Value(registration));
        self
    }

    pub fn start_event_pump(mut self, value: bool) -> Self {
        self.start_event_pump = value;
        self
    }

    pub fn event_pump_options(mut self, options: RuntimeEventPumpOptions) -> Self {
        self.event_pump_options = options;
        self
    }

    pub fn diagnostics(mut self, sender: mpsc::Sender<RuntimeDiagnostic>) -> Self {
        self.diagnostics = Some(sender);
        self
    }

    pub fn slow_invoke_threshold(mut self, threshold: Option<Duration>) -> Self {
        self.slow_invoke_threshold = threshold;
        self
    }

    pub fn start(self) -> Result<RuntimeApp> {
        let resources = self
            .resources
            .ok_or_else(|| RuntimeError::new("runtime resources registration is required"))?;
        let agent_cluster = self
            .agent_cluster
            .ok_or_else(|| RuntimeError::new("runtime agent cluster registration is required"))?;

        let mut runtime = Runtime::load(&self.library_path)?;
        runtime.create_with_options(&self.create_options)?;
        resources.register_resources(&mut runtime)?;
        match self.llm {
            Some(llm) => {
                register_llm(&mut runtime, &llm, &self.empty_llm_registration_id)?;
            }
            None => {
                runtime.register_llm(empty_llm_registration(&self.empty_llm_registration_id))?;
            }
        }
        agent_cluster.register_agent_cluster(&mut runtime)?;
        runtime.start()?;

        let event_bus = RuntimeEventBus::new();
        let event_pump = if self.start_event_pump {
            let mut pump = RuntimeEventPump::new(runtime.event_reader(), event_bus.clone())
                .options(self.event_pump_options);
            if let Some(sender) = self.diagnostics.clone() {
                pump = pump.diagnostics(sender);
            }
            Some(pump.spawn())
        } else {
            None
        };

        Ok(RuntimeApp {
            inner: Arc::new(RuntimeAppInner {
                runtime: Mutex::new(runtime),
                event_bus,
                event_pump: Mutex::new(event_pump),
                diagnostics: self.diagnostics,
                slow_invoke_threshold: self.slow_invoke_threshold,
            }),
        })
    }
}

#[derive(Clone)]
pub struct RuntimeApp {
    inner: Arc<RuntimeAppInner>,
}

struct RuntimeAppInner {
    runtime: Mutex<Runtime>,
    event_bus: RuntimeEventBus,
    event_pump: Mutex<Option<RuntimeEventPumpHandle>>,
    diagnostics: Option<mpsc::Sender<RuntimeDiagnostic>>,
    slow_invoke_threshold: Option<Duration>,
}

impl RuntimeApp {
    pub fn event_bus(&self) -> RuntimeEventBus {
        self.inner.event_bus.clone()
    }

    pub fn subscribe_events(&self) -> RuntimeEventSubscription {
        self.inner.event_bus.subscribe_all()
    }

    pub fn subscribe_conversation(
        &self,
        conversation_id: impl Into<String>,
    ) -> RuntimeEventSubscription {
        self.inner.event_bus.subscribe_conversation(conversation_id)
    }

    pub fn version(&self) -> Result<String> {
        Ok(self.lock_runtime()?.version().to_string())
    }

    pub fn capabilities(&self) -> Result<Value> {
        Ok(self.lock_runtime()?.capabilities().clone())
    }

    pub fn invoke(&self, command_type: &str, payload: Value) -> Result<Value> {
        let started = Instant::now();
        let result = self.lock_runtime()?.invoke(command_type, payload);
        self.report_slow_invoke(command_type, started.elapsed());
        result
    }

    pub fn spawn_conversation(
        &self,
        options: ConversationSpawnOptions,
    ) -> Result<ConversationInfo> {
        self.lock_runtime()?.spawn_conversation(options)
    }

    pub fn send_message(&self, conversation_id: &str, content: &str) -> Result<AdmissionResult> {
        self.lock_runtime()?.send_message(conversation_id, content)
    }

    pub fn pause_conversation(&self, conversation_id: &str) -> Result<AdmissionResult> {
        let result = self.invoke(
            "conversation.pause",
            json!({ "conversation_id": conversation_id }),
        )?;
        Ok(admission_from_value(result))
    }

    pub fn close_conversation(&self, conversation_id: &str) -> Result<Value> {
        self.invoke(
            "conversation.close",
            json!({ "conversation_id": conversation_id }),
        )
    }

    pub fn export_conversation_snapshot(
        &self,
        conversation_id: &str,
        options: Option<Value>,
    ) -> Result<Value> {
        let mut payload = json!({ "conversation_id": conversation_id });
        if let Some(options) = options {
            payload["options"] = options;
        }
        self.invoke("conversation.export_snapshot", payload)
    }

    pub fn spawn_conversation_from_snapshot(
        &self,
        spawn: ConversationSpawnOptions,
        snapshot: Value,
    ) -> Result<ConversationInfo> {
        let spawn = serde_json::to_value(spawn).map_err(|error| {
            RuntimeError::new(format!("serialize conversation spawn options: {error}"))
        })?;
        conversation_info_from_value(self.invoke(
            "conversation.spawn_from_snapshot",
            json!({ "spawn": spawn, "snapshot": snapshot }),
        )?)
    }

    pub fn import_conversation_snapshot(
        &self,
        snapshot: Value,
        options: Option<Value>,
    ) -> Result<Value> {
        let mut payload = json!({ "snapshot": snapshot });
        if let Some(options) = options {
            payload["options"] = options;
        }
        self.invoke("conversation.import_snapshot", payload)
    }

    pub fn set_dynamic_snapshot(
        &self,
        conversation_id: &str,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> Result<Value> {
        self.invoke(
            "conversation.set_dynamic_snapshot",
            json!({
                "conversation_id": conversation_id,
                "agent_id": agent_id,
                "field_name": field_name,
                "text": text
            }),
        )
    }

    pub fn resolve_tool_permission(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        decision: &str,
    ) -> Result<Value> {
        self.invoke(
            "conversation.resolve_tool_permission",
            json!({
                "conversation_id": conversation_id,
                "tool_call_id": tool_call_id,
                "decision": decision
            }),
        )
    }

    pub fn decide_tool_permission(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        decision: ToolPermissionDecision,
    ) -> Result<Value> {
        self.resolve_tool_permission(conversation_id, tool_call_id, decision.as_str())
    }

    pub fn agent_tasks(&self, conversation_id: &str) -> Result<Value> {
        self.invoke(
            "conversation.agent_tasks",
            json!({ "conversation_id": conversation_id }),
        )
    }

    pub fn set_summary_model(
        &self,
        conversation_id: &str,
        model_name: &str,
    ) -> Result<AdmissionResult> {
        let result = self.invoke(
            "conversation.set_summary_model",
            json!({ "conversation_id": conversation_id, "model_name": model_name }),
        )?;
        Ok(admission_from_value(result))
    }

    pub fn compact_history(
        &self,
        conversation_id: &str,
        agent_ids: Option<Vec<String>>,
    ) -> Result<Value> {
        let mut payload = json!({ "conversation_id": conversation_id });
        if let Some(agent_ids) = agent_ids {
            payload["agent_ids"] = json!(agent_ids);
        }
        self.invoke("conversation.compact_history", payload)
    }

    pub fn reload_llm(&self, registration: Value) -> Result<Value> {
        self.invoke(
            "runtime.reload_llm",
            json!({ "registration": registration }),
        )
    }

    pub fn reload_llm_path(&self, path: impl AsRef<Path>) -> Result<Value> {
        self.invoke(
            "runtime.reload_llm",
            json!({ "input": path.as_ref().display().to_string() }),
        )
    }

    pub fn set_current_model(&self, model_uid: u64) -> Result<Value> {
        self.invoke(
            "runtime.set_current_model",
            json!({ "model_uid": model_uid }),
        )
    }

    pub fn get_provider_definitions(&self) -> Result<Value> {
        self.invoke("runtime.get_provider_definitions", json!({}))
    }

    pub fn open_workflow_studio(&self, options: Value) -> Result<Value> {
        self.invoke("studio.open_workflow", json!({ "options": options }))
    }

    pub fn open_agent_test_studio(&self, options: Value) -> Result<Value> {
        self.invoke("studio.open_agent_test", json!({ "options": options }))
    }

    pub fn start_event_pump(&self, options: RuntimeEventPumpOptions) -> Result<bool> {
        let mut event_pump = self
            .inner
            .event_pump
            .lock()
            .map_err(|_| RuntimeError::new("runtime event pump lock poisoned"))?;
        if event_pump.is_some() {
            return Ok(false);
        }
        let reader = self.lock_runtime()?.event_reader();
        let mut pump = RuntimeEventPump::new(reader, self.inner.event_bus.clone()).options(options);
        if let Some(sender) = self.inner.diagnostics.clone() {
            pump = pump.diagnostics(sender);
        }
        *event_pump = Some(pump.spawn());
        Ok(true)
    }

    pub fn shutdown(&self, timeout_ms: u32) -> Result<()> {
        self.lock_runtime()?.shutdown(timeout_ms)
    }

    pub fn destroy(&self) -> Result<()> {
        self.stop_event_pump();
        self.lock_runtime()?.destroy()
    }

    pub fn close(&self, timeout_ms: u32) -> Result<()> {
        self.lock_runtime()?.shutdown(timeout_ms)?;
        self.drain_event_pump();
        self.lock_runtime()?.destroy()
    }

    pub fn stop_event_pump(&self) {
        if let Some(pump) = self.take_event_pump() {
            pump.stop();
            let _ = pump.join();
        }
    }

    pub fn drain_event_pump(&self) {
        if let Some(pump) = self.take_event_pump() {
            let _ = pump.join();
        }
    }

    fn take_event_pump(&self) -> Option<RuntimeEventPumpHandle> {
        let Ok(mut pump) = self.inner.event_pump.lock() else {
            return None;
        };
        pump.take()
    }

    fn lock_runtime(&self) -> Result<MutexGuard<'_, Runtime>> {
        self.inner
            .runtime
            .lock()
            .map_err(|_| RuntimeError::new("runtime lock poisoned"))
    }

    fn report_slow_invoke(&self, command_type: &str, elapsed: Duration) {
        let Some(threshold) = self.inner.slow_invoke_threshold else {
            return;
        };
        if elapsed < threshold {
            return;
        }
        if let Some(sender) = &self.inner.diagnostics {
            let _ = sender.send(RuntimeDiagnostic {
                level: RuntimeDiagnosticLevel::Warn,
                message: format!(
                    "slow runtime invoke: command={command_type} elapsed_ms={}",
                    elapsed.as_millis()
                ),
                code: None,
                detail: Some(json!({
                    "command_type": command_type,
                    "elapsed_ms": elapsed.as_millis()
                })),
            });
        }
    }
}

impl Drop for RuntimeAppInner {
    fn drop(&mut self) {
        if let Ok(mut pump) = self.event_pump.lock() {
            if let Some(pump) = pump.take() {
                pump.stop();
                let _ = pump.join();
            }
        }
    }
}

fn empty_llm_registration(id: &str) -> Value {
    json!({
        "schema": "agent-runtime-llm-registration/v1",
        "id": id,
        "providers": [],
        "current_model_uid": null
    })
}

fn register_llm(
    runtime: &mut Runtime,
    registration: &RuntimeRegistration,
    default_id: &str,
) -> Result<Value> {
    match registration {
        RuntimeRegistration::Path(path) => {
            runtime.register_llm(read_llm_registration_path(path, default_id)?)
        }
        RuntimeRegistration::Value(value) => {
            runtime.register_llm(normalize_llm_registration(value.clone(), default_id)?)
        }
    }
}

pub fn read_llm_registration_path(path: impl AsRef<Path>, default_id: &str) -> Result<Value> {
    let path = path.as_ref();
    let content = fs::read_to_string(path).map_err(|error| {
        RuntimeError::new(format!(
            "read LLM registration '{}': {error}",
            path.display()
        ))
    })?;
    let value = serde_json::from_str(&content).map_err(|error| {
        RuntimeError::new(format!(
            "parse LLM registration '{}': {error}",
            path.display()
        ))
    })?;
    normalize_llm_registration(value, default_id)
}

pub fn normalize_llm_registration(mut value: Value, default_id: &str) -> Result<Value> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| RuntimeError::new("LLM registration root must be an object"))?;
    object.insert(
        "schema".to_string(),
        json!("agent-runtime-llm-registration/v1"),
    );
    if object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .is_none()
    {
        object.insert("id".to_string(), json!(default_id));
    }
    if let Some(current_model_uid) = object.remove("currentModelUid") {
        object
            .entry("current_model_uid".to_string())
            .or_insert(current_model_uid);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_permission_decision_serializes_to_runtime_values() {
        assert_eq!(ToolPermissionDecision::Allow.as_str(), "allow");
        assert_eq!(ToolPermissionDecision::Deny.as_str(), "deny");
    }

    #[test]
    fn empty_llm_registration_uses_runtime_schema() {
        let registration = empty_llm_registration("host-test");

        assert_eq!(
            registration.get("schema").and_then(Value::as_str),
            Some("agent-runtime-llm-registration/v1")
        );
        assert_eq!(
            registration.get("id").and_then(Value::as_str),
            Some("host-test")
        );
        assert_eq!(
            registration
                .get("providers")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn normalize_llm_registration_accepts_provider_config_shape() {
        let registration = normalize_llm_registration(
            json!({
                "schema": "agent-runtime-provider-config/v1",
                "currentModelUid": 42,
                "providers": []
            }),
            "host-test",
        )
        .unwrap();

        assert_eq!(
            registration.get("schema").and_then(Value::as_str),
            Some("agent-runtime-llm-registration/v1")
        );
        assert_eq!(
            registration.get("id").and_then(Value::as_str),
            Some("host-test")
        );
        assert_eq!(
            registration
                .get("current_model_uid")
                .and_then(Value::as_u64),
            Some(42)
        );
        assert!(registration.get("currentModelUid").is_none());
    }
}
