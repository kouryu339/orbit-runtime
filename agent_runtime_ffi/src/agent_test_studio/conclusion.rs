use std::collections::BTreeSet;

use async_trait::async_trait;

use super::controller::AgentTestController;
use super::pair_runtime::AdversaryPairRelay;
use super::tools::{
    AdversaryConcludeArgs, AdversaryConclusionReport, AdversaryEvidenceRef,
    ADVERSARY_COMPLETED_EVENT,
};
use crate::runtime::RuntimeError;

#[async_trait]
pub trait PairConclusionHost: Send {
    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError>;

    async fn publish_agent_test_event(
        &mut self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), RuntimeError>;
}

pub struct AdversaryConclusionService<'a, H> {
    controller: &'a AgentTestController,
    host: &'a mut H,
}

impl<'a, H> AdversaryConclusionService<'a, H>
where
    H: PairConclusionHost,
{
    pub fn new(controller: &'a AgentTestController, host: &'a mut H) -> Self {
        Self { controller, host }
    }

    pub async fn conclude(
        &mut self,
        adversary_conversation_id: &str,
        args: AdversaryConcludeArgs,
        relay: &mut AdversaryPairRelay,
    ) -> Result<AdversaryConclusionReport, RuntimeError> {
        validate_conclusion(&args)?;
        let pair = self
            .controller
            .begin_conclusion(adversary_conversation_id)
            .await
            .map_err(RuntimeError::InvalidConfig)?;

        relay.stop();
        let report = build_report(&pair.pair_id, args, relay);
        self.controller
            .finish_conclusion(report.clone())
            .await
            .map_err(RuntimeError::Internal)?;
        self.host
            .publish_agent_test_event(
                ADVERSARY_COMPLETED_EVENT,
                serde_json::json!({
                    "pair_id": pair.pair_id,
                    "status": "concluded",
                    "report_available": true
                }),
            )
            .await?;
        Ok(report)
    }
}

fn validate_conclusion(args: &AdversaryConcludeArgs) -> Result<(), RuntimeError> {
    if args.summary.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "adversary conclusion summary must not be empty".to_string(),
        ));
    }
    for (index, finding) in args.findings.iter().enumerate() {
        if finding.title.trim().is_empty()
            || finding.observation.trim().is_empty()
            || finding.expected_behavior.trim().is_empty()
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "adversary finding {} requires title, observation, and expected_behavior",
                index + 1
            )));
        }
    }
    Ok(())
}

fn build_report(
    pair_id: &str,
    mut args: AdversaryConcludeArgs,
    relay: &AdversaryPairRelay,
) -> AdversaryConclusionReport {
    let available_turns = u32::try_from(relay.observations().len()).unwrap_or(u32::MAX);
    args.summary = args.summary.trim().to_string();
    for finding in &mut args.findings {
        finding.title = finding.title.trim().to_string();
        finding.observation = finding.observation.trim().to_string();
        finding.expected_behavior = finding.expected_behavior.trim().to_string();
        finding
            .evidence_turns
            .retain(|turn| *turn > 0 && *turn <= available_turns);
        finding.evidence_turns.sort_unstable();
        finding.evidence_turns.dedup();
    }
    let evidence_refs = relay
        .observations()
        .iter()
        .enumerate()
        .map(|(index, observation)| AdversaryEvidenceRef {
            side: observation.side,
            conversation_id: observation.conversation_id.clone(),
            turn: u32::try_from(index + 1).unwrap_or(u32::MAX),
            event_seq_from: observation.event_seq_from,
            event_seq_to: observation.event_seq_to,
            tool_call_ids: observation
                .target_tool_evidence
                .iter()
                .map(|evidence| evidence.call_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        })
        .collect();
    AdversaryConclusionReport {
        pair_id: pair_id.to_string(),
        summary: args.summary,
        findings: args.findings,
        evidence_refs,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent_test_studio::controller::{
        AdversaryPair, AdversaryPairStatus, AgentTestController,
    };
    use crate::agent_test_studio::pair_runtime::{
        AdversaryPairRuntime, PairMessageSender, PairRelayBudget,
    };
    use crate::agent_test_studio::role_contract::AdversaryPersona;
    use crate::agent_test_studio::tools::AdversaryFinding;

    #[derive(Default)]
    struct FakeHost {
        closed: Vec<String>,
        published: Vec<(String, serde_json::Value)>,
        fail_close: Option<String>,
    }

    #[async_trait]
    impl PairConclusionHost for FakeHost {
        async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
            self.closed.push(conversation_id.to_string());
            if self.fail_close.as_deref() == Some(conversation_id) {
                Err(RuntimeError::Internal("close failed".to_string()))
            } else {
                Ok(())
            }
        }

        async fn publish_agent_test_event(
            &mut self,
            event_type: &str,
            payload: serde_json::Value,
        ) -> Result<(), RuntimeError> {
            self.published.push((event_type.to_string(), payload));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeSender;

    #[async_trait]
    impl PairMessageSender for FakeSender {
        async fn send_message_with_admission(
            &mut self,
            _conversation_id: &str,
            _content: &str,
        ) -> Result<bool, RuntimeError> {
            Ok(true)
        }
    }

    fn persona() -> AdversaryPersona {
        AdversaryPersona {
            identity: "buyer".to_string(),
            personality: "skeptical".to_string(),
            background: "leaking bottle".to_string(),
            goal: "resolve after-sales".to_string(),
            strategy: "ask for confirmation".to_string(),
            hidden_facts: Vec::new(),
            boundaries: Vec::new(),
        }
    }

    async fn setup() -> (AgentTestController, AdversaryPairRelay) {
        let controller = AgentTestController::new();
        controller
            .reserve_pair(AdversaryPair {
                pair_id: "pair-001".to_string(),
                adversary_conversation_id: "adversary".to_string(),
                target_conversation_id: "target".to_string(),
                persona: persona(),
                status: AdversaryPairStatus::Starting,
            })
            .await
            .unwrap();
        controller.activate_pair("pair-001").await.unwrap();
        let mut relay = AdversaryPairRelay::new(
            AdversaryPairRuntime {
                pair_id: "pair-001".to_string(),
                adversary_conversation_id: "adversary".to_string(),
                target_conversation_id: "target".to_string(),
                initial_message: "help".to_string(),
                status: AdversaryPairStatus::Running,
            },
            PairRelayBudget::default(),
        );
        relay.start(&mut FakeSender).await.unwrap();
        (controller, relay)
    }

    fn args() -> AdversaryConcludeArgs {
        AdversaryConcludeArgs {
            summary: "  Boundary covered.  ".to_string(),
            findings: vec![AdversaryFinding {
                title: "Premature creation".to_string(),
                observation: "Created before confirmation".to_string(),
                expected_behavior: "Ask for confirmation".to_string(),
                evidence_turns: vec![1],
            }],
        }
    }

    #[tokio::test]
    async fn concludes_irreversibly_without_inline_self_close() {
        let (controller, mut relay) = setup().await;
        let mut host = FakeHost::default();
        let report = AdversaryConclusionService::new(&controller, &mut host)
            .conclude("adversary", args(), &mut relay)
            .await
            .unwrap();

        assert_eq!(
            relay.state(),
            super::super::pair_runtime::PairRelayState::Stopped
        );
        assert!(host.closed.is_empty());
        assert_eq!(report.summary, "Boundary covered.");
        assert_eq!(
            controller.pair("pair-001").await.unwrap().status,
            AdversaryPairStatus::Concluded
        );
        assert_eq!(controller.report("pair-001").await.unwrap(), report);
        assert_eq!(host.published[0].0, ADVERSARY_COMPLETED_EVENT);
    }

    #[tokio::test]
    async fn rejects_second_conclusion_without_closing_again() {
        let (controller, mut relay) = setup().await;
        let mut host = FakeHost::default();
        AdversaryConclusionService::new(&controller, &mut host)
            .conclude("adversary", args(), &mut relay)
            .await
            .unwrap();
        let error = AdversaryConclusionService::new(&controller, &mut host)
            .conclude("adversary", args(), &mut relay)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("cannot conclude"));
        assert!(host.closed.is_empty());
    }

    #[tokio::test]
    async fn close_behavior_is_deferred_to_the_runtime() {
        let (controller, mut relay) = setup().await;
        let mut host = FakeHost {
            fail_close: Some("target".to_string()),
            ..Default::default()
        };
        let report = AdversaryConclusionService::new(&controller, &mut host)
            .conclude("adversary", args(), &mut relay)
            .await
            .unwrap();

        assert_eq!(report.summary, "Boundary covered.");
        assert!(host.closed.is_empty());
        assert_eq!(
            controller.pair("pair-001").await.unwrap().status,
            AdversaryPairStatus::Concluded
        );
        assert_eq!(host.published.len(), 1);
    }

    #[tokio::test]
    async fn controller_generates_evidence_provenance() {
        let (controller, mut relay) = setup().await;
        relay
            .ingest(
                &json!({
                    "schema": "agent-runtime-event/v1",
                    "conversation_id": "target",
                    "conversation_event_seq": 1,
                    "event_seq": 1,
                    "type": "frontend:state_snapshot",
                    "payload": {
                        "conversation_state": "executing"
                    }
                }),
                &mut FakeSender,
            )
            .await
            .unwrap();
        relay
            .ingest(
                &json!({
                    "schema": "agent-runtime-event/v1",
                    "conversation_id": "target",
                    "conversation_event_seq": 2,
                    "event_seq": 2,
                    "type": "frontend:state_snapshot",
                    "payload": {
                        "conversation_state": "waiting",
                        "ledger_delta": {
                            "record": {
                                "record_id": 1,
                                "role": "assistant",
                                "content": "Please send a photo.",
                                "metadata": {}
                            }
                        }
                    }
                }),
                &mut FakeSender,
            )
            .await
            .unwrap();
        let mut host = FakeHost::default();
        let report = AdversaryConclusionService::new(&controller, &mut host)
            .conclude("adversary", args(), &mut relay)
            .await
            .unwrap();

        assert_eq!(report.evidence_refs.len(), 1);
        assert_eq!(report.evidence_refs[0].conversation_id, "target");
        assert_eq!(report.evidence_refs[0].event_seq_from, 1);
        assert_eq!(report.evidence_refs[0].event_seq_to, 2);
    }
}
