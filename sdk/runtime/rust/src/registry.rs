use std::collections::HashMap;

use serde_json::Value;

use crate::{
    conversation_id_from_event, CONVERSATION_CLOSED_EVENT_TYPE, CONVERSATION_CREATED_EVENT_TYPE,
    FRONTEND_STATE_SNAPSHOT_EVENT_TYPE, STATE_DELTA_EVENT_TYPE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationPosition {
    Current,
    Background,
}

impl ConversationPosition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Background => "background",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationInstanceInfo {
    pub conversation_id: String,
    pub position: ConversationPosition,
    pub runtime_state: String,
    pub closing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRegistryAction {
    CloseBackground { conversation_id: String },
}

#[derive(Debug, Clone)]
pub struct ConversationRegistry {
    current_conversation_id: Option<String>,
    instances: HashMap<String, ConversationInstanceInfo>,
    runtime_conversation_prefix: String,
}

impl Default for ConversationRegistry {
    fn default() -> Self {
        Self {
            current_conversation_id: None,
            instances: HashMap::new(),
            runtime_conversation_prefix: "conv_".to_string(),
        }
    }
}

impl ConversationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_runtime_conversation_prefix(prefix: impl Into<String>) -> Self {
        Self {
            runtime_conversation_prefix: prefix.into(),
            ..Self::default()
        }
    }

    pub fn current_conversation_id(&self) -> Option<&str> {
        self.current_conversation_id.as_deref()
    }

    pub fn instance(&self, conversation_id: &str) -> Option<&ConversationInstanceInfo> {
        self.instances.get(conversation_id)
    }

    pub fn position(&self, conversation_id: &str) -> Option<ConversationPosition> {
        self.instances
            .get(conversation_id)
            .map(|instance| instance.position)
    }

    pub fn is_closing(&self, conversation_id: &str) -> bool {
        self.instances
            .get(conversation_id)
            .is_some_and(|instance| instance.closing)
    }

    pub fn select_current(&mut self, conversation_id: impl Into<String>) -> Option<String> {
        let conversation_id = conversation_id.into();
        let previous = self
            .current_conversation_id
            .replace(conversation_id.clone())
            .filter(|previous| previous != &conversation_id);
        if let Some(previous) = previous.as_ref() {
            let entry = self.instances.entry(previous.clone()).or_insert_with(|| {
                ConversationInstanceInfo {
                    conversation_id: previous.clone(),
                    position: ConversationPosition::Background,
                    runtime_state: "unknown".to_string(),
                    closing: false,
                }
            });
            entry.position = ConversationPosition::Background;
        }
        let entry = self
            .instances
            .entry(conversation_id.clone())
            .or_insert_with(|| ConversationInstanceInfo {
                conversation_id,
                position: ConversationPosition::Current,
                runtime_state: "unknown".to_string(),
                closing: false,
            });
        entry.position = ConversationPosition::Current;
        entry.closing = false;
        previous
    }

    pub fn mark_background_waiting(
        &mut self,
        conversation_id: &str,
        runtime_state: &str,
    ) -> Option<ConversationRegistryAction> {
        let entry = self.instances.get_mut(conversation_id)?;
        entry.runtime_state = runtime_state.to_string();
        if entry.position != ConversationPosition::Background
            || runtime_state != "waiting"
            || entry.closing
        {
            return None;
        }
        entry.closing = true;
        Some(ConversationRegistryAction::CloseBackground {
            conversation_id: conversation_id.to_string(),
        })
    }

    pub fn cancel_background_close(&mut self, conversation_id: &str) {
        if let Some(entry) = self.instances.get_mut(conversation_id) {
            entry.closing = false;
        }
    }

    pub fn observe_runtime_event(&mut self, event: &Value) -> Option<ConversationRegistryAction> {
        let conversation_id = conversation_id_from_event(event)?;
        if !self.is_runtime_conversation_id(&conversation_id) {
            return None;
        }
        match event.get("type").and_then(Value::as_str) {
            Some(CONVERSATION_CREATED_EVENT_TYPE) => {
                let position =
                    if self.current_conversation_id.as_deref() == Some(conversation_id.as_str()) {
                        ConversationPosition::Current
                    } else {
                        ConversationPosition::Background
                    };
                self.instances
                    .entry(conversation_id.clone())
                    .or_insert_with(|| ConversationInstanceInfo {
                        conversation_id,
                        position,
                        runtime_state: "waiting".to_string(),
                        closing: false,
                    });
                None
            }
            Some(CONVERSATION_CLOSED_EVENT_TYPE) => {
                self.instances.remove(&conversation_id);
                if self.current_conversation_id.as_deref() == Some(conversation_id.as_str()) {
                    self.current_conversation_id = None;
                }
                None
            }
            Some(FRONTEND_STATE_SNAPSHOT_EVENT_TYPE) | Some(STATE_DELTA_EVENT_TYPE) => {
                let runtime_state = event
                    .pointer("/payload/conversation_state")
                    .or_else(|| event.pointer("/payload/state"))
                    .and_then(Value::as_str)?;
                self.mark_background_waiting(&conversation_id, runtime_state)
            }
            _ => None,
        }
    }

    fn is_runtime_conversation_id(&self, conversation_id: &str) -> bool {
        self.runtime_conversation_prefix.is_empty()
            || conversation_id.starts_with(&self.runtime_conversation_prefix)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn switching_moves_previous_conversation_to_background() {
        let mut registry = ConversationRegistry::new();
        assert_eq!(registry.select_current("conv_one"), None);
        assert_eq!(
            registry.select_current("conv_two").as_deref(),
            Some("conv_one")
        );

        assert_eq!(
            registry.position("conv_two"),
            Some(ConversationPosition::Current)
        );
        assert_eq!(
            registry.position("conv_one"),
            Some(ConversationPosition::Background)
        );
    }

    #[test]
    fn background_waiting_requests_close_once() {
        let mut registry = ConversationRegistry::new();
        registry.select_current("conv_one");
        registry.select_current("conv_two");

        let event = json!({
            "type": "frontend:state_snapshot",
            "conversation_id": "conv_one",
            "payload": { "conversation_state": "waiting" }
        });

        assert_eq!(
            registry.observe_runtime_event(&event),
            Some(ConversationRegistryAction::CloseBackground {
                conversation_id: "conv_one".to_string()
            })
        );
        assert_eq!(registry.observe_runtime_event(&event), None);
    }

    #[test]
    fn current_waiting_does_not_request_close() {
        let mut registry = ConversationRegistry::new();
        registry.select_current("conv_one");
        let event = json!({
            "type": "conversation.state_delta",
            "conversation_id": "conv_one",
            "payload": { "state": "waiting" }
        });

        assert_eq!(registry.observe_runtime_event(&event), None);
        assert!(!registry.is_closing("conv_one"));
    }
}
