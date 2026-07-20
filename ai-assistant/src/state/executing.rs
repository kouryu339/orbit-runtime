//! ## CLI 命令模式
//! 1. 读取 `PENDING_TOOLS`（`Vec<String>`）
//! 2. 对每个命令字符串：
//!    - 解析 `"SystemName --arg1 val1"` 格式
//!    - 将结果以 `role: "tool"` 推入对话历史
//! 3. 完成后 → 回到 thinking

use std::sync::Arc;

use crate::context::{keys, AssistantContext};
use crate::events::{types as ev_types, ToolEndPayload, ToolStartPayload};
use crate::permission::{
    denied_tool_result, PendingToolPermission, PermissionBroker, PermissionOutcome,
    ToolPermissionMode,
};
use crate::persistence::DisplayMeta;
use crate::tool_runner;
use crate::tool_runner::parse_tool_command;

use corework::cache::CacheExt;
use corework::event::BaseEvent;
use corework::execution_unit::ExecutionUnit;
use corework::orchestration::Context;
use corework::statemachine::{FnState, SimpleTransition};
use std::collections::BTreeMap;
use tokio::task::JoinSet;

use super::{events, states};

/// 构建执行状态
/// 状态转移由 `on_transition` 读取 `NEXT_STATE` cache 键路由，
/// 不需要注册事件驱动的 transition。
pub fn build() -> FnState {
    FnState::new(states::EXECUTING)
        .with_description("并发执行 AI 系统")
        .with_on_enter(|ctx| Box::pin(on_enter(ctx)))
        .with_on_transition(|ctx| Box::pin(on_transition(ctx)))
        .add_transition(
            events::PAUSE,
            Box::new(SimpleTransition::new(events::PAUSE, states::SUSPENDED)),
        )
}

// ============================================================================
// on_enter：读取待执行工具 → 逐个执行 → 推入对话历史
// ============================================================================

async fn on_enter(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<()> {
    let cache = sm_ctx.cache();
    let event_bus = sm_ctx.event_bus();
    crate::agent::publish_focus_status_for_cache(
        sm_ctx.as_ref(),
        &*cache,
        &*event_bus,
        states::EXECUTING,
    )
    .await;

    // ★ 提前检查暂停信号：工具还没执行就可以立刻停
    if super::consume_pause_if_requested(&cache).await? {
        // consume 已消费信号并设置了 PENDING_RESPONSE，直接指向 asking
        cache
            .set(keys::NEXT_STATE, &states::SUSPENDED.to_string(), None)
            .await?;
        return Ok(());
    }

    // 构建 Context（DynamicExecute 需要）
    let exec_ctx = sm_ctx.create_context();

    // ---- 读取待执行命令 ----
    let tools: Vec<String> = cache.get(keys::PENDING_TOOLS).await?.unwrap_or_default();

    if tools.is_empty() {
        tracing::warn!("执行状态无待执行工具，直接返回");
        return Ok(());
    }

    let tool_call_refs: Vec<crate::context::ToolCallRef> = cache
        .get(keys::PENDING_TOOL_CALLS)
        .await?
        .unwrap_or_default();
    let preallocated_call_ids: Vec<String> = cache
        .get(keys::PENDING_TOOL_CALL_IDS)
        .await?
        .unwrap_or_default();
    let source_id = crate::agent::source_id_from_cache(&*cache).await;
    let turn_id = crate::context::AssistantContext::current_turn_id(&cache).await;
    let conversation_id = exec_ctx.conversation_id.as_deref().unwrap_or("default");
    tracing::info!(
        conversation_id = %conversation_id,
        agent_id = %source_id,
        turn_id,
        tool_count = tools.len(),
        "executing tool batch"
    );
    let event_bus = exec_ctx.event_bus.clone();
    let call_ids = tools
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            preallocated_call_ids
                .get(idx)
                .cloned()
                .or_else(|| {
                    tool_call_refs
                        .get(idx)
                        .map(|tool_call| tool_call.id.clone())
                })
                .unwrap_or_else(|| format!("{}:{}:{}", source_id, turn_id, idx))
        })
        .collect::<Vec<_>>();
    let recovery_results: BTreeMap<String, crate::decision::ToolResult> = cache
        .get(keys::PENDING_TOOL_RECOVERY_RESULTS)
        .await?
        .unwrap_or_default();
    let results = run_tools(&tools, &call_ids, &recovery_results, &exec_ctx, &*cache).await;

    // ---- 构建结果文本 + DisplayMeta ----
    struct ToolPart {
        content: String,
        display: DisplayMeta,
        result: serde_json::Value,
    }

    let mut tool_parts: Vec<ToolPart> = Vec::with_capacity(results.len());
    for r in &results {
        let (cmd_name, _) = parse_tool_command(&r.command);
        let entry = r.to_ai.clone();
        tracing::debug!(
            conversation_id = %conversation_id,
            agent_id = %source_id,
            turn_id,
            tool_name = %cmd_name,
            success = r.success,
            error_code = r.error_code,
            result_len = entry.len(),
            "tool execution result summarized"
        );
        if !r.success {
            tracing::warn!(
                conversation_id = %conversation_id,
                agent_id = %source_id,
                turn_id,
                tool_name = %cmd_name,
                error_code = r.error_code,
                "tool execution failed"
            );
        }
        tool_parts.push(ToolPart {
            content: entry,
            display: DisplayMeta {
                display_role: "tool_step".to_string(),
                tool_name: Some(cmd_name.to_string()),
                tool_command: Some(r.command.clone()),
                success: Some(r.success),
                reasoning: None,
                decision: None,
                tools: Vec::new(),
                agent_name: None,
            },
            result: r.result.clone(),
        });
    }

    // ---- 推入执行结果消息 ----
    // 只有真实 FC 路径才会带 PENDING_TOOL_CALLS，此时必须写 role:"tool" 来配对。
    // EXEC 行式路径的模型输出只是 assistant 文本，结果应作为普通上下文回灌，
    // 避免把历史伪造成模型没有实际发出的 Function Calling 协议。
    for (idx, tool_part) in tool_parts.into_iter().enumerate() {
        let call_id = call_ids
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("{}:{}:{}", source_id, turn_id, idx));
        let status = if tool_part.display.success == Some(false) {
            "error"
        } else {
            "success"
        };
        let subtype = if tool_part.display.success == Some(false) {
            crate::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FAILED
        } else {
            crate::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FINISHED
        };
        let mut metadata = crate::ledger::LedgerMessageMeta::default();
        metadata.subtype = Some(subtype.to_string());
        metadata.tool_name = tool_part.display.tool_name.clone();
        metadata.tool_command = tool_part.display.tool_command.clone();
        metadata.success = tool_part.display.success;
        metadata.collapsed = Some(true);
        metadata
            .extra
            .insert("kind".to_string(), serde_json::json!("tool"));
        metadata
            .extra
            .insert("status".to_string(), serde_json::json!(status));
        metadata
            .extra
            .insert("call_id".to_string(), serde_json::json!(call_id));
        metadata
            .extra
            .insert("turn_id".to_string(), serde_json::json!(turn_id));
        if let Some(object) = tool_part.result.as_object() {
            for key in ["recovery_kind", "effect", "source"] {
                if let Some(value) = object.get(key) {
                    metadata.extra.insert(key.to_string(), value.clone());
                }
            }
            if object.get("status").and_then(serde_json::Value::as_str)
                == Some("recovery_interrupted")
            {
                metadata.extra.insert(
                    "status".to_string(),
                    serde_json::json!("recovery_interrupted"),
                );
            }
        }
        if let Some(tool_call) = tool_call_refs.get(idx) {
            AssistantContext::push_message_with_metadata_and_display_on_event_bus(
                &cache,
                &event_bus,
                crate::context::Message::tool_with_id(
                    tool_part.content,
                    tool_call.id.clone(),
                    tool_call.name.clone(),
                ),
                metadata,
                Some(tool_part.display),
            )
            .await?;
        } else {
            AssistantContext::push_message_with_metadata_and_display_on_event_bus(
                &cache,
                &event_bus,
                crate::context::Message::tool(tool_part.content.clone()),
                metadata,
                Some(tool_part.display),
            )
            .await?;
        }
    }
    cache.delete(keys::PENDING_TOOL_CALLS).await?;
    cache.delete(keys::PENDING_TOOL_CALL_IDS).await?;
    cache.delete(keys::PENDING_TOOL_DISPLAY_COMMANDS).await?;
    cache.delete(keys::PENDING_TOOL_RECOVERY_RESULTS).await?;
    let wait_for_input: bool = cache
        .get(keys::PENDING_TOOLS_WAIT_FOR_INPUT)
        .await?
        .unwrap_or(false);
    cache.delete(keys::PENDING_TOOLS_WAIT_FOR_INPUT).await?;
    if wait_for_input {
        cache
            .set(keys::TASK_STATUS, &"waiting".to_string(), None)
            .await?;
        cache
            .set(keys::LAST_STOP_REASON, &"waiting".to_string(), None)
            .await?;
        cache
            .set(keys::NEXT_STATE, &states::SUSPENDED.to_string(), None)
            .await?;
    }

    Ok(())
}

// ============================================================================
// on_transition：检查暂停信号，决定转向 thinking 还是 asking
// ============================================================================

async fn on_transition(sm_ctx: Arc<ExecutionUnit>) -> corework::error::Result<Option<String>> {
    let cache = sm_ctx.cache();

    if super::consume_pause_if_requested(&cache).await? {
        return Ok(Some(states::SUSPENDED.to_string()));
    }

    // 优先读取 on_enter 写入的 NEXT_STATE（暂停时指向 asking），否则默认回 thinking
    // 读取后立即清除，防止残留值被下一轮 executing.on_transition 误读
    let next: Option<String> = cache.get(keys::NEXT_STATE).await?;
    tracing::debug!(next_state = ?next, "executing transition resolved");
    cache.delete(keys::NEXT_STATE).await?;
    Ok(Some(next.unwrap_or_else(|| states::THINKING.to_string())))
}

// ============================================================================
// 工具执行
// ============================================================================

/// 每个工具执行前后分别发布 TOOL_START / TOOL_END 事件。
async fn run_tools(
    tools: &[String],
    call_ids: &[String],
    recovery_results: &BTreeMap<String, crate::decision::ToolResult>,
    ctx: &Context,
    _cache: &dyn corework::cache::Cache,
) -> Vec<crate::decision::ToolResult> {
    if tools.is_empty() {
        return Vec::new();
    }

    let mut join_set = JoinSet::new();

    for (idx, cmd) in tools.iter().enumerate() {
        let cmd = cmd.clone();
        let ctx = ctx.clone();
        let call_id = call_ids.get(idx).cloned();
        let recovery_result = call_id
            .as_ref()
            .and_then(|id| recovery_results.get(id))
            .cloned();

        join_set.spawn(async move {
            let (cmd_name, _) = parse_tool_command(&cmd);
            if let Some(result) = recovery_result {
                let turn_id = crate::context::AssistantContext::current_turn_id(&ctx.cache).await;
                let src = crate::agent::source_id_from_cache(&*ctx.cache).await;
                let conversation_id = ctx.conversation_id.as_deref().unwrap_or("default");
                tracing::warn!(
                    conversation_id = %conversation_id,
                    agent_id = %src,
                    turn_id,
                    tool_call_id = call_id.as_deref().unwrap_or(""),
                    tool_index = idx,
                    tool_name = %cmd_name,
                    "tool execution restored from interrupted non-repeatable call"
                );
                publish_tool_end_event(&ctx, &cmd_name, &cmd, &src, turn_id, &result).await;
                return (idx, result);
            }
            let conversation_id = ctx.conversation_id.as_deref().unwrap_or("default");
            tracing::debug!(
                conversation_id = %conversation_id,
                tool_call_id = call_id.as_deref().unwrap_or(""),
                tool_index = idx,
                tool_name = %cmd_name,
                command_len = cmd.len(),
                "tool execution start"
            );

            // 读取当前 turn id —— 工具事件携带，用于前端过滤过期事件
            let turn_id = crate::context::AssistantContext::current_turn_id(&ctx.cache).await;
            let src = crate::agent::source_id_from_cache(&*ctx.cache).await;
            let call_id = call_id.unwrap_or_else(|| format!("{}:{}:{}", src, turn_id, idx));
            let permission =
                match authorize_tool(&ctx, &cmd, &cmd_name, &call_id, &src, turn_id).await {
                    Ok(()) => None,
                    Err(result) => Some(result),
                };
            if let Some(result) = permission {
                publish_tool_end_event(&ctx, &cmd_name, &cmd, &src, turn_id, &result).await;
                return (idx, result);
            }
            publish_tool_fact(
                &ctx,
                crate::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED,
                &cmd_name,
                &cmd,
                &call_id,
                turn_id,
                "running",
                None,
                format!("正在执行 {}", cmd_name),
            )
            .await;

            // 发布工具开始事件
            if let Ok(mut payload_json) = serde_json::to_value(&ToolStartPayload {
                name: cmd_name.to_string(),
                command: cmd.clone(),
                turn_id,
            }) {
                if let Some(obj) = payload_json.as_object_mut() {
                    obj.insert(
                        "agent_id".to_string(),
                        serde_json::Value::String(src.clone()),
                    );
                }
                let event = BaseEvent::new(ev_types::TOOL_START, payload_json);
                let _ = ctx.event_bus.publish(event).await;
            }

            let result = tool_runner::execute_single_with_call_id(&cmd, Some(&call_id), &ctx).await;

            // 发布工具结束事件
            publish_tool_end_event(&ctx, &cmd_name, &cmd, &src, turn_id, &result).await;

            (idx, result)
        });
    }

    // 收集所有结果，按原始顺序排列
    let mut results = Vec::with_capacity(tools.len());
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((idx, result)) => {
                results.push((idx, result));
            }
            Err(e) => {
                tracing::error!("任务执行 panicked: {:?}", e);
            }
        }
    }

    // 按索引排序以恢复原始顺序
    results.sort_by_key(|(idx, _)| *idx);
    results.into_iter().map(|(_, r)| r).collect()
}

async fn authorize_tool(
    ctx: &Context,
    command: &str,
    tool_name: &str,
    tool_call_id: &str,
    agent_id: &str,
    turn_id: u64,
) -> Result<(), crate::decision::ToolResult> {
    let metadata = tool_runner::permission_metadata(tool_name)
        .map_err(|_| denied_tool_result(command, tool_name, "tool_unavailable"))?;
    let broker = ctx
        .resolve_shared_component::<PermissionBroker>()
        .map_err(|_| denied_tool_result(command, tool_name, "policy"))?;
    match broker.policy().mode_for(metadata.effect) {
        ToolPermissionMode::Full => Ok(()),
        ToolPermissionMode::Deny => Err(denied_tool_result(command, tool_name, "policy")),
        ToolPermissionMode::Ask => {
            let conversation_id = ctx
                .conversation_id
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let request = PendingToolPermission {
                conversation_id,
                tool_call_id: tool_call_id.to_string(),
                agent_id: agent_id.to_string(),
                tool_name: tool_name.to_string(),
                display_name: metadata.display_name,
                effect: metadata.effect,
                arguments: if metadata.secret {
                    serde_json::json!({"redacted": true})
                } else {
                    tool_runner::permission_arguments(command)
                },
                turn_id,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            publish_tool_fact(
                ctx,
                crate::ledger::GATEWAY_SUBTYPE_TOOL_CALL_PERMISSION_REQUESTED,
                tool_name,
                command,
                tool_call_id,
                turn_id,
                "waiting_permission",
                None,
                format!("Waiting for permission: {tool_name}"),
            )
            .await;
            match broker.request(request).await {
                PermissionOutcome::Allowed => Ok(()),
                PermissionOutcome::UserDenied => {
                    Err(denied_tool_result(command, tool_name, "user"))
                }
                PermissionOutcome::TimedOut => {
                    Err(denied_tool_result(command, tool_name, "timeout"))
                }
                PermissionOutcome::Cancelled => {
                    Err(denied_tool_result(command, tool_name, "cancelled"))
                }
            }
        }
    }
}

async fn publish_tool_end_event(
    ctx: &Context,
    tool_name: &str,
    command: &str,
    agent_id: &str,
    turn_id: u64,
    result: &crate::decision::ToolResult,
) {
    if let Ok(mut payload_json) = serde_json::to_value(&ToolEndPayload {
        name: tool_name.to_string(),
        command: command.to_string(),
        success: result.success,
        result: result.to_ai.clone(),
        turn_id,
    }) {
        if let Some(obj) = payload_json.as_object_mut() {
            obj.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
        }
        let event = BaseEvent::new(ev_types::TOOL_END, payload_json);
        let _ = ctx.event_bus.publish(event).await;
    }
}

async fn publish_tool_fact(
    ctx: &Context,
    subtype: &str,
    tool_name: &str,
    command: &str,
    call_id: &str,
    turn_id: u64,
    status: &str,
    success: Option<bool>,
    content: String,
) {
    let (agent_id, agent_name) = crate::agent::source_meta_from_cache(&*ctx.cache).await;
    let mut metadata = crate::ledger::LedgerMessageMeta::default();
    metadata.subtype = Some(subtype.to_string());
    metadata.tool_name = Some(tool_name.to_string());
    metadata.tool_command = Some(command.to_string());
    metadata.success = success;
    metadata.collapsed = Some(true);
    metadata
        .extra
        .insert("kind".to_string(), serde_json::json!("tool"));
    metadata
        .extra
        .insert("status".to_string(), serde_json::json!(status));
    metadata
        .extra
        .insert("call_id".to_string(), serde_json::json!(call_id));
    metadata
        .extra
        .insert("turn_id".to_string(), serde_json::json!(turn_id));
    let _ = ctx
        .world_event_bus
        .publish(BaseEvent::new(
            crate::events::types::AGENT_MESSAGE_PRODUCED,
            serde_json::to_value(crate::events::AgentMessageProducedPayload {
                conversation_id: crate::ledger::DEFAULT_CONVERSATION_ID.to_string(),
                agent_id,
                agent_name,
                role: crate::ledger::LedgerRole::GatewayMessage,
                content,
                metadata,
                display: None,
                tool_call_id: None,
                tool_name: Some(tool_name.to_string()),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        ))
        .await;
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use crate::tool_runner::parse_tool_command;

    #[test]
    fn test_parse_tool_command_with_args() {
        let (name, args) =
            parse_tool_command("CallBaiduOcr --image_url http://example.com/img.png");
        assert_eq!(name, "CallBaiduOcr");
        assert_eq!(args, "--image_url http://example.com/img.png");
    }

    #[test]
    fn test_parse_tool_command_name_only() {
        let (name, args) = parse_tool_command("GetSkillsList");
        assert_eq!(name, "GetSkillsList");
        assert_eq!(args, "");
    }

    #[test]
    fn test_parse_tool_command_with_whitespace() {
        let (name, args) = parse_tool_command("  SomeSys   --key val  ");
        assert_eq!(name, "SomeSys");
        assert_eq!(args, "--key val");
    }
}
