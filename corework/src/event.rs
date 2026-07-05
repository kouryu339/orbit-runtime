//!

use crate::error::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Runtime event contract.
pub trait Event: Send + Sync + Debug + Clone {
    fn event_type(&self) -> &str;

    /// Stable event identifier.
    fn event_id(&self) -> String;

    fn timestamp(&self) -> chrono::DateTime<chrono::Utc>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseEvent {
    pub event_type: String,
    pub event_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

impl Event for BaseEvent {
    fn event_type(&self) -> &str {
        &self.event_type
    }

    fn event_id(&self) -> String {
        self.event_id.clone()
    }

    fn timestamp(&self) -> chrono::DateTime<chrono::Utc> {
        self.timestamp
    }
}

impl BaseEvent {
    pub fn new(event_type: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            event_type: event_type.into(),
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            payload,
            scope_id: None,
            conversation_id: None,
        }
    }

    pub fn with_scope(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = Some(scope_id.into());
        self
    }

    pub fn with_conversation_id(mut self, conversation_id: impl Into<String>) -> Self {
        self.conversation_id = Some(conversation_id.into());
        self
    }
}

#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &BaseEvent) -> Result<()>;

    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

/// Event bus abstraction.
#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: BaseEvent) -> Result<()>;

    async fn subscribe(&self, event_type: String, handler: Arc<dyn EventHandler>) -> Result<()>;

    async fn unsubscribe(&self, event_type: &str, handler_name: &str) -> Result<()>;
}

pub struct InMemoryEventBus {
    subscribers: Arc<RwLock<HashMap<String, Vec<Arc<dyn EventHandler>>>>>,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(&self, event: BaseEvent) -> Result<()> {
        let subscribers = self.subscribers.read().await;

        if let Some(handlers) = subscribers.get(event.event_type()) {
            let mut tasks = Vec::new();

            for handler in handlers {
                let handler = handler.clone();
                let event = event.clone();

                tasks.push(tokio::spawn(async move {
                    if let Err(e) = handler.handle(&event).await {
                        tracing::error!(
                            "Handler {} failed for event {}: {}",
                            handler.name(),
                            event.event_id(),
                            e
                        );
                    }
                }));
            }

            for task in tasks {
                let _ = task.await;
            }
        }

        Ok(())
    }

    async fn subscribe(&self, event_type: String, handler: Arc<dyn EventHandler>) -> Result<()> {
        let mut subscribers = self.subscribers.write().await;

        subscribers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(handler);

        Ok(())
    }

    async fn unsubscribe(&self, event_type: &str, handler_name: &str) -> Result<()> {
        let mut subscribers = self.subscribers.write().await;

        if let Some(handlers) = subscribers.get_mut(event_type) {
            handlers.retain(|h| h.name() != handler_name);
        }

        Ok(())
    }
}

pub struct EventFilter {
    event_types: Vec<String>,
}

impl EventFilter {
    pub fn new() -> Self {
        Self {
            event_types: Vec::new(),
        }
    }

    pub fn add_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_types.push(event_type.into());
        self
    }

    pub fn matches(&self, event: &BaseEvent) -> bool {
        self.event_types.is_empty() || self.event_types.contains(&event.event_type)
    }
}

impl Default for EventFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_filter() {
        let filter = EventFilter::new()
            .add_type("order.created")
            .add_type("order.updated");

        let event1 = BaseEvent::new("order.created", serde_json::json!({}));
        let event2 = BaseEvent::new("payment.processed", serde_json::json!({}));

        assert!(filter.matches(&event1));
        assert!(!filter.matches(&event2));
    }
}
