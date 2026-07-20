use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runtime::RuntimeError;

const RUNTIME_EVENT_SCHEMA: &str = "agent-runtime-event/v1";
const FRONTEND_STATE_SNAPSHOT: &str = "frontend:state_snapshot";
const CONVERSATION_LEDGER_DELTA: &str = "conversation.ledger_delta";
const CONVERSATION_STATE_DELTA: &str = "conversation.state_delta";
const TOOL_CALL_STARTED: &str = "tool_call_started";
const TOOL_CALL_FINISHED: &str = "tool_call_finished";
const TOOL_CALL_FAILED: &str = "tool_call_failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairConversationSide {
    Target,
    Adversary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantEvidence {
    pub record_id: u64,
    pub conversation_event_seq: u64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetToolEvidence {
    pub call_id: String,
    pub tool_name: Option<String>,
    pub tool_command: Option<String>,
    pub status: String,
    pub success: Option<bool>,
    pub content: String,
    pub conversation_event_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairTurnObservation {
    pub side: PairConversationSide,
    pub conversation_id: String,
    pub event_seq_from: u64,
    pub event_seq_to: u64,
    pub assistant_text: String,
    pub assistant_records: Vec<AssistantEvidence>,
    pub target_tool_evidence: Vec<TargetToolEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairObserverUpdate {
    Ignored,
    Observed,
    TurnReady(PairTurnObservation),
}

#[derive(Debug, Default)]
struct PendingTurn {
    event_seq_from: u64,
    observed_activity: bool,
    observed_running_state: bool,
    assistant_record_observed: bool,
    assistant_records: BTreeMap<u64, AssistantEvidence>,
    tool_evidence: BTreeMap<String, Vec<TargetToolEvidence>>,
}

#[derive(Debug)]
struct ConversationState {
    conversation_id: String,
    last_conversation_event_seq: u64,
    pending_turn: Option<PendingTurn>,
}

impl ConversationState {
    fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            last_conversation_event_seq: 0,
            pending_turn: None,
        }
    }
}

#[derive(Debug)]
pub struct PairEventObserver {
    target: ConversationState,
    adversary: ConversationState,
}

impl PairEventObserver {
    pub fn new(
        target_conversation_id: impl Into<String>,
        adversary_conversation_id: impl Into<String>,
    ) -> Self {
        Self {
            target: ConversationState::new(target_conversation_id),
            adversary: ConversationState::new(adversary_conversation_id),
        }
    }

    pub fn begin_turn(&mut self, side: PairConversationSide) -> Result<(), RuntimeError> {
        let state = self.state_mut(side);
        if state.pending_turn.is_some() {
            return Err(RuntimeError::InvalidConfig(format!(
                "{} conversation '{}' already has an in-flight turn",
                side.label(),
                state.conversation_id
            )));
        }
        state.pending_turn = Some(PendingTurn {
            event_seq_from: state.last_conversation_event_seq.saturating_add(1),
            ..Default::default()
        });
        Ok(())
    }

    pub fn cancel_turn(&mut self, side: PairConversationSide) {
        self.state_mut(side).pending_turn = None;
    }

    pub fn ingest(&mut self, envelope: &Value) -> Result<PairObserverUpdate, RuntimeError> {
        if envelope.get("schema").and_then(Value::as_str) != Some(RUNTIME_EVENT_SCHEMA) {
            return Ok(PairObserverUpdate::Ignored);
        }
        let Some(conversation_id) = envelope.get("conversation_id").and_then(Value::as_str) else {
            return Ok(PairObserverUpdate::Ignored);
        };
        let Some(side) = self.side_for_conversation(conversation_id) else {
            return Ok(PairObserverUpdate::Ignored);
        };
        let sequence = envelope
            .get("conversation_event_seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "pair event for conversation '{}' is missing conversation_event_seq",
                    conversation_id
                ))
            })?;

        let state = self.state_mut(side);
        if sequence == state.last_conversation_event_seq {
            return Err(RuntimeError::InvalidConfig(format!(
                "duplicate conversation_event_seq {} for '{}'",
                sequence, conversation_id
            )));
        }
        if sequence < state.last_conversation_event_seq {
            return Err(RuntimeError::InvalidConfig(format!(
                "regressing conversation_event_seq {} after {} for '{}'",
                sequence, state.last_conversation_event_seq, conversation_id
            )));
        }
        state.last_conversation_event_seq = sequence;

        let event_type = envelope.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = envelope.get("payload").ok_or_else(|| {
            RuntimeError::InvalidConfig(format!("{event_type} event is missing payload"))
        })?;
        let conversation_state = match event_type {
            FRONTEND_STATE_SNAPSHOT => payload.get("conversation_state").and_then(Value::as_str),
            CONVERSATION_STATE_DELTA => payload
                .get("conversation_state")
                .or_else(|| payload.get("state"))
                .and_then(Value::as_str),
            _ => None,
        };
        if let Some(pending) = state.pending_turn.as_mut() {
            match event_type {
                FRONTEND_STATE_SNAPSHOT => {
                    collect_snapshot_ledger_delta(sequence, payload, pending)?
                }
                CONVERSATION_LEDGER_DELTA => collect_ledger_delta(sequence, payload, pending)?,
                _ => {}
            }
            if conversation_state.is_some_and(|state| state != "waiting") {
                pending.observed_running_state = true;
            }
            if pending.observed_running_state
                || pending.assistant_record_observed
                || !pending.assistant_records.is_empty()
                || !pending.tool_evidence.is_empty()
            {
                pending.observed_activity = true;
            }
        }
        if conversation_state == Some("waiting") {
            return finish_turn(state, side, sequence);
        }
        Ok(PairObserverUpdate::Observed)
    }

    fn side_for_conversation(&self, conversation_id: &str) -> Option<PairConversationSide> {
        if self.target.conversation_id == conversation_id {
            Some(PairConversationSide::Target)
        } else if self.adversary.conversation_id == conversation_id {
            Some(PairConversationSide::Adversary)
        } else {
            None
        }
    }

    fn state_mut(&mut self, side: PairConversationSide) -> &mut ConversationState {
        match side {
            PairConversationSide::Target => &mut self.target,
            PairConversationSide::Adversary => &mut self.adversary,
        }
    }
}

fn finish_turn(
    state: &mut ConversationState,
    side: PairConversationSide,
    sequence: u64,
) -> Result<PairObserverUpdate, RuntimeError> {
    let Some(pending) = state.pending_turn.as_ref() else {
        return Ok(PairObserverUpdate::Observed);
    };
    if !pending.observed_activity
        || !pending.observed_running_state
        || !tool_calls_settled(&pending.tool_evidence)
    {
        return Ok(PairObserverUpdate::Observed);
    }

    let pending = state.pending_turn.take().expect("pending turn checked");
    let assistant_records = pending.assistant_records.into_values().collect::<Vec<_>>();
    let assistant_text = assistant_records
        .iter()
        .map(|record| record.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let target_tool_evidence = if side == PairConversationSide::Target {
        pending.tool_evidence.into_values().flatten().collect()
    } else {
        Vec::new()
    };
    Ok(PairObserverUpdate::TurnReady(PairTurnObservation {
        side,
        conversation_id: state.conversation_id.clone(),
        event_seq_from: pending.event_seq_from,
        event_seq_to: sequence,
        assistant_text,
        assistant_records,
        target_tool_evidence,
    }))
}

impl PairConversationSide {
    fn label(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Adversary => "adversary",
        }
    }
}

fn collect_ledger_delta(
    sequence: u64,
    payload: &Value,
    pending: &mut PendingTurn,
) -> Result<(), RuntimeError> {
    if let Some(records) = payload.get("records").and_then(Value::as_array) {
        for record in records {
            collect_ledger_record(sequence, record, pending)?;
        }
        return Ok(());
    }
    if let Some(record) = payload.get("record") {
        collect_ledger_record(sequence, record, pending)?;
    }
    Ok(())
}

fn collect_snapshot_ledger_delta(
    sequence: u64,
    payload: &Value,
    pending: &mut PendingTurn,
) -> Result<(), RuntimeError> {
    let Some(delta) = payload.get("ledger_delta") else {
        return Ok(());
    };
    collect_ledger_delta(sequence, delta, pending)
}

fn collect_ledger_record(
    sequence: u64,
    record: &Value,
    pending: &mut PendingTurn,
) -> Result<(), RuntimeError> {
    let role = record.get("role").and_then(Value::as_str).unwrap_or("");
    let record_id = record.get("record_id").and_then(Value::as_u64).unwrap_or(0);
    let content = record
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if role == "assistant" {
        pending.assistant_record_observed = true;
        let content = strip_tool_status_placeholders(&content);
        if !content.is_empty() {
            pending.assistant_records.insert(
                record_id,
                AssistantEvidence {
                    record_id,
                    conversation_event_seq: sequence,
                    content,
                },
            );
        }
        return Ok(());
    }

    let metadata = record.get("metadata").unwrap_or(&Value::Null);
    let subtype = metadata
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !matches!(
        subtype,
        TOOL_CALL_STARTED | TOOL_CALL_FINISHED | TOOL_CALL_FAILED
    ) {
        return Ok(());
    }
    let call_id = metadata
        .get("extra")
        .and_then(|extra| extra.get("call_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::InvalidConfig(format!(
                "target tool evidence at sequence {} is missing call_id",
                sequence
            ))
        })?
        .to_string();
    pending
        .tool_evidence
        .entry(call_id.clone())
        .or_default()
        .push(TargetToolEvidence {
            call_id,
            tool_name: optional_string(metadata, "tool_name"),
            tool_command: optional_string(metadata, "tool_command"),
            status: subtype.to_string(),
            success: metadata.get("success").and_then(Value::as_bool),
            content,
            conversation_event_seq: sequence,
        });
    Ok(())
}

fn strip_tool_status_placeholders(content: &str) -> String {
    content
        .lines()
        .filter(|line| {
            let line = line.trim();
            !(line.starts_with("[tool:status") && line.ends_with(']') && line.contains("call_id="))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn tool_calls_settled(tool_evidence: &BTreeMap<String, Vec<TargetToolEvidence>>) -> bool {
    tool_evidence.values().all(|events| {
        events.last().is_some_and(|event| {
            matches!(event.status.as_str(), TOOL_CALL_FINISHED | TOOL_CALL_FAILED)
        })
    })
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn snapshot(
        conversation_id: &str,
        sequence: u64,
        conversation_state: &str,
        record: Option<Value>,
    ) -> Value {
        let mut payload = json!({
            "conversation_id": conversation_id,
            "conversation_state": conversation_state
        });
        if let Some(record) = record {
            payload["ledger_delta"] = json!({"kind": "append", "record": record});
        }
        json!({
            "schema": RUNTIME_EVENT_SCHEMA,
            "conversation_id": conversation_id,
            "conversation_event_seq": sequence,
            "event_seq": sequence,
            "type": FRONTEND_STATE_SNAPSHOT,
            "source": FRONTEND_STATE_SNAPSHOT,
            "payload": payload
        })
    }

    fn assistant(record_id: u64, content: &str) -> Value {
        json!({
            "record_id": record_id,
            "role": "assistant",
            "content": content,
            "metadata": {}
        })
    }

    fn stopped_snapshot(conversation_id: &str, sequence: u64, record: Option<Value>) -> Value {
        snapshot(conversation_id, sequence, "waiting", record)
    }

    fn ledger_delta(conversation_id: &str, sequence: u64, record: Value) -> Value {
        json!({
            "schema": RUNTIME_EVENT_SCHEMA,
            "conversation_id": conversation_id,
            "conversation_event_seq": sequence,
            "event_seq": sequence,
            "type": CONVERSATION_LEDGER_DELTA,
            "source": CONVERSATION_LEDGER_DELTA,
            "payload": {"kind": "append", "record": record}
        })
    }

    fn state_delta(conversation_id: &str, sequence: u64, state: &str) -> Value {
        json!({
            "schema": RUNTIME_EVENT_SCHEMA,
            "conversation_id": conversation_id,
            "conversation_event_seq": sequence,
            "event_seq": sequence,
            "type": CONVERSATION_STATE_DELTA,
            "source": CONVERSATION_STATE_DELTA,
            "payload": {"state": state}
        })
    }

    fn tool(record_id: u64, subtype: &str, call_id: &str) -> Value {
        json!({
            "record_id": record_id,
            "role": if subtype == TOOL_CALL_STARTED { "gateway_message" } else { "tool" },
            "content": "internal tool result",
            "metadata": {
                "subtype": subtype,
                "tool_name": "AfterSalesCreate",
                "success": subtype == TOOL_CALL_FINISHED,
                "extra": {"call_id": call_id}
            }
        })
    }

    #[test]
    fn ignores_other_conversations() {
        let mut observer = PairEventObserver::new("target", "adversary");
        assert_eq!(
            observer
                .ingest(&snapshot("other", 1, "waiting", None))
                .unwrap(),
            PairObserverUpdate::Ignored
        );
    }

    #[test]
    fn detects_duplicate_and_regressing_sequences() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer
            .ingest(&snapshot("target", 2, "waiting", None))
            .unwrap();
        assert!(observer
            .ingest(&snapshot("target", 2, "waiting", None))
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
        assert!(observer
            .ingest(&snapshot("target", 1, "waiting", None))
            .unwrap_err()
            .to_string()
            .contains("regressing"));
    }

    #[test]
    fn idle_snapshot_does_not_finish_until_turn_is_started() {
        let mut observer = PairEventObserver::new("target", "adversary");
        assert_eq!(
            observer
                .ingest(&snapshot("target", 1, "waiting", None))
                .unwrap(),
            PairObserverUpdate::Observed
        );
        observer.begin_turn(PairConversationSide::Target).unwrap();
        assert_eq!(
            observer
                .ingest(&snapshot("target", 2, "thinking", None))
                .unwrap(),
            PairObserverUpdate::Observed
        );
    }

    #[test]
    fn delayed_idle_snapshot_after_begin_turn_does_not_finish_empty() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer.begin_turn(PairConversationSide::Target).unwrap();

        assert_eq!(
            observer
                .ingest(&snapshot("target", 1, "waiting", None))
                .unwrap(),
            PairObserverUpdate::Observed
        );
        assert_eq!(
            observer
                .ingest(&snapshot("target", 2, "thinking", None))
                .unwrap(),
            PairObserverUpdate::Observed
        );

        let update = observer
            .ingest(&stopped_snapshot(
                "target",
                3,
                Some(assistant(1, "Actual target reply.")),
            ))
            .unwrap();
        let PairObserverUpdate::TurnReady(turn) = update else {
            panic!("expected completed turn");
        };
        assert_eq!(turn.assistant_text, "Actual target reply.");
        assert_eq!(turn.event_seq_from, 1);
        assert_eq!(turn.event_seq_to, 3);
    }

    #[test]
    fn admission_event_does_not_finish_a_started_turn() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer.begin_turn(PairConversationSide::Target).unwrap();
        let admission = json!({
            "schema": RUNTIME_EVENT_SCHEMA,
            "conversation_id": "target",
            "conversation_event_seq": 1,
            "event_seq": 1,
            "type": "gateway:command-admitted",
            "source": "gateway:command-admitted",
            "payload": {"decision": "accepted"}
        });

        assert_eq!(
            observer.ingest(&admission).unwrap(),
            PairObserverUpdate::Observed
        );
    }

    #[test]
    fn does_not_finish_while_pause_is_available() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer.begin_turn(PairConversationSide::Target).unwrap();
        assert_eq!(
            observer
                .ingest(&snapshot(
                    "target",
                    1,
                    "thinking",
                    Some(assistant(1, "still working"))
                ))
                .unwrap(),
            PairObserverUpdate::Observed
        );
    }

    #[test]
    fn emits_assistant_text_on_terminal_snapshot() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer.begin_turn(PairConversationSide::Target).unwrap();
        observer
            .ingest(&snapshot(
                "target",
                1,
                "thinking",
                Some(assistant(1, "First paragraph.")),
            ))
            .unwrap();
        let update = observer
            .ingest(&stopped_snapshot(
                "target",
                2,
                Some(assistant(2, "Final paragraph.")),
            ))
            .unwrap();

        let PairObserverUpdate::TurnReady(turn) = update else {
            panic!("expected completed turn");
        };
        assert_eq!(turn.assistant_text, "First paragraph.\n\nFinal paragraph.");
        assert_eq!(turn.event_seq_from, 1);
        assert_eq!(turn.event_seq_to, 2);
    }

    #[test]
    fn consumes_ledger_and_state_delta_events() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer.begin_turn(PairConversationSide::Target).unwrap();

        assert_eq!(
            observer
                .ingest(&state_delta("target", 1, "thinking"))
                .unwrap(),
            PairObserverUpdate::Observed
        );
        assert_eq!(
            observer
                .ingest(&ledger_delta("target", 2, assistant(7, "Delta reply.")))
                .unwrap(),
            PairObserverUpdate::Observed
        );
        let update = observer
            .ingest(&state_delta("target", 3, "waiting"))
            .unwrap();

        let PairObserverUpdate::TurnReady(turn) = update else {
            panic!("expected completed turn");
        };
        assert_eq!(turn.assistant_text, "Delta reply.");
        assert_eq!(turn.event_seq_from, 1);
        assert_eq!(turn.event_seq_to, 3);
    }

    #[test]
    fn preserves_target_tool_evidence_without_mixing_it_into_reply() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer.begin_turn(PairConversationSide::Target).unwrap();
        observer
            .ingest(&snapshot(
                "target",
                1,
                "executing",
                Some(tool(1, TOOL_CALL_STARTED, "target:1:0")),
            ))
            .unwrap();
        observer
            .ingest(&snapshot(
                "target",
                2,
                "executing",
                Some(tool(2, TOOL_CALL_FINISHED, "target:1:0")),
            ))
            .unwrap();
        let update = observer
            .ingest(&stopped_snapshot(
                "target",
                3,
                Some(assistant(3, "Your request was created.")),
            ))
            .unwrap();

        let PairObserverUpdate::TurnReady(turn) = update else {
            panic!("expected completed turn");
        };
        assert_eq!(turn.assistant_text, "Your request was created.");
        assert!(!turn.assistant_text.contains("internal tool result"));
        assert_eq!(turn.target_tool_evidence.len(), 2);
        assert_eq!(turn.target_tool_evidence[0].status, TOOL_CALL_STARTED);
        assert_eq!(turn.target_tool_evidence[1].status, TOOL_CALL_FINISHED);
        assert!(turn
            .target_tool_evidence
            .iter()
            .all(|evidence| evidence.call_id == "target:1:0"));
    }

    #[test]
    fn adversary_tool_placeholder_is_not_relayed_before_post_tool_reply() {
        let mut observer = PairEventObserver::new("target", "adversary");
        observer
            .begin_turn(PairConversationSide::Adversary)
            .unwrap();
        assert_eq!(
            observer
                .ingest(&snapshot(
                    "adversary",
                    1,
                    "thinking",
                    Some(assistant(
                        1,
                        "Let me check.\n\n[tool:status | call_id=\"adversary:1:0\"]"
                    )),
                ))
                .unwrap(),
            PairObserverUpdate::Observed
        );
        assert_eq!(
            observer
                .ingest(&snapshot(
                    "adversary",
                    2,
                    "executing",
                    Some(tool(2, TOOL_CALL_FAILED, "adversary:1:0")),
                ))
                .unwrap(),
            PairObserverUpdate::Observed
        );

        let update = observer
            .ingest(&stopped_snapshot(
                "adversary",
                3,
                Some(assistant(3, "Could you share the order number?")),
            ))
            .unwrap();
        let PairObserverUpdate::TurnReady(turn) = update else {
            panic!("expected completed adversary turn");
        };
        assert_eq!(
            turn.assistant_text,
            "Let me check.\n\nCould you share the order number?"
        );
        assert!(turn.target_tool_evidence.is_empty());
        assert!(!turn.assistant_text.contains("[tool:status"));
    }
}
