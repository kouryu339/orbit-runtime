use super::*;

impl RuntimeFacade {
    pub fn agent_cluster_definitions(&self) -> Result<String, RuntimeError> {
        let mut clusters = self
            .registries
            .agent_clusters
            .values()
            .map(|cluster| cluster_definition(cluster, "registered"))
            .collect::<Vec<_>>();

        if let Some(builtin) = &self.builtin_clusters {
            clusters.extend([
                cluster_definition(&builtin.workflow_editor, "builtin"),
                cluster_definition(&builtin.agent_test_supervisor, "builtin"),
                cluster_definition(&builtin.agent_test_adversary, "builtin"),
            ]);
        }
        clusters.sort_by(|left, right| {
            left["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["id"].as_str().unwrap_or_default())
        });

        serde_json::to_string(&json!({
            "schema": "agent-runtime-agent-cluster-definitions/v1",
            "runtime_started": self.started,
            "configuration_scope": "effective",
            "prompt_content_included": false,
            "clusters": clusters
        }))
        .map_err(|error| {
            RuntimeError::Internal(format!(
                "serialize agent cluster definitions failed: {error}"
            ))
        })
    }

    pub fn rpc_endpoint_definitions(&self) -> Result<String, RuntimeError> {
        let retrieval_endpoints = self.retrieval_endpoint_tools();
        let mut endpoints = self
            .config
            .rpc_tools
            .iter()
            .map(|endpoint| {
                let retrieval_tools = retrieval_endpoints.get(&endpoint.endpoint_id);
                let is_retrieval = retrieval_tools.is_some();
                let startup_verified = !is_retrieval && endpoint.tools.is_empty();
                let mut tool_names = endpoint
                    .tools
                    .iter()
                    .map(|tool| tool.name.clone())
                    .chain(
                        self.runtime_tools
                            .iter()
                            .filter(|tool| tool.endpoint_id == endpoint.endpoint_id)
                            .map(|tool| tool.name.clone()),
                    )
                    .chain(
                        retrieval_tools
                            .into_iter()
                            .flat_map(|tools| tools.iter().cloned()),
                    )
                    .collect::<BTreeSet<_>>();
                tool_names.retain(|name| !name.trim().is_empty());
                let usage = [
                    (!is_retrieval).then_some("tool_rpc"),
                    is_retrieval.then_some("retrieval"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
                let lifecycle = endpoint
                    .launch
                    .as_ref()
                    .map(|launch| launch.kind.as_str())
                    .unwrap_or("external");
                json!({
                    "id": endpoint.endpoint_id,
                    "protocol": endpoint.protocol,
                    "lifecycle": lifecycle,
                    "managed": lifecycle == "process",
                    "timeout_ms": endpoint.timeout_ms,
                    "address_configured": !endpoint.address.trim().is_empty(),
                    "connection_state": if !self.started {
                        "registered"
                    } else if startup_verified {
                        "ready"
                    } else {
                        "configured"
                    },
                    "verification_scope": if startup_verified {
                        "startup_list_tools"
                    } else {
                        "deferred_until_invoke"
                    },
                    "usage": usage,
                    "tool_count": tool_names.len(),
                    "tool_names": tool_names
                })
            })
            .collect::<Vec<_>>();
        endpoints.sort_by(|left, right| {
            left["id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["id"].as_str().unwrap_or_default())
        });

        serde_json::to_string(&json!({
            "schema": "agent-runtime-rpc-endpoint-definitions/v1",
            "runtime_started": self.started,
            "health_scope": "startup_only",
            "sensitive_connection_details_included": false,
            "endpoints": endpoints
        }))
        .map_err(|error| {
            RuntimeError::Internal(format!(
                "serialize RPC endpoint definitions failed: {error}"
            ))
        })
    }

    fn retrieval_endpoint_tools(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut endpoints = BTreeMap::<String, BTreeSet<String>>::new();
        let mut record = |retrieval: &RetrievalConfig| {
            if !retrieval.enabled {
                return;
            }
            let Some(endpoint_id) = retrieval
                .endpoint_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return;
            };
            endpoints
                .entry(endpoint_id.to_string())
                .or_default()
                .insert(retrieval.tool_name.clone());
        };

        self.config
            .agents
            .iter()
            .filter_map(|agent| agent.retrieval.as_ref())
            .for_each(&mut record);
        self.registries
            .agent_clusters
            .values()
            .flat_map(|cluster| cluster.agents.iter())
            .filter_map(|agent| agent.retrieval.as_ref())
            .for_each(&mut record);
        if let Some(resources) = &self.registries.resources {
            resources
                .agent_profiles
                .values()
                .filter_map(|profile| profile.retrieval.as_ref())
                .for_each(&mut record);
        }
        endpoints
    }
}

fn cluster_definition(cluster: &RuntimeAgentCluster, source: &str) -> Value {
    json!({
        "id": cluster.id,
        "description": cluster.description,
        "source": source,
        "focus_agent_id": cluster.focus_agent_id,
        "max_thinking_rounds": cluster.max_thinking_rounds,
        "permissions": cluster.permissions,
        "agents": cluster.agents.iter().map(|agent| json!({
            "id": agent.id,
            "profile_id": agent.profile_id,
            "name": agent.name,
            "role": agent.role,
            "features": agent.features,
            "system_skills": agent.system_skills,
            "model_uid": (agent.model_uid != 0).then_some(agent.model_uid),
            "retrieval": agent.retrieval,
            "system_prompt_constraints": agent.system_prompt_constraints,
            "frontend_widgets_enabled": agent.frontend_widgets_enabled
        })).collect::<Vec<_>>()
    })
}
