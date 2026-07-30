use std::sync::Arc;

use async_trait::async_trait;
use corework::wait_control::{WaitInterrupt, WaitInterruptSource};

use crate::conversation_state::{AgentTaskInputRequestStatus, ConversationState};

/// Projects durable delegated-task input requests into Corework's generic
/// wait-interrupt contract for one delegating Agent.
pub(crate) struct DelegatedTaskWaitInterruptSource {
    state: Arc<ConversationState>,
    delegator_agent_id: String,
}

impl DelegatedTaskWaitInterruptSource {
    pub(crate) fn new(
        state: Arc<ConversationState>,
        delegator_agent_id: impl Into<String>,
    ) -> Self {
        Self {
            state,
            delegator_agent_id: delegator_agent_id.into(),
        }
    }

    async fn current_interrupt(&self) -> Option<WaitInterrupt> {
        let tasks = self.state.agent_tasks().await;
        for task in &tasks {
            if task.status.is_terminal()
                || task.delegator_agent_id != self.delegator_agent_id
                || task.assignee_agent_id.as_deref() == Some(self.delegator_agent_id.as_str())
            {
                continue;
            }
            let Some(request) = task
                .input_requests
                .iter()
                .find(|request| request.status == AgentTaskInputRequestStatus::Pending)
            else {
                continue;
            };
            return Some(WaitInterrupt {
                reason: "delegated_agent_input_requested".to_string(),
                details: serde_json::json!({
                    "attention_task_id": task.task_id,
                    "attention_task_revision": task.revision,
                    "assignee_agent_id": task.assignee_agent_id,
                    "request_id": request.request_id,
                    "question": request.question,
                    "required_fields": request.required_fields,
                    "blocking": request.blocking,
                }),
                summary: Some(format!(
                    "Agent task '{}' needs input from its delegator.",
                    task.task_id
                )),
            });
        }
        for task in &tasks {
            if task.status != crate::conversation_state::AgentTaskStatus::Reported
                || task.delegator_agent_id != self.delegator_agent_id
                || task.assignee_agent_id.as_deref() == Some(self.delegator_agent_id.as_str())
            {
                continue;
            }
            let report = task.report.as_ref();
            return Some(WaitInterrupt {
                reason: "delegated_agent_task_reported".to_string(),
                details: serde_json::json!({
                    "attention_task_id": task.task_id,
                    "attention_task_revision": task.revision,
                    "assignee_agent_id": task.assignee_agent_id,
                    "report_type": report.map(|report| report.report_type.as_str()),
                    "summary": report.map(|report| report.summary.as_str()),
                }),
                summary: Some(format!(
                    "Agent task '{}' submitted a result for review.",
                    task.task_id
                )),
            });
        }
        None
    }
}

#[async_trait]
impl WaitInterruptSource for DelegatedTaskWaitInterruptSource {
    async fn wait_for_interrupt(&self) -> WaitInterrupt {
        // Subscribe before reading so a request committed between the initial
        // state check and the wait cannot be lost.
        let mut changes = self.state.subscribe_agent_tasks();
        loop {
            if let Some(interrupt) = self.current_interrupt().await {
                return interrupt;
            }
            if changes.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation_state::{
        AgentTaskEntry, AgentTaskInputRequest, AgentTaskStatus, ConversationRequestHeaders,
    };
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::system::SystemOperation;

    fn task(task_id: &str, delegator: &str, assignee: &str) -> AgentTaskEntry {
        AgentTaskEntry {
            task_id: task_id.to_string(),
            revision: 2,
            title: "Need context".to_string(),
            objective: "Continue after clarification".to_string(),
            acceptance: Vec::new(),
            delegator_agent_id: delegator.to_string(),
            delegator_agent_name: delegator.to_string(),
            assignee_agent_id: Some(assignee.to_string()),
            assignee_agent_name: Some(assignee.to_string()),
            status: AgentTaskStatus::Running,
            report: None,
            progress: Vec::new(),
            input_requests: vec![AgentTaskInputRequest {
                request_id: "request-1".to_string(),
                requester_agent_id: assignee.to_string(),
                question: "Which schema?".to_string(),
                required_fields: vec!["schema".to_string()],
                blocking: true,
                status: AgentTaskInputRequestStatus::Pending,
                answer: None,
                requested_at: "now".to_string(),
                answered_at: None,
                delivery: None,
            }],
            updates: Vec::new(),
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        }
    }

    #[tokio::test]
    async fn returns_existing_direct_child_request_without_waiting() {
        let state = Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ));
        state
            .upsert_agent_task(task("task-1", "boss", "worker"))
            .await;
        let source = DelegatedTaskWaitInterruptSource::new(state, "boss");

        let interrupt = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            source.wait_for_interrupt(),
        )
        .await
        .expect("existing request should interrupt immediately");

        assert_eq!(interrupt.reason, "delegated_agent_input_requested");
        assert_eq!(interrupt.details["attention_task_id"], "task-1");
        assert_eq!(interrupt.details["request_id"], "request-1");
    }

    #[tokio::test]
    async fn ignores_requests_delegated_by_another_agent() {
        let state = Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ));
        state
            .upsert_agent_task(task("task-1", "other", "worker"))
            .await;
        let source = DelegatedTaskWaitInterruptSource::new(state, "boss");

        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            source.wait_for_interrupt(),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn observes_request_created_after_wait_registration() {
        let state = Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ));
        let mut entry = task("task-1", "boss", "worker");
        entry.input_requests.clear();
        state.upsert_agent_task(entry).await;
        let source = DelegatedTaskWaitInterruptSource::new(Arc::clone(&state), "boss");
        let waiting = tokio::spawn(async move { source.wait_for_interrupt().await });

        tokio::task::yield_now().await;
        state
            .request_agent_task_input(
                "task-1",
                "worker",
                "request-late",
                "Which schema?",
                vec!["schema".to_string()],
                true,
            )
            .await
            .unwrap();

        let interrupt = tokio::time::timeout(std::time::Duration::from_millis(50), waiting)
            .await
            .expect("task revision should wake the registered wait")
            .unwrap();
        assert_eq!(interrupt.details["request_id"], "request-late");
    }

    #[tokio::test]
    async fn answered_request_no_longer_interrupts_future_waits() {
        let state = Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ));
        state
            .upsert_agent_task(task("task-1", "boss", "worker"))
            .await;
        state
            .respond_agent_task_input("task-1", "request-1", "boss", "schema-v2")
            .await
            .unwrap();
        let source = DelegatedTaskWaitInterruptSource::new(state, "boss");

        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            source.wait_for_interrupt(),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn generic_wait_returns_child_request_routing_details() {
        let state = Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ));
        state
            .upsert_agent_task(task("task-1", "boss", "worker"))
            .await;
        let framework = corework::world::FrameworkState::initialize().unwrap();
        let unit = Arc::new(ExecutionUnit::new_root(UnitType::StateMachine, framework));
        let source: Arc<dyn WaitInterruptSource> = Arc::new(DelegatedTaskWaitInterruptSource::new(
            Arc::clone(&state),
            "boss",
        ));
        unit.attach_shared_component(Arc::new(
            corework::wait_control::WaitInterruptSourceHandle::new(source),
        ))
        .unwrap();

        let output = corework::common_tools::Wait
            .execute(
                corework::ai_system::AIInput {
                    input: "--timeout_ms 1000 --reason worker-completion".to_string(),
                },
                &unit.create_context(),
            )
            .await
            .unwrap();

        assert_eq!(output.result["wake_reason"], "external_attention");
        assert_eq!(
            output.result["interrupt"]["details"]["attention_task_id"],
            "task-1"
        );
        assert_eq!(
            output.result["interrupt"]["details"]["request_id"],
            "request-1"
        );
        assert!(output
            .to_ai
            .contains("original wait condition is not complete"));
    }

    #[tokio::test]
    async fn generic_wait_returns_reported_task_for_review() {
        let state = Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ));
        let mut entry = task("task-1", "boss", "worker");
        entry.status = AgentTaskStatus::Reported;
        entry.input_requests.clear();
        entry.report = Some(crate::conversation_state::AgentTaskReport {
            report_type: "completed".to_string(),
            summary: "Candidate result".to_string(),
            result: serde_json::Value::Null,
            artifacts: Vec::new(),
            reported_at: "now".to_string(),
        });
        state.upsert_agent_task(entry).await;
        let source = DelegatedTaskWaitInterruptSource::new(state, "boss");

        let interrupt = source.wait_for_interrupt().await;
        assert_eq!(interrupt.reason, "delegated_agent_task_reported");
        assert_eq!(interrupt.details["attention_task_id"], "task-1");
        assert_eq!(interrupt.details["report_type"], "completed");
    }
}
