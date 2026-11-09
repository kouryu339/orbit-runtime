//! Shared tool execution helpers.
//! This module converts legacy CLI-style tool commands such as
//! `SystemName --param value` into registered corework dynamic system calls.

use std::collections::HashMap;

use crate::decision::ToolResult;

use corework::ai_system::AISystemFactory;
use corework::cache::CacheExt;
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
    if args_str.trim_start().starts_with('{') {
        let call = corework::tool_arguments::parse_object(args_str).map(|args| {
            crate::decision_line::ParsedToolCall::from_json(system_name.to_string(), args)
        });
        match call {
            Ok(call) => return execute_parsed_with_call_id(&call, tool_call_id, ctx).await,
            Err(reason) => {
                return ToolResult {
                    command: cmd.into(),
                    success: false,
                    error_code: -4,
                    to_ai: reason,
                    result: serde_json::Value::Null,
                }
            }
        }
    }
    execute_prepared(
        system_name,
        cmd,
        tool_call_id,
        validate_param_name_list_using(
            system_name,
            &extract_param_names(args_str),
            Some(ctx.registry()),
        ),
        build_tool_input_map(args_str),
        ctx,
    )
    .await
}

/// Execute an already parsed EXEC call without serializing its parameters back
/// into CLI text and parsing them a second time.
pub async fn execute_parsed_with_call_id(
    call: &crate::decision_line::ParsedToolCall,
    tool_call_id: Option<&str>,
    ctx: &Context,
) -> ToolResult {
    let command = call.to_legacy_command();
    if call.arguments.is_some() {
        if let Err(reason) = validate_recovery_call(&command, Some(call), ctx.registry()) {
            return ToolResult {
                command,
                success: false,
                error_code: -3,
                to_ai: reason,
                result: serde_json::Value::Null,
            };
        }
    }
    let used_names = call
        .params
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let used_names = call
        .arguments
        .as_ref()
        .map(|a| a.keys().cloned().collect())
        .unwrap_or(used_names);
    execute_prepared(
        &call.name,
        &command,
        tool_call_id,
        validate_param_name_list_using(&call.name, &used_names, Some(ctx.registry())),
        Ok(build_parsed_tool_input_map(call)),
        ctx,
    )
    .await
}

async fn execute_prepared(
    system_name: &str,
    command: &str,
    tool_call_id: Option<&str>,
    validation_error: Option<String>,
    input_map: corework::error::Result<HashMap<String, serde_json::Value>>,
    ctx: &Context,
) -> ToolResult {
    let registry = Some(ctx.registry());
    let tool_kind = tool_kind(system_name, registry);
    let agent_enabled = inventory::iter::<AISystemFactory>()
        .find(|factory| factory.metadata.name == system_name)
        .map(|factory| factory.metadata.agent_enabled)
        .or_else(|| {
            crate::runtime_tools::get_runtime_tool_in_registry(registry, system_name)
                .map(|metadata| metadata.agent_enabled)
        })
        .unwrap_or(false);
    if !agent_enabled {
        return ToolResult {
            command: command.to_string(),
            success: false,
            to_ai: format!("Tool '{system_name}' is not callable by an Agent."),
            error_code: -5,
            result: serde_json::Value::Null,
        };
    }

    let mut active_tools = match crate::AssistantContext::get_active_tools(&ctx.cache).await {
        Ok(tools) => tools,
        Err(error) => {
            return ToolResult {
                command: command.to_string(),
                success: false,
                to_ai: format!("Unable to verify active tools: {error}"),
                error_code: -5,
                result: serde_json::Value::Null,
            };
        }
    };
    if let Some(manager) = crate::skills::systems::SKILL_MANAGER.get() {
        let system: std::collections::BTreeMap<String, String> = ctx
            .cache
            .get(crate::context::keys::SYSTEM_SKILLS)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let protocol = ctx
            .cache
            .get::<crate::ToolProtocol>(crate::context::keys::TOOL_PROTOCOL)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let configured = system
            .get("thinking")
            .map(String::as_str)
            .unwrap_or("thinking");
        let thinking = match (protocol, configured) {
            (crate::ToolProtocol::NativeFc, "thinking") => "thinking-fc",
            (crate::ToolProtocol::NativeFc, "thinking-pro") => "thinking-pro-fc",
            (_, name) => name,
        };
        active_tools = manager
            .read()
            .await
            .filtered_tools_for_state(thinking, active_tools);
    }
    if !active_tools.iter().any(|tool| tool == system_name) {
        return ToolResult {
            command: command.to_string(),
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
                command: command.to_string(),
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
    if let Some(err) = validation_error {
        tracing::warn!(
            tool_name = %system_name,
            error_len = err.len(),
            "tool parameter validation failed"
        );
        return ToolResult {
            command: command.to_string(),
            success: false,
            to_ai: err,
            error_code: -3,
            result: serde_json::Value::Null,
        };
    }

    let mut input_map = match input_map {
        Ok(input_map) => input_map,
        Err(e) => {
            return ToolResult {
                command: command.to_string(),
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
                    command: command.to_string(),
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
                command: command.to_string(),
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
                command: command.to_string(),
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
    permission_metadata_in_registry(system_name, None)
}
pub fn permission_metadata_in_registry(
    system_name: &str,
    registry: Option<&corework::system::SystemRegistry>,
) -> Result<ToolPermissionMetadata, String> {
    let (display_name, readonly, destructive, secret) = if let Some(factory) =
        inventory::iter::<AISystemFactory>().find(|factory| factory.metadata.name == system_name)
    {
        (
            factory.metadata.display_name.to_string(),
            factory.metadata.readonly,
            factory.metadata.destructive,
            factory.metadata.secret,
        )
    } else if let Some(metadata) =
        crate::runtime_tools::get_runtime_tool_in_registry(registry, system_name)
    {
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
    if args_str.trim_start().starts_with('{') {
        return corework::tool_arguments::parse_object(args_str)
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);
    }
    let Ok(args) = build_tool_input_map(args_str) else {
        return serde_json::Value::Object(serde_json::Map::new());
    };
    serde_json::Value::Object(
        args.into_iter()
            .filter(|(name, _)| name != "input")
            .collect(),
    )
}

pub fn permission_arguments_parsed(
    call: &crate::decision_line::ParsedToolCall,
) -> serde_json::Value {
    if let Some(args) = &call.arguments {
        return serde_json::Value::Object(args.clone());
    }
    serde_json::Value::Object(
        build_parsed_tool_input_map(call)
            .into_iter()
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

fn tool_kind(system_name: &str, registry: Option<&corework::system::SystemRegistry>) -> ToolKind {
    if let Some(meta) = crate::runtime_tools::get_runtime_tool_in_registry(registry, system_name) {
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

/// Validate historical arguments against today's contract without executing a tool.
pub fn validate_recovery_call(
    command: &str,
    parsed: Option<&crate::decision_line::ParsedToolCall>,
    registry: &corework::system::SystemRegistry,
) -> Result<(), String> {
    let (name, raw) = parse_tool_command(command);
    let params = if let Some(parsed) = parsed {
        parsed.params.clone()
    } else {
        let args = corework::ai_system::SimpleArgs::parse(raw).map_err(|e| e.to_string())?;
        args.names()
            .filter_map(|key| args.get(key).map(|v| (key.to_string(), v.to_string())))
            .collect()
    };
    let used = params
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let used = parsed
        .and_then(|p| p.arguments.as_ref())
        .map(|a| a.keys().cloned().collect())
        .unwrap_or(used);
    if let Some(error) = validate_param_name_list_using(name, &used, Some(registry)) {
        return Err(error);
    }
    let specs: Vec<(String, String, bool, bool)> = if let Some(factory) =
        inventory::iter::<AISystemFactory>().find(|f| f.metadata.name == name)
    {
        factory
            .metadata
            .parameters
            .iter()
            .map(|p| {
                (
                    p.name.into(),
                    p.param_type.into(),
                    p.required,
                    p.default_value.is_some(),
                )
            })
            .collect()
    } else if let Some(tool) =
        crate::runtime_tools::get_runtime_tool_in_registry(Some(registry), name)
    {
        tool.parameters
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.param_type.clone(),
                    p.required,
                    p.default_value.is_some(),
                )
            })
            .collect()
    } else {
        return Err(format!("tool_unavailable: {name}"));
    };
    let typed = parsed.and_then(|p| p.arguments.clone()).or_else(|| {
        if raw.trim_start().starts_with('{') {
            corework::tool_arguments::parse_object(raw).ok()
        } else {
            None
        }
    });
    if let Some(args) = typed {
        for (key, kind, required, has_default) in &specs {
            match args.get(key) {
                None if *required && !has_default => {
                    return Err(format!(
                        "tool_contract_changed: required parameter '{key}' is missing"
                    ))
                }
                Some(value) if !corework::tool_arguments::matches_type(value, kind) => {
                    return Err(format!(
                        "tool_contract_changed: parameter '{key}' must be {kind}"
                    ))
                }
                _ => {}
            }
        }
        return Ok(());
    }
    for (key, kind, required, has_default) in specs {
        let value = params.iter().find(|(name, _)| name == &key).map(|(_, v)| v);
        if required
            && !has_default
            && value.is_none()
            && !(key == "inputs" && used.iter().any(|name| name.starts_with("input.")))
        {
            return Err(format!(
                "tool_contract_changed: required parameter '{key}' is missing for '{name}'"
            ));
        }
        if let Some(value) = value {
            let valid = match kind.to_ascii_lowercase().as_str() {
                "bool" | "boolean" => value.parse::<bool>().is_ok(),
                "u8" => value.parse::<u8>().is_ok(),
                "u16" => value.parse::<u16>().is_ok(),
                "u32" => value.parse::<u32>().is_ok(),
                "u64" | "usize" => value.parse::<u64>().is_ok(),
                "i8" => value.parse::<i8>().is_ok(),
                "i16" => value.parse::<i16>().is_ok(),
                "i32" => value.parse::<i32>().is_ok(),
                "i64" | "isize" => value.parse::<i64>().is_ok(),
                "f32" | "f64" | "number" | "num" => {
                    value.parse::<f64>().is_ok_and(|n| n.is_finite())
                }
                kind if kind.starts_with("array") || kind.starts_with("vec") => {
                    serde_json::from_str::<serde_json::Value>(value).is_ok_and(|v| v.is_array())
                }
                _ => true,
            };
            if !valid {
                return Err(format!(
                    "tool_contract_changed: parameter '{key}' must be {kind} for '{name}'"
                ));
            }
        }
    }
    Ok(())
}

fn build_tool_input_map(
    args_str: &str,
) -> corework::error::Result<HashMap<String, serde_json::Value>> {
    if args_str.trim_start().starts_with('{') {
        let args = corework::tool_arguments::parse_object(args_str)
            .map_err(corework::error::FrameworkError::InvalidData)?;
        let mut map: HashMap<_, _> = args.into_iter().collect();
        map.insert(
            "input".into(),
            serde_json::Value::String(args_str.to_string()),
        );
        return Ok(map);
    }
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

fn build_parsed_tool_input_map(
    call: &crate::decision_line::ParsedToolCall,
) -> HashMap<String, serde_json::Value> {
    if let Some(args) = &call.arguments {
        let mut map: HashMap<_, _> = args.clone().into_iter().collect();
        map.insert(
            "input".into(),
            serde_json::Value::String(serde_json::Value::Object(args.clone()).to_string()),
        );
        return map;
    }
    let command = call.to_legacy_command();
    let (_, args_str) = parse_tool_command(&command);
    let mut input_map = HashMap::from([(
        "input".to_string(),
        serde_json::Value::String(args_str.to_string()),
    )]);
    for (name, value) in &call.params {
        input_map.insert(name.clone(), serde_json::Value::String(value.clone()));
    }
    input_map
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

#[cfg(test)]
fn validate_param_names(system_name: &str, args_str: &str) -> Option<String> {
    if args_str.is_empty() {
        return None;
    }

    validate_param_name_list(system_name, &extract_param_names(args_str))
}

#[cfg(test)]
fn validate_param_name_list(system_name: &str, used: &[String]) -> Option<String> {
    validate_param_name_list_using(system_name, used, None)
}
fn validate_param_name_list_using(
    system_name: &str,
    used: &[String],
    registry: Option<&corework::system::SystemRegistry>,
) -> Option<String> {
    let valid_names: Vec<String> = if let Some(factory) =
        inventory::iter::<AISystemFactory>().find(|f| f.metadata.name == system_name)
    {
        factory
            .metadata
            .parameters
            .iter()
            .map(|p| p.name.to_string())
            .collect()
    } else if let Some(meta) =
        crate::runtime_tools::get_runtime_tool_in_registry(registry, system_name)
    {
        meta.parameters.iter().map(|p| p.name.clone()).collect()
    } else {
        return None;
    };

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
    use corework::cache::{Cache, CacheExt, InMemoryCache};
    use corework::event::InMemoryEventBus;
    use corework::rpc_tool::{RuntimeAIParameter, RuntimeToolMetadata};
    use corework::system::SystemRegistry;
    use corework::workflow::dynamic_node::DynamicExecute;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    struct ForbiddenProbe;

    struct EchoStructuredScript;

    fn register_agent_test_tool(name: &str, parameters: Vec<RuntimeAIParameter>) {
        crate::runtime_tools::register_runtime_tool(RuntimeToolMetadata {
            name: name.to_string(),
            display_name: name.to_string(),
            description: "Test-only Agent tool metadata".to_string(),
            tool_kind: "local".to_string(),
            parameters,
            outputs: vec![],
            destructive: false,
            readonly: true,
            idempotent: true,
            open_world: false,
            secret: false,
            agent_enabled: true,
            jev_enabled: true,
            workflow_enabled: false,
            required_capabilities: vec![],
            endpoint_id: "test".to_string(),
            service: "test".to_string(),
            method: "execute".to_string(),
        });
    }

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

    #[async_trait]
    impl DynamicExecute for EchoStructuredScript {
        async fn execute_dynamic(
            &self,
            input: HashMap<String, Value>,
            _ctx: &corework::orchestration::Context,
        ) -> corework::error::Result<Value> {
            let script = input
                .get("script")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(json!({
                "result": {"script": script},
                "to_ai": script,
                "error_code": 0
            }))
        }
    }

    #[tokio::test]
    async fn rejects_registered_tool_when_it_is_not_active_for_agent() {
        register_agent_test_tool("ForbiddenProbe", vec![]);
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

    #[tokio::test]
    async fn executes_parsed_script_without_cli_roundtrip() {
        let script = "input visibility:String=\"好友可见\"\n1: EXEC BrowserClick --selector \"button[aria-label=\\\"发布\\\"]\"\nreturn";
        register_agent_test_tool(
            "EchoStructuredScript",
            vec![RuntimeAIParameter {
                name: "script".to_string(),
                param_type: "String".to_string(),
                required: true,
                default_value: None,
                description: "Script body".to_string(),
            }],
        );
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        crate::AssistantContext::set_active_tools(&cache, vec!["EchoStructuredScript".to_string()])
            .await
            .unwrap();
        let registry = Arc::new(SystemRegistry::new());
        registry.register_dynamic("EchoStructuredScript", Arc::new(EchoStructuredScript));
        let event_bus = Arc::new(InMemoryEventBus::new());
        let ctx = super::build_exec_ctx(cache, event_bus, registry);
        let call = crate::decision_line::ParsedToolCall {
            arguments: None,
            name: "EchoStructuredScript".to_string(),
            params: vec![("script".to_string(), script.to_string())],
        };

        let result = super::execute_parsed_with_call_id(&call, None, &ctx).await;

        assert!(result.success, "{}", result.to_ai);
        assert_eq!(result.to_ai, script);
        assert_eq!(result.result["script"], script);
        assert_eq!(super::permission_arguments_parsed(&call)["script"], script);
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
    fn parsed_tool_input_map_preserves_workflow_script_verbatim() {
        let script = "input visibility:String=\"好友可见\"\n1: EXEC BrowserClick --selector \"button[aria-label=\\\"发布\\\"]\"\nreturn";
        let call = crate::decision_line::ParsedToolCall {
            arguments: None,
            name: "WorkflowOpen".to_string(),
            params: vec![
                ("name".to_string(), "发布视频".to_string()),
                ("script".to_string(), script.to_string()),
            ],
        };

        let input = super::build_parsed_tool_input_map(&call);

        assert_eq!(input.get("script").and_then(Value::as_str), Some(script));
        assert_eq!(input.get("name").and_then(Value::as_str), Some("发布视频"));
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
            agent_enabled: true,
            jev_enabled: true,
            workflow_enabled: true,
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
    #[tokio::test]
    async fn json_and_fc_reach_rpc_with_the_same_typed_arguments() {
        use corework::rpc_tool::*;
        struct Capture(Arc<std::sync::Mutex<Vec<AgentToolRequest>>>);
        #[async_trait]
        impl RpcToolClient for Capture {
            async fn execute(
                &self,
                _: RpcEndpointInfo,
                _: RuntimeToolMetadata,
                r: AgentToolRequest,
                _: &corework::orchestration::Context,
            ) -> corework::error::Result<corework::ai_system::AIOutput> {
                let value = r.args_json.clone();
                self.0.lock().unwrap().push(r);
                Ok(corework::ai_system::AIOutput::success(value, "captured"))
            }
        }
        let meta:RuntimeToolMetadata=serde_json::from_value(json!({"name":"TypedRpcProbe","endpoint_id":"fixture","service":"fixture","method":"execute","readonly":true,"destructive":false,"parameters":[
            {"name":"payload","param_type":"Json","required":true},{"name":"count","param_type":"Number","required":true},{"name":"input","param_type":"String","required":true},{"name":"command","param_type":"String","required":true}
        ]})).unwrap();
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        crate::AssistantContext::set_active_tools(&cache, vec![meta.name.clone()])
            .await
            .unwrap();
        cache
            .set("conversation_id", &"real-conversation".to_string(), None)
            .await
            .unwrap();
        cache
            .set("agent_id", &"real-agent".to_string(), None)
            .await
            .unwrap();
        let endpoints = Arc::new(RpcEndpointRegistry::new());
        endpoints
            .insert(RpcEndpointInfo {
                endpoint_id: "fixture".into(),
                address: "127.0.0.1:1".into(),
                timeout_ms: 1000,
                to_ai_max_chars: 10000,
            })
            .unwrap();
        let tools = Arc::new(RuntimeToolRegistry::new());
        tools.insert(meta.clone()).unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rpc = Arc::new(RpcStubSystem::new(
            &meta.name,
            endpoints,
            tools,
            Arc::new(Capture(requests.clone())),
        ));
        let registry = Arc::new(SystemRegistry::new());
        registry.register_dynamic_with_metadata(meta, rpc);
        let ctx = super::build_exec_ctx(cache, Arc::new(InMemoryEventBus::new()), registry);
        let raw = r#"{"payload":{"items":[1,true,null,"001"]},"count":2,"input":"001","command":"literal"}"#;
        let text = format!("说明\nEXEC TypedRpcProbe {raw}");
        crate::runtime::parser::validate_response_shape(&text).unwrap();
        let calls = crate::runtime::parser::parse_tool_calls(&text).unwrap();
        let projected = crate::runtime::parser::frontend_projection(&text, &["text-call".into()]);
        assert!(!projected.contains("items"));
        let a = super::execute_parsed_with_call_id(&calls[0], Some("text-call"), &ctx).await;
        assert!(a.success, "{}", a.to_ai);
        let definition:llm_gateway::ToolDefinition=serde_json::from_value(json!({"type":"function","function":{"name":"TypedRpcProbe","description":"fixture","parameters":{"type":"object","properties":{"payload":{},"count":{"type":"number"},"input":{"type":"string"},"command":{"type":"string"}},"required":["payload","count","input","command"]}}})).unwrap();
        let fc = crate::tool_schema::validate_and_project_call(
            &llm_gateway::ToolCall::function("fc-call", "TypedRpcProbe", raw),
            &[definition],
        )
        .unwrap();
        let b = super::execute_parsed_with_call_id(&fc, Some("fc-call"), &ctx).await;
        assert!(b.success, "{}", b.to_ai);
        assert_eq!(requests.lock().unwrap().len(), 2);
        let expected: Value = serde_json::from_str(raw).unwrap();
        for request in requests.lock().unwrap().iter() {
            assert_eq!(request.args_json, expected);
            assert_eq!(request.conversation_id, "real-conversation");
        }
        assert_eq!(requests.lock().unwrap()[1].tool_call_id, "fc-call");
        assert_eq!(super::permission_arguments_parsed(&fc), expected);
        let rejected = super::execute_single_with_call_id(
            r#"TypedRpcProbe {"payload":null,"count":"2","input":"x","command":"literal"}"#,
            Some("bad"),
            &ctx,
        )
        .await;
        assert!(!rejected.success);
        assert_eq!(requests.lock().unwrap().len(), 2);
    }
}
