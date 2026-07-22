#![allow(
    clippy::collapsible_else_if,
    clippy::collapsible_str_replace,
    clippy::doc_lazy_continuation,
    clippy::empty_line_after_doc_comments,
    clippy::field_reassign_with_default,
    clippy::for_kv_map,
    clippy::into_iter_on_ref,
    clippy::items_after_test_module,
    clippy::len_zero,
    clippy::let_and_return,
    clippy::manual_ok_err,
    clippy::manual_pattern_char_comparison,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::redundant_closure,
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_lazy_evaluations,
    clippy::useless_conversion
)]

//! AI assistant runtime built on top of corework.
//! The assistant state machine is intentionally small:
//! `suspended`, `thinking`, `executing`, and `saying`.
//! Multi-agent routing, focus changes, pause, reports, persistence, and
//! frontend projection are coordinated above that state machine.
//! Core data flow:
//! - `Conversation` owns the session boundary and Ledger resources.
//! - `AgentGateway` is the single coordination/write path.
//! - `AgentCluster` manages `AgentRuntime` instances and the active agent.
//! - Tools are acquired through corework dynamic execution units via `EXEC`.
//! Optional pre-thinking RAG retrieval is configured by the runtime and is
//! injected as dynamic context. The vector database implementation remains in
//! an external sidecar exposed as a normal runtime tool.

pub mod admission;
pub mod agent;
pub mod assistant;
pub mod config_resolver;
pub mod context;
pub mod conversation;
pub mod conversation_manager;
pub mod conversation_state;
pub mod decision;
pub mod decision_line;
pub mod error;
pub mod events;
pub mod gateway;
pub mod ledger;
pub mod permission;
pub mod persistence;
pub mod prompt_assets;
pub mod response_guard;
pub mod retrieval;
pub mod runtime;
pub mod runtime_tools;
pub mod skills;
pub mod snapshot;
pub mod state;
pub mod state_machine;
pub mod systems;
pub mod tool_runner;
pub mod views;

// Tool crates are linked by the top-level binary crate through inventory
// registration. `ai-assistant` no longer imports them directly.
// pub use file_ops;
// pub use audio_tools;
// pub use browser_buns;
// pub use workflows;

pub use agent::{
    set_active_agent, AgentClass, AgentCluster, AgentClusterSnapshot, AgentEntry, AgentId,
    AgentInfo, AgentKind, AgentPermissions, AgentPool, AgentReport, AgentRuntime,
    AgentRuntimeSnapshot, AgentSpec, AgentStatus,
};
pub use assistant::AIAssistant;
pub use context::{keys, roles, AssistantContext, Message};
pub use conversation::{Conversation, ConversationOptions};
pub use conversation_manager::{ConversationInfo, ConversationManager, ConversationRuntimeStatus};
pub use conversation_state::ConversationState;
pub use decision::{AIDecision, ToolResult};
pub use error::{Error, Result};
pub use gateway::AgentGateway;
pub use ledger::{LedgerMessageMeta, LedgerRecord, LedgerRole};
pub use permission::{
    PendingToolPermission, ToolEffect, ToolPermissionDecision, ToolPermissionMode,
    ToolPermissionPolicy,
};
pub use retrieval::{RetrievalConfig, RetrievalFailPolicy};
pub use skills::{Skill, SkillManager, SkillMetadata, SkillRegistry};
pub use views::ViewPayload;

use std::path::PathBuf;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::OnceLock;
    use tokio::sync::{Mutex, MutexGuard};

    static GLOBAL_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    pub async fn global_test_guard() -> MutexGuard<'static, ()> {
        GLOBAL_TEST_LOCK.get_or_init(|| Mutex::new(())).lock().await
    }
}

/// System-prompt output constraints.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SystemPromptConstraints {
    /// Whether this agent may emit frontend widget tags.
    #[serde(alias = "frontendWidgetsEnabled")]
    pub frontend_widgets_enabled: Option<bool>,
}

/// AI assistant configuration.
/// The `model` field selects the LLM provider/model routed by `llm_gateway`.
/// Common values:
/// | `model` value | Provider | Required environment variable |
/// | --- | --- | --- |
/// | `"default"` / empty | DashScope Qwen | `DASHSCOPE_API_KEY` |
/// | `"qwen-plus"` | DashScope Qwen | `DASHSCOPE_API_KEY` |
/// | `"qwen-max"` | DashScope Qwen | `DASHSCOPE_API_KEY` |
/// | `"deepseek-chat"` | DeepSeek | `DEEPSEEK_API_KEY` |
/// | `"deepseek-reasoner"` | DeepSeek | `DEEPSEEK_API_KEY` |
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AIAssistantConfig {
    pub agent_id: String,
    /// Assistant display name.
    pub name: String,
    /// Model name.
    /// - `"default"` or empty: use the DashScope default model.
    /// - `"deepseek-chat"` / `"deepseek-reasoner"`: use the DeepSeek API.
    pub model: String,
    /// Maximum token budget.
    pub max_tokens: usize,
    /// Skills directory path.
    pub skills_dir: PathBuf,
    /// Application data root directory, typically provided by the host.
    /// Persistence (`conversations/`) and runtime skills (`skills/`) are stored
    /// under this directory. When omitted, the assistant falls back to the
    /// legacy environment/hard-coded paths for compatibility.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
    /// Runtime prompt template directory.
    #[serde(default)]
    pub prompts_dir: Option<PathBuf>,
    /// Runtime prompt language, for example `zh` or `en`.
    #[serde(default = "default_language")]
    pub language: String,
    /// Optional pre-thinking retrieval configuration.
    #[serde(default)]
    pub retrieval: RetrievalConfig,
    /// System-prompt output constraints.
    #[serde(alias = "systemPromptConstraints", default)]
    pub system_prompt_constraints: SystemPromptConstraints,
    /// Whether this agent may emit frontend widget tags.
    #[serde(default = "default_frontend_widgets_enabled")]
    pub frontend_widgets_enabled: bool,
    /// Maps logical state names to concrete system-layer skills.
    #[serde(alias = "systemSkills", default)]
    pub system_skills: std::collections::BTreeMap<String, String>,
}

fn default_language() -> String {
    "zh".to_string()
}

fn default_frontend_widgets_enabled() -> bool {
    true
}

impl Default for AIAssistantConfig {
    fn default() -> Self {
        Self {
            agent_id: "default-agent".to_string(),
            name: "AI助手".to_string(),
            model: "default".to_string(),
            max_tokens: 4096,
            skills_dir: PathBuf::from("skills"),
            data_dir: None,
            prompts_dir: None,
            language: default_language(),
            retrieval: RetrievalConfig::default(),
            system_prompt_constraints: SystemPromptConstraints::default(),
            frontend_widgets_enabled: true,
            system_skills: std::collections::BTreeMap::new(),
        }
    }
}

impl AIAssistantConfig {
    pub fn effective_frontend_widgets_enabled(&self) -> bool {
        self.system_prompt_constraints
            .frontend_widgets_enabled
            .unwrap_or(self.frontend_widgets_enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = AIAssistantConfig::default();
        assert_eq!(config.name, "AI助手");
        assert_eq!(config.max_tokens, 4096);
        assert!(config.effective_frontend_widgets_enabled());
    }
}
