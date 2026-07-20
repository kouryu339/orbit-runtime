use super::*;
use corework::system::SystemRegistry;
use corework::workflow::registry::NodeRegistry;

impl RuntimeFacade {
    pub fn tool_definitions(&self) -> Result<String, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }

        let mut tools = BTreeMap::<String, Value>::new();
        for metadata in SystemRegistry::list_ai_systems() {
            tools.insert(metadata.name.to_string(), static_tool_definition(metadata));
        }
        // Dynamic definitions are the effective implementation if a runtime
        // provider intentionally replaces a statically known tool name.
        for metadata in &self.runtime_tools {
            tools.insert(metadata.name.clone(), runtime_tool_definition(metadata));
        }

        serde_json::to_string(&json!({
            "schema": "agent-runtime-tool-definitions/v1",
            "catalog_scope": "runtime_registered",
            "authorization_scope": "definitions_only",
            "tools": tools.into_values().collect::<Vec<_>>()
        }))
        .map_err(|error| {
            RuntimeError::Internal(format!("serialize tool definitions failed: {error}"))
        })
    }
}

fn static_tool_definition(metadata: &corework::ai_system::AISystemMetadata) -> Value {
    let workflow_node = NodeRegistry::get(metadata.name).map(|node| {
        json!({
            "version": node.version,
            "category": node.category,
            "pins": node.pins,
            "permissions": node.permissions,
            "wildcard_constraints": node.wildcard_constraints
        })
    });
    json!({
        "name": metadata.name,
        "display_name": if metadata.display_name.trim().is_empty() { metadata.name } else { metadata.display_name },
        "description": metadata.description,
        "tool_kind": metadata.tool_kind,
        "parameters": metadata.parameters.iter().map(|parameter| json!({
            "name": parameter.name,
            "param_type": parameter.param_type,
            "required": parameter.required,
            "default_value": parameter.default_value,
            "description": parameter.description
        })).collect::<Vec<_>>(),
        "outputs": metadata.outputs.iter().map(|output| json!({
            "name": output.name,
            "field_type": output.field_type,
            "description": output.description
        })).collect::<Vec<_>>(),
        "destructive": metadata.destructive,
        "readonly": metadata.readonly,
        "idempotent": metadata.idempotent,
        "open_world": metadata.open_world,
        "secret": metadata.secret,
        "required_capabilities": [],
        "workflow_node_capable": workflow_node.is_some(),
        "workflow_node": workflow_node,
        "transport": Value::Null
    })
}

fn runtime_tool_definition(metadata: &RuntimeToolMetadata) -> Value {
    let workflow_node = json!({
        "version": "runtime",
        "category": if metadata.tool_kind == "rpc" { "RPC" } else { "Runtime" },
        "pins": runtime_tool_pins(metadata),
        "permissions": Value::Null,
        "wildcard_constraints": []
    });
    let transport = if metadata.tool_kind == "rpc" {
        json!({
            "endpoint_id": metadata.endpoint_id,
            "service": metadata.service,
            "method": metadata.method
        })
    } else {
        Value::Null
    };
    json!({
        "name": metadata.name,
        "display_name": metadata.display_name_or_name(),
        "description": metadata.description,
        "tool_kind": metadata.tool_kind,
        "parameters": metadata.parameters,
        "outputs": metadata.outputs,
        "destructive": metadata.destructive,
        "readonly": metadata.readonly,
        "idempotent": metadata.idempotent,
        "open_world": metadata.open_world,
        "secret": metadata.secret,
        "required_capabilities": metadata.required_capabilities,
        "workflow_node_capable": true,
        "workflow_node": workflow_node,
        "transport": transport
    })
}

pub(super) fn runtime_tool_pins(metadata: &RuntimeToolMetadata) -> Vec<Value> {
    let mut pins = vec![
        json!({
            "name": "Exec",
            "kind": "ExecInput",
            "data_type": "",
            "description": "Execution input",
            "default_value": null
        }),
        json!({
            "name": "Then",
            "kind": "ExecOutput",
            "data_type": "",
            "description": "Execution output",
            "default_value": null
        }),
    ];
    pins.extend(metadata.parameters.iter().map(|parameter| {
        json!({
            "name": parameter.name,
            "kind": "DataInput",
            "data_type": parameter.param_type,
            "description": parameter.description,
            "default_value": parameter.default_value
        })
    }));
    pins.extend(metadata.outputs.iter().map(|output| {
        json!({
            "name": output.name,
            "kind": "DataOutput",
            "data_type": output.field_type,
            "description": output.description,
            "default_value": null
        })
    }));
    pins
}
