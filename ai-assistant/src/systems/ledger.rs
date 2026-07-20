use async_trait::async_trait;
use corework::buns_system;
use corework::error::FrameworkError;
use corework::event::BaseEvent;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::context::Message;
use crate::conversation_state::{ConversationState, LedgerReadOptions};
#[cfg(test)]
use crate::ledger::FOCUS_RESOURCE_KEY;
use crate::ledger::{self, LedgerMessageMeta, LedgerRecord, LedgerRole};
use crate::persistence::DisplayMeta;
use crate::systems::agent_route::record_focus_change_if_needed;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendLedgerMessageInput {
    #[serde(default = "default_conversation_id")]
    pub conversation_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub role: LedgerRole,
    pub content: String,
    #[serde(default)]
    pub metadata: LedgerMessageMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryLedgerInput {
    #[serde(default = "default_conversation_id")]
    pub conversation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAgentContextInput {
    #[serde(default = "default_conversation_id")]
    pub conversation_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLlmExecutionSnapshot {
    pub conversation_id: String,
    pub agent_id: String,
    pub messages: Vec<Message>,
    pub message_count: usize,
    pub has_summary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetLedgerFocusInput {
    #[serde(default = "default_conversation_id")]
    pub conversation_id: String,
    pub to_agent_id: String,
    #[serde(default = "default_focus_reason")]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerFocusOutput {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSnapshot {
    pub ledger: Vec<LedgerRecord>,
    pub runtime: crate::agent::AgentClusterSnapshot,
}

async fn effective_conversation_id(input: &str, ctx: &Context) -> Result<String, FrameworkError> {
    let state = conversation_state(ctx)?;
    let conversation_id = state.conversation_id();
    if input != ledger::DEFAULT_CONVERSATION_ID
        && !input.trim().is_empty()
        && input != conversation_id
    {
        return Err(FrameworkError::InvalidOperation(format!(
            "conversation '{}' is outside the current execution hierarchy",
            input
        )));
    }
    Ok(conversation_id.to_string())
}

fn conversation_state(ctx: &Context) -> Result<Arc<ConversationState>, FrameworkError> {
    ctx.resolve_shared_component::<ConversationState>()
}

#[cfg(test)]
async fn read_ledger(ctx: &Context) -> Result<Vec<LedgerRecord>, FrameworkError> {
    Ok(conversation_state(ctx)?
        .list_recent(LedgerReadOptions::default())
        .await)
}

#[cfg(test)]
async fn write_ledger(ctx: &Context, records: &[LedgerRecord]) -> Result<(), FrameworkError> {
    conversation_state(ctx)?.replace(records.to_vec()).await;
    Ok(())
}

#[buns_system(
    "AppendLedgerMessageSystem",
    description = "Append a canonical message to the AI Assistant ledger.",
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct AppendLedgerMessageSystem;

#[cfg(test)]
async fn write_focus(ctx: &Context, agent_id: &str) -> Result<(), FrameworkError> {
    ctx.cache
        .set_raw(FOCUS_RESOURCE_KEY, serde_json::to_value(agent_id)?, None)
        .await
}

#[async_trait]
impl SystemOperation for AppendLedgerMessageSystem {
    type Input = AppendLedgerMessageInput;
    type Output = LedgerRecord;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let conversation_id = effective_conversation_id(&input.conversation_id, ctx).await?;
        let record =
            if input.display.is_some() || input.tool_call_id.is_some() || input.tool_name.is_some()
            {
                let mut message = Message {
                    role: input.role.as_str().to_string(),
                    content: input.content,
                    cache_control: false,
                    tool_call_id: input.tool_call_id,
                    name: input.tool_name,
                    tool_calls: None,
                    reasoning_content: None,
                };
                if input.role == LedgerRole::GatewayMessage {
                    message.role = "gateway_message".to_string();
                }
                let mut record = LedgerRecord::from_message(
                    0,
                    conversation_id.clone(),
                    input.agent_id,
                    input.agent_name,
                    message,
                    input.display,
                );
                record.metadata = record.metadata.overlay(input.metadata);
                record
            } else {
                LedgerRecord {
                    record_id: 0,
                    conversation_id: conversation_id.clone(),
                    agent_id: input.agent_id,
                    agent_name: input.agent_name,
                    role: input.role,
                    content: input.content,
                    metadata: input.metadata,
                    created_at: chrono::Local::now().to_rfc3339(),
                }
            };
        let record = conversation_state(ctx)?.append(record).await?;
        ctx.world_event_bus
            .publish(BaseEvent::new(
                crate::events::types::LEDGER_RECORD_APPENDED,
                serde_json::to_value(crate::events::LedgerRecordAppendedPayload {
                    record: record.clone(),
                })
                .map_err(|e| FrameworkError::SystemError(e.to_string()))?,
            ))
            .await?;
        Ok(record)
    }

    fn name(&self) -> &str {
        "AppendLedgerMessageSystem"
    }
}

#[buns_system(
    "QueryLedgerSystem",
    description = "Query the raw AI Assistant ledger.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryLedgerSystem;

#[async_trait]
impl SystemOperation for QueryLedgerSystem {
    type Input = QueryLedgerInput;
    type Output = Vec<LedgerRecord>;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let conversation_id = effective_conversation_id(&input.conversation_id, ctx).await?;
        let _ = conversation_id;
        Ok(conversation_state(ctx)?
            .list_recent(LedgerReadOptions::default())
            .await)
    }

    fn name(&self) -> &str {
        "QueryLedgerSystem"
    }
}

#[buns_system(
    "QueryFrontendMessagesSystem",
    description = "Query frontend-visible ledger messages.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryFrontendMessagesSystem;

#[async_trait]
impl SystemOperation for QueryFrontendMessagesSystem {
    type Input = QueryLedgerInput;
    type Output = Vec<crate::gateway::GatewayMessage>;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let query = ctx.system_by_type::<QueryLedgerSystem>()?;
        let ledger = query.execute(input, ctx).await?;
        Ok(ledger::frontend_messages(&ledger))
    }

    fn name(&self) -> &str {
        "QueryFrontendMessagesSystem"
    }
}

#[buns_system(
    "QueryAgentContextSystem",
    description = "Query the LLM context projection for an agent.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryAgentContextSystem;

#[async_trait]
impl SystemOperation for QueryAgentContextSystem {
    type Input = QueryAgentContextInput;
    type Output = Vec<crate::context::Message>;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let query = ctx.system_by_type::<QueryLedgerSystem>()?;
        let ledger = query
            .execute(
                QueryLedgerInput {
                    conversation_id: input.conversation_id,
                },
                ctx,
            )
            .await?;
        Ok(ledger::agent_context(&ledger, &input.agent_id))
    }

    fn name(&self) -> &str {
        "QueryAgentContextSystem"
    }
}

#[buns_system(
    "QueryAgentLlmExecutionSnapshotSystem",
    description = "Query the LLM execution snapshot for a conversation agent.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryAgentLlmExecutionSnapshotSystem;

#[async_trait]
impl SystemOperation for QueryAgentLlmExecutionSnapshotSystem {
    type Input = QueryAgentContextInput;
    type Output = AgentLlmExecutionSnapshot;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let conversation_id = effective_conversation_id(&input.conversation_id, ctx).await?;
        let agent_id = input.agent_id;
        let query = ctx.system_by_type::<QueryAgentContextSystem>()?;
        let messages = query
            .execute(
                QueryAgentContextInput {
                    conversation_id: conversation_id.clone(),
                    agent_id: agent_id.clone(),
                },
                ctx,
            )
            .await?;
        let has_summary = QueryLatestAgentSummarySystem
            .execute(
                QueryAgentContextInput {
                    conversation_id: conversation_id.clone(),
                    agent_id: agent_id.clone(),
                },
                ctx,
            )
            .await?
            .is_some();
        Ok(AgentLlmExecutionSnapshot {
            conversation_id,
            agent_id,
            message_count: messages.len(),
            has_summary,
            messages,
        })
    }

    fn name(&self) -> &str {
        "QueryAgentLlmExecutionSnapshotSystem"
    }
}

#[buns_system(
    "QueryLatestAgentSummarySystem",
    description = "Query the latest summary record for an agent.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryLatestAgentSummarySystem;

#[async_trait]
impl SystemOperation for QueryLatestAgentSummarySystem {
    type Input = QueryAgentContextInput;
    type Output = Option<LedgerRecord>;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let query = ctx.system_by_type::<QueryLedgerSystem>()?;
        let ledger = query
            .execute(
                QueryLedgerInput {
                    conversation_id: input.conversation_id,
                },
                ctx,
            )
            .await?;
        Ok(ledger
            .into_iter()
            .rev()
            .find(|record| record.agent_id == input.agent_id && record.role == LedgerRole::Summary))
    }

    fn name(&self) -> &str {
        "QueryLatestAgentSummarySystem"
    }
}

#[buns_system(
    "QueryConversationSnapshotSystem",
    description = "Query the full ledger and agent runtime snapshot for persistence.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryConversationSnapshotSystem;

#[async_trait]
impl SystemOperation for QueryConversationSnapshotSystem {
    type Input = QueryLedgerInput;
    type Output = ConversationSnapshot;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let query = ctx.system_by_type::<QueryLedgerSystem>()?;
        let ledger = query.execute(input, ctx).await?;
        let cluster = ctx
            .resolve_shared_component::<crate::agent::cluster::AgentCluster>()
            .map_err(|_| {
                FrameworkError::InvalidOperation("conversation is not initialized".into())
            })?;
        let runtime = cluster.snapshot().await;
        Ok(ConversationSnapshot { ledger, runtime })
    }

    fn name(&self) -> &str {
        "QueryConversationSnapshotSystem"
    }
}

#[buns_system(
    "SetLedgerFocusSystem",
    description = "Set the current focused agent and record a focus_changed gateway fact.",
    destructive = false,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct SetLedgerFocusSystem;

#[async_trait]
impl SystemOperation for SetLedgerFocusSystem {
    type Input = SetLedgerFocusInput;
    type Output = LedgerFocusOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let cluster = ctx.resolve_shared_component::<crate::agent::cluster::AgentCluster>()?;
        let ledger = ctx.execution_unit().ok_or_else(|| {
            FrameworkError::InvalidOperation(
                "ledger execution unit is unavailable from context".to_string(),
            )
        })?;
        record_focus_change_if_needed(&cluster, &ledger, input.to_agent_id.clone(), &input.reason)
            .await?;

        Ok(LedgerFocusOutput {
            agent_id: input.to_agent_id,
        })
    }

    fn name(&self) -> &str {
        "SetLedgerFocusSystem"
    }
}

#[buns_system(
    "QueryLedgerFocusSystem",
    description = "Query the current focused agent.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct QueryLedgerFocusSystem;

#[async_trait]
impl SystemOperation for QueryLedgerFocusSystem {
    type Input = QueryLedgerInput;
    type Output = LedgerFocusOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let conversation_id = effective_conversation_id(&input.conversation_id, ctx).await?;
        let _ = conversation_id;
        let agent_id = conversation_state(ctx)?.focus().await;
        Ok(LedgerFocusOutput { agent_id })
    }

    fn name(&self) -> &str {
        "QueryLedgerFocusSystem"
    }
}

fn default_conversation_id() -> String {
    ledger::DEFAULT_CONVERSATION_ID.to_string()
}

fn default_focus_reason() -> String {
    "system".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::system::SystemOperation;
    use std::sync::Arc;

    fn test_unit(conversation_id: &str) -> Arc<corework::execution_unit::ExecutionUnit> {
        let framework = corework::world::FrameworkState::initialize().unwrap();
        let unit = Arc::new(corework::execution_unit::ExecutionUnit::new_root_in_scope(
            corework::execution_unit::UnitType::Module,
            framework,
            format!("conversation:{conversation_id}"),
        ));
        unit.attach_shared_component(Arc::new(ConversationState::new(
            conversation_id,
            Default::default(),
            crate::agent::keys::BOSS_AGENT_ID,
        )))
        .unwrap();
        unit
    }

    #[tokio::test]
    async fn query_frontend_context_and_snapshot_from_world_ledger() {
        let _guard = crate::test_support::global_test_guard().await;
        let unit = test_unit("ledger-test-frontend");
        let ctx = unit.create_context();
        crate::agent::set_conversation_id_in_cache(&*ctx.cache, "ledger-test-frontend")
            .await
            .unwrap();
        write_ledger(&ctx, &Vec::<LedgerRecord>::new())
            .await
            .unwrap();
        write_focus(&ctx, crate::agent::keys::BOSS_AGENT_ID)
            .await
            .unwrap();

        let append = AppendLedgerMessageSystem;
        append
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id: "agent_1".to_string(),
                    agent_name: "Agent 1".to_string(),
                    role: LedgerRole::User,
                    content: "hello".to_string(),
                    metadata: LedgerMessageMeta::default(),
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                },
                &ctx,
            )
            .await
            .unwrap();
        append
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id: "agent_1".to_string(),
                    agent_name: "Agent 1".to_string(),
                    role: LedgerRole::GatewayMessage,
                    content: String::new(),
                    metadata: LedgerMessageMeta {
                        subtype: Some(ledger::GATEWAY_SUBTYPE_PAUSE_REQUESTED.to_string()),
                        ..Default::default()
                    },
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let frontend = QueryFrontendMessagesSystem
            .execute(
                QueryLedgerInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(frontend.len(), 1);
        assert_eq!(frontend[0].role, "user");

        let context = QueryAgentContextSystem
            .execute(
                QueryAgentContextInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id: "agent_1".to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].content, "hello");

        let ledger = QueryLedgerSystem
            .execute(
                QueryLedgerInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(ledger.len(), 2);
    }

    #[tokio::test]
    async fn agent_ctx_inherits_conversation_state_from_parent() {
        let _guard = crate::test_support::global_test_guard().await;
        let framework = corework::world::FrameworkState::initialize().unwrap();
        let ledger_unit = Arc::new(corework::execution_unit::ExecutionUnit::new_root_in_scope(
            corework::execution_unit::UnitType::Module,
            framework.clone(),
            "conversation:ctx-reg-test",
        ));
        ledger_unit
            .attach_shared_component(Arc::new(ConversationState::new(
                "ctx-reg-test",
                Default::default(),
                crate::agent::keys::BOSS_AGENT_ID,
            )))
            .unwrap();
        let agent_unit = Arc::new(
            corework::execution_unit::ExecutionUnit::new_child(
                corework::execution_unit::UnitType::StateMachine,
                &ledger_unit,
            )
            .unwrap(),
        );
        let ledger_ctx = ledger_unit.create_context();
        write_ledger(&ledger_ctx, &Vec::<LedgerRecord>::new())
            .await
            .unwrap();
        AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: "ctx-reg-test".to_string(),
                    agent_id: "agent_1".to_string(),
                    agent_name: "Agent 1".to_string(),
                    role: LedgerRole::User,
                    content: "from ledger".to_string(),
                    metadata: LedgerMessageMeta::default(),
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                },
                &ledger_ctx,
            )
            .await
            .unwrap();
        AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: "ctx-reg-test".to_string(),
                    agent_id: "agent_1".to_string(),
                    agent_name: "Agent 1".to_string(),
                    role: LedgerRole::Summary,
                    content: "important summary".to_string(),
                    metadata: LedgerMessageMeta::default(),
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                },
                &ledger_ctx,
            )
            .await
            .unwrap();
        AppendLedgerMessageSystem
            .execute(
                AppendLedgerMessageInput {
                    conversation_id: "ctx-reg-test".to_string(),
                    agent_id: "agent_1".to_string(),
                    agent_name: "Agent 1".to_string(),
                    role: LedgerRole::User,
                    content: "after summary".to_string(),
                    metadata: LedgerMessageMeta::default(),
                    display: None,
                    tool_call_id: None,
                    tool_name: None,
                },
                &ledger_ctx,
            )
            .await
            .unwrap();

        let agent_ctx = agent_unit.create_context();
        crate::agent::set_conversation_id_in_cache(&*agent_ctx.cache, "ctx-reg-test")
            .await
            .unwrap();
        let snapshot = QueryAgentLlmExecutionSnapshotSystem
            .execute(
                QueryAgentContextInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id: "agent_1".to_string(),
                },
                &agent_ctx,
            )
            .await
            .unwrap();
        assert_eq!(snapshot.conversation_id, "ctx-reg-test");
        assert!(snapshot.has_summary);
        assert_eq!(snapshot.messages.len(), 2);
        assert!(snapshot.messages[0].content.contains("important summary"));
        assert_eq!(snapshot.messages[1].content, "after summary");

        let frontend = QueryFrontendMessagesSystem
            .execute(
                QueryLedgerInput {
                    conversation_id: "ctx-reg-test".to_string(),
                },
                &ledger_ctx,
            )
            .await
            .unwrap();
        assert_eq!(frontend.len(), 2);
        assert!(frontend
            .iter()
            .all(|message| message.content != "important summary"));
    }

    #[tokio::test]
    async fn concurrent_appends_keep_monotonic_record_ids() {
        let _guard = crate::test_support::global_test_guard().await;
        let unit = test_unit("ledger-test-concurrent");
        let ctx = unit.create_context();
        crate::agent::set_conversation_id_in_cache(&*ctx.cache, "ledger-test-concurrent")
            .await
            .unwrap();
        write_ledger(&ctx, &Vec::<LedgerRecord>::new())
            .await
            .unwrap();

        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..50usize {
            let ctx = ctx.clone();
            tasks.spawn(async move {
                AppendLedgerMessageSystem
                    .execute(
                        AppendLedgerMessageInput {
                            conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                            agent_id: format!("agent_{}", index % 4),
                            agent_name: format!("Agent {}", index % 4),
                            role: LedgerRole::Tool,
                            content: format!("tool result {index}"),
                            metadata: LedgerMessageMeta::default(),
                            display: None,
                            tool_call_id: None,
                            tool_name: None,
                        },
                        &ctx,
                    )
                    .await
                    .unwrap()
                    .record_id
            });
        }

        let mut returned_ids = Vec::new();
        while let Some(result) = tasks.join_next().await {
            returned_ids.push(result.unwrap());
        }

        let ledger = read_ledger(&ctx).await.unwrap();
        returned_ids.sort_unstable();
        returned_ids.dedup();

        let batch_records = ledger
            .iter()
            .filter(|record| record.content.starts_with("tool result "))
            .collect::<Vec<_>>();
        let mut batch_contents = batch_records
            .iter()
            .map(|record| record.content.clone())
            .collect::<Vec<_>>();
        batch_contents.sort();
        batch_contents.dedup();

        assert_eq!(returned_ids.len(), 50);
        assert_eq!(batch_records.len(), 50);
        assert_eq!(batch_contents.len(), 50);
        for pair in batch_records.windows(2) {
            assert!(
                pair[0].record_id < pair[1].record_id,
                "append order should keep record ids increasing"
            );
        }
    }

    #[tokio::test]
    async fn restored_legacy_records_project_correctly() {
        let _guard = crate::test_support::global_test_guard().await;
        let unit = test_unit("ledger-test-legacy");
        let ctx = unit.create_context();
        crate::agent::set_conversation_id_in_cache(&*ctx.cache, "ledger-test-legacy")
            .await
            .unwrap();
        let legacy = crate::persistence::PersistedMessage {
            ledger: None,
            inner: crate::context::Message::tool("legacy tool result"),
            agent_id: Some("agent_1".to_string()),
            display: None,
        };
        let ledger = vec![legacy.into_ledger(1)];
        write_ledger(&ctx, &ledger).await.unwrap();

        let frontend = QueryFrontendMessagesSystem
            .execute(
                QueryLedgerInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(frontend.len(), 1);
        assert_eq!(frontend[0].role, "tool");

        let context = QueryAgentContextSystem
            .execute(
                QueryAgentContextInput {
                    conversation_id: ledger::DEFAULT_CONVERSATION_ID.to_string(),
                    agent_id: "agent_1".to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].role, "user");
        assert!(context[0].content.contains("legacy tool result"));
    }
}
