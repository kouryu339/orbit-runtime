//! Public metadata projection shared by Native and browser entry points.
use crate::{
    rpc_tool::RuntimeToolMetadata,
    workflow::registry::{NodeRegistry, PinMetadata},
};
use serde_json::{json, Value};
pub fn static_tool_definition(metadata: &corework::ai_system::AISystemMetadata) -> Value {
    let workflow_node = NodeRegistry::get(metadata.name).map(|node| {
        json!({
            "version": node.version,
            "category": node.category,
            "pins": public_node_pins(node.pins),
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
            "param_type": corework::data_type::public_type_name(parameter.param_type),
            "required": parameter.required,
            "default_value": parameter.default_value,
            "description": parameter.description
        })).collect::<Vec<_>>(),
        "outputs": metadata.outputs.iter().map(|output| json!({
            "name": output.name,
            "field_type": corework::data_type::public_type_name(output.field_type),
            "description": output.description
        })).collect::<Vec<_>>(),
        "destructive": metadata.destructive,
        "readonly": metadata.readonly,
        "idempotent": metadata.idempotent,
        "open_world": metadata.open_world,
        "secret": metadata.secret,
        "required_capabilities": [],
        "workflow_enabled": metadata.workflow_enabled,
        "workflow_node_capable": metadata.workflow_enabled && workflow_node.is_some(),
        "workflow_node": workflow_node,
        "transport": Value::Null
    })
}

pub fn runtime_tool_definition(metadata: &RuntimeToolMetadata) -> Value {
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
        "parameters": metadata.parameters.iter().map(|parameter| json!({
            "name": parameter.name,
            "param_type": corework::data_type::public_type_name(&parameter.param_type),
            "required": parameter.required,
            "default_value": parameter.default_value,
            "description": parameter.description
        })).collect::<Vec<_>>(),
        "outputs": metadata.outputs.iter().map(|output| json!({
            "name": output.name,
            "field_type": corework::data_type::public_type_name(&output.field_type),
            "description": output.description
        })).collect::<Vec<_>>(),
        "destructive": metadata.destructive,
        "readonly": metadata.readonly,
        "idempotent": metadata.idempotent,
        "open_world": metadata.open_world,
        "secret": metadata.secret,
        "required_capabilities": metadata.required_capabilities,
        "workflow_enabled": metadata.workflow_enabled,
        "workflow_node_capable": metadata.workflow_enabled,
        "workflow_node": workflow_node,
        "transport": transport
    })
}

pub fn runtime_tool_pins(metadata: &RuntimeToolMetadata) -> Vec<Value> {
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
            "data_type": corework::data_type::public_type_name(&parameter.param_type),
            "description": parameter.description,
            "default_value": parameter.default_value
        })
    }));
    pins.extend(metadata.outputs.iter().map(|output| {
        json!({
            "name": output.name,
            "kind": "DataOutput",
            "data_type": corework::data_type::public_type_name(&output.field_type),
            "description": output.description,
            "default_value": null
        })
    }));
    pins
}

pub fn public_node_pins(pins: &[PinMetadata]) -> Vec<Value> {
    pins.iter()
        .map(|pin| {
            json!({
                "name": pin.name,
                "kind": pin.kind,
                "data_type": corework::data_type::public_type_name(pin.data_type),
                "description": pin.description,
                "default_value": pin.default_value
            })
        })
        .collect()
}
