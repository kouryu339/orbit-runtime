use crate::error::Result;
use crate::orchestration::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn default_model() -> String {
    "jev-latest".to_string()
}
fn default_max_steps() -> u32 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum JevArgumentBinding {
    Task,
    State { pointer: String },
    Literal { value: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JevAction {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default)]
    pub arguments: BTreeMap<String, JevArgumentBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<JevRunStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JevDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub initial_state: Value,
    pub instructions: String,
    pub actions: BTreeMap<String, JevAction>,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
}

impl JevDefinition {
    pub fn validate(&self) -> Result<()> {
        use crate::error::FrameworkError;
        if self.name.trim().is_empty() {
            return Err(FrameworkError::ValidationError(
                "Jev definition name must not be empty".to_string(),
            ));
        }
        if self.instructions.trim().is_empty() {
            return Err(FrameworkError::ValidationError(format!(
                "Jev definition '{}' instructions must not be empty",
                self.name
            )));
        }
        if self.actions.is_empty() {
            return Err(FrameworkError::ValidationError(format!(
                "Jev definition '{}' must contain at least one action",
                self.name
            )));
        }
        if self.max_steps == 0 {
            return Err(FrameworkError::ValidationError(format!(
                "Jev definition '{}' max_steps must be greater than zero",
                self.name
            )));
        }
        for (name, action) in &self.actions {
            if name.trim().is_empty() || action.description.trim().is_empty() {
                return Err(FrameworkError::ValidationError(format!(
                    "Jev definition '{}' contains an unnamed or undescribed action",
                    self.name
                )));
            }
            if action.terminal_status.is_none()
                && action
                    .tool
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                return Err(FrameworkError::ValidationError(format!(
                    "Jev action '{name}' must name a tool or be terminal"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JevExecutionContext {
    pub jev_name: String,
    pub jev_run_id: String,
    pub snapshot_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct JevSnapshotUpdate {
    #[serde(default)]
    pub set: BTreeMap<String, Value>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JevToolResult {
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub to_ai: String,
    #[serde(default)]
    pub error_code: i32,
    #[serde(default)]
    pub snapshot_update: JevSnapshotUpdate,
}

#[async_trait]
pub trait JevToolExecutor: Send + Sync {
    async fn execute(
        &self,
        tool: &str,
        arguments: BTreeMap<String, Value>,
        context: &Context,
        jev: &JevExecutionContext,
    ) -> Result<JevToolResult>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JevRunStatus {
    Completed,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JevRunOutcome {
    pub run_id: String,
    pub jev_name: String,
    pub status: JevRunStatus,
    pub selected_action: String,
    pub snapshot: Value,
    pub snapshot_revision: u64,
    pub steps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
