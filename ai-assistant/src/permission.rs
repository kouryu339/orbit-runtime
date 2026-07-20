use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use corework::event::{BaseEvent, EventBus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};

const PERMISSION_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadOnly,
    ControlledChange,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionMode {
    Full,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolPermissionPolicy {
    pub read_only: ToolPermissionMode,
    pub controlled_change: ToolPermissionMode,
    pub destructive: ToolPermissionMode,
}

impl Default for ToolPermissionPolicy {
    fn default() -> Self {
        Self {
            read_only: ToolPermissionMode::Full,
            controlled_change: ToolPermissionMode::Full,
            destructive: ToolPermissionMode::Full,
        }
    }
}

impl ToolPermissionPolicy {
    pub fn mode_for(&self, effect: ToolEffect) -> ToolPermissionMode {
        match effect {
            ToolEffect::ReadOnly => self.read_only,
            ToolEffect::ControlledChange => self.controlled_change,
            ToolEffect::Destructive => self.destructive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingToolPermission {
    pub conversation_id: String,
    pub tool_call_id: String,
    pub agent_id: String,
    pub tool_name: String,
    pub display_name: String,
    pub effect: ToolEffect,
    pub arguments: Value,
    pub turn_id: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionOutcome {
    Allowed,
    UserDenied,
    TimedOut,
    Cancelled,
}

struct PendingEntry {
    request: PendingToolPermission,
    sender: oneshot::Sender<ToolPermissionDecision>,
}

pub struct PermissionBroker {
    conversation_id: String,
    policy: ToolPermissionPolicy,
    pending: Mutex<HashMap<String, PendingEntry>>,
    event_bus: Arc<dyn EventBus>,
}

impl PermissionBroker {
    pub fn new(
        conversation_id: impl Into<String>,
        policy: ToolPermissionPolicy,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            policy,
            pending: Mutex::new(HashMap::new()),
            event_bus,
        }
    }

    pub fn policy(&self) -> &ToolPermissionPolicy {
        &self.policy
    }

    pub async fn pending(&self) -> Vec<PendingToolPermission> {
        let mut values = self
            .pending
            .lock()
            .await
            .values()
            .map(|entry| entry.request.clone())
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        values
    }

    pub async fn request(&self, request: PendingToolPermission) -> PermissionOutcome {
        debug_assert_eq!(request.conversation_id, self.conversation_id);
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            if pending.contains_key(&request.tool_call_id) {
                return PermissionOutcome::Cancelled;
            }
            pending.insert(
                request.tool_call_id.clone(),
                PendingEntry {
                    request: request.clone(),
                    sender,
                },
            );
        }
        self.publish(
            crate::events::types::TOOL_PERMISSION_REQUESTED,
            serde_json::to_value(&request).unwrap_or(Value::Null),
        )
        .await;

        match tokio::time::timeout(PERMISSION_TIMEOUT, receiver).await {
            Ok(Ok(ToolPermissionDecision::Allow)) => PermissionOutcome::Allowed,
            Ok(Ok(ToolPermissionDecision::Deny)) => PermissionOutcome::UserDenied,
            Ok(Err(_)) => PermissionOutcome::Cancelled,
            Err(_) => {
                let removed = self.pending.lock().await.remove(&request.tool_call_id);
                if removed.is_some() {
                    self.publish_resolution(&request, "timeout").await;
                }
                PermissionOutcome::TimedOut
            }
        }
    }

    pub async fn resolve(&self, tool_call_id: &str, decision: ToolPermissionDecision) -> bool {
        let entry = self.pending.lock().await.remove(tool_call_id);
        let Some(entry) = entry else {
            return false;
        };
        let resolution = match decision {
            ToolPermissionDecision::Allow => "allow",
            ToolPermissionDecision::Deny => "deny",
        };
        self.publish_resolution(&entry.request, resolution).await;
        let _ = entry.sender.send(decision);
        true
    }

    pub async fn cancel_all(&self) {
        let entries = {
            let mut pending = self.pending.lock().await;
            pending.drain().map(|(_, entry)| entry).collect::<Vec<_>>()
        };
        for entry in entries {
            self.publish_resolution(&entry.request, "cancelled").await;
            drop(entry.sender);
        }
    }

    async fn publish_resolution(&self, request: &PendingToolPermission, decision: &str) {
        self.publish(
            crate::events::types::TOOL_PERMISSION_RESOLVED,
            serde_json::json!({
                "conversation_id": request.conversation_id,
                "tool_call_id": request.tool_call_id,
                "agent_id": request.agent_id,
                "tool_name": request.tool_name,
                "decision": decision,
            }),
        )
        .await;
    }

    async fn publish(&self, event_type: &str, payload: Value) {
        if let Err(error) = self
            .event_bus
            .publish(
                BaseEvent::new(event_type, payload)
                    .with_conversation_id(self.conversation_id.clone()),
            )
            .await
        {
            tracing::warn!(event_type, %error, "publish tool permission event failed");
        }
    }
}

pub fn denied_tool_result(
    command: &str,
    tool_name: &str,
    denied_by: &str,
) -> crate::decision::ToolResult {
    let to_ai = match denied_by {
        "user" => format!(
            "[未执行] 用户拒绝了本次 {tool_name} 权限申请，工具没有执行，外部状态未发生变化。不要直接重复申请或再次调用相同工具。请询问用户是否对操作范围、目标位置或结果安全性存在顾虑，并根据回答缩小范围或提供替代方案。"
        ),
        "policy" => format!(
            "[未执行] 当前 Cluster 权限策略不允许执行 {tool_name}。工具没有执行，外部状态未发生变化。不要重复调用该工具。请明确告知用户当前策略禁止此操作，并在可能时提供只读、创建副本或其他非破坏性方案。"
        ),
        "tool_unavailable" => format!(
            "[未执行] RPC 工具集端点异常，当前运行时没有发现 `{tool_name}` 的工具元数据。工具没有执行，外部状态未发生变化。请确保对应工具服务已经启用并完成注册；开发测试阶段还需要确认工具已经实现，并且工具名与当前 Skill 的 `tools` 白名单完全一致。"
        ),
        "timeout" => format!(
            "[未执行] {tool_name} 的权限申请已超时，工具没有执行，外部状态未发生变化。请告知用户尚未获得确认；如果操作仍有必要，可以在说明具体范围后重新申请。"
        ),
        _ => format!(
            "[未执行] {tool_name} 的权限申请已取消，工具没有执行，外部状态未发生变化。不要自动重试。"
        ),
    };
    crate::decision::ToolResult {
        command: command.to_string(),
        success: false,
        to_ai,
        error_code: 102,
        result: serde_json::json!({
            "status": "denied",
            "denied_by": denied_by,
            "retryable": denied_by == "timeout",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::event::InMemoryEventBus;

    fn request(call_id: &str) -> PendingToolPermission {
        PendingToolPermission {
            conversation_id: "conversation-1".to_string(),
            tool_call_id: call_id.to_string(),
            agent_id: "boss".to_string(),
            tool_name: "WriteFile".to_string(),
            display_name: "Write file".to_string(),
            effect: ToolEffect::ControlledChange,
            arguments: serde_json::json!({"path": "report.md"}),
            turn_id: 7,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn resolves_pending_request_by_tool_call_id() {
        let broker = Arc::new(PermissionBroker::new(
            "conversation-1",
            ToolPermissionPolicy::default(),
            Arc::new(InMemoryEventBus::new()),
        ));
        let waiting = {
            let broker = broker.clone();
            tokio::spawn(async move { broker.request(request("call-1")).await })
        };

        tokio::task::yield_now().await;
        assert_eq!(broker.pending().await.len(), 1);
        assert!(
            broker
                .resolve("call-1", ToolPermissionDecision::Allow)
                .await
        );
        assert_eq!(waiting.await.unwrap(), PermissionOutcome::Allowed);
        assert!(broker.pending().await.is_empty());
        assert!(!broker.resolve("call-1", ToolPermissionDecision::Deny).await);
    }

    #[test]
    fn denied_result_distinguishes_user_and_policy_guidance() {
        let user = denied_tool_result("WriteFile --path report.md", "WriteFile", "user");
        assert!(user.to_ai.contains("询问用户"));
        assert_eq!(user.result["denied_by"], "user");

        let policy = denied_tool_result("DeleteFile --path report.md", "DeleteFile", "policy");
        assert!(policy.to_ai.contains("Cluster 权限策略"));
        assert_eq!(policy.result["denied_by"], "policy");

        let unavailable = denied_tool_result("FileList --path .", "FileList", "tool_unavailable");
        assert!(unavailable.to_ai.contains("RPC 工具集端点异常"));
        assert!(unavailable.to_ai.contains("工具服务已经启用"));
        assert!(unavailable.to_ai.contains("Skill"));
        assert_eq!(unavailable.result["denied_by"], "tool_unavailable");
    }
}
