//! Native function-calling schema and argument projection.

use corework::ai_system::{AIParameter, AISystemFactory};
use corework::rpc_tool::RuntimeAIParameter;
use corework::workflow::registry::NodeRegistry;
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

fn is_plan_steps(tool_name: &str, parameter: &ParameterSpec) -> bool {
    matches!(tool_name, "PlanWrite" | "PlanUpdate") && parameter.name == "steps"
}

fn parameter_schema(tool_name: &str, parameter: &ParameterSpec, strict: bool) -> Value {
    let mut schema = if parameter.name == "inputs" {
        input_pairs_schema()
    } else if is_plan_steps(tool_name, parameter) {
        json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "text": {"type": "string"},
                    "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "blocked", "canceled"]}
                },
                "required": ["id", "text", "status"],
                "additionalProperties": false
            }
        })
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
        properties.insert(
            parameter.name.clone(),
            parameter_schema(name, parameter, strict),
        );
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
    workflow_script_enabled: bool,
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
            if !factory.metadata.agent_enabled {
                continue;
            }
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
            let description = resident_tool_description(
                factory.metadata.description,
                !factory.metadata.workflow_enabled,
                workflow_script_enabled,
            );
            definitions.push(definition(
                factory.metadata.name,
                &description,
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
            if !metadata.agent_enabled {
                continue;
            }
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
            let description = resident_tool_description(
                &metadata.description,
                !metadata.workflow_enabled,
                workflow_script_enabled,
            );
            definitions.push(definition(
                &metadata.name,
                &description,
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

fn resident_tool_description(
    description: &str,
    ai_only: bool,
    workflow_script_enabled: bool,
) -> String {
    let mut sections = vec![if ai_only {
        description.trim().to_string()
    } else {
        compact_summary(description)
    }];
    if ai_only {
        if workflow_script_enabled {
            sections.push(
                crate::prompt_assets::template("tool_ai_only_description.md")
                    .trim()
                    .to_string(),
            );
        }
    }
    sections
        .into_iter()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn compact_summary(description: &str) -> String {
    let trimmed = description.trim();
    let first_line = trimmed
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let sentence_end = first_line
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '。' | '！' | '？').then_some(index + ch.len_utf8()))
        .or_else(|| first_line.find(". ").map(|index| index + 1));
    let summary = sentence_end
        .map(|end| &first_line[..end])
        .unwrap_or(first_line)
        .trim();
    const MAX_CHARS: usize = 240;
    if summary.chars().count() <= MAX_CHARS {
        summary.to_string()
    } else {
        let mut value = summary.chars().take(MAX_CHARS - 1).collect::<String>();
        value.push('…');
        value
    }
}

pub fn describe_registered_tools(names: &[String]) -> Result<Vec<Value>, String> {
    let factories = inventory::iter::<AISystemFactory>
        .into_iter()
        .collect::<Vec<_>>();
    names
        .iter()
        .map(|name| {
            if let Some(factory) = factories
                .iter()
                .find(|factory| factory.metadata.name == name)
            {
                let metadata = &factory.metadata;
                let mut descriptor = json!({
                    "name": metadata.name,
                    "display_name": metadata.display_name,
                    "description": metadata.description,
                    "tool_kind": metadata.tool_kind,
                    "ai_only": !metadata.workflow_enabled,
                    "availability": {
                        "agent": metadata.agent_enabled,
                        "workflow": metadata.workflow_enabled,
                        "jev": metadata.jev_enabled
                    },
                    "parameters": metadata.parameters.iter().map(|parameter| json!({
                        "name": parameter.name,
                        "type": parameter.param_type,
                        "required": parameter.required,
                        "default": parameter.default_value,
                        "description": parameter.description
                    })).collect::<Vec<_>>(),
                    "outputs": metadata.outputs.iter().map(|output| json!({
                        "name": output.name,
                        "type": output.field_type,
                        "description": output.description
                    })).collect::<Vec<_>>(),
                    "behavior": {
                        "readonly": metadata.readonly,
                        "destructive": metadata.destructive,
                        "idempotent": metadata.idempotent,
                        "open_world": metadata.open_world,
                        "secret": metadata.secret
                    }
                });
                add_workflow_descriptor(&mut descriptor, metadata.name, metadata.workflow_enabled);
                add_descriptor_identity(&mut descriptor);
                return Ok(descriptor);
            }
            if let Some(metadata) = crate::runtime_tools::get_runtime_tool(name) {
                let mut descriptor = json!({
                    "name": metadata.name,
                    "display_name": metadata.display_name,
                    "description": metadata.description,
                    "tool_kind": metadata.tool_kind,
                    "ai_only": !metadata.workflow_enabled,
                    "availability": {
                        "agent": metadata.agent_enabled,
                        "workflow": metadata.workflow_enabled,
                        "jev": metadata.jev_enabled
                    },
                    "parameters": metadata.parameters.iter().map(|parameter| json!({
                        "name": parameter.name,
                        "type": parameter.param_type,
                        "required": parameter.required,
                        "default": parameter.default_value,
                        "description": parameter.description
                    })).collect::<Vec<_>>(),
                    "outputs": metadata.outputs.iter().map(|output| json!({
                        "name": output.name,
                        "type": output.field_type,
                        "description": output.description
                    })).collect::<Vec<_>>(),
                    "behavior": {
                        "readonly": metadata.readonly,
                        "destructive": metadata.destructive,
                        "idempotent": metadata.idempotent,
                        "open_world": metadata.open_world,
                        "secret": metadata.secret
                    }
                });
                add_workflow_descriptor(&mut descriptor, &metadata.name, metadata.workflow_enabled);
                add_descriptor_identity(&mut descriptor);
                return Ok(descriptor);
            }
            Err(format!("tool '{name}' has no registered metadata"))
        })
        .collect()
}

fn add_workflow_descriptor(descriptor: &mut Value, name: &str, enabled: bool) {
    let node = NodeRegistry::get(name);
    descriptor["workflow"] = if let Some(node) = node {
        json!({
            "enabled": enabled,
            "node_type": node.node_type,
            "version": node.version,
            "category": node.category,
            "display_name": node.display_name,
            "description": node.description,
            "pins": node.pins.iter().map(|pin| json!({
                "name": pin.name,
                "kind": format!("{:?}", pin.kind),
                "type": pin.data_type,
                "description": pin.description,
                "default": pin.default_value
            })).collect::<Vec<_>>(),
            "wildcard_constraints": node.wildcard_constraints.iter().map(|(pin, allowed)| json!({
                "pin": pin,
                "allowed": allowed
            })).collect::<Vec<_>>()
        })
    } else {
        json!({"enabled": enabled, "node_type": Value::Null, "version": "1"})
    };
}

fn add_descriptor_identity(descriptor: &mut Value) {
    let revision = descriptor
        .pointer("/workflow/version")
        .and_then(Value::as_str)
        .unwrap_or("1")
        .to_string();
    descriptor["schema_revision"] = Value::String(revision);
    let canonical = serde_json::to_string(descriptor).unwrap_or_default();
    let hash = canonical
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    descriptor["descriptor_hash"] = Value::String(format!("{hash:016x}"));
}

fn validate_strict_parameter_compatibility(
    tool_name: &str,
    parameters: &[ParameterSpec],
) -> Result<(), String> {
    for parameter in parameters {
        if parameter.name == "inputs" || is_plan_steps(tool_name, parameter) {
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
    fn fc_marks_system_only_local_tool_only_when_workflow_scripts_are_enabled() {
        let names = vec!["GetSkillsList".to_string()];
        let regular = definitions_for_active_tools(&names, false, false).unwrap();
        let script_capable = definitions_for_active_tools(&names, false, true).unwrap();

        assert_eq!(
            regular[0].function.description,
            "获取所有可用的 Skills 列表。返回每个 Skill 的名称和描述，供 AI 判断需要激活哪些技能。"
        );
        assert_ne!(
            regular[0].function.description,
            script_capable[0].function.description
        );
    }

    #[test]
    fn fc_keeps_output_contract_out_of_resident_tool_definitions() {
        let names = vec!["executeWorkflowScript".to_string()];
        let regular = definitions_for_active_tools(&names, false, false).unwrap();
        let script_capable = definitions_for_active_tools(&names, false, true).unwrap();

        assert!(!regular[0]
            .function
            .description
            .contains("`outputs` (Object)"));
        assert!(!script_capable[0]
            .function
            .description
            .contains("`outputs` (Object)"));
        assert!(!script_capable[0]
            .function
            .description
            .contains("`trace` (Array)"));
    }

    #[test]
    fn general_tools_compact_only_the_usage_description() {
        let summary = resident_tool_description(
            "筛选 Excel 物理行号。这里是只应通过 ToolDescribe 按需读取的长篇适用说明。",
            false,
            false,
        );
        assert_eq!(summary, "筛选 Excel 物理行号。");

        let definition = definition(
            "ExcelQuerySessionRows",
            &summary,
            vec![ParameterSpec {
                name: "query".into(),
                param_type: "String".into(),
                required: true,
                default_value: None,
                description: "必填。SQL WHERE 表达式；不得包含 SELECT。".into(),
            }],
            false,
        );
        assert_eq!(
            definition.function.parameters["properties"]["query"]["description"],
            "必填。SQL WHERE 表达式；不得包含 SELECT。"
        );
        assert_eq!(definition.function.parameters["required"], json!(["query"]));
    }

    #[test]
    fn tool_describe_keeps_a_typed_required_name_array() {
        let definitions =
            definitions_for_active_tools(&["ToolDescribe".to_string()], false, false).unwrap();
        let parameters = &definitions[0].function.parameters;

        assert_eq!(parameters["properties"]["tool_names"]["type"], "array");
        assert_eq!(
            parameters["properties"]["tool_names"]["items"]["type"],
            "string"
        );
        assert_eq!(parameters["required"], json!(["tool_names"]));
    }

    #[test]
    fn describe_returns_full_outputs_workflow_contract_and_identity() {
        let descriptors = describe_registered_tools(&["Wait".to_string()]).unwrap();
        let descriptor = &descriptors[0];

        assert_eq!(descriptor["name"], "Wait");
        assert!(descriptor["outputs"]
            .as_array()
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(descriptor["workflow"]["enabled"], true);
        assert!(descriptor["schema_revision"].as_str().is_some());
        assert_eq!(
            descriptor["descriptor_hash"].as_str().map(str::len),
            Some(16)
        );
    }

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
    fn direct_fc_array_arguments_keep_json_for_rpc_type_restoration() {
        let definition = definition(
            "ExcelQuerySessionColumn",
            "query",
            vec![ParameterSpec {
                name: "parameters".into(),
                param_type: "Array<Any>".into(),
                required: true,
                default_value: None,
                description: String::new(),
            }],
            false,
        );
        let call = llm_gateway::ToolCall::function(
            "call-1",
            "ExcelQuerySessionColumn",
            r#"{"parameters":[2,"ready",null]}"#,
        );

        let parsed = validate_and_project_call(&call, &[definition]).unwrap();

        assert_eq!(
            parsed.params,
            vec![("parameters".into(), "[2,\"ready\",null]".into())]
        );
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

    #[test]
    fn plan_steps_have_closed_schema_and_preserve_structured_arguments() {
        let tools =
            definitions_for_active_tools(&["PlanWrite".into(), "PlanUpdate".into()], true, false)
                .unwrap();
        for tool in &tools {
            let items = &tool.function.parameters["properties"]["steps"]["items"];
            assert_eq!(items["additionalProperties"], false);
            assert_eq!(items["required"], json!(["id", "text", "status"]));
        }
        let steps = json!([{"id":"one", "text":"Inspect", "status":"in_progress"}]);
        let call = llm_gateway::ToolCall::function(
            "plan-call",
            "PlanWrite",
            json!({
                "title":"Plan", "summary":null, "content":null, "steps":steps
            })
            .to_string(),
        );
        let parsed = validate_and_project_call(&call, &tools).unwrap();
        let value = &parsed
            .params
            .iter()
            .find(|(key, _)| key == "steps")
            .unwrap()
            .1;
        assert_eq!(serde_json::from_str::<Value>(value).unwrap(), steps);
        assert_eq!(
            tools[1].function.parameters["properties"]["revision"]["type"],
            json!(["integer", "null"])
        );
    }
}
