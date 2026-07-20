use super::*;
use corework::system::SystemRegistry;
use corework::workflow::registry::{NodeRegistry, PinKind};

impl RuntimeFacade {
    pub fn workflow_node_definitions(&self) -> Result<String, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }

        serde_json::to_string(&json!({
            "schema": "agent-runtime-workflow-node-definitions/v1",
            "catalog_scope": "runtime_registered",
            "nodes": workflow_node_definition_values(&self.runtime_tools)
        }))
        .map_err(|error| {
            RuntimeError::Internal(format!(
                "serialize workflow node definitions failed: {error}"
            ))
        })
    }
}

pub(crate) fn workflow_node_definition_values(runtime_tools: &[RuntimeToolMetadata]) -> Vec<Value> {
    let static_tools = SystemRegistry::list_ai_systems()
        .into_iter()
        .map(|metadata| (metadata.name, metadata))
        .collect::<BTreeMap<_, _>>();
    let callable_local_tools = AssistantContext::all_registered_tool_names()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut nodes = BTreeMap::<String, Value>::new();

    for node in NodeRegistry::all() {
        let source = if static_tools.contains_key(node.node_type) {
            "local"
        } else {
            "corework"
        };
        let pure = !node
            .pins
            .iter()
            .any(|pin| matches!(pin.kind, PinKind::ExecInput | PinKind::ExecOutput));
        let definition = json!({
            "node_type": node.node_type,
            "name": node.node_type,
            "source": source,
            "version": node.version,
            "display_name": node.display_name,
            "category": canonical_category(source, node.category),
            "native_category": node.category,
            "pure": pure,
            "description": node.description,
            "pins": node.pins,
            "parameters": [],
            "outputs": [],
            "permissions": node.permissions,
            "wildcard_constraints": node.wildcard_constraints,
            "editor_callable": callable_local_tools.contains(node.node_type),
        });

        // Inventory order is not stable. Prefer the lexicographically first
        // equivalent registration so duplicate node types remain deterministic.
        match nodes.entry(node.node_type.to_string()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(definition);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let current_category = entry
                    .get()
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let next_category = definition["category"].as_str().unwrap_or_default();
                if next_category < current_category {
                    entry.insert(definition);
                }
            }
        }
    }

    // Runtime registrations are the effective implementation when they replace
    // a statically known node type.
    for tool in runtime_tools {
        let source = if tool.tool_kind == "rpc" {
            "rpc"
        } else {
            "local"
        };
        nodes.insert(
            tool.name.clone(),
            json!({
                "node_type": tool.name,
                "name": tool.name,
                "source": source,
                "version": "runtime",
                "display_name": tool.display_name_or_name(),
                "category": canonical_category(source, "Runtime"),
                "native_category": if source == "rpc" { "RPC" } else { "Runtime" },
                "pure": false,
                "description": tool.description,
                "pins": super::tool_definitions::runtime_tool_pins(tool),
                "parameters": tool.parameters,
                "outputs": tool.outputs,
                "permissions": null,
                "wildcard_constraints": [],
                "editor_callable": true,
            }),
        );
    }

    nodes.into_values().collect()
}

fn canonical_category(source: &str, category: &str) -> String {
    if source == "rpc" {
        return "runtime/rpc".to_string();
    }
    if source == "local" {
        return format!("runtime/local/{}", category_slug(category));
    }
    match category {
        "Control Flow" => "control".to_string(),
        "Array" => "data/array".to_string(),
        "Logic" => "data/logic".to_string(),
        "Math" => "data/math".to_string(),
        "String" => "data/string".to_string(),
        "Variable" => "data/variable".to_string(),
        "Placeholder" => "utility".to_string(),
        other => format!("corework/{}", category_slug(other)),
    }
}

fn category_slug(category: &str) -> String {
    category
        .trim()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
