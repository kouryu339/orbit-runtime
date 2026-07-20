use std::collections::BTreeMap;

use ai_assistant::{AssistantContext, ConversationManager};
use corework::cache::CacheExt;
use serde::{de, Deserialize, Deserializer};
use serde_json::{json, Value};

use super::*;

#[derive(Debug, Clone)]
pub(super) struct RestoredConversationRecovery {
    pub(super) entry_states: Vec<(String, String)>,
    pub(super) execution_plans: Vec<RestoredExecutionPlan>,
}

#[derive(Debug, Clone)]
pub(super) struct RestoredExecutionPlan {
    pub(super) agent_id: String,
    pub(super) tools: Vec<String>,
    pub(super) call_ids: Vec<String>,
    pub(super) recovery_results: BTreeMap<String, ai_assistant::ToolResult>,
}

#[derive(Debug, Clone)]
struct OpenToolCall {
    call_id: String,
    tool_name: Option<String>,
    tool_command: Option<String>,
    agent_id: String,
    turn_id: Option<u64>,
    source: OpenToolCallSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenToolCallSource {
    AssistantDeclared,
    Started,
}

pub(super) fn restored_conversation_recovery(
    records: &[ai_assistant::ledger::LedgerRecord],
) -> RestoredConversationRecovery {
    let execution_plans = restored_execution_plans(records);
    if !execution_plans.is_empty() {
        return RestoredConversationRecovery {
            entry_states: Vec::new(),
            execution_plans,
        };
    }

    let entry_states = restored_entry_states(records);
    RestoredConversationRecovery {
        entry_states,
        execution_plans,
    }
}

fn restored_entry_states(records: &[ai_assistant::ledger::LedgerRecord]) -> Vec<(String, String)> {
    let Some(last) = records.last() else {
        return Vec::new();
    };
    let state = match last.role {
        ai_assistant::ledger::LedgerRole::User => Some(ai_assistant::state::states::THINKING),
        ai_assistant::ledger::LedgerRole::Assistant => Some(ai_assistant::state::states::SUSPENDED),
        ai_assistant::ledger::LedgerRole::Tool => Some(ai_assistant::state::states::THINKING),
        ai_assistant::ledger::LedgerRole::GatewayMessage if is_terminal_tool_record(last) => {
            Some(ai_assistant::state::states::THINKING)
        }
        _ => None,
    };
    state
        .map(|state| vec![(last.agent_id.clone(), state.to_string())])
        .unwrap_or_default()
}

pub(super) fn restored_execution_plans(
    records: &[ai_assistant::ledger::LedgerRecord],
) -> Vec<RestoredExecutionPlan> {
    let mut open_calls = BTreeMap::<String, OpenToolCall>::new();
    for record in records.iter() {
        remember_assistant_declared_tool_calls(&mut open_calls, record);
        update_open_tool_calls(&mut open_calls, record);
    }
    if !open_calls.is_empty() {
        return execution_plans_from_open_calls(open_calls);
    }

    let Some(last) = records.last() else {
        return Vec::new();
    };
    if last.role == ai_assistant::ledger::LedgerRole::Assistant {
        if let Ok(tool_calls) = ai_assistant::runtime::parser::parse_tool_calls(&last.content) {
            if !tool_calls.is_empty() {
                let tools = tool_calls
                    .iter()
                    .map(|call| call.to_legacy_command())
                    .collect::<Vec<_>>();
                let mut call_ids = last
                    .metadata
                    .extra
                    .get("tool_call_ids")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                while call_ids.len() < tools.len() {
                    call_ids.push(format!("recovered:{}:{}", last.record_id, call_ids.len()));
                }
                call_ids.truncate(tools.len());
                return vec![RestoredExecutionPlan {
                    agent_id: last.agent_id.clone(),
                    tools,
                    call_ids,
                    recovery_results: BTreeMap::new(),
                }];
            }
        }
    }

    Vec::new()
}

fn execution_plans_from_open_calls(
    open_calls: BTreeMap<String, OpenToolCall>,
) -> Vec<RestoredExecutionPlan> {
    let mut grouped = BTreeMap::<String, RestoredExecutionPlan>::new();
    for open_call in open_calls.into_values() {
        let Some(command) = open_call.tool_command.clone() else {
            continue;
        };
        let entry = grouped
            .entry(open_call.agent_id.clone())
            .or_insert_with(|| RestoredExecutionPlan {
                agent_id: open_call.agent_id.clone(),
                tools: Vec::new(),
                call_ids: Vec::new(),
                recovery_results: BTreeMap::new(),
            });
        entry.tools.push(command.clone());
        entry.call_ids.push(open_call.call_id.clone());
        if recovery_tool_effect(open_call.tool_name.as_deref()) != "read_only" {
            entry.recovery_results.insert(
                open_call.call_id.clone(),
                recovery_tool_result(&open_call, &command),
            );
        }
    }
    grouped
        .into_values()
        .filter(|plan| !plan.tools.is_empty())
        .collect()
}

#[cfg(test)]
pub(super) fn repair_restored_ledger(
    records: &mut Vec<ai_assistant::ledger::LedgerRecord>,
    target_conversation_id: &str,
) {
    let mut next_record_id = records
        .iter()
        .map(|record| record.record_id)
        .max()
        .unwrap_or(0)
        + 1;
    for plan in restored_execution_plans(records) {
        for (index, call_id) in plan.call_ids.iter().enumerate() {
            let command = plan.tools.get(index).cloned().unwrap_or_default();
            let result = plan
                .recovery_results
                .get(call_id)
                .cloned()
                .unwrap_or_else(|| {
                    let (tool_name, _) = ai_assistant::tool_runner::parse_tool_command(&command);
                    recovery_tool_result(
                        &OpenToolCall {
                            call_id: call_id.clone(),
                            tool_name: Some(tool_name.to_string()),
                            tool_command: Some(command.clone()),
                            agent_id: plan.agent_id.clone(),
                            turn_id: None,
                            source: OpenToolCallSource::AssistantDeclared,
                        },
                        &command,
                    )
                });
            let (tool_name, _) = ai_assistant::tool_runner::parse_tool_command(&result.command);
            let mut metadata = ai_assistant::ledger::LedgerMessageMeta::default();
            metadata.subtype =
                Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FAILED.to_string());
            metadata.tool_name = Some(tool_name.to_string());
            metadata.tool_command = Some(result.command.clone());
            metadata.success = Some(false);
            metadata.collapsed = Some(true);
            metadata.extra.insert("kind".to_string(), json!("tool"));
            metadata
                .extra
                .insert("status".to_string(), json!("recovery_interrupted"));
            metadata.extra.insert("call_id".to_string(), json!(call_id));
            if let Some(object) = result.result.as_object() {
                for key in ["recovery_kind", "effect", "source"] {
                    if let Some(value) = object.get(key) {
                        metadata.extra.insert(key.to_string(), value.clone());
                    }
                }
            }
            records.push(ai_assistant::ledger::LedgerRecord {
                record_id: next_record_id,
                conversation_id: target_conversation_id.to_string(),
                agent_id: plan.agent_id.clone(),
                agent_name: plan.agent_id.clone(),
                role: ai_assistant::ledger::LedgerRole::Tool,
                content: result.to_ai.clone(),
                metadata,
                created_at: chrono::Local::now().to_rfc3339(),
            });
            next_record_id += 1;
        }
    }
}

fn remember_assistant_declared_tool_calls(
    open_calls: &mut BTreeMap<String, OpenToolCall>,
    record: &ai_assistant::ledger::LedgerRecord,
) {
    if record.role != ai_assistant::ledger::LedgerRole::Assistant {
        return;
    }
    let Some(call_ids) = record
        .metadata
        .extra
        .get("tool_call_ids")
        .and_then(Value::as_array)
    else {
        return;
    };
    let commands = ai_assistant::runtime::parser::parse_tool_calls(&record.content)
        .map(|calls| {
            calls
                .into_iter()
                .map(|call| call.to_legacy_command())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (index, call_id) in call_ids.iter().filter_map(Value::as_str).enumerate() {
        let command = commands.get(index).cloned();
        let tool_name = command
            .as_deref()
            .map(ai_assistant::tool_runner::parse_tool_command)
            .map(|(name, _)| name.to_string());
        open_calls
            .entry(call_id.to_string())
            .or_insert(OpenToolCall {
                call_id: call_id.to_string(),
                tool_name,
                tool_command: command,
                agent_id: record.agent_id.clone(),
                turn_id: record.metadata.extra.get("turn_id").and_then(Value::as_u64),
                source: OpenToolCallSource::AssistantDeclared,
            });
    }
}

fn update_open_tool_calls(
    open_calls: &mut BTreeMap<String, OpenToolCall>,
    record: &ai_assistant::ledger::LedgerRecord,
) {
    let Some(call_id) = record
        .metadata
        .extra
        .get("call_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    if is_terminal_tool_record(record) {
        open_calls.remove(&call_id);
        return;
    }
    if is_started_tool_record(record) {
        open_calls.insert(
            call_id.clone(),
            OpenToolCall {
                call_id,
                tool_name: record.metadata.tool_name.clone(),
                tool_command: record.metadata.tool_command.clone(),
                agent_id: record.agent_id.clone(),
                turn_id: record.metadata.extra.get("turn_id").and_then(Value::as_u64),
                source: OpenToolCallSource::Started,
            },
        );
    }
}

fn is_started_tool_record(record: &ai_assistant::ledger::LedgerRecord) -> bool {
    record.metadata.subtype.as_deref()
        == Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_STARTED)
        || record.metadata.extra.get("status").and_then(Value::as_str) == Some("running")
}

fn is_terminal_tool_record(record: &ai_assistant::ledger::LedgerRecord) -> bool {
    matches!(
        record.metadata.subtype.as_deref(),
        Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FINISHED)
            | Some(ai_assistant::ledger::GATEWAY_SUBTYPE_TOOL_CALL_FAILED)
    ) || matches!(
        record.metadata.extra.get("status").and_then(Value::as_str),
        Some("success") | Some("error") | Some("failed") | Some("canceled")
    )
}

fn recovery_tool_effect(tool_name: Option<&str>) -> &'static str {
    let Some(tool_name) = tool_name else {
        return "unknown";
    };
    match ai_assistant::tool_runner::permission_metadata(tool_name).map(|metadata| metadata.effect)
    {
        Ok(ai_assistant::ToolEffect::ReadOnly) => "read_only",
        Ok(ai_assistant::ToolEffect::ControlledChange) => "controlled_change",
        Ok(ai_assistant::ToolEffect::Destructive) => "destructive",
        Err(_) => "unknown",
    }
}

fn recovery_tool_result(open_call: &OpenToolCall, command: &str) -> ai_assistant::ToolResult {
    let effect = recovery_tool_effect(open_call.tool_name.as_deref());
    ai_assistant::ToolResult {
        command: command.to_string(),
        success: false,
        to_ai: recovery_tool_to_ai(open_call, effect),
        error_code: 1,
        result: json!({
            "status": "recovery_interrupted",
            "recovery_kind": "unfinished_tool_call",
            "call_id": open_call.call_id,
            "effect": effect,
            "source": match open_call.source {
                OpenToolCallSource::AssistantDeclared => "assistant_declared",
                OpenToolCallSource::Started => "started",
            },
            "turn_id": open_call.turn_id,
        }),
    }
}

fn recovery_tool_to_ai(open_call: &OpenToolCall, effect: &str) -> String {
    let tool_name = open_call.tool_name.as_deref().unwrap_or("unknown");
    let command = recovery_command_line(open_call.tool_command.as_deref().unwrap_or(""));
    if effect == "read_only" {
        return format!(
            "The runtime was interrupted before this tool call produced a closed result.\n\
tool_call_id: {}\n\
tool_name: {}\n\
effect: read_only\n\
This tool is classified as read-only, so it may be run again if the current state still needs a fresh observation.\n\
{}",
            open_call.call_id, tool_name, command
        );
    }
    format!(
        "The runtime was interrupted before this tool call produced a closed result.\n\
tool_call_id: {}\n\
tool_name: {}\n\
effect: {}\n\
Do not assume the operation succeeded, and do not directly repeat the same operation. If it may have side effects, first verify the external system state or report the interruption and re-plan.\n\
For child agents, report the uncertainty to the parent agent instead of asking the user directly.\n\
{}",
        open_call.call_id, tool_name, effect, command
    )
}

fn recovery_command_line(command: &str) -> String {
    if command.trim().is_empty() {
        String::new()
    } else {
        format!("command: {command}")
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ConversationSnapshotImport {
    #[allow(dead_code)]
    pub(super) schema: Option<String>,
    #[allow(dead_code)]
    pub(super) conversation_id: Option<String>,
    #[serde(default)]
    pub(super) ledger: Vec<ai_assistant::ledger::LedgerRecord>,
    #[serde(
        default,
        alias = "state_delta",
        alias = "stateDeltas",
        deserialize_with = "deserialize_state_deltas"
    )]
    state_deltas: Vec<Value>,
    #[serde(default)]
    state: Option<Value>,
    #[allow(dead_code)]
    runtime: Option<serde_json::Value>,
}

fn deserialize_state_deltas<'de, D>(deserializer: D) -> Result<Vec<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?.unwrap_or(Value::Null);
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => Ok(items),
        Value::Object(_) => Ok(vec![value]),
        _ => Err(de::Error::custom("state_deltas must be an object or array")),
    }
}

pub(super) fn snapshot_state_deltas(snapshot: &ConversationSnapshotImport) -> Vec<Value> {
    let mut deltas = snapshot.state_deltas.clone();
    if let Some(state) = &snapshot.state {
        if let Some(items) = state.get("deltas").and_then(Value::as_array) {
            deltas.extend(items.iter().cloned());
        }
        if let Some(items) = state.get("state_deltas").and_then(Value::as_array) {
            deltas.extend(items.iter().cloned());
        }
    }
    deltas
}

pub(super) fn parse_conversation_snapshot(
    snapshot_json: &str,
) -> Result<ConversationSnapshotImport, RuntimeError> {
    let snapshot: ConversationSnapshotImport =
        serde_json::from_str(snapshot_json).map_err(|e| {
            RuntimeError::InvalidConfig(format!("parse conversation snapshot failed: {e}"))
        })?;
    if snapshot.schema.as_deref() != Some("agent-runtime-conversation-snapshot/v1") {
        return Err(RuntimeError::InvalidConfig(
            "conversation snapshot schema must be 'agent-runtime-conversation-snapshot/v1'"
                .to_string(),
        ));
    }
    Ok(snapshot)
}

pub(super) async fn apply_conversation_state_delta(
    manager: &ConversationManager,
    conversation_id: &str,
    delta: Value,
) -> Result<(), RuntimeError> {
    let op = match delta.get("op").and_then(Value::as_str) {
        Some(op) if !op.trim().is_empty() => op,
        _ => return Ok(()),
    };

    match op {
        "focus.set" => {
            let Some(agent_id) = delta
                .get("focus_agent_id")
                .or_else(|| delta.get("agent_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            manager
                .set_focus(conversation_id, agent_id.to_string())
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        "dynamic_snapshot.set" => {
            let Some(agent_id) = delta
                .get("agent_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let Some(field) = delta
                .get("field")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let text = delta
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            manager
                .set_host_dynamic_snapshot_field(conversation_id, agent_id, field, text)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        "agent_task.upsert" => {
            let task_value = delta.get("task").cloned().unwrap_or_else(|| delta.clone());
            match serde_json::from_value::<ai_assistant::conversation_state::AgentTaskEntry>(
                task_value,
            ) {
                Ok(task) => {
                    manager
                        .upsert_agent_task(conversation_id, task)
                        .await
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                }
                Err(error) => {
                    tracing::warn!("skip invalid restored agent_task.upsert delta: {}", error);
                }
            }
        }
        "agent_skills.set" => {
            let Some(agent_id) = delta
                .get("agent_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let cache = match manager.agent_cache(conversation_id, agent_id).await {
                Ok(cache) => cache,
                Err(error) => {
                    tracing::warn!(
                        "skip restored agent_skills.set for unavailable agent '{}': {}",
                        agent_id,
                        error
                    );
                    return Ok(());
                }
            };
            if let Some(main_skills) = string_array_field(&delta, "main_skills") {
                cache
                    .set(ai_assistant::context::keys::MAIN_SKILLS, &main_skills, None)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            if let Some(imported_skills) = string_array_field(&delta, "imported_skills") {
                cache
                    .set(
                        ai_assistant::context::keys::IMPORTED_SKILLS,
                        &imported_skills,
                        None,
                    )
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            if let Some(active_tools) = string_array_field(&delta, "active_tools") {
                AssistantContext::set_active_tools(&cache, active_tools)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
        }
        "agent_plan.set" => {
            let Some(agent_id) = delta
                .get("agent_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(());
            };
            let Some(plan_value) = delta.get("plan").cloned() else {
                return Ok(());
            };
            let plan =
                match serde_json::from_value::<ai_assistant::context::CurrentPlan>(plan_value) {
                    Ok(plan) => plan,
                    Err(error) => {
                        tracing::warn!("skip invalid restored agent_plan.set delta: {}", error);
                        return Ok(());
                    }
                };
            let cache = match manager.agent_cache(conversation_id, agent_id).await {
                Ok(cache) => cache,
                Err(error) => {
                    tracing::warn!(
                        "skip restored agent_plan.set for unavailable agent '{}': {}",
                        agent_id,
                        error
                    );
                    return Ok(());
                }
            };
            AssistantContext::set_current_plan(&cache, &plan)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }
        _ => {}
    }
    Ok(())
}

fn string_array_field(value: &Value, field: &str) -> Option<Vec<String>> {
    value.get(field).and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

impl RuntimeFacade {
    pub fn snapshot(&self) -> Result<String, RuntimeError> {
        let state_store = Arc::clone(&self.state_store);
        let cluster_id = self.config.runtime.cluster_id.clone();
        let conversations = if self.started {
            self.rt
                .block_on(async move { load_conversation_index(&state_store, &cluster_id).await })?
        } else if let Some(manager) = self.conversation_manager.clone() {
            self.rt.block_on(async move { manager.list().await })
        } else {
            Vec::new()
        };
        Ok(json!({
            "schema": "agent-runtime-snapshot/v1",
            "started": self.started,
            "config": self.config,
            "conversations": conversations,
            "note": "per-conversation snapshot export is not wired in v0.1 shell"
        })
        .to_string())
    }

    pub fn export_conversation_snapshot(
        &self,
        conversation_id: &str,
        options_json: &str,
    ) -> Result<String, RuntimeError> {
        let options = ConversationSnapshotExportOptions::parse(options_json)?;
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let conversation_state = manager
                .frontend_state(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let is_stable =
                conversation_state == ai_assistant::snapshot::ConversationState::Waiting;
            if options.stable_required() && !is_stable {
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation_not_waiting: conversation '{}' is in state '{}'",
                    conversation_id,
                    serde_json::to_value(conversation_state)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_else(|| "unknown".to_string())
                )));
            }
            let ledger = manager
                .ledger(
                    &conversation_id,
                    ai_assistant::conversation_state::LedgerReadOptions::default(),
                )
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let cluster = manager
                .cluster_snapshot(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let tasks = manager
                .agent_tasks(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let info = manager
                .list()
                .await
                .into_iter()
                .find(|info| info.conversation_id == conversation_id)
                .ok_or_else(|| {
                    RuntimeError::InvalidConfig(format!(
                        "conversation '{}' not found",
                        conversation_id
                    ))
                })?;
            Ok(json!({
                "schema": "agent-runtime-conversation-snapshot/v1",
                "conversation_id": conversation_id,
                "tenant_id": info.tenant_id,
                "user_id": info.user_id,
                "consistency": options.as_str(),
                "stable": is_stable,
                "conversation_state": conversation_state,
                "ledger": ledger,
                "runtime": cluster,
                "tasks": tasks,
                "exported_at_unix_ms": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis())
                    .unwrap_or(0)
            })
            .to_string())
        })
    }

    pub fn agent_tasks_json(&self, conversation_id: &str) -> Result<String, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let tasks = manager
                .agent_tasks(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            Ok(json!({
                "schema": "agent-runtime-agent-tasks/v1",
                "conversation_id": conversation_id,
                "tasks": tasks,
            })
            .to_string())
        })
    }

    pub fn import_conversation_snapshot(
        &mut self,
        snapshot_json: &str,
        options_json: &str,
    ) -> Result<(), RuntimeError> {
        let options = parse_conversation_options(options_json)?;
        let target_conversation_id = non_empty_arg(
            options.conversation_id.as_deref().unwrap_or_default(),
            "options.conversation_id",
        )?;
        let snapshot = parse_conversation_snapshot(snapshot_json)?;
        let state_deltas = snapshot_state_deltas(&snapshot);
        let recovery =
            self.replace_conversation_ledger(&target_conversation_id, snapshot.ledger)?;
        self.apply_conversation_state_deltas(&target_conversation_id, state_deltas)?;
        self.apply_conversation_recovery(&target_conversation_id, recovery)
    }

    pub(super) fn replace_conversation_ledger(
        &mut self,
        target_conversation_id: &str,
        mut records: Vec<ai_assistant::ledger::LedgerRecord>,
    ) -> Result<RestoredConversationRecovery, RuntimeError> {
        let manager = self.manager()?;
        let target_conversation_id =
            non_empty_arg(target_conversation_id, "target_conversation_id")?;
        self.require_conversation_owner(&target_conversation_id)?;
        records.sort_by_key(|record| record.record_id);
        for (index, record) in records.iter_mut().enumerate() {
            record.conversation_id = target_conversation_id.clone();
            record.record_id = index as u64 + 1;
        }
        let recovery = restored_conversation_recovery(&records);
        self.rt.block_on(async move {
            manager
                .replace_ledger(&target_conversation_id, records)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            Ok(recovery)
        })
    }

    pub(super) fn apply_conversation_state_deltas(
        &mut self,
        target_conversation_id: &str,
        state_deltas: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        if state_deltas.is_empty() {
            return Ok(());
        }
        let manager = self.manager()?;
        let target_conversation_id =
            non_empty_arg(target_conversation_id, "target_conversation_id")?;
        self.require_conversation_owner(&target_conversation_id)?;
        self.rt.block_on(async move {
            for delta in state_deltas {
                apply_conversation_state_delta(&manager, &target_conversation_id, delta).await?;
            }
            Ok(())
        })
    }

    pub(super) fn apply_conversation_recovery(
        &mut self,
        target_conversation_id: &str,
        recovery: RestoredConversationRecovery,
    ) -> Result<(), RuntimeError> {
        if recovery.entry_states.is_empty() && recovery.execution_plans.is_empty() {
            return Ok(());
        }
        let manager = self.manager()?;
        let target_conversation_id =
            non_empty_arg(target_conversation_id, "target_conversation_id")?;
        self.require_conversation_owner(&target_conversation_id)?;
        self.rt.block_on(async move {
            for (agent_id, state) in recovery.entry_states {
                manager
                    .restore_agent_state_entry(&target_conversation_id, &agent_id, &state)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            for plan in recovery.execution_plans {
                manager
                    .restore_agent_execution_entry(
                        &target_conversation_id,
                        &plan.agent_id,
                        plan.tools,
                        plan.call_ids,
                        plan.recovery_results,
                    )
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            }
            Ok(())
        })
    }
}
