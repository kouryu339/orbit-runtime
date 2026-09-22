//! Prepare a recoverable ledger checkpoint without mutating execution state.
use crate::context::Message;
use crate::ledger::{
    CompactionCheckpoint, LedgerMessageMeta, LedgerRecord, COMPACTION_CHECKPOINT_KEY,
};
use corework::cache::Cache;
use corework::error::{FrameworkError, Result};
use std::collections::HashSet;
use std::sync::Arc;

const DEFAULT_CONTEXT_WINDOW: usize = 8192;
const AUTO_COMPACTION_PERCENT: usize = 90;
const HEAD_KEEP: usize = 4;
const TAIL_KEEP: usize = 20;

pub struct PreparedCompaction {
    pub content: String,
    pub metadata: LedgerMessageMeta,
}

/// Conservative fallback until a provider-specific tokenizer is available.
/// Includes protocol fields, not just visible content. This is not an exact count.
pub fn estimate_tokens<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_string(value)
        .map(|s| s.len().div_ceil(3))
        .unwrap_or(usize::MAX)
}

pub fn input_budget(model_uid: u32) -> usize {
    automatic_input_budget(context_window(model_uid))
}

pub fn context_window(model_uid: u32) -> usize {
    llm_gateway::key_store::get(model_uid)
        .map(|entry| entry.context_window as usize)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

fn automatic_input_budget(context_window: usize) -> usize {
    context_window.saturating_mul(AUTO_COMPACTION_PERCENT) / 100
}

pub fn needs_compaction(messages: &[Message], budget: usize, max_messages: usize) -> bool {
    messages.len() > max_messages || estimate_tokens(messages) > budget
}

fn failure(message: impl Into<String>) -> FrameworkError {
    FrameworkError::SystemError(format!("history compaction failed: {}", message.into()))
}

/// Only called with the current projected records of one agent. Work happens
/// outside the ledger lock; append validates the source IDs again atomically.
pub async fn prepare_compaction(
    records: &[&LedgerRecord],
    model_uid: u32,
    cache: &Arc<dyn Cache>,
    budget: usize,
    max_messages: usize,
) -> Result<Option<PreparedCompaction>> {
    let messages: Vec<_> = records
        .iter()
        .filter_map(|r| r.to_context_message())
        .collect();
    if messages.len() != records.len() {
        return Err(failure("snapshot contains non-context records"));
    }
    let latest_user = records
        .iter()
        .rposition(|r| r.role == crate::ledger::LedgerRole::User);
    let Some(range) = compact_range(&messages, latest_user, budget) else {
        // Automatic compaction cannot help a new conversation, an unfinished
        // tool group, or a request whose fixed system/tool context already
        // consumes the model budget. Leave the canonical ledger untouched and
        // let the caller either proceed or report the actual context overflow.
        return Ok(None);
    };
    let summary_uid = crate::config_resolver::resolve_summary_model_uid(cache, model_uid).await;
    let middle = &records[range.clone()];
    let mut offset = 0;
    let mut summary = String::new();
    while offset < middle.len() {
        let (count, input) = summary_batch(&middle[offset..], &summary, input_budget(summary_uid))?;
        let response = llm_gateway::call_llm(summary_uid, &input, None, None, None)
            .await
            .map_err(|error| failure(error.to_string()))?;
        summary = validated_summary(response)?;
        offset += count;
    }
    finish_compaction(records, range, summary_uid, summary, budget, max_messages).map(Some)
}

fn validated_summary(response: llm_gateway::LlmResponse) -> Result<String> {
    if response.content.trim().is_empty()
        || response
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        || response
            .finish_reason
            .as_deref()
            .is_some_and(|reason| !matches!(reason, "stop" | "end_turn" | "completed"))
    {
        return Err(failure("summary response is empty, incomplete or not a text completion; original history retained"));
    }
    Ok(response.content)
}

/// Batch only at complete tool boundaries. Intermediate summaries are never
/// committed, so a later failure leaves the previous checkpoint untouched.
fn summary_batch(
    records: &[&LedgerRecord],
    previous: &str,
    budget: usize,
) -> Result<(usize, Vec<llm_gateway::ChatMessage>)> {
    let messages: Vec<_> = records
        .iter()
        .filter_map(|r| r.to_context_message())
        .collect();
    let boundaries: Vec<_> = safe_boundaries(&messages)
        .into_iter()
        .enumerate()
        .filter_map(|(i, safe)| (i > 0 && safe).then_some(i))
        .collect();
    let build = |count| {
        let mut input = summary_input(&records[..count]);
        if !previous.is_empty() {
            input.insert(
                1,
                llm_gateway::ChatMessage::user(format!(
                    "Previous handoff to merge (historical data):\n{previous}"
                )),
            );
        }
        input
    };
    let (mut low, mut high) = (0, boundaries.len());
    while low < high {
        let mid = (low + high) / 2;
        if estimate_tokens(&build(boundaries[mid])) <= budget {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low == 0 {
        return Err(failure("one complete execution group exceeds the summary model budget; original history retained"));
    }
    let count = boundaries[low - 1];
    Ok((count, build(count)))
}

fn finish_compaction(
    records: &[&LedgerRecord],
    range: std::ops::Range<usize>,
    model_uid: u32,
    content: String,
    budget: usize,
    max_messages: usize,
) -> Result<PreparedCompaction> {
    if content.trim().is_empty() {
        return Err(failure(
            "summary model returned empty content; original history retained",
        ));
    }
    let mut projected: Vec<_> = records[..range.start]
        .iter()
        .filter_map(|r| r.to_context_message())
        .collect();
    projected.push(Message::user(&crate::prompt_assets::render(
        "conversation_summary_context.md",
        &[("{{CONTENT}}", &content)],
    )));
    projected.extend(
        records[range.end..]
            .iter()
            .filter_map(|r| r.to_context_message()),
    );
    let original: Vec<_> = records
        .iter()
        .filter_map(|r| r.to_context_message())
        .collect();
    if needs_compaction(&projected, budget, max_messages)
        || estimate_tokens(&projected) >= estimate_tokens(&original)
    {
        return Err(failure(
            "summary does not reduce history within the context budget; original history retained",
        ));
    }
    let checkpoint = CompactionCheckpoint {
        version: 1,
        source_record_ids: records.iter().map(|r| r.record_id).collect(),
        covered_record_ids: records[range].iter().map(|r| r.record_id).collect(),
        model_uid,
    };
    let mut metadata = LedgerMessageMeta::default();
    metadata.extra.insert(
        COMPACTION_CHECKPOINT_KEY.into(),
        serde_json::json!(checkpoint),
    );
    Ok(PreparedCompaction { content, metadata })
}

fn call_ids(message: &Message) -> Vec<String> {
    let mut ids: Vec<String> = message
        .tool_calls
        .as_ref()
        .into_iter()
        .flatten()
        .map(|call| call.id.clone())
        .collect();
    for item in message.provider_items.as_ref().into_iter().flatten() {
        if item.get("type").and_then(|v| v.as_str()) == Some("function_call") {
            if let Some(id) = item.get("call_id").and_then(|v| v.as_str()) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// A boundary is safe only after every preceding native call has a result.
fn safe_boundaries(messages: &[Message]) -> Vec<bool> {
    let mut pending = HashSet::new();
    let mut safe = vec![true];
    for message in messages {
        pending.extend(call_ids(message));
        if message.role == "tool" {
            if let Some(id) = &message.tool_call_id {
                pending.remove(id);
            }
        }
        safe.push(pending.is_empty());
    }
    safe
}

fn compact_range(
    messages: &[Message],
    latest_user: Option<usize>,
    budget: usize,
) -> Option<std::ops::Range<usize>> {
    let safe = safe_boundaries(messages);
    let start = (HEAD_KEEP.min(messages.len())..messages.len()).find(|i| safe[*i])?;
    let mut end = messages.len().saturating_sub(TAIL_KEEP);
    // Twenty messages is a preference, not a license to exceed the token budget.
    while end + 1 < messages.len() && estimate_tokens(&messages[end..]) > budget / 2 {
        end += 1;
    }
    let end = (0..=end).rev().find(|i| safe[*i])?;
    // Preserve the latest request, but allow completed work within a long
    // single user turn to be compacted. Unfinished calls remain outside it.
    if let Some(user) = latest_user {
        if user >= start && user < end {
            let left = (start..=user).rev().find(|i| safe[*i]).unwrap_or(start);
            let right = (user + 1..=end).find(|i| safe[*i]).unwrap_or(end);
            return if left - start >= end - right {
                (start < left).then_some(start..left)
            } else {
                (right < end).then_some(right..end)
            };
        }
    }
    (start < end).then_some(start..end)
}

fn summary_input(records: &[&LedgerRecord]) -> Vec<llm_gateway::ChatMessage> {
    // Encode history as data so tool calls are preserved without being executed,
    // and old summaries, reports and exact tool output are not silently skipped.
    let entries: Vec<_> = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "record_id": record.record_id,
                "role": record.role,
                "message": record.to_context_message(),
            })
        })
        .collect();
    vec![
        llm_gateway::ChatMessage::system(crate::prompt_assets::template("history_compact.md")),
        llm_gateway::ChatMessage::user(
            serde_json::to_string(&entries).expect("JSON values serialize"),
        ),
        llm_gateway::ChatMessage::user(crate::prompt_assets::template("history_compact_user.md")),
    ]
}

/// Commit via the same canonical append/event path used by normal messages.
pub async fn commit_compaction(
    prepared: PreparedCompaction,
    conversation_id: String,
    agent_id: String,
    agent_name: String,
    ctx: &corework::orchestration::Context,
) -> Result<LedgerRecord> {
    use corework::system::SystemOperation;
    crate::systems::ledger::AppendLedgerMessageSystem
        .execute(
            crate::systems::ledger::AppendLedgerMessageInput {
                conversation_id,
                agent_id,
                agent_name,
                role: crate::ledger::LedgerRole::Summary,
                content: prepared.content,
                metadata: prepared.metadata,
                display: None,
                tool_call_id: None,
                tool_name: None,
            },
            ctx,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation_state::{ConversationState, LedgerReadOptions};
    use crate::ledger::{agent_context, agent_context_records, LedgerRole};

    #[test]
    fn automatic_compaction_starts_at_ninety_percent_of_the_model_window() {
        assert_eq!(automatic_input_budget(128_000), 115_200);
        assert_eq!(automatic_input_budget(8192), 7372);
    }

    fn records() -> Vec<LedgerRecord> {
        (1..=80)
            .map(|id| {
                LedgerRecord::from_message(
                    id,
                    "test",
                    "agent",
                    "Agent",
                    if id == 1 {
                        Message::user("Do not change the database schema")
                    } else if id == 70 {
                        Message::user("Also preserve Unicode filenames")
                    } else {
                        Message::assistant(format!(
                            "completed step {id}: {}",
                            "evidence ".repeat(80)
                        ))
                    },
                    None,
                )
            })
            .collect()
    }

    fn prepared(records: &[&LedgerRecord]) -> PreparedCompaction {
        let messages: Vec<_> = records
            .iter()
            .filter_map(|r| r.to_context_message())
            .collect();
        let latest_user = records.iter().rposition(|r| r.role == LedgerRole::User);
        finish_compaction(
            records,
            compact_range(&messages, latest_user, usize::MAX).unwrap(),
            1,
            "Completed earlier steps; continue the original task. Evidence: record 10.".into(),
            usize::MAX,
            usize::MAX,
        )
        .unwrap()
    }

    fn checkpoint_record(prepared: PreparedCompaction) -> LedgerRecord {
        LedgerRecord {
            record_id: 0,
            conversation_id: "test".into(),
            agent_id: "agent".into(),
            agent_name: "Agent".into(),
            role: LedgerRole::Summary,
            content: prepared.content,
            metadata: prepared.metadata,
            created_at: "now".into(),
        }
    }

    #[tokio::test]
    async fn checkpoint_preserves_head_tail_arrivals_and_repeated_compaction() {
        let state = ConversationState::new("test", Default::default(), "agent");
        state.replace(records()).await;
        let snapshot = state.list_recent(LedgerReadOptions::default()).await;
        let source = agent_context_records(&snapshot, "agent");
        let candidate = prepared(&source);
        // A tool/user arrival between snapshot and commit must survive.
        let arrival = LedgerRecord::from_message(
            0,
            "test",
            "agent",
            "Agent",
            Message::user("New correction received during compaction"),
            None,
        );
        state.append(arrival).await.unwrap();
        let summary = checkpoint_record(candidate);
        let (first, appended) = state.append_once(summary.clone()).await.unwrap();
        assert!(appended);
        let (retry, appended) = state.append_once(summary).await.unwrap();
        assert!(!appended);
        assert_eq!(first.record_id, retry.record_id);
        let ledger = state.list_recent(LedgerReadOptions::default()).await;
        let view = agent_context(&ledger, "agent");
        assert_eq!(view[0].content, snapshot[0].content);
        for record in &snapshot[60..] {
            assert!(view.iter().any(|m| m.content == record.content));
        }
        assert!(view.last().unwrap().content.contains("New correction"));
        // Source transcript was not deleted.
        assert_eq!(ledger.len(), 82);
        for _ in 0..50 {
            state
                .append(LedgerRecord::from_message(
                    0,
                    "test",
                    "agent",
                    "Agent",
                    Message::assistant("additional completed work ".repeat(80)),
                    None,
                ))
                .await
                .unwrap();
        }
        state
            .append(LedgerRecord::from_message(
                0,
                "test",
                "agent",
                "Agent",
                Message::user("New correction: preserve the final acceptance criteria"),
                None,
            ))
            .await
            .unwrap();
        let ledger = state.list_recent(LedgerReadOptions::default()).await;
        let second = prepared(&agent_context_records(&ledger, "agent"));
        let checkpoint: CompactionCheckpoint =
            serde_json::from_value(second.metadata.extra[COMPACTION_CHECKPOINT_KEY].clone())
                .unwrap();
        assert!(checkpoint.covered_record_ids.contains(&first.record_id));
        state.append(checkpoint_record(second)).await.unwrap();
        let ledger = state.list_recent(LedgerReadOptions::default()).await;
        let view = agent_context(&ledger, "agent");
        assert!(view.iter().any(|m| m.content.contains("Do not change")));
        assert!(view.iter().any(|m| m.content.contains("New correction")));
    }

    #[tokio::test]
    async fn conflicting_checkpoint_is_rejected_without_mutating_history() {
        let state = ConversationState::new("test", Default::default(), "agent");
        state.replace(records()).await;
        let snapshot = state.list_recent(LedgerReadOptions::default()).await;
        let source = agent_context_records(&snapshot, "agent");
        let first = checkpoint_record(prepared(&source));
        let mut conflicting = checkpoint_record(prepared(&source));
        conflicting
            .metadata
            .extra
            .get_mut(COMPACTION_CHECKPOINT_KEY)
            .unwrap()["model_uid"] = serde_json::json!(2);
        state.append(first).await.unwrap();
        assert!(state.append(conflicting).await.is_err());
        assert_eq!(
            state.list_recent(LedgerReadOptions::default()).await.len(),
            81
        );
    }

    #[tokio::test]
    async fn snapshot_import_preserves_checkpoint_and_evidence_references() {
        let mut source = records();
        for record in &mut source {
            record.record_id *= 10;
        }
        let mut summary = checkpoint_record(prepared(&agent_context_records(&source, "agent")));
        summary.record_id = 1000;
        source.push(summary);
        let expected = agent_context(&source, "agent");
        let serialized = serde_json::to_string(&source).unwrap();
        let state = ConversationState::new("restored", Default::default(), "agent");
        state
            .replace(serde_json::from_str(&serialized).unwrap())
            .await;
        let restored = state.list_recent(LedgerReadOptions::default()).await;
        assert_eq!(restored[0].record_id, 10);
        assert!(state.agent_record("agent", 1000).await.is_some());
        assert_eq!(
            serde_json::to_value(agent_context(&restored, "agent")).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn summary_input_preserves_old_summary_parameters_and_full_tool_output() {
        let mut source = records();
        source[5].role = LedgerRole::Summary;
        source[5].content = "prior decision must survive".into();
        let mut call = Message::assistant("");
        call.tool_calls = Some(serde_json::from_value(serde_json::json!([
            {"id":"call-7", "type":"function", "function":{"name":"run", "arguments":"session=123"}}
        ])).unwrap());
        source[6] = LedgerRecord::from_message(7, "test", "agent", "Agent", call, None);
        source[7] = LedgerRecord::from_message(
            8,
            "test",
            "agent",
            "Agent",
            Message::tool_with_id(
                format!("{}critical final error", "x".repeat(1000)),
                "call-7",
                "run",
            ),
            None,
        );
        let input = summary_input(&source.iter().collect::<Vec<_>>());
        assert!(input[1].content.contains("prior decision must survive"));
        assert!(input[1].content.contains("session=123"));
        assert!(input[1].content.contains("critical final error"));
        assert!(input[1].content.contains("record_id"));
    }

    #[test]
    fn batching_fits_budget_and_carries_previous_handoff() {
        let source = records();
        let refs: Vec<_> = source.iter().collect();
        let prior = "Earlier constraint and unfinished task";
        let (count, input) = summary_batch(&refs, prior, 2500).unwrap();
        assert!(count > 0 && count < refs.len());
        assert!(estimate_tokens(&input) <= 2500);
        assert!(input.iter().any(|message| message.content.contains(prior)));
        assert!(summary_batch(&refs, prior, 1).is_err());
    }

    #[test]
    fn truncated_model_completion_is_not_a_valid_summary() {
        let response: llm_gateway::LlmResponse = serde_json::from_value(serde_json::json!({
            "content": "plausible but truncated handoff", "finish_reason": "length", "tokens": null
        }))
        .unwrap();
        assert!(validated_summary(response).is_err());
    }

    #[test]
    fn failures_never_produce_a_checkpoint() {
        let source = records();
        let refs: Vec<_> = source.iter().collect();
        for content in ["", "   "] {
            assert!(
                finish_compaction(&refs, 4..60, 1, content.into(), usize::MAX, usize::MAX).is_err()
            );
        }
        assert!(finish_compaction(&refs, 4..60, 1, "summary".into(), 1, usize::MAX).is_err());
        assert!(finish_compaction(
            &refs,
            4..60,
            1,
            "summary".repeat(100000),
            usize::MAX,
            usize::MAX
        )
        .is_err());
    }

    #[test]
    fn long_single_turn_can_compact_but_never_splits_pending_calls() {
        let mut messages: Vec<_> = records()
            .iter()
            .filter_map(|r| r.to_context_message())
            .collect();
        messages[69] = Message::assistant("still working");
        assert!(compact_range(&messages, Some(0), usize::MAX).is_some());
        messages[3].provider_items = Some(vec![
            serde_json::json!({"type":"function_call", "call_id":"pending"}),
        ]);
        assert!(compact_range(&messages, Some(0), usize::MAX).is_none());
        messages[8] = Message::tool_with_id("done", "pending", "run");
        assert_eq!(
            compact_range(&messages, Some(0), usize::MAX).unwrap().start,
            9
        );
        messages[55].provider_items = Some(vec![
            serde_json::json!({"type":"function_call", "call_id":"running"}),
        ]);
        assert_eq!(
            compact_range(&messages, Some(0), usize::MAX).unwrap().end,
            55
        );
    }

    #[tokio::test]
    async fn new_conversation_with_no_compactable_range_is_not_a_compaction_failure() {
        let source = vec![LedgerRecord::from_message(
            1,
            "test",
            "agent",
            "Agent",
            Message::user("hello"),
            None,
        )];
        let refs = source.iter().collect::<Vec<_>>();
        let cache: Arc<dyn Cache> = Arc::new(corework::cache::InMemoryCache::new());

        let result = prepare_compaction(&refs, 1, &cache, 0, usize::MAX)
            .await
            .unwrap();

        assert!(result.is_none());
    }
}
