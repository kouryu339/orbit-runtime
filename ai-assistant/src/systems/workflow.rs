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
use corework::workflow::workflows::{
    preserve_workflow_blueprint_layout, WorkflowEditorSession, WorkflowResourceKind,
    WorkflowValidation, WorkflowsModule,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct WorkflowRuntimeToolCatalog {
    tools: Vec<RuntimeToolMetadata>,
}

impl WorkflowRuntimeToolCatalog {
    pub fn new(tools: Vec<RuntimeToolMetadata>) -> Self {
        Self { tools }
    }

    fn tools(&self) -> &[RuntimeToolMetadata] {
        &self.tools
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
        .map(|catalog| catalog.tools().to_vec())
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
    Ok(WorkflowExecutionContext {
        conversation_id,
        agent_id,
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
    Ok(runtime_tools(ctx)
        .into_iter()
        .filter(|tool| active_tools.contains(&tool.name))
        .collect())
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
    .map_err(|error| {
        AIOutput::error(
            400,
            format!(
                "script compile failed at line {}: {}. Ensure every referenced tool appears in this agent's active tools and has explicit registered description, input pins, and output pins.",
                error.line, error.message,
            ),
        )
    })
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
    display_name = "列出{kind}工作流{workflows}",
    category = "Workflow",
    description = "List unified Workflow catalog resources. kind may be all, draft, or registered.",
    params { kind: "String@Optional: all, draft, or registered." },
    outputs { workflows: "Array@Workflow summaries with id, kind, revision, trust, and validation." },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
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

#[define_operation(
    name = "readWorkflow",
    display_name = "读取工作流{workflow_id}并返回资源{workflow}",
    category = "Workflow",
    description = "Read a Draft or Registered Workflow from the unified catalog by stable id.",
    params { workflow_id: "String@Stable Workflow resource id. Required." },
    outputs { workflow: "Object@Complete Workflow resource view including script and blueprint." },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ReadWorkflow;

#[async_trait]
impl SystemOperation for ReadWorkflow {
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
        let resource = match workflows(ctx)?.read_workflow_resource(&id) {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(404, error.to_string())),
        };
        sync_selection(ctx, &resource.summary.id, resource.summary.revision);
        Ok(AIOutput::success(
            json!({
                "workflow": serde_json::to_value(&resource)
                    .map_err(FrameworkError::SerializationError)?
            }),
            format!(
                "Read Workflow '{}' at revision {}.",
                resource.summary.name, resource.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "readWorkflow"
    }
}

#[define_operation(
    name = "createWorkflowDraft",
    display_name = "用脚本{script}和说明{description}创建标识{workflow_id}的工作流草稿{name}并返回{workflow}",
    category = "Workflow",
    description = "Compile a complete script and create an untrusted Draft in the unified Workflow catalog.",
    params {
        name: "String@Globally unique display name. Required.",
        workflow_id: "String@Optional requested stable id.",
        description: "String@Optional description.",
        script: "String@Complete Workflow v2 script. Required."
    },
    outputs { workflow: "Object@Created Draft resource." },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct CreateWorkflowDraft;

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

#[define_operation(
    name = "updateWorkflow",
    display_name = "用脚本{script}和说明{description}将工作流{workflow_id}的版本{expected_revision}更新为{name}并返回{workflow}",
    category = "Workflow",
    description = "Compile and update a Draft or Registered Workflow using optimistic revision control.",
    params {
        workflow_id: "String@Stable Workflow resource id. Required.",
        expected_revision: "u64@Current revision. Required.",
        name: "String@Optional replacement display name.",
        description: "String@Optional replacement description.",
        script: "String@Complete replacement Workflow v2 script. Required."
    },
    outputs { workflow: "Object@Updated Workflow resource." },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct UpdateWorkflow;

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
        let current = match module.read_workflow_resource(&id) {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(404, error.to_string())),
        };
        let mut blueprint = match compile_script(&script, ctx).await {
            Ok(blueprint) => blueprint,
            Err(error) => return Ok(error),
        };
        blueprint.metadata.id = current.summary.id.clone();
        blueprint.metadata.name = args
            .get("name")
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&current.summary.name)
            .to_string();
        blueprint.metadata.description = args
            .get("description")
            .unwrap_or(&current.summary.description)
            .to_string();
        if let Some(previous) = current.blueprint.as_ref() {
            preserve_workflow_blueprint_layout(previous, &mut blueprint);
        }
        let next_name = blueprint.metadata.name.clone();
        let next_description = blueprint.metadata.description.clone();
        let updated = match current.summary.kind {
            WorkflowResourceKind::Draft => {
                module
                    .update_draft_resource(
                        &id,
                        Some(expected_revision),
                        &next_name,
                        &next_description,
                        Some(script),
                        Some(blueprint),
                        WorkflowValidation::valid(),
                    )
                    .await
            }
            WorkflowResourceKind::Registered => {
                module
                    .update_registered_resource(&blueprint, Some(expected_revision))
                    .await
            }
        };
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

#[define_operation(
    name = "compileWorkflow",
    display_name = "编译工作流{workflow_id}或脚本{script}并返回蓝图{blueprint}",
    category = "Workflow",
    description = "Compile script text, or inspect the compiled validation and blueprint of a catalog resource.",
    params {
        workflow_id: "String@Stable resource id when compiling a catalog resource.",
        script: "String@Complete Workflow v2 script when compiling temporary text."
    },
    outputs { blueprint: "Object@Compiled BlueprintJson and validation." },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct CompileWorkflow;

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
        if let Some(script) = args
            .get("script")
            .filter(|script| !script.trim().is_empty())
        {
            let blueprint = match compile_script(script, ctx).await {
                Ok(blueprint) => blueprint,
                Err(error) => return Ok(error),
            };
            return Ok(AIOutput::success(
                json!({ "validation": WorkflowValidation::valid(), "blueprint": blueprint }),
                "Workflow script compiled successfully.".to_string(),
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
    name = "registerWorkflow",
    display_name = "将草稿{workflow_id}的版本{expected_revision}注册为{name}并返回{workflow}",
    category = "Workflow",
    description = "Promote a valid Draft to a trusted Registered Workflow using optimistic revision control.",
    params {
        workflow_id: "String@Stable Draft id. Required.",
        expected_revision: "u64@Current Draft revision. Required.",
        name: "String@Optional registered display name."
    },
    outputs { workflow: "Object@Registered Workflow resource." },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RegisterWorkflow;

#[async_trait]
impl SystemOperation for RegisterWorkflow {
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
            .register_draft_resource(
                &id,
                Some(revision),
                args.get("name")
                    .map(str::trim)
                    .filter(|name| !name.is_empty()),
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
                "Registered Workflow '{}' at revision {}.",
                resource.summary.name, resource.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "registerWorkflow"
    }
}

#[define_operation(
    name = "deleteWorkflow",
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
pub struct DeleteWorkflow;

#[async_trait]
impl SystemOperation for DeleteWorkflow {
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
        "deleteWorkflow"
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

#[define_operation(
    name = "testWorkflow",
    display_name = "测试工作流{workflow_id}并用{inputs}返回输出{outputs}和追踪{trace}",
    category = "Workflow",
    description = "Test a Draft or Registered Workflow. Inputs use --input.<name> and trace is optional.",
    params {
        workflow_id: "String@Stable Workflow resource id. Required.",
        trace: "bool@Include structured trace when true.",
        inputs: "String@Marker: pass Workflow inputs as --input.<name>."
    },
    outputs { outputs: "Object@Workflow return values.", trace: "Array@Structured node trace when requested or failed." },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct TestWorkflow;

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
    display_name = "执行已注册工作流{workflow_id}并用{inputs}返回输出{outputs}和追踪{trace}",
    category = "Workflow",
    description = "Execute a trusted Registered Workflow by stable id. Inputs use --input.<name>.",
    params {
        workflow_id: "String@Stable Registered Workflow id. Required.",
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
        let resource = match workflows(ctx)?.read_workflow_resource(&id) {
            Ok(resource) => resource,
            Err(error) => return Ok(AIOutput::error(404, error.to_string())),
        };
        if resource.summary.kind != WorkflowResourceKind::Registered {
            return Ok(AIOutput::error(
                400,
                "executeWorkflow only accepts Registered resources; use testWorkflow for Drafts",
            ));
        }
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

#[define_operation(
    name = "executeWorkflowScript",
    display_name = "执行临时工作流脚本{script}并用{inputs}返回输出{outputs}和追踪{trace}",
    category = "Workflow",
    description = "Compile and execute temporary Workflow v2 script without creating a catalog resource.",
    params {
        script: "String@Complete Workflow v2 script. Required.",
        trace: "bool@Include structured trace when true.",
        inputs: "String@Marker: pass Workflow inputs as --input.<name>."
    },
    outputs { outputs: "Object@Workflow return values.", trace: "Array@Structured node trace when requested or failed." },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct ExecuteWorkflowScript;

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

#[cfg(test)]
mod tests {
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

        let registered = RegisterWorkflow
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

        let deleted = DeleteWorkflow
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
        for tool in ["executeWorkflow", "executeWorkflowScript"] {
            let metadata = crate::tool_runner::permission_metadata(tool).unwrap();
            assert_eq!(metadata.effect, crate::ToolEffect::Destructive, "{tool}");
        }
    }
}
