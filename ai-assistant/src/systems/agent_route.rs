use std::sync::Arc;

use async_trait::async_trait;
use corework::cache::CacheExt;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use crate::agent::{AgentCluster, AgentId};
use crate::events::{AgentAppointRequestedPayload, AgentReportSubmittedPayload};
use crate::gateway::GatewayLedger;
use crate::ledger::{self, LedgerMessageMeta, LedgerRole, FOCUS_RESOURCE_KEY};
use crate::systems::ledger::{AppendLedgerMessageInput, AppendLedgerMessageSystem};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRouteResult {
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub to_agent_name: String,
    pub focus_changed: bool,
}

#[derive(Clone)]
pub struct RouteAgentAppointmentInput {
    pub cluster: Arc<AgentCluster>,
    pub ledger: GatewayLedger,
    pub payload: AgentAppointRequestedPayload,
}

#[derive(Clone)]
pub struct RouteAgentReportInput {
    pub cluster: Arc<AgentCluster>,
    pub ledger: GatewayLedger,
    pub payload: AgentReportSubmittedPayload,
}

pub struct RouteAgentAppointmentSystem;

#[async_trait]
impl SystemOperation for RouteAgentAppointmentSystem {
    type Input = RouteAgentAppointmentInput;
    type Output = AgentRouteResult;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let target_agent = input
            .cluster
            .appoint_from(
                &input.payload.from_agent_id,
                &input.payload.target,
                input.payload.message.clone(),
            )
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
        let to_agent_id = target_agent.id.clone();
        let focus_changed = record_focus_change_if_needed(
            &input.cluster,
            &input.ledger,
            to_agent_id.clone(),
            "appoint",
        )
        .await?;
        target_agent
            .drive(None)
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
        Ok(route_result(
            &input.cluster,
            input.payload.from_agent_id,
            to_agent_id,
            focus_changed,
        )
        .await)
    }

    fn name(&self) -> &str {
        "RouteAgentAppointmentSystem"
    }
}

pub struct RouteAgentReportSystem;

#[async_trait]
impl SystemOperation for RouteAgentReportSystem {
    type Input = RouteAgentReportInput;
    type Output = AgentRouteResult;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let handoff_target = input
            .cluster
            .report_to(
                &input.payload.from_agent_id,
                &input.payload.target,
                &input.payload.report_type,
                input.payload.report.clone(),
                input.payload.handoff,
            )
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
        let (to_agent_id, focus_changed) = if let Some(target_agent) = handoff_target {
            let to_agent_id = target_agent.id.clone();
            let focus_changed = record_focus_change_if_needed(
                &input.cluster,
                &input.ledger,
                to_agent_id.clone(),
                "report",
            )
            .await?;
            target_agent
                .drive(None)
                .await
                .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
            (to_agent_id, focus_changed)
        } else {
            (input.cluster.active_agent_id().await, false)
        };
        Ok(route_result(
            &input.cluster,
            input.payload.from_agent_id,
            to_agent_id,
            focus_changed,
        )
        .await)
    }

    fn name(&self) -> &str {
        "RouteAgentReportSystem"
    }
}

async fn route_result(
    cluster: &Arc<AgentCluster>,
    from_agent_id: String,
    to_agent_id: AgentId,
    focus_changed: bool,
) -> AgentRouteResult {
    let to_agent_name = cluster
        .get(&to_agent_id)
        .await
        .map(|agent| agent.name.clone())
        .unwrap_or_else(|| to_agent_id.clone());
    AgentRouteResult {
        from_agent_id,
        to_agent_id,
        to_agent_name,
        focus_changed,
    }
}

pub(crate) async fn record_focus_change_if_needed(
    cluster: &Arc<AgentCluster>,
    ledger: &GatewayLedger,
    to_agent_id: String,
    reason: &str,
) -> Result<bool, FrameworkError> {
    let state = ledger
        .resolve_shared_component::<crate::conversation_state::ConversationState>()
        .ok_or_else(|| {
            FrameworkError::InvalidOperation(
                "conversation state is unavailable from the ledger hierarchy".to_string(),
            )
        })?;
    let _transition_guard = state.lock_focus_transition().await;
    let from_agent_id = state.focus().await;
    cluster
        .set_active_agent_id(to_agent_id.clone())
        .await
        .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
    ledger
        .cache()
        .set(FOCUS_RESOURCE_KEY, &to_agent_id, None)
        .await
        .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
    if from_agent_id == to_agent_id {
        return Ok(false);
    }

    let from_agent_name = cluster
        .get(&from_agent_id)
        .await
        .map(|agent| agent.name.clone())
        .unwrap_or_else(|| from_agent_id.clone());
    let to_agent_name = cluster
        .get(&to_agent_id)
        .await
        .map(|agent| agent.name.clone())
        .unwrap_or_else(|| to_agent_id.clone());

    if let Err(e) = cluster
        .event_bus()
        .publish(corework::event::BaseEvent::new(
            crate::events::types::AGENT_FOCUS_CHANGED,
            serde_json::json!({
                "from_agent_id": from_agent_id,
                "from_agent_name": from_agent_name,
                "to_agent_id": to_agent_id,
                "to_agent_name": to_agent_name,
                "reason": reason,
            }),
        ))
        .await
    {
        tracing::warn!("publish agent focus changed event failed: {}", e);
    }

    if let Some(agent) = cluster.get(&to_agent_id).await {
        let cache = agent.sm.unit().cache();
        let event_bus = cluster.event_bus();
        crate::agent::publish_focus_view_for_cache(
            agent.sm.unit().as_ref(),
            &*cache,
            event_bus.as_ref(),
            &agent.sm.current_state(),
        )
        .await;
    }

    let mut meta = LedgerMessageMeta::default();
    meta.from_agent_id = Some(from_agent_id.clone());
    meta.to_agent_id = Some(to_agent_id.clone());
    meta.reason = Some(reason.to_string());
    AppendLedgerMessageSystem
        .execute(
            AppendLedgerMessageInput {
                conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                agent_id: to_agent_id.clone(),
                agent_name: to_agent_name,
                role: LedgerRole::GatewayMessage,
                content: format!(
                    "focus changed: {} -> {} ({})",
                    from_agent_id, to_agent_id, reason
                ),
                metadata: {
                    meta.subtype = Some(ledger::GATEWAY_SUBTYPE_FOCUS_CHANGED.to_string());
                    meta
                },
                display: None,
                tool_call_id: None,
                tool_name: None,
            },
            &ledger.create_context(),
        )
        .await?;
    Ok(true)
}

pub(crate) async fn restore_focus_without_record(
    cluster: &Arc<AgentCluster>,
    ledger: &GatewayLedger,
    to_agent_id: String,
) -> Result<(), FrameworkError> {
    let state = ledger
        .resolve_shared_component::<crate::conversation_state::ConversationState>()
        .ok_or_else(|| {
            FrameworkError::InvalidOperation(
                "conversation state is unavailable from the ledger hierarchy".to_string(),
            )
        })?;
    let _transition_guard = state.lock_focus_transition().await;
    cluster
        .set_active_agent_id(to_agent_id.clone())
        .await
        .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
    ledger
        .cache()
        .set(FOCUS_RESOURCE_KEY, &to_agent_id, None)
        .await
        .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentKind, AgentPermissions, AgentRuntime};
    use crate::context::keys;
    use corework::cache::CacheExt;
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::statemachine::{FnState, SimpleTransition, StateMachine};
    use corework::system::SystemOperation;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    async fn test_ledger() -> GatewayLedger {
        static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
        let scope_id = NEXT_SCOPE.fetch_add(1, Ordering::Relaxed);
        let conversation_id = format!("agent-route-test:{scope_id}");
        let framework = corework::world::FrameworkState::initialize().unwrap();
        let ledger = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            format!("ai-assistant:test-ledger:{scope_id}"),
        ));
        ledger
            .cache()
            .set(
                ledger::LEDGER_RESOURCE_KEY,
                &Vec::<ledger::LedgerRecord>::new(),
                None,
            )
            .await
            .unwrap();
        ledger
            .cache()
            .set(
                ledger::FOCUS_RESOURCE_KEY,
                &crate::agent::keys::BOSS_AGENT_ID.to_string(),
                None,
            )
            .await
            .unwrap();
        crate::agent::set_conversation_id_in_cache(&*ledger.cache(), &conversation_id)
            .await
            .unwrap();
        ledger
            .attach_shared_component(Arc::new(crate::conversation_state::ConversationState::new(
                conversation_id,
                Default::default(),
                crate::agent::keys::BOSS_AGENT_ID,
            )))
            .unwrap();
        ledger
    }

    async fn test_sm(
        id: &str,
        name: &str,
        initial: &str,
        parent: &GatewayLedger,
    ) -> Arc<StateMachine> {
        let suspended = FnState::new(crate::state::states::SUSPENDED)
            .with_description("test suspended")
            .add_transition(
                crate::state::events::USER_INPUT,
                Box::new(SimpleTransition::new(
                    crate::state::events::USER_INPUT,
                    crate::state::states::SUSPENDED,
                )),
            );
        let executing = FnState::new(crate::state::states::EXECUTING)
            .with_description("test executing")
            .add_transition(
                crate::state::events::PAUSE,
                Box::new(SimpleTransition::new(
                    crate::state::events::PAUSE,
                    crate::state::states::SUSPENDED,
                )),
            );
        let sm = Arc::new(
            StateMachine::builder("test_agent")
                .add_state(Box::new(suspended))
                .add_state(Box::new(executing))
                .initial_state(initial)
                .with_parent_unit(parent.clone())
                .build()
                .await
                .unwrap(),
        );
        let cache = sm.unit().cache();
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &id.to_string(),
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_NAME,
                &name.to_string(),
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_CLASS,
                &"interactive".to_string(),
                None,
            )
            .await
            .unwrap();
        sm.start().await.unwrap();
        sm
    }

    async fn runtime(
        id: &str,
        name: &str,
        initial: &str,
        permissions: AgentPermissions,
        parent: &GatewayLedger,
    ) -> Arc<AgentRuntime> {
        Arc::new(AgentRuntime::new(
            id.to_string(),
            name.to_string(),
            AgentKind::Persistent,
            test_sm(id, name, initial, parent).await,
            permissions,
        ))
    }

    async fn ledger_records(ledger: &GatewayLedger) -> Vec<ledger::LedgerRecord> {
        ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .unwrap()
            .list_recent(crate::conversation_state::LedgerReadOptions::default())
            .await
    }

    async fn cached_focus(ledger: &GatewayLedger) -> String {
        ledger
            .cache()
            .get(FOCUS_RESOURCE_KEY)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn appointment_routes_target_message_and_focus_fact() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::SUSPENDED,
            AgentPermissions {
                can_appoint: true,
                ..Default::default()
            },
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        let worker = runtime(
            "agent_1",
            "Worker",
            crate::state::states::SUSPENDED,
            AgentPermissions::default(),
            &ledger,
        )
        .await;
        cluster.register_for_test(worker).await;
        let gateway = Arc::new(crate::gateway::AgentGateway::new(
            Arc::clone(&cluster),
            Arc::clone(&ledger),
        ));
        gateway.install_routes().await.unwrap();

        let route = RouteAgentAppointmentSystem
            .execute(
                RouteAgentAppointmentInput {
                    cluster: Arc::clone(&cluster),
                    ledger: Arc::clone(&ledger),
                    payload: crate::events::AgentAppointRequestedPayload {
                        from_agent_id: crate::agent::keys::BOSS_AGENT_ID.to_string(),
                        from_agent_name: "Boss".to_string(),
                        target: "agent_1".to_string(),
                        message: "Handle this task".to_string(),
                    },
                },
                &ledger.create_context(),
            )
            .await
            .unwrap();

        assert!(route.focus_changed);
        assert_eq!(cluster.active_agent_id().await, "agent_1");
        assert_eq!(cached_focus(&ledger).await, "agent_1");
        let records = ledger_records(&ledger).await;
        assert!(records.iter().any(|record| {
            record.agent_id == "agent_1"
                && record.role == LedgerRole::User
                && record.content.contains("Handle this task")
                && record.metadata.subtype.as_deref()
                    == Some(ledger::GATEWAY_SUBTYPE_AGENT_APPOINTMENT)
        }));
        assert!(records.iter().any(|record| {
            record.role == LedgerRole::GatewayMessage
                && record.metadata.subtype.as_deref() == Some(ledger::GATEWAY_SUBTYPE_FOCUS_CHANGED)
                && record.metadata.to_agent_id.as_deref() == Some("agent_1")
        }));
        let context = ledger::agent_context(&records, "agent_1");
        assert!(context.iter().any(|message| {
            message.role == "user" && message.content.contains("Handle this task")
        }));
    }

    #[tokio::test]
    async fn report_routes_to_target_context_and_projects_source_name() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::SUSPENDED,
            AgentPermissions {
                can_appoint: true,
                ..Default::default()
            },
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        let worker = runtime(
            "agent_1",
            "Worker",
            crate::state::states::EXECUTING,
            AgentPermissions {
                allowed_report_targets: vec![crate::agent::keys::BOSS_AGENT_ID.to_string()],
                ..Default::default()
            },
            &ledger,
        )
        .await;
        cluster.register_for_test(Arc::clone(&worker)).await;
        cluster
            .set_active_agent_id("agent_1".to_string())
            .await
            .unwrap();
        let gateway = Arc::new(crate::gateway::AgentGateway::new(
            Arc::clone(&cluster),
            Arc::clone(&ledger),
        ));
        gateway.install_routes().await.unwrap();

        let route = RouteAgentReportSystem
            .execute(
                RouteAgentReportInput {
                    cluster: Arc::clone(&cluster),
                    ledger: Arc::clone(&ledger),
                    payload: crate::events::AgentReportSubmittedPayload {
                        from_agent_id: "agent_1".to_string(),
                        from_agent_name: "Worker".to_string(),
                        target: crate::agent::keys::BOSS_AGENT_ID.to_string(),
                        report_type: "completed".to_string(),
                        report: "Finished the task".to_string(),
                        handoff: true,
                    },
                },
                &ledger.create_context(),
            )
            .await
            .unwrap();

        assert!(route.focus_changed);
        assert_eq!(
            cluster.active_agent_id().await,
            crate::agent::keys::BOSS_AGENT_ID
        );
        assert_eq!(
            cached_focus(&ledger).await,
            crate::agent::keys::BOSS_AGENT_ID
        );
        let records = ledger_records(&ledger).await;
        let report = records
            .iter()
            .find(|record| {
                record.agent_id == crate::agent::keys::BOSS_AGENT_ID
                    && record.role == LedgerRole::AgentReport
            })
            .expect("report ledger record");
        assert_eq!(report.metadata.from_agent_id.as_deref(), Some("agent_1"));
        assert_eq!(
            report.metadata.subtype.as_deref(),
            Some(ledger::GATEWAY_SUBTYPE_AGENT_REPORT)
        );
        let projected = report.to_frontend_message().unwrap();
        assert_eq!(projected.role, "agent_report");
        assert_eq!(projected.agent_name.as_deref(), Some("Worker"));
        let boss_context = ledger::agent_context(&records, crate::agent::keys::BOSS_AGENT_ID);
        assert!(boss_context.iter().any(|message| {
            message.role == "user" && message.content.contains("Finished the task")
        }));
        let pause_requested = worker
            .sm
            .unit()
            .cache()
            .get::<bool>(keys::PAUSE_REQUESTED)
            .await
            .unwrap()
            .unwrap_or(false);
        assert!(pause_requested);
    }

    #[tokio::test]
    async fn pause_event_pauses_payload_agent_not_current_focus() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::EXECUTING,
            AgentPermissions {
                can_appoint: true,
                ..Default::default()
            },
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        let worker = runtime(
            "agent_1",
            "Worker",
            crate::state::states::EXECUTING,
            AgentPermissions::default(),
            &ledger,
        )
        .await;
        cluster.register_for_test(Arc::clone(&worker)).await;
        assert_eq!(
            cluster.active_agent_id().await,
            crate::agent::keys::BOSS_AGENT_ID
        );
        let gateway = Arc::new(crate::gateway::AgentGateway::new(
            Arc::clone(&cluster),
            Arc::clone(&ledger),
        ));
        gateway.install_routes().await.unwrap();

        ledger
            .event_bus()
            .publish(corework::event::BaseEvent::new(
                crate::events::types::AGENT_PAUSE_REQUESTED,
                serde_json::to_value(crate::events::AgentPauseRequestedPayload {
                    agent_id: "agent_1".to_string(),
                    agent_name: "Worker".to_string(),
                })
                .unwrap(),
            ))
            .await
            .unwrap();

        let worker_pause = worker
            .sm
            .unit()
            .cache()
            .get::<bool>(keys::PAUSE_REQUESTED)
            .await
            .unwrap()
            .unwrap_or(false);
        let boss_pause = boss
            .sm
            .unit()
            .cache()
            .get::<bool>(keys::PAUSE_REQUESTED)
            .await
            .unwrap()
            .unwrap_or(false);
        assert!(worker_pause);
        assert!(!boss_pause);
        let records = ledger_records(&ledger).await;
        assert!(records.iter().any(|record| {
            record.agent_id == "agent_1"
                && record.role == LedgerRole::GatewayMessage
                && record.metadata.subtype.as_deref()
                    == Some(ledger::GATEWAY_SUBTYPE_PAUSE_REQUESTED)
        }));
        assert!(records.iter().any(|record| {
            record.agent_id == "agent_1"
                && record.role == LedgerRole::GatewayMessage
                && record.metadata.subtype.as_deref()
                    == Some(ledger::GATEWAY_SUBTYPE_AGENT_SUSPENDED)
                && record
                    .metadata
                    .extra
                    .get("state")
                    .and_then(|value| value.as_str())
                    == Some(crate::state::states::SUSPENDED)
        }));
    }

    #[tokio::test]
    async fn dismiss_records_agent_canceled_gateway_fact() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::SUSPENDED,
            AgentPermissions {
                can_dismiss: true,
                ..Default::default()
            },
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        let worker = runtime(
            "agent_1",
            "Worker",
            crate::state::states::SUSPENDED,
            AgentPermissions::default(),
            &ledger,
        )
        .await;
        cluster.register_for_test(worker).await;
        let gateway = Arc::new(crate::gateway::AgentGateway::new(
            Arc::clone(&cluster),
            Arc::clone(&ledger),
        ));
        gateway.install_routes().await.unwrap();

        cluster.dismiss("agent_1").await.unwrap();

        assert!(cluster.get("agent_1").await.is_none());
        let records = ledger_records(&ledger).await;
        assert!(records.iter().any(|record| {
            record.agent_id == "agent_1"
                && record.role == LedgerRole::GatewayMessage
                && record.metadata.subtype.as_deref()
                    == Some(ledger::GATEWAY_SUBTYPE_AGENT_CANCELED)
                && record
                    .metadata
                    .extra
                    .get("state")
                    .and_then(|value| value.as_str())
                    == Some("canceled")
        }));
    }

    #[tokio::test]
    async fn concurrent_focus_transitions_keep_runtime_cache_and_ledger_aligned() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::SUSPENDED,
            AgentPermissions::default(),
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        for (id, name) in [("agent_1", "Worker 1"), ("agent_2", "Worker 2")] {
            cluster
                .register_for_test(
                    runtime(
                        id,
                        name,
                        crate::state::states::SUSPENDED,
                        AgentPermissions::default(),
                        &ledger,
                    )
                    .await,
                )
                .await;
        }

        let first = {
            let cluster = Arc::clone(&cluster);
            let ledger = Arc::clone(&ledger);
            tokio::spawn(async move {
                record_focus_change_if_needed(&cluster, &ledger, "agent_1".into(), "concurrent")
                    .await
                    .unwrap();
            })
        };
        let second = {
            let cluster = Arc::clone(&cluster);
            let ledger = Arc::clone(&ledger);
            tokio::spawn(async move {
                record_focus_change_if_needed(&cluster, &ledger, "agent_2".into(), "concurrent")
                    .await
                    .unwrap();
            })
        };
        first.await.unwrap();
        second.await.unwrap();

        let runtime_focus = cluster.active_agent_id().await;
        assert_eq!(cached_focus(&ledger).await, runtime_focus);
        let last_recorded_focus = ledger_records(&ledger)
            .await
            .iter()
            .rev()
            .find(|record| {
                record.metadata.subtype.as_deref() == Some(ledger::GATEWAY_SUBTYPE_FOCUS_CHANGED)
            })
            .and_then(|record| record.metadata.to_agent_id.clone())
            .unwrap();
        assert_eq!(last_recorded_focus, runtime_focus);
    }

    #[tokio::test]
    async fn dismissing_focused_agent_transitions_to_default_consistently() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::SUSPENDED,
            AgentPermissions {
                can_dismiss: true,
                ..Default::default()
            },
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        cluster
            .register_for_test(
                runtime(
                    "agent_1",
                    "Worker",
                    crate::state::states::SUSPENDED,
                    AgentPermissions {
                        can_dismiss: true,
                        ..Default::default()
                    },
                    &ledger,
                )
                .await,
            )
            .await;
        record_focus_change_if_needed(&cluster, &ledger, "agent_1".into(), "test")
            .await
            .unwrap();

        let fallback = cluster.dismiss("agent_1").await.unwrap().unwrap();
        record_focus_change_if_needed(&cluster, &ledger, fallback, "dismiss")
            .await
            .unwrap();

        assert_eq!(
            cluster.active_agent_id().await,
            crate::agent::keys::BOSS_AGENT_ID
        );
        assert_eq!(
            cached_focus(&ledger).await,
            crate::agent::keys::BOSS_AGENT_ID
        );
    }

    #[tokio::test]
    async fn snapshot_updated_records_version_gateway_fact() {
        let _guard = crate::test_support::global_test_guard().await;
        let ledger = test_ledger().await;
        let boss = runtime(
            crate::agent::keys::BOSS_AGENT_ID,
            "Boss",
            crate::state::states::SUSPENDED,
            AgentPermissions::default(),
            &ledger,
        )
        .await;
        let cluster = Arc::new(AgentCluster::new(
            Arc::clone(&boss),
            ledger.event_bus().clone(),
        ));
        let gateway = Arc::new(crate::gateway::AgentGateway::new(
            Arc::clone(&cluster),
            Arc::clone(&ledger),
        ));
        gateway.install_routes().await.unwrap();

        ledger
            .event_bus()
            .publish(corework::event::BaseEvent::new(
                crate::events::types::SNAPSHOT_UPDATED,
                serde_json::to_value(crate::events::SnapshotUpdatedPayload {
                    snapshot_text: "large draft text should stay event-only".to_string(),
                    version: 7,
                })
                .unwrap(),
            ))
            .await
            .unwrap();

        let records = ledger_records(&ledger).await;
        let fact = records
            .iter()
            .find(|record| {
                record.role == LedgerRole::GatewayMessage
                    && record.metadata.subtype.as_deref()
                        == Some(ledger::GATEWAY_SUBTYPE_SNAPSHOT_UPDATED)
            })
            .expect("snapshot_updated gateway fact");
        assert_eq!(
            fact.metadata
                .extra
                .get("version")
                .and_then(|value| value.as_u64()),
            Some(7)
        );
        assert!(!fact.content.contains("large draft text"));
        assert!(!fact.metadata.extra.contains_key("snapshot_text"));
    }
}
