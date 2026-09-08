use super::*;
use std::time::Instant;

const WORKFLOW_RESOURCE_SCHEMA: &str = "agent-runtime-workflow-resource/v1";
const MAX_WORKFLOW_RESULT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowResourceInput {
    #[serde(default = "default_workflow_resource_schema")]
    schema: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    blueprint: Option<BlueprintJson>,
}

fn default_workflow_resource_schema() -> String {
    WORKFLOW_RESOURCE_SCHEMA.to_string()
}

impl RuntimeFacade {
    pub fn describe_workflow_inputs(
        &self,
        workflow_id: Option<&str>,
        script: Option<&str>,
    ) -> Result<Value, RuntimeError> {
        if workflow_id.is_some() == script.is_some() {
            return Err(RuntimeError::InvalidConfig(
                "workflow.describe_inputs requires exactly one of payload.id or payload.script"
                    .to_string(),
            ));
        }
        let blueprint = if let Some(id) = workflow_id {
            self.workflow_module()?
                .read_workflow_resource(id)
                .map_err(workflow_input_error)?
                .blueprint
                .ok_or_else(|| {
                    RuntimeError::InvalidConfig(format!(
                        "workflow '{id}' does not have a valid compiled blueprint"
                    ))
                })?
        } else {
            corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
                script.unwrap_or_default(),
                &self.runtime_tools,
            )
            .map_err(|error| {
                RuntimeError::InvalidConfig(format!(
                    "workflow script compile failed at line {}: {}",
                    error.line, error.message
                ))
            })?
        };
        let inputs = if blueprint.metadata.inputs.is_empty() {
            blueprint
                .nodes
                .iter()
                .filter(|node| node.node_type == "StartNode")
                .flat_map(|node| node.pins.iter())
                .filter(|pin| pin.kind == "DataOutput")
                .map(workflow_node_input_metadata)
                .collect::<Vec<_>>()
        } else {
            blueprint
                .metadata
                .inputs
                .iter()
                .map(workflow_input_metadata)
                .collect::<Vec<_>>()
        };
        Ok(json!({
            "schema": "agent-runtime-workflow-inputs/v1",
            "workflow_id": workflow_id,
            "inputs": inputs
        }))
    }
    pub fn start_workflow_run(&self) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let workflow_run_id = self
            .workflow_runs
            .lock()
            .map_err(|_| RuntimeError::Internal("workflow run registry lock poisoned".to_string()))?
            .create()
            .map_err(RuntimeError::InvalidConfig)?;
        Ok(json!({
            "schema": "agent-runtime-workflow-run-start/v1",
            "workflow_run_id": workflow_run_id,
            "run_id": workflow_run_id,
            "status": "created"
        }))
    }

    pub fn run_workflow(
        &mut self,
        workflow_run_id: String,
        workflow_id: Option<String>,
        script: Option<String>,
        mode: Option<String>,
        inputs: HashMap<String, Value>,
        execution_context: corework::workflow::workflows::WorkflowExecutionContext,
    ) -> Result<Value, RuntimeError> {
        if workflow_id.is_some() == script.is_some() {
            return Err(RuntimeError::InvalidConfig(
                "workflow.run requires exactly one of payload.id or payload.script".to_string(),
            ));
        }
        let module = self.workflow_module()?;
        let runtime_tools = self.runtime_tools.clone();
        let source = if script.is_some() {
            "script"
        } else {
            "catalog"
        };
        self.workflow_runs
            .lock()
            .map_err(|_| RuntimeError::Internal("workflow run registry lock poisoned".to_string()))?
            .begin(&workflow_run_id)
            .map_err(RuntimeError::InvalidConfig)?;

        let task_run_id = workflow_run_id.clone();
        self.rt.handle().spawn(async move {
            let started = Instant::now();
            let execution = if let Some(script) = script {
                let mut blueprint = match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
                    script.trim(),
                    &runtime_tools,
                ) {
                    Ok(blueprint) => blueprint,
                    Err(error) => {
                        let diagnostic = workflow_diagnostic(&error);
                        publish_async_workflow_failure(
                            &module,
                            &task_run_id,
                            None,
                            "script",
                            1,
                            400,
                            format!(
                                "workflow script compile failed at line {}: {}",
                                error.line, error.message
                            ),
                            Some(json!([diagnostic])),
                        )
                        .await;
                        return;
                    }
                };
                if blueprint.metadata.name.trim().is_empty() {
                    blueprint.metadata.name = "Inline Workflow".to_string();
                }
                module
                    .execute_from_blueprint_outcome_with_context_and_run_id(
                        blueprint,
                        inputs,
                        &execution_context,
                        Some(task_run_id.clone()),
                    )
                    .await
            } else {
                let workflow_id = workflow_id.clone().unwrap_or_default();
                let resource = match module.read_workflow_resource(&workflow_id) {
                    Ok(resource) => resource,
                    Err(error) => {
                        publish_async_workflow_failure(
                            &module,
                            &task_run_id,
                            Some(&workflow_id),
                            "catalog",
                            1,
                            404,
                            error.to_string(),
                            None,
                        )
                        .await;
                        return;
                    }
                };
                match resource.summary.kind {
                    corework::workflow::workflows::WorkflowResourceKind::Registered => {
                        if mode.as_deref().is_some_and(|value| value != "production") {
                            publish_async_workflow_failure(
                                &module,
                                &task_run_id,
                                Some(&workflow_id),
                                "registered",
                                1,
                                400,
                                "registered workflow mode must be 'production' when provided"
                                    .to_string(),
                                None,
                            )
                            .await;
                            return;
                        }
                        module
                            .execute_registered_outcome_with_context_and_run_id(
                                &workflow_id,
                                inputs,
                                &execution_context,
                                Some(task_run_id.clone()),
                            )
                            .await
                    }
                    corework::workflow::workflows::WorkflowResourceKind::Draft => {
                        if mode.as_deref() != Some("test") {
                            publish_async_workflow_failure(
                                &module,
                                &task_run_id,
                                Some(&workflow_id),
                                "draft",
                                1,
                                400,
                                "draft workflow execution requires payload.mode = 'test'"
                                    .to_string(),
                                None,
                            )
                            .await;
                            return;
                        }
                        let blueprint = match module.draft_blueprint(&workflow_id) {
                            Ok(blueprint) => blueprint,
                            Err(error) => {
                                publish_async_workflow_failure(
                                    &module,
                                    &task_run_id,
                                    Some(&workflow_id),
                                    "draft",
                                    1,
                                    400,
                                    error.to_string(),
                                    None,
                                )
                                .await;
                                return;
                            }
                        };
                        module
                            .execute_from_blueprint_outcome_with_context_and_run_id(
                                blueprint,
                                inputs,
                                &execution_context,
                                Some(task_run_id.clone()),
                            )
                            .await
                    }
                }
            };

            match execution {
                Ok(outcome) => {
                    let duration_ms = started.elapsed().as_millis();
                    let error = outcome.error.clone();
                    let trace_meta = workflow_trace_meta(&outcome);
                    let response = workflow_execution_response(outcome, duration_ms, false);
                    let succeeded = error.is_none();
                    let result = response
                        .pointer("/result/outputs")
                        .cloned()
                        .unwrap_or(Value::Null);
                    if succeeded {
                        let result_bytes = serde_json::to_vec(&result)
                            .map(|bytes| bytes.len())
                            .unwrap_or(usize::MAX);
                        if result_bytes > MAX_WORKFLOW_RESULT_BYTES {
                            publish_async_workflow_failure_kind(
                                &module,
                                &task_run_id,
                                trace_meta.workflow_id.as_deref(),
                                source,
                                trace_meta.completed_sequence.unwrap_or(1),
                                413,
                                "WORKFLOW_RESULT_TOO_LARGE",
                                format!(
                                    "workflow result is {result_bytes} bytes, above the event-channel limit of {MAX_WORKFLOW_RESULT_BYTES} bytes"
                                ),
                                None,
                            )
                            .await;
                            return;
                        }
                    }
                    let mut payload = json!({
                        "schema": "agent-runtime-workflow-trace/v1",
                        "trace_type": if succeeded { "workflow.completed" } else { "workflow.failed" },
                        "workflow_run_id": task_run_id,
                        "run_id": task_run_id,
                        "workflow_id": trace_meta.workflow_id,
                        "sequence": trace_meta.completed_sequence.unwrap_or(1),
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "status": if succeeded { "succeeded" } else { "failed" },
                        "code": if succeeded { 0 } else { -1 },
                        "duration_ms": duration_ms
                    });
                    if succeeded {
                        payload["result"] = result;
                    }
                    if let Some(error) = error {
                        payload["error"] = json!({
                            "code": "WORKFLOW_EXECUTION_FAILED",
                            "message": error,
                            "node_id": trace_meta.failed_node_id,
                            "execution_id": trace_meta.failed_execution_id
                        });
                    }
                    module.publish_execution_event(payload).await;
                }
                Err(error) => {
                    publish_async_workflow_failure(
                        &module,
                        &task_run_id,
                        workflow_id.as_deref(),
                        source,
                        1,
                        400,
                        error.to_string(),
                        None,
                    )
                    .await;
                }
            }
        });

        Ok(json!({
            "schema": "agent-runtime-workflow-run/v1",
            "workflow_run_id": workflow_run_id,
            "run_id": workflow_run_id,
            "status": "running"
        }))
    }

    pub fn workflow_script_to_blueprint(&self, script: &str) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let script = script.trim();
        if script.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "workflow script must not be empty".to_string(),
            ));
        }
        match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
            script,
            &self.runtime_tools,
        ) {
            Ok(blueprint) => Ok(json!({
                "schema": "agent-runtime-workflow-conversion/v1",
                "script": script,
                "blueprint": blueprint,
                "validation": corework::workflow::workflows::WorkflowValidation::valid()
            })),
            Err(error) => Ok(json!({
                "schema": "agent-runtime-workflow-conversion/v1",
                "script": script,
                "blueprint": null,
                "validation": workflow_compile_validation(&error)
            })),
        }
    }

    pub fn workflow_blueprint_to_script(
        &self,
        blueprint_value: &Value,
    ) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let blueprint = BlueprintJson::from_json_value(blueprint_value.clone())
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        blueprint.validate().map_err(|error| {
            RuntimeError::InvalidConfig(format!("workflow blueprint validation failed: {error}"))
        })?;
        let script = corework::workflow::chain_decompiler::decompile_chain(&blueprint)
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        Ok(json!({
            "schema": "agent-runtime-workflow-conversion/v1",
            "script": script,
            "blueprint": blueprint,
            "validation": corework::workflow::workflows::WorkflowValidation::valid()
        }))
    }

    pub fn create_workflow_draft(&mut self, input: &Value) -> Result<Value, RuntimeError> {
        let prepared = prepare_draft_resource(input, &self.runtime_tools, None)?;
        let module = self.workflow_module()?;
        let workflow = self
            .rt
            .block_on(module.create_draft_resource(
                prepared.id.as_deref(),
                &prepared.name,
                &prepared.description,
                prepared.script,
                prepared.blueprint,
                prepared.validation,
            ))
            .map_err(workflow_input_error)?;
        serde_json::to_value(workflow).map_err(|error| RuntimeError::Internal(error.to_string()))
    }

    pub fn read_workflow_resource(&self, id: &str) -> Result<Value, RuntimeError> {
        let workflow = self
            .workflow_module()?
            .read_workflow_resource(id)
            .map_err(workflow_input_error)?;
        serde_json::to_value(workflow).map_err(|error| RuntimeError::Internal(error.to_string()))
    }

    pub fn update_workflow_resource(
        &mut self,
        input: &Value,
        expected_revision: Option<u64>,
    ) -> Result<Value, RuntimeError> {
        let parsed = parse_workflow_resource_input(input)?;
        let id = parsed
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                RuntimeError::InvalidConfig("workflow resource id is required".to_string())
            })?
            .to_string();
        let current = self
            .workflow_module()?
            .read_workflow_resource(&id)
            .map_err(workflow_input_error)?;
        let workflow = match current.summary.kind {
            corework::workflow::workflows::WorkflowResourceKind::Draft => {
                let prepared =
                    prepare_draft_from_input(parsed, &self.runtime_tools, Some(&current))?;
                let module = self.workflow_module()?;
                self.rt
                    .block_on(module.update_draft_resource(
                        &id,
                        expected_revision,
                        &prepared.name,
                        &prepared.description,
                        prepared.script,
                        prepared.blueprint,
                        prepared.validation,
                    ))
                    .map_err(workflow_input_error)?
            }
            corework::workflow::workflows::WorkflowResourceKind::Registered => {
                let blueprint = prepare_registered_update(parsed, &self.runtime_tools, &current)?;
                let module = self.workflow_module()?;
                let workflow = self
                    .rt
                    .block_on(module.update_registered_resource(&blueprint, expected_revision))
                    .map_err(workflow_input_error)?;
                self.sync_parent_workflow_registry()?;
                workflow
            }
        };
        serde_json::to_value(workflow).map_err(|error| RuntimeError::Internal(error.to_string()))
    }

    pub fn register_workflow_draft(
        &mut self,
        id: &str,
        expected_revision: Option<u64>,
        registered_name: Option<&str>,
    ) -> Result<Value, RuntimeError> {
        let current = self
            .workflow_module()?
            .read_workflow_resource(id)
            .map_err(workflow_input_error)?;
        if current.summary.kind != corework::workflow::workflows::WorkflowResourceKind::Draft {
            return Err(RuntimeError::InvalidConfig(format!(
                "workflow '{}' is already registered",
                id
            )));
        }
        let module = self.workflow_module()?;
        let workflow = self
            .rt
            .block_on(module.register_draft_resource(id, expected_revision, registered_name))
            .map_err(workflow_input_error)?;
        self.sync_parent_workflow_registry()?;
        serde_json::to_value(workflow).map_err(|error| RuntimeError::Internal(error.to_string()))
    }

    pub fn delete_workflow_resource(
        &mut self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<Value, RuntimeError> {
        let module = self.workflow_module()?;
        let workflow = self
            .rt
            .block_on(module.delete_catalog_resource(id, expected_revision))
            .map_err(workflow_input_error)?;
        if workflow.kind == corework::workflow::workflows::WorkflowResourceKind::Registered {
            self.sync_parent_workflow_registry()?;
        }
        let tombstone_revision = workflow.revision.saturating_add(1);
        Ok(json!({"deleted": workflow, "revision": tombstone_revision}))
    }

    pub fn list_workflow_resources(
        &self,
        kind: Option<corework::workflow::workflows::WorkflowResourceKind>,
    ) -> Result<Value, RuntimeError> {
        let workflows = self
            .workflow_module()?
            .list_workflow_catalog(kind)
            .map_err(workflow_input_error)?;
        Ok(json!({"workflows": workflows}))
    }

    pub fn compile_workflow_draft(&self, id: &str) -> Result<Value, RuntimeError> {
        let workflow = self
            .workflow_module()?
            .read_workflow_resource(id)
            .map_err(workflow_input_error)?;
        if workflow.summary.kind != corework::workflow::workflows::WorkflowResourceKind::Draft {
            return Err(RuntimeError::InvalidConfig(format!(
                "workflow '{}' is registered; compile is only available for drafts",
                id
            )));
        }
        Ok(json!({
            "workflow_id": workflow.summary.id,
            "kind": "draft",
            "revision": workflow.summary.revision,
            "validation": workflow.summary.validation,
            "blueprint": workflow.blueprint
        }))
    }

    pub fn execute_workflow_resource(
        &mut self,
        id: &str,
        mode: Option<&str>,
        inputs: HashMap<String, Value>,
        trace_enabled: bool,
        execution_context: &corework::workflow::workflows::WorkflowExecutionContext,
    ) -> Result<Value, RuntimeError> {
        let workflow = match self.workflow_module()?.read_workflow_resource(id) {
            Ok(workflow) => workflow,
            Err(error) => {
                let message = error.to_string();
                self.publish_workflow_event(
                    WORKFLOW_EXECUTION_COMPLETED_EVENT,
                    json!({
                        "source": "catalog",
                        "workflow_id": id,
                        "status": "failed",
                        "code": 404,
                        "duration_ms": 0,
                        "error": message
                    }),
                );
                return Ok(workflow_failure_response(404, message));
            }
        };
        match workflow.summary.kind {
            corework::workflow::workflows::WorkflowResourceKind::Registered => {
                if mode.is_some_and(|mode| mode != "production") {
                    return Err(RuntimeError::InvalidConfig(
                        "registered workflow mode must be 'production' when provided".to_string(),
                    ));
                }
                self.execute_registered_workflow(id, inputs, trace_enabled, execution_context)
            }
            corework::workflow::workflows::WorkflowResourceKind::Draft => {
                if mode != Some("test") {
                    return Err(RuntimeError::InvalidConfig(
                        "draft workflow execution requires payload.mode = 'test'".to_string(),
                    ));
                }
                let blueprint = self
                    .workflow_module()?
                    .draft_blueprint(id)
                    .map_err(workflow_input_error)?;
                self.execute_blueprint_workflow(
                    "draft",
                    id,
                    Some(workflow.summary.revision),
                    blueprint,
                    inputs,
                    trace_enabled,
                    execution_context,
                )
            }
        }
    }

    pub fn execute_registered_workflow(
        &mut self,
        selector: &str,
        inputs: HashMap<String, Value>,
        trace_enabled: bool,
        execution_context: &corework::workflow::workflows::WorkflowExecutionContext,
    ) -> Result<Value, RuntimeError> {
        let module = self.workflow_module()?;
        let started = Instant::now();
        let execution = self
            .rt
            .block_on(module.execute_registered_outcome_with_context(
                selector,
                inputs,
                execution_context,
            ));
        let outcome = match execution {
            Ok(outcome) => outcome,
            Err(error) => {
                let duration_ms = started.elapsed().as_millis();
                let message = error.to_string();
                let code = if message.contains("does not exist") {
                    404
                } else {
                    400
                };
                self.publish_workflow_event(
                    WORKFLOW_EXECUTION_COMPLETED_EVENT,
                    json!({
                        "source": "registered",
                        "workflow_id": selector,
                        "status": "failed",
                        "code": code,
                        "duration_ms": duration_ms,
                        "error": message
                    }),
                );
                return Ok(workflow_failure_response(code, message));
            }
        };
        let duration_ms = started.elapsed().as_millis();
        let execution_error = outcome.error.clone();
        let code = if outcome.error.is_some() { -1 } else { 0 };
        let trace_meta = workflow_trace_meta(&outcome);
        let response = workflow_execution_response(outcome, duration_ms, trace_enabled);
        let mut event_payload = json!({
            "source": "registered",
            "workflow_id": selector,
            "status": if code == 0 { "succeeded" } else { "failed" },
            "code": code,
            "duration_ms": duration_ms,
            "run_id": trace_meta.run_id,
            "sequence": trace_meta.completed_sequence
        });
        if let Some(error) = execution_error {
            event_payload["error"] = Value::String(error);
        }
        self.publish_workflow_event(WORKFLOW_EXECUTION_COMPLETED_EVENT, event_payload);
        Ok(response)
    }

    fn execute_blueprint_workflow(
        &mut self,
        source: &str,
        id: &str,
        revision: Option<u64>,
        blueprint: BlueprintJson,
        inputs: HashMap<String, Value>,
        trace_enabled: bool,
        execution_context: &corework::workflow::workflows::WorkflowExecutionContext,
    ) -> Result<Value, RuntimeError> {
        let started = Instant::now();
        let execution = self.rt.block_on(
            self.workflow_module()?
                .execute_from_blueprint_outcome_with_context(blueprint, inputs, execution_context),
        );
        let duration_ms = started.elapsed().as_millis();
        let outcome = match execution {
            Ok(outcome) => outcome,
            Err(error) => {
                let message = error.to_string();
                self.publish_workflow_event(
                    WORKFLOW_EXECUTION_COMPLETED_EVENT,
                    json!({
                        "source": source,
                        "workflow_id": id,
                        "revision": revision,
                        "execution_mode": "test",
                        "trust": "untrusted",
                        "status": "failed",
                        "code": 400,
                        "duration_ms": duration_ms,
                        "error": message
                    }),
                );
                return Ok(workflow_failure_response(400, message));
            }
        };
        let execution_error = outcome.error.clone();
        let code = if execution_error.is_some() { -1 } else { 0 };
        let trace_meta = workflow_trace_meta(&outcome);
        let mut response = workflow_execution_response(outcome, duration_ms, trace_enabled);
        response["source"] = Value::String(source.to_string());
        response["trust"] = Value::String("untrusted".to_string());
        response["execution_mode"] = Value::String("test".to_string());
        self.publish_workflow_event(
            WORKFLOW_EXECUTION_COMPLETED_EVENT,
            json!({
                "source": source,
                "workflow_id": id,
                "revision": revision,
                "execution_mode": "test",
                "trust": "untrusted",
                "status": if code == 0 { "succeeded" } else { "failed" },
                "code": code,
                "duration_ms": duration_ms,
                "run_id": trace_meta.run_id,
                "sequence": trace_meta.completed_sequence,
                "error": execution_error
            }),
        );
        Ok(response)
    }

    pub fn execute_workflow_script(
        &mut self,
        script: &str,
        inputs: HashMap<String, Value>,
        trace_enabled: bool,
        execution_context: &corework::workflow::workflows::WorkflowExecutionContext,
    ) -> Result<Value, RuntimeError> {
        let script = script.trim();
        if script.is_empty() {
            return Ok(workflow_failure_response(
                400,
                "workflow script must not be empty".to_string(),
            ));
        }
        let mut blueprint =
            match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
                script,
                &self.runtime_tools,
            ) {
                Ok(blueprint) => blueprint,
                Err(error) => {
                    let trace = format!(
                        "workflow script compile failed at line {}: {}",
                        error.line, error.message
                    );
                    let diagnostic = workflow_diagnostic(&error);
                    self.publish_workflow_event(
                        WORKFLOW_EXECUTION_COMPLETED_EVENT,
                        json!({
                            "source": "script",
                            "status": "failed",
                            "code": 400,
                            "duration_ms": 0,
                            "error": trace,
                            "diagnostics": [diagnostic.clone()]
                        }),
                    );
                    return Ok(workflow_compile_failure_response(400, trace, diagnostic));
                }
            };
        if blueprint.metadata.name.trim().is_empty() {
            blueprint.metadata.name = "Inline Workflow".to_string();
        }
        let module = self.workflow_module()?;
        let started = Instant::now();
        let execution = self
            .rt
            .block_on(module.execute_from_blueprint_outcome_with_context(
                blueprint,
                inputs,
                execution_context,
            ));
        let outcome = match execution {
            Ok(outcome) => outcome,
            Err(error) => {
                let duration_ms = started.elapsed().as_millis();
                let message = error.to_string();
                self.publish_workflow_event(
                    WORKFLOW_EXECUTION_COMPLETED_EVENT,
                    json!({
                        "source": "script",
                        "status": "failed",
                        "code": 400,
                        "duration_ms": duration_ms,
                        "error": message
                    }),
                );
                return Ok(workflow_failure_response(400, message));
            }
        };
        let duration_ms = started.elapsed().as_millis();
        let execution_error = outcome.error.clone();
        let code = if outcome.error.is_some() { -1 } else { 0 };
        let trace_meta = workflow_trace_meta(&outcome);
        let response = workflow_execution_response(outcome, duration_ms, trace_enabled);
        let mut event_payload = json!({
            "source": "script",
            "status": if code == 0 { "succeeded" } else { "failed" },
            "code": code,
            "duration_ms": duration_ms,
            "workflow_id": trace_meta.workflow_id,
            "run_id": trace_meta.run_id,
            "sequence": trace_meta.completed_sequence
        });
        if let Some(error) = execution_error {
            event_payload["error"] = Value::String(error);
        }
        self.publish_workflow_event(WORKFLOW_EXECUTION_COMPLETED_EVENT, event_payload);
        Ok(response)
    }

    pub(super) fn workflow_module(&self) -> Result<Arc<WorkflowsModule>, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        self.workflow_module
            .as_ref()
            .cloned()
            .ok_or(RuntimeError::NotStarted)
    }

    fn sync_parent_workflow_registry(&mut self) -> Result<(), RuntimeError> {
        let module = self.workflow_module()?;
        let registry = scan_parent_workflow_registry(Some(module.workflows_dir()))?;
        self.manager()?
            .unit()
            .world()
            .set_resource(PARENT_WORKFLOW_REGISTRY, &registry, None)
            .map_err(|error| {
                RuntimeError::Internal(format!(
                    "synchronize parent workflow registry failed: {error}"
                ))
            })?;
        if let Some(resources) = self.registries.resources.as_mut() {
            resources.workflow_registry = registry;
        }
        Ok(())
    }

    fn publish_workflow_event(&self, event_type: &str, payload: Value) {
        debug_assert_eq!(event_type, WORKFLOW_EXECUTION_COMPLETED_EVENT);
        let Some(module) = self.workflow_module.as_ref() else {
            return;
        };
        self.rt.block_on(module.publish_execution_event(payload));
    }
}

async fn publish_async_workflow_failure(
    module: &WorkflowsModule,
    run_id: &str,
    workflow_id: Option<&str>,
    source: &str,
    sequence: u64,
    code: i32,
    message: String,
    diagnostics: Option<Value>,
) {
    publish_async_workflow_failure_kind(
        module,
        run_id,
        workflow_id,
        source,
        sequence,
        code,
        "WORKFLOW_PREPARATION_FAILED",
        message,
        diagnostics,
    )
    .await;
}

async fn publish_async_workflow_failure_kind(
    module: &WorkflowsModule,
    run_id: &str,
    workflow_id: Option<&str>,
    source: &str,
    sequence: u64,
    code: i32,
    error_kind: &str,
    message: String,
    diagnostics: Option<Value>,
) {
    let mut payload = json!({
        "schema": "agent-runtime-workflow-trace/v1",
        "trace_type": "workflow.failed",
        "workflow_run_id": run_id,
        "run_id": run_id,
        "workflow_id": workflow_id,
        "sequence": sequence,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "source": source,
        "status": "failed",
        "code": code,
        "error": {"code": error_kind, "message": message}
    });
    if let Some(diagnostics) = diagnostics {
        payload["diagnostics"] = diagnostics;
    }
    module.publish_execution_event(payload).await;
}

fn workflow_input_metadata(pin: &corework::workflow::blueprint_json::PinMetadata) -> Value {
    let data_type = pin.data_type.trim();
    let derived_element_type = data_type
        .strip_prefix("Array<")
        .and_then(|value| value.strip_suffix('>'));
    let element_type = pin.element_type.as_deref().or(derived_element_type);
    let format = match data_type {
        "File" => Some("file"),
        _ if element_type == Some("File") => Some("file"),
        _ => None,
    };
    json!({
        "name": pin.name,
        "display_name": pin.display_name.as_deref().unwrap_or(&pin.name),
        "type": if element_type.is_some() { "Array" } else { data_type },
        "required": pin.required.unwrap_or_else(|| pin.default_value.is_none()),
        "default": pin.default_value,
        "description": pin.description,
        "sensitive": pin.sensitive,
        "element_type": element_type,
        "format": pin.format.as_deref().or(format)
    })
}

fn workflow_node_input_metadata(pin: &corework::workflow::blueprint_json::NodePin) -> Value {
    let data_type = pin.data_type.trim();
    let element_type = data_type
        .strip_prefix("Array<")
        .and_then(|value| value.strip_suffix('>'));
    let format = match data_type {
        "File" => Some("file"),
        _ if element_type == Some("File") => Some("file"),
        _ => None,
    };
    json!({
        "name": pin.name,
        "display_name": pin.name,
        "type": if element_type.is_some() { "Array" } else { data_type },
        "required": pin.default_value.is_none(),
        "default": pin.default_value,
        "description": pin.description,
        "sensitive": false,
        "element_type": element_type,
        "format": format
    })
}

struct PreparedDraftResource {
    id: Option<String>,
    name: String,
    description: String,
    script: Option<String>,
    blueprint: Option<BlueprintJson>,
    validation: corework::workflow::workflows::WorkflowValidation,
}

fn parse_workflow_resource_input(input: &Value) -> Result<WorkflowResourceInput, RuntimeError> {
    let input: WorkflowResourceInput = serde_json::from_value(input.clone()).map_err(|error| {
        RuntimeError::InvalidConfig(format!("parse workflow resource failed: {error}"))
    })?;
    if input.schema != WORKFLOW_RESOURCE_SCHEMA {
        return Err(RuntimeError::InvalidConfig(format!(
            "workflow resource schema '{}' is not supported",
            input.schema
        )));
    }
    Ok(input)
}

fn prepare_draft_resource(
    input: &Value,
    runtime_tools: &[RuntimeToolMetadata],
    current: Option<&corework::workflow::workflows::WorkflowResourceView>,
) -> Result<PreparedDraftResource, RuntimeError> {
    prepare_draft_from_input(
        parse_workflow_resource_input(input)?,
        runtime_tools,
        current,
    )
}

fn prepare_draft_from_input(
    input: WorkflowResourceInput,
    runtime_tools: &[RuntimeToolMetadata],
    current: Option<&corework::workflow::workflows::WorkflowResourceView>,
) -> Result<PreparedDraftResource, RuntimeError> {
    let name = input
        .name
        .or_else(|| current.map(|workflow| workflow.summary.name.clone()))
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            RuntimeError::InvalidConfig("workflow resource name is required".to_string())
        })?;
    let description = input
        .description
        .or_else(|| current.map(|workflow| workflow.summary.description.clone()))
        .unwrap_or_default();
    let (script, mut blueprint, validation) = match (input.script, input.blueprint) {
        (Some(script), None) => {
            match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
                &script,
                runtime_tools,
            ) {
                Ok(mut blueprint) => {
                    if let Some(previous) = current.and_then(|workflow| workflow.blueprint.as_ref())
                    {
                        corework::workflow::workflows::preserve_workflow_blueprint_layout(
                            previous,
                            &mut blueprint,
                        );
                    }
                    (
                        Some(script),
                        Some(blueprint),
                        corework::workflow::workflows::WorkflowValidation::valid(),
                    )
                }
                Err(error) => (Some(script), None, workflow_compile_validation(&error)),
            }
        }
        (None, Some(blueprint)) => {
            let validation = match blueprint.validate() {
                Ok(()) => corework::workflow::workflows::WorkflowValidation::valid(),
                Err(error) => corework::workflow::workflows::WorkflowValidation::invalid(error),
            };
            let script = if validation.valid {
                corework::workflow::chain_decompiler::decompile_chain(&blueprint).ok()
            } else {
                None
            };
            (script, Some(blueprint), validation)
        }
        _ => {
            return Err(RuntimeError::InvalidConfig(
                "workflow resource must provide exactly one of script or blueprint".to_string(),
            ))
        }
    };
    if let Some(blueprint) = blueprint.as_mut() {
        if let Some(id) = input.id.as_ref() {
            blueprint.metadata.id = id.clone();
        }
        blueprint.metadata.name = name.clone();
        blueprint.metadata.description = description.clone();
    }
    Ok(PreparedDraftResource {
        id: input.id,
        name,
        description,
        script,
        blueprint,
        validation,
    })
}

fn prepare_registered_update(
    input: WorkflowResourceInput,
    runtime_tools: &[RuntimeToolMetadata],
    current: &corework::workflow::workflows::WorkflowResourceView,
) -> Result<BlueprintJson, RuntimeError> {
    let id = input.id.clone().ok_or_else(|| {
        RuntimeError::InvalidConfig("workflow resource id is required".to_string())
    })?;
    let mut blueprint = match (input.script, input.blueprint) {
        (Some(script), None) => {
            let mut blueprint =
                corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
                    &script,
                    runtime_tools,
                )
                .map_err(|error| {
                    RuntimeError::InvalidConfig(format!(
                        "workflow resource script compile failed at line {}: {}",
                        error.line, error.message
                    ))
                })?;
            if let Some(previous) = current.blueprint.as_ref() {
                corework::workflow::workflows::preserve_workflow_blueprint_layout(
                    previous,
                    &mut blueprint,
                );
            }
            blueprint
        }
        (None, Some(blueprint)) => blueprint,
        _ => {
            return Err(RuntimeError::InvalidConfig(
                "workflow resource must provide exactly one of script or blueprint".to_string(),
            ))
        }
    };
    blueprint.metadata.id = id;
    blueprint.metadata.name = input.name.unwrap_or_else(|| current.summary.name.clone());
    blueprint.metadata.description = input
        .description
        .unwrap_or_else(|| current.summary.description.clone());
    Ok(blueprint)
}

fn workflow_execution_response(
    outcome: corework::workflow::workflows::executor::WorkflowExecutionOutcome,
    duration_ms: u128,
    include_structured_trace: bool,
) -> Value {
    let run_id = outcome
        .report
        .trace
        .as_ref()
        .map(|trace| trace.run_id.clone());
    let trace = outcome.report.to_ai(
        corework::workflow::execution::WorkflowToAiMode::Detailed,
        outcome.error.as_deref(),
    );
    let mut result = json!({
        "outputs": outcome.report.outputs_json(),
        "duration_ms": duration_ms
    });
    if include_structured_trace {
        result["node_trace"] = serde_json::to_value(&outcome.report.trace).unwrap_or(Value::Null);
    }
    json!({
        "code": if outcome.error.is_some() { -1 } else { 0 },
        "run_id": run_id,
        "trace": trace,
        "result": result
    })
}

struct WorkflowTraceMeta {
    workflow_id: Option<String>,
    run_id: Option<String>,
    completed_sequence: Option<u64>,
    failed_node_id: Option<String>,
    failed_execution_id: Option<String>,
}

fn workflow_trace_meta(
    outcome: &corework::workflow::workflows::executor::WorkflowExecutionOutcome,
) -> WorkflowTraceMeta {
    let trace = outcome.report.trace.as_ref();
    let failed_node = trace.and_then(|trace| {
        trace.nodes.iter().rev().find(|node| {
            matches!(
                node.status,
                corework::workflow::execution::WorkflowNodeStatus::Failed
            )
        })
    });
    WorkflowTraceMeta {
        workflow_id: trace.map(|trace| trace.workflow_id.clone()),
        run_id: trace.map(|trace| trace.run_id.clone()),
        completed_sequence: trace.map(|trace| trace.event_count.saturating_add(1)),
        failed_node_id: failed_node.and_then(|node| node.node_id.clone()),
        failed_execution_id: failed_node.map(|node| node.execution_id.clone()),
    }
}

fn workflow_failure_response(code: i32, trace: String) -> Value {
    json!({
        "code": code,
        "trace": trace
    })
}

fn workflow_compile_failure_response(
    code: i32,
    trace: String,
    diagnostic: corework::workflow::workflows::catalog::WorkflowDiagnostic,
) -> Value {
    json!({
        "code": code,
        "trace": trace,
        "diagnostics": [diagnostic]
    })
}

fn workflow_diagnostic(
    error: &corework::workflow::chain_compiler::ChainError,
) -> corework::workflow::workflows::catalog::WorkflowDiagnostic {
    corework::workflow::workflows::catalog::WorkflowDiagnostic {
        severity: "error".to_string(),
        kind: error.kind.as_str().to_string(),
        line: error.line,
        col: error.col,
        message: error.message.clone(),
        suggestion: error.suggestion.clone(),
    }
}

fn workflow_compile_validation(
    error: &corework::workflow::chain_compiler::ChainError,
) -> corework::workflow::workflows::WorkflowValidation {
    corework::workflow::workflows::WorkflowValidation::invalid_with_diagnostic(
        format!(
            "script compile failed at line {}: {}",
            error.line, error.message
        ),
        workflow_diagnostic(error),
    )
}

fn workflow_input_error(error: corework::error::FrameworkError) -> RuntimeError {
    RuntimeError::InvalidConfig(error.to_string())
}
