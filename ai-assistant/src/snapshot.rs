use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use corework::cache::CacheExt;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};

use crate::agent::AgentCluster;
use crate::context::keys;
use crate::context::AssistantContext;
use crate::ledger::LedgerRecord;
use crate::systems::ledger::{QueryAgentContextInput, QueryAgentLlmExecutionSnapshotSystem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendStateSnapshot {
    pub conversation_id: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_delta: Option<LedgerDelta>,
    pub agents: Vec<AgentRuntimeView>,
    pub active_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub conversation_state: ConversationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_permissions: Vec<crate::permission::PendingToolPermission>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationState {
    Waiting,
    Thinking,
    Executing,
    Compacting,
    Stopping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimeView {
    pub agent_id: String,
    pub agent_name: String,
    pub status: String,
    pub context_usage: ContextUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextUsage {
    pub message_count: usize,
    pub estimated_tokens: u64,
    pub has_summary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_summary_at_msg: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerDelta {
    pub kind: LedgerDeltaKind,
    pub record: LedgerRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerDeltaKind {
    Append,
    Replace,
}

pub struct SnapshotBuilder {
    revision: AtomicU64,
    last_hash: Mutex<Option<u64>>,
}

impl Default for SnapshotBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotBuilder {
    pub fn new() -> Self {
        Self {
            revision: AtomicU64::new(0),
            last_hash: Mutex::new(None),
        }
    }

    pub async fn build_if_changed(
        &self,
        conversation_id: impl Into<String>,
        cluster: &Arc<AgentCluster>,
        ledger_delta: Option<LedgerDelta>,
    ) -> Option<FrontendStateSnapshot> {
        let mut snapshot = self.build(conversation_id, cluster, ledger_delta).await;
        let hash = snapshot_hash(&snapshot);
        {
            let mut guard = self.last_hash.lock().expect("snapshot hash mutex poisoned");
            if guard.as_ref().copied() == Some(hash) {
                return None;
            }
            *guard = Some(hash);
        }
        snapshot.revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        Some(snapshot)
    }

    pub async fn build(
        &self,
        conversation_id: impl Into<String>,
        cluster: &Arc<AgentCluster>,
        ledger_delta: Option<LedgerDelta>,
    ) -> FrontendStateSnapshot {
        let conversation_id = conversation_id.into();
        let cluster_snapshot = cluster.snapshot().await;
        let active_agent_id = cluster_snapshot.active_agent_id.clone();
        let active_cache = cluster
            .get(&active_agent_id)
            .await
            .map(|agent| agent.sm.unit().cache());
        let pending_permissions = match cluster.get(&active_agent_id).await {
            Some(agent) => match agent
                .sm
                .unit()
                .resolve_shared_component::<crate::permission::PermissionBroker>()
            {
                Some(broker) => broker.pending().await,
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        let model = read_string_config(
            active_cache.as_ref(),
            crate::config_resolver::conversation_keys::CONFIG_MODEL,
            Some(keys::MODEL),
        )
        .await;
        let summary_model = read_string_config(
            active_cache.as_ref(),
            crate::config_resolver::conversation_keys::CONFIG_SUMMARY_MODEL,
            None,
        )
        .await;
        let language = read_string_config(
            active_cache.as_ref(),
            crate::config_resolver::conversation_keys::CONFIG_LANGUAGE,
            Some(keys::LANGUAGE),
        )
        .await;
        let plan = match active_cache.as_ref() {
            Some(cache) => AssistantContext::get_current_plan(cache)
                .await
                .ok()
                .flatten()
                .and_then(|plan| serde_json::to_value(plan).ok()),
            None => None,
        };
        let mut agents = Vec::with_capacity(cluster_snapshot.agents.len());
        let mut conversation_state = ConversationState::Waiting;
        for agent in cluster_snapshot.agents {
            let runtime = cluster.get(&agent.agent_id).await;
            if let Some(runtime) = runtime.as_ref() {
                let cache = runtime.sm.unit().cache();
                let compacting =
                    read_bool_config(Some(&cache), crate::context::keys::COMPACT_IN_PROGRESS).await;
                let stopping = read_bool_config(Some(&cache), keys::PAUSE_REQUESTED).await;
                conversation_state = merge_conversation_state(
                    conversation_state,
                    project_agent_state(&agent.state, compacting, stopping, Some(&cache)).await,
                );
            }
            let context_usage = match runtime {
                Some(runtime) => {
                    let ctx = runtime.sm.unit().create_context();
                    match QueryAgentLlmExecutionSnapshotSystem
                        .execute(
                            QueryAgentContextInput {
                                conversation_id: conversation_id.clone(),
                                agent_id: agent.agent_id.clone(),
                            },
                            &ctx,
                        )
                        .await
                    {
                        Ok(snapshot) => {
                            let mut usage = context_usage_from_messages(&snapshot.messages);
                            usage.has_summary = snapshot.has_summary;
                            usage
                        }
                        Err(_) => ContextUsage {
                            message_count: 0,
                            estimated_tokens: 0,
                            has_summary: false,
                            last_summary_at_msg: None,
                        },
                    }
                }
                None => ContextUsage {
                    message_count: 0,
                    estimated_tokens: 0,
                    has_summary: false,
                    last_summary_at_msg: None,
                },
            };
            agents.push(AgentRuntimeView {
                agent_id: agent.agent_id,
                agent_name: agent.agent_name,
                status: agent.state,
                context_usage,
            });
        }
        FrontendStateSnapshot {
            conversation_id,
            revision: 0,
            ledger_delta,
            agents,
            active_agent_id,
            model,
            summary_model,
            language,
            conversation_state,
            plan,
            pending_permissions,
        }
    }
}

fn merge_conversation_state(
    current: ConversationState,
    candidate: ConversationState,
) -> ConversationState {
    if conversation_state_priority(candidate) > conversation_state_priority(current) {
        candidate
    } else {
        current
    }
}

fn conversation_state_priority(state: ConversationState) -> u8 {
    match state {
        ConversationState::Waiting => 0,
        ConversationState::Thinking => 1,
        ConversationState::Executing => 2,
        ConversationState::Stopping => 3,
        ConversationState::Compacting => 4,
    }
}

async fn project_agent_state(
    state: &str,
    compacting: bool,
    stopping: bool,
    cache: Option<&Arc<dyn corework::cache::Cache>>,
) -> ConversationState {
    if compacting {
        return ConversationState::Compacting;
    }
    if stopping {
        return ConversationState::Stopping;
    }
    match state {
        crate::state::states::EXECUTING => ConversationState::Executing,
        crate::state::states::THINKING => ConversationState::Thinking,
        crate::state::states::SAYING => {
            let next = match cache {
                Some(cache) => cache
                    .get::<String>(keys::NEXT_STATE_AFTER_SAYING)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            if next.as_deref() == Some(crate::state::states::THINKING) {
                ConversationState::Thinking
            } else {
                ConversationState::Waiting
            }
        }
        _ => ConversationState::Waiting,
    }
}

fn snapshot_hash(snapshot: &FrontendStateSnapshot) -> u64 {
    let mut stable = snapshot.clone();
    stable.revision = 0;
    let bytes = serde_json::to_vec(&stable).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn ledger_append_delta(record: LedgerRecord) -> LedgerDelta {
    let mut record = record;
    if let Some(display_content) = record.metadata.display_content.clone() {
        record.content = display_content;
    }
    LedgerDelta {
        kind: LedgerDeltaKind::Append,
        record,
    }
}

fn context_usage_from_messages(messages: &[crate::context::Message]) -> ContextUsage {
    let message_count = messages.len();
    let estimated_tokens = messages
        .iter()
        .map(|message| {
            (message.content.chars().count() as u64)
                .saturating_div(4)
                .max(1)
        })
        .sum();
    let mut summary_positions = BTreeMap::new();
    for (idx, message) in messages.iter().enumerate() {
        if message.role == crate::context::roles::SUMMARY
            || message.role == crate::context::roles::COMPACT_SUMMARY
        {
            summary_positions.insert(idx, idx);
        }
    }
    let last_summary_at_msg = summary_positions.keys().next_back().copied();
    ContextUsage {
        message_count,
        estimated_tokens,
        has_summary: last_summary_at_msg.is_some(),
        last_summary_at_msg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_state_priority_is_stable() {
        assert!(
            conversation_state_priority(ConversationState::Compacting)
                > conversation_state_priority(ConversationState::Stopping)
        );
        assert!(
            conversation_state_priority(ConversationState::Executing)
                > conversation_state_priority(ConversationState::Thinking)
        );
    }

    #[test]
    fn ledger_delta_uses_display_projection_without_mutating_canonical_shape() {
        let mut record = LedgerRecord {
            record_id: 1,
            conversation_id: "current".to_string(),
            agent_id: "boss".to_string(),
            agent_name: "Boss".to_string(),
            role: crate::ledger::LedgerRole::Assistant,
            content: "EXEC ReadFile --path a.docx".to_string(),
            metadata: crate::ledger::LedgerMessageMeta::default(),
            created_at: "2026-05-27T00:00:00+08:00".to_string(),
        };
        record.metadata.display_content = Some("[tool:status | call_id=\"boss:1:0\"]".to_string());

        let delta = ledger_append_delta(record);
        assert_eq!(delta.record.content, "[tool:status | call_id=\"boss:1:0\"]");
    }
}

async fn read_string_config(
    cache: Option<&Arc<dyn corework::cache::Cache>>,
    conversation_key: &str,
    fallback_key: Option<&str>,
) -> Option<String> {
    let cache = cache?;
    if let Ok(Some(value)) = cache.get::<String>(conversation_key).await {
        if !value.is_empty() {
            return Some(value);
        }
    }
    if let Some(fallback_key) = fallback_key {
        if let Ok(Some(value)) = cache.get::<String>(fallback_key).await {
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

async fn read_bool_config(cache: Option<&Arc<dyn corework::cache::Cache>>, key: &str) -> bool {
    let Some(cache) = cache else {
        return false;
    };
    cache.get::<bool>(key).await.ok().flatten().unwrap_or(false)
}
