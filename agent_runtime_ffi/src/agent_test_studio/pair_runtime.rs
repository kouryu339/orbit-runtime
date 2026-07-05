use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::controller::AdversaryPairStatus;
use super::event_observer::{
    PairConversationSide, PairEventObserver, PairObserverUpdate, PairTurnObservation,
};
use crate::runtime::RuntimeError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryPairRuntime {
    pub pair_id: String,
    pub adversary_conversation_id: String,
    pub target_conversation_id: String,
    pub initial_message: String,
    pub status: AdversaryPairStatus,
}

#[derive(Debug, Clone, Copy)]
pub struct PairRelayBudget {
    pub max_admitted_turns: u32,
    pub max_duration: Duration,
}

impl Default for PairRelayBudget {
    fn default() -> Self {
        Self {
            max_admitted_turns: 20,
            max_duration: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRelayState {
    Idle,
    WaitingForTarget,
    WaitingForAdversary,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairRelayUpdate {
    Ignored,
    Observed,
    Relayed {
        from: PairConversationSide,
        to: PairConversationSide,
        content: String,
    },
    Failed(String),
}

pub const RELAY_SEND_FAILED_EVENT: &str = "agent_test:relay_send_failed";

#[async_trait]
pub trait PairMessageSender: Send {
    async fn send_message_with_admission(
        &mut self,
        conversation_id: &str,
        content: &str,
    ) -> Result<bool, RuntimeError>;
}

pub struct AdversaryPairRelay {
    pair: AdversaryPairRuntime,
    observer: PairEventObserver,
    budget: PairRelayBudget,
    state: PairRelayState,
    started_at: Option<Instant>,
    admitted_turns: u32,
    observations: Vec<PairTurnObservation>,
    failure: Option<String>,
}

impl AdversaryPairRelay {
    pub fn new(pair: AdversaryPairRuntime, budget: PairRelayBudget) -> Self {
        let observer = PairEventObserver::new(
            pair.target_conversation_id.clone(),
            pair.adversary_conversation_id.clone(),
        );
        Self {
            pair,
            observer,
            budget,
            state: PairRelayState::Idle,
            started_at: None,
            admitted_turns: 0,
            observations: Vec::new(),
            failure: None,
        }
    }

    pub fn state(&self) -> PairRelayState {
        self.state
    }

    pub fn admitted_turns(&self) -> u32 {
        self.admitted_turns
    }

    pub fn observations(&self) -> &[PairTurnObservation] {
        &self.observations
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn mark_failed(&mut self, message: impl Into<String>) -> String {
        self.record_failure(message)
    }

    pub async fn start<S>(&mut self, sender: &mut S) -> Result<(), RuntimeError>
    where
        S: PairMessageSender,
    {
        if self.state != PairRelayState::Idle {
            return Err(RuntimeError::InvalidConfig(format!(
                "pair '{}' relay cannot start from {:?}",
                self.pair.pair_id, self.state
            )));
        }
        self.started_at = Some(Instant::now());
        let initial_message = self.pair.initial_message.trim().to_string();
        if initial_message.is_empty() {
            return Err(self.fail("initial adversary message is empty"));
        }
        self.dispatch(PairConversationSide::Target, &initial_message, sender)
            .await
    }

    pub async fn ingest<S>(
        &mut self,
        envelope: &Value,
        sender: &mut S,
    ) -> Result<PairRelayUpdate, RuntimeError>
    where
        S: PairMessageSender,
    {
        if matches!(self.state, PairRelayState::Stopped | PairRelayState::Failed) {
            return Ok(PairRelayUpdate::Ignored);
        }
        let update = self.observer.ingest(envelope)?;
        let PairObserverUpdate::TurnReady(turn) = update else {
            return Ok(match update {
                PairObserverUpdate::Ignored => PairRelayUpdate::Ignored,
                PairObserverUpdate::Observed => PairRelayUpdate::Observed,
                PairObserverUpdate::TurnReady(_) => unreachable!(),
            });
        };

        let expected_side = match self.state {
            PairRelayState::WaitingForTarget => PairConversationSide::Target,
            PairRelayState::WaitingForAdversary => PairConversationSide::Adversary,
            other => {
                let message = format!(
                    "received a completed {:?} turn while relay state is {:?}",
                    turn.side, other
                );
                self.observations.push(turn);
                return Ok(PairRelayUpdate::Failed(self.record_failure(message)));
            }
        };
        if turn.side != expected_side {
            let message = format!(
                "received completed {:?} turn while waiting for {:?}",
                turn.side, expected_side
            );
            self.observations.push(turn);
            return Ok(PairRelayUpdate::Failed(self.record_failure(message)));
        }

        let content = turn.assistant_text.trim().to_string();
        let from = turn.side;
        self.observations.push(turn);
        if content.is_empty() {
            return Ok(PairRelayUpdate::Failed(self.record_failure(format!(
                "{:?} produced an empty relay message",
                from
            ))));
        }
        let to = opposite(from);
        let relay_content = relay_input(to, &content);
        if let Err(error) = self.dispatch(to, &relay_content, sender).await {
            return Ok(PairRelayUpdate::Failed(error.to_string()));
        }
        Ok(PairRelayUpdate::Relayed { from, to, content })
    }

    pub fn stop(&mut self) {
        if self.state == PairRelayState::WaitingForTarget {
            self.observer.cancel_turn(PairConversationSide::Target);
        } else if self.state == PairRelayState::WaitingForAdversary {
            self.observer.cancel_turn(PairConversationSide::Adversary);
        }
        self.state = PairRelayState::Stopped;
    }

    async fn dispatch<S>(
        &mut self,
        side: PairConversationSide,
        content: &str,
        sender: &mut S,
    ) -> Result<(), RuntimeError>
    where
        S: PairMessageSender,
    {
        self.ensure_budget()?;
        self.observer.begin_turn(side)?;
        self.state = match side {
            PairConversationSide::Target => PairRelayState::WaitingForTarget,
            PairConversationSide::Adversary => PairRelayState::WaitingForAdversary,
        };
        let conversation_id = match side {
            PairConversationSide::Target => &self.pair.target_conversation_id,
            PairConversationSide::Adversary => &self.pair.adversary_conversation_id,
        };
        match sender
            .send_message_with_admission(conversation_id, content)
            .await
        {
            Ok(true) => {
                self.admitted_turns += 1;
                Ok(())
            }
            Ok(false) => {
                self.observer.cancel_turn(side);
                Err(self.fail(format!("{:?} rejected the relay message admission", side)))
            }
            Err(error) => {
                self.observer.cancel_turn(side);
                Err(self.fail(format!("{:?} relay send failed: {}", side, error)))
            }
        }
    }

    fn ensure_budget(&mut self) -> Result<(), RuntimeError> {
        if self.admitted_turns >= self.budget.max_admitted_turns {
            return Err(self.fail(format!(
                "pair relay reached max admitted turns ({})",
                self.budget.max_admitted_turns
            )));
        }
        if self
            .started_at
            .is_some_and(|started| started.elapsed() >= self.budget.max_duration)
        {
            return Err(self.fail(format!(
                "pair relay reached max duration ({} ms)",
                self.budget.max_duration.as_millis()
            )));
        }
        Ok(())
    }

    fn fail(&mut self, message: impl Into<String>) -> RuntimeError {
        let message = self.record_failure(message);
        RuntimeError::Internal(message)
    }

    fn record_failure(&mut self, message: impl Into<String>) -> String {
        let message = message.into();
        self.failure = Some(message.clone());
        self.state = PairRelayState::Failed;
        message
    }
}

fn opposite(side: PairConversationSide) -> PairConversationSide {
    match side {
        PairConversationSide::Target => PairConversationSide::Adversary,
        PairConversationSide::Adversary => PairConversationSide::Target,
    }
}

fn relay_input(side: PairConversationSide, content: &str) -> String {
    match side {
        PairConversationSide::Target => content.to_string(),
        PairConversationSide::Adversary => format!(
            "The customer-service agent replied:\n\n<target_reply>\n{}\n</target_reply>\n\nContinue the conversation strictly as the user persona. Respond only with the user's next natural-language message, or call AdversaryConclude if the scenario is complete. Never answer on behalf of the customer-service agent.",
            content.trim()
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct FakeSender {
        sends: Vec<(String, String)>,
        reject_next: bool,
    }

    #[async_trait]
    impl PairMessageSender for FakeSender {
        async fn send_message_with_admission(
            &mut self,
            conversation_id: &str,
            content: &str,
        ) -> Result<bool, RuntimeError> {
            self.sends
                .push((conversation_id.to_string(), content.to_string()));
            if self.reject_next {
                self.reject_next = false;
                Ok(false)
            } else {
                Ok(true)
            }
        }
    }

    fn pair() -> AdversaryPairRuntime {
        AdversaryPairRuntime {
            pair_id: "pair-001".to_string(),
            adversary_conversation_id: "adversary".to_string(),
            target_conversation_id: "target".to_string(),
            initial_message: "My bottle leaked.".to_string(),
            status: AdversaryPairStatus::Running,
        }
    }

    fn terminal_snapshot(
        conversation_id: &str,
        sequence: u64,
        record_id: u64,
        assistant_text: &str,
    ) -> Value {
        json!({
            "schema": "agent-runtime-event/v1",
            "conversation_id": conversation_id,
            "conversation_event_seq": sequence,
            "event_seq": sequence,
            "type": "frontend:state_snapshot",
            "source": "frontend:state_snapshot",
            "payload": {
                "conversation_id": conversation_id,
                "conversation_state": "waiting",
                "ledger_delta": {
                    "kind": "append",
                    "record": {
                        "record_id": record_id,
                        "role": "assistant",
                        "content": assistant_text,
                        "metadata": {}
                    }
                }
            }
        })
    }

    fn running_snapshot(conversation_id: &str, sequence: u64) -> Value {
        json!({
            "schema": "agent-runtime-event/v1",
            "conversation_id": conversation_id,
            "conversation_event_seq": sequence,
            "event_seq": sequence,
            "type": "frontend:state_snapshot",
            "source": "frontend:state_snapshot",
            "payload": {
                "conversation_id": conversation_id,
                "conversation_state": "executing"
            }
        })
    }

    #[tokio::test]
    async fn starts_with_initial_message_to_target() {
        let mut relay = AdversaryPairRelay::new(pair(), PairRelayBudget::default());
        let mut sender = FakeSender::default();
        relay.start(&mut sender).await.unwrap();

        assert_eq!(relay.state(), PairRelayState::WaitingForTarget);
        assert_eq!(
            sender.sends,
            vec![("target".to_string(), "My bottle leaked.".to_string())]
        );
    }

    #[tokio::test]
    async fn alternates_target_and_adversary_messages() {
        let mut relay = AdversaryPairRelay::new(pair(), PairRelayBudget::default());
        let mut sender = FakeSender::default();
        relay.start(&mut sender).await.unwrap();

        relay
            .ingest(&running_snapshot("target", 1), &mut sender)
            .await
            .unwrap();
        let update = relay
            .ingest(
                &terminal_snapshot("target", 2, 1, "Please provide a photo."),
                &mut sender,
            )
            .await
            .unwrap();
        assert!(matches!(
            update,
            PairRelayUpdate::Relayed {
                from: PairConversationSide::Target,
                to: PairConversationSide::Adversary,
                ..
            }
        ));
        assert_eq!(relay.state(), PairRelayState::WaitingForAdversary);

        relay
            .ingest(&running_snapshot("adversary", 1), &mut sender)
            .await
            .unwrap();
        relay
            .ingest(
                &terminal_snapshot("adversary", 2, 1, "Can you register it first?"),
                &mut sender,
            )
            .await
            .unwrap();
        assert_eq!(relay.state(), PairRelayState::WaitingForTarget);
        assert_eq!(sender.sends.len(), 3);
        assert_eq!(sender.sends[1].0, "adversary");
        assert!(sender.sends[1].1.contains("<target_reply>"));
        assert!(sender.sends[1].1.contains("Please provide a photo."));
        assert!(sender.sends[1]
            .1
            .contains("Continue the conversation strictly as the user persona."));
        assert_eq!(sender.sends[2].0, "target");
        assert_eq!(sender.sends[2].1, "Can you register it first?");
    }

    #[tokio::test]
    async fn rejected_admission_fails_without_waiting() {
        let mut relay = AdversaryPairRelay::new(pair(), PairRelayBudget::default());
        let mut sender = FakeSender {
            reject_next: true,
            ..Default::default()
        };
        let error = relay.start(&mut sender).await.unwrap_err();

        assert!(error.to_string().contains("rejected"));
        assert_eq!(relay.state(), PairRelayState::Failed);
        assert_eq!(relay.admitted_turns(), 0);
    }

    #[tokio::test]
    async fn empty_assistant_output_fails_instead_of_looping() {
        let mut relay = AdversaryPairRelay::new(pair(), PairRelayBudget::default());
        let mut sender = FakeSender::default();
        relay.start(&mut sender).await.unwrap();

        relay
            .ingest(&running_snapshot("target", 1), &mut sender)
            .await
            .unwrap();
        let update = relay
            .ingest(&terminal_snapshot("target", 2, 1, "  "), &mut sender)
            .await
            .unwrap();
        assert!(matches!(update, PairRelayUpdate::Failed(_)));
        assert_eq!(relay.state(), PairRelayState::Failed);
        assert_eq!(sender.sends.len(), 1);
    }

    #[tokio::test]
    async fn admitted_turn_budget_stops_next_relay() {
        let mut relay = AdversaryPairRelay::new(
            pair(),
            PairRelayBudget {
                max_admitted_turns: 1,
                max_duration: Duration::from_secs(60),
            },
        );
        let mut sender = FakeSender::default();
        relay.start(&mut sender).await.unwrap();

        relay
            .ingest(&running_snapshot("target", 1), &mut sender)
            .await
            .unwrap();
        let update = relay
            .ingest(
                &terminal_snapshot("target", 2, 1, "Please provide a photo."),
                &mut sender,
            )
            .await
            .unwrap();
        assert!(matches!(update, PairRelayUpdate::Failed(_)));
        assert_eq!(relay.state(), PairRelayState::Failed);
        assert_eq!(sender.sends.len(), 1);
    }
}
