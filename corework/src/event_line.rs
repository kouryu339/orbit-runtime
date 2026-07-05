use crate::error::{FrameworkError, Result};
use crate::event::{BaseEvent, EventBus, EventHandler, InMemoryEventBus};
use crate::execution_unit::ExecutionUnit;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLineAccess {
    OwnerOnly,
    Subtree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLinePolicy {
    pub publish: EventLineAccess,
    pub subscribe: EventLineAccess,
}

impl EventLinePolicy {
    pub const fn private() -> Self {
        Self {
            publish: EventLineAccess::OwnerOnly,
            subscribe: EventLineAccess::OwnerOnly,
        }
    }

    pub const fn subtree_publish_owner_subscribe() -> Self {
        Self {
            publish: EventLineAccess::Subtree,
            subscribe: EventLineAccess::OwnerOnly,
        }
    }

    pub const fn subtree() -> Self {
        Self {
            publish: EventLineAccess::Subtree,
            subscribe: EventLineAccess::Subtree,
        }
    }
}

impl Default for EventLinePolicy {
    fn default() -> Self {
        Self::private()
    }
}

pub(crate) struct EventLine {
    name: String,
    owner_id: String,
    owner_liveness: Weak<()>,
    policy: EventLinePolicy,
    scope_id: String,
    conversation_id: Option<String>,
    bus: Arc<InMemoryEventBus>,
}

impl EventLine {
    pub(crate) fn new(name: String, owner: &ExecutionUnit, policy: EventLinePolicy) -> Self {
        Self {
            name,
            owner_id: owner.id().to_string(),
            owner_liveness: Arc::downgrade(owner.event_line_liveness()),
            policy,
            scope_id: owner.scope_id().to_string(),
            conversation_id: owner.conversation_id().map(str::to_string),
            bus: Arc::new(InMemoryEventBus::new()),
        }
    }

    fn ensure_live(&self) -> Result<()> {
        if self.owner_liveness.upgrade().is_none() {
            return Err(FrameworkError::InvalidOperation(format!(
                "event line '{}' is no longer available because its owner was dropped",
                self.name
            )));
        }
        Ok(())
    }

    fn allows(&self, access: EventLineAccess, requester: &EventLineHandle) -> bool {
        requester.requester_id == self.owner_id
            || (access == EventLineAccess::Subtree
                && requester
                    .requester_ancestor_ids
                    .iter()
                    .any(|ancestor_id| ancestor_id == &self.owner_id))
    }
}

#[derive(Clone)]
pub struct EventLineHandle {
    line: Arc<EventLine>,
    requester_id: String,
    requester_ancestor_ids: Arc<[String]>,
}

impl EventLineHandle {
    pub(crate) fn new(line: Arc<EventLine>, requester: &ExecutionUnit) -> Self {
        Self {
            line,
            requester_id: requester.id().to_string(),
            requester_ancestor_ids: requester.ancestor_ids().to_vec().into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.line.name
    }

    pub fn owner_id(&self) -> &str {
        &self.line.owner_id
    }

    pub fn policy(&self) -> EventLinePolicy {
        self.line.policy
    }

    pub fn requester_id(&self) -> &str {
        &self.requester_id
    }

    pub fn can_publish(&self) -> bool {
        self.line.owner_liveness.upgrade().is_some()
            && self.line.allows(self.line.policy.publish, self)
    }

    pub fn can_subscribe(&self) -> bool {
        self.line.owner_liveness.upgrade().is_some()
            && self.line.allows(self.line.policy.subscribe, self)
    }

    pub(crate) fn has_access(&self) -> bool {
        self.can_publish() || self.can_subscribe()
    }

    fn ensure_publish_access(&self) -> Result<()> {
        self.line.ensure_live()?;
        if self.can_publish() {
            return Ok(());
        }
        Err(FrameworkError::InvalidOperation(format!(
            "execution unit '{}' cannot publish to event line '{}' owned by '{}'",
            self.requester_id, self.line.name, self.line.owner_id
        )))
    }

    fn ensure_subscribe_access(&self) -> Result<()> {
        self.line.ensure_live()?;
        if self.can_subscribe() {
            return Ok(());
        }
        Err(FrameworkError::InvalidOperation(format!(
            "execution unit '{}' cannot subscribe to event line '{}' owned by '{}'",
            self.requester_id, self.line.name, self.line.owner_id
        )))
    }

    fn subscriber_name(&self, handler_name: &str) -> String {
        format!(
            "{}@event-line:{}:{}",
            handler_name, self.line.owner_id, self.requester_id
        )
    }
}

struct EventLineHandler {
    name: String,
    inner: Arc<dyn EventHandler>,
}

#[async_trait]
impl EventHandler for EventLineHandler {
    async fn handle(&self, event: &BaseEvent) -> Result<()> {
        self.inner.handle(event).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[async_trait]
impl EventBus for EventLineHandle {
    async fn publish(&self, mut event: BaseEvent) -> Result<()> {
        self.ensure_publish_access()?;
        event.scope_id = Some(self.line.scope_id.clone());
        if self.line.conversation_id.is_some() {
            event.conversation_id = self.line.conversation_id.clone();
        }
        self.line.bus.publish(event).await
    }

    async fn subscribe(&self, event_type: String, handler: Arc<dyn EventHandler>) -> Result<()> {
        self.ensure_subscribe_access()?;
        let handler = Arc::new(EventLineHandler {
            name: self.subscriber_name(handler.name()),
            inner: handler,
        });
        self.line.bus.subscribe(event_type, handler).await
    }

    async fn unsubscribe(&self, event_type: &str, handler_name: &str) -> Result<()> {
        self.ensure_subscribe_access()?;
        self.line
            .bus
            .unsubscribe(event_type, &self.subscriber_name(handler_name))
            .await
    }
}
