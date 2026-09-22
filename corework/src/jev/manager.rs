use crate::cache::CacheExt;
use crate::error::{FrameworkError, Result};
use crate::event::{BaseEvent, EventBus};
use crate::execution_unit::{ExecutionUnit, UnitType};
use crate::jev::{
    apply_snapshot_update, JevArgumentBinding, JevClient, JevDecision, JevDefinition,
    JevExecutionContext, JevRunOutcome, JevRunStatus, JevToolExecutor, JEV_DECISION_EVENT,
    JEV_RUN_COMPLETED_EVENT, JEV_RUN_STARTED_EVENT, JEV_SNAPSHOT_UPDATED_EVENT,
    JEV_TOOL_COMPLETED_EVENT, JEV_TOOL_STARTED_EVENT,
};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

const SNAPSHOT_KEY: &str = "jev:snapshot";
const SNAPSHOT_REVISION_KEY: &str = "jev:snapshot_revision";

pub struct JevManager {
    owner: Arc<ExecutionUnit>,
    client: Arc<dyn JevClient>,
    executor: Arc<dyn JevToolExecutor>,
    event_bus: Arc<dyn EventBus>,
    definitions: RwLock<HashMap<String, JevDefinition>>,
    active_runs: DashMap<String, Arc<ExecutionUnit>>,
}

impl JevManager {
    pub fn new(
        owner: Arc<ExecutionUnit>,
        client: Arc<dyn JevClient>,
        executor: Arc<dyn JevToolExecutor>,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            owner,
            client,
            executor,
            event_bus,
            definitions: RwLock::new(HashMap::new()),
            active_runs: DashMap::new(),
        }
    }

    pub fn register(&self, definition: JevDefinition) -> Result<()> {
        definition.validate()?;
        let mut definitions = self.definitions.write();
        if definitions.contains_key(&definition.name) {
            return Err(FrameworkError::InvalidOperation(format!(
                "Jev definition '{}' is already registered",
                definition.name
            )));
        }
        definitions.insert(definition.name.clone(), definition);
        Ok(())
    }

    pub fn replace(&self, definition: JevDefinition) -> Result<()> {
        definition.validate()?;
        self.definitions
            .write()
            .insert(definition.name.clone(), definition);
        Ok(())
    }

    pub fn list(&self) -> Vec<JevDefinition> {
        let mut values = self
            .definitions
            .read()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.name.cmp(&b.name));
        values
    }

    pub fn get(&self, name: &str) -> Option<JevDefinition> {
        self.definitions.read().get(name).cloned()
    }
    pub fn active_run_count(&self) -> usize {
        self.active_runs.len()
    }

    pub async fn run(
        &self,
        jev_name: &str,
        task: &str,
        conversation_id: Option<String>,
        agent_id: Option<String>,
        parent_run_id: Option<String>,
        caller: Option<Arc<ExecutionUnit>>,
    ) -> Result<JevRunOutcome> {
        self.run_with_executor(
            jev_name,
            task,
            conversation_id,
            agent_id,
            parent_run_id,
            caller,
            Arc::clone(&self.executor),
        )
        .await
    }

    /// Runs a Jev instance with a caller-specific tool execution boundary.
    ///
    /// The host uses the manager's direct executor. AI callers provide an
    /// executor that applies the conversation's permission policy before
    /// delegating to the same tool registry.
    pub async fn run_with_executor(
        &self,
        jev_name: &str,
        task: &str,
        conversation_id: Option<String>,
        agent_id: Option<String>,
        parent_run_id: Option<String>,
        caller: Option<Arc<ExecutionUnit>>,
        executor: Arc<dyn JevToolExecutor>,
    ) -> Result<JevRunOutcome> {
        validate_identity(&conversation_id, &agent_id)?;
        let definition = self.get(jev_name).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "Jev definition '{jev_name}' is not registered"
            ))
        })?;
        if task.trim().is_empty() {
            return Err(FrameworkError::ValidationError(
                "Jev task must not be empty".to_string(),
            ));
        }
        let parent = caller.unwrap_or_else(|| Arc::clone(&self.owner));
        let unit = Arc::new(ExecutionUnit::new_child(UnitType::Jev, &parent)?);
        let run_id = format!("jev-{}", uuid::Uuid::new_v4());
        self.active_runs.insert(run_id.clone(), Arc::clone(&unit));
        let outcome = self
            .run_inner(
                &definition,
                task,
                conversation_id.clone(),
                agent_id,
                parent_run_id,
                &run_id,
                unit,
                executor,
            )
            .await;
        if let Err(error) = &outcome {
            let _ = self
                .publish(
                    JEV_RUN_COMPLETED_EVENT,
                    &run_id,
                    &definition,
                    &conversation_id,
                    json!({
                        "run_id": run_id,
                        "jev_name": definition.name,
                        "status": JevRunStatus::Failed,
                        "error": error.to_string(),
                    }),
                )
                .await;
        }
        self.active_runs.remove(&run_id);
        outcome
    }

    async fn run_inner(
        &self,
        definition: &JevDefinition,
        task: &str,
        conversation_id: Option<String>,
        agent_id: Option<String>,
        parent_run_id: Option<String>,
        run_id: &str,
        unit: Arc<ExecutionUnit>,
        executor: Arc<dyn JevToolExecutor>,
    ) -> Result<JevRunOutcome> {
        let mut snapshot = definition.initial_state.clone();
        if snapshot.is_null() {
            snapshot = json!({});
        }
        let object = snapshot.as_object_mut().ok_or_else(|| {
            FrameworkError::InvalidData("Jev initial_state must be a JSON object".to_string())
        })?;
        object.insert("task".to_string(), Value::String(task.to_string()));
        let mut revision = 0u64;
        unit.cache().set(SNAPSHOT_KEY, &snapshot, None).await?;
        unit.cache()
            .set(SNAPSHOT_REVISION_KEY, &revision, None)
            .await?;
        self.publish(
            JEV_RUN_STARTED_EVENT,
            run_id,
            definition,
            &conversation_id,
            json!({"task": task, "snapshot_revision": revision}),
        )
        .await?;

        let mut selected_action = String::new();
        for step in 1..=definition.max_steps {
            let JevDecision { action, confidence } =
                self.client.choose(definition, &snapshot).await?;
            selected_action = action.clone();
            let selected = definition.actions.get(&action).ok_or_else(|| {
                FrameworkError::InvalidData(format!("Jev selected unknown action '{action}'"))
            })?;
            self.publish(JEV_DECISION_EVENT, run_id, definition, &conversation_id, json!({"step": step, "action": action, "confidence": confidence, "snapshot_revision": revision})).await?;
            if let Some(status) = selected.terminal_status {
                let outcome = JevRunOutcome {
                    run_id: run_id.to_string(),
                    jev_name: definition.name.clone(),
                    status,
                    selected_action,
                    snapshot,
                    snapshot_revision: revision,
                    steps: step,
                    error: None,
                };
                self.publish(
                    JEV_RUN_COMPLETED_EVENT,
                    run_id,
                    definition,
                    &conversation_id,
                    serde_json::to_value(&outcome)?,
                )
                .await?;
                return Ok(outcome);
            }
            let tool = selected.tool.as_deref().ok_or_else(|| {
                FrameworkError::InvalidData(format!("Jev action '{action}' has no tool"))
            })?;
            let arguments = bind_arguments(&selected.arguments, task, &snapshot)?;
            let jev_context = JevExecutionContext {
                jev_name: definition.name.clone(),
                jev_run_id: run_id.to_string(),
                snapshot_revision: revision,
                conversation_id: conversation_id.clone(),
                agent_id: agent_id.clone(),
                parent_run_id: parent_run_id.clone(),
            };
            let ctx = build_context(&unit, &jev_context)?;
            self.publish(
                JEV_TOOL_STARTED_EVENT,
                run_id,
                definition,
                &conversation_id,
                json!({"step": step, "tool": tool, "snapshot_revision": revision}),
            )
            .await?;
            let result = executor
                .execute(tool, arguments, &ctx, &jev_context)
                .await?;
            self.publish(JEV_TOOL_COMPLETED_EVENT, run_id, definition, &conversation_id, json!({"step": step, "tool": tool, "error_code": result.error_code, "to_ai": result.to_ai, "result": result.result, "snapshot_revision": revision})).await?;
            apply_snapshot_update(&mut snapshot, &result.snapshot_update)?;
            revision = revision.checked_add(1).ok_or_else(|| {
                FrameworkError::InvalidOperation("Jev snapshot revision overflow".to_string())
            })?;
            unit.cache().set(SNAPSHOT_KEY, &snapshot, None).await?;
            unit.cache()
                .set(SNAPSHOT_REVISION_KEY, &revision, None)
                .await?;
            self.publish(JEV_SNAPSHOT_UPDATED_EVENT, run_id, definition, &conversation_id, json!({"step": step, "tool": tool, "snapshot_revision": revision, "update": result.snapshot_update})).await?;
        }
        let outcome = JevRunOutcome {
            run_id: run_id.to_string(),
            jev_name: definition.name.clone(),
            status: JevRunStatus::Failed,
            selected_action,
            snapshot,
            snapshot_revision: revision,
            steps: definition.max_steps,
            error: Some(format!(
                "Jev run exceeded max_steps {}",
                definition.max_steps
            )),
        };
        self.publish(
            JEV_RUN_COMPLETED_EVENT,
            run_id,
            definition,
            &conversation_id,
            serde_json::to_value(&outcome)?,
        )
        .await?;
        Ok(outcome)
    }

    async fn publish(
        &self,
        event_type: &str,
        run_id: &str,
        definition: &JevDefinition,
        conversation_id: &Option<String>,
        details: Value,
    ) -> Result<()> {
        let mut payload = json!({"schema":"agent-runtime-jev-trace/v1", "jev_name":definition.name, "jev_run_id":run_id, "run_id":run_id, "timestamp":chrono::Utc::now().to_rfc3339(), "details":details});
        if let Some(object) = payload.as_object_mut() {
            if let Some(id) = conversation_id {
                object.insert("conversation_id".to_string(), Value::String(id.clone()));
            }
        }
        let mut event = BaseEvent::new(event_type, payload);
        if let Some(id) = conversation_id {
            event = event.with_conversation_id(id.clone());
        }
        self.event_bus.publish(event).await
    }
}

fn validate_identity(conversation_id: &Option<String>, agent_id: &Option<String>) -> Result<()> {
    match (
        conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
        agent_id.as_deref().map(str::trim).filter(|v| !v.is_empty()),
    ) {
        (None, None) | (Some(_), Some(_)) => Ok(()),
        _ => Err(FrameworkError::ValidationError(
            "Jev conversation_id and agent_id must be provided together".to_string(),
        )),
    }
}

fn build_context(
    unit: &Arc<ExecutionUnit>,
    jev: &JevExecutionContext,
) -> Result<crate::orchestration::Context> {
    let mut ctx = unit.create_context();
    if let Some(id) = &jev.conversation_id {
        ctx = ctx.with_conversation_id(id.clone());
        ctx.set("conversation_id", id.clone())?;
    }
    if let Some(id) = &jev.agent_id {
        ctx.set("agent_id", id.clone())?;
    }
    ctx.set("jev", jev.clone())?;
    ctx.set("jev_name", jev.jev_name.clone())?;
    ctx.set("jev_run_id", jev.jev_run_id.clone())?;
    ctx.set("jev_snapshot_revision", jev.snapshot_revision)?;
    if let Some(id) = &jev.parent_run_id {
        ctx.set("parent_run_id", id.clone())?;
    }
    Ok(ctx)
}

fn bind_arguments(
    bindings: &BTreeMap<String, JevArgumentBinding>,
    task: &str,
    snapshot: &Value,
) -> Result<BTreeMap<String, Value>> {
    bindings
        .iter()
        .map(|(name, binding)| {
            let value = match binding {
                JevArgumentBinding::Task => Value::String(task.to_string()),
                JevArgumentBinding::Literal { value } => value.clone(),
                JevArgumentBinding::State { pointer } => {
                    snapshot.pointer(pointer).cloned().ok_or_else(|| {
                        FrameworkError::InvalidData(format!(
                            "Jev argument '{name}' state pointer '{pointer}' does not exist"
                        ))
                    })?
                }
            };
            Ok((name.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::InMemoryEventBus;
    use crate::jev::{JevAction, JevToolResult};
    use crate::world::FrameworkState;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::VecDeque;

    struct SequenceClient {
        actions: Mutex<VecDeque<String>>,
    }

    #[async_trait]
    impl JevClient for SequenceClient {
        async fn choose(&self, _definition: &JevDefinition, _state: &Value) -> Result<JevDecision> {
            let action = self.actions.lock().pop_front().ok_or_else(|| {
                FrameworkError::InvalidData("mock Jev action sequence exhausted".to_string())
            })?;
            Ok(JevDecision {
                action,
                confidence: Some(1.0),
            })
        }
    }

    struct UpdatingExecutor;

    #[async_trait]
    impl JevToolExecutor for UpdatingExecutor {
        async fn execute(
            &self,
            tool: &str,
            arguments: BTreeMap<String, Value>,
            _context: &crate::orchestration::Context,
            jev: &JevExecutionContext,
        ) -> Result<JevToolResult> {
            assert_eq!(tool, "GenerateRegex");
            assert_eq!(jev.snapshot_revision, 0);
            assert_eq!(arguments["requirement"], "find an order id");
            Ok(JevToolResult {
                result: json!({"pattern":"ORD-[0-9]{6}"}),
                to_ai: "generated".to_string(),
                error_code: 0,
                snapshot_update: crate::jev::JevSnapshotUpdate {
                    set: BTreeMap::from([("regex".to_string(), json!("ORD-[0-9]{6}"))]),
                    remove: vec!["matches".to_string()],
                },
            })
        }
    }

    fn definition() -> JevDefinition {
        JevDefinition {
            name: "text_matcher".to_string(),
            description: "choose the next text matching action".to_string(),
            model: "jev-latest".to_string(),
            initial_state: json!({"regex": null, "matches": ["stale"]}),
            instructions: "Choose the next valid action.".to_string(),
            actions: BTreeMap::from([
                (
                    "generate".to_string(),
                    JevAction {
                        description: "Generate a regex when none is available".to_string(),
                        tool: Some("GenerateRegex".to_string()),
                        arguments: BTreeMap::from([(
                            "requirement".to_string(),
                            JevArgumentBinding::Task,
                        )]),
                        terminal_status: None,
                    },
                ),
                (
                    "finish".to_string(),
                    JevAction {
                        description: "Finish when the regex is ready".to_string(),
                        tool: None,
                        arguments: BTreeMap::new(),
                        terminal_status: Some(JevRunStatus::Completed),
                    },
                ),
            ]),
            max_steps: 4,
        }
    }

    #[tokio::test]
    async fn run_updates_versioned_snapshot_and_releases_execution_unit() {
        let framework = FrameworkState::initialize().unwrap();
        let owner = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
        let manager = JevManager::new(
            owner,
            Arc::new(SequenceClient {
                actions: Mutex::new(VecDeque::from([
                    "generate".to_string(),
                    "finish".to_string(),
                ])),
            }),
            Arc::new(UpdatingExecutor),
            Arc::new(InMemoryEventBus::new()),
        );
        manager.register(definition()).unwrap();
        let outcome = manager
            .run(
                "text_matcher",
                "find an order id",
                Some("conversation-1".to_string()),
                Some("agent-1".to_string()),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(outcome.status, JevRunStatus::Completed);
        assert_eq!(outcome.snapshot_revision, 1);
        assert_eq!(outcome.snapshot["regex"], "ORD-[0-9]{6}");
        assert!(outcome.snapshot.get("matches").is_none());
        assert_eq!(manager.active_run_count(), 0);
    }

    #[tokio::test]
    async fn rejects_partial_conversation_identity() {
        let framework = FrameworkState::initialize().unwrap();
        let owner = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
        let manager = JevManager::new(
            owner,
            Arc::new(SequenceClient {
                actions: Mutex::new(VecDeque::new()),
            }),
            Arc::new(UpdatingExecutor),
            Arc::new(InMemoryEventBus::new()),
        );
        manager.register(definition()).unwrap();
        let error = manager
            .run(
                "text_matcher",
                "task",
                Some("conversation-1".to_string()),
                None,
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("must be provided together"));
    }
}
