//! Conversation history compaction.
//! The ledger is the source of truth. This module only builds the LLM call view
//! for the current turn; it must not write conversation or compact history
//! snapshots into an agent-local cache.

use crate::context::{roles, Message};
use corework::cache::Cache;
use corework::error::Result;
use std::sync::Arc;

// ============================================================================
// Constants
// ============================================================================

/// Tokens reserved for the model output.
const OUTPUT_RESERVE_TOKENS: u32 = 4000;

/// Fallback context window when the model is not registered.
const DEFAULT_CONTEXT_WINDOW: u32 = 8192;

/// Number of head messages kept around the summary.
const HEAD_KEEP: usize = 4;

/// Number of tail messages kept around the summary.
const TAIL_KEEP: usize = 20;

// ============================================================================
// Public API
// ============================================================================

/// Build the LLM call view for the current turn.
/// When the input fits the model context window this returns the full input.
/// When it is too long, the middle section is summarized and the returned view
/// is `head + summary + tail`.
pub async fn compress_for_llm_call(
    history: &[Message],
    model_uid: u32,
    cache: &Arc<dyn Cache>,
) -> Result<Vec<Message>> {
    let context_window = get_context_window(model_uid);
    let threshold = context_window.saturating_sub(OUTPUT_RESERVE_TOKENS);
    let current_tokens = estimate_tokens(history);

    if current_tokens <= threshold {
        let _ = cache;
        return Ok(history.to_vec());
    }

    return compact_with_summary(history, model_uid, cache).await;
}

pub fn needs_compaction(history: &[Message], model_uid: u32) -> bool {
    let threshold = get_context_window(model_uid).saturating_sub(OUTPUT_RESERVE_TOKENS);
    estimate_tokens(history) > threshold
}

/// Explicit compact request from the gateway.
/// This is intentionally not token-threshold driven: once the gateway accepts
/// an active compact request for a long conversation, history must receive a
/// summary marker so the next LLM call can use "latest summary + following
/// context".
pub async fn compact_history_now(
    history: &[Message],
    model_uid: u32,
    cache: &Arc<dyn Cache>,
) -> Result<Vec<Message>> {
    compact_with_summary(history, model_uid, cache).await
}

async fn compact_with_summary(
    history: &[Message],
    model_uid: u32,
    cache: &Arc<dyn Cache>,
) -> Result<Vec<Message>> {
    let context_window = get_context_window(model_uid);
    let current_tokens = estimate_tokens(history);
    let n = history.len();
    let head_keep = HEAD_KEEP.min(n);
    let tail_keep = TAIL_KEEP.min(n.saturating_sub(head_keep));
    let mid_start = head_keep;
    let mid_end = n.saturating_sub(tail_keep);

    if mid_start >= mid_end {
        tracing::warn!(
            "history compaction skipped: overlapping head/tail (n={}, head={}, tail={})",
            n,
            head_keep,
            tail_keep
        );
        return Ok(history.to_vec());
    }

    let middle = &history[mid_start..mid_end];
    let summary_uid = crate::config_resolver::resolve_summary_model_uid(cache, model_uid).await;
    if summary_uid != model_uid {
        tracing::debug!(
            "summary model uid={} (inference uid={})",
            summary_uid,
            model_uid
        );
    }

    let summary_text = match summarize_with_llm(middle, summary_uid).await {
        Ok(text) if !text.trim().is_empty() => {
            tracing::info!("LLM summary succeeded for {} messages", middle.len());
            text
        }
        Ok(_) => {
            tracing::warn!("LLM returned an empty summary; using local fallback");
            summarize_middle(middle, 200)
        }
        Err(e) => {
            tracing::warn!("LLM summary failed: {}; using local fallback", e);
            summarize_middle(middle, 200)
        }
    };

    let summary_msg = Message {
        role: roles::SUMMARY.to_string(),
        content: format!(
            "[history summary: compacted {} messages]\n{}",
            middle.len(),
            summary_text
        ),
        cache_control: false,
        tool_call_id: None,
        name: None,
        tool_calls: None,
        reasoning_content: None,
    };

    let mut compact: Vec<Message> = Vec::with_capacity(head_keep + 1 + tail_keep);
    compact.extend_from_slice(&history[..head_keep]);
    compact.push(summary_msg);
    compact.extend_from_slice(&history[mid_end..]);

    tracing::info!(
        "history compacted: {} messages -> {} messages (head={}, tail={}, context_window={}, tokens {} -> {})",
        n,
        compact.len(),
        head_keep,
        compact.len().saturating_sub(head_keep + 1),
        context_window,
        current_tokens,
        estimate_tokens(&compact),
    );

    Ok(compact)
}

// ============================================================================
// Helpers
// ============================================================================

fn estimate_tokens(messages: &[Message]) -> u32 {
    let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    (total_chars / 4) as u32
}

fn get_context_window(model_uid: u32) -> u32 {
    llm_gateway::key_store::get(model_uid)
        .map(|e| e.context_window)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

async fn summarize_with_llm(
    middle: &[Message],
    model_uid: u32,
) -> std::result::Result<String, String> {
    let system_msg = llm_gateway::ChatMessage {
        role: "system".to_string(),
        content: crate::prompt_assets::template("history_compact.md"),
        cache_control: false,
        tool_call_id: None,
        name: None,
        tool_calls: None,
        reasoning_content: None,
    };

    let mut msgs: Vec<llm_gateway::ChatMessage> = vec![system_msg];

    for m in middle {
        match m.role.as_str() {
            "user" | "assistant" => {
                if !m.content.trim().is_empty() {
                    msgs.push(llm_gateway::ChatMessage {
                        role: m.role.clone(),
                        content: m.content.clone(),
                        cache_control: false,
                        tool_call_id: None,
                        name: None,
                        tool_calls: None,
                        reasoning_content: None,
                    });
                }
            }
            "tool" => {
                let preview = &m.content[..m.content.len().min(300)];
                if !preview.trim().is_empty() {
                    msgs.push(llm_gateway::ChatMessage {
                        role: "user".to_string(),
                        content: format!("[tool result] {}", preview),
                        cache_control: false,
                        tool_call_id: None,
                        name: None,
                        tool_calls: None,
                        reasoning_content: None,
                    });
                }
            }
            "summary" | "compact_summary" => {}
            _ => {}
        }
    }

    msgs.push(llm_gateway::ChatMessage {
        role: "user".to_string(),
        content: crate::prompt_assets::template("history_compact_user.md")
            .trim()
            .to_string(),
        cache_control: false,
        tool_call_id: None,
        name: None,
        tool_calls: None,
        reasoning_content: None,
    });
    llm_gateway::call_llm(model_uid, &msgs, None, None, None)
        .await
        .map(|resp| resp.content)
        .map_err(|e| e.to_string())
}

fn summarize_middle(messages: &[Message], max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "user" => {
                let content = truncate(&msg.content, max_chars);
                lines.push(format!("user: {}", content));
            }
            "assistant" => {
                if let Some(ref tcs) = msg.tool_calls {
                    let names: Vec<&str> = tcs.iter().map(|tc| tc.function.name.as_str()).collect();
                    lines.push(format!("assistant tool calls: {}", names.join(", ")));
                } else if !msg.content.is_empty() {
                    let content = truncate(&msg.content, max_chars);
                    lines.push(format!("assistant: {}", content));
                }
            }
            "tool" => {
                let preview = truncate(&msg.content, 60);
                lines.push(format!("tool result: {}", preview));
            }
            _ => {}
        }
    }

    if lines.is_empty() {
        "(no effective content)".to_string()
    } else {
        lines.join("\n")
    }
}

fn truncate(s: &str, max_chars: usize) -> &str {
    if s.chars().count() <= max_chars {
        s
    } else {
        s.char_indices()
            .nth(max_chars)
            .map(|(i, _)| &s[..i])
            .unwrap_or(s)
    }
}

// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msgs(n: usize) -> Vec<Message> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    Message::user(&format!("user message {}", i))
                } else {
                    Message::assistant(&format!("assistant message {}", i))
                }
            })
            .collect()
    }

    #[test]
    fn test_estimate_tokens() {
        let msgs = make_msgs(4); // each msg ~10 chars
        let tokens = estimate_tokens(&msgs);
        assert!(tokens > 0);
    }

    #[test]
    fn test_summarize_middle_local() {
        let msgs = make_msgs(10);
        let summary = summarize_middle(&msgs, 50);
        assert!(!summary.is_empty());
        assert!(summary.contains("user:"));
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn test_get_context_window_default() {
        let cw = get_context_window(99999);
        assert_eq!(cw, DEFAULT_CONTEXT_WINDOW);
    }
}
