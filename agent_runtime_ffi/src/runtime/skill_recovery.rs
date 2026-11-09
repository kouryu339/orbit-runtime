//! Rebind recoverable history to accepted current definitions, never cached authority.
use super::*;

pub(super) const RESTORE_REPORT: &str = "runtime:restore_report";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct SnapshotSkillBindings {
    #[serde(default)]
    pub main_skills: Vec<String>,
    #[serde(default)]
    pub imported_skills: Vec<String>,
}

pub(super) struct RecoveredAgentSkills {
    agent_id: String,
    main: Vec<String>,
    imported: Vec<String>,
    system: BTreeMap<String, String>,
    tools: Vec<String>,
}

pub(super) struct SkillRecoveryPlan {
    agents: Vec<RecoveredAgentSkills>,
    pub report: Value,
}

impl RuntimeFacade {
    pub(super) fn prepare_recovered_skills(
        &self,
        conversation_id: &str,
        snapshot: &super::recovery::ConversationSnapshotImport,
    ) -> Result<SkillRecoveryPlan, RuntimeError> {
        let manager = self.manager()?;
        let cluster_id = self
            .conversation_instances
            .get(conversation_id)
            .map(|m| m.cluster_id.as_str());
        let definitions = cluster_id.and_then(|id| self.registries.agent_clusters.get(id));
        let mut imported = snapshot.skill_bindings.clone();
        for delta in super::recovery::snapshot_state_deltas(snapshot) {
            if delta["op"] != "agent_skills.set" {
                continue;
            }
            if let Some(id) = delta["agent_id"].as_str() {
                let binding = imported.entry(id.to_string()).or_default();
                if let Some(names) = delta["imported_skills"].as_array() {
                    binding.imported_skills = names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                }
                if let Some(names) = delta["main_skills"].as_array() {
                    binding.main_skills = names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                }
            }
        }
        self.rt.block_on(async {
            let cluster = manager.cluster_snapshot(conversation_id).await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let catalog = ai_assistant::skills::systems::SKILL_MANAGER.get()
                .ok_or_else(|| RuntimeError::InvalidConfig("required_skill_missing: Skill catalog is not initialized".into()))?;
            let mut catalog = catalog.write().await;
            let mut agents = Vec::new();
            let mut removed = Vec::new();
            let mut names = BTreeSet::new();
            let policy=manager.tool_permission_policy(conversation_id).await.map_err(|e|RuntimeError::Internal(e.to_string()))?;
            for agent in cluster.agents {
                let cache = manager.agent_cache(conversation_id, &agent.agent_id).await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                let definition = definitions.and_then(|cluster| cluster.agents.iter().find(|d| d.id == agent.definition_id || d.id == agent.agent_id));
                let (role, features, system) = if let Some(definition) = definition {
                    (definition.role.clone(), definition.features.clone(), definition.system_skills.clone())
                } else {
                    let main: Vec<String> = cache.get(ai_assistant::context::keys::MAIN_SKILLS).await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?.unwrap_or_default();
                    let role = main.iter().find(|name| catalog.get(name).is_some_and(|s| s.metadata.is_role())).cloned();
                    let features = main.into_iter().filter(|name| Some(name) != role.as_ref()).collect();
                    let system = cache.get(ai_assistant::context::keys::SYSTEM_SKILLS).await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?.unwrap_or_default();
                    (role, features, system)
                };
                let role = role.ok_or_else(|| RuntimeError::InvalidConfig(format!("required_skill_missing: agent '{}' has no Role Skill", agent.agent_id)))?;
                let loaded = catalog.load(&role).await.map_err(|e| RuntimeError::InvalidConfig(format!("required_skill_missing: agent '{}' Role '{role}': {e}",agent.agent_id)))?;
                if !loaded.metadata.is_role() || loaded.instructions.trim().is_empty() {
                    return Err(RuntimeError::InvalidConfig(format!("required_skill_invalid: agent '{}' Role '{role}' must have kind=role and non-empty instructions", agent.agent_id)));
                }
                let mut main = vec![role];
                let mut active_imports = Vec::new();
                for (optional, destination) in [
                    (features, &mut main),
                    (imported.get(&agent.agent_id).map(|s| s.imported_skills.clone()).unwrap_or_default(), &mut active_imports),
                ] {
                    for name in optional {
                        match catalog.load(&name).await {
                            Ok(skill) if !skill.metadata.is_role() && !skill.metadata.system_layer && !skill.instructions.trim().is_empty() => {
                                if !destination.contains(&name) { destination.push(name); }
                            }
                            _ => removed.push(json!({"agent_id":agent.agent_id,"name":name,"reason":"optional_skill_unavailable"})),
                        }
                    }
                }
                if let Some(previous) = imported.get(&agent.agent_id) {
                    for old in &previous.main_skills {
                        if !main.contains(old) && !catalog.get(old).is_some_and(|s| s.metadata.is_role() || s.metadata.system_layer) {
                            removed.push(json!({"agent_id":agent.agent_id,"name":old,"reason":"not_in_current_cluster"}));
                        }
                    }
                }
                let protocol: ai_assistant::ToolProtocol = cache.get(ai_assistant::context::keys::TOOL_PROTOCOL).await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?.unwrap_or_default();
                let configured = system.get("thinking").map(String::as_str).unwrap_or("thinking");
                let thinking = match (protocol, configured) {
                    (ai_assistant::ToolProtocol::NativeFc,"thinking") => "thinking-fc",
                    (ai_assistant::ToolProtocol::NativeFc,"thinking-pro") => "thinking-pro-fc",
                    (_, name) => name,
                };
                for name in system.values().map(String::as_str).chain(std::iter::once(thinking)) {
                    let skill = catalog.load(name).await.map_err(|e| RuntimeError::InvalidConfig(format!("required_skill_missing: agent '{}' system Skill '{name}': {e}",agent.agent_id)))?;
                    if !skill.metadata.system_layer || skill.instructions.trim().is_empty() {
                        return Err(RuntimeError::InvalidConfig(format!("required_skill_invalid: agent '{}' system Skill '{name}' must be system_layer with non-empty instructions",agent.agent_id)));
                    }
                    names.insert(name.to_string());
                }
                let selected: Vec<&str> = main.iter().chain(active_imports.iter()).map(String::as_str).collect();
                let mut tools = Vec::new();
                catalog.inject_tools_for_skills(&selected, &mut tools);
                tools = catalog.filtered_tools_for_state(thinking, tools);
                tools.retain(|name| {
                    let agent_enabled = inventory::iter::<AISystemFactory>().find(|f| f.metadata.name == name.as_str()).map(|f| f.metadata.agent_enabled)
                        .or_else(|| self.tool_registry.get_dynamic_metadata(name).map(|m| m.agent_enabled)).unwrap_or(false);
                    agent_enabled && ai_assistant::tool_runner::permission_metadata_in_registry(name, Some(&self.tool_registry))
                        .is_ok_and(|m| policy.mode_for(m.effect) != ai_assistant::ToolPermissionMode::Deny)
                });
                tools.sort(); tools.dedup();
                if protocol == ai_assistant::ToolProtocol::Disabled { tools.clear(); }
                names.extend(main.iter().chain(active_imports.iter()).cloned());
                agents.push(RecoveredAgentSkills {agent_id:agent.agent_id, main, imported:active_imports, system, tools});
            }
            let versions = catalog.content_versions(&names.into_iter().collect::<Vec<_>>()).await
                .map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;
            if let Some(old) = &snapshot.skill_versions {
                for name in old.keys().filter(|name| !versions.contains_key(*name)) {
                    if !catalog.get(name).is_some_and(|s| s.metadata.is_role() || s.metadata.system_layer)
                        && !removed.iter().any(|r| r["name"] == *name) {
                        removed.push(json!({"name":name,"reason":"not_in_current_active_skills"}));
                    }
                }
            }
            let updated = versions.iter().filter(|(name,version)| snapshot.skill_versions.as_ref().and_then(|old|old.get(*name)) != Some(*version))
                .map(|(name,version)|json!({"name":name,"version":version})).collect::<Vec<_>>();
            Ok(SkillRecoveryPlan {agents, report:json!({"schema":"agent-runtime-conversation-restore-report/v1","skill_versions":versions,"updated_skills":updated,"removed_optional_skills":removed,"blocked_calls":[]})})
        })
    }

    pub(super) fn apply_recovered_skills(
        &self,
        conversation_id: &str,
        plan: SkillRecoveryPlan,
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        self.rt.block_on(async {
            manager
                .clear_recovery_caches(conversation_id)
                .await
                .map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;
            for agent in plan.agents {
                let cache = manager
                    .agent_cache(conversation_id, &agent.agent_id)
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                for (key, value) in [
                    (ai_assistant::context::keys::MAIN_SKILLS, json!(agent.main)),
                    (
                        ai_assistant::context::keys::IMPORTED_SKILLS,
                        json!(agent.imported),
                    ),
                    (
                        ai_assistant::context::keys::SYSTEM_SKILLS,
                        json!(agent.system),
                    ),
                    (
                        ai_assistant::context::keys::ACTIVE_TOOLS,
                        json!(agent.tools),
                    ),
                ] {
                    cache
                        .set(key, &value, None)
                        .await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                }
            }
            let cache = manager
                .default_agent_cache(conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            cache
                .set(RESTORE_REPORT, &plan.report, None)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            Ok(())
        })
    }

    pub fn conversation_restore_report(
        &self,
        conversation_id: &str,
    ) -> Result<Value, RuntimeError> {
        let manager = self.manager()?;
        self.rt.block_on(async {
            manager
                .default_agent_cache(conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?
                .get::<Value>(RESTORE_REPORT)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
                .map(|r| r.unwrap_or(Value::Null))
        })
    }

    pub(super) fn revalidate_recovered_calls(
        &self,
        conversation_id: &str,
        recovery: &mut super::recovery::RestoredConversationRecovery,
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let mut report = self.conversation_restore_report(conversation_id)?;
        if !report.is_object() {
            report = json!({"blocked_calls":[]});
        }
        let mut blocked = Vec::new();
        self.rt.block_on(async {
            let current=manager.cluster_snapshot(conversation_id).await.map_err(|e|RuntimeError::Internal(e.to_string()))?;
            let live=current.agents.iter().map(|a|a.agent_id.as_str()).collect::<BTreeSet<_>>();
            recovery.entry_states.retain(|(id,_)|live.contains(id.as_str()));
            recovery.paused_agents.retain(|id|live.contains(id.as_str()));
            let policy=manager.tool_permission_policy(conversation_id).await.map_err(|e|RuntimeError::Internal(e.to_string()))?;
            for plan in &mut recovery.execution_plans {
                if !live.contains(plan.agent_id.as_str()) {
                    for call_id in &plan.call_ids { blocked.push(json!({"agent_id":plan.agent_id,"call_id":call_id,"reason":"agent_definition_unavailable","retry_safe":false})); }
                    continue;
                }
                let cache=manager.agent_cache(conversation_id,&plan.agent_id).await.map_err(|e|RuntimeError::Internal(e.to_string()))?;
                let active=AssistantContext::get_active_tools(&cache).await.map_err(|e|RuntimeError::Internal(e.to_string()))?;
                for (index,command) in plan.tools.iter().enumerate() {
                    let call_id=&plan.call_ids[index];
                    let name=ai_assistant::tool_runner::parse_tool_command(command).0;
                    let metadata=ai_assistant::tool_runner::permission_metadata_in_registry(name,Some(&self.tool_registry));
                    let uncertain=plan.started_calls.contains(call_id) && !metadata.as_ref().is_ok_and(|m|m.effect==ai_assistant::permission::ToolEffect::ReadOnly);
                    let reason=if uncertain { Some("interrupted_outcome_unknown".to_string()) }
                        else if let Err(error)=&metadata {Some(format!("tool_unavailable: {error}"))}
                        else if metadata.as_ref().is_ok_and(|m|policy.mode_for(m.effect)==ai_assistant::ToolPermissionMode::Deny) {Some("current_permission_denied".into())}
                        else if !active.iter().any(|tool|tool==name) {Some("tool_not_in_current_skills".into())}
                        else {ai_assistant::tool_runner::validate_recovery_call(command,plan.structured_tools.get(index),&self.tool_registry).err()};
                    if let Some(reason)=reason {
                        blocked.push(json!({"agent_id":plan.agent_id,"call_id":call_id,"tool_name":name,"reason":reason,"retry_safe":false}));
                        plan.recovery_results.insert(call_id.clone(),ai_assistant::ToolResult {
                            command:command.clone(),success:false,error_code:if uncertain {-6}else{103},
                            to_ai:format!("Recovery did not execute '{name}' (call_id={call_id}): {reason}. Preserve the original history; do not automatically repeat this call. Verify external state before re-planning."),
                            result:json!({"status":if uncertain {"interrupted_unknown"} else {"recovery_blocked"},"reason":reason,"retry_safe":false,"source":"current_recovery_contract","effect":metadata.as_ref().ok().map(|m|json!(m.effect)).unwrap_or(json!("unknown"))}),
                        });
                    } else { plan.recovery_results.remove(call_id); }
                }
            }
            recovery.execution_plans.retain(|plan|live.contains(plan.agent_id.as_str()));
            report["blocked_calls"]=json!(blocked);
            manager.default_agent_cache(conversation_id).await.map_err(|e|RuntimeError::Internal(e.to_string()))?
                .set(RESTORE_REPORT,&report,None).await.map_err(|e|RuntimeError::Internal(e.to_string()))?;
            Ok::<_,RuntimeError>(())
        })?;
        self.append_conversation_log(conversation_id, "conversation_restored", report);
        Ok(())
    }
}
