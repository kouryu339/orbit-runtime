use super::*;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddRpcEndpointRequest {
    schema: String,
    endpoint: ResourceRpcEndpointConfig,
}

pub(crate) struct RpcEndpointAddition {
    handle: tokio::runtime::Handle,
    registry: Arc<corework::system::SystemRegistry>,
    endpoint: RpcToolEndpointConfig,
}

pub(crate) struct DiscoveredRpcEndpoint {
    registry: Arc<corework::system::SystemRegistry>,
    endpoint: RpcToolEndpointConfig,
    prepared: PreparedRpcTools,
    contract_hash: String,
}

impl RpcEndpointAddition {
    /// Called without holding the facade lock. ListTools never executes a tool.
    pub(crate) fn discover(self) -> Result<DiscoveredRpcEndpoint, RuntimeError> {
        let prepared = self.handle.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(self.endpoint.timeout_ms),
                prepare_rpc_tools(vec![self.endpoint.clone()]),
            )
            .await
            .map_err(|_| RuntimeError::Rpc("RPC discovery deadline exceeded".into()))?
        })?;
        if prepared.tools.is_empty() || prepared.tools.len() > 1024 {
            return Err(RuntimeError::InvalidConfig(
                "RPC discovery must return between 1 and 1024 tools".into(),
            ));
        }
        let mut tools = prepared.tools.clone();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let bytes = serde_json::to_vec(&tools)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(RuntimeError::InvalidConfig(
                "RPC tool catalog exceeds 4 MiB".into(),
            ));
        }
        Ok(DiscoveredRpcEndpoint {
            registry: self.registry,
            endpoint: self.endpoint,
            prepared,
            contract_hash: format!("sha256:{:x}", Sha256::digest(bytes)),
        })
    }
}

impl RuntimeFacade {
    pub(crate) fn prepare_rpc_endpoint_addition(
        &self,
        payload: Value,
    ) -> Result<RpcEndpointAddition, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let request: AddRpcEndpointRequest = serde_json::from_value(payload)
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        if request.schema != "agent-runtime-rpc-endpoint-add/v1" {
            return Err(RuntimeError::InvalidConfig(
                "schema must be 'agent-runtime-rpc-endpoint-add/v1'".into(),
            ));
        }
        // Dynamic process ownership and retrieval bindings remain startup-only.
        if request.endpoint.protocol != "grpc" || request.endpoint.launch.is_some() {
            return Err(RuntimeError::InvalidConfig(
                "dynamic RPC additions require protocol=grpc and an external endpoint; omit launch"
                    .into(),
            ));
        }
        if !(1..=60_000).contains(&request.endpoint.timeout_ms)
            || !(1..=1_000_000).contains(&request.endpoint.to_ai_max_chars)
            || request.endpoint.endpoint.len() > 4096
        {
            return Err(RuntimeError::InvalidConfig(
                "RPC timeout_ms must be 1..60000, to_ai_max_chars 1..1000000, endpoint at most 4096 bytes".into(),
            ));
        }
        let endpoint = resource_rpc_endpoint_to_runtime(request.endpoint, None)?;
        self.ensure_new_rpc_endpoint(&endpoint.endpoint_id)?;
        Ok(RpcEndpointAddition {
            handle: self.rt.handle().clone(),
            registry: self.tool_registry.clone(),
            endpoint,
        })
    }

    fn ensure_new_rpc_endpoint(&self, id: &str) -> Result<(), RuntimeError> {
        if self
            .config
            .rpc_tools
            .iter()
            .any(|endpoint| endpoint.endpoint_id == id)
        {
            return Err(RuntimeError::RpcAdditionConflict {
                kind: "rpc_endpoint_exists",
                names: vec![id.to_owned()],
            });
        }
        Ok(())
    }

    pub(crate) fn commit_rpc_endpoint_addition(
        &mut self,
        addition: DiscoveredRpcEndpoint,
        command_id: &str,
    ) -> Result<Value, RuntimeError> {
        if !self.started || !Arc::ptr_eq(&self.tool_registry, &addition.registry) {
            return Err(RuntimeError::NotStarted);
        }
        self.ensure_new_rpc_endpoint(&addition.endpoint.endpoint_id)?;
        let conflicts = addition
            .prepared
            .tools
            .iter()
            .filter(|tool| {
                tool.name.starts_with("WorkflowRef_")
                    || self.tool_registry.get_dynamic(&tool.name).is_some()
                    || corework::workflow::registry::NodeRegistry::get(&tool.name).is_some()
                    || corework::system::SystemRegistry::list_ai_systems()
                        .iter()
                        .any(|builtin| builtin.name == tool.name)
            })
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            return Err(RuntimeError::RpcAdditionConflict {
                kind: "rpc_tool_name_conflict",
                names: conflicts,
            });
        }
        // Stage every fallible operation before publishing the catalog batch.
        let mut rpc_pool = self
            .registries
            .resources
            .as_ref()
            .map(|resources| resources.rpc_pool.clone());
        if let Some(pool) = rpc_pool.as_mut() {
            pool.insert(addition.endpoint.clone())?;
        }
        let names = addition
            .prepared
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let revision = self
            .tool_registry
            .append_runtime_tools(addition.prepared.entries)
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        self.config.rpc_tools.push(addition.endpoint.clone());
        self.dynamic_rpc_endpoint_ids
            .insert(addition.endpoint.endpoint_id.clone());
        if let (Some(resources), Some(pool)) = (self.registries.resources.as_mut(), rpc_pool) {
            resources.rpc_pool = pool;
        }
        self.runtime_tools.extend(addition.prepared.tools);
        self.sidecar_children.extend(addition.prepared.sidecars);
        let result = json!({
            "schema": "agent-runtime-rpc-endpoint-add-result/v1",
            "endpoint_id": addition.endpoint.endpoint_id,
            "protocol": "grpc",
            "catalog_revision": revision,
            "contract_hash": addition.contract_hash,
            "tool_count": names.len(),
            "tool_names": names,
            "persistence": "host_managed"
        });
        let mut event = result.clone();
        event["command_id"] = json!(command_id);
        if let Err(error) = self.rt.block_on(
            self.event_bus
                .publish(BaseEvent::new(RPC_CATALOG_CHANGED_EVENT, event)),
        ) {
            tracing::warn!(%error, "RPC catalog committed but catalog event delivery failed");
        }
        tracing::info!(endpoint_id = %addition.endpoint.endpoint_id, revision, command_id, "RPC endpoint added");
        Ok(result)
    }
}
