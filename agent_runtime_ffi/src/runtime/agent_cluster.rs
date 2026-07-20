use super::*;

#[derive(Debug, Clone)]
pub struct RuntimeAgentCluster {
    pub id: String,
    pub description: String,
    pub focus_agent_id: String,
    pub agents: Vec<RuntimeAgentDefinition>,
    pub max_thinking_rounds: u32,
    pub permissions: ai_assistant::ToolPermissionPolicy,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentDefinition {
    pub id: String,
    pub profile_id: Option<String>,
    pub name: String,
    pub role: Option<String>,
    pub features: Vec<String>,
    pub model_uid: u32,
    pub retrieval: Option<RetrievalConfig>,
    pub system_prompt_constraints: SystemPromptConstraints,
    pub frontend_widgets_enabled: bool,
}

#[derive(Debug, Clone)]
pub(super) struct BuiltinClusterConfigs {
    pub(super) workflow_editor: RuntimeAgentCluster,
    pub(super) agent_test_supervisor: RuntimeAgentCluster,
    pub(super) agent_test_adversary: RuntimeAgentCluster,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct AgentClusterRegistration {
    schema: String,
    id: String,
    description: String,
    #[serde(alias = "focusAgentId")]
    focus_agent_id: Option<String>,
    agents: Vec<AgentClusterAgentRegistration>,
    #[serde(alias = "maxThinkingRounds")]
    max_thinking_rounds: u32,
    permissions: ai_assistant::ToolPermissionPolicy,
}

impl Default for AgentClusterRegistration {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-agent-cluster-registration/v1".to_string(),
            id: String::new(),
            description: String::new(),
            focus_agent_id: None,
            agents: Vec::new(),
            max_thinking_rounds: 0,
            permissions: ai_assistant::ToolPermissionPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AgentClusterAgentRegistration {
    id: String,
    profile: Option<String>,
    name: String,
    role: Option<String>,
    features: Vec<String>,
    #[serde(alias = "modelUid")]
    model_uid: Option<u32>,
    retrieval: Option<RetrievalConfig>,
    #[serde(alias = "systemPromptConstraints", default)]
    system_prompt_constraints: SystemPromptConstraints,
    #[serde(alias = "frontendWidgetsEnabled", default = "default_true")]
    frontend_widgets_enabled: bool,
}

impl Default for AgentClusterAgentRegistration {
    fn default() -> Self {
        Self {
            id: String::new(),
            profile: None,
            name: String::new(),
            role: None,
            features: Vec::new(),
            model_uid: None,
            retrieval: None,
            system_prompt_constraints: SystemPromptConstraints::default(),
            frontend_widgets_enabled: true,
        }
    }
}

fn default_true() -> bool {
    true
}

pub(super) fn build_agent_cluster_registry(
    mut registration: AgentClusterRegistration,
    registries: &RuntimeRegistries,
) -> Result<RuntimeAgentCluster, RuntimeError> {
    if registration.schema.trim().is_empty() {
        registration.schema = "agent-runtime-agent-cluster-registration/v1".to_string();
    }
    if registration.schema != "agent-runtime-agent-cluster-registration/v1" {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster registration schema '{}' is not supported",
            registration.schema
        )));
    }
    validate_resource_id("agent cluster id", &registration.id)?;
    registration.description = registration.description.trim().to_string();
    if registration.description.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' description must not be empty",
            registration.id
        )));
    }
    if registration.agents.is_empty() {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' agents must not be empty",
            registration.id
        )));
    }
    let default_model_uid = registries
        .llm
        .as_ref()
        .and_then(|llm| llm.current_model_uid);
    let requested_focus_agent_id = registration
        .focus_agent_id
        .take()
        .filter(|value| !value.trim().is_empty());
    let mut ids = std::collections::HashSet::new();
    let mut agents = Vec::new();
    let mut focus_aliases = Vec::<(String, String)>::new();
    for mut agent in registration.agents {
        let explicit_profile = agent
            .profile
            .as_deref()
            .map(str::trim)
            .filter(|profile| !profile.is_empty());
        let profile = if let Some(profile_ref) = explicit_profile {
            let profile =
                resolve_registered_agent_profile(registries, profile_ref).ok_or_else(|| {
                    RuntimeError::InvalidConfig(format!(
                        "agent cluster '{}' references unknown agent profile '{}'",
                        registration.id, profile_ref
                    ))
                })?;
            Some(profile)
        } else {
            resolve_registered_agent_profile(registries, &agent.name)
                .or_else(|| resolve_registered_agent_profile(registries, &agent.id))
        };
        let profile_id = profile.map(|profile| profile.id.clone());
        if let Some(profile) = profile {
            apply_registered_agent_profile(&registration.id, &mut agent, profile)?;
        }
        let id = agent.id.trim().to_string();
        validate_resource_id("agent id", &id)?;
        if !ids.insert(id.clone()) {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' agent id '{}' is duplicated",
                registration.id, id
            )));
        }
        let model_uid = agent.model_uid.or(default_model_uid).unwrap_or(0);
        if model_uid != 0 && key_store::get(model_uid).is_none() {
            return Err(RuntimeError::InvalidConfig(format!(
                "agent cluster '{}' agent '{}' model_uid {} is not registered",
                registration.id, id, model_uid
            )));
        }
        validate_registered_agent_retrieval(
            &registration.id,
            &id,
            agent.retrieval.as_ref(),
            registries.resources.as_ref(),
        )?;
        let features = normalize_string_list(&format!("agent '{}'.features", id), agent.features)?;
        let role = agent
            .role
            .map(|role| role.trim().to_string())
            .filter(|role| !role.is_empty());
        let name = if agent.name.trim().is_empty() {
            id.clone()
        } else {
            agent.name.trim().to_string()
        };
        focus_aliases.push((id.clone(), id.clone()));
        focus_aliases.push((name.clone(), id.clone()));
        if let Some(profile_id) = profile_id.as_ref() {
            focus_aliases.push((profile_id.clone(), id.clone()));
        }
        agents.push(RuntimeAgentDefinition {
            id,
            profile_id,
            name,
            role,
            features,
            model_uid,
            retrieval: agent.retrieval,
            system_prompt_constraints: effective_system_prompt_constraints(
                agent.frontend_widgets_enabled,
                &agent.system_prompt_constraints,
            ),
            frontend_widgets_enabled: effective_frontend_widgets_enabled(
                agent.frontend_widgets_enabled,
                &agent.system_prompt_constraints,
            ),
        });
    }
    let requested_focus_agent_id = requested_focus_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let focus_agent_id = match requested_focus_agent_id {
        Some(focus) => resolve_cluster_focus_agent_id(&registration.id, focus, &focus_aliases)?,
        None => agents[0].id.clone(),
    };
    if !ids.contains(focus_agent_id.trim()) {
        return Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' focus_agent_id '{}' was not found",
            registration.id, focus_agent_id
        )));
    }
    Ok(RuntimeAgentCluster {
        id: registration.id,
        description: registration.description,
        focus_agent_id: focus_agent_id.trim().to_string(),
        agents,
        max_thinking_rounds: registration.max_thinking_rounds,
        permissions: registration.permissions,
    })
}

fn resolve_cluster_focus_agent_id(
    cluster_id: &str,
    focus: &str,
    focus_aliases: &[(String, String)],
) -> Result<String, RuntimeError> {
    let mut matches = focus_aliases
        .iter()
        .filter(|(alias, _)| alias == focus)
        .map(|(_, agent_id)| agent_id.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [agent_id] => Ok(agent_id.clone()),
        [] => Ok(focus.to_string()),
        _ => Err(RuntimeError::InvalidConfig(format!(
            "agent cluster '{}' focus_agent_id '{}' matches multiple agents; use a concrete agent id",
            cluster_id, focus
        ))),
    }
}

fn resolve_registered_agent_profile<'a>(
    registries: &'a RuntimeRegistries,
    name: &str,
) -> Option<&'a ResourceAgentProfileConfig> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let resources = registries.resources.as_ref()?;
    resources.agent_profiles.get(name).or_else(|| {
        resources.agent_profiles.values().find(|profile| {
            profile
                .name
                .as_deref()
                .map(str::trim)
                .is_some_and(|profile_name| profile_name == name)
        })
    })
}

fn apply_registered_agent_profile(
    cluster_id: &str,
    agent: &mut AgentClusterAgentRegistration,
    profile: &ResourceAgentProfileConfig,
) -> Result<(), RuntimeError> {
    if agent.id.trim().is_empty() {
        agent.id = profile.id.clone();
    }
    let explicit_name = agent.name.trim().to_string();
    if explicit_name.is_empty() || explicit_name == profile.id {
        agent.name = profile.name.clone().unwrap_or_else(|| profile.id.clone());
    }
    if agent
        .role
        .as_ref()
        .map(|role| role.trim().is_empty())
        .unwrap_or(true)
    {
        agent.role = profile.role.clone();
    }
    if agent.features.is_empty() {
        agent.features = profile.features.clone();
    } else {
        let mut features = profile.features.clone();
        for feature in &agent.features {
            if !features.iter().any(|existing| existing == feature) {
                features.push(feature.clone());
            }
        }
        agent.features = features;
    }
    if agent.model_uid.is_none() {
        agent.model_uid = profile.model_uid;
    }
    if agent.retrieval.is_none() {
        agent.retrieval = profile.retrieval.clone();
    }
    if agent
        .system_prompt_constraints
        .frontend_widgets_enabled
        .is_none()
    {
        agent.system_prompt_constraints.frontend_widgets_enabled =
            profile.system_prompt_constraints.frontend_widgets_enabled;
    }
    if agent.frontend_widgets_enabled {
        agent.frontend_widgets_enabled = profile.frontend_widgets_enabled;
    }
    let _ = cluster_id;
    Ok(())
}

pub(super) fn build_builtin_cluster_configs(
    registries: &RuntimeRegistries,
) -> Result<BuiltinClusterConfigs, RuntimeError> {
    let builtin_model_uid = registries
        .llm
        .as_ref()
        .and_then(|llm| llm.current_model_uid);
    let cluster = |id: &str, description: &str, agent_id: &str, agent_name: &str, role: &str| {
        let registration = AgentClusterRegistration {
            id: id.to_string(),
            description: description.to_string(),
            focus_agent_id: Some(agent_id.to_string()),
            agents: vec![AgentClusterAgentRegistration {
                id: agent_id.to_string(),
                name: agent_name.to_string(),
                role: Some(role.to_string()),
                model_uid: builtin_model_uid,
                ..AgentClusterAgentRegistration::default()
            }],
            ..AgentClusterRegistration::default()
        };
        if builtin_model_uid.is_some() {
            build_agent_cluster_registry(registration, registries)
        } else {
            // Legacy runtime configs may start without an LLM registration.
            // Keep the built-in cluster formal and defer model resolution until use.
            Ok(RuntimeAgentCluster {
                id: registration.id,
                description: registration.description,
                focus_agent_id: agent_id.to_string(),
                agents: vec![RuntimeAgentDefinition {
                    id: agent_id.to_string(),
                    profile_id: None,
                    name: agent_name.to_string(),
                    role: Some(role.to_string()),
                    features: Vec::new(),
                    model_uid: 0,
                    retrieval: None,
                    system_prompt_constraints: SystemPromptConstraints {
                        frontend_widgets_enabled: Some(true),
                    },
                    frontend_widgets_enabled: true,
                }],
                max_thinking_rounds: 0,
                permissions: registration.permissions,
            })
        }
    };

    Ok(BuiltinClusterConfigs {
        workflow_editor: cluster(
            "workflow-studio",
            "Workflow Studio editor conversation",
            "workflow-studio-editor",
            "Workflow Studio Editor",
            crate::workflow_studio::WORKFLOW_EDITOR_ROLE_SKILL,
        )?,
        agent_test_supervisor: cluster(
            "agent-test-supervisor",
            "Agent Test Studio supervisor conversation",
            "agent-test-supervisor",
            "Agent Test Supervisor",
            "agent_test_supervisor",
        )?,
        agent_test_adversary: cluster(
            "agent-test-adversary",
            "Adversarial actor conversation for Agent Test",
            "agent-test-adversary",
            "Agent Test Adversary",
            "agent_test_adversary",
        )?,
    })
}

pub(super) fn cluster_focus_agent_section(
    cluster: &RuntimeAgentCluster,
) -> Result<AgentSection, RuntimeError> {
    let agent = cluster
        .agents
        .iter()
        .find(|agent| agent.id == cluster.focus_agent_id)
        .ok_or_else(|| {
            RuntimeError::InvalidConfig(format!(
                "built-in cluster '{}' focus agent '{}' was not found",
                cluster.id, cluster.focus_agent_id
            ))
        })?;
    Ok(runtime_agent_definition_to_agent_section(agent))
}

pub(super) fn normalize_string_list(
    label: &str,
    values: Vec<String>,
) -> Result<Vec<String>, RuntimeError> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        validate_resource_id(label, &value)?;
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

pub(super) fn effective_frontend_widgets_enabled(
    legacy_frontend_widgets_enabled: bool,
    constraints: &SystemPromptConstraints,
) -> bool {
    constraints
        .frontend_widgets_enabled
        .unwrap_or(legacy_frontend_widgets_enabled)
}

pub(super) fn effective_system_prompt_constraints(
    legacy_frontend_widgets_enabled: bool,
    constraints: &SystemPromptConstraints,
) -> SystemPromptConstraints {
    SystemPromptConstraints {
        frontend_widgets_enabled: Some(effective_frontend_widgets_enabled(
            legacy_frontend_widgets_enabled,
            constraints,
        )),
    }
}

pub(super) fn runtime_agent_definition_to_agent_section(
    agent: &RuntimeAgentDefinition,
) -> AgentSection {
    let model = if agent.model_uid == 0 {
        "default".to_string()
    } else {
        key_store::get(agent.model_uid)
            .map(|entry| entry.model_name)
            .unwrap_or_else(|| agent.model_uid.to_string())
    };
    AgentSection {
        id: agent.id.clone(),
        name: agent.name.clone(),
        is_default: true,
        model,
        max_tokens: 4096,
        skills_dir: PathBuf::from("skills"),
        role: agent.role.clone(),
        features: agent.features.clone(),
        retrieval: agent.retrieval.clone(),
        system_prompt_constraints: agent.system_prompt_constraints.clone(),
        frontend_widgets_enabled: agent.frontend_widgets_enabled,
    }
}

pub(super) fn resolve_studio_target_agent(
    config: &RuntimeConfig,
    registries: &RuntimeRegistries,
    requested_agent_id: &str,
) -> Result<AgentSection, RuntimeError> {
    let requested = requested_agent_id.trim();
    if requested.is_empty() {
        return Ok(config.default_agent().clone());
    }
    if let Some(agent) = config.agents.iter().find(|agent| agent.id == requested) {
        return Ok(agent.clone());
    }

    let mut matches = registries
        .agent_clusters
        .values()
        .flat_map(|cluster| {
            cluster
                .agents
                .iter()
                .filter(move |agent| {
                    agent.id == requested || agent.profile_id.as_deref() == Some(requested)
                })
                .map(move |agent| (cluster.id.as_str(), agent))
        })
        .collect::<Vec<_>>();
    matches
        .sort_by(|left, right| (left.0, left.1.id.as_str()).cmp(&(right.0, right.1.id.as_str())));

    match matches.as_slice() {
        [(_, agent)] => Ok(runtime_agent_definition_to_agent_section(agent)),
        [] => Err(RuntimeError::InvalidConfig(format!(
            "Studio agent_id '{}' was not found as a runtime config agent id, registered cluster agent/focus id, or bound profile id",
            requested
        ))),
        many => {
            let candidates = many
                .iter()
                .map(|(cluster_id, agent)| format!("{}:{}", cluster_id, agent.id))
                .collect::<Vec<_>>()
                .join(", ");
            Err(RuntimeError::InvalidConfig(format!(
                "Studio agent_id '{}' is ambiguous across registered cluster agents: {}; use a concrete unique agent id",
                requested, candidates
            )))
        }
    }
}
