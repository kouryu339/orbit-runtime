//! Conversation-safe Workflow catalog tools shared by normal agents and Studio.

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput, SimpleArgs};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::rpc_tool::RuntimeToolMetadata;
use corework::system::SystemOperation;
use corework::workflow::execution::WorkflowToAiMode;
use corework::workflow::workflows::executor::{WorkflowExecutionContext, WorkflowExecutionOutcome};
#[cfg(test)]
use corework::workflow::workflows::WorkflowValidation;
use corework::workflow::workflows::{WorkflowEditorSession, WorkflowResourceKind, WorkflowsModule};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{Arc, Weak};
use std::time::Instant;

#[derive(Clone, Default)]
pub struct WorkflowRuntimeToolCatalog {
    registry: Option<Arc<corework::system::SystemRegistry>>,
    tools: Vec<RuntimeToolMetadata>,
}

impl WorkflowRuntimeToolCatalog {
    pub fn new(tools: Vec<RuntimeToolMetadata>) -> Self {
        Self {
            tools,
            registry: None,
        }
    }

    pub fn from_registry(registry: Arc<corework::system::SystemRegistry>) -> Self {
        Self {
            registry: Some(registry),
            tools: Vec::new(),
        }
    }
    /// Read the current metadata catalog; this does not grant tool access.
    pub fn tools(&self) -> Vec<RuntimeToolMetadata> {
        self.registry
            .as_ref()
            .map(|registry| registry.runtime_tools())
            .unwrap_or_else(|| self.tools.clone())
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowModuleHandle {
    module: Weak<WorkflowsModule>,
}

impl WorkflowModuleHandle {
    pub fn new(module: &Arc<WorkflowsModule>) -> Self {
        Self {
            module: Arc::downgrade(module),
        }
    }

    fn upgrade(&self) -> Result<Arc<WorkflowsModule>, FrameworkError> {
        self.module.upgrade().ok_or_else(|| {
            FrameworkError::NotFoundError("Workflow runtime is no longer available".to_string())
        })
    }
}

fn workflows(ctx: &Context) -> Result<Arc<WorkflowsModule>, FrameworkError> {
    match ctx.resolve_shared_component::<WorkflowsModule>() {
        Ok(module) => Ok(module),
        Err(_) => ctx
            .resolve_shared_component::<WorkflowModuleHandle>()?
            .upgrade(),
    }
}

fn editor_session(ctx: &Context) -> Option<Arc<WorkflowEditorSession>> {
    ctx.resolve_shared_component::<WorkflowEditorSession>().ok()
}

fn runtime_tools(ctx: &Context) -> Vec<RuntimeToolMetadata> {
    ctx.resolve_shared_component::<WorkflowRuntimeToolCatalog>()
        .map(|catalog| catalog.tools())
        .unwrap_or_default()
}

async fn workflow_execution_context(
    ctx: &Context,
) -> Result<WorkflowExecutionContext, FrameworkError> {
    let conversation_id = match ctx.conversation_id.clone() {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => ctx
            .cache
            .get::<String>(crate::state_machine::agent_keys::CONVERSATION_ID)
            .await?
            .filter(|value| !value.trim().is_empty()),
    };
    let agent_id = match ctx.get::<String>("agent_id")? {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => ctx
            .cache
            .get::<String>(crate::state_machine::agent_keys::AGENT_ID)
            .await?
            .filter(|value| !value.trim().is_empty()),
    };
    let turn_id = match ctx.get::<Value>("turn_id")? {
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => ctx
            .cache
            .get::<u64>(crate::context::keys::TURN_ID)
            .await?
            .map(|value| value.to_string()),
    };
    Ok(WorkflowExecutionContext {
        conversation_id,
        agent_id,
        turn_id,
    })
}

async fn active_runtime_tools(ctx: &Context) -> Result<Vec<RuntimeToolMetadata>, AIOutput> {
    let active_tools = crate::AssistantContext::get_active_tools(&ctx.cache)
        .await
        .map_err(|error| {
            AIOutput::error(
                500,
                format!("unable to verify active Workflow node tools: {error}"),
            )
        })?
        .into_iter()
        .collect::<HashSet<_>>();
    let mut tools: Vec<_> = runtime_tools(ctx)
        .into_iter()
        .filter(|tool| tool.workflow_enabled && active_tools.contains(&tool.name))
        .collect();
    tools.extend(
        workflows(ctx)
            .and_then(|module| module.workflow_reference_tools())
            .map_err(|error| AIOutput::error(500, error.to_string()))?,
    );
    Ok(tools)
}

fn sync_selection(ctx: &Context, workflow_id: &str, revision: u64) {
    if let Some(session) = editor_session(ctx) {
        session.select(workflow_id.to_string(), revision);
    }
}

fn clear_selection_if_deleted(ctx: &Context, workflow_id: &str) {
    if let Some(session) = editor_session(ctx) {
        if session
            .selection()
            .is_some_and(|selection| selection.workflow_id == workflow_id)
        {
            session.clear();
        }
    }
}

fn required_revision(args: &SimpleArgs) -> Result<u64, AIOutput> {
    let raw = args
        .safe_require("expected_revision")?
        .parse::<u64>()
        .map_err(|_| AIOutput::error(400, "expected_revision must be a non-negative integer"))?;
    Ok(raw)
}

fn parse_kind(value: Option<&str>) -> Result<Option<WorkflowResourceKind>, AIOutput> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("all") => Ok(None),
        Some("draft") => Ok(Some(WorkflowResourceKind::Draft)),
        Some("registered") => Ok(Some(WorkflowResourceKind::Registered)),
        Some(other) => Err(AIOutput::error(
            400,
            format!("kind must be all, draft, or registered; got '{other}'"),
        )),
    }
}

fn parse_inputs(args: &SimpleArgs, reserved: &[&str]) -> Result<HashMap<String, Value>, AIOutput> {
    let mut inputs = HashMap::new();
    let mut invalid = Vec::new();
    for key in args.keys() {
        if let Some(name) = key.strip_prefix("input.") {
            if name.is_empty() {
                return Err(AIOutput::error(
                    400,
                    "workflow input name must not be empty",
                ));
            }
            let raw = args.get(key).unwrap_or_default();
            let value =
                serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
            inputs.insert(name.to_string(), value);
        } else if !reserved.contains(&key) {
            invalid.push(format!("--{key}"));
        }
    }
    if invalid.is_empty() {
        Ok(inputs)
    } else {
        Err(AIOutput::error(
            400,
            format!(
                "workflow inputs must use --input.<name>; invalid argument(s): {}",
                invalid.join(", ")
            ),
        ))
    }
}

#[cfg(test)]
async fn compile_script(
    script: &str,
    ctx: &Context,
) -> Result<corework::workflow::blueprint_json::BlueprintJson, AIOutput> {
    let script = script.replace(r"\n", "\n").replace(r#"\""#, "\"");
    if script.trim().is_empty() {
        return Err(AIOutput::error(400, "script must not be empty"));
    }
    let runtime_tools = active_runtime_tools(ctx).await?;
    corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
        &script,
        &runtime_tools,
    )
    .map_err(|error| compile_error_output(&error))
}

#[cfg(test)]
fn compile_error_output(error: &corework::workflow::chain_compiler::ChainError) -> AIOutput {
    let mut validation = WorkflowValidation::from_compile_error(error);
    for diagnostic in &mut validation.diagnostics {
        if diagnostic.kind == "unknown_operation" && diagnostic.suggestion.is_none() {
            diagnostic.suggestion = Some("Ensure the referenced tool appears in this agent's active tools and has explicit registered description, input pins, and output pins.".to_string());
        }
    }
    let result = json!(validation);
    // Include structured details in to_ai as well: text-only transports use this field.
    let mut output = AIOutput::error(400, result.to_string());
    output.result = result;
    output
}

fn execution_output(
    outcome: WorkflowExecutionOutcome,
    duration_ms: u128,
    trace_enabled: bool,
) -> AIOutput {
    let mut result = json!({
        "outputs": outcome.report.outputs_json(),
        "duration_ms": duration_ms,
    });
    if trace_enabled || outcome.error.is_some() {
        result["trace"] = serde_json::to_value(&outcome.report.trace).unwrap_or(Value::Null);
    }
    if let Some(error) = outcome.error.as_ref() {
        result["error"] = Value::String(error.clone());
    }
    let to_ai = outcome.report.to_ai(
        if trace_enabled {
            WorkflowToAiMode::Detailed
        } else {
            WorkflowToAiMode::DetailedOnError
        },
        outcome.error.as_deref(),
    );
    match outcome.error {
        Some(_) => AIOutput::error(-1, to_ai),
        None => AIOutput::success(result, to_ai),
    }
}

#[define_operation(
    name = "listWorkflows",
    workflow_enabled = false,
    display_name = "列出工作流{kind}并返回{workflows}", category = "Workflow",
    description = "List accepted Workflow resources, including drafts and registered workflows. Returns identity, metadata, state, revision and validation summary; does not return source or execute nodes.",
    params { kind: "String@Optional filter: all, draft, or registered. Defaults to all." },
    outputs { workflows: "Array@Workflow resource summaries with stable ids, state and revision." },
    destructive = false, readonly = true, idempotent = true, open_world = false
)]
pub struct ListWorkflows;

#[async_trait]
impl SystemOperation for ListWorkflows {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let kind = match parse_kind(args.get("kind")) {
            Ok(kind) => kind,
            Err(error) => return Ok(error),
        };
        let resources = workflows(ctx)?.list_workflow_catalog(kind)?;
        Ok(AIOutput::success(
            json!({ "workflows": resources }),
            format!("Found {} Workflow resource(s).", resources.len()),
        ))
    }

    fn name(&self) -> &str {
        "listWorkflows"
    }
}

#[cfg(test)]
#[deprecated(
    note = "Legacy script API is disabled externally; retained only for compatibility tests."
)]
#[derive(Default)]
pub(crate) struct CreateWorkflowDraft;

#[cfg(test)]
#[allow(deprecated)]
#[async_trait]
impl SystemOperation for CreateWorkflowDraft {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let name = match args.safe_require("name") {
            Ok(name) => name,
            Err(error) => return Ok(error),
        };
        let script = match args.safe_require("script") {
            Ok(script) => script.replace(r"\n", "\n").replace(r#"\""#, "\""),
            Err(error) => return Ok(error),
        };
        let mut blueprint = match compile_script(&script, ctx).await {
            Ok(blueprint) => blueprint,
            Err(error) => return Ok(error),
        };
        blueprint.metadata.name = name.clone();
        blueprint.metadata.description = args.get("description").unwrap_or_default().to_string();
        if let Some(id) = args.get("workflow_id").filter(|id| !id.trim().is_empty()) {
            blueprint.metadata.id = id.trim().to_string();
        }
        let resource = match workflows(ctx)?
            .create_draft_resource(
                args.get("workflow_id"),
                &name,
                args.get("description").unwrap_or_default(),
                Some(script),
                Some(blueprint),
                WorkflowValidation::valid(),
            )
            .await
        {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(409, error.to_string())),
        };
        sync_selection(ctx, &resource.summary.id, resource.summary.revision);
        Ok(AIOutput::success(
            json!({
                "workflow": serde_json::to_value(&resource)
                    .map_err(FrameworkError::SerializationError)?
            }),
            format!(
                "Created Workflow Draft '{}' at revision 1.",
                resource.summary.name
            ),
        ))
    }

    fn name(&self) -> &str {
        "createWorkflowDraft"
    }
}

#[cfg(test)]
#[deprecated(
    note = "Legacy script API is disabled externally; retained only for compatibility tests."
)]
#[derive(Default)]
pub(crate) struct UpdateWorkflow;

#[cfg(test)]
#[allow(deprecated)]
#[async_trait]
impl SystemOperation for UpdateWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let id = match args.safe_require("workflow_id") {
            Ok(id) => id,
            Err(error) => return Ok(error),
        };
        let expected_revision = match required_revision(&args) {
            Ok(value) => value,
            Err(error) => return Ok(error),
        };
        let script = match args.safe_require("script") {
            Ok(script) => script.replace(r"\n", "\n").replace(r#"\""#, "\""),
            Err(error) => return Ok(error),
        };
        let module = workflows(ctx)?;
        match module.read_workflow_resource(&id) {
            Ok(_) => {}
            Err(error) => return Ok(AIOutput::error(404, error.to_string())),
        }
        let tools = match active_runtime_tools(ctx).await {
            Ok(tools) => tools,
            Err(error) => return Ok(error),
        };
        let request = corework::workflow::workflows::revision::RevisionRequest {
            id: id.clone(),
            expected_revision,
            change: corework::workflow::workflows::revision::WorkflowChange::Script(script),
            name: args
                .get("name")
                .filter(|name| !name.trim().is_empty())
                .map(str::to_string),
            description: args.get("description").map(str::to_string),
        };
        let updated = module
            .revise_workflow(request, &tools, true)
            .await
            .and_then(|value| {
                serde_json::from_value::<corework::workflow::workflows::WorkflowResourceView>(value)
                    .map_err(FrameworkError::SerializationError)
            });
        let updated = match updated {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(409, error.to_string())),
        };
        sync_selection(ctx, &updated.summary.id, updated.summary.revision);
        Ok(AIOutput::success(
            json!({
                "workflow": serde_json::to_value(&updated)
                    .map_err(FrameworkError::SerializationError)?
            }),
            format!(
                "Updated Workflow '{}' to revision {}.",
                updated.summary.name, updated.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "updateWorkflow"
    }
}

#[cfg(test)]
#[deprecated(note = "Legacy Workflow API is no longer exposed.")]
#[derive(Default)]
pub(crate) struct CompileWorkflow;

#[cfg(test)]
#[allow(deprecated)]
#[async_trait]
impl SystemOperation for CompileWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        if args.get("script").is_some() {
            return Ok(AIOutput::error(
                410,
                "Legacy script compilation is deprecated and disabled; supply workflow_id.",
            ));
        }
        let id = match args.safe_require("workflow_id") {
            Ok(id) => id,
            Err(error) => return Ok(error),
        };
        let resource = match workflows(ctx)?.read_workflow_resource(&id) {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(404, error.to_string())),
        };
        Ok(AIOutput::success(
            json!({
                "workflow_id": resource.summary.id,
                "kind": resource.summary.kind,
                "revision": resource.summary.revision,
                "validation": resource.summary.validation,
                "blueprint": resource.blueprint,
            }),
            format!(
                "Workflow '{}' is valid at revision {}.",
                resource.summary.name, resource.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "compileWorkflow"
    }
}

#[define_operation(
    name = "promoteWorkflow",
    workflow_enabled = false,
    display_name = "将工作流{workflow_id}的版本{expected_revision}转正并返回{workflow}",
    category = "Workflow",
    description = "Promote a valid Draft to a trusted Registered Workflow using optimistic revision control.",
    params {
        workflow_id: "String@Stable Workflow resource id. Required.",
        expected_revision: "u64@Current revision. Required."
    },
    outputs { workflow: "Object@Registered Workflow resource." },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct PromoteWorkflow;

#[async_trait]
impl SystemOperation for PromoteWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let id = match args.safe_require("workflow_id") {
            Ok(id) => id,
            Err(error) => return Ok(error),
        };
        let revision = match required_revision(&args) {
            Ok(value) => value,
            Err(error) => return Ok(error),
        };
        let resource = match workflows(ctx)?
            .register_draft_resource(&id, Some(revision), None)
            .await
        {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(409, error.to_string())),
        };
        sync_selection(ctx, &resource.summary.id, resource.summary.revision);
        Ok(AIOutput::success(
            json!({
                "workflow": serde_json::to_value(&resource)
                    .map_err(FrameworkError::SerializationError)?
            }),
            format!(
                "Registered Workflow '{}' at revision {}.",
                resource.summary.name, resource.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "promoteWorkflow"
    }
}

#[define_operation(
    name = "removeWorkflow",
    workflow_enabled = false,
    display_name = "删除工作流{workflow_id}的版本{expected_revision}并返回{workflow}",
    category = "Workflow",
    description = "Delete a Draft or Registered Workflow using optimistic revision control.",
    params {
        workflow_id: "String@Stable Workflow resource id. Required.",
        expected_revision: "u64@Current revision. Required."
    },
    outputs { workflow: "Object@Deleted Workflow summary." },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RemoveWorkflow;

#[async_trait]
impl SystemOperation for RemoveWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let id = match args.safe_require("workflow_id") {
            Ok(id) => id,
            Err(error) => return Ok(error),
        };
        let revision = match required_revision(&args) {
            Ok(value) => value,
            Err(error) => return Ok(error),
        };
        let deleted = match workflows(ctx)?
            .delete_catalog_resource(&id, Some(revision))
            .await
        {
            Ok(deleted) => deleted,
            Err(error) => return Ok(AIOutput::error(409, error.to_string())),
        };
        clear_selection_if_deleted(ctx, &id);
        Ok(AIOutput::success(
            json!({
                "workflow": serde_json::to_value(&deleted)
                    .map_err(FrameworkError::SerializationError)?
            }),
            format!("Deleted Workflow '{}'.", deleted.name),
        ))
    }

    fn name(&self) -> &str {
        "removeWorkflow"
    }
}

async fn execute_resource(
    id: &str,
    inputs: HashMap<String, Value>,
    trace: bool,
    ctx: &Context,
) -> Result<AIOutput, FrameworkError> {
    let module = workflows(ctx)?;
    let resource = match module.read_workflow_resource(id) {
        Ok(resource) => resource,
        Err(error) => return Ok(AIOutput::error(404, error.to_string())),
    };
    let started = Instant::now();
    let execution_context = workflow_execution_context(ctx).await?;
    let execution = match resource.summary.kind {
        WorkflowResourceKind::Draft => {
            let blueprint = match resource.blueprint {
                Some(blueprint) => blueprint,
                None => return Ok(AIOutput::error(400, "Draft has no compiled blueprint")),
            };
            module
                .execute_from_blueprint_outcome_with_context(blueprint, inputs, &execution_context)
                .await
        }
        WorkflowResourceKind::Registered => {
            module
                .execute_registered_outcome_with_context(id, inputs, &execution_context)
                .await
        }
    };
    match execution {
        Ok(outcome) => Ok(execution_output(
            outcome,
            started.elapsed().as_millis(),
            trace,
        )),
        Err(error) => Ok(AIOutput::error(-1, error.to_string())),
    }
}

#[cfg(test)]
#[deprecated(note = "Legacy Workflow API is no longer exposed.")]
#[derive(Default)]
pub(crate) struct TestWorkflow;

#[cfg(test)]
#[allow(deprecated)]
#[async_trait]
impl SystemOperation for TestWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let id = match args.safe_require("workflow_id") {
            Ok(id) => id,
            Err(error) => return Ok(error),
        };
        let inputs = match parse_inputs(&args, &["workflow_id", "trace", "inputs"]) {
            Ok(inputs) => inputs,
            Err(error) => return Ok(error),
        };
        execute_resource(&id, inputs, args.get_bool("trace"), ctx).await
    }

    fn name(&self) -> &str {
        "testWorkflow"
    }
}

#[define_operation(
    name = "executeWorkflow",
    workflow_enabled = false,
    display_name = "执行已转正工作流{workflow_id}并用{inputs}返回输出{outputs}和追踪{trace}",
    category = "Workflow",
    description = "Execute a trusted Workflow by stable id; its JSON state determines draft or production execution. Inputs use --input.<name>.",
    params {
        workflow_id: "String@Stable Workflow resource id. Required.",
        trace: "bool@Include structured trace when true.",
        inputs: "String@Marker: pass Workflow inputs as --input.<name>."
    },
    outputs { outputs: "Object@Workflow return values.", trace: "Array@Structured node trace when requested or failed." },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct ExecuteWorkflow;

#[async_trait]
impl SystemOperation for ExecuteWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let id = match args.safe_require("workflow_id") {
            Ok(id) => id,
            Err(error) => return Ok(error),
        };
        let inputs = match parse_inputs(&args, &["workflow_id", "trace", "inputs"]) {
            Ok(inputs) => inputs,
            Err(error) => return Ok(error),
        };
        execute_resource(&id, inputs, args.get_bool("trace"), ctx).await
    }

    fn name(&self) -> &str {
        "executeWorkflow"
    }
}

#[cfg(test)]
#[deprecated(
    note = "Legacy script API is disabled externally; retained only for compatibility tests."
)]
#[derive(Default)]
pub(crate) struct ExecuteWorkflowScript;

#[cfg(test)]
#[allow(deprecated)]
#[async_trait]
impl SystemOperation for ExecuteWorkflowScript {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let script = match args.safe_require("script") {
            Ok(script) => script,
            Err(error) => return Ok(error),
        };
        let blueprint = match compile_script(&script, ctx).await {
            Ok(blueprint) => blueprint,
            Err(error) => return Ok(error),
        };
        let inputs = match parse_inputs(&args, &["script", "trace", "inputs"]) {
            Ok(inputs) => inputs,
            Err(error) => return Ok(error),
        };
        let started = Instant::now();
        let execution_context = workflow_execution_context(ctx).await?;
        match workflows(ctx)?
            .execute_from_blueprint_outcome_with_context(blueprint, inputs, &execution_context)
            .await
        {
            Ok(outcome) => Ok(execution_output(
                outcome,
                started.elapsed().as_millis(),
                args.get_bool("trace"),
            )),
            Err(error) => Ok(AIOutput::error(-1, error.to_string())),
        }
    }

    fn name(&self) -> &str {
        "executeWorkflowScript"
    }
}

#[define_operation(
    name = "registerWorkflow",
    workflow_enabled = false,
    display_name = "注册工作流文档{source}并返回{workflow}", category = "Workflow",
    description = "Register or replace a complete orbit.workflow.resource/v1 source document. JSON state selects draft or registered; revision is the expected current version, or 1 for a new identity. Existing state transitions require promoteWorkflow.",
    params { source: "String@Complete resource source text, including metadata, state, revision and logic. Required." },
    outputs { workflow: "Object@Accepted identity, state, revision, metadata and validation summary." },
    destructive = true, readonly = false, idempotent = false, open_world = false
)]
pub struct RegisterWorkflow;

#[async_trait]
impl SystemOperation for RegisterWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        resource_source_operation(input, ctx, true).await
    }
    fn name(&self) -> &str {
        "registerWorkflow"
    }
}

#[define_operation(
    name = "validateWorkflow",
    workflow_enabled = false,
    display_name = "验证工作流文档{source}并返回{validation}", category = "Workflow",
    description = "Statically validate a complete resource source document, tool contracts and references without saving or executing it. No prior draft is required.",
    params { source: "String@Complete orbit.workflow.resource/v1 source text. Required." },
    outputs { validation: "Object@Validation result with identity, state, revision and input/output contracts." },
    destructive = false, readonly = true, idempotent = true, open_world = false
)]
pub struct ValidateWorkflow;

#[async_trait]
impl SystemOperation for ValidateWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        resource_source_operation(input, ctx, false).await
    }
    fn name(&self) -> &str {
        "validateWorkflow"
    }
}

async fn resource_source_operation(
    input: AIInput,
    ctx: &Context,
    commit: bool,
) -> Result<AIOutput, FrameworkError> {
    let args = match input.safe_parse_args() {
        Ok(args) => args,
        Err(error) => return Ok(error),
    };
    let source = match args.safe_require("source") {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let tools = match active_runtime_tools(ctx).await {
        Ok(tools) => tools,
        Err(error) => return Ok(error),
    };
    match workflows(ctx)?
        .register_logic_resource(&source, &tools, commit)
        .await
    {
        Ok(result) => {
            if commit {
                sync_selection(
                    ctx,
                    result["id"].as_str().unwrap(),
                    result["revision"].as_u64().unwrap(),
                );
            }
            let output = if commit {
                json!({"workflow":result})
            } else {
                json!({"validation":result})
            };
            Ok(AIOutput::success(
                output,
                if commit {
                    "Workflow resource registered."
                } else {
                    "Workflow source is valid; no changes saved."
                },
            ))
        }
        Err(error) => Ok(AIOutput::error(400, error.to_string())),
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    #[test]
    fn compile_errors_reach_both_ai_transports_as_structured_diagnostics() {
        let error = corework::workflow::chain_compiler_v2::parse_v2(
            "input\n2: EXEC DebugPrintNode --Value \"a\"\n4: EXEC DebugPrintNode --Value \"b\"\nreturn"
        ).unwrap_err();
        let output = super::compile_error_output(&error);
        assert_eq!(output.result["diagnostics"].as_array().unwrap().len(), 2);
        assert_eq!(
            output.result["diagnostics"][0]["numbering"]["expected"],
            "1"
        );
        let text: serde_json::Value = serde_json::from_str(&output.to_ai).unwrap();
        assert_eq!(text, output.result);
        assert!(!output.to_ai.contains("Ensure every referenced tool"));
    }
    use super::*;
    use corework::event::InMemoryEventBus;
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::world::FrameworkState;

    fn input(values: &[(&str, &str)]) -> AIInput {
        AIInput::from_args(
            values
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        )
    }

    fn test_context_with_runtime_tools(
        runtime_tools: Vec<RuntimeToolMetadata>,
    ) -> (
        Context,
        Arc<ExecutionUnit>,
        Arc<WorkflowsModule>,
        Arc<WorkflowEditorSession>,
    ) {
        let framework = FrameworkState::initialize().unwrap();
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            format!("workflow-tools-{}", uuid::Uuid::new_v4().simple()),
        ));
        let directory = std::env::temp_dir().join(format!(
            "ai-assistant-workflow-tools-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let workflows = Arc::new(
            WorkflowsModule::new_with_event_bus(directory, Arc::new(InMemoryEventBus::new()))
                .unwrap(),
        );
        let selection = Arc::new(WorkflowEditorSession::new(Vec::new()));
        unit.attach_shared_component(Arc::clone(&workflows))
            .unwrap();
        unit.attach_shared_component(Arc::clone(&selection))
            .unwrap();
        unit.attach_shared_component(Arc::new(WorkflowRuntimeToolCatalog::new(runtime_tools)))
            .unwrap();
        (unit.create_context(), unit, workflows, selection)
    }

    fn test_context() -> (
        Context,
        Arc<ExecutionUnit>,
        Arc<WorkflowsModule>,
        Arc<WorkflowEditorSession>,
    ) {
        test_context_with_runtime_tools(Vec::new())
    }

    #[tokio::test]
    async fn agent_workflow_tool_preserves_agent_origin_context() {
        let _guard = crate::test_support::global_test_guard().await;
        let (ctx, _unit, _workflows, _selection) = test_context();
        let ctx = ctx.with_conversation_id("conversation-1");
        ctx.cache
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &"agent-1".to_string(),
                None,
            )
            .await
            .unwrap();
        ctx.cache
            .set(crate::context::keys::TURN_ID, &9u64, None)
            .await
            .unwrap();

        let propagated = workflow_execution_context(&ctx).await.unwrap();
        assert_eq!(
            propagated.conversation_id.as_deref(),
            Some("conversation-1")
        );
        assert_eq!(propagated.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(propagated.turn_id.as_deref(), Some("9"));
    }

    #[tokio::test]
    async fn unified_tools_cover_draft_crud_registration_and_execution() {
        let _guard = crate::test_support::global_test_guard().await;
        let (ctx, _unit, workflows, selection) = test_context();
        let created = CreateWorkflowDraft
            .execute(
                input(&[
                    ("workflow_id", "echo-flow"),
                    ("name", "Echo Flow"),
                    ("script", "input name:String\nreturn result=input.name"),
                ]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(created.error_code, 0, "{}", created.to_ai);
        assert_eq!(created.result["workflow"]["id"], "echo-flow");
        assert_eq!(created.result["workflow"]["kind"], "draft");
        assert_eq!(selection.selection().unwrap().workflow_id, "echo-flow");

        let listed = ListWorkflows
            .execute(input(&[("kind", "draft")]), &ctx)
            .await
            .unwrap();
        assert_eq!(listed.result["workflows"].as_array().unwrap().len(), 1);

        let conflict = UpdateWorkflow
            .execute(
                input(&[
                    ("workflow_id", "echo-flow"),
                    ("expected_revision", "9"),
                    ("script", "input name:String\nreturn result=input.name"),
                ]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(conflict.error_code, 409);

        let updated = UpdateWorkflow
            .execute(
                input(&[
                    ("workflow_id", "echo-flow"),
                    ("expected_revision", "1"),
                    ("script", "input name:String\nreturn result=input.name"),
                ]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(updated.error_code, 0, "{}", updated.to_ai);
        assert_eq!(updated.result["workflow"]["revision"], 2);

        let tested = TestWorkflow
            .execute(
                input(&[
                    ("workflow_id", "echo-flow"),
                    ("input.name", "Ada"),
                    ("trace", "true"),
                ]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(tested.error_code, 0, "{}", tested.to_ai);
        assert_eq!(tested.result["outputs"]["result"], "Ada");

        let registered = PromoteWorkflow
            .execute(
                input(&[("workflow_id", "echo-flow"), ("expected_revision", "2")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(registered.error_code, 0, "{}", registered.to_ai);
        assert_eq!(registered.result["workflow"]["kind"], "registered");
        assert_eq!(registered.result["workflow"]["revision"], 3);

        let executed = ExecuteWorkflow
            .execute(
                input(&[("workflow_id", "echo-flow"), ("input.name", "Grace")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(executed.error_code, 0, "{}", executed.to_ai);
        assert_eq!(executed.result["outputs"]["result"], "Grace");

        let deleted = RemoveWorkflow
            .execute(
                input(&[("workflow_id", "echo-flow"), ("expected_revision", "3")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(deleted.error_code, 0, "{}", deleted.to_ai);
        assert!(selection.selection().is_none());
        assert!(workflows.read_workflow_resource("echo-flow").is_err());
    }

    #[tokio::test]
    async fn temporary_script_returns_outputs_without_creating_a_resource() {
        let _guard = crate::test_support::global_test_guard().await;
        let (ctx, _unit, workflows, _) = test_context();
        let output = ExecuteWorkflowScript
            .execute(
                input(&[
                    ("script", "input value:String\nreturn result=input.value"),
                    ("input.value", "temporary"),
                ]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.error_code, 0, "{}", output.to_ai);
        assert_eq!(output.result["outputs"]["result"], "temporary");
        assert!(workflows.list_workflow_catalog(None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn temporary_script_rejects_runtime_tools_not_active_for_the_agent() {
        let _guard = crate::test_support::global_test_guard().await;
        let hidden_tool = serde_json::from_value(json!({
            "name": "HiddenTool",
            "description": "A tool that is registered but not active for this Agent."
        }))
        .unwrap();
        let (ctx, _unit, _workflows, _) = test_context_with_runtime_tools(vec![hidden_tool]);
        crate::AssistantContext::set_active_tools(&ctx.cache, Vec::new())
            .await
            .unwrap();

        let output = ExecuteWorkflowScript
            .execute(
                input(&[(
                    "script",
                    "input value:String\n1: EXEC HiddenTool\nreturn result=input.value",
                )]),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.error_code, 400);
        assert!(output.to_ai.contains("active tools"), "{}", output.to_ai);
        assert!(
            output.to_ai.contains("registered description"),
            "{}",
            output.to_ai
        );
    }

    #[test]
    fn workflow_execution_tools_are_destructive() {
        for tool in ["executeWorkflow"] {
            let metadata = crate::tool_runner::permission_metadata(tool).unwrap();
            assert_eq!(metadata.effect, crate::ToolEffect::Destructive, "{tool}");
        }
    }

    #[test]
    fn workflow_tool_catalog_contains_exactly_six_operations() {
        let mut names = inventory::iter::<corework::ai_system::AISystemFactory>
            .into_iter()
            .filter(|factory| {
                factory
                    .metadata
                    .name
                    .to_ascii_lowercase()
                    .contains("workflow")
            })
            .map(|factory| factory.metadata.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "executeWorkflow",
                "listWorkflows",
                "promoteWorkflow",
                "registerWorkflow",
                "removeWorkflow",
                "validateWorkflow"
            ]
        );
        assert_eq!(
            crate::tool_runner::permission_metadata("validateWorkflow")
                .unwrap()
                .effect,
            crate::ToolEffect::ReadOnly
        );
        assert_eq!(
            crate::tool_runner::permission_metadata("listWorkflows")
                .unwrap()
                .effect,
            crate::ToolEffect::ReadOnly
        );
    }

    #[tokio::test]
    async fn resource_tools_list_validate_register_execute_promote_and_remove_documents() {
        let _guard = crate::test_support::global_test_guard().await;
        let (ctx, _unit, module, _) = test_context();
        let source = r#"{"schema":"orbit.workflow.resource/v1","id":"logic-flow","metadata":{"name":"Logic Flow","description":"Echo","extensions":{}},"state":"draft","revision":1,"logic":{"schema":"orbit.workflow.logic/v1","inputs":{"name":{"type":"String"}},"steps":[],"returns":{"result":{"value":input.name}}}}"#;
        let validation = ValidateWorkflow
            .execute(input(&[("source", source)]), &ctx)
            .await
            .unwrap();
        assert_eq!(validation.error_code, 0, "{}", validation.to_ai);
        assert!(module.list_workflow_catalog(None).unwrap().is_empty());
        let registered = RegisterWorkflow
            .execute(input(&[("source", source)]), &ctx)
            .await
            .unwrap();
        assert_eq!(registered.error_code, 0, "{}", registered.to_ai);
        assert_eq!(registered.result["workflow"]["state"], "draft");
        let listed = ListWorkflows
            .execute(input(&[("kind", "draft")]), &ctx)
            .await
            .unwrap();
        assert_eq!(listed.error_code, 0);
        assert_eq!(listed.result["workflows"][0]["id"], "logic-flow");
        assert_eq!(listed.result["workflows"][0]["revision"], 1);
        assert!(ListWorkflows
            .execute(input(&[("kind", "registered")]), &ctx)
            .await
            .unwrap()
            .result["workflows"]
            .as_array()
            .unwrap()
            .is_empty());
        let executed = ExecuteWorkflow
            .execute(
                input(&[("workflow_id", "logic-flow"), ("input.name", "Draft")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(executed.result["outputs"]["result"], "Draft");
        let invalid = source.replace("input.name", "missing(1)");
        assert_ne!(
            RegisterWorkflow
                .execute(input(&[("source", &invalid)]), &ctx)
                .await
                .unwrap()
                .error_code,
            0
        );
        assert_eq!(
            module
                .read_workflow_resource("logic-flow")
                .unwrap()
                .summary
                .revision,
            1
        );
        let changed_state = source.replace("\"state\":\"draft\"", "\"state\":\"registered\"");
        assert!(module
            .register_logic_resource(&changed_state, &[], true)
            .await
            .is_err());
        let (first, second) = tokio::join!(
            module.register_logic_resource(source, &[], true),
            module.register_logic_resource(source, &[], true),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let promoted = PromoteWorkflow
            .execute(
                input(&[("workflow_id", "logic-flow"), ("expected_revision", "2")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(promoted.error_code, 0, "{}", promoted.to_ai);
        assert_eq!(promoted.result["workflow"]["revision"], 3);
        let listed = ListWorkflows
            .execute(input(&[("kind", "registered")]), &ctx)
            .await
            .unwrap();
        assert_eq!(listed.result["workflows"][0]["kind"], "registered");
        assert_eq!(listed.result["workflows"][0]["revision"], 3);
        assert_ne!(
            ListWorkflows
                .execute(input(&[("kind", "invalid")]), &ctx)
                .await
                .unwrap()
                .error_code,
            0
        );
        let removed = RemoveWorkflow
            .execute(
                input(&[("workflow_id", "logic-flow"), ("expected_revision", "3")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(removed.error_code, 0, "{}", removed.to_ai);
        assert!(module.list_workflow_catalog(None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn compile_tool_rejects_script_even_with_resource_id() {
        let _guard = crate::test_support::global_test_guard().await;
        let (ctx, _unit, _workflows, _selection) = test_context();
        let output = CompileWorkflow
            .execute(
                input(&[("script", "return"), ("workflow_id", "existing")]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(output.error_code, 410);
    }

    #[tokio::test]
    async fn grep_is_rejected_by_workflow_scripts_even_when_ai_active() {
        let _guard = crate::test_support::global_test_guard().await;
        let (ctx, _unit, _workflows, _) = test_context();
        crate::AssistantContext::set_active_tools(&ctx.cache, vec!["Grep".into()])
            .await
            .unwrap();
        let output = ExecuteWorkflowScript.execute(input(&[("script",
            "input value:String\n1: EXEC Grep --root_id project --pattern needle\nreturn result=input.value")]), &ctx).await.unwrap();
        assert_ne!(
            output.error_code, 0,
            "AI-only tool must never execute from a script"
        );
    }
}
