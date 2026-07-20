//! Agent gateway.
//! Conversation owns the ledger. AgentGateway is the only write path for user
//! input, focus changes, pause requests, and frontend-facing message records.

use std::sync::Arc;

use crate::agent::{AgentCluster, AgentId};
use crate::context::Message;
use crate::conversation_state::{ConversationState, LedgerReadOptions};
use crate::events::{
    AgentAppointRequestedPayload, AgentMessageProducedPayload, AgentPauseRequestedPayload,
    AgentReportSubmittedPayload, AgentTaskAssignedPayload, AgentTaskCreatedPayload,
    AgentTaskReportedPayload, LedgerRecordAppendedPayload,
};
use crate::ledger::{self, LedgerMessageMeta, LedgerRecord};
use crate::persistence::DisplayMeta;
use crate::systems::agent_route::{
    record_focus_change_if_needed, restore_focus_without_record, RouteAgentAppointmentInput,
    RouteAgentAppointmentSystem, RouteAgentReportInput, RouteAgentReportSystem,
};
use crate::systems::ledger::{
    AppendLedgerMessageInput, AppendLedgerMessageSystem, QueryLedgerInput, QueryLedgerSystem,
};
use crate::views::ViewPayload;
use async_trait::async_trait;
use corework::cache::CacheExt;
use corework::event::{BaseEvent, EventBus, EventHandler};
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};

pub type GatewayLedger = Arc<corework::execution_unit::ExecutionUnit>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactHistoryReport {
    pub done: Vec<(String, usize, usize)>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

enum CompactAgentOutcome {
    Done { before: usize, after: usize },
    Skipped { reason: &'static str },
    Failed { error: String },
}

static NEXT_COMMAND_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_command_id() -> String {
    let seq = NEXT_COMMAND_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let millis = chrono::Utc::now().timestamp_millis();
    format!("cmd_{millis}_{seq}")
}

#[derive(Debug, Clone)]
pub struct AdmissionResult {
    pub command_id: String,
    pub decision: crate::admission::Decision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "toolName")]
    pub tool_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "toolCommand"
    )]
    pub tool_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "callId")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "turnId")]
    pub turn_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "agentId")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "agentName")]
    pub agent_name: Option<String>,
}

#[async_trait]
impl EventHandler for AgentGateway {
    async fn handle(&self, event: &BaseEvent) -> corework::error::Result<()> {
        match event.event_type.as_str() {
            crate::events::types::AGENT_MESSAGE_PRODUCED => {
                let payload: AgentMessageProducedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                self.route_message_produced(payload).await;
            }
            crate::events::types::AGENT_PAUSE_REQUESTED => {
                let payload: AgentPauseRequestedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                self.route_pause_requested(payload).await;
            }
            crate::events::types::AGENT_APPOINT_REQUESTED => {
                let payload: AgentAppointRequestedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                if let Err(e) = RouteAgentAppointmentSystem
                    .execute(
                        RouteAgentAppointmentInput {
                            cluster: Arc::clone(&self.cluster),
                            ledger: Arc::clone(&self.ledger),
                            payload,
                        },
                        &self.ledger.create_context(),
                    )
                    .await
                {
                    tracing::warn!(
                        conversation_id = %self.conversation_id,
                        event_type = %crate::events::types::AGENT_APPOINT_REQUESTED,
                        error = %e,
                        "appoint agent route failed"
                    );
                }
            }
            crate::events::types::AGENT_REPORT_SUBMITTED => {
                let payload: AgentReportSubmittedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                if let Err(e) = RouteAgentReportSystem
                    .execute(
                        RouteAgentReportInput {
                            cluster: Arc::clone(&self.cluster),
                            ledger: Arc::clone(&self.ledger),
                            payload,
                        },
                        &self.ledger.create_context(),
                    )
                    .await
                {
                    tracing::warn!(
                        conversation_id = %self.conversation_id,
                        event_type = %crate::events::types::AGENT_REPORT_SUBMITTED,
                        error = %e,
                        "report agent route failed"
                    );
                }
            }
            crate::events::types::AGENT_TASK_CREATED => {
                let payload: AgentTaskCreatedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                self.route_agent_task_created(payload).await;
            }
            crate::events::types::AGENT_TASK_ASSIGNED => {
                let payload: AgentTaskAssignedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                self.route_agent_task_assigned(payload).await;
            }
            crate::events::types::AGENT_TASK_REPORTED => {
                let payload: AgentTaskReportedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                self.route_agent_task_reported(payload).await;
            }
            crate::events::types::AGENT_DYNAMIC_SNAPSHOT_SET => {
                self.publish_dynamic_snapshot_state_delta(event.payload.clone())
                    .await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::AGENT_SKILLS_CHANGED => {
                self.publish_agent_skills_state_delta(event.payload.clone())
                    .await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::LEDGER_RECORD_APPENDED => {
                let payload: LedgerRecordAppendedPayload =
                    serde_json::from_value(event.payload.clone())
                        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;
                let record = payload.record;
                self.after_ledger_record_appended(record.clone()).await;
                let ledger_delta = if record.role == ledger::LedgerRole::Summary {
                    None
                } else {
                    Some(crate::snapshot::ledger_append_delta(record))
                };
                self.publish_frontend_snapshot(ledger_delta).await;
            }
            crate::events::types::AGENT_SUSPENDED => {
                self.route_agent_lifecycle_fact(
                    ledger::GATEWAY_SUBTYPE_AGENT_SUSPENDED,
                    event.payload.clone(),
                )
                .await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::AGENT_CANCELED => {
                self.route_agent_lifecycle_fact(
                    ledger::GATEWAY_SUBTYPE_AGENT_CANCELED,
                    event.payload.clone(),
                )
                .await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::AGENT_COMPLETED
            | crate::events::types::AGENT_RESUMED
            | crate::events::types::CONVERSATION_STATE_CHANGED
            | corework::statemachine::SM_STATE_ENTER => {
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::AGENT_FOCUS_CHANGED => {
                self.publish_focus_state_delta(event.payload.clone()).await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::SNAPSHOT_UPDATED => {
                self.route_snapshot_updated_fact(event.payload.clone())
                    .await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::GATEWAY_COMMAND_ADMITTED
            | crate::events::types::GATEWAY_COMPACT_SKIPPED
            | crate::events::types::GATEWAY_COMPACT_DONE
            | crate::events::types::GATEWAY_COMPACT_FAILED => {
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::PLAN_WRITTEN
            | crate::events::types::PLAN_UPDATED
            | crate::events::types::PLAN_FINISHED => {
                self.publish_agent_plan_state_delta(event.payload.clone())
                    .await;
                self.publish_frontend_snapshot(None).await;
            }
            crate::events::types::LLM_USAGE | crate::events::types::LLM_ERROR => {
                self.publish_export_event(event.event_type.as_str(), event.payload.clone())
                    .await;
            }
            crate::events::types::TOOL_PERMISSION_REQUESTED
            | crate::events::types::TOOL_PERMISSION_RESOLVED => {
                self.publish_export_event(event.event_type.as_str(), event.payload.clone())
                    .await;
                self.publish_frontend_snapshot(None).await;
            }
            _ => {}
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "AgentGatewayRouter"
    }
}

pub struct AgentGateway {
    conversation_id: String,
    cluster: Arc<AgentCluster>,
    ledger: GatewayLedger,
    state: Arc<ConversationState>,
    export_event_bus: Arc<dyn EventBus>,
    snapshot_builder: crate::snapshot::SnapshotBuilder,
}

impl AgentGateway {
    pub fn new(cluster: Arc<AgentCluster>, ledger: GatewayLedger) -> Self {
        let export_event_bus = ledger.global_event_bus();
        let state = ledger
            .resolve_shared_component::<ConversationState>()
            .unwrap_or_else(|| {
                let state = Arc::new(ConversationState::new(
                    crate::ledger::DEFAULT_CONVERSATION_ID,
                    Default::default(),
                    cluster.default_agent_id(),
                ));
                ledger
                    .attach_shared_component(Arc::clone(&state))
                    .expect("install default conversation state");
                state
            });
        Self::new_for_conversation(
            crate::ledger::DEFAULT_CONVERSATION_ID.to_string(),
            cluster,
            ledger,
            state,
            export_event_bus,
        )
    }

    pub fn new_for_conversation(
        conversation_id: impl Into<String>,
        cluster: Arc<AgentCluster>,
        ledger: GatewayLedger,
        state: Arc<ConversationState>,
        export_event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            cluster,
            ledger,
            state,
            export_event_bus,
            snapshot_builder: crate::snapshot::SnapshotBuilder::new(),
        }
    }

    pub fn cluster(&self) -> &Arc<AgentCluster> {
        &self.cluster
    }

    pub fn ledger(&self) -> &GatewayLedger {
        &self.ledger
    }

    pub async fn install_routes(self: &Arc<Self>) -> crate::Result<()> {
        let handler: Arc<dyn EventHandler> = self.clone();
        let bus = self.ledger.event_bus();
        bus.subscribe(
            crate::events::types::AGENT_MESSAGE_PRODUCED.to_string(),
            Arc::clone(&handler),
        )
        .await?;
        bus.subscribe(
            crate::events::types::AGENT_PAUSE_REQUESTED.to_string(),
            Arc::clone(&handler),
        )
        .await?;
        bus.subscribe(
            crate::events::types::AGENT_APPOINT_REQUESTED.to_string(),
            Arc::clone(&handler),
        )
        .await?;
        bus.subscribe(
            crate::events::types::AGENT_REPORT_SUBMITTED.to_string(),
            Arc::clone(&handler),
        )
        .await?;
        for event_type in [
            crate::events::types::AGENT_TASK_CREATED,
            crate::events::types::AGENT_TASK_ASSIGNED,
            crate::events::types::AGENT_TASK_REPORTED,
            crate::events::types::AGENT_DYNAMIC_SNAPSHOT_SET,
            crate::events::types::AGENT_SKILLS_CHANGED,
        ] {
            bus.subscribe(event_type.to_string(), Arc::clone(&handler))
                .await?;
        }
        bus.subscribe(
            crate::events::types::LEDGER_RECORD_APPENDED.to_string(),
            Arc::clone(&handler),
        )
        .await?;
        for event_type in [
            crate::events::types::AGENT_SUSPENDED,
            crate::events::types::AGENT_RESUMED,
            crate::events::types::AGENT_COMPLETED,
            crate::events::types::AGENT_CANCELED,
            crate::events::types::AGENT_FOCUS_CHANGED,
            crate::events::types::CONVERSATION_STATE_CHANGED,
            corework::statemachine::SM_STATE_ENTER,
        ] {
            bus.subscribe(event_type.to_string(), Arc::clone(&handler))
                .await?;
        }
        let handler: Arc<dyn EventHandler> = self.clone();
        bus.subscribe(crate::events::types::SNAPSHOT_UPDATED.to_string(), handler)
            .await?;
        let handler: Arc<dyn EventHandler> = self.clone();
        for event_type in [
            crate::events::types::GATEWAY_COMMAND_ADMITTED,
            crate::events::types::GATEWAY_COMPACT_SKIPPED,
            crate::events::types::GATEWAY_COMPACT_DONE,
            crate::events::types::GATEWAY_COMPACT_FAILED,
            crate::events::types::PLAN_WRITTEN,
            crate::events::types::PLAN_UPDATED,
            crate::events::types::PLAN_FINISHED,
        ] {
            bus.subscribe(event_type.to_string(), Arc::clone(&handler))
                .await?;
        }
        let handler: Arc<dyn EventHandler> = self.clone();
        bus.subscribe(
            crate::events::types::LLM_USAGE.to_string(),
            Arc::clone(&handler),
        )
        .await?;
        bus.subscribe(crate::events::types::LLM_ERROR.to_string(), handler)
            .await?;
        let handler: Arc<dyn EventHandler> = self.clone();
        for event_type in [
            crate::events::types::TOOL_PERMISSION_REQUESTED,
            crate::events::types::TOOL_PERMISSION_RESOLVED,
        ] {
            bus.subscribe(event_type.to_string(), Arc::clone(&handler))
                .await?;
        }
        Ok(())
    }

    pub async fn send(&self, input: &str) -> crate::Result<()> {
        let _ = self.send_with_admission(input, None).await?;
        Ok(())
    }

    pub async fn send_with_admission(
        &self,
        input: &str,
        command_id: Option<String>,
    ) -> crate::Result<AdmissionResult> {
        let command = crate::admission::Command::SendMessage {
            content: input.to_string(),
        };
        let admission = self.admit_with_command_id(&command, command_id).await;
        match &admission.decision {
            crate::admission::Decision::Accepted { .. } => {
                if admission.decision.inserted_during_state().is_some() {
                    let agent = self.cluster.active_agent().await?;
                    agent.push_user_message(input).await?;
                } else {
                    self.cluster.send_to_active(input).await?;
                }
            }
            crate::admission::Decision::Rejected { .. } => {}
        }
        Ok(admission)
    }

    pub async fn request_pause(&self) -> crate::Result<()> {
        let _ = self.request_pause_with_admission(None).await?;
        Ok(())
    }

    pub async fn request_pause_with_admission(
        &self,
        command_id: Option<String>,
    ) -> crate::Result<AdmissionResult> {
        let command = crate::admission::Command::Pause;
        let admission = self.admit_with_command_id(&command, command_id).await;
        if matches!(
            admission.decision,
            crate::admission::Decision::Rejected { .. }
        ) {
            return Ok(admission);
        }
        let agent_id = self.focus_id().await;
        let agent_name = self
            .cluster
            .get(&agent_id)
            .await
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| agent_id.clone());
        self.ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_PAUSE_REQUESTED,
                serde_json::to_value(AgentPauseRequestedPayload {
                    agent_id,
                    agent_name,
                })?,
            ))
            .await?;
        Ok(admission)
    }

    pub async fn compact_history(
        &self,
        agent_ids: Vec<String>,
    ) -> crate::Result<CompactHistoryReport> {
        let (_, report) = self.compact_history_with_admission(agent_ids, None).await?;
        Ok(report)
    }

    pub async fn compact_history_with_admission(
        &self,
        agent_ids: Vec<String>,
        command_id: Option<String>,
    ) -> crate::Result<(AdmissionResult, CompactHistoryReport)> {
        let admission = self
            .admit_with_command_id(
                &crate::admission::Command::CompactHistory {
                    agent_ids: agent_ids.clone(),
                },
                command_id,
            )
            .await;
        if matches!(
            admission.decision,
            crate::admission::Decision::Rejected { .. }
        ) {
            return Ok((admission, CompactHistoryReport::default()));
        }

        let targets: Vec<String> = if agent_ids.is_empty() {
            self.cluster.list_agent_ids().await
        } else {
            agent_ids
        };

        self.set_compact_in_progress(true).await;
        self.publish_frontend_snapshot(None).await;

        let mut report = CompactHistoryReport::default();
        for agent_id in targets {
            match self.compact_single_agent(&agent_id).await {
                CompactAgentOutcome::Done { before, after } => {
                    report.done.push((agent_id.clone(), before, after));
                    self.publish_compact_event(
                        crate::events::types::GATEWAY_COMPACT_DONE,
                        serde_json::json!({
                            "agent_id": agent_id,
                            "before": before,
                            "after": after,
                        }),
                    )
                    .await;
                }
                CompactAgentOutcome::Skipped { reason } => {
                    report.skipped.push((agent_id.clone(), reason.to_string()));
                    self.publish_compact_event(
                        crate::events::types::GATEWAY_COMPACT_SKIPPED,
                        serde_json::json!({
                            "agent_id": agent_id,
                            "reason": reason,
                        }),
                    )
                    .await;
                }
                CompactAgentOutcome::Failed { error } => {
                    report.failed.push((agent_id.clone(), error.clone()));
                    self.publish_compact_event(
                        crate::events::types::GATEWAY_COMPACT_FAILED,
                        serde_json::json!({
                            "agent_id": agent_id,
                            "error": error,
                        }),
                    )
                    .await;
                }
            }
        }
        self.set_compact_in_progress(false).await;
        self.publish_frontend_snapshot(None).await;
        Ok((admission, report))
    }

    async fn compact_single_agent(&self, agent_id: &str) -> CompactAgentOutcome {
        let Some(agent) = self.cluster.get(agent_id).await else {
            return CompactAgentOutcome::Failed {
                error: format!("agent '{}' not found", agent_id),
            };
        };
        let cache = agent.sm.unit().cache();
        let records = QueryLedgerSystem
            .execute(
                QueryLedgerInput {
                    conversation_id: self.conversation_id.clone(),
                },
                &self.ledger.create_context(),
            )
            .await
            .unwrap_or_default();
        let agent_records = records
            .iter()
            .filter(|record| record.agent_id == agent_id)
            .collect::<Vec<_>>();
        let history = agent_records
            .iter()
            .filter_map(|record| record.to_context_message())
            .collect::<Vec<_>>();
        let has_recent_summary = agent_records
            .iter()
            .rev()
            .take(20)
            .any(|record| record.role == ledger::LedgerRole::Summary);
        if has_recent_summary {
            return CompactAgentOutcome::Skipped {
                reason: "recent_summary",
            };
        }
        let user_turns = history
            .iter()
            .filter(|m| m.role == crate::context::roles::USER)
            .count();
        if user_turns <= 20 {
            return CompactAgentOutcome::Skipped {
                reason: "history_too_short",
            };
        }
        let Some(model_uid) = crate::config_resolver::resolve_inference_model_uid(&cache).await
        else {
            return CompactAgentOutcome::Failed {
                error: "no_model_available".to_string(),
            };
        };
        let before = history.len();
        match crate::systems::history::compact_history_now(&history, model_uid, &cache).await {
            Ok(compact) => {
                if let Some(summary) = compact
                    .iter()
                    .rev()
                    .find(|m| {
                        m.role == crate::context::roles::SUMMARY
                            || m.role == crate::context::roles::COMPACT_SUMMARY
                    })
                    .cloned()
                {
                    self.record_message(agent_id.to_string(), agent.name.clone(), summary, None)
                        .await;
                }
                CompactAgentOutcome::Done {
                    before,
                    after: compact.len(),
                }
            }
            Err(e) => CompactAgentOutcome::Failed {
                error: e.to_string(),
            },
        }
    }

    async fn publish_compact_event(&self, event_type: &str, payload: serde_json::Value) {
        if let Err(e) = self
            .ledger
            .event_bus()
            .publish(BaseEvent::new(event_type, payload))
            .await
        {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                event_type = %event_type,
                error = %e,
                "publish compact event failed"
            );
        }
    }

    pub async fn admit(&self, command: &crate::admission::Command) -> crate::admission::Decision {
        self.admit_with_command_id(command, None).await.decision
    }

    pub async fn admit_with_command_id(
        &self,
        command: &crate::admission::Command,
        command_id: Option<String>,
    ) -> AdmissionResult {
        let agent_id = self.cluster.active_agent_id().await;
        let current_state = self
            .cluster
            .get(&agent_id)
            .await
            .map(|agent| agent.sm.current_state())
            .unwrap_or_default();
        let compact_in_progress = self.compact_in_progress().await;
        let decision = if matches!(command, crate::admission::Command::SendMessage { .. })
            && compact_in_progress
        {
            crate::admission::Decision::Rejected {
                reason: crate::admission::RejectReason::CompactInProgress,
            }
        } else if matches!(command, crate::admission::Command::CompactHistory { .. })
            && self
                .cluster
                .snapshot()
                .await
                .agents
                .iter()
                .any(|agent| agent.state != crate::state::states::SUSPENDED)
        {
            crate::admission::Decision::Rejected {
                reason: crate::admission::RejectReason::ConversationNotWaiting,
            }
        } else {
            crate::admission::AdmissionPolicy::decide(command, &current_state)
        };
        let command_id = command_id.unwrap_or_else(next_command_id);
        self.publish_command_admitted(command, &decision, &agent_id, &current_state, &command_id)
            .await;
        AdmissionResult {
            command_id,
            decision,
        }
    }

    async fn compact_in_progress(&self) -> bool {
        let agent_id = self.cluster.active_agent_id().await;
        let Some(agent) = self.cluster.get(&agent_id).await else {
            return false;
        };
        agent
            .sm
            .unit()
            .cache()
            .get::<bool>(crate::context::keys::COMPACT_IN_PROGRESS)
            .await
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    async fn set_compact_in_progress(&self, value: bool) {
        for agent_id in self.cluster.list_agent_ids().await {
            let Some(agent) = self.cluster.get(&agent_id).await else {
                continue;
            };
            if let Err(e) = agent
                .sm
                .unit()
                .cache()
                .set(crate::context::keys::COMPACT_IN_PROGRESS, &value, None)
                .await
            {
                tracing::warn!(
                    conversation_id = %self.conversation_id,
                    agent_id = %agent_id,
                    error = %e,
                    "set compact-in-progress flag failed"
                );
            }
        }
    }

    async fn publish_command_admitted(
        &self,
        command: &crate::admission::Command,
        decision: &crate::admission::Decision,
        agent_id: &str,
        current_state: &str,
        command_id: &str,
    ) {
        use crate::admission::Decision;
        let (decision_str, inserted_during_state, reason) = match decision {
            Decision::Accepted {
                inserted_during_state,
            } => ("accepted", inserted_during_state.clone(), None),
            Decision::Rejected { reason } => ("rejected", None, Some(reason.as_str())),
        };
        let mut payload = serde_json::json!({
            "command_id": command_id,
            "command": command.name(),
            "decision": decision_str,
            "agent_id": agent_id,
            "current_state": current_state,
        });
        if let Some(state) = inserted_during_state {
            payload["inserted_during_state"] = serde_json::Value::String(state);
        }
        if let Some(reason) = reason {
            payload["reason"] = serde_json::Value::String(reason.to_string());
        }
        if let Err(e) = self
            .ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::GATEWAY_COMMAND_ADMITTED,
                payload,
            ))
            .await
        {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                agent_id = %agent_id,
                command_id = %command_id,
                command = %command.name(),
                error = %e,
                "publish gateway command admitted failed"
            );
        }
    }

    async fn publish_frontend_snapshot(&self, ledger_delta: Option<crate::snapshot::LedgerDelta>) {
        let conversation_id = ledger_delta
            .as_ref()
            .map(|delta| delta.record.conversation_id.clone())
            .unwrap_or_else(|| self.conversation_id.clone());
        let snapshot = self
            .snapshot_builder
            .build_if_changed(conversation_id, &self.cluster, ledger_delta)
            .await;
        let Some(snapshot) = snapshot else {
            return;
        };
        let payload = match serde_json::to_value(snapshot) {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(
                    conversation_id = %self.conversation_id,
                    error = %e,
                    "serialize frontend state snapshot failed"
                );
                return;
            }
        };
        if let Err(e) = self
            .export_event_bus
            .publish(
                BaseEvent::new(crate::events::types::FRONTEND_STATE_SNAPSHOT, payload)
                    .with_conversation_id(self.conversation_id.clone()),
            )
            .await
        {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                event_type = %crate::events::types::FRONTEND_STATE_SNAPSHOT,
                error = %e,
                "publish frontend state snapshot failed"
            );
        }
    }

    async fn publish_export_event(&self, event_type: &str, mut payload: serde_json::Value) {
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("conversation_id".to_string())
                .or_insert_with(|| serde_json::Value::String(self.conversation_id.clone()));
        }
        if let Err(e) = self
            .export_event_bus
            .publish(
                BaseEvent::new(event_type, payload)
                    .with_conversation_id(self.conversation_id.clone()),
            )
            .await
        {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                event_type = %event_type,
                error = %e,
                "publish export event failed"
            );
        }
    }

    async fn publish_state_delta(&self, mut payload: serde_json::Value) {
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("schema".to_string()).or_insert_with(|| {
                serde_json::Value::String("agent-runtime-state-delta/v1".to_string())
            });
            obj.entry("conversation_id".to_string())
                .or_insert_with(|| serde_json::Value::String(self.conversation_id.clone()));
        }
        self.publish_export_event(crate::events::types::CONVERSATION_STATE_DELTA, payload)
            .await;
    }

    async fn publish_ledger_delta(&self, record: &LedgerRecord) {
        self.publish_export_event(
            crate::events::types::CONVERSATION_LEDGER_DELTA,
            serde_json::json!({
                "schema": "agent-runtime-ledger-delta/v1",
                "op": "append",
                "record_id": record.record_id,
                "conversation_id": record.conversation_id,
                "record": record,
            }),
        )
        .await;
    }

    async fn publish_task_state_delta(&self, task: &crate::conversation_state::AgentTaskEntry) {
        self.publish_state_delta(serde_json::json!({
            "op": "agent_task.upsert",
            "task_id": task.task_id,
            "task": task,
        }))
        .await;
    }

    async fn publish_focus_state_delta(&self, payload: serde_json::Value) {
        self.publish_state_delta(serde_json::json!({
            "op": "focus.set",
            "focus_agent_id": payload.get("to_agent_id").cloned().unwrap_or(serde_json::Value::Null),
            "from_agent_id": payload.get("from_agent_id").cloned().unwrap_or(serde_json::Value::Null),
            "reason": payload.get("reason").cloned().unwrap_or(serde_json::Value::Null),
            "state": payload,
        }))
        .await;
    }

    async fn publish_dynamic_snapshot_state_delta(&self, payload: serde_json::Value) {
        self.publish_state_delta(serde_json::json!({
            "op": "dynamic_snapshot.set",
            "agent_id": payload.get("agent_id").cloned().unwrap_or(serde_json::Value::Null),
            "field": payload.get("field").cloned().unwrap_or(serde_json::Value::Null),
            "text": payload.get("text").cloned().unwrap_or(serde_json::Value::Null),
            "host_owned": payload.get("host_owned").cloned().unwrap_or_else(|| serde_json::json!(true)),
            "stale_after_restore": payload
                .get("stale_after_restore")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(true)),
        }))
        .await;
    }

    async fn publish_agent_skills_state_delta(&self, payload: serde_json::Value) {
        self.publish_state_delta(serde_json::json!({
            "op": "agent_skills.set",
            "agent_id": payload.get("agent_id").cloned().unwrap_or(serde_json::Value::Null),
            "agent_name": payload.get("agent_name").cloned().unwrap_or(serde_json::Value::Null),
            "main_skills": payload.get("main_skills").cloned().unwrap_or_else(|| serde_json::json!([])),
            "imported_skills": payload
                .get("imported_skills")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
            "active_tools": payload.get("active_tools").cloned().unwrap_or_else(|| serde_json::json!([])),
        }))
        .await;
    }

    async fn publish_agent_plan_state_delta(&self, payload: serde_json::Value) {
        self.publish_state_delta(serde_json::json!({
            "op": "agent_plan.set",
            "agent_id": payload.get("agent_id").cloned().unwrap_or(serde_json::Value::Null),
            "agent_name": payload.get("agent_name").cloned().unwrap_or(serde_json::Value::Null),
            "plan": payload,
        }))
        .await;
    }

    async fn route_pause_requested(&self, payload: AgentPauseRequestedPayload) {
        let agent_id = payload.agent_id.clone();
        self.record_gateway_fact(
            payload.agent_id,
            payload.agent_name,
            ledger::GATEWAY_SUBTYPE_PAUSE_REQUESTED,
            "",
            LedgerMessageMeta::default(),
        )
        .await;
        if let Err(e) = self.cluster.pause_agent(&agent_id).await {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                agent_id = %agent_id,
                error = %e,
                "pause agent failed"
            );
        }
    }

    async fn route_message_produced(&self, payload: AgentMessageProducedPayload) {
        match self.append_message_record(payload).await {
            Ok(_) => {}
            Err(e) => tracing::warn!(
                conversation_id = %self.conversation_id,
                error = %e,
                "route agent message into ledger failed"
            ),
        }
    }

    async fn route_agent_task_created(&self, payload: AgentTaskCreatedPayload) {
        match self
            .state
            .create_agent_task(
                payload.task_id.clone(),
                payload.title.clone(),
                payload.objective.clone(),
                payload.acceptance.clone(),
                payload.delegator_agent_id.clone(),
                payload.delegator_agent_name.clone(),
            )
            .await
        {
            Ok(task) => {
                let mut metadata = LedgerMessageMeta::default();
                metadata
                    .extra
                    .insert("task_id".to_string(), serde_json::json!(task.task_id));
                metadata
                    .extra
                    .insert("title".to_string(), serde_json::json!(task.title));
                metadata
                    .extra
                    .insert("objective".to_string(), serde_json::json!(task.objective));
                metadata
                    .extra
                    .insert("acceptance".to_string(), serde_json::json!(task.acceptance));
                self.record_gateway_fact(
                    payload.delegator_agent_id,
                    payload.delegator_agent_name,
                    ledger::GATEWAY_SUBTYPE_AGENT_TASK_CREATED,
                    "background task created",
                    metadata,
                )
                .await;
                self.publish_task_state_delta(&task).await;
                self.publish_frontend_snapshot(None).await;
            }
            Err(e) => tracing::warn!(
                conversation_id = %self.conversation_id,
                task_id = %payload.task_id,
                agent_id = %payload.delegator_agent_id,
                error = %e,
                "create agent task route failed"
            ),
        }
    }

    async fn route_agent_task_assigned(&self, payload: AgentTaskAssignedPayload) {
        match self
            .state
            .assign_agent_task(
                &payload.task_id,
                payload.assignee_agent_id.clone(),
                payload.assignee_agent_name.clone(),
            )
            .await
        {
            Ok(task) => {
                let mut metadata = LedgerMessageMeta::default();
                metadata
                    .extra
                    .insert("task_id".to_string(), serde_json::json!(task.task_id));
                metadata.extra.insert(
                    "assignee_agent_id".to_string(),
                    serde_json::json!(payload.assignee_agent_id),
                );
                metadata.extra.insert(
                    "assignee_agent_name".to_string(),
                    serde_json::json!(payload.assignee_agent_name),
                );
                self.record_gateway_fact(
                    task.delegator_agent_id.clone(),
                    task.delegator_agent_name.clone(),
                    ledger::GATEWAY_SUBTYPE_AGENT_TASK_ASSIGNED,
                    "background task assigned",
                    metadata,
                )
                .await;
                self.publish_task_state_delta(&task).await;
                self.publish_frontend_snapshot(None).await;
            }
            Err(e) => tracing::warn!(
                conversation_id = %self.conversation_id,
                task_id = %payload.task_id,
                agent_id = %payload.assignee_agent_id,
                error = %e,
                "assign agent task route failed"
            ),
        }
    }

    async fn route_agent_task_reported(&self, payload: AgentTaskReportedPayload) {
        let report = crate::conversation_state::AgentTaskReport {
            report_type: payload.report_type.clone(),
            summary: payload.summary.clone(),
            result: payload.result.clone(),
            artifacts: payload.artifacts.clone(),
            reported_at: chrono::Local::now().to_rfc3339(),
        };
        let task = match self
            .state
            .report_agent_task(&payload.task_id, &payload.reporter_agent_id, report)
            .await
        {
            Ok(task) => task,
            Err(e) => {
                tracing::warn!(
                    conversation_id = %self.conversation_id,
                    task_id = %payload.task_id,
                    agent_id = %payload.reporter_agent_id,
                    error = %e,
                    "report agent task route failed"
                );
                return;
            }
        };

        let mut metadata = LedgerMessageMeta {
            subtype: Some(ledger::GATEWAY_SUBTYPE_AGENT_TASK_REPORT.to_string()),
            from_agent_id: Some(payload.reporter_agent_id.clone()),
            to_agent_id: Some(task.delegator_agent_id.clone()),
            reason: Some(payload.report_type.clone()),
            ..Default::default()
        };
        metadata.extra.insert(
            "task_id".to_string(),
            serde_json::json!(payload.task_id.clone()),
        );
        metadata.extra.insert(
            "from_agent_name".to_string(),
            serde_json::json!(payload.reporter_agent_name.clone()),
        );
        metadata.extra.insert(
            "report_type".to_string(),
            serde_json::json!(payload.report_type.clone()),
        );
        metadata.extra.insert(
            "artifacts".to_string(),
            serde_json::json!(payload.artifacts),
        );
        if !payload.result.is_null() {
            metadata
                .extra
                .insert("result".to_string(), payload.result.clone());
        }

        let content = format!(
            "[Background task report: {}]\nTask: {}\nStatus: {}\n{}",
            payload.reporter_agent_name, payload.task_id, payload.report_type, payload.summary
        );
        if let Err(e) = AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: self.conversation_id.clone(),
                    agent_id: task.delegator_agent_id.clone(),
                    agent_name: task.delegator_agent_name.clone(),
                    role: ledger::LedgerRole::AgentReport,
                    content,
                    metadata,
                    display: Some(DisplayMeta {
                        display_role: "agent_report".to_string(),
                        tool_name: None,
                        tool_command: None,
                        success: None,
                        reasoning: None,
                        decision: None,
                        tools: Vec::new(),
                        agent_name: Some(payload.reporter_agent_name),
                    }),
                    tool_call_id: None,
                    tool_name: None,
                },
                &self.ledger.create_context(),
            )
            .await
        {
            tracing::warn!("append delegated task report failed: {}", e);
        }

        self.publish_task_state_delta(&task).await;
        self.publish_frontend_snapshot(None).await;
    }

    async fn route_agent_lifecycle_fact(&self, subtype: &str, payload: serde_json::Value) {
        let agent_id = payload
            .get("agent_id")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| self.cluster.default_agent_id())
            .to_string();
        let agent_name = payload
            .get("agent_name")
            .and_then(|value| value.as_str())
            .unwrap_or(&agent_id)
            .to_string();
        let mut metadata = LedgerMessageMeta::default();
        if let Some(state) = payload.get("state").cloned() {
            metadata.extra.insert("state".to_string(), state);
        }
        self.record_gateway_fact(agent_id, agent_name, subtype, "", metadata)
            .await;
    }

    async fn route_snapshot_updated_fact(&self, payload: serde_json::Value) {
        let agent_id = self.focus_id().await;
        let agent_name = self
            .cluster
            .get(&agent_id)
            .await
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| agent_id.clone());
        let mut metadata = LedgerMessageMeta::default();
        if let Some(version) = payload.get("version").cloned() {
            metadata.extra.insert("version".to_string(), version);
        }
        self.record_gateway_fact(
            agent_id,
            agent_name,
            ledger::GATEWAY_SUBTYPE_SNAPSHOT_UPDATED,
            "",
            metadata,
        )
        .await;
    }

    async fn append_message_record(
        &self,
        payload: AgentMessageProducedPayload,
    ) -> Result<LedgerRecord, corework::error::FrameworkError> {
        let conversation_id = if payload.conversation_id == ledger::DEFAULT_CONVERSATION_ID {
            self.conversation_id.clone()
        } else {
            payload.conversation_id
        };
        AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id,
                    agent_id: payload.agent_id,
                    agent_name: payload.agent_name,
                    role: payload.role,
                    content: payload.content,
                    metadata: payload.metadata,
                    display: payload.display,
                    tool_call_id: payload.tool_call_id,
                    tool_name: payload.tool_name,
                },
                &self.ledger.create_context(),
            )
            .await
    }

    async fn append_gateway_fact_record(
        &self,
        agent_id: String,
        agent_name: String,
        subtype: impl Into<String>,
        content: impl Into<String>,
        metadata: LedgerMessageMeta,
    ) -> Result<LedgerRecord, corework::error::FrameworkError> {
        let mut meta = metadata;
        meta.subtype = Some(subtype.into());
        AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: self.conversation_id.clone(),
                    agent_id,
                    agent_name,
                    role: ledger::LedgerRole::GatewayMessage,
                    content: content.into(),
                    metadata: meta,
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                },
                &self.ledger.create_context(),
            )
            .await
    }

    async fn after_ledger_record_appended(&self, record: LedgerRecord) {
        self.publish_ledger_delta(&record).await;
        let focus_id = self.focus_id().await;
        if record.agent_id == focus_id {
            if let Some(gateway_message) = record.to_frontend_message() {
                self.publish_message_append(
                    record.agent_id.clone(),
                    record.agent_name.clone(),
                    gateway_message,
                )
                .await;
            }
        }

        if crate::persistence::auto_file_persistence_enabled() {
            if let Err(e) = crate::persistence::append_ledger_record_current(&record).await {
                tracing::warn!("persist routed ledger record failed: {}", e);
            }
        }
    }

    pub async fn record_gateway_fact(
        &self,
        agent_id: String,
        agent_name: String,
        subtype: impl Into<String>,
        content: impl Into<String>,
        metadata: LedgerMessageMeta,
    ) {
        match self
            .append_gateway_fact_record(agent_id, agent_name, subtype, content, metadata)
            .await
        {
            Ok(_) => {}
            Err(e) => tracing::warn!("route gateway fact into ledger failed: {}", e),
        }
    }

    pub async fn set_focus(&self, agent_id: Option<String>) {
        let id = agent_id.unwrap_or_else(|| self.cluster.default_agent_id().to_string());
        if let Err(e) = self.set_focus_id(id).await {
            tracing::warn!("set agent focus failed: {}", e);
        }
    }

    pub async fn set_focus_id(&self, agent_id: AgentId) -> crate::Result<()> {
        record_focus_change_if_needed(&self.cluster, &self.ledger, agent_id, "gateway")
            .await
            .map_err(|error| crate::Error::Other(anyhow::anyhow!(error.to_string())))?;
        Ok(())
    }

    pub async fn focus(&self) -> Option<String> {
        Some(self.cluster.active_agent_id().await)
    }

    pub async fn focus_id(&self) -> AgentId {
        self.state.focus().await
    }

    pub async fn record_message(
        &self,
        agent_id: String,
        agent_name: String,
        message: Message,
        display: Option<DisplayMeta>,
    ) {
        let role = ledger::LedgerRole::from_message_role(&message.role, display.as_ref());
        match AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: self.conversation_id.clone(),
                    agent_id,
                    agent_name,
                    role,
                    content: message.content,
                    metadata: LedgerMessageMeta::default(),
                    display,
                    tool_call_id: message.tool_call_id,
                    tool_name: message.name,
                },
                &self.ledger.create_context(),
            )
            .await
        {
            Ok(_) => {}
            Err(e) => tracing::warn!("record gateway ledger message failed: {}", e),
        }
    }

    pub async fn publish_ledger_snapshot(&self) {
        let agent_id = self.focus_id().await;
        let agent_name = self
            .cluster
            .get(&agent_id)
            .await
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| agent_id.clone());
        let messages = self
            .read_records()
            .await
            .iter()
            .filter(|record| record.agent_id == agent_id)
            .filter_map(|record| record.to_frontend_message())
            .collect::<Vec<_>>();

        self.publish_export_event(
            crate::events::types::MESSAGES_CHANGED,
            serde_json::to_value(crate::events::MessagesChangedPayload {
                agent_id,
                agent_name,
                mode: "replace".to_string(),
                messages,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    }

    pub async fn restore_from_persisted(&self, records: Vec<crate::persistence::PersistedMessage>) {
        let mut restored = Vec::new();
        for (index, record) in records.into_iter().enumerate() {
            let mut ledger_record = record.into_ledger((index + 1) as u64);
            ledger_record.agent_name = self
                .cluster
                .get(&ledger_record.agent_id)
                .await
                .map(|agent| agent.name.clone())
                .unwrap_or_else(|| ledger_record.agent_name.clone());
            restored.push(ledger_record);
        }

        self.write_records(restored).await;
        self.publish_ledger_snapshot().await;
    }

    pub async fn restore_focus_from_ledger(&self) {
        let Some(target_agent_id) = self
            .read_records()
            .await
            .iter()
            .rev()
            .find(|record| {
                record.role == ledger::LedgerRole::GatewayMessage
                    && record.metadata.subtype.as_deref()
                        == Some(ledger::GATEWAY_SUBTYPE_FOCUS_CHANGED)
            })
            .and_then(|record| record.metadata.to_agent_id.clone())
        else {
            return;
        };

        if self.cluster.get(&target_agent_id).await.is_none() {
            tracing::warn!(
                "skip restoring focus to missing agent from ledger: {}",
                target_agent_id
            );
            return;
        }

        if let Err(e) =
            restore_focus_without_record(&self.cluster, &self.ledger, target_agent_id.clone()).await
        {
            tracing::warn!("restore focus failed: {}", e);
            return;
        }
        self.publish_ledger_snapshot().await;
    }

    pub async fn agent_context(&self, agent_id: &str) -> Vec<Message> {
        ledger::agent_context(&self.read_records().await, agent_id)
    }

    async fn read_records(&self) -> Vec<LedgerRecord> {
        self.state.list_recent(LedgerReadOptions::default()).await
    }

    async fn write_records(&self, records: Vec<LedgerRecord>) {
        self.state.replace(records).await;
    }

    async fn publish_message_append(
        &self,
        agent_id: String,
        agent_name: String,
        message: GatewayMessage,
    ) {
        self.publish_export_event(
            crate::events::types::MESSAGES_CHANGED,
            serde_json::to_value(crate::events::MessagesChangedPayload {
                agent_id,
                agent_name,
                mode: "append".to_string(),
                messages: vec![message],
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
    }
}
