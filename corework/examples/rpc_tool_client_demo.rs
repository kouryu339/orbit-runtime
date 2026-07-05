use corework::prelude::*;
use corework::rpc_tool::{
    JsonLineRpcToolClient, RpcEndpointInfo, RpcEndpointRegistry, RpcStubSystem,
    RuntimeAIOutputField, RuntimeAIParameter, RuntimeToolMetadata, RuntimeToolRegistry,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> corework::error::Result<()> {
    let address =
        std::env::var("COREWORK_RPC_TEST_ADDR").unwrap_or_else(|_| "127.0.0.1:58081".to_string());

    let cache = Arc::new(InMemoryCache::new());
    let event_bus = Arc::new(InMemoryEventBus::new());
    let telemetry = Arc::new(NoopTelemetry);
    let registry = Arc::new(SystemRegistry::new());
    let ctx = Context::with_registry(cache.clone(), event_bus, telemetry, registry.clone());

    let endpoints = Arc::new(RpcEndpointRegistry::new());
    endpoints.insert(RpcEndpointInfo {
        endpoint_id: "python-demo".to_string(),
        address,
        timeout_ms: 10_000,
    })?;

    let tools = Arc::new(RuntimeToolRegistry::new());
    tools.insert(RuntimeToolMetadata {
        name: "PythonEchoProbe".to_string(),
        display_name: "Python Echo Probe".to_string(),
        description: "Ask a Python RPC tool to echo a probe value.".to_string(),
        tool_kind: "rpc".to_string(),
        parameters: vec![RuntimeAIParameter {
            name: "value".to_string(),
            param_type: "String".to_string(),
            required: true,
            default_value: None,
            description: "Probe value to echo.".to_string(),
        }],
        outputs: vec![RuntimeAIOutputField {
            name: "echoed_value".to_string(),
            field_type: "String".to_string(),
            description: "Value echoed by the sidecar.".to_string(),
        }],
        destructive: false,
        readonly: false,
        idempotent: false,
        open_world: true,
        secret: false,
        required_capabilities: vec![],
        endpoint_id: "python-demo".to_string(),
        service: "json-lines-test".to_string(),
        method: "execute".to_string(),
    })?;

    registry.register_dynamic(
        "PythonEchoProbe",
        Arc::new(RpcStubSystem::new(
            "PythonEchoProbe",
            endpoints,
            tools,
            Arc::new(JsonLineRpcToolClient),
        )),
    );

    let executor = ctx.get_dynamic_system("PythonEchoProbe")?;
    let mut input = std::collections::HashMap::new();
    input.insert(
        "input".to_string(),
        serde_json::Value::String("--value hello-from-corework".to_string()),
    );

    let output = executor.execute_dynamic(input, &ctx).await?;
    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}
