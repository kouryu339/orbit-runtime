use async_trait::async_trait;
use corework::buns_system;
use corework::cache::CacheExt;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};

pub const WAKE_DELEGATOR_AGENT_SYSTEM: &str = "WakeDelegatorAgentForTaskSystem";

/// Authenticated identity carried by an internal System invocation.
///
/// The values are produced from the bound Corework Context and are checked
/// again by the receiving System. They are not accepted from an AI tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSystemInvocationIdentity {
    pub conversation_id: String,
    pub execution_unit_id: String,
    pub agent_id: String,
}

impl AgentSystemInvocationIdentity {
    pub async fn from_context(ctx: &Context) -> Result<Self, FrameworkError> {
        let unit = ctx.execution_unit().ok_or_else(|| {
            FrameworkError::InvalidOperation(
                "agent System invocation is not bound to an ExecutionUnit".to_string(),
            )
        })?;
        let conversation_id = ctx.conversation_id.clone().ok_or_else(|| {
            FrameworkError::InvalidOperation(
                "agent System invocation has no conversation identity".to_string(),
            )
        })?;
        let agent_id = ctx
            .cache
            .get::<String>(crate::state_machine::agent_keys::AGENT_ID)
            .await?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(
                    "agent System invocation has no Agent identity".to_string(),
                )
            })?;
        let cached_conversation_id = ctx
            .cache
            .get::<String>(crate::state_machine::agent_keys::CONVERSATION_ID)
            .await?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(
                    "agent System invocation cache has no conversation identity".to_string(),
                )
            })?;
        if cached_conversation_id != conversation_id {
            return Err(FrameworkError::InvalidOperation(
                "agent System invocation context and cache identities differ".to_string(),
            ));
        }
        Ok(Self {
            conversation_id,
            execution_unit_id: unit.id().to_string(),
            agent_id,
        })
    }

    async fn verify(&self, ctx: &Context) -> Result<(), FrameworkError> {
        let actual = Self::from_context(ctx).await?;
        if actual != *self {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent System invocation identity mismatch: expected {:?}, got {:?}",
                actual, self
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeDelegatorAgentInput {
    pub identity: AgentSystemInvocationIdentity,
    pub task_id: String,
    pub reason: DelegatorWakeReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub task_revision: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegatorWakeReason {
    InputRequested,
    TaskReported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WakeDelegatorAgentOutput {
    pub caller_agent_id: String,
    pub target_agent_id: String,
    pub task_id: String,
    pub task_revision: u64,
    pub outcome: crate::agent::cluster::AgentWakeOutcome,
}

/// Internal Corework System. No AI parameters are declared, so it is not
/// registered in the AI tool catalog. Hosts may replace the dynamic System
/// registered under `WAKE_DELEGATOR_AGENT_SYSTEM`.
#[buns_system(
    "WakeDelegatorAgentForTaskSystem",
    description = "Wake a stopped task delegator after an authenticated child Agent requests input or submits a result."
)]
pub struct WakeDelegatorAgentForTaskSystem;

#[async_trait]
impl SystemOperation for WakeDelegatorAgentForTaskSystem {
    type Input = WakeDelegatorAgentInput;
    type Output = WakeDelegatorAgentOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        input.identity.verify(ctx).await?;
        let state =
            ctx.resolve_shared_component::<crate::conversation_state::ConversationState>()?;
        if state.conversation_id() != input.identity.conversation_id {
            return Err(FrameworkError::InvalidOperation(
                "task and caller belong to different conversations".to_string(),
            ));
        }
        let task = state.agent_task(&input.task_id).await.ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "agent task '{}' does not exist",
                input.task_id
            ))
        })?;
        if task.status.is_terminal()
            || task.assignee_agent_id.as_deref() != Some(input.identity.agent_id.as_str())
        {
            return Err(FrameworkError::InvalidOperation(
                "caller is not the current assignee of this non-terminal task".to_string(),
            ));
        }
        if task.revision < input.task_revision {
            return Err(FrameworkError::InvalidOperation(format!(
                "task revision {} is older than requested revision {}",
                task.revision, input.task_revision
            )));
        }
        match input.reason {
            DelegatorWakeReason::InputRequested => {
                let request_id = input.request_id.as_deref().ok_or_else(|| {
                    FrameworkError::InvalidOperation(
                        "input-requested wake requires request_id".to_string(),
                    )
                })?;
                let pending_request = task.input_requests.iter().any(|request| {
                    request.request_id == request_id
                        && request.requester_agent_id == input.identity.agent_id
                        && request.status
                            == crate::conversation_state::AgentTaskInputRequestStatus::Pending
                });
                if !pending_request {
                    return Err(FrameworkError::InvalidOperation(
                        "the authenticated caller has no matching pending input request"
                            .to_string(),
                    ));
                }
            }
            DelegatorWakeReason::TaskReported => {
                if task.status != crate::conversation_state::AgentTaskStatus::Reported
                    || task.report.is_none()
                {
                    return Err(FrameworkError::InvalidOperation(
                        "the authenticated caller has no reported result awaiting review"
                            .to_string(),
                    ));
                }
            }
        }
        if task.delegator_agent_id == input.identity.agent_id {
            return Err(FrameworkError::InvalidOperation(
                "an Agent cannot wake itself through delegated-task routing".to_string(),
            ));
        }

        let cluster = ctx.resolve_shared_component::<crate::agent::cluster::AgentCluster>()?;
        let outcome = cluster
            .wake_agent_if_suspended(&task.delegator_agent_id)
            .await
            .map_err(|error| FrameworkError::SystemError(error.to_string()))?;
        tracing::info!(
            conversation_id = %input.identity.conversation_id,
            caller_execution_unit_id = %input.identity.execution_unit_id,
            caller_agent_id = %input.identity.agent_id,
            target_agent_id = %task.delegator_agent_id,
            task_id = %input.task_id,
            reason = ?input.reason,
            request_id = ?input.request_id,
            task_revision = task.revision,
            outcome = ?outcome,
            "authenticated delegated-task wake evaluated"
        );
        Ok(WakeDelegatorAgentOutput {
            caller_agent_id: input.identity.agent_id,
            target_agent_id: task.delegator_agent_id,
            task_id: input.task_id,
            task_revision: task.revision,
            outcome,
        })
    }

    fn is_idempotent(&self) -> bool {
        true
    }
}

pub async fn invoke_wake_delegator_agent(
    input: WakeDelegatorAgentInput,
    ctx: &Context,
) -> Result<WakeDelegatorAgentOutput, FrameworkError> {
    let system = ctx.get_dynamic_system(WAKE_DELEGATOR_AGENT_SYSTEM)?;
    let value = serde_json::to_value(input)?;
    let object = value.as_object().cloned().ok_or_else(|| {
        FrameworkError::InvalidData("wake System input must serialize as an object".to_string())
    })?;
    let output = system
        .execute_dynamic(object.into_iter().collect(), ctx)
        .await?;
    serde_json::from_value(output).map_err(FrameworkError::SerializationError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::cache::CacheExt;
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::statemachine::{FnState, StateMachine};
    use std::sync::Arc;

    async fn caller_context() -> (Context, Arc<ExecutionUnit>) {
        let framework = corework::world::FrameworkState::initialize().unwrap();
        let root = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            "conversation:identity-test",
        ));
        let caller = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &root).unwrap());
        caller
            .cache()
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &"worker".to_string(),
                None,
            )
            .await
            .unwrap();
        caller
            .cache()
            .set(
                crate::state_machine::agent_keys::CONVERSATION_ID,
                &"identity-test".to_string(),
                None,
            )
            .await
            .unwrap();
        (caller.create_context(), caller)
    }

    #[tokio::test]
    async fn invocation_identity_is_bound_to_execution_context() {
        let (ctx, _caller) = caller_context().await;
        let identity = AgentSystemInvocationIdentity::from_context(&ctx)
            .await
            .unwrap();
        assert_eq!(identity.conversation_id, "identity-test");
        assert_eq!(identity.agent_id, "worker");
        assert_eq!(
            identity.execution_unit_id,
            ctx.execution_unit().unwrap().id()
        );

        let mut forged = identity;
        forged.agent_id = "boss".to_string();
        assert!(forged.verify(&ctx).await.is_err());
    }

    #[test]
    fn internal_wake_system_is_not_exposed_as_an_ai_tool() {
        assert!(!corework::system::SystemRegistry::list_ai_systems()
            .iter()
            .any(|metadata| metadata.name == WAKE_DELEGATOR_AGENT_SYSTEM));
    }

    #[tokio::test]
    async fn authenticated_child_request_wakes_its_stopped_delegator() {
        let _guard = crate::test_support::global_test_guard().await;
        let framework = corework::world::FrameworkState::initialize().unwrap();
        let root = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            "conversation:wake-test",
        ));
        let state = Arc::new(crate::conversation_state::ConversationState::new(
            "wake-test",
            Default::default(),
            "boss",
        ));
        root.attach_shared_component(Arc::clone(&state)).unwrap();

        let thinking = FnState::new(crate::state::states::THINKING)
            .with_description("test thinking")
            .with_on_transition(|_| {
                Box::pin(async { Ok(Some(crate::state::states::SUSPENDED.to_string())) })
            });
        let boss_sm = Arc::new(
            StateMachine::builder("wake-test-boss")
                .with_parent_unit(Arc::clone(&root))
                .add_state(Box::new(crate::state::suspended::build()))
                .add_state(Box::new(thinking))
                .initial_state(crate::state::states::SUSPENDED)
                .build()
                .await
                .unwrap(),
        );
        boss_sm.start().await.unwrap();
        boss_sm
            .unit()
            .cache()
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &"boss".to_string(),
                None,
            )
            .await
            .unwrap();
        boss_sm
            .unit()
            .cache()
            .set(
                crate::state_machine::agent_keys::CONVERSATION_ID,
                &"wake-test".to_string(),
                None,
            )
            .await
            .unwrap();
        let boss = Arc::new(crate::agent::AgentRuntime::new(
            "boss".to_string(),
            "Boss".to_string(),
            crate::agent::AgentKind::Persistent,
            boss_sm,
            Default::default(),
        ));
        let cluster = Arc::new(crate::agent::AgentCluster::new(
            Arc::clone(&boss),
            root.event_bus(),
        ));
        root.attach_shared_component(Arc::clone(&cluster)).unwrap();

        state
            .create_agent_task("task-1", "test", "test", Vec::new(), "boss", "Boss")
            .await
            .unwrap();
        state
            .assign_agent_task("task-1", "worker", "Worker")
            .await
            .unwrap();
        let (task, request) = state
            .request_agent_task_input(
                "task-1",
                "worker",
                "request-1",
                "Need a value",
                Vec::new(),
                true,
            )
            .await
            .unwrap();

        let worker = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &root).unwrap());
        worker
            .cache()
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &"worker".to_string(),
                None,
            )
            .await
            .unwrap();
        worker
            .cache()
            .set(
                crate::state_machine::agent_keys::CONVERSATION_ID,
                &"wake-test".to_string(),
                None,
            )
            .await
            .unwrap();
        let ctx = worker.create_context();
        let output = invoke_wake_delegator_agent(
            WakeDelegatorAgentInput {
                identity: AgentSystemInvocationIdentity::from_context(&ctx)
                    .await
                    .unwrap(),
                task_id: task.task_id,
                reason: DelegatorWakeReason::InputRequested,
                request_id: Some(request.request_id),
                task_revision: task.revision,
            },
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(output.target_agent_id, "boss");
        assert_eq!(
            output.outcome,
            crate::agent::cluster::AgentWakeOutcome::Woken
        );
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while boss.sm.current_state() != crate::state::states::SUSPENDED {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        state
            .create_agent_task("task-2", "report", "report", Vec::new(), "boss", "Boss")
            .await
            .unwrap();
        state
            .assign_agent_task("task-2", "worker", "Worker")
            .await
            .unwrap();
        let reported = state
            .report_agent_task(
                "task-2",
                "worker",
                crate::conversation_state::AgentTaskReport {
                    report_type: "completed".to_string(),
                    summary: "candidate".to_string(),
                    result: serde_json::Value::Null,
                    artifacts: Vec::new(),
                    reported_at: "now".to_string(),
                },
            )
            .await
            .unwrap();
        let output = invoke_wake_delegator_agent(
            WakeDelegatorAgentInput {
                identity: AgentSystemInvocationIdentity::from_context(&ctx)
                    .await
                    .unwrap(),
                task_id: reported.task_id,
                reason: DelegatorWakeReason::TaskReported,
                request_id: None,
                task_revision: reported.revision,
            },
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(output.target_agent_id, "boss");
        cluster.shutdown().await;
    }
}
