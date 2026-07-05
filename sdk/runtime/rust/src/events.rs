use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde_json::Value;

use crate::{
    RuntimeError, RuntimeEventReader, CONVERSATION_CLOSED_EVENT_TYPE,
    CONVERSATION_CREATED_EVENT_TYPE, ERR_BAD_STATE, FRONTEND_STATE_SNAPSHOT_EVENT_TYPE,
    LEDGER_DELTA_EVENT_TYPE, STATE_DELTA_EVENT_TYPE,
};

pub const PUBLIC_RUNTIME_EVENT_TYPES: &[&str] = &[
    CONVERSATION_CREATED_EVENT_TYPE,
    CONVERSATION_CLOSED_EVENT_TYPE,
    LEDGER_DELTA_EVENT_TYPE,
    STATE_DELTA_EVENT_TYPE,
    FRONTEND_STATE_SNAPSHOT_EVENT_TYPE,
];

#[derive(Debug, Clone)]
pub struct RuntimeDiagnostic {
    pub level: RuntimeDiagnosticLevel,
    pub message: String,
    pub code: Option<i32>,
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDiagnosticLevel {
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEventFilter {
    AllPublic,
    Conversation(String),
    EventType(String),
}

impl RuntimeEventFilter {
    fn matches(&self, event: &Value) -> bool {
        if !is_public_runtime_event(event) {
            return false;
        }
        match self {
            Self::AllPublic => true,
            Self::Conversation(conversation_id) => {
                conversation_id_from_event(event).as_deref() == Some(conversation_id.as_str())
            }
            Self::EventType(event_type) => {
                event.get("type").and_then(Value::as_str) == Some(event_type.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeEventBus {
    inner: Arc<Mutex<RuntimeEventBusInner>>,
}

#[derive(Debug, Default)]
struct RuntimeEventBusInner {
    next_id: u64,
    subscribers: Vec<RuntimeEventSubscriber>,
}

#[derive(Debug)]
struct RuntimeEventSubscriber {
    id: u64,
    filter: RuntimeEventFilter,
    sender: mpsc::Sender<Value>,
}

#[derive(Debug)]
pub struct RuntimeEventSubscription {
    id: u64,
    bus: RuntimeEventBus,
    receiver: mpsc::Receiver<Value>,
}

impl RuntimeEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, filter: RuntimeEventFilter) -> RuntimeEventSubscription {
        let (sender, receiver) = mpsc::channel();
        let id = match self.inner.lock() {
            Ok(mut inner) => {
                let id = inner.next_id;
                inner.next_id = inner.next_id.saturating_add(1);
                inner
                    .subscribers
                    .push(RuntimeEventSubscriber { id, filter, sender });
                id
            }
            Err(_) => 0,
        };
        RuntimeEventSubscription {
            id,
            bus: self.clone(),
            receiver,
        }
    }

    pub fn subscribe_all(&self) -> RuntimeEventSubscription {
        self.subscribe(RuntimeEventFilter::AllPublic)
    }

    pub fn subscribe_conversation(
        &self,
        conversation_id: impl Into<String>,
    ) -> RuntimeEventSubscription {
        self.subscribe(RuntimeEventFilter::Conversation(conversation_id.into()))
    }

    pub fn publish(&self, event: Value) -> usize {
        if !is_public_runtime_event(&event) {
            return 0;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return 0;
        };
        let mut delivered = 0usize;
        inner.subscribers.retain(|subscriber| {
            if !subscriber.filter.matches(&event) {
                return true;
            }
            match subscriber.sender.send(event.clone()) {
                Ok(()) => {
                    delivered += 1;
                    true
                }
                Err(_) => false,
            }
        });
        delivered
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.subscribers.len())
            .unwrap_or(0)
    }

    fn unsubscribe(&self, id: u64) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.subscribers.retain(|subscriber| subscriber.id != id);
    }
}

impl RuntimeEventSubscription {
    pub fn recv(&self) -> std::result::Result<Value, mpsc::RecvError> {
        self.receiver.recv()
    }

    pub fn try_recv(&self) -> std::result::Result<Value, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Drop for RuntimeEventSubscription {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.id);
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeEventPumpOptions {
    pub poll_timeout_ms: u32,
}

impl Default for RuntimeEventPumpOptions {
    fn default() -> Self {
        Self {
            poll_timeout_ms: 250,
        }
    }
}

pub struct RuntimeEventPump {
    reader: RuntimeEventReader,
    bus: RuntimeEventBus,
    options: RuntimeEventPumpOptions,
    diagnostics: Option<mpsc::Sender<RuntimeDiagnostic>>,
}

pub struct RuntimeEventPumpHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl RuntimeEventPump {
    pub fn new(reader: RuntimeEventReader, bus: RuntimeEventBus) -> Self {
        Self {
            reader,
            bus,
            options: RuntimeEventPumpOptions::default(),
            diagnostics: None,
        }
    }

    pub fn options(mut self, options: RuntimeEventPumpOptions) -> Self {
        self.options = options;
        self
    }

    pub fn diagnostics(mut self, sender: mpsc::Sender<RuntimeDiagnostic>) -> Self {
        self.diagnostics = Some(sender);
        self
    }

    pub fn spawn(self) -> RuntimeEventPumpHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let join = thread::spawn(move || {
            run_event_pump(
                self.reader,
                self.bus,
                self.options,
                self.diagnostics,
                thread_stop,
            );
        });
        RuntimeEventPumpHandle {
            stop,
            join: Some(join),
        }
    }
}

impl RuntimeEventPumpHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn join(mut self) -> thread::Result<()> {
        if let Some(join) = self.join.take() {
            join.join()
        } else {
            Ok(())
        }
    }
}

impl Drop for RuntimeEventPumpHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run_event_pump(
    reader: RuntimeEventReader,
    bus: RuntimeEventBus,
    options: RuntimeEventPumpOptions,
    diagnostics: Option<mpsc::Sender<RuntimeDiagnostic>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        match reader.next_event(options.poll_timeout_ms) {
            Ok(Some(event)) => {
                bus.publish(event);
            }
            Ok(None) => {}
            Err(error) if error.code() == Some(ERR_BAD_STATE) => return,
            Err(error) => emit_diagnostic(&diagnostics, diagnostic_from_error(error)),
        }
    }
}

fn emit_diagnostic(
    diagnostics: &Option<mpsc::Sender<RuntimeDiagnostic>>,
    diagnostic: RuntimeDiagnostic,
) {
    if let Some(sender) = diagnostics {
        let _ = sender.send(diagnostic);
    }
}

fn diagnostic_from_error(error: RuntimeError) -> RuntimeDiagnostic {
    RuntimeDiagnostic {
        level: RuntimeDiagnosticLevel::Error,
        message: error.to_string(),
        code: error.code(),
        detail: error.detail().cloned(),
    }
}

pub fn is_public_runtime_event(event: &Value) -> bool {
    event
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| PUBLIC_RUNTIME_EVENT_TYPES.contains(&event_type))
}

pub fn conversation_id_from_event(event: &Value) -> Option<String> {
    event
        .get("conversation_id")
        .or_else(|| event.pointer("/payload/conversation_id"))
        .or_else(|| event.pointer("/payload/record/conversation_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn public_event_filter_excludes_telemetry_mirrors() {
        assert!(is_public_runtime_event(&json!({
            "type": "conversation.ledger_delta",
            "conversation_id": "conv_1",
            "payload": {}
        })));
        assert!(!is_public_runtime_event(&json!({
            "type": "llm_usage",
            "conversation_id": "conv_1",
            "payload": {}
        })));
        assert!(!is_public_runtime_event(&json!({
            "type": "workflow_studio:draft_update",
            "conversation_id": "studio_1_editor",
            "payload": {}
        })));
    }

    #[test]
    fn bus_delivers_public_events_to_matching_conversation() {
        let bus = RuntimeEventBus::new();
        let all = bus.subscribe_all();
        let conv_one = bus.subscribe_conversation("conv_1");
        let conv_two = bus.subscribe_conversation("conv_2");

        let delivered = bus.publish(json!({
            "type": "frontend:state_snapshot",
            "conversation_id": "conv_1",
            "payload": {}
        }));

        assert_eq!(delivered, 2);
        assert_eq!(
            all.try_recv()
                .unwrap()
                .get("conversation_id")
                .and_then(Value::as_str),
            Some("conv_1")
        );
        assert_eq!(
            conv_one
                .try_recv()
                .unwrap()
                .get("conversation_id")
                .and_then(Value::as_str),
            Some("conv_1")
        );
        assert!(matches!(
            conv_two.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }
}
