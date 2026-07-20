use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use corework::prelude::FrameworkState;
use corework::rpc_tool::{
    validate_runtime_tool_for_endpoint, GrpcRpcToolClient, GrpcRpcToolDiscoveryClient,
    JsonLineRpcToolClient, JsonLineRpcToolDiscoveryClient, RpcEndpointInfo, RpcEndpointRegistry,
    RpcStubSystem, RpcToolClient, RuntimeToolMetadata, RuntimeToolRegistry,
};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use super::RuntimeError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RpcToolEndpointConfig {
    pub endpoint_id: String,
    pub address: String,
    pub protocol: String,
    pub launch: Option<RpcToolLaunchConfig>,
    pub timeout_ms: u64,
    pub tools: Vec<RuntimeToolMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcToolLaunchConfig {
    pub kind: String,
    pub program: Option<PathBuf>,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub env: std::collections::BTreeMap<String, String>,
    pub startup_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
}

impl Default for RpcToolLaunchConfig {
    fn default() -> Self {
        Self {
            kind: "external".to_string(),
            program: None,
            args: Vec::new(),
            working_dir: None,
            env: std::collections::BTreeMap::new(),
            startup_timeout_ms: 10_000,
            shutdown_timeout_ms: 3_000,
        }
    }
}

impl Default for RpcToolEndpointConfig {
    fn default() -> Self {
        Self {
            endpoint_id: String::new(),
            address: String::new(),
            protocol: "grpc".to_string(),
            launch: None,
            timeout_ms: 30_000,
            tools: Vec::new(),
        }
    }
}

pub(super) struct ManagedSidecar {
    child: Child,
    shutdown_timeout_ms: u64,
}

impl Drop for ManagedSidecar {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_millis(self.shutdown_timeout_ms);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.wait();
    }
}

pub(super) async fn install_rpc_tools_from_config(
    configs: Vec<RpcToolEndpointConfig>,
) -> Result<(Vec<ManagedSidecar>, Vec<RuntimeToolMetadata>), RuntimeError> {
    if configs.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let framework = FrameworkState::initialize()
        .map_err(|e| RuntimeError::Internal(format!("initialize framework failed: {e}")))?;
    let system_registry = framework.registry();
    let endpoint_registry = Arc::new(RpcEndpointRegistry::new());
    let tool_registry = Arc::new(RuntimeToolRegistry::new());
    let discovery = GrpcRpcToolDiscoveryClient::default();
    let json_line_discovery = JsonLineRpcToolDiscoveryClient;
    let mut active_tools = Vec::new();
    let mut sidecar_children = Vec::new();

    for endpoint_config in configs {
        if endpoint_config.endpoint_id.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "rpc_tools endpoint_id must not be empty".to_string(),
            ));
        }
        if endpoint_config.address.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(format!(
                "rpc_tools endpoint '{}' address must not be empty",
                endpoint_config.endpoint_id
            )));
        }
        let endpoint_info = RpcEndpointInfo {
            endpoint_id: endpoint_config.endpoint_id.clone(),
            address: endpoint_config.address.clone(),
            timeout_ms: endpoint_config.timeout_ms,
        };
        let launch = endpoint_config
            .launch
            .clone()
            .unwrap_or_else(RpcToolLaunchConfig::default);
        if launch.kind == "process" {
            sidecar_children.push(launch_sidecar_process(&endpoint_config, &launch)?);
        }

        let (tools, client): (Vec<RuntimeToolMetadata>, Arc<dyn RpcToolClient>) =
            match endpoint_config.protocol.as_str() {
                "json-lines" => {
                    let tools = if endpoint_config.tools.is_empty() {
                        discover_json_line_tools_with_retry(
                            &json_line_discovery,
                            endpoint_info.clone(),
                            if launch.kind == "process" {
                                launch.startup_timeout_ms
                            } else {
                                0
                            },
                        )
                        .await?
                    } else {
                        endpoint_config.tools.clone()
                    };
                    (tools, Arc::new(JsonLineRpcToolClient))
                }
                "grpc" => {
                    if !endpoint_config.tools.is_empty() {
                        return Err(RuntimeError::InvalidConfig(format!(
                            "rpc_tools endpoint '{}' uses protocol=grpc; tool metadata must be served by AgentToolService.ListTools, not configured inline",
                            endpoint_config.endpoint_id
                        )));
                    }
                    (
                        discover_grpc_tools_with_retry(
                            &discovery,
                            endpoint_info.clone(),
                            if launch.kind == "process" {
                                launch.startup_timeout_ms
                            } else {
                                0
                            },
                        )
                        .await?,
                        Arc::new(GrpcRpcToolClient),
                    )
                }
                other => {
                    return Err(RuntimeError::Rpc(format!(
                        "rpc_tools endpoint '{}' protocol '{}' is not supported; use 'json-lines' or 'grpc'",
                        endpoint_config.endpoint_id, other
                    )));
                }
            };

        endpoint_registry
            .insert(endpoint_info.clone())
            .map_err(|e| RuntimeError::Rpc(e.to_string()))?;

        for mut tool in tools {
            if tool.name.trim().is_empty() {
                return Err(RuntimeError::InvalidConfig(format!(
                    "rpc_tools endpoint '{}' has a tool with empty name",
                    endpoint_config.endpoint_id
                )));
            }
            if tool.endpoint_id.trim().is_empty() {
                tool.endpoint_id = endpoint_config.endpoint_id.clone();
            }
            if tool.endpoint_id != endpoint_config.endpoint_id {
                return Err(RuntimeError::InvalidConfig(format!(
                    "rpc tool '{}' endpoint_id '{}' does not match enclosing endpoint '{}'",
                    tool.name, tool.endpoint_id, endpoint_config.endpoint_id
                )));
            }
            validate_runtime_tool_for_endpoint(&endpoint_info, &tool)
                .map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;

            tool_registry
                .insert(tool.clone())
                .map_err(|e| RuntimeError::Rpc(e.to_string()))?;
            ai_assistant::runtime_tools::register_runtime_tool(tool.clone());

            system_registry.register_dynamic_with_metadata(
                tool.clone(),
                Arc::new(RpcStubSystem::new(
                    tool.name.clone(),
                    Arc::clone(&endpoint_registry),
                    Arc::clone(&tool_registry),
                    client.clone(),
                )),
            );
            active_tools.push(tool);
        }
    }

    Ok((sidecar_children, active_tools))
}

fn launch_sidecar_process(
    endpoint_config: &RpcToolEndpointConfig,
    launch: &RpcToolLaunchConfig,
) -> Result<ManagedSidecar, RuntimeError> {
    let program = launch.program.as_ref().ok_or_else(|| {
        RuntimeError::InvalidConfig(format!(
            "rpc_tools endpoint '{}' launch.program is required when launch.kind=process",
            endpoint_config.endpoint_id
        ))
    })?;

    let mut command = Command::new(program);
    command
        .args(&launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(working_dir) = &launch.working_dir {
        command.current_dir(working_dir);
    }
    for (key, value) in &launch.env {
        command.env(key, value);
    }

    let child = command.spawn().map_err(|e| {
        RuntimeError::Rpc(format!(
            "start rpc_tools endpoint '{}' sidecar failed: {e}",
            endpoint_config.endpoint_id
        ))
    })?;

    Ok(ManagedSidecar {
        child,
        shutdown_timeout_ms: launch.shutdown_timeout_ms,
    })
}

async fn discover_grpc_tools_with_retry(
    discovery: &GrpcRpcToolDiscoveryClient,
    endpoint_info: RpcEndpointInfo,
    startup_timeout_ms: u64,
) -> Result<Vec<RuntimeToolMetadata>, RuntimeError> {
    if startup_timeout_ms == 0 {
        return discovery
            .list_tools(endpoint_info)
            .await
            .map_err(|e| RuntimeError::Rpc(e.to_string()));
    }

    let deadline = Instant::now() + Duration::from_millis(startup_timeout_ms);
    loop {
        let error = match discovery.list_tools(endpoint_info.clone()).await {
            Ok(tools) => return Ok(tools),
            Err(error) => error.to_string(),
        };

        if Instant::now() >= deadline {
            return Err(RuntimeError::Rpc(format!(
                "rpc_tools endpoint '{}' did not become ready within {}ms: {}",
                endpoint_info.endpoint_id, startup_timeout_ms, error
            )));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn discover_json_line_tools_with_retry(
    discovery: &JsonLineRpcToolDiscoveryClient,
    endpoint_info: RpcEndpointInfo,
    startup_timeout_ms: u64,
) -> Result<Vec<RuntimeToolMetadata>, RuntimeError> {
    if startup_timeout_ms == 0 {
        return discovery
            .list_tools(endpoint_info)
            .await
            .map_err(|e| RuntimeError::Rpc(e.to_string()));
    }

    let deadline = Instant::now() + Duration::from_millis(startup_timeout_ms);
    loop {
        let error = match discovery.list_tools(endpoint_info.clone()).await {
            Ok(tools) => return Ok(tools),
            Err(error) => error.to_string(),
        };
        if Instant::now() >= deadline {
            return Err(RuntimeError::Rpc(format!(
                "rpc_tools endpoint '{}' did not become ready within {}ms: {}",
                endpoint_info.endpoint_id, startup_timeout_ms, error
            )));
        }
        sleep(Duration::from_millis(100)).await;
    }
}
