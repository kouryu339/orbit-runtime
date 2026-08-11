//! Native function-calling schema and argument projection.

use corework::ai_system::{AIParameter, AISystemFactory};
use corework::rpc_tool::RuntimeAIParameter;
use serde_json::{json, Map, Value};

use crate::decision_line::ParsedToolCall;

#[derive(Clone)]
struct ParameterSpec {
    name: String,
    param_type: String,
    required: bool,
    default_value: Option<String>,
    description: String,
}

impl From<&AIParameter> for ParameterSpec {
    fn from(value: &AIParameter) -> Self {
        Self {
            name: value.name.to_string(),
            param_type: value.param_type.to_string(),
            required: value.required,
            default_value: value.default_value.map(str::to_string),
            description: value.description.to_string(),
        }
    }
}

impl From<&RuntimeAIParameter> for ParameterSpec {
    fn from(value: &RuntimeAIParameter) -> Self {
        Self {
            name: value.name.clone(),
            param_type: value.param_type.clone(),
            required: value.required,
            default_value: value.default_value.clone(),
            description: value.description.clone(),
        }
    }
}

fn base_schema(param_type: &str) -> Value {
    let normalized = param_type.trim().to_ascii_lowercase();
    if normalized.starts_with("array") || normalized.starts_with("vec") {
        let item_type = param_type
            .split_once('<')
            .and_then(|(_, rest)| rest.rsplit_once('>').map(|(inner, _)| inner.trim()));
        return json!({
            "type": "array",
            "items": item_type.map(base_schema).unwrap_or_else(|| json!({}))
        });
    }
    match normalized.as_str() {
        "bool" | "boolean" => json!({"type": "boolean"}),
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            json!({"type": "integer"})
        }
        "f32" | "f64" | "num" | "number" => json!({"type": "number"}),
        "object" | "map" | "json" | "any" => json!({"type": "object"}),
        _ => json!({"type": "string"}),
    }
}

fn input_pairs_schema() -> Value {
    json!({
        "type": "array",
        "description": "Workflow inputs. Each value is passed using its textual representation.",
        "items": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "value": {"type": "string"}
            },
            "required": ["name", "value"],
            "additionalProperties": false
        }
    })
}

fn parameter_schema(parameter: &ParameterSpec, strict: bool) -> Value {
    let mut schema = if parameter.name == "inputs" {
        input_pairs_schema()
    } else {
        base_schema(&parameter.param_type)
    };
    let object = schema.as_object_mut().expect("parameter schema object");
    if !parameter.description.trim().is_empty() {
        object.insert("description".into(), json!(parameter.description));
    }
    if !strict {
        if let Some(default) = parameter.default_value.as_deref() {
            let value = serde_json::from_str(default).unwrap_or_else(|_| json!(default));
            object.insert("default".into(), value);
        }
    }
    if strict && !parameter.required {
        if let Some(kind) = object.get("type").cloned() {
            object.insert("type".into(), json!([kind, "null"]));
        }
    }
    schema
}

fn definition(
    name: &str,
    description: &str,
    parameters: Vec<ParameterSpec>,
    strict: bool,
) -> llm_gateway::ToolDefinition {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for parameter in &parameters {
        properties.insert(parameter.name.clone(), parameter_schema(parameter, strict));
        if parameter.required || strict {
            required.push(Value::String(parameter.name.clone()));
        }
    }
    llm_gateway::ToolDefinition {
        tool_type: "function".to_string(),
        function: llm_gateway::FunctionDefinition {
            name: name.to_string(),
            description: description.to_string(),
            parameters: json!({
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false
            }),
            strict: strict.then_some(true),
        },
    }
}

pub fn definitions_for_active_tools(
    names: &[String],
    strict: bool,
) -> Result<Vec<llm_gateway::ToolDefinition>, String> {
    let factories = inventory::iter::<AISystemFactory>
        .into_iter()
        .collect::<Vec<_>>();
    let mut definitions = Vec::with_capacity(names.len());
    for name in names {
        if let Some(factory) = factories
            .iter()
            .find(|factory| factory.metadata.name == name)
        {
            if strict {
                validate_strict_parameter_compatibility(
                    factory.metadata.name,
                    &factory
                        .metadata
                        .parameters
                        .iter()
                        .map(ParameterSpec::from)
                        .collect::<Vec<_>>(),
                )?;
            }
            definitions.push(definition(
                factory.metadata.name,
                factory.metadata.description,
                factory
                    .metadata
                    .parameters
                    .iter()
                    .map(ParameterSpec::from)
                    .collect(),
                strict,
            ));
            continue;
        }
        if let Some(metadata) = crate::runtime_tools::get_runtime_tool(name) {
            if strict {
                validate_strict_parameter_compatibility(
                    &metadata.name,
                    &metadata
                        .parameters
                        .iter()
                        .map(ParameterSpec::from)
                        .collect::<Vec<_>>(),
                )?;
            }
            definitions.push(definition(
                &metadata.name,
                &metadata.description,
                metadata
                    .parameters
                    .iter()
                    .map(ParameterSpec::from)
                    .collect(),
                strict,
            ));
            continue;
        }
        return Err(format!("active tool '{name}' has no registered metadata"));
    }
    Ok(definitions)
}

fn validate_strict_parameter_compatibility(
    tool_name: &str,
    parameters: &[ParameterSpec],
) -> Result<(), String> {
    for parameter in parameters {
        if parameter.name == "inputs" {
            continue;
        }
        let normalized = parameter.param_type.trim().to_ascii_lowercase();
        let unconstrained_object = matches!(normalized.as_str(), "object" | "map" | "json" | "any");
        let unconstrained_array = (normalized.starts_with("array")
            || normalized.starts_with("vec"))
            && (!normalized.contains('<') || normalized.contains("<any>"));
        if unconstrained_object || unconstrained_array {
            return Err(format!(
                "tool '{tool_name}' parameter '{}' uses type '{}' without a closed schema; strict_tool_schema cannot be enabled for this active tool",
                parameter.name, parameter.param_type
            ));
        }
    }
    Ok(())
}

fn value_matches_type(value: &Value, expected: &Value) -> bool {
    if value.is_null() {
        return expected
            .as_array()
            .map(|types| types.iter().any(|kind| kind == "null"))
            .unwrap_or(false);
    }
    let matches = |kind: &str| match kind {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    };
    if let Some(kind) = expected.as_str() {
        matches(kind)
    } else if let Some(types) = expected.as_array() {
        types.iter().filter_map(Value::as_str).any(matches)
    } else {
        true
    }
}

pub fn validate_and_project_call(
    call: &llm_gateway::ToolCall,
    definitions: &[llm_gateway::ToolDefinition],
) -> Result<ParsedToolCall, String> {
    let definition = definitions
        .iter()
        .find(|definition| definition.function.name == call.function.name)
        .ok_or_else(|| format!("model requested inactive tool '{}'", call.function.name))?;
    let arguments: Value = serde_json::from_str(&call.function.arguments)
        .map_err(|error| format!("invalid arguments for '{}': {error}", call.function.name))?;
    let arguments = arguments
        .as_object()
        .ok_or_else(|| format!("arguments for '{}' must be an object", call.function.name))?;
    let properties = definition
        .function
        .parameters
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("tool '{}' has an invalid schema", call.function.name))?;
    for name in arguments.keys() {
        if !properties.contains_key(name) {
            return Err(format!(
                "tool '{}' received unknown argument '{}'",
                call.function.name, name
            ));
        }
    }
    for required in definition
        .function
        .parameters
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !arguments.contains_key(required) {
            return Err(format!(
                "tool '{}' is missing required argument '{}'",
                call.function.name, required
            ));
        }
    }
    let mut params = Vec::new();
    for (name, value) in arguments {
        if value.is_null() {
            continue;
        }
        let expected = properties
            .get(name)
            .and_then(|schema| schema.get("type"))
            .cloned()
            .unwrap_or(Value::Null);
        if !value_matches_type(value, &expected) {
            return Err(format!(
                "tool '{}' argument '{}' does not match its schema",
                call.function.name, name
            ));
        }
        if name == "inputs" {
            let pairs = value
                .as_array()
                .ok_or_else(|| format!("tool '{}' inputs must be an array", call.function.name))?;
            for pair in pairs {
                let pair = pair.as_object().ok_or_else(|| {
                    format!(
                        "tool '{}' input entry must be an object",
                        call.function.name
                    )
                })?;
                let input_name = pair.get("name").and_then(Value::as_str).ok_or_else(|| {
                    format!("tool '{}' input entry is missing name", call.function.name)
                })?;
                let input_value = pair.get("value").and_then(Value::as_str).ok_or_else(|| {
                    format!("tool '{}' input entry is missing value", call.function.name)
                })?;
                params.push((format!("input.{input_name}"), input_value.to_string()));
            }
            continue;
        }
        let rendered = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        params.push((name.clone(), rendered));
    }
    Ok(ParsedToolCall {
        name: call.function.name.clone(),
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_arguments_and_projects_workflow_inputs() {
        let definition = definition(
            "executeWorkflowScript",
            "execute",
            vec![
                ParameterSpec {
                    name: "script".into(),
                    param_type: "String".into(),
                    required: true,
                    default_value: None,
                    description: String::new(),
                },
                ParameterSpec {
                    name: "inputs".into(),
                    param_type: "String".into(),
                    required: false,
                    default_value: None,
                    description: String::new(),
                },
            ],
            false,
        );
        let call = llm_gateway::ToolCall::function(
            "call-1",
            "executeWorkflowScript",
            r#"{"script":"input\nreturn","inputs":[{"name":"count","value":"2"}]}"#,
        );
        let parsed = validate_and_project_call(&call, std::slice::from_ref(&definition)).unwrap();
        assert!(parsed.params.contains(&("input.count".into(), "2".into())));
        let bad = llm_gateway::ToolCall::function(
            "call-2",
            "executeWorkflowScript",
            r#"{"script":"x","unknown":true}"#,
        );
        assert!(validate_and_project_call(&bad, &[definition]).is_err());
    }

    #[test]
    fn strict_schema_closes_objects_and_makes_optional_fields_nullable() {
        let parameters = vec![
            ParameterSpec {
                name: "path".into(),
                param_type: "String".into(),
                required: true,
                default_value: None,
                description: String::new(),
            },
            ParameterSpec {
                name: "limit".into(),
                param_type: "u64".into(),
                required: false,
                default_value: Some("10".into()),
                description: String::new(),
            },
        ];
        validate_strict_parameter_compatibility("Read", &parameters).unwrap();
        let tool = definition("Read", "read", parameters, true);

        assert_eq!(
            tool.function.parameters["additionalProperties"],
            json!(false)
        );
        assert_eq!(
            tool.function.parameters["required"],
            json!(["path", "limit"])
        );
        assert_eq!(
            tool.function.parameters["properties"]["limit"]["type"],
            json!(["integer", "null"])
        );
        assert!(tool.function.parameters["properties"]["limit"]
            .get("default")
            .is_none());
        assert_eq!(tool.function.strict, Some(true));
    }

    #[test]
    fn strict_schema_rejects_unconstrained_metadata() {
        let parameters = vec![ParameterSpec {
            name: "payload".into(),
            param_type: "Any".into(),
            required: true,
            default_value: None,
            description: String::new(),
        }];

        let error = validate_strict_parameter_compatibility("Unsafe", &parameters).unwrap_err();
        assert!(error.contains("without a closed schema"));
    }
}
