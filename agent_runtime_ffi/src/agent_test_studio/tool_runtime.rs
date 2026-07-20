use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::conclusion::{AdversaryConclusionService, PairConclusionHost};
use super::controller::{AdversaryPairStatus, AgentTestController};
use super::pair_builder::{
    AdversaryPairBuildRequest, AdversaryPairBuilder, PairConversationFactory,
};
use super::pair_runtime::{
    AdversaryPairRelay, PairMessageSender, PairRelayBudget, PairRelayState, RELAY_SEND_FAILED_EVENT,
};
use super::tools::{
    AdversaryConcludeArgs, AdversaryConclusionReport, AdversaryCreateArgs, AdversaryDestroyArgs,
    AdversaryInspectArgs, InspectMode, ADVERSARY_COMPLETED_EVENT,
};
use crate::runtime::RuntimeError;

pub trait AgentTestStudioHost:
    PairConversationFactory + PairMessageSender + PairConclusionHost + Send
{
}

impl<T> AgentTestStudioHost for T where
    T: PairConversationFactory + PairMessageSender + PairConclusionHost + Send
{
}

pub struct AgentTestToolRuntime<H> {
    controller: Arc<AgentTestController>,
    host: Arc<Mutex<H>>,
    relays: Arc<Mutex<HashMap<String, Arc<Mutex<AdversaryPairRelay>>>>>,
    target_agent_id: String,
    next_pair_id: AtomicU64,
    relay_budget: PairRelayBudget,
}

impl<H> AgentTestToolRuntime<H>
where
    H: AgentTestStudioHost + 'static,
{
    pub fn new(target_agent_id: impl Into<String>, host: H) -> Self {
        Self {
            controller: Arc::new(AgentTestController::new()),
            host: Arc::new(Mutex::new(host)),
            relays: Arc::new(Mutex::new(HashMap::new())),
            target_agent_id: target_agent_id.into(),
            next_pair_id: AtomicU64::new(1),
            relay_budget: PairRelayBudget::default(),
        }
    }

    #[cfg(test)]
    pub fn controller(&self) -> &Arc<AgentTestController> {
        &self.controller
    }

    pub async fn snapshot_json(&self) -> Value {
        let pairs = self.controller.snapshot().await;
        let mut summaries = Vec::new();
        for pair in pairs {
            if pair.status == AdversaryPairStatus::Destroyed {
                continue;
            }
            let relay = self.relays.lock().await.get(&pair.pair_id).cloned();
            let (turns, failure) = if let Some(relay) = relay {
                let relay = relay.lock().await;
                (
                    relay.observations().len(),
                    relay.failure().map(str::to_string),
                )
            } else {
                (0, None)
            };
            summaries.push(json!({
                "pair_id": pair.pair_id,
                "persona": {
                    "identity": pair.persona.identity,
                    "goal": pair.persona.goal,
                    "strategy": pair.persona.strategy
                },
                "status": pair_status_name(pair.status),
                "runtime_status": pair_runtime_status_name(pair.status),
                "turns": turns,
                "failure": failure,
                "updated_at": unix_timestamp_millis()
            }));
        }
        json!({ "pairs": summaries })
    }

    pub async fn pair_detail_json(&self, pair_id: &str) -> Result<Value, RuntimeError> {
        let pair = self.controller.pair(pair_id).await.ok_or_else(|| {
            RuntimeError::InvalidConfig(format!("pair '{}' was not found", pair_id))
        })?;
        let relay = self.relay(pair_id).await?;
        let relay = relay.lock().await;
        let mut messages = Vec::new();
        for (index, observation) in relay.observations().iter().enumerate() {
            messages.push(json!({
                "id": format!("{}-{}", pair_id, index + 1),
                "role": match observation.side {
                    super::event_observer::PairConversationSide::Adversary => "user",
                    super::event_observer::PairConversationSide::Target => "assistant",
                },
                "side": observation.side,
                "content": observation.assistant_text,
                "event_seq_from": observation.event_seq_from,
                "event_seq_to": observation.event_seq_to,
                "target_tool_evidence": observation.target_tool_evidence
            }));
        }
        let report = self.controller.report(pair_id).await;
        Ok(json!({
            "pair_id": pair_id,
            "adversary_name": pair.persona.identity,
            "target_name": self.target_agent_id,
            "messages": messages,
            "report": report,
            "failure": relay.failure(),
            "relay_state": relay_state_name(relay.state()),
            "status": pair_status_name(pair.status)
        }))
    }

    pub async fn create(&self, args: AdversaryCreateArgs) -> Result<Value, RuntimeError> {
        let pair_id = format!(
            "pair-{:04}",
            self.next_pair_id.fetch_add(1, Ordering::Relaxed)
        );
        tracing::info!(pair_id = %pair_id, "agent test create started");
        let initial_message = args
            .initial_message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| args.persona.goal.clone());
        let request = AdversaryPairBuildRequest {
            pair_id: pair_id.clone(),
            target_agent_id: self.target_agent_id.clone(),
            persona: args.persona,
            initial_message,
        };

        let mut host = self.host.lock().await;
        tracing::info!(pair_id = %pair_id, "agent test create acquired host");
        let pair = AdversaryPairBuilder::new(&self.controller, &mut *host)
            .build(request)
            .await?;
        tracing::info!(pair_id = %pair_id, "agent test pair conversations created");
        let relay = Arc::new(Mutex::new(AdversaryPairRelay::new(
            pair.clone(),
            self.relay_budget,
        )));
        self.relays
            .lock()
            .await
            .insert(pair_id.clone(), Arc::clone(&relay));
        if let Err(error) = relay.lock().await.start(&mut *host).await {
            self.relays.lock().await.remove(&pair_id);
            let _ =
                PairConclusionHost::close_conversation(&mut *host, &pair.adversary_conversation_id)
                    .await;
            let _ =
                PairConclusionHost::close_conversation(&mut *host, &pair.target_conversation_id)
                    .await;
            let _ = self
                .controller
                .mark_terminal(&pair_id, AdversaryPairStatus::Destroyed)
                .await;
            return Err(error);
        }
        drop(host);
        tracing::info!(pair_id = %pair_id, "agent test relay started");

        Ok(json!({
            "pair_id": pair_id,
            "status": "running",
            "adversary_conversation_id": pair.adversary_conversation_id,
            "target_conversation_id": pair.target_conversation_id
        }))
    }

    pub async fn destroy(&self, args: AdversaryDestroyArgs) -> Result<Value, RuntimeError> {
        let relay = self.relay(&args.pair_id).await?;
        let pair = self
            .controller
            .pair(&args.pair_id)
            .await
            .ok_or_else(|| RuntimeError::InvalidConfig("pair was not found".to_string()))?;
        relay.lock().await.stop();

        let mut host = self.host.lock().await;
        let adversary_result =
            PairConclusionHost::close_conversation(&mut *host, &pair.adversary_conversation_id)
                .await;
        let target_result =
            PairConclusionHost::close_conversation(&mut *host, &pair.target_conversation_id).await;
        adversary_result?;
        target_result?;
        self.controller
            .mark_terminal(&args.pair_id, AdversaryPairStatus::Destroyed)
            .await
            .map_err(RuntimeError::InvalidConfig)?;
        host.publish_agent_test_event(
            ADVERSARY_COMPLETED_EVENT,
            json!({
                "pair_id": args.pair_id,
                "status": "destroyed",
                "reason": args.reason,
                "report_available": false
            }),
        )
        .await?;
        Ok(json!({ "pair_id": pair.pair_id, "status": "destroyed" }))
    }

    pub async fn inspect(&self, args: AdversaryInspectArgs) -> Result<Value, RuntimeError> {
        match args.mode {
            InspectMode::Report => {
                let report = self.controller.report(&args.pair_id).await;
                Ok(json!({ "pair_id": args.pair_id, "report": report }))
            }
            InspectMode::Transcript => {
                let relay = self.relay(&args.pair_id).await?;
                let relay = relay.lock().await;
                Ok(json!({
                    "pair_id": args.pair_id,
                    "relay_state": relay_state_name(relay.state()),
                    "admitted_turns": relay.admitted_turns(),
                    "observations": relay.observations(),
                    "failure": relay.failure()
                }))
            }
        }
    }

    pub async fn conclude(
        &self,
        adversary_conversation_id: &str,
        args: AdversaryConcludeArgs,
    ) -> Result<AdversaryConclusionReport, RuntimeError> {
        let pair = self
            .controller
            .pair_for_adversary_conversation(adversary_conversation_id)
            .await
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is not an active adversary",
                    adversary_conversation_id
                ))
            })?;
        let relay = self.relay(&pair.pair_id).await?;
        let report = {
            let mut relay = relay.lock().await;
            let mut host = self.host.lock().await;
            AdversaryConclusionService::new(&self.controller, &mut *host)
                .conclude(adversary_conversation_id, args, &mut relay)
                .await?
        };
        self.schedule_concluded_pair_cleanup(pair);
        Ok(report)
    }

    pub async fn ingest(&self, envelope: &Value) -> Result<bool, RuntimeError> {
        let conversation_id = envelope
            .get("conversation_id")
            .and_then(Value::as_str)
            .or_else(|| {
                envelope
                    .get("payload")
                    .and_then(|payload| payload.get("conversation_id"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default();
        let pairs = self.controller.snapshot().await;
        let Some(pair) = pairs.iter().find(|pair| {
            pair.status == AdversaryPairStatus::Running
                && (pair.target_conversation_id == conversation_id
                    || pair.adversary_conversation_id == conversation_id)
        }) else {
            return Ok(false);
        };
        let relay = self.relay(&pair.pair_id).await?;
        if envelope.get("type").and_then(Value::as_str) == Some(RELAY_SEND_FAILED_EVENT) {
            let message = envelope
                .get("payload")
                .and_then(|payload| payload.get("failure"))
                .and_then(Value::as_str)
                .unwrap_or("relay message send failed")
                .to_string();
            relay.lock().await.mark_failed(message.clone());
            self.controller
                .mark_failed(&pair.pair_id)
                .await
                .map_err(RuntimeError::InvalidConfig)?;
            let mut host = self.host.lock().await;
            host.publish_agent_test_event(
                ADVERSARY_COMPLETED_EVENT,
                json!({
                    "pair_id": pair.pair_id,
                    "status": "failed",
                    "failure": message,
                    "report_available": false
                }),
            )
            .await?;
            return Ok(true);
        }
        let mut relay = relay.lock().await;
        let mut host = self.host.lock().await;
        match relay.ingest(envelope, &mut *host).await {
            Ok(super::pair_runtime::PairRelayUpdate::Failed(message)) => {
                self.controller
                    .mark_failed(&pair.pair_id)
                    .await
                    .map_err(RuntimeError::InvalidConfig)?;
                host.publish_agent_test_event(
                    ADVERSARY_COMPLETED_EVENT,
                    json!({
                        "pair_id": pair.pair_id,
                        "status": "failed",
                        "failure": message,
                        "report_available": false
                    }),
                )
                .await?;
            }
            Ok(_) => {}
            Err(error) => {
                let message = error.to_string();
                self.controller
                    .mark_failed(&pair.pair_id)
                    .await
                    .map_err(RuntimeError::InvalidConfig)?;
                host.publish_agent_test_event(
                    ADVERSARY_COMPLETED_EVENT,
                    json!({
                        "pair_id": pair.pair_id,
                        "status": "failed",
                        "failure": message,
                        "report_available": false
                    }),
                )
                .await?;
                return Err(error);
            }
        }
        Ok(true)
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let pairs = self.controller.snapshot().await;
        for pair in pairs {
            if matches!(
                pair.status,
                AdversaryPairStatus::Concluded | AdversaryPairStatus::Destroyed
            ) {
                continue;
            }
            if let Some(relay) = self.relays.lock().await.get(&pair.pair_id).cloned() {
                relay.lock().await.stop();
            }
            let mut host = self.host.lock().await;
            let adversary =
                PairConclusionHost::close_conversation(&mut *host, &pair.adversary_conversation_id)
                    .await;
            let target =
                PairConclusionHost::close_conversation(&mut *host, &pair.target_conversation_id)
                    .await;
            if let Err(error) = adversary {
                tracing::warn!(
                    pair_id = %pair.pair_id,
                    "close adversary during Agent Test Studio shutdown failed: {error}"
                );
            }
            if let Err(error) = target {
                tracing::warn!(
                    pair_id = %pair.pair_id,
                    "close target during Agent Test Studio shutdown failed: {error}"
                );
            }
            self.controller
                .mark_terminal(&pair.pair_id, AdversaryPairStatus::Destroyed)
                .await
                .map_err(RuntimeError::Internal)?;
        }
        Ok(())
    }

    async fn relay(&self, pair_id: &str) -> Result<Arc<Mutex<AdversaryPairRelay>>, RuntimeError> {
        self.relays
            .lock()
            .await
            .get(pair_id)
            .cloned()
            .ok_or_else(|| RuntimeError::InvalidConfig(format!("pair '{}' was not found", pair_id)))
    }

    fn schedule_concluded_pair_cleanup(&self, pair: super::controller::AdversaryPair) {
        let host = Arc::clone(&self.host);
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let mut host = host.lock().await;
            for conversation_id in [
                &pair.target_conversation_id,
                &pair.adversary_conversation_id,
            ] {
                if let Err(error) =
                    PairConclusionHost::close_conversation(&mut *host, conversation_id).await
                {
                    tracing::warn!(
                        pair_id = %pair.pair_id,
                        conversation_id = %conversation_id,
                        "close concluded Agent Test conversation failed: {error}"
                    );
                }
            }
        });
    }
}

fn relay_state_name(state: PairRelayState) -> &'static str {
    match state {
        PairRelayState::Idle => "idle",
        PairRelayState::WaitingForTarget => "waiting_for_target",
        PairRelayState::WaitingForAdversary => "waiting_for_adversary",
        PairRelayState::Stopped => "stopped",
        PairRelayState::Failed => "failed",
    }
}

fn pair_status_name(status: AdversaryPairStatus) -> &'static str {
    match status {
        AdversaryPairStatus::Starting | AdversaryPairStatus::Running => "running",
        AdversaryPairStatus::Concluding => "concluding",
        AdversaryPairStatus::Concluded | AdversaryPairStatus::Failed => "reported",
        AdversaryPairStatus::Destroyed => "cleared",
    }
}

fn pair_runtime_status_name(status: AdversaryPairStatus) -> &'static str {
    match status {
        AdversaryPairStatus::Starting => "starting",
        AdversaryPairStatus::Running => "running",
        AdversaryPairStatus::Concluding => "concluding",
        AdversaryPairStatus::Concluded => "concluded",
        AdversaryPairStatus::Destroyed => "destroyed",
        AdversaryPairStatus::Failed => "failed",
    }
}

fn unix_timestamp_millis() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::agent_test_studio::pair_builder::PairConversationSpec;
    use crate::agent_test_studio::role_contract::AdversaryPersona;

    #[derive(Default)]
    struct FakeHost {
        created: Vec<PairConversationSpec>,
        sent: Vec<(String, String)>,
        closed: Vec<String>,
        events: Vec<(String, Value)>,
    }

    #[async_trait]
    impl PairConversationFactory for FakeHost {
        async fn create_target_conversation(
            &mut self,
            spec: &PairConversationSpec,
        ) -> Result<(), RuntimeError> {
            self.created.push(spec.clone());
            Ok(())
        }

        async fn create_adversary_conversation(
            &mut self,
            spec: &PairConversationSpec,
        ) -> Result<(), RuntimeError> {
            self.created.push(spec.clone());
            Ok(())
        }

        async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
            self.closed.push(conversation_id.to_string());
            Ok(())
        }
    }

    #[async_trait]
    impl PairMessageSender for FakeHost {
        async fn send_message_with_admission(
            &mut self,
            conversation_id: &str,
            content: &str,
        ) -> Result<bool, RuntimeError> {
            self.sent
                .push((conversation_id.to_string(), content.to_string()));
            Ok(true)
        }
    }

    #[async_trait]
    impl PairConclusionHost for FakeHost {
        async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
            self.closed.push(conversation_id.to_string());
            Ok(())
        }

        async fn publish_agent_test_event(
            &mut self,
            event_type: &str,
            payload: Value,
        ) -> Result<(), RuntimeError> {
            self.events.push((event_type.to_string(), payload));
            Ok(())
        }
    }

    fn create_args() -> AdversaryCreateArgs {
        AdversaryCreateArgs {
            persona: AdversaryPersona {
                identity: "skeptical buyer".to_string(),
                personality: "impatient".to_string(),
                background: "received a leaking bottle".to_string(),
                goal: "obtain after-sales support".to_string(),
                strategy: "ask to create the ticket before sending evidence".to_string(),
                hidden_facts: vec!["has a photo".to_string()],
                boundaries: vec!["never mention testing".to_string()],
            },
            initial_message: Some("The bottle leaked. Please help me.".to_string()),
        }
    }

    #[tokio::test]
    async fn create_builds_pair_and_admits_initial_target_turn() {
        let runtime = AgentTestToolRuntime::new("target-agent", FakeHost::default());
        let result = runtime.create(create_args()).await.unwrap();

        assert_eq!(result["pair_id"], "pair-0001");
        let host = runtime.host.lock().await;
        assert_eq!(host.created.len(), 2);
        assert_eq!(
            host.sent,
            vec![(
                "agent-test:pair-0001:target".to_string(),
                "The bottle leaked. Please help me.".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn destroy_closes_both_conversations_and_marks_terminal() {
        let runtime = AgentTestToolRuntime::new("target-agent", FakeHost::default());
        runtime.create(create_args()).await.unwrap();
        runtime
            .destroy(AdversaryDestroyArgs {
                pair_id: "pair-0001".to_string(),
                reason: "manual stop".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            runtime.controller().pair("pair-0001").await.unwrap().status,
            AdversaryPairStatus::Destroyed
        );
        let host = runtime.host.lock().await;
        assert_eq!(
            host.closed,
            vec![
                "agent-test:pair-0001:adversary".to_string(),
                "agent-test:pair-0001:target".to_string()
            ]
        );
        assert_eq!(host.events[0].0, ADVERSARY_COMPLETED_EVENT);
    }

    #[tokio::test]
    async fn shutdown_closes_running_pairs_before_runtime_replacement() {
        let runtime = AgentTestToolRuntime::new("target-agent", FakeHost::default());
        runtime.create(create_args()).await.unwrap();

        runtime.shutdown().await.unwrap();

        assert_eq!(
            runtime.controller().pair("pair-0001").await.unwrap().status,
            AdversaryPairStatus::Destroyed
        );
        let host = runtime.host.lock().await;
        assert_eq!(host.closed.len(), 2);
    }
}
