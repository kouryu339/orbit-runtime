use crate::agent::keys::BOSS_AGENT_ID;
use crate::context::Message;
use crate::gateway::GatewayMessage;
use crate::persistence::DisplayMeta;
use crate::views::ViewPayload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_CONVERSATION_ID: &str = "current";
pub const LEDGER_RESOURCE_KEY: &str = "ai_assistant:ledger";
pub const FOCUS_RESOURCE_KEY: &str = "ai_assistant:focus";
pub const GATEWAY_SUBTYPE_FOCUS_CHANGED: &str = "focus_changed";
pub const GATEWAY_SUBTYPE_PAUSE_REQUESTED: &str = "pause_requested";
pub const GATEWAY_SUBTYPE_INTERRUPTED: &str = "interrupted";
pub const GATEWAY_SUBTYPE_LLM_USAGE: &str = "llm_usage";
pub const GATEWAY_SUBTYPE_LLM_ERROR: &str = "llm_error";
pub const GATEWAY_SUBTYPE_AGENT_APPOINTMENT: &str = "agent_appointment";
pub const GATEWAY_SUBTYPE_AGENT_REPORT: &str = "agent_report";
pub const GATEWAY_SUBTYPE_AGENT_TASK_CREATED: &str = "agent_task_created";
pub const GATEWAY_SUBTYPE_AGENT_TASK_ASSIGNED: &str = "agent_task_assigned";
pub const GATEWAY_SUBTYPE_AGENT_TASK_REPORT: &str = "agent_task_report";
pub const GATEWAY_SUBTYPE_AGENT_TASK_COMPLETED: &str = "agent_task_completed";
pub const GATEWAY_SUBTYPE_AGENT_SUSPENDED: &str = "agent_suspended";
pub const GATEWAY_SUBTYPE_AGENT_RESUMED: &str = "agent_resumed";
pub const GATEWAY_SUBTYPE_AGENT_CANCELED: &str = "agent_canceled";
pub const GATEWAY_SUBTYPE_SNAPSHOT_UPDATED: &str = "snapshot_updated";
pub const GATEWAY_SUBTYPE_TOOL_CALL_PERMISSION_REQUESTED: &str = "tool_call_permission_requested";
pub const GATEWAY_SUBTYPE_TOOL_CALL_STARTED: &str = "tool_call_started";
pub const GATEWAY_SUBTYPE_TOOL_CALL_FINISHED: &str = "tool_call_finished";
pub const GATEWAY_SUBTYPE_TOOL_CALL_FAILED: &str = "tool_call_failed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerRole {
    User,
    Assistant,
    Tool,
    AgentReport,
    GatewayMessage,
    Summary,
}

impl LedgerRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::AgentReport => "agent_report",
            Self::GatewayMessage => "gateway_message",
            Self::Summary => "summary",
        }
    }

    pub fn from_message_role(role: &str, display: Option<&DisplayMeta>) -> Self {
        // agent_report display_role overrides the message role to produce LedgerRole::AgentReport.
        // All other display roles (e.g. tool_step) defer to the message role.
        if let Some(display) = display {
            if display.display_role == "agent_report" {
                return Self::AgentReport;
            }
        }

        match role {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "agent_report" => Self::AgentReport,
            "summary" | "compact_summary" => Self::Summary,
            _ => Self::GatewayMessage,
        }
    }
}

impl std::fmt::Display for LedgerRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LedgerMessageMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewPayload>,
    /// User-facing projection when canonical content contains runtime protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl LedgerMessageMeta {
    pub fn from_display(display: DisplayMeta) -> Self {
        Self {
            subtype: Some(display.display_role),
            tool_name: display.tool_name,
            tool_command: display.tool_command,
            success: display.success,
            collapsed: Some(true),
            reasoning: display.reasoning,
            decision: display.decision,
            tools: display.tools,
            extra: display
                .agent_name
                .map(|name| {
                    BTreeMap::from([("agent_name".to_string(), serde_json::Value::String(name))])
                })
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    pub fn overlay(mut self, other: Self) -> Self {
        if other.subtype.is_some() {
            self.subtype = other.subtype;
        }
        if other.view.is_some() {
            self.view = other.view;
        }
        if other.display_content.is_some() {
            self.display_content = other.display_content;
        }
        if other.tool_name.is_some() {
            self.tool_name = other.tool_name;
        }
        if other.tool_command.is_some() {
            self.tool_command = other.tool_command;
        }
        if other.success.is_some() {
            self.success = other.success;
        }
        if other.collapsed.is_some() {
            self.collapsed = other.collapsed;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
        if other.decision.is_some() {
            self.decision = other.decision;
        }
        if !other.tools.is_empty() {
            self.tools = other.tools;
        }
        if other.from_agent_id.is_some() {
            self.from_agent_id = other.from_agent_id;
        }
        if other.to_agent_id.is_some() {
            self.to_agent_id = other.to_agent_id;
        }
        if other.reason.is_some() {
            self.reason = other.reason;
        }
        self.extra.extend(other.extra);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub record_id: u64,
    pub conversation_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub role: LedgerRole,
    pub content: String,
    #[serde(default)]
    pub metadata: LedgerMessageMeta,
    pub created_at: String,
}

impl LedgerRecord {
    pub fn from_message(
        record_id: u64,
        conversation_id: impl Into<String>,
        agent_id: impl Into<String>,
        agent_name: impl Into<String>,
        message: Message,
        display: Option<DisplayMeta>,
    ) -> Self {
        let role = LedgerRole::from_message_role(&message.role, display.as_ref());
        let metadata = display
            .map(LedgerMessageMeta::from_display)
            .unwrap_or_default();
        Self {
            record_id,
            conversation_id: conversation_id.into(),
            agent_id: agent_id.into(),
            agent_name: agent_name.into(),
            role,
            content: message.content,
            metadata,
            created_at: chrono::Local::now().to_rfc3339(),
        }
    }

    pub fn from_persisted(record_id: u64, persisted: crate::persistence::PersistedMessage) -> Self {
        let agent_id = persisted
            .agent_id
            .unwrap_or_else(|| BOSS_AGENT_ID.to_string());
        let agent_name = persisted
            .display
            .as_ref()
            .and_then(|display| display.agent_name.clone())
            .unwrap_or_else(|| agent_id.clone());
        Self::from_message(
            record_id,
            DEFAULT_CONVERSATION_ID,
            agent_id,
            agent_name,
            persisted.inner,
            persisted.display,
        )
    }

    pub fn to_frontend_message(&self) -> Option<GatewayMessage> {
        match self.role {
            LedgerRole::User
            | LedgerRole::Assistant
            | LedgerRole::Tool
            | LedgerRole::AgentReport => Some(GatewayMessage {
                role: self.role.as_str().to_string(),
                content: self
                    .metadata
                    .display_content
                    .clone()
                    .unwrap_or_else(|| self.content.clone()),
                view: self.metadata.view.clone(),
                tool_name: self.metadata.tool_name.clone(),
                tool_command: self.metadata.tool_command.clone(),
                success: self.metadata.success,
                collapsed: self.metadata.collapsed,
                reasoning: self.metadata.reasoning.clone(),
                decision: self.metadata.decision.clone(),
                tools: if self.metadata.tools.is_empty() {
                    None
                } else {
                    Some(self.metadata.tools.clone())
                },
                kind: self
                    .metadata
                    .extra
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                status: self
                    .metadata
                    .extra
                    .get("status")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                call_id: self
                    .metadata
                    .extra
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                turn_id: self
                    .metadata
                    .extra
                    .get("turn_id")
                    .and_then(|value| value.as_u64()),
                agent_id: None,
                agent_name: if self.role == LedgerRole::AgentReport {
                    self.metadata
                        .extra
                        .get("from_agent_name")
                        .or_else(|| self.metadata.extra.get("agent_name"))
                        .and_then(|value| value.as_str())
                        .map(|name| name.to_string())
                        .or_else(|| Some(self.agent_name.clone()))
                } else {
                    None
                },
            }),
            LedgerRole::GatewayMessage => match self.metadata.subtype.as_deref() {
                Some(GATEWAY_SUBTYPE_TOOL_CALL_STARTED)
                | Some(GATEWAY_SUBTYPE_TOOL_CALL_PERMISSION_REQUESTED)
                | Some(GATEWAY_SUBTYPE_TOOL_CALL_FINISHED)
                | Some(GATEWAY_SUBTYPE_TOOL_CALL_FAILED) => Some(GatewayMessage {
                    role: "tool".to_string(),
                    content: self.content.clone(),
                    view: None,
                    tool_name: self.metadata.tool_name.clone(),
                    tool_command: self.metadata.tool_command.clone(),
                    success: self.metadata.success,
                    collapsed: self.metadata.collapsed,
                    reasoning: None,
                    decision: None,
                    tools: None,
                    kind: self
                        .metadata
                        .extra
                        .get("kind")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    status: self
                        .metadata
                        .extra
                        .get("status")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    call_id: self
                        .metadata
                        .extra
                        .get("call_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    turn_id: self
                        .metadata
                        .extra
                        .get("turn_id")
                        .and_then(|value| value.as_u64()),
                    agent_id: None,
                    agent_name: None,
                }),
                _ => None,
            },
            LedgerRole::Summary => None,
        }
    }

    pub fn to_context_message(&self) -> Option<Message> {
        match self.role {
            LedgerRole::User => Some(Message::user(&self.content)),
            LedgerRole::Assistant => {
                if self.content.trim().is_empty() {
                    None
                } else {
                    Some(Message::assistant(&self.content))
                }
            }
            LedgerRole::Tool => Some(Message::user(format_tool_content(&self.content))),
            LedgerRole::AgentReport => Some(Message::user(&self.content)),
            LedgerRole::Summary => Some(Message::user(crate::prompt_assets::render(
                "conversation_summary_context.md",
                &[("{{CONTENT}}", &self.content)],
            ))),
            LedgerRole::GatewayMessage => None,
        }
    }
}

pub fn frontend_messages(records: &[LedgerRecord]) -> Vec<GatewayMessage> {
    records
        .iter()
        .filter_map(LedgerRecord::to_frontend_message)
        .collect()
}

pub fn agent_context(records: &[LedgerRecord], agent_id: &str) -> Vec<Message> {
    let current_agent_records = records
        .iter()
        .filter(|record| record.agent_id == agent_id)
        .collect::<Vec<_>>();
    let start = current_agent_records
        .iter()
        .rposition(|record| record.role == LedgerRole::Summary)
        .unwrap_or(0);

    current_agent_records[start..]
        .iter()
        .filter_map(|record| record.to_context_message())
        .collect()
}

pub fn focus_changed_record(
    record_id: u64,
    conversation_id: impl Into<String>,
    from_agent_id: impl Into<String>,
    to_agent_id: impl Into<String>,
    reason: impl Into<String>,
) -> LedgerRecord {
    let from_agent_id = from_agent_id.into();
    let to_agent_id = to_agent_id.into();
    LedgerRecord {
        record_id,
        conversation_id: conversation_id.into(),
        agent_id: BOSS_AGENT_ID.to_string(),
        agent_name: BOSS_AGENT_ID.to_string(),
        role: LedgerRole::GatewayMessage,
        content: String::new(),
        metadata: LedgerMessageMeta {
            subtype: Some(GATEWAY_SUBTYPE_FOCUS_CHANGED.to_string()),
            from_agent_id: Some(from_agent_id),
            to_agent_id: Some(to_agent_id),
            reason: Some(reason.into()),
            ..Default::default()
        },
        created_at: chrono::Local::now().to_rfc3339(),
    }
}

pub fn gateway_message_record(
    record_id: u64,
    conversation_id: impl Into<String>,
    agent_id: impl Into<String>,
    agent_name: impl Into<String>,
    subtype: impl Into<String>,
    content: impl Into<String>,
    mut metadata: LedgerMessageMeta,
) -> LedgerRecord {
    metadata.subtype = Some(subtype.into());
    LedgerRecord {
        record_id,
        conversation_id: conversation_id.into(),
        agent_id: agent_id.into(),
        agent_name: agent_name.into(),
        role: LedgerRole::GatewayMessage,
        content: content.into(),
        metadata,
        created_at: chrono::Local::now().to_rfc3339(),
    }
}

fn format_tool_content(content: &str) -> String {
    let tool_prefix = crate::prompt_assets::template("tool_execution_result.md")
        .split("{{COMMAND}}")
        .next()
        .unwrap_or("工具执行：")
        .trim_start()
        .to_string();
    if content.trim_start().starts_with(&tool_prefix)
        || content.trim_start().starts_with("工具执行：")
    {
        content.to_string()
    } else {
        crate::prompt_assets::render("tool_result_context.md", &[("{{CONTENT}}", content)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u64, agent_id: &str, role: LedgerRole, content: &str) -> LedgerRecord {
        LedgerRecord {
            record_id: id,
            conversation_id: DEFAULT_CONVERSATION_ID.to_string(),
            agent_id: agent_id.to_string(),
            agent_name: agent_id.to_string(),
            role,
            content: content.to_string(),
            metadata: LedgerMessageMeta::default(),
            created_at: "2026-04-30T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn frontend_projection_hides_gateway_and_summary() {
        let records = vec![
            record(1, "boss", LedgerRole::User, "hi"),
            record(2, "boss", LedgerRole::Summary, "old"),
            record(3, "boss", LedgerRole::GatewayMessage, ""),
            record(4, "boss", LedgerRole::Assistant, "hello"),
            record(5, "agent_1", LedgerRole::AgentReport, "done"),
        ];

        let projected = frontend_messages(&records);
        assert_eq!(projected.len(), 3);
        assert_eq!(projected[0].role, "user");
        assert_eq!(projected[1].role, "assistant");
        assert_eq!(projected[2].role, "agent_report");
    }

    #[test]
    fn agent_context_starts_at_latest_agent_summary() {
        let records = vec![
            record(1, "agent_1", LedgerRole::User, "old user"),
            record(2, "agent_2", LedgerRole::Summary, "other summary"),
            record(3, "agent_1", LedgerRole::Summary, "agent summary"),
            record(4, "agent_1", LedgerRole::Tool, "tool result"),
            record(5, "agent_1", LedgerRole::Assistant, "new answer"),
        ];

        let context = agent_context(&records, "agent_1");
        assert_eq!(context.len(), 3);
        assert_eq!(context[0].role, "user");
        assert!(context[0].content.contains("agent summary"));
        assert_eq!(context[1].role, "user");
        assert!(context[1].content.contains("tool result"));
        assert_eq!(context[2].role, "assistant");
    }

    #[test]
    fn frontend_agent_report_prefers_source_name_from_metadata() {
        let mut record = record(1, "boss", LedgerRole::AgentReport, "done");
        record.agent_name = "Boss".to_string();
        record.metadata.extra.insert(
            "from_agent_name".to_string(),
            serde_json::Value::String("Worker".to_string()),
        );

        let projected = record.to_frontend_message().unwrap();

        assert_eq!(projected.role, "agent_report");
        assert_eq!(projected.agent_name.as_deref(), Some("Worker"));
        assert!(projected.agent_id.is_none());
    }

    #[test]
    fn context_projection_filters_display_only_and_empty_assistant_records() {
        let records = vec![
            record(1, "agent_1", LedgerRole::User, "task"),
            record(2, "agent_1", LedgerRole::GatewayMessage, "focus changed"),
            record(3, "agent_1", LedgerRole::Assistant, "   "),
            record(4, "agent_1", LedgerRole::Tool, "ok"),
            record(5, "agent_1", LedgerRole::Assistant, "final"),
        ];

        let context = agent_context(&records, "agent_1");

        assert_eq!(context.len(), 3);
        assert_eq!(context[0].content, "task");
        assert!(context[1].content.contains("ok"));
        assert_eq!(context[2].content, "final");
    }

    #[test]
    fn assistant_frontend_projection_does_not_change_llm_context_content() {
        let mut assistant = record(
            1,
            "agent_1",
            LedgerRole::Assistant,
            "$path = \"a.docx\"\nEXEC ReadFile --path $path",
        );
        assistant.metadata.display_content =
            Some("[tool:status | call_id=\"agent_1:1:0\"]".to_string());

        let frontend = assistant.to_frontend_message().unwrap();
        assert_eq!(frontend.content, "[tool:status | call_id=\"agent_1:1:0\"]");

        let context = assistant.to_context_message().unwrap();
        assert!(context.content.contains("$path"));
        assert!(context.content.contains("EXEC ReadFile"));
    }
}
