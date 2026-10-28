use super::*;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ClusterDefinitionVersion {
    pub cluster_id: String,
    pub version: String,
}

pub(super) fn cluster_version(cluster: &RuntimeAgentCluster) -> String {
    let bytes = serde_json::to_vec(cluster).expect("cluster configuration is JSON serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

impl RuntimeFacade {
    /// The FFI facade write lock serializes this transaction with spawn/close.
    /// Only accepted configuration changes; live conversations are never patched.
    pub fn add_agent_cluster(&mut self, registration: &Value) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let cluster = self.prepare_dynamic_agent_cluster(registration)?;
        if self.registries.agent_clusters.contains_key(&cluster.id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' is already registered; use runtime.update_agent_cluster",
                cluster.id
            )));
        }
        self.commit_dynamic_agent_cluster(cluster, "added")
    }

    #[allow(dead_code)]
    pub fn update_agent_cluster(
        &mut self,
        registration: &Value,
        expected_version: &str,
    ) -> Result<Value, RuntimeError> {
        self.update_agent_cluster_with_skills(registration, expected_version, &[])
    }

    pub fn update_agent_cluster_with_skills(
        &mut self,
        registration: &Value,
        expected_version: &str,
        reload_skills: &[String],
    ) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        if let Some(resources) = &self.registries.resources {
            let state =
                super::skill_transactions::read_managed_state(&resources.managed_skills_dir)?;
            if registration
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| state.clusters.contains_key(id))
            {
                return Err(RuntimeError::InvalidConfig("Transaction-managed clusters must be updated through runtime.commit_skill_transaction".into()));
            }
        }
        let request: AgentClusterRegistration = serde_json::from_value(registration.clone())
            .map_err(|error| {
                RuntimeError::InvalidConfig(format!("parse agent cluster failed: {error}"))
            })?;
        // Check the old identity before any candidate parsing can load skills.
        let current = self
            .registries
            .agent_clusters
            .get(&request.id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "agent cluster '{}' is not registered",
                    request.id
                ))
            })?;
        let actual = cluster_version(current);
        if expected_version != actual {
            return Err(RuntimeError::ClusterVersionConflict {
                cluster_id: request.id,
                expected: expected_version.to_string(),
                actual,
            });
        }
        let manager = self.manager()?;
        let live_ids = self
            .rt
            .block_on(manager.list())
            .into_iter()
            .map(|info| info.conversation_id)
            .collect::<BTreeSet<_>>();
        let mut conversation_ids = self
            .conversation_instances
            .iter()
            .filter(|(id, metadata)| metadata.cluster_id == request.id && live_ids.contains(*id))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        conversation_ids.sort();
        if !conversation_ids.is_empty() {
            return Err(RuntimeError::ClusterInUse {
                cluster_id: request.id,
                conversation_ids,
            });
        }
        if reload_skills.is_empty() {
            let cluster = self.prepare_dynamic_agent_cluster(registration)?;
            return self.commit_dynamic_agent_cluster(cluster, "updated");
        }
        let names: BTreeSet<String> = reload_skills.iter().cloned().collect();
        if names.len() != reload_skills.len() || names.iter().any(|name| name.trim().is_empty()) {
            return Err(RuntimeError::InvalidConfig(
                "reload_skills must contain unique non-empty names".into(),
            ));
        }
        let catalog = ai_assistant::skills::systems::SKILL_MANAGER
            .get()
            .ok_or_else(|| RuntimeError::InvalidConfig("Skill catalog is not initialized".into()))?
            .clone();
        let rt = &self.rt;
        // A global barrier is necessary because SkillManager is process-wide,
        // and Agents may activate skills or spawn workers without an FFI call.
        let admission = rt.block_on(ai_assistant::skills::usage::SKILL_UPDATE_GATE.write());
        let dependencies = rt
            .block_on(ai_assistant::skills::usage::dependencies(Some(&names)))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        if !dependencies.is_empty() {
            return Err(RuntimeError::SkillInUse { dependencies });
        }
        let mut staged = rt.block_on(async { catalog.read().await.clone() });
        rt.block_on(staged.reload_registered(reload_skills))
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        let cluster = self.prepare_dynamic_agent_cluster(registration)?;
        let selected: BTreeSet<_> = cluster
            .agents
            .iter()
            .flat_map(|agent| agent.role.iter().chain(agent.features.iter()).cloned())
            .collect();
        if !names.is_subset(&selected) {
            return Err(RuntimeError::InvalidConfig(
                "reload_skills must be referenced by the submitted cluster".into(),
            ));
        }
        let versions = self
            .rt
            .block_on(staged.content_versions(reload_skills))
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        // Prevent thinking from seeing the new catalog before the configuration
        // has committed. No file writes or fallible operations follow the swap.
        let mut live = self.rt.block_on(catalog.write());
        let mut result = self.commit_dynamic_agent_cluster(cluster, "updated")?;
        live.publish_reloaded(&staged, reload_skills);
        tracing::info!(skills = ?versions, "Skill content reload accepted");
        result["reloaded_skills"] = json!(versions);
        drop(live);
        drop(admission);
        Ok(result)
    }

    fn prepare_dynamic_agent_cluster(
        &self,
        registration: &Value,
    ) -> Result<RuntimeAgentCluster, RuntimeError> {
        let mut manager = self.rt.block_on(async {
            let catalog = ai_assistant::skills::systems::SKILL_MANAGER
                .get()
                .ok_or_else(|| {
                    RuntimeError::InvalidConfig("Skill catalog is not initialized".into())
                })?;
            Ok::<_, RuntimeError>(catalog.read().await.clone())
        })?;
        self.prepare_cluster_with_catalog(registration, &mut manager)
    }

    pub(super) fn prepare_cluster_with_catalog(
        &self,
        registration: &Value,
        manager: &mut SkillManager,
    ) -> Result<RuntimeAgentCluster, RuntimeError> {
        let request: AgentClusterRegistration = serde_json::from_value(registration.clone())
            .map_err(|error| {
                RuntimeError::InvalidConfig(format!("parse agent cluster failed: {error}"))
            })?;
        let cluster = build_agent_cluster_registry(request, &self.registries)?;
        if matches!(
            cluster.id.as_str(),
            "workflow-studio" | "agent-test-supervisor" | "agent-test-adversary"
        ) {
            return Err(RuntimeError::InvalidConfig(
                "built-in agent clusters cannot be replaced".into(),
            ));
        }
        let focus = cluster
            .agents
            .iter()
            .find(|agent| agent.id == cluster.focus_agent_id)
            .expect("cluster builder validates focus membership");
        if focus.role.is_none() {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' primary agent '{}' must provide a Role Skill",
                cluster.id, focus.id
            )));
        }
        // Resolve identities against the accepted catalog. Explicit reloads
        // preserve name/kind and publish only after dependency admission.
        for agent in &cluster.agents {
            for (name, role) in agent
                .role
                .iter()
                .map(|name| (name, true))
                .chain(agent.features.iter().map(|name| (name, false)))
            {
                let skill = manager.get(name).ok_or_else(|| RuntimeError::InvalidConfig(format!(
                        "Skill '{name}' is not registered in the active catalog; install it before startup"
                    )))?;
                if skill.metadata.is_role() != role || (!role && skill.metadata.system_layer) {
                    return Err(RuntimeError::InvalidConfig(format!(
                        "Skill '{name}' has the wrong kind in the active catalog"
                    )));
                }
            }
        }
        // Validate using an isolated manager. Failed requests leave the running
        // SkillManager, registry, source files and conversation caches unchanged.
        let mut config = self.config.clone();
        config.agents = cluster
            .agents
            .iter()
            .map(runtime_agent_definition_to_agent_section)
            .collect();
        self.rt
            .block_on(validate_agent_skill_catalog(&config, true, manager))?;
        for agent in &cluster.agents {
            if let Some(retrieval) = agent
                .retrieval
                .as_ref()
                .filter(|r| r.before_thinking_enabled())
            {
                if !self
                    .runtime_tools
                    .iter()
                    .any(|tool| tool.name == retrieval.tool_name && tool.tool_kind == "local")
                {
                    return Err(RuntimeError::InvalidConfig(format!(
                        "retrieval tool '{}' was not installed at startup",
                        retrieval.tool_name
                    )));
                }
            }
        }
        Ok(cluster)
    }

    fn commit_dynamic_agent_cluster(
        &mut self,
        cluster: RuntimeAgentCluster,
        action: &str,
    ) -> Result<Value, RuntimeError> {
        let result = json!({
            "schema": "agent-runtime-agent-cluster-change/v1",
            "action": action,
            "cluster": super::resource_definitions::cluster_definition(&cluster, "registered")
        });
        tracing::info!(cluster_id = %cluster.id, focus_agent_id = %cluster.focus_agent_id,
            agent_count = cluster.agents.len(), action, "agent cluster configuration accepted");
        self.registries
            .dynamic_agent_clusters
            .insert(cluster.id.clone());
        self.registries
            .agent_clusters
            .insert(cluster.id.clone(), cluster);
        Ok(result)
    }

    pub(super) fn cluster_definition_version(
        &self,
        cluster_id: &str,
    ) -> Option<ClusterDefinitionVersion> {
        self.registries
            .agent_clusters
            .get(cluster_id)
            .map(|cluster| ClusterDefinitionVersion {
                cluster_id: cluster.id.clone(),
                version: cluster_version(cluster),
            })
    }

    pub(super) fn check_snapshot_cluster_version(
        &self,
        cluster_id: &str,
        snapshot: Option<&ClusterDefinitionVersion>,
    ) -> Result<(), RuntimeError> {
        let current = self.cluster_definition_version(cluster_id).ok_or_else(|| {
            RuntimeError::InvalidConfig(format!("agent cluster '{cluster_id}' is not registered"))
        })?;
        if let Some(snapshot) = snapshot {
            if snapshot != &current {
                return Err(RuntimeError::ClusterVersionConflict {
                    cluster_id: cluster_id.to_string(),
                    expected: format!("{}:{}", snapshot.cluster_id, snapshot.version),
                    actual: format!("{}:{}", current.cluster_id, current.version),
                });
            }
        } else if self.registries.dynamic_agent_clusters.contains(cluster_id) {
            return Err(RuntimeError::InvalidConfig(format!(
                "cluster_version_missing: snapshot has no definition version for dynamic cluster '{cluster_id}'"
            )));
        }
        Ok(())
    }

    pub(super) fn conversation_skill_versions(
        &self,
        conversation_id: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        self.rt.block_on(async {
            let used = ai_assistant::skills::usage::dependencies(None)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let names = used
                .into_iter()
                .filter(|usage| usage.conversation_id == conversation_id)
                .flat_map(|usage| usage.skill_names)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let catalog = ai_assistant::skills::systems::SKILL_MANAGER
                .get()
                .ok_or_else(|| {
                    RuntimeError::InvalidConfig("Skill catalog is not initialized".into())
                })?;
            catalog
                .write()
                .await
                .content_versions(&names)
                .await
                .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))
        })
    }

    // Caller holds read admission before changing recovery state.
    pub(super) fn check_snapshot_skill_versions(
        &self,
        cluster_id: Option<&str>,
        versions: Option<&BTreeMap<String, String>>,
    ) -> Result<(), RuntimeError> {
        let Some(versions) = versions else {
            if cluster_id.is_some_and(|id| self.registries.dynamic_agent_clusters.contains(id)) {
                return Err(RuntimeError::InvalidConfig(
                    "skill_version_missing: dynamic cluster snapshot requires skill_versions"
                        .into(),
                ));
            }
            return Ok(());
        };
        let actual = self.rt.block_on(async {
            let catalog = ai_assistant::skills::systems::SKILL_MANAGER
                .get()
                .ok_or_else(|| {
                    RuntimeError::InvalidConfig("Skill catalog is not initialized".into())
                })?;
            catalog
                .write()
                .await
                .content_versions(&versions.keys().cloned().collect::<Vec<_>>())
                .await
                .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))
        })?;
        if &actual != versions {
            let (name, expected) = versions
                .iter()
                .find(|(name, expected)| actual.get(*name) != Some(*expected))
                .expect("different maps have a differing entry");
            return Err(RuntimeError::SkillVersionConflict {
                name: name.clone(),
                expected: Some(expected.clone()),
                actual: actual.get(name).cloned(),
            });
        }
        Ok(())
    }
}
