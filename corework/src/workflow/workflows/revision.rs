//! One preparation pipeline for both editable workflow representations.
use crate::error::{FrameworkError, Result};
use crate::rpc_tool::RuntimeToolMetadata;
use crate::workflow::{
    blueprint_json::BlueprintJson, chain_compiler_v2::compile_chain_v2_with_runtime_tools,
    chain_decompiler::decompile_chain,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkflowChange {
    Script(String),
    Blueprint(BlueprintJson),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedRevision {
    pub script: String,
    pub blueprint: BlueprintJson,
    pub source: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRequest {
    pub id: String,
    pub expected_revision: u64,
    pub change: WorkflowChange,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl super::WorkflowsModule {
    /// Version check, prepare and commit share the same catalog lock.
    pub async fn revise_workflow(
        &self,
        request: RevisionRequest,
        tools: &[RuntimeToolMetadata],
        commit: bool,
    ) -> Result<Value> {
        let result = {
            let _guard = self.reference_lock.lock();
            let current = self.read_workflow_resource(&request.id)?;
            if current.summary.revision != request.expected_revision {
                return Err(invalid(format!(
                    "Workflow revision conflict: expected {}, current {}",
                    request.expected_revision, current.summary.revision
                )));
            }
            let mut effective = tools.to_vec();
            effective.extend(self.workflow_reference_tools_unlocked()?);
            let previous =
                current
                    .blueprint
                    .clone()
                    .zip(current.script.clone())
                    .map(|(blueprint, script)| PreparedRevision {
                        blueprint,
                        script,
                        source: current.source.clone().unwrap_or_else(|| "script".into()),
                        validator_version: "1".into(),
                    });
            let mut prepared = prepare(request.change, previous.as_ref(), &effective)?;
            prepared.blueprint.metadata.id = current.summary.id.clone();
            prepared.blueprint.metadata.name = request
                .name
                .unwrap_or_else(|| current.summary.name.clone())
                .trim()
                .to_string();
            if prepared.blueprint.metadata.name.is_empty() {
                return Err(invalid("Workflow name must not be empty"));
            }
            self.ensure_name_available(&prepared.blueprint.metadata.name, Some(&request.id))?;
            prepared.blueprint.metadata.description = request
                .description
                .unwrap_or_else(|| current.summary.description.clone());
            self.validate_references_unlocked(&prepared.blueprint)?;
            let frozen = self.freeze_references_unlocked(&prepared.blueprint)?;
            crate::workflow::BlueprintLoader::with_workflows(std::sync::Arc::new(frozen))
                .load_from_blueprint_json(prepared.blueprint.clone(), &self.create_run_context())?;
            let revision = if commit {
                request
                    .expected_revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("Revision overflow"))?
            } else {
                request.expected_revision
            };
            if commit {
                match current.summary.kind {
                    super::WorkflowResourceKind::Registered => {
                        self.persist_resource_unlocked(
                            &prepared.blueprint,
                            true,
                            Some(revision),
                            Some(&prepared),
                        )?;
                    }
                    super::WorkflowResourceKind::Draft => {
                        let mut drafts = self.get_draft_registry()?;
                        let draft = drafts
                            .iter_mut()
                            .find(|d| d.id == request.id)
                            .ok_or_else(|| invalid("Draft disappeared"))?;
                        draft.script = Some(prepared.script.clone());
                        draft.blueprint = Some(prepared.blueprint.clone());
                        draft.revision = revision;
                        draft.validation = super::WorkflowValidation::valid();
                        draft.source = Some(prepared.source.clone());
                        draft.validator_version = Some(prepared.validator_version.clone());
                        draft.name = prepared.blueprint.metadata.name.clone();
                        draft.description = prepared.blueprint.metadata.description.clone();
                        self.unit
                            .set_resource(super::catalog::DRAFT_REGISTRY, &drafts, None)?;
                    }
                }
            }
            json!({"schema":"agent-runtime-workflow-revision/v1","id":request.id,"revision":revision,"kind":current.summary.kind,
                "trusted":current.summary.trusted,"production_executable":current.summary.production_executable,
                "validator_version":prepared.validator_version,
                "name":prepared.blueprint.metadata.name,"description":prepared.blueprint.metadata.description,
                "script":prepared.script,"blueprint":prepared.blueprint,"source":prepared.source,
                "validation":{"valid":true,"validator_version":prepared.validator_version},"committed":commit,
                "reference_script":if current.summary.kind==super::WorkflowResourceKind::Registered {Some(super::reference::reference_script(&prepared.blueprint)?)} else {None}})
        };
        if commit {
            self.publish_event(
                super::WORKFLOW_RESOURCE_CHANGED_EVENT,
                json!({
                    "schema":"agent-runtime-workflow-change/v1","workflow_id":result["id"],
                    "operation":"updated","kind":result["kind"],"revision":result["revision"],
                    "previous_revision":request.expected_revision
                }),
            )
            .await;
        }
        Ok(result)
    }
}

fn invalid(message: impl Into<String>) -> FrameworkError {
    FrameworkError::InvalidOperation(message.into())
}

pub(crate) fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
            }
            let from = temporary
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let to = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Semantic graph representation: editor IDs, layout and source annotations are excluded.
/// Ambiguous identical nodes are rejected rather than guessed during conversion.
fn semantic_graph(bp: &BlueprintJson) -> Result<Value> {
    bp.validate_graph().map_err(invalid)?;
    let mut exec_outputs = HashSet::new();
    let mut data_inputs = HashSet::new();
    for edge in &bp.connections {
        let unique = if edge.connection_type == "Exec" {
            exec_outputs.insert((&edge.source_node, &edge.source_pin))
        } else {
            data_inputs.insert((&edge.target_node, &edge.target_pin))
        };
        if !unique {
            return Err(invalid(
                "Multiple connections on one script-owned pin cannot be represented losslessly",
            ));
        }
    }
    let mut labels = HashMap::new();
    for node in &bp.nodes {
        let mut pins = node
            .pins
            .iter()
            .map(|p| {
                json!({"name":p.name,"kind":p.kind,
            "type":crate::data_type::public_type_name(&p.data_type),"default":p.default_value})
            })
            .collect::<Vec<_>>();
        pins.sort_by_key(Value::to_string);
        let properties = node
            .properties
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "source_script" | "layout" | "studio"))
            .collect::<BTreeMap<_, _>>();
        labels.insert(
            node.id.clone(),
            json!({"type":node.node_type,"pins":pins,"properties":properties}).to_string(),
        );
    }
    let mut names = HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    let starts = bp
        .nodes
        .iter()
        .filter(|n| n.node_type == "StartNode")
        .collect::<Vec<_>>();
    if starts.len() != 1 {
        return Err(invalid("Workflow must have exactly one StartNode"));
    }
    names.insert(starts[0].id.clone(), 0usize);
    queue.push_back(starts[0].id.clone());
    while let Some(id) = queue.pop_front() {
        let mut adjacent = Vec::new();
        for edge in &bp.connections {
            if edge.source_node == id {
                adjacent.push((
                    format!(
                        "out:{}:{}:{}",
                        edge.connection_type, edge.source_pin, edge.target_pin
                    ),
                    edge.target_node.clone(),
                ));
            }
            if edge.target_node == id {
                adjacent.push((
                    format!(
                        "in:{}:{}:{}",
                        edge.connection_type, edge.target_pin, edge.source_pin
                    ),
                    edge.source_node.clone(),
                ));
            }
        }
        adjacent.sort_by(|a, b| a.0.cmp(&b.0).then(labels[&a.1].cmp(&labels[&b.1])));
        for (_, next) in adjacent {
            if !names.contains_key(&next) {
                names.insert(next.clone(), names.len());
                queue.push_back(next);
            }
        }
    }
    if names.len() != bp.nodes.len() {
        return Err(invalid(
            "Disconnected nodes cannot be preserved by script conversion",
        ));
    }
    let mut nodes = labels
        .into_iter()
        .map(|(id, label)| (names[&id], label))
        .collect::<Vec<_>>();
    nodes.sort();
    let mut edges = bp
        .connections
        .iter()
        .map(|e| {
            (
                names[&e.source_node],
                e.source_pin.clone(),
                names[&e.target_node],
                e.target_pin.clone(),
                e.connection_type.clone(),
            )
        })
        .collect::<Vec<_>>();
    edges.sort();
    let mut variables = bp
        .variables
        .iter()
        .map(|v| json!({"name":v.name,"type":v.data_type,"default":v.default_value}))
        .collect::<Vec<_>>();
    variables.sort_by_key(Value::to_string);
    Ok(json!({"nodes":nodes,"edges":edges,"variables":variables}))
}

pub fn prepare(
    change: WorkflowChange,
    previous: Option<&PreparedRevision>,
    tools: &[RuntimeToolMetadata],
) -> Result<PreparedRevision> {
    let compile = |script: &str| {
        compile_chain_v2_with_runtime_tools(script, tools)
            .map_err(|e| invalid(format!("Script line {}: {}", e.line, e.message)))
    };
    let authored_change = matches!(&change, WorkflowChange::Script(_));
    let (script, mut blueprint, source) = match change {
        WorkflowChange::Script(script) => {
            let blueprint = compile(&script)?;
            (script, blueprint, "script".to_string())
        }
        WorkflowChange::Blueprint(blueprint) => {
            let original = semantic_graph(&blueprint)?;
            let (script, source) = if let Some(old) = previous
                .filter(|old| semantic_graph(&old.blueprint).ok().as_ref() == Some(&original))
            {
                (old.script.clone(), old.source.clone())
            } else {
                (
                    decompile_chain(&blueprint).map_err(|e| invalid(e.to_string()))?,
                    "blueprint".to_string(),
                )
            };
            if semantic_graph(&compile(&script)?)? != original {
                return Err(invalid("Graph to script conversion changes workflow semantics; revision was not committed"));
            }
            (script, blueprint, source)
        }
    };
    blueprint.normalize_node_sizes();
    blueprint.validate().map_err(invalid)?;
    semantic_graph(&blueprint)?;
    if authored_change {
        if let Some(old) = previous {
            super::preserve_workflow_blueprint_layout(&old.blueprint, &mut blueprint);
        }
    }
    Ok(PreparedRevision {
        script,
        blueprint,
        source,
        validator_version: "1".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "# authored comment\ninput value:String\nreturn result=input.value\n";

    #[test]
    fn preserves_authored_script_and_renumbered_graph() {
        let first = prepare(WorkflowChange::Script(SCRIPT.into()), None, &[]).unwrap();
        assert_eq!(first.script, SCRIPT);
        let mut graph = first.blueprint.clone();
        for node in &mut graph.nodes {
            node.id = format!("renumbered-{}", node.id);
        }
        for edge in &mut graph.connections {
            edge.source_node = format!("renumbered-{}", edge.source_node);
            edge.target_node = format!("renumbered-{}", edge.target_node);
        }
        let next = prepare(WorkflowChange::Blueprint(graph), Some(&first), &[]).unwrap();
        assert_eq!(next.script, SCRIPT);
    }

    #[test]
    fn rejects_duplicate_edges_instead_of_silently_dropping_them() {
        let mut graph = prepare(WorkflowChange::Script(SCRIPT.into()), None, &[])
            .unwrap()
            .blueprint;
        graph.connections.push(graph.connections[0].clone());
        assert!(decompile_chain(&graph).is_err());
        assert!(prepare(WorkflowChange::Blueprint(graph), None, &[]).is_err());
    }

    #[test]
    fn rejects_missing_revision_and_multiple_sources() {
        assert!(serde_json::from_value::<RevisionRequest>(
            json!({"id":"a","change":{"type":"script","value":SCRIPT}})
        )
        .is_err());
        assert!(serde_json::from_value::<RevisionRequest>(json!({"id":"a","expected_revision":1,"change":{"type":"script","value":SCRIPT,"blueprint":{}}})).is_err());
    }

    #[test]
    fn accepts_lossless_control_flow_round_trip() {
        let script = r#"input first:bool second:bool
$result = ""
1: IF input.first
    1.1.1: setvar result = "first"
1.2: ELIF input.second
    1.2.1: setvar result = "second"
1.0: ELSE
    1.0.1: setvar result = "fallback"
END
return result=$result
"#;
        let compiled = prepare(WorkflowChange::Script(script.into()), None, &[]).unwrap();
        let converted = prepare(WorkflowChange::Blueprint(compiled.blueprint), None, &[]).unwrap();
        assert!(converted.script.contains("ELIF"));
    }
}
