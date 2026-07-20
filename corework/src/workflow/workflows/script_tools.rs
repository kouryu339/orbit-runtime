use std::collections::HashMap;
use std::time::Instant;

use crate::cache::CacheExt;
use async_trait::async_trait;
use corework::define_operation;
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ai_system::{AIInput, AIOutput, SimpleArgs};
use crate::error::FrameworkError;
use crate::orchestration::Context;
use crate::prelude::SystemOperation;
use crate::workflow::blueprint_json::{BlueprintJson, BlueprintMetadata, BlueprintVisibility};
use crate::workflow::chain_decompiler::ChainDecompiler;
use crate::workflow::core::DataValue;
use crate::workflow::execution::{ExecutionContext, WorkflowExecutionReport, WorkflowToAiMode};
use crate::workflow::BlueprintLoader;

const HOST_DYNAMIC_SNAPSHOTS_KEY: &str = "host_dynamic_snapshots";
const WORKFLOW_STUDIO_CURRENT_DRAFT_KEY: &str = "workflow_studio.current_draft";
pub const PARENT_WORKFLOW_REGISTRY: &str = "wf:parent_workflow_registry";

fn workflow_editor_catalog(
    ctx: &Context,
) -> Option<(
    Arc<crate::workflow::workflows::WorkflowsModule>,
    Arc<crate::workflow::workflows::WorkflowEditorSession>,
)> {
    Some((
        ctx.resolve_shared_component::<crate::workflow::workflows::WorkflowsModule>()
            .ok()?,
        ctx.resolve_shared_component::<crate::workflow::workflows::WorkflowEditorSession>()
            .ok()?,
    ))
}

fn selected_workflow_resource(
    ctx: &Context,
    requested_id: Option<&str>,
) -> std::result::Result<
    Option<(
        Arc<crate::workflow::workflows::WorkflowsModule>,
        Arc<crate::workflow::workflows::WorkflowEditorSession>,
        crate::workflow::workflows::WorkflowResourceView,
    )>,
    AIOutput,
> {
    let Some((workflows, session)) = workflow_editor_catalog(ctx) else {
        return Ok(None);
    };
    let workflow_id = requested_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| session.selection().map(|selection| selection.workflow_id))
        .ok_or_else(|| AIOutput::error(400, "no Workflow resource is selected"))?;
    let workflow = workflows
        .read_workflow_resource(&workflow_id)
        .map_err(|error| AIOutput::error(404, error.to_string()))?;
    Ok(Some((workflows, session, workflow)))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParentWorkflowEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub file_name: String,
    pub file_path: PathBuf,
    pub inputs: Vec<crate::workflow::blueprint_json::PinMetadata>,
    pub outputs: Vec<crate::workflow::blueprint_json::PinMetadata>,
    pub nodes: usize,
    pub connections: usize,
    pub variables: usize,
}

impl ParentWorkflowEntry {
    pub fn from_blueprint_path(
        path: PathBuf,
        blueprint: &BlueprintJson,
    ) -> std::result::Result<Self, String> {
        let Some(file_name) = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(ToString::to_string)
        else {
            return Err(format!(
                "workflow path has no file name: {}",
                path.display()
            ));
        };
        Ok(Self {
            id: blueprint.metadata.id.clone(),
            name: blueprint.metadata.name.clone(),
            description: blueprint.metadata.description.clone(),
            file_name,
            file_path: path,
            inputs: blueprint.metadata.inputs.clone(),
            outputs: blueprint.metadata.outputs.clone(),
            nodes: blueprint.nodes.len(),
            connections: blueprint.connections.len(),
            variables: blueprint.variables.len(),
        })
    }

    pub fn metadata(&self) -> BlueprintMetadata {
        BlueprintMetadata {
            id: self.id.clone(),
            name: self.name.clone(),
            created: String::new(),
            modified: String::new(),
            description: self.description.clone(),
            author: String::new(),
            tags: Vec::new(),
            visibility: BlueprintVisibility::Private,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
        }
    }
}

pub fn parent_workflow_entries_from_context(
    ctx: &Context,
) -> std::result::Result<Vec<ParentWorkflowEntry>, AIOutput> {
    let world = ctx
        .get_world_cache()
        .map_err(|e| AIOutput::error(500, format!("workflow world cache unavailable: {}", e)))?;
    world
        .get_resource(PARENT_WORKFLOW_REGISTRY)
        .map_err(|e| AIOutput::error(500, format!("failed to read workflow registry: {}", e)))?
        .ok_or_else(|| AIOutput::error(404, "workflow registry is not initialized"))
}

fn find_parent_workflow_entry(
    ctx: &Context,
    selector: &str,
) -> std::result::Result<ParentWorkflowEntry, AIOutput> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return Err(AIOutput::error(400, "workflow selector must not be empty"));
    }
    let selector_path = Path::new(trimmed);
    if selector_path.file_name().and_then(|value| value.to_str()) != Some(trimmed) {
        return Err(AIOutput::error(
            400,
            "workflow selector must be a registered file_name, id, or name; paths are not allowed",
        ));
    }
    let entries = parent_workflow_entries_from_context(ctx)?;
    entries
        .into_iter()
        .find(|entry| {
            entry.file_name == trimmed
                || entry.name == trimmed
                || (!entry.id.is_empty() && entry.id == trimmed)
        })
        .ok_or_else(|| AIOutput::error(404, format!("workflow not registered: {}", trimmed)))
}

fn parse_prefixed_workflow_inputs(
    args: &SimpleArgs,
    reserved: &[&str],
) -> std::result::Result<HashMap<String, DataValue>, AIOutput> {
    let mut inputs = HashMap::new();
    let mut bare_workflow_args = Vec::new();

    for key in args.keys() {
        if let Some(input_name) = key.strip_prefix("input.") {
            if input_name.is_empty() {
                return Err(AIOutput::error(
                    400,
                    "workflow input prefix cannot be empty; use --input.<name>",
                ));
            }
            let value = args.get(key).unwrap_or_default();
            inputs.insert(input_name.to_string(), json_arg_to_data_value(value));
        } else if !reserved.iter().any(|reserved_key| reserved_key == &key) {
            bare_workflow_args.push(key.to_string());
        }
    }

    if bare_workflow_args.is_empty() {
        Ok(inputs)
    } else {
        Err(AIOutput::error(
            400,
            format!(
                "workflow inputs must use --input.<name>; invalid bare argument(s): {}",
                bare_workflow_args
                    .iter()
                    .map(|key| format!("--{key}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))
    }
}

fn json_arg_to_data_value(value: &str) -> DataValue {
    match serde_json::from_str::<JsonValue>(value) {
        Ok(JsonValue::Bool(v)) => DataValue::from_bool(v),
        Ok(JsonValue::Number(n)) => {
            if let Some(v) = n.as_i64() {
                DataValue::from_i64(v)
            } else if let Some(v) = n.as_f64() {
                DataValue::from_f64(v)
            } else {
                DataValue::new("Number", JsonValue::Number(n))
            }
        }
        Ok(JsonValue::String(v)) => DataValue::from_string(v),
        Ok(JsonValue::Null) => DataValue::null(),
        Ok(v @ JsonValue::Array(_)) => DataValue::new("Array", v),
        Ok(v @ JsonValue::Object(_)) => DataValue::new("Object", v),
        Err(_) => DataValue::from_string(value.to_string()),
    }
}

async fn execute_blueprint_json_report(
    blueprint: BlueprintJson,
    ctx: &Context,
    inputs: HashMap<String, DataValue>,
    trace_enabled: bool,
) -> crate::error::Result<WorkflowExecutionReport> {
    let loaded = BlueprintLoader::new().load_from_blueprint_json(blueprint, ctx)?;
    let mut exec_ctx = ExecutionContext::from_context(ctx.clone());

    if trace_enabled {
        exec_ctx.enable_trace(
            loaded.metadata.name.clone(),
            loaded.compiled.source_map.clone(),
        );
    }

    loaded.compiled.initialize_defaults(&mut exec_ctx).await?;
    let outputs = loaded
        .compiled
        .executor()
        .execute_with_params(&mut exec_ctx, inputs)
        .await?;
    let trace = exec_ctx.take_trace();

    Ok(WorkflowExecutionReport { outputs, trace })
}

fn workflow_report_ai_output(
    report: WorkflowExecutionReport,
    duration_ms: u128,
    trace_enabled: bool,
) -> AIOutput {
    let mut result = serde_json::json!({
        "outputs": report.outputs_json(),
        "duration_ms": duration_ms,
    });

    if trace_enabled {
        result["trace"] = serde_json::to_value(&report.trace).unwrap_or(JsonValue::Null);
    }

    let to_ai_mode = if trace_enabled {
        WorkflowToAiMode::Detailed
    } else {
        WorkflowToAiMode::DetailedOnError
    };
    AIOutput::success(result, report.to_ai(to_ai_mode, None))
}

fn workflows_dir_from_context(ctx: &Context) -> std::result::Result<PathBuf, AIOutput> {
    let world = ctx
        .get_world_cache()
        .map_err(|e| AIOutput::error(500, format!("workflow world cache unavailable: {}", e)))?;
    let dir: String = world
        .get_resource("wf:workflows_dir")
        .map_err(|e| AIOutput::error(500, format!("failed to read workflows_dir: {}", e)))?
        .unwrap_or_else(|| "workflows".to_string());
    Ok(PathBuf::from(dir))
}

fn safe_workflow_file_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch.is_whitespace()
            || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches('_').trim();
    if trimmed.is_empty() {
        "workflow".to_string()
    } else {
        trimmed.to_string()
    }
}

fn find_workflow_file(workflows_dir: &Path, name_or_path: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(name_or_path);
    if direct.exists() && BlueprintJson::is_workflow_file_path(&direct) {
        return Some(direct);
    }

    let candidates = [
        workflows_dir.join(name_or_path),
        workflows_dir.join(format!("{name_or_path}.workflow.json")),
        workflows_dir.join(format!(
            "{}.workflow.json",
            safe_workflow_file_stem(name_or_path)
        )),
    ];
    for candidate in candidates {
        if candidate.exists() && BlueprintJson::is_workflow_file_path(&candidate) {
            return Some(candidate);
        }
    }

    let entries = std::fs::read_dir(workflows_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !BlueprintJson::is_workflow_file_path(&path) {
            continue;
        }
        if let Ok(bp) = BlueprintJson::from_workflow_file(&path) {
            if bp.metadata.name == name_or_path || bp.metadata.id == name_or_path {
                return Some(path);
            }
        }
    }
    None
}

#[define_operation(
    name = "execSC",
    display_name = "执行工作流脚本{script}或{scripts}，传入{inputs}并按{trace}返回输出{outputs}",
    description = "Compile and execute temporary workflow script text. Workflow inputs must use --input.<name>.",
    params {
        script: "String@Temporary AI-friendly workflow script text. 必填. Alias: scripts.",
        scripts: "String@Alias of script.",
        trace: "bool@Optional. Include structured trace in result when true.",
        inputs: "String@Marker parameter: workflow inputs are passed as --input.<name>, for example --input.name alice."
    },
    outputs {
        outputs: "Any@Workflow data outputs."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct ExecSC;

#[async_trait]
impl SystemOperation for ExecSC {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let script = match args.get("script").or_else(|| args.get("scripts")) {
            Some(v) => v.to_string(),
            None => return Ok(AIOutput::error(400, "missing required argument: --script")),
        };
        let inputs = match parse_prefixed_workflow_inputs(
            &args,
            &["script", "scripts", "trace", "inputs"],
        ) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let trace_enabled = args.get_bool("trace");
        let script_text = script.replace(r"\n", "\n").replace(r#"\""#, "\"");
        if script_text.trim().is_empty() {
            return Ok(AIOutput::error(400, "script must not be empty"));
        }

        let blueprint = match crate::workflow::chain_compiler_v2::compile_chain_v2(&script_text) {
            Ok(v) => v,
            Err(e) => {
                return Ok(AIOutput::error(
                    400,
                    format!("script compile failed at line {}: {}", e.line, e.message),
                ))
            }
        };

        let started = Instant::now();
        match execute_blueprint_json_report(blueprint, ctx, inputs, trace_enabled).await {
            Ok(report) => Ok(workflow_report_ai_output(
                report,
                started.elapsed().as_millis(),
                trace_enabled,
            )),
            Err(e) => Ok(AIOutput::error(
                -1,
                format!("script execution failed: {}", e),
            )),
        }
    }

    fn name(&self) -> &str {
        "execSC"
    }
}

#[define_operation(
    name = "execSCForPath",
    display_name = "执行工作流文件{file_name}，传入{inputs}并按{trace}返回输出{outputs}",
    description = "Execute a validated persisted workflow selected by exact file_name. Workflow inputs must use --input.<name>.",
    params {
        file_name: "String@Exact *.workflow.json file name under workflows_dir. 必填.",
        trace: "bool@Optional. Include structured trace in result when true.",
        inputs: "String@Marker parameter: workflow inputs are passed as --input.<name>, for example --input.path D:/a.txt."
    },
    outputs {
        outputs: "Any@Workflow data outputs."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct ExecSCForPath;

#[async_trait]
impl SystemOperation for ExecSCForPath {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let file_name = match args.safe_require("file_name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let inputs = match parse_prefixed_workflow_inputs(&args, &["file_name", "trace", "inputs"])
        {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let trace_enabled = args.get_bool("trace");
        let entry = match find_parent_workflow_entry(ctx, &file_name) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        let blueprint = match BlueprintJson::from_workflow_file(&entry.file_path) {
            Ok(v) => v,
            Err(e) => return Ok(AIOutput::error(400, e)),
        };

        let started = Instant::now();
        match execute_blueprint_json_report(blueprint, ctx, inputs, trace_enabled).await {
            Ok(report) => Ok(workflow_report_ai_output(
                report,
                started.elapsed().as_millis(),
                trace_enabled,
            )),
            Err(e) => Ok(AIOutput::error(
                -1,
                format!("workflow execution failed: {}", e),
            )),
        }
    }

    fn name(&self) -> &str {
        "execSCForPath"
    }
}

#[define_operation(
    name = "listParentWorkflows",
    display_name = "列出已注册工作流{workflows}",
    description = "List registered parent-agent workflows.",
    system_only,
    outputs {
        workflows: "Array@Persisted *.workflow.json workflow summaries."
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ListParentWorkflows;

#[async_trait]
impl SystemOperation for ListParentWorkflows {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let entries = match parent_workflow_entries_from_context(ctx) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let workflows: Vec<_> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name,
                    "id": entry.id,
                    "description": entry.description,
                    "file_name": entry.file_name,
                    "path": entry.file_path.to_string_lossy(),
                    "inputs": entry.inputs,
                    "outputs": entry.outputs,
                    "nodes": entry.nodes,
                    "connections": entry.connections,
                    "variables": entry.variables,
                })
            })
            .collect();
        let to_ai = if workflows.is_empty() {
            "No registered parent workflows.".to_string()
        } else {
            let lines = workflows
                .iter()
                .map(|wf| {
                    format!(
                        "- {}: {} ({} nodes, {} connections)",
                        wf["name"].as_str().unwrap_or(""),
                        wf["description"].as_str().unwrap_or(""),
                        wf["nodes"].as_u64().unwrap_or(0),
                        wf["connections"].as_u64().unwrap_or(0)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("Registered parent workflows:\n{}", lines)
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "workflows": workflows,
            }),
            to_ai,
        ))
    }

    fn name(&self) -> &str {
        "listParentWorkflows"
    }
}

#[define_operation(
    name = "readWorkflow",
    display_name = "读取工作流{workflow_id}或文件{file_name}，返回蓝图{blueprint}和脚本{script}",
    description = "Read a Draft or Registered Workflow resource by stable id. When workflow_id is omitted in Workflow Studio, reads the current selection.",
    system_only,
    params {
        workflow_id: "String@Stable Workflow resource id. Optional for the current Workflow Studio selection.",
        file_name: "String@Legacy exact file name fallback outside the unified Workflow editor."
    },
    outputs {
        blueprint: "Object@BlueprintJson content.",
        script: "String@Decompiled workflow script text."
    },
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

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        match selected_workflow_resource(ctx, args.get("workflow_id")) {
            Ok(Some((_workflows, session, workflow))) => {
                session.select(workflow.summary.id.clone(), workflow.summary.revision);
                let script = workflow.script.clone().unwrap_or_default();
                return Ok(AIOutput::success(
                    serde_json::to_value(&workflow).map_err(FrameworkError::SerializationError)?,
                    script,
                ));
            }
            Ok(None) => {}
            Err(error) => return Ok(error),
        }
        let file_name = match args.get("file_name").map(str::trim) {
            Some(v) => v,
            None => {
                return Ok(AIOutput::error(
                    400,
                    "missing required argument: --file_name",
                ))
            }
        };
        let file_path = Path::new(file_name);
        if file_path.file_name().and_then(|value| value.to_str()) != Some(file_name) {
            return Ok(AIOutput::error(
                400,
                "file_name must be a file name only, not a path",
            ));
        }
        if !BlueprintJson::is_workflow_file_path(file_path) {
            return Ok(AIOutput::error(
                400,
                "file_name must end with .workflow.json",
            ));
        }
        let workflows_dir = match workflows_dir_from_context(ctx) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let path = workflows_dir.join(file_name);
        if !path.exists() {
            return Ok(AIOutput::error(
                404,
                format!("workflow not found: {}", file_name),
            ));
        }
        let blueprint = match BlueprintJson::from_workflow_file(&path) {
            Ok(v) => v,
            Err(e) => return Ok(AIOutput::error(400, e)),
        };
        let script = match crate::workflow::chain_decompiler::decompile_chain(&blueprint) {
            Ok(v) => v,
            Err(e) => return Ok(AIOutput::error(400, e.to_string())),
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "path": path,
                "blueprint": blueprint,
                "script": script,
            }),
            script,
        ))
    }

    fn name(&self) -> &str {
        "readWorkflow"
    }
}

#[define_operation(
    name = "saveWorkflow",
    display_name = "将蓝图{blueprint}或脚本{script}保存为工作流文件{file_name}，返回路径{path}",
    description = "Save a BlueprintJson, script, or current Workflow Studio draft under the fixed workflows directory using an exact *.workflow.json file name.",
    system_only,
    params {
        blueprint: "String@Full BlueprintJson object as JSON. Optional when script or current Studio draft is available.",
        script: "String@Workflow script text to compile and save. Optional.",
        file_name: "String@Exact file name under workflows_dir. Must end with .workflow.json and must not contain a path. Required."
    },
    outputs {
        file_name: "String@Saved workflow file name.",
        path: "String@Saved workflow path."
    },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct SaveWorkflow;

#[async_trait]
impl SystemOperation for SaveWorkflow {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        if workflow_editor_catalog(ctx).is_some() {
            return Ok(AIOutput::error(
                410,
                "saveWorkflow file writes are disabled in the unified Workflow editor; use updateCurrentWorkflowDraft, then registerCurrentWorkflowDraft for promotion",
            ));
        }
        let mut blueprint = if let Some(raw) =
            args.get("blueprint").filter(|v| !v.trim().is_empty())
        {
            let value: JsonValue = match serde_json::from_str(raw) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(AIOutput::error(
                        400,
                        format!("invalid blueprint JSON: {}", e),
                    ))
                }
            };
            match BlueprintJson::from_json_value(value) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(AIOutput::error(
                        400,
                        format!("invalid BlueprintJson: {}", e),
                    ))
                }
            }
        } else {
            let (script_text, source) =
                match args.get("script").filter(|value| !value.trim().is_empty()) {
                    Some(script) => (
                        script.replace(r"\n", "\n").replace(r#"\""#, "\""),
                        "argument",
                    ),
                    None => match current_workflow_studio_draft(ctx).await? {
                        Some(script) => (script, WORKFLOW_STUDIO_CURRENT_DRAFT_KEY),
                        None => {
                            return Ok(AIOutput::error(
                                400,
                                "provide --blueprint, --script, or an available workflow_studio.current_draft",
                            ))
                        }
                    },
                };
            match crate::workflow::chain_compiler_v2::compile_chain_v2(&script_text) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(AIOutput::error(
                        400,
                        format!(
                            "script compile failed from {} at line {}: {}",
                            source, e.line, e.message
                        ),
                    ))
                }
            }
        };
        let file_name = match args.get("file_name").map(str::trim) {
            Some(value) if !value.is_empty() => value,
            _ => {
                return Ok(AIOutput::error(
                    400,
                    "missing required argument: --file_name",
                ))
            }
        };
        let file_path = Path::new(file_name);
        if file_path.file_name().and_then(|value| value.to_str()) != Some(file_name) {
            return Ok(AIOutput::error(
                400,
                "file_name must be a file name only, not a path",
            ));
        }
        if !BlueprintJson::is_workflow_file_path(file_path) {
            return Ok(AIOutput::error(
                400,
                "file_name must end with .workflow.json",
            ));
        }
        let workflows_dir = match workflows_dir_from_context(ctx) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        if let Err(e) = std::fs::create_dir_all(&workflows_dir) {
            return Ok(AIOutput::error(
                500,
                format!("create workflow directory failed: {}", e),
            ));
        }
        let path = workflows_dir.join(file_name);
        if let Err(e) = blueprint.save_to_workflow_file(&path) {
            return Ok(AIOutput::error(500, e));
        }
        Ok(AIOutput::success(
            serde_json::json!({
                "file_name": file_name,
                "path": path,
                "workflow_name": blueprint.metadata.name,
            }),
            format!(
                "Saved workflow '{}' to {}.",
                blueprint.metadata.name,
                path.display()
            ),
        ))
    }

    fn name(&self) -> &str {
        "saveWorkflow"
    }
}

#[define_operation(
    name = "compileWorkflowScript",
    display_name = "编译工作流{workflow_id}或文件{file_name}并返回蓝图{blueprint}",
    description = "Return validation and compiled BlueprintJson for the current or requested Workflow resource.",
    system_only,
    params {
        workflow_id: "String@Stable Workflow resource id. Optional for the current Workflow Studio selection.",
        file_name: "String@Legacy exact file name fallback outside the unified Workflow editor."
    },
    outputs {
        blueprint: "Object@Compiled BlueprintJson."
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct CompileWorkflowScript;

#[async_trait]
impl SystemOperation for CompileWorkflowScript {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        match selected_workflow_resource(ctx, args.get("workflow_id")) {
            Ok(Some((_workflows, _session, workflow))) => {
                return Ok(AIOutput::success(
                    serde_json::json!({
                        "workflow_id": workflow.summary.id,
                        "kind": workflow.summary.kind,
                        "revision": workflow.summary.revision,
                        "validation": workflow.summary.validation,
                        "blueprint": workflow.blueprint,
                        "script_source": "workflow_catalog"
                    }),
                    format!(
                        "Workflow '{}' is {} at revision {}.",
                        workflow.summary.name,
                        if workflow.summary.validation.valid {
                            "valid"
                        } else {
                            "invalid"
                        },
                        workflow.summary.revision
                    ),
                ));
            }
            Ok(None) => {}
            Err(error) => return Ok(error),
        }
        let (script_text, source) = match workflow_script_for_editor_target(&args, ctx).await {
            Ok(value) => value,
            Err(e) => return Ok(e),
        };
        let blueprint = match crate::workflow::chain_compiler_v2::compile_chain_v2(&script_text) {
            Ok(v) => v,
            Err(e) => {
                return Ok(AIOutput::error(
                    400,
                    format!("script compile failed at line {}: {}", e.line, e.message),
                ))
            }
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "blueprint": blueprint,
                "script_source": source,
                "script_bytes": script_text.len()
            }),
            format!(
                "Compiled workflow '{}' from {} ({} nodes, {} connections).",
                blueprint.metadata.name,
                source,
                blueprint.nodes.len(),
                blueprint.connections.len()
            ),
        ))
    }

    fn name(&self) -> &str {
        "compileWorkflowScript"
    }
}

async fn current_workflow_studio_draft(
    ctx: &Context,
) -> std::result::Result<Option<String>, FrameworkError> {
    let snapshots = ctx
        .cache
        .get::<HashMap<String, String>>(HOST_DYNAMIC_SNAPSHOTS_KEY)
        .await?;
    Ok(snapshots.and_then(|fields| {
        fields
            .get(WORKFLOW_STUDIO_CURRENT_DRAFT_KEY)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }))
}

fn exact_workflow_file_path(
    ctx: &Context,
    file_name: &str,
) -> std::result::Result<PathBuf, AIOutput> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return Err(AIOutput::error(400, "file_name must not be empty"));
    }
    let file_path = Path::new(trimmed);
    if file_path.file_name().and_then(|value| value.to_str()) != Some(trimmed) {
        return Err(AIOutput::error(
            400,
            "file_name must be a file name only, not a path",
        ));
    }
    if !BlueprintJson::is_workflow_file_path(file_path) {
        return Err(AIOutput::error(
            400,
            "file_name must end with .workflow.json",
        ));
    }
    let workflows_dir = workflows_dir_from_context(ctx)?;
    let path = workflows_dir.join(trimmed);
    if !path.exists() {
        return Err(AIOutput::error(
            404,
            format!("workflow not found: {}", trimmed),
        ));
    }
    Ok(path)
}

async fn workflow_script_for_editor_target(
    args: &SimpleArgs,
    ctx: &Context,
) -> std::result::Result<(String, String), AIOutput> {
    if let Some(file_name) = args
        .get("file_name")
        .filter(|value| !value.trim().is_empty())
    {
        let path = exact_workflow_file_path(ctx, file_name)?;
        let blueprint =
            BlueprintJson::from_workflow_file(&path).map_err(|e| AIOutput::error(400, e))?;
        let script = crate::workflow::chain_decompiler::decompile_chain(&blueprint)
            .map_err(|e| AIOutput::error(400, e.to_string()))?;
        return Ok((script, format!("file_name:{file_name}")));
    }
    match current_workflow_studio_draft(ctx)
        .await
        .map_err(|e| AIOutput::error(500, e.to_string()))?
    {
        Some(script) => Ok((script, WORKFLOW_STUDIO_CURRENT_DRAFT_KEY.to_string())),
        None => Err(AIOutput::error(
            400,
            "current Studio draft is unavailable; omit file_name only after the Studio page has published workflow_studio.current_draft",
        )),
    }
}

#[define_operation(
    name = "decompileWorkflowToScript",
    display_name = "将蓝图{blueprint}或工作流{name}/{path}反编译为脚本{script}",
    description = "Decompile BlueprintJson or a persisted workflow into AI-friendly workflow script text.",
    system_only,
    params {
        blueprint: "String@BlueprintJson object as JSON. Optional when name/path is provided.",
        name: "String@Workflow name/id/file name/path. Optional.",
        path: "String@Workflow file path or file name. Optional."
    },
    outputs {
        script: "String@AI-friendly workflow script text."
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct DecompileWorkflowToScript;

#[async_trait]
impl SystemOperation for DecompileWorkflowToScript {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let blueprint = if let Some(raw) = args.get("blueprint") {
            let value: JsonValue = match serde_json::from_str(raw) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(AIOutput::error(
                        400,
                        format!("invalid blueprint JSON: {}", e),
                    ))
                }
            };
            match BlueprintJson::from_json_value(value) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(AIOutput::error(
                        400,
                        format!("invalid BlueprintJson: {}", e),
                    ))
                }
            }
        } else {
            let name = match args.get("name").or_else(|| args.get("path")) {
                Some(v) => v,
                None => return Ok(AIOutput::error(400, "provide --blueprint or --name/--path")),
            };
            let workflows_dir = match workflows_dir_from_context(ctx) {
                Ok(v) => v,
                Err(e) => return Ok(e),
            };
            let Some(path) = find_workflow_file(&workflows_dir, name) else {
                return Ok(AIOutput::error(
                    404,
                    format!("workflow not found: {}", name),
                ));
            };
            match BlueprintJson::from_workflow_file(&path) {
                Ok(v) => v,
                Err(e) => return Ok(AIOutput::error(400, e)),
            }
        };
        let script = match ChainDecompiler::decompile(&blueprint) {
            Ok(v) => v,
            Err(e) => return Ok(AIOutput::error(400, e.to_string())),
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "workflow_name": blueprint.metadata.name,
                "script": script,
            }),
            format!(
                "Decompiled workflow '{}':\n{}",
                blueprint.metadata.name, script
            ),
        ))
    }

    fn name(&self) -> &str {
        "decompileWorkflowToScript"
    }
}

#[define_operation(
    name = "testWorkflow",
    display_name = "测试工作流{workflow_id}或文件{file_name}，传入{inputs}并按{trace}返回输出{outputs}",
    description = "Execute the current or requested Workflow resource with optional trace. Drafts run as untrusted tests; Registered resources use their registered blueprint.",
    system_only,
    params {
        workflow_id: "String@Stable Workflow resource id. Optional for the current Workflow Studio selection.",
        file_name: "String@Legacy exact file name fallback outside the unified Workflow editor.",
        trace: "bool@Optional. Include structured trace in result when true.",
        inputs: "String@Marker parameter: workflow inputs are passed as --input.<name>."
    },
    outputs {
        outputs: "Object@Workflow data outputs."
    },
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

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        match selected_workflow_resource(ctx, args.get("workflow_id")) {
            Ok(Some((workflows, _session, workflow))) => {
                let inputs = match parse_prefixed_workflow_inputs(
                    &args,
                    &["workflow_id", "file_name", "trace", "inputs"],
                ) {
                    Ok(inputs) => inputs
                        .into_iter()
                        .map(|(key, value)| (key, value.json_value().clone()))
                        .collect::<HashMap<_, _>>(),
                    Err(error) => return Ok(error),
                };
                let trace_enabled = args.get_bool("trace");
                let started = Instant::now();
                let execution = match workflow.summary.kind {
                    crate::workflow::workflows::WorkflowResourceKind::Draft => {
                        let blueprint = match workflows.draft_blueprint(&workflow.summary.id) {
                            Ok(blueprint) => blueprint,
                            Err(error) => return Ok(AIOutput::error(400, error.to_string())),
                        };
                        workflows
                            .execute_from_blueprint_outcome(blueprint, inputs)
                            .await
                    }
                    crate::workflow::workflows::WorkflowResourceKind::Registered => {
                        workflows
                            .execute_registered_outcome(&workflow.summary.id, inputs)
                            .await
                    }
                };
                let duration_ms = started.elapsed().as_millis();
                let outcome = match execution {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        workflows
                            .publish_execution_event(serde_json::json!({
                                "source": "workflow_editor",
                                "workflow_id": workflow.summary.id,
                                "revision": workflow.summary.revision,
                                "status": "failed",
                                "code": 400,
                                "duration_ms": duration_ms,
                                "error": error.to_string()
                            }))
                            .await;
                        return Ok(AIOutput::error(400, error.to_string()));
                    }
                };
                let execution_error = outcome.error.clone();
                workflows
                    .publish_execution_event(serde_json::json!({
                        "source": "workflow_editor",
                        "workflow_id": workflow.summary.id,
                        "revision": workflow.summary.revision,
                        "status": if execution_error.is_none() { "succeeded" } else { "failed" },
                        "code": if execution_error.is_none() { 0 } else { -1 },
                        "duration_ms": duration_ms,
                        "error": execution_error
                    }))
                    .await;
                return Ok(workflow_report_ai_output(
                    outcome.report,
                    duration_ms,
                    trace_enabled,
                ));
            }
            Ok(None) => {}
            Err(error) => return Ok(error),
        }
        let (script_text, source) = match workflow_script_for_editor_target(&args, ctx).await {
            Ok(value) => value,
            Err(e) => return Ok(e),
        };
        let blueprint = match crate::workflow::chain_compiler_v2::compile_chain_v2(&script_text) {
            Ok(v) => v,
            Err(e) => {
                return Ok(AIOutput::error(
                    400,
                    format!(
                        "script compile failed from {} at line {}: {}",
                        source, e.line, e.message
                    ),
                ))
            }
        };
        let inputs = match parse_prefixed_workflow_inputs(&args, &["file_name", "trace", "inputs"])
        {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let trace_enabled = args.get_bool("trace");
        let started = Instant::now();
        match execute_blueprint_json_report(blueprint, ctx, inputs, trace_enabled).await {
            Ok(report) => Ok(workflow_report_ai_output(
                report,
                started.elapsed().as_millis(),
                trace_enabled,
            )),
            Err(e) => Ok(AIOutput::error(
                -1,
                format!("workflow execution failed: {}", e),
            )),
        }
    }

    fn name(&self) -> &str {
        "testWorkflow"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::FrameworkState;

    fn test_context() -> Context {
        FrameworkState::initialize().unwrap().create_context()
    }

    #[test]
    fn prefixed_inputs_reject_bare_workflow_args() {
        let args = SimpleArgs::parse("--path ./demo.workflow.json --name foo").unwrap();
        let err = parse_prefixed_workflow_inputs(&args, &["path"]).unwrap_err();
        assert_ne!(err.error_code, 0);
        assert!(err.to_ai.contains("--input.<name>"));
    }

    #[test]
    fn prefixed_inputs_collect_input_namespace() {
        let args =
            SimpleArgs::parse("--path ./demo.workflow.json --input.name foo --input.count 3")
                .unwrap();
        let inputs = parse_prefixed_workflow_inputs(&args, &["path"]).unwrap();
        assert_eq!(
            inputs.get("name").unwrap().json_value(),
            &serde_json::json!("foo")
        );
        assert_eq!(
            inputs.get("count").unwrap().json_value(),
            &serde_json::json!(3)
        );
    }

    #[tokio::test]
    async fn exec_sc_runs_temp_script_with_prefixed_inputs() {
        let system = ExecSC;
        let ctx = test_context();
        let input = AIInput::from_args(HashMap::from([
            (
                "script".to_string(),
                "input name\nreturn result=$name".to_string(),
            ),
            ("input.name".to_string(), "alice".to_string()),
        ]));

        let output = system.execute(input, &ctx).await.unwrap();
        assert_eq!(output.error_code, 0, "{}", output.to_ai);
        assert_eq!(
            output.result["outputs"]["result"],
            serde_json::json!("alice")
        );
    }

    #[tokio::test]
    async fn exec_sc_for_path_requires_workflow_json_suffix_and_runs() {
        let system = ExecSCForPath;
        let state = FrameworkState::initialize().unwrap();
        let ctx = state.create_context();
        let mut blueprint = crate::workflow::chain_compiler_v2::compile_chain_v2(
            r#"
input name
return result=$name
"#,
        )
        .unwrap();
        blueprint.metadata.name = "path_exec_test".to_string();
        let path = std::env::temp_dir().join(format!(
            "path_exec_test_{}.workflow.json",
            std::process::id()
        ));
        blueprint.save_to_workflow_file(&path).unwrap();
        let registry =
            vec![ParentWorkflowEntry::from_blueprint_path(path.clone(), &blueprint).unwrap()];
        state
            .world()
            .set_resource(PARENT_WORKFLOW_REGISTRY, &registry, None)
            .unwrap();

        let input = AIInput::from_args(HashMap::from([
            ("file_name".to_string(), registry[0].file_name.clone()),
            ("input.name".to_string(), "bob".to_string()),
        ]));
        let output = system.execute(input, &ctx).await.unwrap();
        assert_eq!(output.error_code, 0, "{}", output.to_ai);
        assert_eq!(output.result["outputs"]["result"], serde_json::json!("bob"));

        let bad_input = AIInput::from_args(HashMap::from([(
            "file_name".to_string(),
            "bad.json".to_string(),
        )]));
        let bad = system.execute(bad_input, &ctx).await.unwrap();
        assert_ne!(bad.error_code, 0);
        assert!(bad.to_ai.contains("not registered"));
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn list_parent_workflows_reads_registered_entries() {
        let state = FrameworkState::initialize().unwrap();
        let ctx = state.create_context();
        let registry = vec![ParentWorkflowEntry {
            id: "wf-1".to_string(),
            name: "Registered Flow".to_string(),
            description: "From resource registry".to_string(),
            file_name: "registered.workflow.json".to_string(),
            file_path: PathBuf::from("D:/registered.workflow.json"),
            inputs: Vec::new(),
            outputs: Vec::new(),
            nodes: 2,
            connections: 1,
            variables: 0,
        }];
        state
            .world()
            .set_resource(PARENT_WORKFLOW_REGISTRY, &registry, None)
            .unwrap();

        let output = ListParentWorkflows
            .execute(AIInput::from_args(HashMap::new()), &ctx)
            .await
            .unwrap();

        assert_eq!(output.error_code, 0, "{}", output.to_ai);
        assert_eq!(output.result["workflows"][0]["name"], "Registered Flow");
        assert_eq!(
            output.result["workflows"][0]["file_name"],
            "registered.workflow.json"
        );
    }

    #[tokio::test]
    async fn read_workflow_returns_script_text_in_to_ai() {
        let state = FrameworkState::initialize().unwrap();
        let ctx = state.create_context();
        let dir = std::env::temp_dir().join(format!("read_workflow_to_ai_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        state
            .world()
            .set_resource("wf:workflows_dir", &dir.to_string_lossy().to_string(), None)
            .unwrap();

        let mut blueprint = crate::workflow::chain_compiler_v2::compile_chain_v2(
            "input text:String\nreturn result=input.text",
        )
        .unwrap();
        let path = dir.join("read_to_ai.workflow.json");
        blueprint.save_to_workflow_file(&path).unwrap();

        let output = ReadWorkflow
            .execute(
                AIInput::from_args(HashMap::from([(
                    "file_name".to_string(),
                    "read_to_ai.workflow.json".to_string(),
                )])),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.error_code, 0, "{}", output.to_ai);
        assert_eq!(output.to_ai, output.result["script"].as_str().unwrap());
        assert!(output.to_ai.starts_with("input "));
        assert!(output.to_ai.contains("return result=input.text"));

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
