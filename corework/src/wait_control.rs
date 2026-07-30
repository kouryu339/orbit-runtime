//! Extension point for interrupting the generic `Wait` tool with durable,
//! domain-owned attention state.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Describes why an ordinary wait should end before its original condition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WaitInterrupt {
    /// Stable machine-readable reason owned by the integrating domain.
    pub reason: String,
    /// Domain-specific routing data used to resolve the attention request.
    #[serde(default)]
    pub details: serde_json::Value,
    /// Concise explanation suitable for the tool result summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A level-triggered source of attention for the generic `Wait` tool.
///
/// Implementations must subscribe before inspecting their durable state so an
/// interrupt cannot be lost between the initial read and the asynchronous wait.
/// Returning should mean the attention condition is still current, rather than
/// merely reporting that an edge occurred in the past.
#[async_trait]
pub trait WaitInterruptSource: Send + Sync {
    async fn wait_for_interrupt(&self) -> WaitInterrupt;
}

/// Type-erased shared component attached to an execution unit by a higher
/// layer. Corework remains unaware of the domain that produces the interrupt.
#[derive(Clone)]
pub struct WaitInterruptSourceHandle {
    source: std::sync::Arc<dyn WaitInterruptSource>,
}

impl WaitInterruptSourceHandle {
    pub fn new(source: std::sync::Arc<dyn WaitInterruptSource>) -> Self {
        Self { source }
    }

    pub async fn wait_for_interrupt(&self) -> WaitInterrupt {
        self.source.wait_for_interrupt().await
    }
}
