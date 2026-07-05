//! Agent Test Studio-only tool contracts.
//!
//! Implementations are bound to a live `AgentTestController` when the Studio
//! session is created, so these tools are never exposed to normal agents.

use serde::{Deserialize, Serialize};

#[cfg(test)]
pub const ADVERSARY_CREATE: &str = "AdversaryCreate";
#[cfg(test)]
pub const ADVERSARY_DESTROY: &str = "AdversaryDestroy";
#[cfg(test)]
pub const ADVERSARY_INSPECT: &str = "AdversaryInspect";
#[cfg(test)]
pub const ADVERSARY_CONCLUDE: &str = "AdversaryConclude";

pub const ADVERSARY_COMPLETED_EVENT: &str = "agent-test.adversary.completed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryCreateArgs {
    pub persona: super::role_contract::AdversaryPersona,
    #[serde(default)]
    pub initial_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryDestroyArgs {
    pub pair_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectMode {
    Report,
    Transcript,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryInspectArgs {
    pub pair_id: String,
    pub mode: InspectMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryConcludeArgs {
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<AdversaryFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdversaryFinding {
    pub title: String,
    pub observation: String,
    pub expected_behavior: String,
    #[serde(default)]
    pub evidence_turns: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdversaryConclusionReport {
    pub pair_id: String,
    pub summary: String,
    pub findings: Vec<AdversaryFinding>,
    pub evidence_refs: Vec<AdversaryEvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdversaryEvidenceRef {
    pub side: super::event_observer::PairConversationSide,
    pub conversation_id: String,
    pub turn: u32,
    pub event_seq_from: u64,
    pub event_seq_to: u64,
    #[serde(default)]
    pub tool_call_ids: Vec<String>,
}
