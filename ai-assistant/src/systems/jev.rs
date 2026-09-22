use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::jev::{
    JevExecutionContext, JevManager, JevSnapshotUpdate, JevToolExecutor, JevToolResult,
    RegistryJevToolExecutor,
};
use corework::orchestration::Context;
use corework::system::SystemOperation;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::permission::{
    PendingToolPermission, PermissionBroker, PermissionOutcome, ToolPermissionMode,
};

#[derive(Debug, Default)]
struct AgentJevToolExecutor {
    direct: RegistryJevToolExecutor,
}

#[async_trait]
impl JevToolExecutor for AgentJevToolExecutor {
    async fn execute(
        &self,
        tool: &str,
        arguments: BTreeMap<String, Value>,
        context: &Context,
        jev: &JevExecutionContext,
    ) -> corework::error::Result<JevToolResult> {
        let metadata = match crate::tool_runner::permission_metadata(tool) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(permission_denied_result(tool, "tool_unavailable")),
        };
        let broker = match context.resolve_shared_component::<PermissionBroker>() {
            Ok(broker) => broker,
            Err(_) => return Ok(permission_denied_result(tool, "policy")),
        };
        match broker.policy().mode_for(metadata.effect) {
            ToolPermissionMode::Full => {}
            ToolPermissionMode::Deny => return Ok(permission_denied_result(tool, "policy")),
            ToolPermissionMode::Ask => {
                let arguments_for_review = if metadata.secret {
                    json!({"redacted": true})
                } else {
                    serde_json::to_value(&arguments).unwrap_or(Value::Null)
                };
                let request = PendingToolPermission {
                    conversation_id: jev
                        .conversation_id
                        .clone()
                        .unwrap_or_else(|| "default".to_string()),
                    tool_call_id: format!("{}:{}:{}", jev.jev_run_id, jev.snapshot_revision, tool),
                    agent_id: jev
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    tool_name: tool.to_string(),
                    display_name: metadata.display_name,
                    effect: metadata.effect,
                    arguments: arguments_for_review,
                    turn_id: crate::AssistantContext::current_turn_id(&context.cache).await,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                match broker.request(request).await {
                    PermissionOutcome::Allowed => {}
                    PermissionOutcome::UserDenied => {
                        return Ok(permission_denied_result(tool, "user"));
                    }
                    PermissionOutcome::TimedOut => {
                        return Ok(permission_denied_result(tool, "timeout"));
                    }
                    PermissionOutcome::Cancelled => {
                        return Ok(permission_denied_result(tool, "cancelled"));
                    }
                }
            }
        }
        self.direct.execute(tool, arguments, context, jev).await
    }
}

fn permission_denied_result(tool: &str, denied_by: &str) -> JevToolResult {
    let message = match denied_by {
        "user" => format!("User denied permission for Jev tool '{tool}'."),
        "timeout" => format!("Permission request for Jev tool '{tool}' timed out."),
        "tool_unavailable" => format!("Jev tool '{tool}' has no permission metadata."),
        "cancelled" => format!("Permission request for Jev tool '{tool}' was cancelled."),
        _ => format!("The current policy denied Jev tool '{tool}'."),
    };
    JevToolResult {
        result: json!({"status": "denied", "tool": tool, "denied_by": denied_by}),
        to_ai: message,
        error_code: 403,
        snapshot_update: JevSnapshotUpdate {
            set: BTreeMap::from([(
                "last_execution".to_string(),
                json!({"status": "denied", "tool": tool, "denied_by": denied_by}),
            )]),
            remove: Vec::new(),
        },
    }
}

#[define_operation(
    name = "ListJev",
    display_name = "列出 Jev 选择器",
    category = "Assistant",
    system_only,
    description = "列出宿主注册的 Jev 有限状态选择器。",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ListJevSystem;

#[async_trait]
impl SystemOperation for ListJevSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, _input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let manager = ctx.resolve_shared_component::<JevManager>()?;
        let definitions = manager.list();
        let summary = if definitions.is_empty() {
            "No Jev selectors are registered.".to_string()
        } else {
            definitions
                .iter()
                .map(|definition| format!("{}: {}", definition.name, definition.description))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(AIOutput::success(
            serde_json::json!({"jev": definitions}),
            summary,
        ))
    }

    fn name(&self) -> &str {
        "ListJev"
    }
}

#[define_operation(
    name = "RunJev",
    display_name = "让 Jev 选择器{jevname}执行{task}",
    category = "Assistant",
    system_only,
    description = "用已注册的 Jev 选择器执行由有限动作组成的任务。只传 jevname 和初始任务文字。",
    params {
        jevname: "已注册的 Jev 名称",
        task: "交给 Jev 的初始任务文字"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct RunJevSystem;

#[async_trait]
impl SystemOperation for RunJevSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let jevname = match args.safe_require("jevname") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let task = match args.safe_require("task") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let manager = ctx.resolve_shared_component::<JevManager>()?;
        let conversation_id = ctx.conversation_id.clone();
        let agent_id = match ctx.get::<String>("agent_id")? {
            Some(value) => Some(value),
            None => ctx.cache.get::<String>("agent_id").await?,
        };
        let parent_run_id = ctx
            .get::<String>("jev_run_id")?
            .or(ctx.get::<String>("workflow_run_id")?);
        let outcome = manager
            .run_with_executor(
                &jevname,
                &task,
                conversation_id,
                agent_id,
                parent_run_id,
                ctx.execution_unit(),
                Arc::new(AgentJevToolExecutor::default()),
            )
            .await?;
        let ok = matches!(outcome.status, corework::jev::JevRunStatus::Completed);
        let text = format!(
            "Jev '{}' finished with status {:?} after {} step(s).",
            outcome.jev_name, outcome.status, outcome.steps
        );
        if ok {
            Ok(AIOutput::success(outcome, text))
        } else {
            let mut output = AIOutput::error(409, text);
            output.result = serde_json::to_value(outcome).unwrap_or_default();
            Ok(output)
        }
    }

    fn name(&self) -> &str {
        "RunJev"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::ToolPermissionPolicy;
    use corework::event::InMemoryEventBus;
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::world::FrameworkState;

    #[tokio::test]
    async fn agent_jev_executor_applies_permission_policy_before_tool_execution() {
        let framework = FrameworkState::initialize().unwrap();
        let unit = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
        let policy = ToolPermissionPolicy {
            read_only: ToolPermissionMode::Deny,
            controlled_change: ToolPermissionMode::Deny,
            destructive: ToolPermissionMode::Deny,
        };
        unit.attach_shared_component(Arc::new(PermissionBroker::new(
            "conversation-1",
            policy,
            Arc::new(InMemoryEventBus::new()),
        )))
        .unwrap();
        let context = unit.create_context().with_conversation_id("conversation-1");
        context.set("agent_id", "agent-1".to_string()).unwrap();
        let result = AgentJevToolExecutor::default()
            .execute(
                "RunJev",
                BTreeMap::new(),
                &context,
                &JevExecutionContext {
                    jev_name: "test".to_string(),
                    jev_run_id: "jev-test".to_string(),
                    snapshot_revision: 0,
                    conversation_id: Some("conversation-1".to_string()),
                    agent_id: Some("agent-1".to_string()),
                    parent_run_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.error_code, 403);
        assert_eq!(
            result.snapshot_update.set["last_execution"]["status"],
            "denied"
        );
    }
}
