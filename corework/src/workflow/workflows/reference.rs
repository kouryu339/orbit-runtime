//! Registered workflows are the authoritative catalog for reference nodes.
use super::executor::{WorkflowExecutionContext, WorkflowsModule};
use crate::error::{FrameworkError, Result};
use crate::workflow::blueprint_json::{BlueprintJson, BlueprintNodeJson};
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const PREFIX: &str = "WorkflowRef_";
pub fn is_reference(node_type: &str) -> bool {
    node_type.starts_with(PREFIX)
}
fn error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::WorkflowError(message.into())
}

impl WorkflowsModule {
    fn reference_catalog_unlocked(&self) -> Result<HashMap<String, BlueprintJson>> {
        if let Some(snapshot) = &self.reference_snapshot {
            return Ok((**snapshot).clone());
        }
        self.get_registry()?
            .into_iter()
            .map(|entry| {
                let raw = std::fs::read_to_string(&entry.file_path).map_err(|e| {
                    error(format!(
                        "Read registered workflow '{}': {e}",
                        entry.metadata.id
                    ))
                })?;
                let blueprint =
                    BlueprintJson::from_json_str(&raw).map_err(|e| error(e.to_string()))?;
                if blueprint.metadata.id != entry.metadata.id {
                    return Err(error(format!(
                        "Registered workflow '{}' has mismatched file identity",
                        entry.metadata.id
                    )));
                }
                Ok((entry.metadata.id, blueprint))
            })
            .collect()
    }

    pub(crate) fn validate_references_unlocked(&self, candidate: &BlueprintJson) -> Result<()> {
        reference_script(candidate)?;
        let mut catalog = self.reference_catalog_unlocked()?;
        catalog.insert(candidate.metadata.id.clone(), candidate.clone());
        let mut done = HashSet::new();
        for blueprint in catalog.values() {
            validate(blueprint, &catalog, &mut Vec::new(), &mut done)?;
        }
        Ok(())
    }

    pub(crate) fn validate_reference_catalog_unlocked(&self) -> Result<()> {
        let catalog = self.reference_catalog_unlocked()?;
        let mut done = HashSet::new();
        for blueprint in catalog.values() {
            reference_script(blueprint)?;
            validate(blueprint, &catalog, &mut Vec::new(), &mut done)?;
        }
        Ok(())
    }

    pub fn validate_workflow_references(&self, candidate: &BlueprintJson) -> Result<()> {
        let _guard = self.reference_lock.lock();
        self.validate_references_unlocked(candidate)
    }

    pub(crate) fn ensure_reference_unused_unlocked(&self, id: &str) -> Result<()> {
        for (owner, blueprint) in self.reference_catalog_unlocked()? {
            if owner != id
                && blueprint
                    .nodes
                    .iter()
                    .any(|node| node.node_type == format!("{PREFIX}{id}"))
            {
                return Err(error(format!("Workflow '{id}' is referenced by '{owner}'")));
            }
        }
        Ok(())
    }

    pub(crate) fn freeze_references(&self, root: &BlueprintJson) -> Result<Self> {
        if let Some(catalog) = &self.reference_snapshot {
            validate(root, catalog, &mut Vec::new(), &mut HashSet::new())?;
            return Ok(self.clone());
        }
        let _guard = self.reference_lock.lock();
        self.freeze_references_unlocked(root)
    }

    pub(crate) fn freeze_references_unlocked(&self, root: &BlueprintJson) -> Result<Self> {
        let mut catalog = self.reference_catalog_unlocked()?;
        let mut reachable = HashSet::new();
        validate(root, &catalog, &mut Vec::new(), &mut reachable)?;
        catalog.retain(|id, _| reachable.contains(id));
        let mut frozen = self.clone();
        frozen.reference_snapshot = Some(Arc::new(catalog));
        Ok(frozen)
    }

    /// Complete compiler-ready node JSON. Instance IDs belong to the caller's graph.
    pub fn workflow_reference_definition(&self, workflow_id: &str, node_id: &str) -> Result<Value> {
        let _guard = self.reference_lock.lock();
        let catalog = self.reference_catalog_unlocked()?;
        let blueprint = catalog
            .get(workflow_id)
            .ok_or_else(|| error(format!("Workflow '{workflow_id}' is not registered")))?;
        Ok(json!({
            "schema": "agent-runtime-workflow-reference-node/v1",
            "category": "workflow/reference",
            "node": node_json(blueprint, node_id),
            "reference_script": reference_script(blueprint)?,
        }))
    }

    pub fn workflow_reference_definitions(&self) -> Result<Vec<Value>> {
        let _guard = self.reference_lock.lock();
        let catalog = self.reference_catalog_unlocked()?;
        let mut ids = catalog.keys().collect::<Vec<_>>();
        ids.sort();
        Ok(ids
            .into_iter()
            .map(|id| {
                json!({
                    "category": "workflow/reference", "node": node_json(&catalog[id], "")
                })
            })
            .collect())
    }

    pub fn workflow_reference_tools(&self) -> Result<Vec<crate::rpc_tool::RuntimeToolMetadata>> {
        let _guard = self.reference_lock.lock();
        self.workflow_reference_tools_unlocked()
    }

    pub(crate) fn workflow_reference_tools_unlocked(
        &self,
    ) -> Result<Vec<crate::rpc_tool::RuntimeToolMetadata>> {
        self.reference_catalog_unlocked()?
            .values()
            .map(reference_tool)
            .collect()
    }

    pub(crate) fn reference_node(
        self: &Arc<Self>,
        node: &BlueprintNodeJson,
        name: &str,
    ) -> Result<ReferenceNode> {
        let id = node
            .node_type
            .strip_prefix(PREFIX)
            .ok_or_else(|| error("Invalid reference type"))?;
        let catalog = self
            .reference_snapshot
            .as_ref()
            .ok_or_else(|| error("Workflow reference catalog is not frozen"))?;
        let target = catalog
            .get(id)
            .ok_or_else(|| error(format!("Workflow '{id}' is not registered")))?;
        validate_pins(node, target)?;
        Ok(ReferenceNode {
            name: name.into(),
            target: target.clone(),
            module: Arc::clone(self),
        })
    }
}

fn node_json(blueprint: &BlueprintJson, node_id: &str) -> Value {
    let mut pins = vec![
        json!({"name":"In", "kind":"ExecInput"}),
        json!({"name":"Then", "kind":"ExecOutput"}),
    ];
    let inputs = interface(blueprint, true);
    let outputs = interface(blueprint, false);
    for (kind, definitions) in [("DataInput", &inputs), ("DataOutput", &outputs)] {
        pins.extend(definitions.iter().map(|pin| json!({"name":pin.name, "kind":kind,
            "data_type":pin.data_type, "description":pin.description, "default_value":pin.default_value})));
    }
    json!({"id":node_id, "node_type":format!("{PREFIX}{}", blueprint.metadata.id),
        "display_name":format!("执行，{}", blueprint.metadata.name),
        "position":{"x":0,"y":0}, "size":{"width":240,"height":48 + 24 * (1 + inputs.len().max(outputs.len()))},
        "pins":pins, "properties":{"workflow_id":blueprint.metadata.id}})
}

fn interface(
    blueprint: &BlueprintJson,
    input: bool,
) -> Vec<crate::workflow::blueprint_json::NodePin> {
    let (node_type, kind) = if input {
        ("StartNode", "DataOutput")
    } else {
        ("EndNode", "DataInput")
    };
    blueprint
        .nodes
        .iter()
        .filter(|node| node.node_type == node_type)
        .flat_map(|node| node.pins.iter())
        .filter(|pin| pin.kind == kind)
        .map(|pin| {
            let mut pin = pin.clone();
            let metadata = if input {
                &blueprint.metadata.inputs
            } else {
                &blueprint.metadata.outputs
            };
            if let Some(meta) = metadata.iter().find(|meta| meta.name == pin.name) {
                if pin.description.is_empty() {
                    pin.description = meta.description.clone();
                }
                if input && pin.default_value.is_none() {
                    pin.default_value = meta.default_value.clone();
                }
            }
            if input {
                if let Some(source) = blueprint
                    .nodes
                    .iter()
                    .filter(|node| node.node_type == "StartNode")
                    .flat_map(|node| &node.pins)
                    .find(|source| source.kind == "DataInput" && source.name == pin.name)
                {
                    if pin.description.is_empty() {
                        pin.description = source.description.clone();
                    }
                    if pin.default_value.is_none() {
                        pin.default_value = source.default_value.clone();
                    }
                }
            }
            pin
        })
        .collect()
}

fn validate_pins(node: &BlueprintNodeJson, target: &BlueprintJson) -> Result<()> {
    let expected: BlueprintNodeJson =
        serde_json::from_value(node_json(target, &node.id)).map_err(|e| error(e.to_string()))?;
    let signature = |node: &BlueprintNodeJson| {
        node.pins
            .iter()
            .map(|pin| (pin.name.clone(), pin.kind.clone(), pin.data_type.clone()))
            .collect::<HashSet<_>>()
    };
    if node.pins.len() != expected.pins.len() || signature(node) != signature(&expected) {
        return Err(error(format!("Workflow reference '{}' interface differs from registered workflow '{}'; regenerate the reference node", node.id, target.metadata.id)));
    }
    if node
        .properties
        .get("workflow_id")
        .is_some_and(|id| id.as_str() != Some(target.metadata.id.as_str()))
    {
        return Err(error(format!(
            "Workflow reference '{}' has mismatched workflow_id",
            node.id
        )));
    }
    Ok(())
}

fn reference_tool(blueprint: &BlueprintJson) -> Result<crate::rpc_tool::RuntimeToolMetadata> {
    serde_json::from_value(json!({
        "name":format!("{PREFIX}{}",blueprint.metadata.id), "tool_kind":"workflow",
        "display_name":format!("执行，{}",blueprint.metadata.name), "description":blueprint.metadata.description,
        "parameters":interface(blueprint,true).iter().map(|p| json!({"name":p.name,"param_type":p.data_type,
            "description":p.description,"required":p.default_value.is_none(),"default_value":p.default_value.as_ref().map(Value::to_string)})).collect::<Vec<_>>(),
        "outputs":interface(blueprint,false).iter().map(|p| json!({"name":p.name,"field_type":p.data_type,"description":p.description})).collect::<Vec<_>>()
    })).map_err(|e| error(e.to_string()))
}

pub fn reference_script(blueprint: &BlueprintJson) -> Result<String> {
    let inputs = interface(blueprint, true);
    let outputs = interface(blueprint, false);
    let declarations = inputs
        .iter()
        .map(|p| format!("{}:{}", p.name, p.data_type))
        .collect::<Vec<_>>()
        .join(" ");
    let arguments = inputs
        .iter()
        .map(|p| format!("--{} input.{}", p.name, p.name))
        .collect::<Vec<_>>()
        .join(" ");
    let returns = outputs
        .iter()
        .map(|p| format!("{}=1.{}", p.name, p.name))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "input {declarations}\n1: EXEC {PREFIX}{} {arguments}\nreturn {returns}",
        blueprint.metadata.id
    );
    crate::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
        &script,
        &[reference_tool(blueprint)?],
    )
    .map_err(|e| {
        error(format!(
            "Cannot generate reference script for '{}': {}",
            blueprint.metadata.id, e.message
        ))
    })?;
    Ok(script)
}

fn validate(
    blueprint: &BlueprintJson,
    catalog: &HashMap<String, BlueprintJson>,
    path: &mut Vec<String>,
    done: &mut HashSet<String>,
) -> Result<()> {
    let id = &blueprint.metadata.id;
    if path.contains(id) {
        path.push(id.clone());
        return Err(error(format!(
            "Workflow reference cycle: {}",
            path.join(" → ")
        )));
    }
    if done.contains(id) {
        return Ok(());
    }
    if path.len() >= 64 {
        return Err(error("Workflow reference depth exceeds 64"));
    }
    path.push(id.clone());
    for node in &blueprint.nodes {
        if matches!(
            node.node_type.as_str(),
            "executeWorkflow" | "executeWorkflowScript" | "testWorkflow"
        ) {
            return Err(error(
                "Nested workflows must use registered workflow reference nodes",
            ));
        }
        if let Some(target_id) = node.node_type.strip_prefix(PREFIX) {
            if path.iter().any(|id| id == target_id) {
                let mut cycle = path.clone();
                cycle.push(target_id.into());
                return Err(error(format!(
                    "Workflow reference cycle: {}",
                    cycle.join(" → ")
                )));
            }
            let target = catalog.get(target_id).ok_or_else(|| {
                error(format!(
                    "Workflow reference '{}' targets unregistered workflow '{target_id}'",
                    node.id
                ))
            })?;
            validate_pins(node, target)?;
            validate(target, catalog, path, done)?;
        }
    }
    path.pop();
    done.insert(id.clone());
    Ok(())
}

pub(crate) struct ReferenceNode {
    name: String,
    target: BlueprintJson,
    module: Arc<WorkflowsModule>,
}
impl BlueprintNode for ReferenceNode {
    fn name(&self) -> &str {
        &self.name
    }
    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }
    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::exec_in("In"), Pin::exec_out("Then")];
        pins.extend(
            interface(&self.target, true)
                .iter()
                .map(|p| Pin::data_in(&p.name, &p.data_type)),
        );
        pins.extend(
            interface(&self.target, false)
                .iter()
                .map(|p| Pin::data_out(&p.name, &p.data_type)),
        );
        pins
    }
    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move {
            let origin = WorkflowExecutionContext {
                conversation_id: ctx.inner().conversation_id.clone(),
                agent_id: ctx.inner().get::<String>("agent_id")?,
                turn_id: ctx.inner().get::<String>("turn_id")?,
            };
            let values = inputs.into_iter().map(|(k, v)| (k, v.value)).collect();
            let parent_run_id = ctx.inner().get::<String>("workflow_run_id")?;
            let identity = ctx.trace_run_identity().or_else(|| {
                parent_run_id.map(|run_id| (run_id, Arc::new(std::sync::Mutex::new(0))))
            });
            let outcome = if let Some((run_id, sequence)) = identity {
                self.module
                    .execute_companion_from_blueprint(
                        self.target.clone(),
                        values,
                        &origin,
                        run_id,
                        sequence,
                    )
                    .await?
            } else {
                self.module
                    .execute_from_blueprint_outcome_with_context(
                        self.target.clone(),
                        values,
                        &origin,
                    )
                    .await?
            };
            if let Some(message) = outcome.error {
                return Err(error(message));
            }
            Ok(NodeOutput::Data(outcome.report.outputs))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BaseEvent, EventBus, EventHandler};
    use crate::workflow::chain_compiler_v2::compile_chain_v2;

    #[derive(Default)]
    struct CapturedEvents(std::sync::Mutex<Vec<Value>>);

    #[async_trait::async_trait]
    impl EventHandler for CapturedEvents {
        async fn handle(&self, event: &BaseEvent) -> Result<()> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event.payload.clone());
            Ok(())
        }
    }

    fn echo(id: &str) -> BlueprintJson {
        let mut blueprint =
            compile_chain_v2("input value:String\nreturn result=input.value").unwrap();
        blueprint.metadata.id = id.into();
        blueprint.metadata.name = id.into();
        blueprint
    }
    fn reference(target: &BlueprintJson) -> BlueprintNodeJson {
        serde_json::from_value(node_json(target, "call-child")).unwrap()
    }
    fn caller(target: &BlueprintJson, id: &str) -> BlueprintJson {
        let mut parent = echo(id);
        let start = parent
            .nodes
            .iter()
            .find(|n| n.node_type == "StartNode")
            .unwrap()
            .id
            .clone();
        let end = parent
            .nodes
            .iter()
            .find(|n| n.node_type == "EndNode")
            .unwrap()
            .id
            .clone();
        let ref_node = reference(target);
        let connection =
            |source: &str, source_pin: &str, dest: &str, dest_pin: &str, kind: &str| {
                serde_json::from_value(json!({"source_node":source,"source_pin":source_pin,
                "target_node":dest,"target_pin":dest_pin,"connection_type":kind}))
                .unwrap()
            };
        let start_exec = parent
            .nodes
            .iter()
            .find(|n| n.id == start)
            .unwrap()
            .pins
            .iter()
            .find(|p| p.kind == "ExecOutput")
            .unwrap()
            .name
            .clone();
        let end_exec = parent
            .nodes
            .iter()
            .find(|n| n.id == end)
            .unwrap()
            .pins
            .iter()
            .find(|p| p.kind == "ExecInput")
            .unwrap()
            .name
            .clone();
        parent.connections = vec![
            connection(&start, &start_exec, &ref_node.id, "In", "Exec"),
            connection(&ref_node.id, "Then", &end, &end_exec, "Exec"),
            connection(&start, "value", &ref_node.id, "value", "Data"),
            connection(&ref_node.id, "result", &end, "result", "Data"),
        ];
        parent.nodes.push(ref_node);
        parent
    }

    #[test]
    fn rejects_cycles_missing_targets_and_stale_interfaces() {
        let a = echo("a");
        let mut b = caller(&a, "b");
        let cyclic_a = caller(&b, "a");
        let catalog = HashMap::from([("a".into(), cyclic_a.clone()), ("b".into(), b.clone())]);
        let err = validate(&cyclic_a, &catalog, &mut Vec::new(), &mut HashSet::new()).unwrap_err();
        assert!(err.to_string().contains("a → b → a"), "{err}");
        assert!(validate(&b, &HashMap::new(), &mut Vec::new(), &mut HashSet::new()).is_err());
        b.nodes
            .last_mut()
            .unwrap()
            .pins
            .retain(|p| p.name != "value");
        assert!(validate(
            &b,
            &HashMap::from([("a".into(), a)]),
            &mut Vec::new(),
            &mut HashSet::new()
        )
        .unwrap_err()
        .to_string()
        .contains("interface"));
    }

    #[test]
    fn reference_scripts_compile_for_supported_interfaces_and_ids() {
        for script in ["input\nreturn", "input sku:String count:num payload:Any rows:Array<String> flag:bool\nreturn sku=input.sku count=input.count payload=input.payload rows=input.rows flag=input.flag"] {
            for id in ["wf_123", "inventory-query", "inventory.query"] {
                let mut blueprint = compile_chain_v2(script).unwrap();
                blueprint.metadata.id = id.into();
                reference_script(&blueprint).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn registered_reference_executes_and_preserves_output_types() {
        let _test_guard = super::super::WORKFLOW_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let directory =
            std::env::temp_dir().join(format!("workflow-reference-{}", uuid::Uuid::new_v4()));
        let bus = Arc::new(crate::event::InMemoryEventBus::new());
        let captured = Arc::new(CapturedEvents::default());
        for event_type in [
            super::super::WORKFLOW_EXECUTION_STARTED_EVENT,
            super::super::WORKFLOW_NODE_STARTED_EVENT,
            super::super::WORKFLOW_NODE_COMPLETED_EVENT,
            super::super::WORKFLOW_TRACE_EVENT,
        ] {
            bus.subscribe(event_type.to_string(), captured.clone())
                .await
                .unwrap();
        }
        let module = WorkflowsModule::new_with_event_bus(directory, bus).unwrap();
        let child = echo("child");
        module.persist_resource(&child, false, None).unwrap();
        let definition = module
            .workflow_reference_definition("child", "instance-1")
            .unwrap();
        assert_eq!(definition["node"]["display_name"], "执行，child");
        assert_eq!(definition["node"]["id"], "instance-1");
        assert!(module.workflow_reference_definition("draft", "x").is_err());
        let parent = caller(&child, "parent");
        module.persist_resource(&parent, false, None).unwrap();
        let outcome = module
            .execute_from_blueprint_outcome_with_context(
                parent,
                HashMap::from([("value".into(), json!("hello"))]),
                &WorkflowExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        assert_eq!(outcome.report.outputs_json()["result"], "hello");
        let trace = outcome.report.trace.as_ref().unwrap();
        let events = captured.0.lock().unwrap_or_else(|e| e.into_inner());
        let run_events = events
            .iter()
            .filter(|event| event["workflow_run_id"] == trace.run_id)
            .collect::<Vec<_>>();
        assert!(run_events
            .iter()
            .any(|event| event["workflow_id"] == "parent"));
        assert!(run_events
            .iter()
            .any(|event| event["workflow_id"] == "child"));
        let sequences = run_events
            .iter()
            .filter_map(|event| event["sequence"].as_u64())
            .collect::<Vec<_>>();
        assert_eq!(
            sequences.len(),
            sequences.iter().collect::<HashSet<_>>().len()
        );
        assert_eq!(trace.event_count, *sequences.iter().max().unwrap());
        drop(events);
        let script = reference_script(&child).unwrap();
        let compiled = crate::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
            &script,
            &module.workflow_reference_tools().unwrap(),
        )
        .unwrap();
        let result = module
            .execute_from_blueprint_outcome_with_context(
                compiled,
                HashMap::from([("value".into(), json!("script"))]),
                &WorkflowExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.report.outputs_json()["result"], "script");
        let cyclic_child = caller(&echo("parent"), "child");
        assert!(module
            .persist_resource(&cyclic_child, true, None)
            .unwrap_err()
            .to_string()
            .contains("cycle"));
    }

    #[tokio::test]
    async fn catalog_lifecycle_and_frozen_execution() {
        let _test_guard = super::super::WORKFLOW_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let directory = std::env::temp_dir().join(format!(
            "workflow-reference-lifecycle-{}",
            uuid::Uuid::new_v4()
        ));
        let module = WorkflowsModule::new_with_event_bus(
            directory.clone(),
            Arc::new(crate::event::InMemoryEventBus::new()),
        )
        .unwrap();
        let child = echo("child");
        let draft = module
            .create_draft_resource(
                Some("child"),
                "Child",
                "",
                None,
                Some(child.clone()),
                super::super::WorkflowValidation::valid(),
            )
            .await
            .unwrap();
        assert!(draft.reference_script.is_none());
        assert!(module.workflow_reference_definition("child", "1").is_err());
        let registered = module
            .register_draft_resource("child", Some(1), None)
            .await
            .unwrap();
        assert!(registered.reference_script.is_some());
        let parent = caller(&child, "parent");
        module.persist_resource(&parent, false, None).unwrap();
        assert!(module.delete_resource("child").is_err());
        let frozen = module.freeze_references(&parent).unwrap();
        let mut updated =
            compile_chain_v2("input value:String\nreturn result=\"updated\"").unwrap();
        updated.metadata.id = "child".into();
        updated.metadata.name = "Renamed".into();
        module.persist_resource(&updated, true, None).unwrap();
        assert_eq!(
            module.workflow_reference_definition("child", "1").unwrap()["node"]["node_type"],
            "WorkflowRef_child"
        );
        let old = frozen
            .execute_from_blueprint_outcome_with_context(
                parent.clone(),
                HashMap::from([("value".into(), json!("old"))]),
                &WorkflowExecutionContext::default(),
            )
            .await
            .unwrap();
        assert_eq!(old.report.outputs_json()["result"], "old");
        let new = module
            .execute_from_blueprint_outcome_with_context(
                parent,
                HashMap::from([("value".into(), json!("old"))]),
                &WorkflowExecutionContext::default(),
            )
            .await
            .unwrap();
        assert!(new.error.is_none(), "{:?}", new.error);
        assert_eq!(
            new.report.outputs_json()["result"],
            "updated",
            "{:?}",
            new.report
        );
        module.delete_resource("parent").unwrap();
        module.delete_resource("child").unwrap();
        assert!(module.workflow_reference_definitions().unwrap().is_empty());
    }
}
