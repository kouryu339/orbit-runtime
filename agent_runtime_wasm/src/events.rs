use corework::event::{BaseEvent, EventHandler};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

#[derive(Default)]
pub struct EventProjector {
    next: AtomicU64,
    conversations: Mutex<std::collections::HashMap<String, u64>>,
    buffered_resources: Mutex<Option<Vec<Value>>>,
}
impl EventProjector {
    pub fn begin_resource_commit(&self) {
        *self.buffered_resources.lock().unwrap() = Some(Vec::new());
    }
    pub fn finish_resource_commit(&self, accepted: bool) {
        let buffered = self
            .buffered_resources
            .lock()
            .unwrap()
            .take()
            .unwrap_or_default();
        if accepted {
            for event in buffered {
                self.deliver(event);
            }
        }
    }
    fn deliver(&self, mut event: Value) {
        event["event_seq"] = json!(self.next.fetch_add(1, Ordering::Relaxed) + 1);
        super::bridge::emit(&event);
    }
    pub fn emit(&self, event: &BaseEvent) {
        let event_type = match event.event_type.as_str() {
            "runtime:conversation_created" => "conversation:created",
            "runtime:conversation_closed" => "conversation:closed",
            other => other,
        };
        let mut envelope = json!({"schema":"agent-runtime-event/v1","type":event_type,"source":event.event_type,
            "event_id":event.event_id,"timestamp":event.timestamp,"payload":event.payload});
        if let Some(id) = event
            .conversation_id
            .as_deref()
            .or_else(|| event.payload.get("conversation_id").and_then(Value::as_str))
        {
            envelope["conversation_id"] = json!(id);
            let mut sequences = self.conversations.lock().unwrap();
            let sequence = sequences.entry(id.into()).or_default();
            *sequence += 1;
            envelope["conversation_event_seq"] = json!(*sequence);
            if event_type == "conversation:closed" {
                sequences.remove(id);
            }
        }
        if event_type == "workflow.resource_changed" {
            if let Some(buffer) = self.buffered_resources.lock().unwrap().as_mut() {
                buffer.push(envelope);
                return;
            }
        }
        self.deliver(envelope);
    }
    pub async fn install(
        self: &Arc<Self>,
        bus: &Arc<corework::event::InMemoryEventBus>,
    ) -> corework::error::Result<()> {
        use corework::event::EventBus;
        for name in [
            "frontend:state_snapshot",
            "conversation.ledger_delta",
            "conversation.state_delta",
            "runtime:conversation_created",
            "runtime:conversation_closed",
            "runtime.rpc_catalog_changed",
            "workflow.resource_changed",
            "workflow.execution_started",
            "workflow.node_started",
            "workflow.node_completed",
            "workflow.node_failed",
            "workflow.trace_event",
            "workflow.execution_completed",
        ] {
            bus.subscribe(name.into(), self.clone()).await?;
        }
        Ok(())
    }
}
#[async_trait::async_trait]
impl EventHandler for EventProjector {
    async fn handle(&self, event: &BaseEvent) -> corework::error::Result<()> {
        self.emit(event);
        Ok(())
    }
}
