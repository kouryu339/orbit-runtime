//! Thinking state: builds the LLM request view, handles retries, and commits
//! validated assistant/tool protocol output back to the conversation ledger.

use crate::context::{keys, AssistantContext};
use crate::decision_line::parse_line_protocol;
use crate::events::{
    types as ev_types, InterruptedPayload, LlmErrorPayload, LlmUsagePayload, ThinkingDonePayload,
    TurnStartPayload,
};
use crate::skills::systems::mgr;
use crate::systems::prompt::{
    build_env_context_section, build_system_prompt_text, format_immutable_cache_entries_section,
    format_tools_section, format_workflows_section, select_history,
};
use corework::cache::{Cache, CacheExt};
use corework::event::{BaseEvent, EventBus};
use corework::execution_unit::ExecutionUnit;
use corework::statemachine::{FnState, SimpleTransition};
use corework::system::SystemOperation;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{events, states};

const DEFAULT_MAX_THINKING_ROUNDS: u32 = 0;
const DEFAULT_AUTO_CONTINUE_MAX_STEPS: u32 = 20;

const DEFAULT_MAX_HISTORY_MESSAGES: usize = 200;

// ============================================================================
//
//
// ============================================================================

static SYNC_STREAM_TX: std::sync::OnceLock<
    std::sync::Mutex<Option<tokio::sync::mpsc::Sender<String>>>,
> = std::sync::OnceLock::new();

fn sync_stream_tx() -> &'static std::sync::Mutex<Option<tokio::sync::mpsc::Sender<String>>> {
    SYNC_STREAM_TX.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn set_stream_sender(tx: Option<tokio::sync::mpsc::Sender<String>>) {
    if let Ok(mut guard) = sync_stream_tx().lock() {
        *guard = tx;
    }
}

// ============================================================================
//
// ============================================================================

static CANCEL_TOKEN: std::sync::OnceLock<std::sync::Mutex<tokio_util::sync::CancellationToken>> =
    std::sync::OnceLock::new();

fn cancel_token_store() -> &'static std::sync::Mutex<tokio_util::sync::CancellationToken> {
    CANCEL_TOKEN.get_or_init(|| std::sync::Mutex::new(tokio_util::sync::CancellationToken::new()))
}

pub fn request_cancel() {
    if let Ok(guard) = cancel_token_store().lock() {
        guard.cancel();
    }
}

fn take_cancel_token() -> tokio_util::sync::CancellationToken {
    if let Ok(guard) = cancel_token_store().lock() {
        guard.clone()
    } else {
        tokio_util::sync::CancellationToken::new()
    }
}

fn reset_cancel_token() {
    if let Ok(mut guard) = cancel_token_store().lock() {
        *guard = tokio_util::sync::CancellationToken::new();
    }
}

fn render_active_skills_message(active_prompt: &str) -> String {
    if active_prompt.trim().is_empty() {
        return String::new();
    }
    crate::prompt_assets::template("active_skills_message.md")
        .replace("{{ACTIVE_SKILLS_PROMPT}}", active_prompt)
        .trim()
        .to_string()
}

fn render_recording_chain_section(chain: &str) -> String {
    if chain.trim().is_empty() {
        crate::prompt_assets::template("recording_chain_empty.md")
            .trim()
            .to_string()
    } else {
        crate::prompt_assets::template("recording_chain.md")
            .replace("{{CHAIN}}", chain)
            .trim()
            .to_string()
    }
}

fn render_current_plan_section(
    title: &str,
    summary: &str,
    status: &str,
    updated_at: &str,
    content: &str,
) -> String {
    crate::prompt_assets::template("current_plan.md")
        .replace("{{TITLE}}", title)
        .replace("{{SUMMARY}}", summary)
        .replace("{{STATUS}}", status)
        .replace("{{UPDATED_AT}}", updated_at)
        .replace("{{CONTENT}}", content)
        .trim()
        .to_string()
}

fn render_dynamic_context_message(content: &str) -> String {
    let template = crate::prompt_assets::template("dynamic_context.md");
    let rendered = if template.trim().is_empty() {
        format!(
            "The following dynamic context is fresh runtime information for answering the current user request. It is not a new user request.\n\n{}",
            content
        )
    } else {
        template.replace("{{CONTENT}}", content)
    };
    rendered.trim().to_string()
}

fn leading_system_message(content: String, prompt_cache_control: bool) -> llm_gateway::ChatMessage {
    if prompt_cache_control {
        llm_gateway::ChatMessage::system_cached(content)
    } else {
        llm_gateway::ChatMessage::system(content)
    }
}

fn mark_conversation_history_cache_boundary(
    messages: &mut [llm_gateway::ChatMessage],
    prompt_cache_control: bool,
) {
    if !prompt_cache_control {
        return;
    }
    if let Some(last_history_message) = messages.iter_mut().skip(1).last() {
        last_history_message.cache_control = true;
    }
}

fn llm_trace_enabled() -> bool {
    matches!(
        std::env::var("RUNTIME_LLM_TRACE").ok().as_deref(),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

fn llm_trace_file() -> String {
    std::env::var("RUNTIME_LLM_TRACE_FILE")
        .unwrap_or_else(|_| "/tmp/agent-runtime-llm-trace.jsonl".to_string())
}

async fn append_llm_trace<T: Serialize>(
    kind: &str,
    conversation_id: &str,
    turn_id: u64,
    attempt: Option<u32>,
    payload: &T,
) {
    if !llm_trace_enabled() {
        return;
    }

    let record = serde_json::json!({
        "schema": "agent-runtime-llm-trace/v1",
        "kind": kind,
        "conversation_id": conversation_id,
        "turn_id": turn_id,
        "attempt": attempt,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "payload": payload,
    });
    let line = match serde_json::to_string(&record) {
        Ok(line) => line,
        Err(err) => {
            tracing::warn!(
                target: "ai_assistant::llm_trace",
                conversation_id = %conversation_id,
                turn_id = turn_id,
                attempt = ?attempt,
                error = %err,
                "failed to serialize llm trace record"
            );
            return;
        }
    };

    tracing::info!(
        target: "ai_assistant::llm_trace",
        conversation_id = %conversation_id,
        turn_id = turn_id,
        attempt = ?attempt,
        kind = kind,
        "{}",
        line
    );

    let path = llm_trace_file();
    let mut line_with_newline = line;
    line_with_newline.push('\n');
    if let Err(err) = async {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        file.write_all(line_with_newline.as_bytes()).await
    }
    .await
    {
        tracing::warn!(
            target: "ai_assistant::llm_trace",
            conversation_id = %conversation_id,
            turn_id = turn_id,
            attempt = ?attempt,
            path = %path,
            error = %err,
            "failed to append llm trace file"
        );
    }
}

async fn trace_llm_request(
    conversation_id: &str,
    turn_id: u64,
    attempt: u32,
    model_uid: u32,
    model_name: &str,
    messages: &[llm_gateway::ChatMessage],
    tool_count: usize,
) {
    append_llm_trace(
        "llm_request",
        conversation_id,
        turn_id,
        Some(attempt),
        &serde_json::json!({
            "model_uid": model_uid,
            "model_name": model_name,
            "tool_count": tool_count,
            "message_count": messages.len(),
            "messages": messages,
        }),
    )
    .await;
}

async fn trace_llm_response(
    conversation_id: &str,
    turn_id: u64,
    attempt: u32,
    response: &llm_gateway::LlmResponse,
) {
    append_llm_trace(
        "llm_response",
        conversation_id,
        turn_id,
        Some(attempt),
        response,
    )
    .await;
}

async fn trace_llm_error(
    conversation_id: &str,
    turn_id: u64,
    attempt: u32,
    error: &str,
    fatal: bool,
) {
    append_llm_trace(
        "llm_error",
        conversation_id,
        turn_id,
        Some(attempt),
        &serde_json::json!({
            "error": error,
            "fatal": fatal,
        }),
    )
    .await;
}

fn emit_stream_reset(sm_ctx: &Arc<ExecutionUnit>) {
    // A retry or correction replaces the visible assistant stream, so clients
    // must clear partial tokens before the next attempt starts.
    let event = BaseEvent::new(ev_types::STREAM_RESET, serde_json::json!({}));
    let bus = sm_ctx.event_bus();
    tokio::spawn(async move {
        let _ = bus.publish(event).await;
    });
}

pub fn build() -> FnState {
    FnState::new(states::THINKING)
        .with_description("prepare and call the LLM")
        .with_on_enter(|ctx| Box::pin(on_enter(ctx)))
        .with_on_transition(|ctx| Box::pin(on_transition(ctx)))
        .add_transition(
            events::PAUSE,
            Box::new(SimpleTransition::new(events::PAUSE, states::SUSPENDED)),
        )
}

async fn publish_new_summary_records(
    history: &[crate::context::Message],
    compact: &[crate::context::Message],
    cache: &Arc<dyn Cache>,
    event_bus: &Arc<dyn EventBus>,
) -> corework::error::Result<()> {
    let existing_summaries = history
        .iter()
        .filter(|message| {
            message.role == crate::context::roles::SUMMARY
                || message.role == crate::context::roles::COMPACT_SUMMARY
        })
        .map(|message| message.content.as_str())
        .collect::<std::collections::HashSet<_>>();

    for summary in compact.iter().filter(|message| {
        message.role == crate::context::roles::SUMMARY
            || message.role == crate::context::roles::COMPACT_SUMMARY
    }) {
        if existing_summaries.contains(summary.content.as_str()) {
            continue;
        }

        let (agent_id, agent_name) = crate::agent::source_meta_from_cache(&**cache).await;
        event_bus
            .publish(BaseEvent::new(
                crate::events::types::AGENT_MESSAGE_PRODUCED,
                serde_json::to_value(crate::events::AgentMessageProducedPayload {
                    conversation_id: crate::ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id,
                    agent_name,
                    role: crate::ledger::LedgerRole::Summary,
                    content: summary.content.clone(),
                    metadata: crate::ledger::LedgerMessageMeta::default(),
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                })?,
            ))
            .await?;
    }
    Ok(())
}

// ============================================================================
// ============================================================================

async fn on_enter(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<()> {
    let cache = sm_ctx.cache();
    let conversation_id = crate::agent::conversation_id_from_cache(&*cache)
        .await
        .ok_or_else(|| {
            corework::error::FrameworkError::SystemError(
                "thinking requires conversation_id in agent cache".to_string(),
            )
        })?;
    let cache_scope_id = sm_ctx.cache_scope_id().to_string();
    let event_bus = sm_ctx.event_bus();

    let turn_id = AssistantContext::bump_turn_id(&cache).await?;
    tracing::info!(
        conversation_id = %conversation_id,
        cache_scope_id = %cache_scope_id,
        turn_id = turn_id,
        "thinking on_enter start"
    );
    {
        let src = crate::agent::source_id_from_cache(&*cache).await;
        crate::agent::publish_user_facing(
            &*event_bus,
            ev_types::TURN_START,
            serde_json::to_value(&TurnStartPayload { turn_id }).unwrap_or(serde_json::json!({})),
            &src,
        )
        .await;
    }
    tracing::info!(
        conversation_id = %conversation_id,
        turn_id = turn_id,
        "thinking turn_start published"
    );
    crate::agent::publish_focus_status_for_cache(
        sm_ctx.as_ref(),
        &*cache,
        &*event_bus,
        states::THINKING,
    )
    .await;

    if super::consume_pause_if_requested(&cache).await? {
        cache
            .set(keys::NEXT_STATE, &states::SUSPENDED.to_string(), None)
            .await?;
        return Ok(());
    }

    // Build the message window sent to the model.
    let round: u32 = cache.get(keys::THINKING_ROUND_COUNT).await?.unwrap_or(0) + 1;
    cache.set(keys::THINKING_ROUND_COUNT, &round, None).await?;
    tracing::info!(
        conversation_id = %conversation_id,
        turn_id = turn_id,
        round = round,
        "thinking round counter updated"
    );

    let max_rounds: u32 = cache
        .get(keys::MAX_THINKING_ROUNDS)
        .await?
        .unwrap_or(DEFAULT_MAX_THINKING_ROUNDS);

    if max_rounds > 0 && round > max_rounds {
        tracing::warn!(
            "thinking exceeded max rounds {}; falling back to result",
            max_rounds
        );
        let fallback = crate::prompt_assets::template("max_rounds_fallback.md")
            .trim()
            .to_string();
        set_result_next_state(&cache, &fallback).await?;
        return Ok(());
    }

    tracing::debug!(
        "thinking round {} (limit: {})",
        round,
        if max_rounds == 0 {
            "unlimited".to_string()
        } else {
            max_rounds.to_string()
        }
    );

    let recorder_active: bool = cache.get("recorder_active").await?.unwrap_or(false);
    let agent_id = crate::agent::source_id_from_cache(&*cache).await;

    let (persona_text, active_prompt, skill_workflows) = {
        let main_names: Vec<String> = cache.get(keys::MAIN_SKILLS).await?.unwrap_or_default();
        let imported_names = AssistantContext::get_imported_skills(&cache).await?;

        let mut all_names = main_names.clone();
        for n in &imported_names {
            if !all_names.contains(n) {
                all_names.push(n.clone());
            }
        }
        let all_refs: Vec<&str> = all_names.iter().map(|s| s.as_str()).collect();

        if !all_refs.is_empty() {
            let mut m = mgr().write().await;
            if let Err(e) = m.load_many(&all_refs).await {
                tracing::warn!("failed to load skills: {}", e);
            }
        }

        let m = mgr().read().await;

        let persona_body = {
            let main_refs: Vec<&str> = main_names.iter().map(|s| s.as_str()).collect();
            let body = main_refs
                .iter()
                .filter_map(|n| m.get(n))
                .filter(|s| s.metadata.is_role())
                .map(|s| s.instructions.as_str())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            body
        };

        let imported_refs: Vec<&str> = imported_names.iter().map(|s| s.as_str()).collect();
        let active = if imported_refs.is_empty() {
            String::new()
        } else {
            m.active_skills_prompt(&imported_refs)
        };

        let skill_workflows = m.collect_workflows_for_skills(&all_refs);

        (persona_body, active, skill_workflows)
    };

    let all_tools = {
        let mut tools = AssistantContext::all_active_tools(&cache).await?;
        let m = mgr().read().await;
        tools = m.filtered_tools_for_state(states::THINKING, tools);
        tools
    };
    let tools_section = format_tools_section(&all_tools);

    tracing::debug!("built {} tool descriptions", all_tools.len());

    // Host-published dynamic text fields for the current agent.
    let dynamic_snapshots = sm_ctx
        .resolve_shared_component::<crate::conversation_state::ConversationState>()
        .ok_or_else(|| {
            corework::error::FrameworkError::InvalidOperation(
                "conversation state is unavailable from the agent hierarchy".to_string(),
            )
        })?
        .dynamic_snapshots(&agent_id)
        .await;
    let combined_structures_section =
        crate::systems::prompt::format_host_dynamic_snapshots_section(&dynamic_snapshots);
    log_host_dynamic_snapshot_probe(&dynamic_snapshots, &agent_id, turn_id, round);

    let workflows_section = {
        let world = sm_ctx.world();
        let registry: Vec<serde_json::Value> = world
            .get_resource("wf:registry")
            .unwrap_or(None)
            .unwrap_or_default();

        if skill_workflows.is_empty() {
            format_workflows_section(&registry)
        } else {
            let filtered: Vec<serde_json::Value> = registry
                .into_iter()
                .filter(|e| {
                    let name = e
                        .get("metadata")
                        .and_then(|m| m.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    skill_workflows.iter().any(|wf| wf == name)
                })
                .collect();
            format_workflows_section(&filtered)
        }
    };

    let model_uid = crate::config_resolver::resolve_inference_model_uid(&cache)
        .await
        .ok_or_else(|| {
            tracing::error!(
                conversation_id = %conversation_id,
                turn_id,
                "no inference model resolved"
            );
            corework::error::FrameworkError::SystemError(
                "no available inference model configured".to_string(),
            )
        })?;
    let model_entry = llm_gateway::key_store::get(model_uid);
    let prompt_cache_control = model_entry
        .as_ref()
        .and_then(|entry| llm_gateway::key_store::resolve_provider_runtime(entry.provider_uid))
        .map(|provider| provider.prompt_cache_control)
        .unwrap_or(false);
    tracing::info!(
        conversation_id = %conversation_id,
        turn_id = turn_id,
        model_uid = model_uid,
        model_name = model_entry
            .as_ref()
            .map(|entry| entry.model_name.as_str())
            .unwrap_or("<missing>"),
        provider_uid = model_entry.as_ref().map(|entry| entry.provider_uid).unwrap_or(0),
        "thinking model resolved"
    );

    let immutable_role_appendix = cache
        .get::<String>(keys::IMMUTABLE_ROLE_APPENDIX)
        .await?
        .unwrap_or_default();
    let immutable_cache_entries = AssistantContext::get_immutable_cache_entries(&cache).await?;
    let persona = if persona_text.trim().is_empty() {
        return Err(corework::error::FrameworkError::InvalidOperation(
            "agent is missing a role skill; cannot build persona".to_string(),
        ));
    } else {
        compose_cached_persona_context(
            &persona_text,
            &immutable_role_appendix,
            &immutable_cache_entries,
        )
    };

    let frontend_widgets_enabled = cache
        .get(keys::FRONTEND_WIDGETS_ENABLED)
        .await?
        .unwrap_or(true);
    let mut system_text = build_system_prompt_text(
        &persona,
        "",
        &render_active_skills_message(&active_prompt),
        &tools_section,
        &workflows_section,
        "",
        states::THINKING,
        frontend_widgets_enabled,
    );

    #[cfg(debug_assertions)]
    {
        use std::time::{SystemTime, UNIX_EPOCH};

        let temp_dir = std::env::temp_dir().join("ai-assistant-prompt");
        let _ = tokio::fs::create_dir_all(&temp_dir).await;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let filename = if recorder_active {
            format!("prompt_{}_recorder.md", timestamp)
        } else {
            format!("prompt_{}.md", timestamp)
        };
        let file_path = temp_dir.join(&filename);

        if let Err(e) = tokio::fs::write(&file_path, &system_text).await {
            tracing::warn!("failed to write prompt debug file: {}", e);
        }
    }

    let ctx = sm_ctx.create_context();
    let history = crate::systems::ledger::QueryAgentLlmExecutionSnapshotSystem
        .execute(
            crate::systems::ledger::QueryAgentContextInput {
                conversation_id: conversation_id.clone(),
                agent_id: agent_id.clone(),
            },
            &ctx,
        )
        .await
        .map(|snapshot| snapshot.messages)
        .map_err(|e| corework::error::FrameworkError::SystemError(e.to_string()))?;

    let compact = if crate::systems::history::needs_compaction(&history, model_uid) {
        cache.set(keys::COMPACT_IN_PROGRESS, &true, None).await?;
        sm_ctx
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::CONVERSATION_STATE_CHANGED,
                serde_json::json!({}),
            ))
            .await?;
        let compact_result =
            crate::systems::history::compress_for_llm_call(&history, model_uid, &cache).await;
        cache.set(keys::COMPACT_IN_PROGRESS, &false, None).await?;
        sm_ctx
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::CONVERSATION_STATE_CHANGED,
                serde_json::json!({}),
            ))
            .await?;
        compact_result?
    } else {
        history.clone()
    };
    publish_new_summary_records(&history, &compact, &cache, &sm_ctx.event_bus()).await?;

    let max_msgs: usize = cache
        .get(keys::MAX_HISTORY_MESSAGES)
        .await?
        .unwrap_or(DEFAULT_MAX_HISTORY_MESSAGES);
    let (selected_slice, _truncated) = select_history(&compact, max_msgs);
    let selected = selected_slice.to_vec();
    log_thinking_context_probe(&history, &compact, &selected, &agent_id, turn_id, round);

    tracing::debug!(
        "message selection: history={}, selected={}, truncated={}, limit={}",
        history.len(),
        selected.len(),
        _truncated,
        max_msgs
    );

    let legacy_system_context = selected
        .iter()
        .filter(|msg| msg.role == "system" && !msg.content.trim().is_empty())
        .map(|msg| msg.content.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !legacy_system_context.is_empty() {
        system_text.push_str("\n\n---\n\n## Legacy System Context\n");
        system_text.push_str(&legacy_system_context);
    }

    let mut messages: Vec<llm_gateway::ChatMessage> = Vec::with_capacity(5 + selected.len());
    messages.push(leading_system_message(system_text, prompt_cache_control));

    let retrieval_query = selected
        .iter()
        .rev()
        .find(|msg| msg.role == "user" && !msg.content.trim().is_empty())
        .map(|msg| msg.content.clone());

    for msg in selected {
        if matches!(
            msg.role.as_str(),
            "display" | "thinking_step" | "tool_step" | "interrupted" | "llm_error"
        ) {
            continue;
        }
        if msg.role == crate::context::roles::SUMMARY
            || msg.role == crate::context::roles::COMPACT_SUMMARY
        {
            messages.push(llm_gateway::ChatMessage::user(
                crate::prompt_assets::render(
                    "conversation_summary_context.md",
                    &[("{{CONTENT}}", &msg.content)],
                ),
            ));
            continue;
        }

        match msg.role.as_str() {
            "tool" => {
                let tool_prefix = crate::prompt_assets::template("tool_execution_result.md")
                    .split("{{COMMAND}}")
                    .next()
                    .unwrap_or("Tool execution:")
                    .trim_start()
                    .to_string();
                let content = if msg.content.trim_start().starts_with(&tool_prefix)
                    || msg.content.trim_start().starts_with("Tool execution:")
                {
                    msg.content.clone()
                } else {
                    crate::prompt_assets::render(
                        "tool_result_context.md",
                        &[("{{CONTENT}}", &msg.content)],
                    )
                };
                messages.push(llm_gateway::ChatMessage::user(content));
            }
            "assistant" => {
                let content = if msg.content.trim().is_empty() {
                    summarize_legacy_tool_calls(&msg)
                } else {
                    msg.content.clone()
                };
                if !content.trim().is_empty() {
                    messages.push(llm_gateway::ChatMessage::assistant(content));
                } else {
                    tracing::warn!(
                        conversation_id = %conversation_id,
                        turn_id = turn_id,
                        role = %msg.role,
                        "skipping empty assistant message while building thinking LLM view"
                    );
                }
            }
            "user" => {
                if !msg.content.trim().is_empty() {
                    messages.push(llm_gateway::ChatMessage::user(msg.content.clone()));
                }
            }
            "agent_report" => {
                if !msg.content.trim().is_empty() {
                    messages.push(llm_gateway::ChatMessage::user(msg.content.clone()));
                }
            }
            "summary" => {
                messages.push(llm_gateway::ChatMessage::user(
                    crate::prompt_assets::render(
                        "conversation_summary_context.md",
                        &[("{{CONTENT}}", &msg.content)],
                    ),
                ));
            }
            "system" => {
                // Legacy system records are merged into the leading system message above.
            }
            _ => {}
        }
    }

    mark_conversation_history_cache_boundary(&mut messages, prompt_cache_control);

    let mut dynamic_context = Vec::new();
    if !combined_structures_section.is_empty() {
        dynamic_context.push(combined_structures_section.clone());
    }
    if recorder_active {
        let chain = cache
            .get::<String>(keys::RECORDER_CHAIN)
            .await?
            .unwrap_or_default();
        dynamic_context.push(render_recording_chain_section(&chain));
    }
    if let Some(plan) = AssistantContext::get_current_plan(&cache).await? {
        if plan.is_active() {
            dynamic_context.push(render_current_plan_section(
                &plan.title,
                if plan.summary.is_empty() {
                    "(none)"
                } else {
                    &plan.summary
                },
                &plan.status,
                &plan.updated_at,
                &plan.content,
            ));
        }
    }
    if let Some(user_msg) = retrieval_query.as_ref() {
        let retrieval_config: crate::RetrievalConfig =
            cache.get(keys::RETRIEVAL_CONFIG).await?.unwrap_or_default();
        if retrieval_config.before_thinking_enabled() {
            if let Some(retrieval_context) = crate::retrieval::retrieve_before_thinking(
                &ctx,
                &cache,
                &retrieval_config,
                user_msg,
                turn_id,
            )
            .await?
            {
                dynamic_context.push(retrieval_context);
            }
        }
    }
    dynamic_context.push(build_env_context_section());
    dynamic_context.push(crate::prompt_assets::render(
        "runtime_context.md",
        &[("{{OS}}", std::env::consts::OS)],
    ));

    if !dynamic_context.is_empty() {
        let content = dynamic_context.join("\n\n---\n\n");
        messages.push(llm_gateway::ChatMessage::user(
            render_dynamic_context_message(&content),
        ));
    }
    log_llm_messages_probe(&messages, &agent_id, turn_id, round);

    tracing::info!(
        conversation_id = %conversation_id,
        turn_id = turn_id,
        model_uid = model_uid,
        message_count = messages.len(),
        tool_count = all_tools.len(),
        "thinking llm call prepared"
    );

    // Invalid protocol output is rejected here so it cannot enter the canonical ledger.
    let mut llm_response: Option<llm_gateway::LlmResponse> = None;
    let mut last_err = String::new();
    let mut last_retryable_status: Option<u16> = None;

    for attempt in 1..=3u32 {
        emit_stream_reset(&sm_ctx);
        reset_cancel_token();
        let cancel = take_cancel_token();
        let model_name = model_entry
            .as_ref()
            .map(|entry| entry.model_name.as_str())
            .unwrap_or("<missing>");
        tracing::info!(
            conversation_id = %conversation_id,
            turn_id = turn_id,
            model_uid = model_uid,
            model_name = model_name,
            attempt = attempt,
            "thinking llm call start"
        );
        trace_llm_request(
            &conversation_id,
            turn_id,
            attempt,
            model_uid,
            model_name,
            &messages,
            all_tools.len(),
        )
        .await;

        let request_headers = sm_ctx
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                corework::error::FrameworkError::InvalidOperation(
                    "conversation state is unavailable from the agent hierarchy".to_string(),
                )
            })?
            .request_headers()
            .await;
        let call_result = llm_gateway::request_context::scope_request_headers(
            request_headers.headers,
            request_headers.allow_insecure,
            llm_gateway::call_llm_cancellable(model_uid, &messages, None, None, None, cancel),
        )
        .await;

        match call_result {
            Ok(mut resp) => {
                resp.content = crate::runtime::parser::normalize_response_for_ledger(&resp.content);
                trace_llm_response(&conversation_id, turn_id, attempt, &resp).await;
                tracing::info!(
                    conversation_id = %conversation_id,
                    turn_id = turn_id,
                    attempt = attempt,
                    content_len = resp.content.len(),
                    tool_call_count = resp.tool_calls.as_ref().map(|calls| calls.len()).unwrap_or(0),
                    "thinking llm call ok"
                );
                let content_empty = resp.content.trim().is_empty();
                if content_empty {
                    last_err = format!(
                        "attempt {} returned an empty response without content or tool calls",
                        attempt
                    );
                    tracing::warn!(
                        conversation_id = %conversation_id,
                        turn_id,
                        attempt,
                        "llm returned empty response; retrying"
                    );
                    if attempt < 3 {
                        emit_stream_reset(&sm_ctx);
                        let retry_prompt =
                            crate::prompt_assets::template("retry_empty_response.md")
                                .trim()
                                .to_string();
                        tracing::warn!(
                            conversation_id = %conversation_id,
                            turn_id = turn_id,
                            attempt = attempt,
                            prompt_name = "retry_empty_response.md",
                            prompts_dir = %crate::prompt_assets::prompts_dir().display(),
                            prompt_language = %crate::prompt_assets::language(),
                            retry_prompt_len = retry_prompt.len(),
                            retry_prompt_empty = retry_prompt.trim().is_empty(),
                            messages_len_before_retry = messages.len(),
                            "injecting retry prompt after empty assistant response"
                        );
                        messages.push(llm_gateway::ChatMessage::user(retry_prompt));
                    }
                    continue;
                }

                if let Err(reason) = crate::runtime::parser::validate_response_shape(&resp.content)
                {
                    last_err = format!("invalid assistant response protocol: {reason}");
                    tracing::warn!(
                        conversation_id = %conversation_id,
                        turn_id = turn_id,
                        attempt = attempt,
                        reason = %reason,
                        "LLM response rejected before ledger write; retrying with protocol correction"
                    );
                    if attempt < 3 {
                        emit_stream_reset(&sm_ctx);
                        let correction = crate::prompt_assets::render(
                            "retry_invalid_response.md",
                            &[("{{REASON}}", &reason)],
                        );
                        tracing::warn!(
                            conversation_id = %conversation_id,
                            turn_id = turn_id,
                            attempt = attempt,
                            prompt_name = "retry_invalid_response.md",
                            prompts_dir = %crate::prompt_assets::prompts_dir().display(),
                            prompt_language = %crate::prompt_assets::language(),
                            reject_reason_len = reason.len(),
                            correction_len = correction.len(),
                            correction_empty = correction.trim().is_empty(),
                            rejected_assistant_len = resp.content.len(),
                            rejected_assistant_empty = resp.content.trim().is_empty(),
                            messages_len_before_retry = messages.len(),
                            "injecting retry prompt after invalid assistant response"
                        );
                        messages.push(llm_gateway::ChatMessage::assistant(resp.content.clone()));
                        messages.push(llm_gateway::ChatMessage::user(correction));
                    }
                    continue;
                }

                llm_response = Some(resp);
                break;
            }
            Err(e) => {
                let error_text = e.to_string();
                let fatal = e.is_fatal();
                trace_llm_error(&conversation_id, turn_id, attempt, &error_text, fatal).await;
                tracing::warn!(
                    conversation_id = %conversation_id,
                    turn_id = turn_id,
                    attempt = attempt,
                    error = %error_text,
                    fatal = fatal,
                    "thinking llm call failed"
                );
                if matches!(e, llm_gateway::error::ApiError::Cancelled) {
                    tracing::info!(
                        conversation_id = %conversation_id,
                        turn_id,
                        attempt,
                        "llm request cancelled; suspending thinking flow"
                    );
                    emit_stream_reset(&sm_ctx);
                    let event = BaseEvent::new(
                        ev_types::INTERRUPTED,
                        serde_json::to_value(&InterruptedPayload { turn_id })
                            .unwrap_or(serde_json::json!({})),
                    );
                    let bus = sm_ctx.event_bus();
                    tokio::spawn(async move {
                        let _ = bus.publish(event).await;
                    });
                    record_gateway_fact_for_cache(
                        &sm_ctx,
                        &cache,
                        crate::ledger::GATEWAY_SUBTYPE_INTERRUPTED,
                        "",
                        crate::ledger::LedgerMessageMeta {
                            extra: std::collections::BTreeMap::from([(
                                "turn_id".to_string(),
                                serde_json::json!(turn_id),
                            )]),
                            ..Default::default()
                        },
                    )
                    .await;
                    super::consume_pause_if_requested(&cache).await?;
                    cache
                        .set(keys::NEXT_STATE, &states::SUSPENDED.to_string(), None)
                        .await?;
                    return Ok(());
                }

                if fatal {
                    let user_msg = e.user_message();
                    let suggestion = e.suggestion();
                    let kind = e.kind_str().to_string();
                    let can_retry = e.retryable_by_user();
                    let (status, attempts) = match &e {
                        llm_gateway::error::ApiError::Fatal(f) => (f.status, f.attempts),
                        _ => (None, 0),
                    };

                    tracing::error!(
                        kind = %kind,
                        status = ?status,
                        attempts = attempts,
                        message_len = user_msg.len(),
                        "fatal llm error; stop retrying"
                    );

                    emit_stream_reset(&sm_ctx);
                    let agent_id = crate::agent::source_id_from_cache(&*cache).await;
                    let payload = LlmErrorPayload {
                        conversation_id: conversation_id.clone(),
                        agent_id: agent_id.clone(),
                        kind,
                        message: user_msg.clone(),
                        suggestion: suggestion.clone(),
                        can_retry,
                        attempts,
                        status,
                        turn_id,
                    };
                    record_gateway_fact_for_cache(
                        &sm_ctx,
                        &cache,
                        crate::ledger::GATEWAY_SUBTYPE_LLM_ERROR,
                        &user_msg,
                        crate::ledger::LedgerMessageMeta {
                            extra: std::collections::BTreeMap::from([
                                (
                                    "error".to_string(),
                                    serde_json::json!({
                                        "kind": payload.kind.clone(),
                                        "message": payload.message.clone(),
                                        "suggestion": payload.suggestion.clone(),
                                        "can_retry": payload.can_retry,
                                        "attempts": payload.attempts,
                                        "status": payload.status,
                                    }),
                                ),
                                ("kind".to_string(), serde_json::json!(payload.kind.clone())),
                                (
                                    "suggestion".to_string(),
                                    serde_json::json!(payload.suggestion.clone()),
                                ),
                                (
                                    "can_retry".to_string(),
                                    serde_json::json!(payload.can_retry),
                                ),
                                ("attempts".to_string(), serde_json::json!(payload.attempts)),
                                ("status".to_string(), serde_json::json!(payload.status)),
                                ("turn_id".to_string(), serde_json::json!(payload.turn_id)),
                            ]),
                            ..Default::default()
                        },
                    )
                    .await;
                    let payload_json =
                        serde_json::to_value(&payload).unwrap_or(serde_json::json!({}));
                    if let Err(error) = sm_ctx
                        .event_bus()
                        .publish(BaseEvent::new(ev_types::LLM_ERROR, payload_json))
                        .await
                    {
                        tracing::warn!(
                            conversation_id = %conversation_id,
                            turn_id,
                            event_type = %ev_types::LLM_ERROR,
                            error = %error,
                            "publish llm error event failed"
                        );
                    }
                    tracing::info!(
                        kind = %payload.kind,
                        turn_id = turn_id,
                        "fatal llm error event published"
                    );

                    let display = match suggestion {
                        Some(s) => format!("{}\n\nSuggestion: {}", user_msg, s),
                        None => user_msg.clone(),
                    };
                    set_result_next_state(&cache, &display).await?;
                    return Ok(());
                }

                if let llm_gateway::error::ApiError::Retryable { status, .. } = &e {
                    last_retryable_status = status.or(last_retryable_status);
                }
                last_err = error_text.clone();
                tracing::warn!(
                    conversation_id = %conversation_id,
                    turn_id = turn_id,
                    attempt = attempt,
                    error_len = error_text.len(),
                    "llm call failed; retry may follow"
                );
                if attempt < 3 {
                    emit_stream_reset(&sm_ctx);
                    let wait_ms = 500u64 * attempt as u64;
                    tracing::debug!(
                        conversation_id = %conversation_id,
                        turn_id,
                        attempt,
                        wait_ms,
                        "wait before llm retry"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                }
            }
        }
    }

    let llm_response = match llm_response {
        Some(r) => r,
        None => {
            let agent_id = crate::agent::source_id_from_cache(&*cache).await;
            let user_msg = if last_err.trim().is_empty() {
                "The model request failed after all retry attempts.".to_string()
            } else {
                last_err.clone()
            };
            let payload = LlmErrorPayload {
                conversation_id: conversation_id.clone(),
                agent_id: agent_id.clone(),
                kind: "retry_exhausted".into(),
                message: user_msg.clone(),
                suggestion: Some(
                    crate::prompt_assets::template("llm_retry_exhausted_suggestion.md")
                        .trim()
                        .to_string(),
                ),
                can_retry: true,
                attempts: 3,
                status: last_retryable_status,
                turn_id,
            };
            record_gateway_fact_for_cache(
                &sm_ctx,
                &cache,
                crate::ledger::GATEWAY_SUBTYPE_LLM_ERROR,
                &user_msg,
                crate::ledger::LedgerMessageMeta {
                    extra: std::collections::BTreeMap::from([
                        (
                            "error".to_string(),
                            serde_json::json!({
                                "kind": payload.kind.clone(),
                                "message": payload.message.clone(),
                                "suggestion": payload.suggestion.clone(),
                                "can_retry": payload.can_retry,
                                "attempts": payload.attempts,
                                "status": payload.status,
                            }),
                        ),
                        ("kind".to_string(), serde_json::json!(payload.kind.clone())),
                        (
                            "suggestion".to_string(),
                            serde_json::json!(payload.suggestion.clone()),
                        ),
                        (
                            "can_retry".to_string(),
                            serde_json::json!(payload.can_retry),
                        ),
                        ("attempts".to_string(), serde_json::json!(payload.attempts)),
                        ("turn_id".to_string(), serde_json::json!(payload.turn_id)),
                    ]),
                    ..Default::default()
                },
            )
            .await;
            let payload_json = serde_json::to_value(&payload).unwrap_or(serde_json::json!({}));
            if let Err(error) = sm_ctx
                .event_bus()
                .publish(BaseEvent::new(ev_types::LLM_ERROR, payload_json))
                .await
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    turn_id,
                    event_type = %ev_types::LLM_ERROR,
                    error = %error,
                    "publish llm error event failed"
                );
            }
            tracing::info!(
                conversation_id = %conversation_id,
                turn_id = turn_id,
                message_len = user_msg.len(),
                "llm retry exhausted event published"
            );
            cache
                .set(
                    keys::PENDING_RESULT,
                    &crate::prompt_assets::render(
                        "llm_retry_exhausted_display.md",
                        &[("{{MESSAGE}}", &user_msg)],
                    ),
                    None,
                )
                .await?;
            let display: String = cache.get(keys::PENDING_RESULT).await?.unwrap_or_default();
            set_result_next_state(&cache, &display).await?;
            return Ok(());
        }
    };

    publish_llm_usage_event(
        &sm_ctx,
        &conversation_id,
        turn_id,
        model_uid,
        model_entry.as_ref(),
        &llm_response,
        &cache,
    )
    .await;

    if super::consume_pause_if_requested(&cache).await? {
        cache
            .set(keys::NEXT_STATE, &states::SUSPENDED.to_string(), None)
            .await?;
        return Ok(());
    }

    let event_bus = sm_ctx.event_bus();
    let thinking_payload =
        build_thinking_payload_from_response(&cache, &event_bus, &llm_response, turn_id).await?;

    if let Ok(mut payload_json) = serde_json::to_value(&thinking_payload) {
        let src = crate::agent::source_id_from_cache(&*cache).await;
        if let Some(obj) = payload_json.as_object_mut() {
            obj.insert("agent_id".to_string(), serde_json::Value::String(src));
        }
        let event = BaseEvent::new(ev_types::THINKING_DONE, payload_json);
        let _ = sm_ctx.event_bus().publish(event).await;
    }
    Ok(())
}

async fn publish_llm_usage_event(
    sm_ctx: &Arc<ExecutionUnit>,
    conversation_id: &str,
    turn_id: u64,
    model_uid: u32,
    model_entry: Option<&llm_gateway::key_store::ModelEntry>,
    response: &llm_gateway::LlmResponse,
    cache: &Arc<dyn corework::cache::Cache>,
) {
    let Some(tokens) = response.tokens.as_ref() else {
        return;
    };

    let cached_input_tokens = response.cached_tokens.min(tokens.input_tokens);
    let uncached_input_tokens = tokens.input_tokens.saturating_sub(cached_input_tokens);
    let total_tokens = tokens.input_tokens.saturating_add(tokens.output_tokens);
    let total_billable_tokens = uncached_input_tokens.saturating_add(tokens.output_tokens);
    let payload = LlmUsagePayload {
        conversation_id: conversation_id.to_string(),
        agent_id: crate::agent::source_id_from_cache(&**cache).await,
        turn_id,
        model_uid,
        model: model_entry
            .map(|entry| entry.model_name.clone())
            .unwrap_or_else(|| "<missing>".to_string()),
        provider_uid: model_entry.map(|entry| entry.provider_uid).unwrap_or(0),
        input_tokens: tokens.input_tokens,
        cached_input_tokens,
        uncached_input_tokens,
        output_tokens: tokens.output_tokens,
        total_tokens,
        total_billable_tokens,
        cache_hit: cached_input_tokens > 0,
    };

    record_gateway_fact_for_cache(
        sm_ctx,
        cache,
        crate::ledger::GATEWAY_SUBTYPE_LLM_USAGE,
        "",
        crate::ledger::LedgerMessageMeta {
            extra: std::collections::BTreeMap::from([
                ("turn_id".to_string(), serde_json::json!(payload.turn_id)),
                (
                    "model_uid".to_string(),
                    serde_json::json!(payload.model_uid),
                ),
                (
                    "model".to_string(),
                    serde_json::json!(payload.model.clone()),
                ),
                (
                    "provider_uid".to_string(),
                    serde_json::json!(payload.provider_uid),
                ),
                (
                    "usage".to_string(),
                    serde_json::json!({
                        "input_tokens": payload.input_tokens,
                        "cached_input_tokens": payload.cached_input_tokens,
                        "uncached_input_tokens": payload.uncached_input_tokens,
                        "output_tokens": payload.output_tokens,
                        "total_tokens": payload.total_tokens,
                        "total_billable_tokens": payload.total_billable_tokens,
                        "cache_hit": payload.cache_hit,
                    }),
                ),
            ]),
            ..Default::default()
        },
    )
    .await;

    let payload_json = serde_json::to_value(payload).unwrap_or_else(|_| serde_json::json!({}));
    if let Err(error) = sm_ctx
        .event_bus()
        .publish(BaseEvent::new(ev_types::LLM_USAGE, payload_json))
        .await
    {
        tracing::warn!(
            conversation_id = %conversation_id,
            turn_id,
            event_type = %ev_types::LLM_USAGE,
            error = %error,
            "publish llm usage event failed"
        );
    }
}

fn summarize_legacy_tool_calls(msg: &crate::context::Message) -> String {
    let Some(tool_calls) = msg.tool_calls.as_ref().filter(|calls| !calls.is_empty()) else {
        return String::new();
    };
    let names = tool_calls
        .iter()
        .map(|tc| tc.function.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    crate::prompt_assets::render("legacy_tool_calls_summary.md", &[("{{NAMES}}", &names)])
}

async fn record_gateway_fact_for_cache(
    sm_ctx: &Arc<ExecutionUnit>,
    cache: &Arc<dyn corework::cache::Cache>,
    subtype: &str,
    content: &str,
    metadata: crate::ledger::LedgerMessageMeta,
) {
    let agent_id = crate::agent::source_id_from_cache(&**cache).await;
    let agent_name = cache
        .get::<String>(crate::state::agent_keys::AGENT_NAME)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| agent_id.clone());
    let event_bus = sm_ctx.event_bus();
    let mut metadata = metadata;
    metadata.subtype = Some(subtype.to_string());
    if let Err(e) = event_bus
        .publish(corework::event::BaseEvent::new(
            crate::events::types::AGENT_MESSAGE_PRODUCED,
            serde_json::to_value(crate::events::AgentMessageProducedPayload {
                conversation_id: crate::ledger::DEFAULT_CONVERSATION_ID.to_string(),
                agent_id: agent_id.clone(),
                agent_name,
                role: crate::ledger::LedgerRole::GatewayMessage,
                content: content.to_string(),
                metadata,
                display: None,
                tool_call_id: None,
                tool_name: None,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        ))
        .await
    {
        tracing::warn!(
            agent_id = %agent_id,
            subtype = %subtype,
            error = %e,
            "publish gateway fact event failed"
        );
    }
}

async fn build_thinking_payload_from_response(
    cache: &Arc<dyn corework::cache::Cache>,
    event_bus: &Arc<dyn EventBus>,
    response: &llm_gateway::LlmResponse,
    turn_id: u64,
) -> corework::error::Result<ThinkingDonePayload> {
    let reasoning_content = response.reasoning_content.clone();
    crate::runtime::parser::validate_response_shape(&response.content)
        .map_err(corework::error::FrameworkError::SystemError)?;
    let tool_kind = crate::runtime::parser::classify_tool_call(&response.content);
    match tool_kind {
        crate::runtime::parser::ToolCallKind::ToolOnly => {
            let tool_calls = crate::runtime::parser::parse_tool_calls(&response.content)
                .map_err(corework::error::FrameworkError::SystemError)?;
            return build_thinking_payload_from_parsed_tool_calls(
                cache,
                event_bus,
                &response.content,
                &tool_calls,
                reasoning_content,
                turn_id,
            )
            .await;
        }
        crate::runtime::parser::ToolCallKind::Mixed { .. } => {
            let tool_calls = crate::runtime::parser::parse_tool_calls(&response.content)
                .map_err(corework::error::FrameworkError::SystemError)?;
            return build_thinking_payload_from_parsed_tool_calls(
                cache,
                event_bus,
                &response.content,
                &tool_calls,
                reasoning_content,
                turn_id,
            )
            .await;
        }
        crate::runtime::parser::ToolCallKind::None => {}
    }

    build_thinking_payload_from_plain_content(
        cache,
        event_bus,
        &response.content,
        reasoning_content,
        turn_id,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopSignal {
    Waiting,
    Done,
}

fn parse_stop_signal(reply: &str) -> (String, Option<StopSignal>) {
    let mut lines = reply.lines().collect::<Vec<_>>();
    let Some(last_idx) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return (String::new(), None);
    };
    let signal = match lines[last_idx].trim() {
        "WAITING" => StopSignal::Waiting,
        "DONE" => StopSignal::Done,
        _ => return (reply.trim().to_string(), None),
    };
    lines.remove(last_idx);
    (lines.join("\n").trim().to_string(), Some(signal))
}

async fn reserve_auto_continue_step(
    cache: &Arc<dyn corework::cache::Cache>,
) -> corework::error::Result<bool> {
    let steps: u32 = cache.get(keys::AUTO_CONTINUE_STEPS).await?.unwrap_or(0);
    let max_steps: u32 = cache
        .get(keys::AUTO_CONTINUE_MAX_STEPS)
        .await?
        .unwrap_or(DEFAULT_AUTO_CONTINUE_MAX_STEPS);
    if steps >= max_steps {
        return Ok(false);
    }
    cache
        .set(keys::AUTO_CONTINUE_STEPS, &(steps + 1), None)
        .await?;
    Ok(true)
}

async fn stop_for_max_auto_steps(
    cache: &Arc<dyn corework::cache::Cache>,
    event_bus: &Arc<dyn EventBus>,
    turn_id: u64,
) -> corework::error::Result<ThinkingDonePayload> {
    let max_steps: u32 = cache
        .get(keys::AUTO_CONTINUE_MAX_STEPS)
        .await?
        .unwrap_or(DEFAULT_AUTO_CONTINUE_MAX_STEPS);
    let max_steps_text = max_steps.to_string();
    let reply =
        crate::prompt_assets::render("max_auto_steps.md", &[("{{MAX_STEPS}}", &max_steps_text)]);
    AssistantContext::push_assistant_message_on_event_bus(cache, event_bus, &reply).await?;
    cache.set(keys::PENDING_RESPONSE, &reply, None).await?;
    cache
        .set(keys::TASK_STATUS, &"waiting".to_string(), None)
        .await?;
    cache
        .set(keys::LAST_STOP_REASON, &"max_steps".to_string(), None)
        .await?;
    cache
        .set(
            keys::NEXT_STATE_AFTER_SAYING,
            &states::SUSPENDED.to_string(),
            None,
        )
        .await?;
    cache
        .set(keys::NEXT_STATE, &states::SAYING.to_string(), None)
        .await?;

    Ok(ThinkingDonePayload {
        reasoning: None,
        decision: "waiting".to_string(),
        tools: vec![],
        turn_id,
    })
}

async fn build_thinking_payload_from_parsed_tool_calls(
    cache: &Arc<dyn corework::cache::Cache>,
    event_bus: &Arc<dyn EventBus>,
    content: &str,
    parsed_tool_calls: &[crate::runtime::parser::ParsedToolCall],
    reasoning_content: Option<String>,
    turn_id: u64,
) -> corework::error::Result<ThinkingDonePayload> {
    let reasoning = parse_line_protocol(content).reasoning;
    if !reserve_auto_continue_step(cache).await? {
        return stop_for_max_auto_steps(cache, event_bus, turn_id).await;
    }
    let cli_cmds: Vec<String> = parsed_tool_calls
        .iter()
        .map(|tc| tc.to_legacy_command())
        .collect();
    let call_ids = parsed_tool_calls
        .iter()
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect::<Vec<_>>();
    let display_content = crate::runtime::parser::frontend_projection(content, &call_ids);
    let has_interactive_widget = crate::decision::contains_widget_tag(&display_content);
    let mut metadata = crate::ledger::LedgerMessageMeta::default();
    metadata.display_content = Some(display_content);
    metadata.extra.insert(
        "tool_call_ids".to_string(),
        serde_json::json!(call_ids.clone()),
    );
    metadata.extra.insert(
        "has_interactive_widget".to_string(),
        serde_json::json!(has_interactive_widget),
    );
    let assistant_msg = crate::context::Message {
        role: crate::context::roles::ASSISTANT.to_string(),
        content: content.to_string(),
        cache_control: false,
        tool_call_id: None,
        name: None,
        tool_calls: None,
        reasoning_content,
    };
    AssistantContext::push_message_with_metadata_and_display_on_event_bus(
        cache,
        event_bus,
        assistant_msg,
        metadata,
        None,
    )
    .await?;
    let display_cmds = crate::runtime::parser::display_exec_commands(content);
    cache.set(keys::PENDING_TOOLS, &cli_cmds, None).await?;
    cache
        .set(keys::PENDING_TOOL_DISPLAY_COMMANDS, &display_cmds, None)
        .await?;
    cache
        .set(keys::PENDING_TOOL_CALL_IDS, &call_ids, None)
        .await?;
    cache
        .set(
            keys::PENDING_TOOLS_WAIT_FOR_INPUT,
            &has_interactive_widget,
            None,
        )
        .await?;
    cache.delete(keys::PENDING_TOOL_CALLS).await?;
    cache
        .set(keys::NEXT_STATE, &states::EXECUTING.to_string(), None)
        .await?;

    Ok(ThinkingDonePayload {
        reasoning,
        decision: "executing".to_string(),
        tools: cli_cmds,
        turn_id,
    })
}

async fn build_thinking_payload_from_plain_content(
    cache: &Arc<dyn corework::cache::Cache>,
    event_bus: &Arc<dyn EventBus>,
    raw_reply: &str,
    reasoning_content: Option<String>,
    turn_id: u64,
) -> corework::error::Result<ThinkingDonePayload> {
    let empty_model_response = crate::prompt_assets::template("empty_model_response.md")
        .trim()
        .to_string();
    let stripped_reply = crate::decision::strip_think_tags_pub(raw_reply).trim();
    let reply = if stripped_reply.is_empty() {
        empty_model_response.as_str()
    } else {
        stripped_reply
    };
    let (visible_reply, stop_signal) = parse_stop_signal(reply);
    let visible_reply = if visible_reply.is_empty() {
        crate::prompt_assets::template("empty_visible_response.md")
            .trim()
            .to_string()
    } else {
        visible_reply
    };
    let (task_status, stop_reason, next_after_saying, decision) = match stop_signal {
        Some(StopSignal::Waiting) => ("waiting", Some("waiting"), states::SUSPENDED, "waiting"),
        Some(StopSignal::Done) => ("done", Some("done"), states::SUSPENDED, "done"),
        None => ("done", Some("done"), states::SUSPENDED, "done"),
    };
    let canonical_content = if raw_reply.trim().is_empty() {
        visible_reply.as_str()
    } else {
        raw_reply
    };
    let mut metadata = crate::ledger::LedgerMessageMeta::default();
    metadata.display_content = Some(visible_reply.clone());
    let assistant_msg = crate::context::Message {
        role: crate::context::roles::ASSISTANT.to_string(),
        content: canonical_content.to_string(),
        cache_control: false,
        tool_call_id: None,
        name: None,
        tool_calls: None,
        reasoning_content,
    };
    AssistantContext::push_message_with_metadata_and_display_on_event_bus(
        cache,
        event_bus,
        assistant_msg,
        metadata,
        None,
    )
    .await?;
    cache
        .set(keys::PENDING_RESPONSE, &visible_reply, None)
        .await?;
    cache
        .set(keys::TASK_STATUS, &task_status.to_string(), None)
        .await?;
    if let Some(reason) = stop_reason {
        cache
            .set(keys::LAST_STOP_REASON, &reason.to_string(), None)
            .await?;
    } else {
        cache.delete(keys::LAST_STOP_REASON).await?;
    }
    cache
        .set(
            keys::NEXT_STATE_AFTER_SAYING,
            &next_after_saying.to_string(),
            None,
        )
        .await?;
    cache
        .set(keys::NEXT_STATE, &states::SAYING.to_string(), None)
        .await?;

    Ok(ThinkingDonePayload {
        reasoning: None,
        decision: decision.to_string(),
        tools: vec![],
        turn_id,
    })
}

// ============================================================================
// ============================================================================

async fn on_transition(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<Option<String>> {
    let cache = sm_ctx.cache();

    if super::consume_pause_if_requested(&cache).await? {
        return Ok(Some(states::SUSPENDED.to_string()));
    }

    let next: Option<String> = cache.get(keys::NEXT_STATE).await?;
    Ok(next)
}

async fn set_result_next_state(
    cache: &Arc<dyn corework::cache::Cache>,
    result: &str,
) -> corework::error::Result<()> {
    cache
        .set(keys::PENDING_RESULT, &result.to_string(), None)
        .await?;

    cache
        .set(keys::NEXT_STATE, &states::SAYING.to_string(), None)
        .await?;
    Ok(())
}

fn log_thinking_context_probe(
    history: &[crate::context::Message],
    compact: &[crate::context::Message],
    selected: &[crate::context::Message],
    agent_id: &str,
    turn_id: u64,
    round: u32,
) {
    if !runtime_context_probe_enabled() {
        return;
    }
    append_context_probe_log(&format!(
        "thinking_context_probe agent={} turn={} round={} history_len={} compact_len={} selected_len={}",
        agent_id,
        turn_id,
        round,
        history.len(),
        compact.len(),
        selected.len()
    ));
    tracing::info!(
        agent_id = %agent_id,
        turn_id = turn_id,
        round = round,
        history_len = history.len(),
        compact_len = compact.len(),
        selected_len = selected.len(),
        "thinking_context_probe: selected history before LLM call"
    );
    for (offset, msg) in history.iter().rev().take(5).enumerate() {
        append_context_probe_log(&format!(
            "thinking_context_probe_history_tail agent={} turn={} round={} offset={} role={} content={}",
            agent_id,
            turn_id,
            round,
            offset,
            msg.role,
            truncate_probe_text(&msg.content, 500)
        ));
        tracing::info!(
            agent_id = %agent_id,
            turn_id = turn_id,
            round = round,
            tail_offset = offset,
            role = %msg.role,
            content = %truncate_probe_text(&msg.content, 500),
            "thinking_context_probe_history_tail"
        );
    }
    for (offset, msg) in compact.iter().rev().take(5).enumerate() {
        append_context_probe_log(&format!(
            "thinking_context_probe_compact_tail agent={} turn={} round={} offset={} role={} content={}",
            agent_id,
            turn_id,
            round,
            offset,
            msg.role,
            truncate_probe_text(&msg.content, 500)
        ));
        tracing::info!(
            agent_id = %agent_id,
            turn_id = turn_id,
            round = round,
            tail_offset = offset,
            role = %msg.role,
            content = %truncate_probe_text(&msg.content, 500),
            "thinking_context_probe_compact_tail"
        );
    }
    for (offset, msg) in selected.iter().rev().take(5).enumerate() {
        append_context_probe_log(&format!(
            "thinking_context_probe_selected_tail agent={} turn={} round={} offset={} role={} content={}",
            agent_id,
            turn_id,
            round,
            offset,
            msg.role,
            truncate_probe_text(&msg.content, 500)
        ));
        tracing::info!(
            agent_id = %agent_id,
            turn_id = turn_id,
            round = round,
            tail_offset = offset,
            role = %msg.role,
            content = %truncate_probe_text(&msg.content, 500),
            "thinking_context_probe_selected_tail"
        );
    }
}

fn log_host_dynamic_snapshot_probe(
    snapshots: &std::collections::HashMap<String, String>,
    agent_id: &str,
    turn_id: u64,
    round: u32,
) {
    if !runtime_context_probe_enabled() {
        return;
    }
    let total_bytes: usize = snapshots.values().map(|value| value.len()).sum();
    append_context_probe_log(&format!(
        "host_dynamic_snapshot_probe agent={} turn={} round={} field_count={} total_bytes={}",
        agent_id,
        turn_id,
        round,
        snapshots.len(),
        total_bytes
    ));
    tracing::info!(
        agent_id = %agent_id,
        turn_id = turn_id,
        round = round,
        field_count = snapshots.len(),
        total_bytes = total_bytes,
        "host_dynamic_snapshot_probe"
    );
    let mut sorted: Vec<_> = snapshots.iter().collect();
    sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (field_name, text) in sorted {
        append_context_probe_log(&format!(
            "host_dynamic_snapshot_field agent={} turn={} round={} field={} bytes={} content={}",
            agent_id,
            turn_id,
            round,
            field_name,
            text.len(),
            truncate_probe_text(text, 1200)
        ));
        tracing::info!(
            agent_id = %agent_id,
            turn_id = turn_id,
            round = round,
            field = %field_name,
            bytes = text.len(),
            content = %truncate_probe_text(text, 1200),
            "host_dynamic_snapshot_field"
        );
    }
}

fn log_llm_messages_probe(
    messages: &[llm_gateway::ChatMessage],
    agent_id: &str,
    turn_id: u64,
    round: u32,
) {
    if !runtime_context_probe_enabled() {
        return;
    }
    append_context_probe_log(&format!(
        "llm_messages_probe agent={} turn={} round={} message_count={}",
        agent_id,
        turn_id,
        round,
        messages.len()
    ));
    for (offset, msg) in messages.iter().rev().take(5).enumerate() {
        append_context_probe_log(&format!(
            "llm_messages_tail agent={} turn={} round={} offset={} role={} cache_control={} content={}",
            agent_id,
            turn_id,
            round,
            offset,
            msg.role,
            msg.cache_control,
            truncate_probe_text(&msg.content, 1200)
        ));
        tracing::info!(
            agent_id = %agent_id,
            turn_id = turn_id,
            round = round,
            tail_offset = offset,
            role = %msg.role,
            cache_control = msg.cache_control,
            content = %truncate_probe_text(&msg.content, 1200),
            "llm_messages_tail"
        );
    }
}

fn truncate_probe_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn compose_cached_persona_context(
    persona_text: &str,
    immutable_role_appendix: &str,
    immutable_cache_entries: &BTreeMap<String, String>,
) -> String {
    let mut sections = vec![persona_text.trim_end().to_string()];
    let appendix = immutable_role_appendix.trim();
    if !appendix.is_empty() {
        sections.push(appendix.to_string());
    }
    let immutable_cache_section = format_immutable_cache_entries_section(immutable_cache_entries);
    if !immutable_cache_section.is_empty() {
        sections.push(immutable_cache_section);
    }
    sections.join("\n\n")
}

fn append_context_probe_log(line: &str) {
    use std::io::Write;
    if !runtime_context_probe_enabled() {
        return;
    }
    let path = runtime_context_probe_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", line);
    }
}

fn runtime_context_probe_enabled() -> bool {
    matches!(
        std::env::var("RUNTIME_CONTEXT_PROBE").ok().as_deref(),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

fn runtime_context_probe_file() -> std::path::PathBuf {
    std::env::var("RUNTIME_CONTEXT_PROBE_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new("data")
                .join("logs")
                .join("runtime-context-probe.log")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::cache::{Cache, InMemoryCache};
    use corework::event::InMemoryEventBus;

    #[test]
    fn leading_system_cache_marker_is_configurable() {
        let plain = leading_system_message("system".to_string(), false);
        let cached = leading_system_message("system".to_string(), true);

        assert_eq!(plain.role, "system");
        assert!(!plain.cache_control);
        assert_eq!(cached.role, "system");
        assert!(cached.cache_control);
    }

    #[test]
    fn conversation_cache_boundary_marks_history_tail_only() {
        let mut messages = vec![
            llm_gateway::ChatMessage::system_cached("system"),
            llm_gateway::ChatMessage::user("request"),
            llm_gateway::ChatMessage::assistant("answer"),
        ];

        mark_conversation_history_cache_boundary(&mut messages, true);
        messages.push(llm_gateway::ChatMessage::user("dynamic context"));

        assert!(messages[0].cache_control);
        assert!(!messages[1].cache_control);
        assert!(messages[2].cache_control);
        assert!(!messages[3].cache_control);
    }

    #[test]
    fn current_plan_tail_contains_facts_but_not_system_rules() {
        let rendered =
            render_current_plan_section("Plan", "Summary", "active", "now", "1. Do work");

        assert!(rendered.contains("Plan"));
        assert!(rendered.contains("1. Do work"));
        assert!(!rendered.contains("PlanUpdate"));
        assert!(!rendered.contains("PlanFinish"));
    }

    #[test]
    fn parse_stop_signal_only_consumes_final_strict_marker() {
        let (visible, signal) = parse_stop_signal("Please provide the project name.\n\nWAITING\n");
        assert_eq!(visible, "Please provide the project name.");
        assert_eq!(signal, Some(StopSignal::Waiting));

        let (visible, signal) = parse_stop_signal("The task is complete.\nDONE");
        assert_eq!(visible, "The task is complete.");
        assert_eq!(signal, Some(StopSignal::Done));

        let (visible, signal) =
            parse_stop_signal("This mentions WAITING but not on the final line.");
        assert_eq!(visible, "This mentions WAITING but not on the final line.");
        assert_eq!(signal, None);

        let (visible, signal) = parse_stop_signal("waiting");
        assert_eq!(visible, "waiting");
        assert_eq!(signal, None);
    }

    #[tokio::test]
    async fn reserve_auto_continue_step_stops_at_configured_max() {
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        cache
            .set(keys::AUTO_CONTINUE_MAX_STEPS, &2u32, None)
            .await
            .expect("set max steps");

        assert!(reserve_auto_continue_step(&cache).await.unwrap());
        assert!(reserve_auto_continue_step(&cache).await.unwrap());
        assert!(!reserve_auto_continue_step(&cache).await.unwrap());

        let steps: Option<u32> = cache.get(keys::AUTO_CONTINUE_STEPS).await.unwrap();
        assert_eq!(steps, Some(2));
    }

    #[tokio::test]
    async fn plain_content_without_stop_marker_defaults_to_done() {
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        let event_bus: Arc<dyn EventBus> = Arc::new(InMemoryEventBus::new());

        let payload = build_thinking_payload_from_plain_content(
            &cache,
            &event_bus,
            "The task is complete.",
            Some("checked final state".to_string()),
            42,
        )
        .await
        .unwrap();

        assert_eq!(payload.decision, "done");
        assert_eq!(payload.turn_id, 42);

        let task_status: Option<String> = cache.get(keys::TASK_STATUS).await.unwrap();
        assert_eq!(task_status.as_deref(), Some("done"));

        let stop_reason: Option<String> = cache.get(keys::LAST_STOP_REASON).await.unwrap();
        assert_eq!(stop_reason.as_deref(), Some("done"));

        let next_state: Option<String> = cache.get(keys::NEXT_STATE).await.unwrap();
        assert_eq!(next_state.as_deref(), Some(states::SAYING));

        let next_after_saying: Option<String> =
            cache.get(keys::NEXT_STATE_AFTER_SAYING).await.unwrap();
        assert_eq!(next_after_saying.as_deref(), Some(states::SUSPENDED));
    }

    #[test]
    fn cached_persona_context_appends_immutable_entries_in_key_order() {
        let entries = BTreeMap::from([
            ("zeta".to_string(), "Z entry".to_string()),
            ("alpha".to_string(), "A entry".to_string()),
        ]);

        let context = compose_cached_persona_context("Role skill", "Appendix", &entries);

        assert!(context.contains("Role skill\n\nAppendix\n\nA entry\n\nZ entry"));
        assert!(!context.contains("alpha"));
        assert!(!context.contains("zeta"));
    }
}
