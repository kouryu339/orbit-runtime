//! Shared tool execution helpers.
//! This module converts legacy CLI-style tool commands such as
//! `SystemName --param value` into registered corework dynamic system calls.

use std::collections::HashMap;

use crate::decision::ToolResult;

use corework::ai_system::AISystemFactory;
use corework::orchestration::Context;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Local,
    Rpc,
    Unknown,
}

pub async fn execute_single(cmd: &str, ctx: &Context) -> ToolResult {
    execute_single_with_call_id(cmd, None, ctx).await
}

pub async fn execute_single_with_call_id(
    cmd: &str,
    tool_call_id: Option<&str>,
    ctx: &Context,
) -> ToolResult {
    let (system_name, args_str) = parse_tool_command(cmd);
    let tool_kind = tool_kind(system_name);

    let mut active_tools = match crate::AssistantContext::get_active_tools(&ctx.cache).await {
        Ok(tools) => tools,
        Err(error) => {
            return ToolResult {
                command: cmd.to_string(),
                success: false,
                to_ai: format!("Unable to verify active tools: {error}"),
                error_code: -5,
                result: serde_json::Value::Null,
            };
        }
    };
    if let Some(manager) = crate::skills::systems::SKILL_MANAGER.get() {
        manager
            .read()
            .await
            .inject_tools_for_state(crate::state::states::THINKING, &mut active_tools);
    }
    if !active_tools.iter().any(|tool| tool == system_name) {
        return ToolResult {
            command: cmd.to_string(),
            success: false,
            to_ai: format!(
                "Tool '{}' is not active for this agent. Activate the matching skill first.",
                system_name
            ),
            error_code: -5,
            result: serde_json::Value::Null,
        };
    }

    let executor = match ctx.get_dynamic_system(system_name) {
        Ok(e) => e,
        Err(_) => {
            return ToolResult {
                command: cmd.to_string(),
                success: false,
                to_ai: format!(
                    "Tool '{}' ({}) is not available. Activate the matching skill or check runtime RPC registration.",
                    system_name,
                    tool_kind.as_str()
                ),
                error_code: -1,
                result: serde_json::Value::Null,
            };
        }
    };
    if let Some(err) = validate_param_names(system_name, args_str) {
        tracing::warn!(
            tool_name = %system_name,
            error_len = err.len(),
            "tool parameter validation failed"
        );
        return ToolResult {
            command: cmd.to_string(),
            success: false,
            to_ai: err,
            error_code: -3,
            result: serde_json::Value::Null,
        };
    }

    let mut input_map = match build_tool_input_map(args_str) {
        Ok(input_map) => input_map,
        Err(e) => {
            return ToolResult {
                command: cmd.to_string(),
                success: false,
                to_ai: format!("Tool argument parsing failed: {}", e),
                error_code: -4,
                result: serde_json::Value::Null,
            };
        }
    };
    if tool_kind == ToolKind::Rpc {
        if let Some(tool_call_id) = tool_call_id {
            input_map.insert(
                "__tool_call_id".to_string(),
                serde_json::Value::String(tool_call_id.to_string()),
            );
        }
    }
    match executor.execute_dynamic(input_map, ctx).await {
        Ok(output) => {
            let error_code = output
                .get("error_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let to_ai_raw = output
                .get("to_ai")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let result = output
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            if to_ai_raw.trim().is_empty() {
                return ToolResult {
                    command: cmd.to_string(),
                    success: false,
                    to_ai: format!(
                        "System {} did not return a non-empty to_ai field",
                        system_name
                    ),
                    error_code: -2,
                    result,
                };
            }

            let tool_result = ToolResult {
                command: cmd.to_string(),
                success: error_code == 0,
                to_ai: to_ai_raw,
                error_code,
                result,
            };

            tool_result
        }
        Err(e) => {
            tracing::error!(
                tool_name = %system_name,
                tool_kind = %tool_kind.as_str(),
                error = %e,
                "tool execution failed"
            );
            ToolResult {
                command: cmd.to_string(),
                success: false,
                to_ai: format!(
                    "{} tool '{}' execution failed: {}",
                    tool_kind.as_label(),
                    system_name,
                    e
                ),
                error_code: -1,
                result: serde_json::Value::Null,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolPermissionMetadata {
    pub display_name: String,
    pub effect: crate::permission::ToolEffect,
    pub secret: bool,
}

pub fn permission_metadata(system_name: &str) -> Result<ToolPermissionMetadata, String> {
    let (display_name, readonly, destructive, secret) = if let Some(factory) =
        inventory::iter::<AISystemFactory>().find(|factory| factory.metadata.name == system_name)
    {
        (
            factory.metadata.display_name.to_string(),
            factory.metadata.readonly,
            factory.metadata.destructive,
            factory.metadata.secret,
        )
    } else if let Some(metadata) = crate::runtime_tools::get_runtime_tool(system_name) {
        (
            metadata.display_name_or_name().to_string(),
            metadata.readonly,
            metadata.destructive,
            metadata.secret,
        )
    } else {
        return Err(format!("Tool '{}' metadata is unavailable", system_name));
    };
    let effect = match (readonly, destructive) {
        (true, false) => crate::permission::ToolEffect::ReadOnly,
        (false, false) => crate::permission::ToolEffect::ControlledChange,
        (false, true) => crate::permission::ToolEffect::Destructive,
        (true, true) => {
            return Err(format!(
                "Tool '{}' has invalid metadata: readonly and destructive cannot both be true",
                system_name
            ))
        }
    };
    Ok(ToolPermissionMetadata {
        display_name,
        effect,
        secret,
    })
}

pub fn permission_arguments(cmd: &str) -> serde_json::Value {
    let (_, args_str) = parse_tool_command(cmd);
    let Ok(args) = build_tool_input_map(args_str) else {
        return serde_json::Value::Object(serde_json::Map::new());
    };
    serde_json::Value::Object(
        args.into_iter()
            .filter(|(name, _)| name != "input")
            .collect(),
    )
}

impl ToolKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Rpc => "rpc",
            Self::Unknown => "unknown",
        }
    }

    fn as_label(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Rpc => "RPC",
            Self::Unknown => "Runtime",
        }
    }
}

fn tool_kind(system_name: &str) -> ToolKind {
    if let Some(meta) = crate::runtime_tools::get_runtime_tool(system_name) {
        return match meta.tool_kind.as_str() {
            "rpc" => ToolKind::Rpc,
            "local" => ToolKind::Local,
            _ => ToolKind::Unknown,
        };
    }
    inventory::iter::<AISystemFactory>()
        .find(|f| f.metadata.name == system_name)
        .map(|f| match f.metadata.tool_kind {
            "local" => ToolKind::Local,
            "rpc" => ToolKind::Rpc,
            _ => ToolKind::Unknown,
        })
        .unwrap_or(ToolKind::Unknown)
}

pub fn build_exec_ctx(
    cache: std::sync::Arc<dyn corework::cache::Cache>,
    event_bus: std::sync::Arc<dyn corework::event::EventBus>,
    registry: std::sync::Arc<corework::system::SystemRegistry>,
) -> Context {
    use corework::monitoring::NoopTelemetry;
    use std::sync::Arc;
    Context::new(cache, event_bus, Arc::new(NoopTelemetry)).set_registry(registry)
}

pub fn parse_tool_command(cmd: &str) -> (&str, &str) {
    let trimmed = cmd.trim();
    match trimmed.find(char::is_whitespace) {
        Some(pos) => (&trimmed[..pos], trimmed[pos..].trim_start()),
        None => (trimmed, ""),
    }
}

fn build_tool_input_map(
    args_str: &str,
) -> corework::error::Result<HashMap<String, serde_json::Value>> {
    let mut input_map = HashMap::new();
    input_map.insert(
        "input".to_string(),
        serde_json::Value::String(args_str.to_string()),
    );

    if args_str.trim().is_empty() {
        return Ok(input_map);
    }

    let args = corework::ai_system::SimpleArgs::parse(args_str)?;
    for name in extract_param_names(args_str) {
        if let Some(value) = args.get(&name) {
            input_map.insert(name, serde_json::Value::String(value.to_string()));
        }
    }

    Ok(input_map)
}

pub fn available_system_names() -> Vec<&'static str> {
    inventory::iter::<AISystemFactory>
        .into_iter()
        .map(|f| f.metadata.name)
        .collect()
}

fn extract_param_names(args_str: &str) -> Vec<String> {
    corework::ai_system::SimpleArgs::parse(args_str)
        .map(|args| args.names().map(|name| name.to_string()).collect())
        .unwrap_or_default()
}

fn is_valid_param_name(name: &str, valid_names: &[String]) -> bool {
    valid_names.iter().any(|valid| valid == name)
        || (valid_names.iter().any(|valid| valid == "inputs") && name.starts_with("input."))
}

fn validate_param_names(system_name: &str, args_str: &str) -> Option<String> {
    if args_str.is_empty() {
        return None;
    }

    let valid_names: Vec<String> = if let Some(factory) =
        inventory::iter::<AISystemFactory>().find(|f| f.metadata.name == system_name)
    {
        factory
            .metadata
            .parameters
            .iter()
            .map(|p| p.name.to_string())
            .collect()
    } else if let Some(meta) = crate::runtime_tools::get_runtime_tool(system_name) {
        meta.parameters.iter().map(|p| p.name.clone()).collect()
    } else {
        return None;
    };

    let used = extract_param_names(args_str);
    if valid_names.is_empty() {
        if used.is_empty() {
            return None;
        }
        return Some(format!(
            "Invalid parameters for tool {}: {}. This tool accepts no parameters.",
            system_name,
            used.iter()
                .map(|n| format!("--{}", n))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    let invalid: Vec<&String> = used
        .iter()
        .filter(|n| !is_valid_param_name(n, &valid_names))
        .collect();
    if invalid.is_empty() {
        return None;
    }

    Some(format!(
        "Invalid parameters for tool {}: {}. Valid parameters:\n{}",
        system_name,
        invalid
            .iter()
            .map(|n| format!("--{}", n))
            .collect::<Vec<_>>()
            .join(", "),
        valid_names
            .iter()
            .map(|name| format!("  --{}", name))
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use corework::cache::{Cache, InMemoryCache};
    use corework::event::InMemoryEventBus;
    use corework::rpc_tool::RuntimeToolMetadata;
    use corework::system::SystemRegistry;
    use corework::workflow::dynamic_node::DynamicExecute;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    struct ForbiddenProbe;

    #[async_trait]
    impl DynamicExecute for ForbiddenProbe {
        async fn execute_dynamic(
            &self,
            _input: HashMap<String, Value>,
            _ctx: &corework::orchestration::Context,
        ) -> corework::error::Result<Value> {
            Ok(json!({
                "result": true,
                "to_ai": "executed",
                "error_code": 0
            }))
        }
    }

    #[tokio::test]
    async fn rejects_registered_tool_when_it_is_not_active_for_agent() {
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        crate::AssistantContext::set_active_tools(&cache, Vec::new())
            .await
            .unwrap();
        let registry = Arc::new(SystemRegistry::new());
        registry.register_dynamic("ForbiddenProbe", Arc::new(ForbiddenProbe));
        let event_bus = Arc::new(InMemoryEventBus::new());
        let ctx = super::build_exec_ctx(cache, event_bus, registry);

        let result = super::execute_single("ForbiddenProbe", &ctx).await;

        assert!(!result.success);
        assert_eq!(result.error_code, -5);
        assert!(result.to_ai.contains("not active"));
    }

    #[test]
    fn build_tool_input_map_includes_raw_and_structured_args() {
        let input = super::build_tool_input_map("--skill_name workflow --flag true").unwrap();

        assert_eq!(
            input.get("input"),
            Some(&serde_json::Value::String(
                "--skill_name workflow --flag true".to_string()
            ))
        );
        assert_eq!(
            input.get("skill_name"),
            Some(&serde_json::Value::String("workflow".to_string()))
        );
        assert_eq!(
            input.get("flag"),
            Some(&serde_json::Value::String("true".to_string()))
        );
    }

    #[test]
    fn build_tool_input_map_keeps_inner_exec_flags_inside_script_value() {
        let input = super::build_tool_input_map(
            r#"--script "input text:String\n1: EXEC CallLlm --user_message input.text\nreturn reply=1.response_text" --note "llm draft""#,
        )
        .unwrap();

        assert_eq!(
            input.get("note"),
            Some(&serde_json::Value::String("llm draft".to_string()))
        );
        let script = input
            .get("script")
            .and_then(|value| value.as_str())
            .unwrap();
        assert!(script.contains("EXEC CallLlm --user_message input.text"));
        assert!(!input.contains_key("user_message"));
    }

    #[test]
    fn validate_param_names_rejects_args_for_zero_param_runtime_tool() {
        crate::runtime_tools::register_runtime_tool(RuntimeToolMetadata {
            name: "ZeroParamProbeForValidation".to_string(),
            display_name: "Zero Param Probe For Validation".to_string(),
            description: String::new(),
            tool_kind: "rpc".to_string(),
            parameters: vec![],
            outputs: vec![],
            destructive: false,
            readonly: true,
            idempotent: true,
            open_world: false,
            secret: false,
            required_capabilities: vec![],
            endpoint_id: "test".to_string(),
            service: "test".to_string(),
            method: "execute".to_string(),
        });

        let err = super::validate_param_names(
            "ZeroParamProbeForValidation",
            "--conversation_id conv-test",
        )
        .expect("expected validation error");

        assert!(err.contains("accepts no parameters"));
        assert!(err.contains("--conversation_id"));
    }

    #[test]
    fn input_namespace_is_valid_when_inputs_marker_exists() {
        let valid_names = vec![
            "path".to_string(),
            "trace".to_string(),
            "inputs".to_string(),
        ];

        assert!(super::is_valid_param_name("path", &valid_names));
        assert!(super::is_valid_param_name("input.name", &valid_names));
        assert!(super::is_valid_param_name("input.count", &valid_names));
        assert!(!super::is_valid_param_name("name", &valid_names));
    }

    #[test]
    fn input_namespace_is_rejected_without_inputs_marker() {
        let valid_names = vec!["path".to_string(), "trace".to_string()];

        assert!(!super::is_valid_param_name("input.name", &valid_names));
    }
}
