use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::event::BaseEvent;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::context::{keys, AssistantContext};
use crate::skills::systems::mgr;
use crate::state_machine::{agent_keys, build_agent_state_machine};
use crate::systems::agent_route::{
    record_focus_change_if_needed, RouteAgentAppointmentInput, RouteAgentAppointmentSystem,
    RouteAgentReportInput, RouteAgentReportSystem,
};

fn require_conversation_shared_components(
    ctx: &Context,
    message: &str,
) -> Result<
    (
        Arc<crate::agent::cluster::AgentCluster>,
        Arc<corework::execution_unit::ExecutionUnit>,
    ),
    AIOutput,
> {
    let cluster = ctx
        .resolve_shared_component::<crate::agent::cluster::AgentCluster>()
        .map_err(|_| AIOutput::error(500, message.to_string()))?;
    let ledger = ctx
        .resolve_shared_component::<corework::execution_unit::ExecutionUnit>()
        .map_err(|_| AIOutput::error(500, message.to_string()))?;
    Ok((cluster, ledger))
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn requester_can_pause_assignee(
    tasks: &[crate::conversation_state::AgentTaskEntry],
    requester_id: &str,
    assignee_id: &str,
) -> bool {
    requester_id != assignee_id
        && tasks.iter().any(|task| {
            !task.status.is_terminal()
                && task.delegator_agent_id == requester_id
                && task.assignee_agent_id.as_deref() == Some(assignee_id)
        })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentResourceProfile {
    id: String,
    name: Option<String>,
    role: Option<String>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    retrieval: Option<crate::RetrievalConfig>,
}

impl AgentResourceProfile {
    fn skill_refs(&self) -> Vec<String> {
        let mut skills = Vec::new();
        if let Some(role) = self
            .role
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            skills.push(role.to_string());
        }
        for feature in &self.features {
            let feature = feature.trim();
            if !feature.is_empty() && !skills.iter().any(|skill| skill == feature) {
                skills.push(feature.to_string());
            }
        }
        skills
    }
}

async fn agent_resource_profile_by_name(
    ctx: &Context,
    name: &str,
) -> Result<Option<AgentResourceProfile>, FrameworkError> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let profiles = ctx
        .cache
        .get::<BTreeMap<String, AgentResourceProfile>>(keys::AGENT_RESOURCE_PROFILES)
        .await?
        .unwrap_or_default();
    if let Some(profile) = profiles.get(name) {
        return Ok(Some(profile.clone()));
    }
    Ok(profiles
        .values()
        .find(|profile| {
            profile
                .name
                .as_deref()
                .map(str::trim)
                .is_some_and(|profile_name| profile_name == name)
        })
        .cloned())
}

async fn resolve_agent_skill_and_tools(
    feature_skills: &[String],
) -> Result<(Vec<String>, Vec<String>), AIOutput> {
    if feature_skills.is_empty() {
        return Err(AIOutput::error(
            400,
            "At least one skill is required.".to_string(),
        ));
    }

    let main_skills_all = {
        let m = mgr().read().await;
        m.main_skill_names()
    };
    {
        let mut probe: Vec<String> = main_skills_all.clone();
        for skill in feature_skills {
            if !probe.contains(skill) {
                probe.push(skill.clone());
            }
        }
        let refs: Vec<&str> = probe.iter().map(|s| s.as_str()).collect();
        let mut mw = mgr().write().await;
        let _ = mw.load_many(&refs).await;
    }
    {
        let m = mgr().read().await;
        for skill_name in feature_skills {
            if m.get(skill_name).is_none() {
                return Err(AIOutput::error(
                    404,
                    format!("Skill '{}' does not exist.", skill_name),
                ));
            }
        }
    }

    let main_skills_capability: Vec<String> = {
        let m = mgr().read().await;
        main_skills_all
            .iter()
            .filter(|name| m.get(name).map(|s| !s.metadata.is_role()).unwrap_or(false))
            .cloned()
            .collect()
    };
    let mut skills = main_skills_capability;
    for skill in feature_skills {
        if !skills.contains(skill) {
            skills.push(skill.clone());
        }
    }

    {
        let m = mgr().read().await;
        let role_list: Vec<&str> = skills
            .iter()
            .filter(|name| m.get(name).map(|s| s.metadata.is_role()).unwrap_or(false))
            .map(|s| s.as_str())
            .collect();
        if role_list.len() != 1 {
            return Err(AIOutput::error(
                400,
                format!(
                    "skills contain {} role skills: {:?}. Exactly one role skill is required.",
                    role_list.len(),
                    role_list
                ),
            ));
        }
    }

    let tool_names = {
        let m = mgr().read().await;
        let refs: Vec<&str> = skills.iter().map(|s| s.as_str()).collect();
        m.collect_tools_for_skills(&refs)
    };
    Ok((skills, tool_names))
}

// ============================================================================
// CreateAgent
// ============================================================================

#[define_operation(
    name = "CreateAgent",
    display_name = "创建{name} Agent，类型{class}，意图{intent}，技能{skills}，工作流{workflow}，间隔{interval}",
    category = "Agent Collaboration",
    system_only,
    description = "Create a temporary OneShot agent with selected skills and run the requested intent.",
    params {
        name:      "Temporary agent name.",
        class:     "Execution class. Only oneshot is supported.",
        skills:    "Comma-separated skills to inject. Exactly one role skill is allowed.",
        workflow:  "Optional workflow name.",
        intent:    "Task intent for the created agent.",
        interval:  "Ignored compatibility parameter."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct CreateAgentSystem;

#[async_trait]
impl SystemOperation for CreateAgentSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let class_str = args.get_or("class", "oneshot");
        let skills_str = match args.safe_require("skills") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let workflow = args.get("workflow").map(|s| s.to_string());
        let intent = args.get_or("intent", "");
        if !matches!(class_str.to_lowercase().as_str(), "oneshot" | "one_shot") {
            return Ok(AIOutput::error(
                400,
                "CreateAgent only supports temporary OneShot agents.".to_string(),
            ));
        }
        let feature_skills: Vec<String> = skills_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if feature_skills.is_empty() {
            return Ok(AIOutput::error(
                400,
                "At least one skill is required.".to_string(),
            ));
        }

        let main_skills_all = {
            let m = mgr().read().await;
            m.main_skill_names()
        };

        {
            let mut probe: Vec<String> = main_skills_all.clone();
            for s in &feature_skills {
                if !probe.contains(s) {
                    probe.push(s.clone());
                }
            }
            let refs: Vec<&str> = probe.iter().map(|s| s.as_str()).collect();
            let mut mw = mgr().write().await;
            let _ = mw.load_many(&refs).await;
        }

        {
            let m = mgr().read().await;
            for skill_name in &feature_skills {
                if m.get(skill_name).is_none() {
                    return Ok(AIOutput::error(
                        404,
                        format!("Skill '{}' does not exist.", skill_name),
                    ));
                }
            }
        }

        let main_skills_capability: Vec<String> = {
            let m = mgr().read().await;
            for n in &main_skills_all {
                match m.get(n) {
                    Some(s) => tracing::info!(
                        "[main_skill_kind_debug] name={} kind={:?} is_role={}",
                        n,
                        s.metadata.kind,
                        s.metadata.is_role()
                    ),
                    None => tracing::warn!("[main_skill_kind_debug] name={} not loaded", n),
                }
            }
            main_skills_all
                .iter()
                .filter(|name| m.get(name).map(|s| !s.metadata.is_role()).unwrap_or(false))
                .cloned()
                .collect()
        };

        let skills: Vec<String> = {
            let mut all = main_skills_capability.clone();
            for s in &feature_skills {
                if !all.contains(s) {
                    all.push(s.clone());
                }
            }
            all
        };

        {
            let m = mgr().read().await;
            let role_list: Vec<&str> = skills
                .iter()
                .filter(|n| m.get(n).map(|s| s.metadata.is_role()).unwrap_or(false))
                .map(|s| s.as_str())
                .collect();
            if role_list.len() != 1 {
                return Ok(AIOutput::error(
                    400,
                    format!(
                        "skills contain {} role skills: {:?}. Exactly one role skill is required.",
                        role_list.len(),
                        role_list
                    ),
                ));
            }
        }

        let tool_names = {
            let m = mgr().read().await;
            let refs: Vec<&str> = skills.iter().map(|s| s.as_str()).collect();
            m.collect_tools_for_skills(&refs)
        };

        let agent_id = format!(
            "oneshot_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );

        let mut builder = build_agent_state_machine();
        if let Some(parent_unit) = ctx.execution_unit() {
            builder = builder.with_parent_unit(parent_unit);
        }
        let sm = Arc::new(
            builder
                .build()
                .await
                .map_err(|e| FrameworkError::SystemError(e.to_string()))?,
        );

        let cache = sm.unit().cache();
        cache.set(agent_keys::AGENT_ID, &agent_id, None).await?;
        cache.set(agent_keys::AGENT_NAME, &name, None).await?;
        cache
            .set(agent_keys::AGENT_CLASS, &class_str.to_lowercase(), None)
            .await?;

        if let Some(conversation_id) = crate::agent::conversation_id_from_cache(&*ctx.cache).await {
            crate::agent::set_conversation_id_in_cache(&*cache, &conversation_id).await?;
        }
        cache.set(keys::ACTIVE_TOOLS, &tool_names, None).await?;
        cache.set(keys::MAIN_SKILLS, &skills, None).await?;

        if let Some(ref wf) = workflow {
            cache.set("agent_workflow", wf, None).await?;
        }

        if !intent.is_empty() {
            use crate::context::AssistantContext;
            let event_bus = sm.unit().event_bus();
            AssistantContext::push_user_message_on_event_bus(&cache, &event_bus, &intent).await?;
        }

        if let Err(e) = sm.start().await {
            tracing::error!("temporary agent sm.start() failed: {}", e);
            return Ok(AIOutput::error(
                500,
                format!("temporary agent failed to start: {}", e),
            ));
        }

        let mut spins = 0u32;
        loop {
            let cur = sm.current_state();
            if cur == crate::state_machine::states::SAYING
                || cur == crate::state_machine::states::SUSPENDED
            {
                break;
            }
            if spins >= 6000 {
                tracing::warn!(
                    "temporary agent {} did not reach terminal state after 60s; current state={}",
                    agent_id,
                    cur
                );
                break;
            }
            if let Err(e) = sm.tick().await {
                tracing::error!("temporary agent {} tick failed: {}", agent_id, e);
                return Ok(AIOutput::error(
                    500,
                    format!("temporary agent execution failed: {}", e),
                ));
            }
            spins += 1;
            tokio::task::yield_now().await;
        }

        let response: String = cache
            .get(keys::PENDING_RESPONSE)
            .await?
            .unwrap_or_else(|| "Temporary agent completed without a text result.".to_string());

        tracing::info!(
            "CreateAgent[OneShot]: name={}, id={}, skills={:?}",
            name,
            agent_id,
            skills
        );

        Ok(AIOutput::success(
            serde_json::json!({
                "agent_id": agent_id,
                "name": name,
                "class": "oneshot",
                "skills": skills,
                "workflow": workflow,
                "result": response,
            }),
            response,
        ))
    }

    fn name(&self) -> &str {
        "CreateAgent"
    }
}

// ============================================================================
// AppointAgent
// ============================================================================

#[define_operation(
    name = "AppointAgent",
    display_name = "任命Agent {name}并发送任务{message}",
    category = "Agent Collaboration",
    system_only,
    description = "Switch focus to an existing persistent Agent by name or id and optionally send it a message.",
    params {
        name:    "Target persistent Agent name or id.",
        message: "Optional message or task to send to the target Agent."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct AppointAgentSystem;

#[async_trait]
impl SystemOperation for AppointAgentSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let message = args.get_or("message", "");

        let (cluster, ledger) = match require_conversation_shared_components(
            ctx,
            "Conversation is not initialized; cannot appoint Agent",
        ) {
            Ok(shared_components) => shared_components,
            Err(e) => return Ok(e),
        };

        let target_agent = cluster.find_by_name_or_id(&name).await.ok_or_else(|| {
            FrameworkError::SystemError(format!("target Agent not found: {}", name))
        })?;
        let (from_agent_id, from_agent_name) =
            crate::agent::source_meta_from_cache(&*ctx.cache).await;
        let route = match RouteAgentAppointmentSystem
            .execute(
                RouteAgentAppointmentInput {
                    cluster: Arc::clone(&cluster),
                    ledger: Arc::clone(&ledger),
                    payload: crate::events::AgentAppointRequestedPayload {
                        from_agent_id,
                        from_agent_name,
                        target: name.to_string(),
                        message: message.to_string(),
                    },
                },
                &ledger.create_context(),
            )
            .await
        {
            Ok(route) => route,
            Err(e) => return Ok(AIOutput::error(400, format!("appoint failed: {}", e))),
        };

        let active_id = target_agent.id.clone();
        let agent_name = cluster
            .get(&active_id)
            .await
            .map(|a| a.name.clone())
            .unwrap_or_else(|| active_id.clone());

        Ok(AIOutput::success(
            serde_json::json!({
                "active_agent_id": route.to_agent_id,
                "active_agent_name": route.to_agent_name,
                "focus_changed": route.focus_changed,
            }),
            format!("Appointed '{}' as the active agent.", agent_name),
        ))
    }

    fn name(&self) -> &str {
        "AppointAgent"
    }
}

// ============================================================================
// DismissAgent
// ============================================================================

#[define_operation(
    name = "DismissAgent",
    display_name = "解除Agent {name}",
    category = "Agent Collaboration",
    system_only,
    description = "Dismiss a child agent by name or id.",
    params {
        name: "Agent name or id to dismiss."
    },
    destructive = true,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct DismissAgentSystem;

#[async_trait]
impl SystemOperation for DismissAgentSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        let (cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(shared_components) => shared_components,
                Err(e) => return Ok(e),
            };

        match cluster.dismiss(&name).await {
            Ok(fallback_focus) => {
                if let Some(fallback_focus) = fallback_focus {
                    if let Err(error) =
                        record_focus_change_if_needed(&cluster, &ledger, fallback_focus, "dismiss")
                            .await
                    {
                        return Ok(AIOutput::error(500, error.to_string()));
                    }
                }
                Ok(AIOutput::success(
                    serde_json::json!({ "dismissed": name }),
                    format!("Dismissed agent '{}'.", name),
                ))
            }
            Err(e) => Ok(AIOutput::error(400, e.to_string())),
        }
    }

    fn name(&self) -> &str {
        "DismissAgent"
    }
}

// ============================================================================
// ListAgents
// ============================================================================

#[define_operation(
    name = "ListAgents",
    display_name = "列出当前Agent",
    category = "Agent Collaboration",
    system_only,
    description = "List all active agents.",
    params {},
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ListAgentsSystem;

#[async_trait]
impl SystemOperation for ListAgentsSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, _input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let (cluster, _) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(shared_components) => shared_components,
                Err(e) => return Ok(e),
            };

        let agents = cluster.list().await;

        if agents.is_empty() {
            return Ok(AIOutput::success(
                serde_json::json!({ "agents": [], "count": 0 }),
                "No active agents.".to_string(),
            ));
        }

        let summary = agents
            .iter()
            .map(|a| {
                let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let state = a.get("state").and_then(|v| v.as_str()).unwrap_or("");
                let kind = a.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                format!("- {} -> {} | {}", name, state, kind)
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(AIOutput::success(
            serde_json::json!({ "agents": agents, "count": agents.len() }),
            format!("{} active agents:\n{}", agents.len(), summary),
        ))
    }

    fn name(&self) -> &str {
        "ListAgents"
    }
}

// ============================================================================
// ReportToAgent
// ============================================================================

#[define_operation(
    name = "ReportToAgent",
    display_name = "向Agent {target}提交类型{report_type}的报告，原因{reason}、交接{handoff}、产物{artifacts}",
    category = "Agent Collaboration",
    system_only,
    description = "Report a result or handoff message to a target agent.",
    params {
        target:      "Target agent name or id.",
        report_type: "Report type: completed, need_help, or canceled.",
        reason:      "Report content.",
        artifacts:   "Optional comma-separated artifact list.",
        handoff:     "Whether to hand off focus to the target agent. Defaults to true."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct ReportToAgentSystem;

#[async_trait]
impl SystemOperation for ReportToAgentSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let target = match args.safe_require("target") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let report_type = match args.safe_require("report_type") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let reason = match args.safe_require("reason") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let artifacts_str = args.get("artifacts").unwrap_or("");
        let handoff = args.get("handoff").map(|v| v != "false").unwrap_or(true);

        // Only named child agents report back to another agent.
        let agent_id: String = ctx
            .cache
            .get(crate::state_machine::agent_keys::AGENT_ID)
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?
            .unwrap_or_default();

        let agent_name: String = ctx
            .cache
            .get(crate::state_machine::agent_keys::AGENT_NAME)
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?
            .unwrap_or_else(|| "Unknown Agent".to_string());

        if agent_id.is_empty() {
            return Ok(AIOutput::error(
                403,
                "ReportToAgent must be called from a named child agent.".to_string(),
            ));
        }

        let artifacts: Vec<String> = artifacts_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let report_text = match report_type.to_lowercase().as_str() {
            "completed" => {
                if artifacts.is_empty() {
                    reason.to_string()
                } else {
                    format!("{}\nArtifacts: {}", reason, artifacts.join(", "))
                }
            }
            "need_help" => format!("Need help: {}", reason),
            "canceled" => format!("[Canceled] {}", reason),
            other => {
                return Ok(AIOutput::error(
                    400,
                    format!(
                        "Unknown report_type: {}. Supported values: completed, need_help, canceled",
                        other
                    ),
                ))
            }
        };

        let (cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(shared_components) => shared_components,
                Err(e) => return Ok(e),
            };

        let target_agent = cluster.find_by_name_or_id(&target).await.ok_or_else(|| {
            FrameworkError::SystemError(format!("Report target agent '{}' not found.", target))
        })?;
        let target_id = target_agent.id.clone();
        let target_name = target_agent.name.clone();
        let route = match RouteAgentReportSystem
            .execute(
                RouteAgentReportInput {
                    cluster: Arc::clone(&cluster),
                    ledger: Arc::clone(&ledger),
                    payload: crate::events::AgentReportSubmittedPayload {
                        from_agent_id: agent_id.clone(),
                        from_agent_name: agent_name.clone(),
                        target: target.to_string(),
                        report_type: report_type.to_string(),
                        report: report_text,
                        handoff,
                    },
                },
                &ledger.create_context(),
            )
            .await
        {
            Ok(route) => route,
            Err(e) => return Ok(AIOutput::error(400, format!("Report failed: {}", e))),
        };

        tracing::info!(
            "ReportToAgent: agent='{}' ({}) -> target='{}' ({}) type={} reason={}",
            agent_name,
            agent_id,
            target_name,
            target_id,
            report_type,
            reason
        );

        Ok(AIOutput::success(
            serde_json::json!({
                "agent_id": agent_id,
                "agent_name": agent_name,
                "target_agent_id": target_id,
                "target_agent_name": target_name,
                "report_type": report_type,
                "focus_changed": route.focus_changed,
            }),
            format!("Reported to '{}' ({}).", target_name, report_type),
        ))
    }

    fn name(&self) -> &str {
        "ReportToAgent"
    }
}

// ============================================================================
// PauseAgent
// ============================================================================

#[define_operation(
    name = "PauseAgent",
    display_name = "Pause agent {agent_id} with mode {mode}",
    category = "Agent Collaboration",
    system_only,
    description = "Pause an agent assigned to a task delegated by the current agent. wait_for_tool preserves the current tool result; detach_tool returns an interrupted_unknown result without assuming the external operation stopped.",
    params {
        agent_id: "Exact target agent id shown in the delegated-task snapshot.",
        mode:     "wait_for_tool (default) or detach_tool."
    },
    destructive = false,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct PauseAgentSystem;

#[async_trait]
impl SystemOperation for PauseAgentSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let agent_id = match args.safe_require("agent_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let mode = match crate::agent::AgentPauseMode::parse(&args.get_or("mode", "wait_for_tool"))
        {
            Ok(mode) => mode,
            Err(error) => return Ok(AIOutput::error(400, error)),
        };
        let (cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        // The caller identity is control-plane context only. It is never
        // exposed as a pause target; target ids must come from delegated-task
        // snapshots.
        let requester_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let tasks = state.agent_tasks().await;
        if !requester_can_pause_assignee(&tasks, &requester_id, &agent_id) {
            return Ok(AIOutput::error(
                403,
                "PauseAgent only accepts an active assignee_agent_id from the caller's delegated-task snapshot."
                    .to_string(),
            ));
        }
        let target = match cluster.get(&agent_id).await {
            Some(target) => target,
            None => {
                return Ok(AIOutput::error(
                    404,
                    format!("Agent '{}' does not exist in this conversation.", agent_id),
                ));
            }
        };
        if let Ok(permission_broker) =
            ctx.resolve_shared_component::<crate::permission::PermissionBroker>()
        {
            permission_broker.cancel_agent(&agent_id).await;
        }
        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_PAUSE_REQUESTED,
                serde_json::to_value(crate::events::AgentPauseRequestedPayload {
                    agent_id: target.id.clone(),
                    agent_name: target.name.clone(),
                    mode,
                })?,
            ))
            .await?;
        Ok(AIOutput::success(
            serde_json::json!({
                "agent_id": target.id,
                "agent_name": target.name,
                "status": "pause_requested",
                "mode": mode.as_str(),
            }),
            format!(
                "Pause requested for agent '{}' using mode '{}'.",
                agent_id,
                mode.as_str()
            ),
        ))
    }

    fn name(&self) -> &str {
        "PauseAgent"
    }
}

// ============================================================================
// WaitAgentTask
// ============================================================================

const DEFAULT_AGENT_TASK_WAIT_MS: u64 = 30_000;
const MAX_AGENT_TASK_WAIT_MS: u64 = 300_000;

async fn wait_for_delegated_agent_task(
    state: &crate::conversation_state::ConversationState,
    requester_id: &str,
    task_id: &str,
    user_input_wakeup: tokio_util::sync::CancellationToken,
    timeout_ms: u64,
) -> Result<
    (
        String,
        crate::conversation_state::AgentTaskEntry,
        Option<crate::conversation_state::AgentTaskEntry>,
    ),
    AIOutput,
> {
    // Subscribe before the first read so a terminal transition between the
    // read and wait cannot be lost.
    let mut task_changes = state.subscribe_agent_tasks();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

    loop {
        let task = match state.agent_task(task_id).await {
            Some(task) => task,
            None => {
                return Err(AIOutput::error(
                    404,
                    format!("Delegated task '{}' does not exist.", task_id),
                ));
            }
        };
        if task.delegator_agent_id != requester_id
            || task.assignee_agent_id.as_deref() == Some(requester_id)
        {
            return Err(AIOutput::error(
                403,
                "WaitAgentTask only accepts a task created by the calling agent for another agent."
                    .to_string(),
            ));
        }
        if task.status.is_terminal() {
            return Ok(("task_terminal".to_string(), task, None));
        }
        if task.status == crate::conversation_state::AgentTaskStatus::Reported {
            return Ok(("task_reported".to_string(), task.clone(), Some(task)));
        }
        let attention_task = state.agent_tasks().await.into_iter().find(|candidate| {
            !candidate.status.is_terminal()
                && candidate.delegator_agent_id == requester_id
                && candidate.assignee_agent_id.as_deref() != Some(requester_id)
                && candidate.input_requests.iter().any(|request| {
                    request.status
                        == crate::conversation_state::AgentTaskInputRequestStatus::Pending
                })
        });
        if let Some(attention_task) = attention_task {
            return Ok(("input_requested".to_string(), task, Some(attention_task)));
        }
        let reported_task = state.agent_tasks().await.into_iter().find(|candidate| {
            candidate.status == crate::conversation_state::AgentTaskStatus::Reported
                && candidate.delegator_agent_id == requester_id
                && candidate.assignee_agent_id.as_deref() != Some(requester_id)
        });
        if let Some(reported_task) = reported_task {
            return Ok(("task_reported".to_string(), task, Some(reported_task)));
        }

        tokio::select! {
            biased;
            changed = task_changes.changed() => {
                if changed.is_err() {
                    return Err(AIOutput::error(500, "Delegated task notifications are unavailable."));
                }
            }
            _ = user_input_wakeup.cancelled() => {
                return Ok(("user_input".to_string(), task, None));
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Ok(("timeout".to_string(), task, None));
            }
        }
    }
}

#[define_operation(
    name = "WaitAgentTask",
    display_name = "Wait for delegated task {task_id} for up to {timeout_ms} milliseconds; revision {task_revision}, wake reason {wake_reason}, status {status}, assignee {assignee_agent_id}, attention task {attention_task_id} revision {attention_task_revision}, input request {input_request}, report {report}",
    category = "Agent Collaboration",
    system_only,
    description = "Wait for a task created by the current agent. Returns early when any task delegated by the caller requests input or submits a result for review, the waited task becomes terminal, or new user input arrives. Call this tool alone, not in a concurrent tool batch.",
    params {
        task_id:    "Exact task id returned by CreateBackgroundAgentTask.",
        timeout_ms: "Maximum wait in milliseconds. Optional; defaults to 30000 and is capped at 300000."
    },
    outputs {
        task_id:           "Delegated task id.",
        task_revision:     "Current task revision.",
        wake_reason:       "input_requested, task_reported, task_terminal, user_input, or timeout.",
        status:            "Current task status.",
        assignee_agent_id: "Current task assignee id when assigned.",
        attention_task_id: "Delegated task needing attention when wake_reason is input_requested or task_reported.",
        attention_task_revision: "Current revision of the delegated task needing attention.",
        input_request:     "Pending task input request when wake_reason is input_requested.",
        report:            "Candidate final report for attention_task_id when wake_reason is task_reported; otherwise the waited task report when available."
    },
    destructive = false,
    readonly = true,
    idempotent = false,
    open_world = false
)]
pub struct WaitAgentTaskSystem;

#[async_trait]
impl SystemOperation for WaitAgentTaskSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let pending_tools: Vec<String> = ctx
            .cache
            .get(keys::PENDING_TOOLS)
            .await?
            .unwrap_or_default();
        if pending_tools.len() != 1 {
            return Ok(AIOutput::error(
                400,
                "WaitAgentTask must be the only tool in its execution batch.".to_string(),
            ));
        }
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let timeout_ms = match args.get("timeout_ms") {
            Some(raw) => match raw.parse::<u64>() {
                Ok(0) => return Ok(AIOutput::error(400, "timeout_ms must be greater than zero")),
                Ok(value) => value.min(MAX_AGENT_TASK_WAIT_MS),
                Err(_) => return Ok(AIOutput::error(400, "timeout_ms must be an integer")),
            },
            None => DEFAULT_AGENT_TASK_WAIT_MS,
        };
        let (_cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let execution_control =
            match ctx.resolve_shared_component::<crate::agent::runtime::AgentExecutionControl>() {
                Ok(control) => control,
                Err(_) => {
                    return Ok(AIOutput::error(
                        500,
                        "Agent wait control is not initialized.".to_string(),
                    ));
                }
            };
        let wait_lease = execution_control.begin_agent_wait();
        let requester_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let (wake_reason, task, attention_task) = match wait_for_delegated_agent_task(
            &state,
            &requester_id,
            &task_id,
            wait_lease.token(),
            timeout_ms,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => return Ok(error),
        };
        let status = task.status.as_str().to_string();
        let attention_task_id = attention_task.as_ref().map(|task| task.task_id.clone());
        let attention_task_revision = attention_task.as_ref().map(|task| task.revision);
        let input_request = attention_task
            .as_ref()
            .unwrap_or(&task)
            .input_requests
            .iter()
            .find(|request| {
                request.status == crate::conversation_state::AgentTaskInputRequestStatus::Pending
            })
            .cloned();
        let report = attention_task.as_ref().unwrap_or(&task).report.clone();
        let summary = if let Some(attention_task_id) = attention_task_id.as_deref() {
            if wake_reason == "task_reported" {
                if attention_task_id == task_id {
                    format!(
                        "Agent task '{}' submitted a result for review and remains reported until you call CompleteAgentTask, UpdateAgentTask, or CancelAgentTask.",
                        task_id
                    )
                } else {
                    format!(
                        "Agent task '{}' submitted a result for review. Waiting for '{}' ended early; the original wait target remains {}.",
                        attention_task_id, task_id, status
                    )
                }
            } else {
                format!(
                    "Agent task '{}' needs input. Waiting for '{}' ended early; the original wait target remains {}.",
                    attention_task_id, task_id, status
                )
            }
        } else {
            format!(
                "Wait for delegated task '{}' ended by {} with status {}.",
                task_id, wake_reason, status
            )
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task.task_id,
                "task_revision": task.revision,
                "wake_reason": wake_reason,
                "status": status,
                "assignee_agent_id": task.assignee_agent_id,
                "assignee_agent_name": task.assignee_agent_name,
                "attention_task_id": attention_task_id,
                "attention_task_revision": attention_task_revision,
                "input_request": input_request,
                "report": report,
            }),
            summary,
        ))
    }

    fn name(&self) -> &str {
        "WaitAgentTask"
    }
}

#[cfg(test)]
mod agent_control_tests {
    use super::*;
    use crate::conversation_state::{
        AgentTaskEntry, AgentTaskInputRequestStatus, AgentTaskStatus, ConversationRequestHeaders,
        ConversationState,
    };

    fn task(status: AgentTaskStatus) -> AgentTaskEntry {
        AgentTaskEntry {
            task_id: "task-1".to_string(),
            revision: 1,
            title: "test".to_string(),
            objective: "test".to_string(),
            acceptance: Vec::new(),
            delegator_agent_id: "boss".to_string(),
            delegator_agent_name: "Boss".to_string(),
            assignee_agent_id: Some("worker".to_string()),
            assignee_agent_name: Some("Worker".to_string()),
            status,
            report: None,
            progress: Vec::new(),
            input_requests: Vec::new(),
            updates: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn pause_authorization_requires_active_direct_delegation() {
        assert!(requester_can_pause_assignee(
            &[task(AgentTaskStatus::Running)],
            "boss",
            "worker"
        ));
        assert!(!requester_can_pause_assignee(
            &[task(AgentTaskStatus::Completed)],
            "boss",
            "worker"
        ));
        assert!(!requester_can_pause_assignee(
            &[task(AgentTaskStatus::Running)],
            "other",
            "worker"
        ));
        let mut self_assigned = task(AgentTaskStatus::Running);
        self_assigned.assignee_agent_id = Some("boss".to_string());
        assert!(!requester_can_pause_assignee(
            &[self_assigned],
            "boss",
            "boss"
        ));
    }

    fn conversation_state() -> Arc<ConversationState> {
        Arc::new(ConversationState::new(
            "conversation-1",
            ConversationRequestHeaders::default(),
            "boss",
        ))
    }

    #[tokio::test]
    async fn wait_agent_task_returns_terminal_state_without_waiting() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Completed))
            .await;

        let (wake_reason, result, attention) = wait_for_delegated_agent_task(
            &state,
            "boss",
            "task-1",
            tokio_util::sync::CancellationToken::new(),
            1_000,
        )
        .await
        .expect("delegator should be allowed to wait");

        assert_eq!(wake_reason, "task_terminal");
        assert_eq!(result.status, AgentTaskStatus::Completed);
        assert!(attention.is_none());
    }

    #[tokio::test]
    async fn wait_agent_task_wakes_for_user_input() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let user_input = tokio_util::sync::CancellationToken::new();
        user_input.cancel();

        let (wake_reason, result, attention) =
            wait_for_delegated_agent_task(&state, "boss", "task-1", user_input, 1_000)
                .await
                .expect("delegator should be allowed to wait");

        assert_eq!(wake_reason, "user_input");
        assert_eq!(result.status, AgentTaskStatus::Running);
        assert!(attention.is_none());
    }

    #[tokio::test]
    async fn wait_agent_task_ignores_unrelated_task_changes() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let waiting_state = Arc::clone(&state);
        let mut wait = tokio::spawn(async move {
            wait_for_delegated_agent_task(
                &waiting_state,
                "boss",
                "task-1",
                tokio_util::sync::CancellationToken::new(),
                1_000,
            )
            .await
        });
        tokio::task::yield_now().await;

        let mut unrelated = task(AgentTaskStatus::Completed);
        unrelated.task_id = "task-2".to_string();
        state.upsert_agent_task(unrelated).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut wait)
                .await
                .is_err()
        );

        state
            .upsert_agent_task(task(AgentTaskStatus::Completed))
            .await;
        let (wake_reason, result, attention) = wait
            .await
            .expect("wait task should join")
            .expect("delegator should remain authorized");
        assert_eq!(wake_reason, "task_terminal");
        assert_eq!(result.task_id, "task-1");
        assert!(attention.is_none());
    }

    #[tokio::test]
    async fn wait_agent_task_rejects_non_delegator() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;

        let result = wait_for_delegated_agent_task(
            &state,
            "other",
            "task-1",
            tokio_util::sync::CancellationToken::new(),
            1,
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn wait_agent_task_wakes_for_pending_input_request() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let waiting_state = Arc::clone(&state);
        let wait = tokio::spawn(async move {
            wait_for_delegated_agent_task(
                &waiting_state,
                "boss",
                "task-1",
                tokio_util::sync::CancellationToken::new(),
                1_000,
            )
            .await
        });
        tokio::task::yield_now().await;

        state
            .request_agent_task_input(
                "task-1",
                "worker",
                "request-1",
                "Which target?",
                vec!["target".to_string()],
                true,
            )
            .await
            .expect("current assignee should be allowed to request input");

        let (wake_reason, result, attention) = wait
            .await
            .expect("wait task should join")
            .expect("delegator should remain authorized");
        assert_eq!(wake_reason, "input_requested");
        assert_eq!(result.status, AgentTaskStatus::Running);
        assert_eq!(attention.unwrap().input_requests[0].request_id, "request-1");
    }

    #[tokio::test]
    async fn wait_agent_task_wakes_for_another_delegated_tasks_input_request() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let mut other = task(AgentTaskStatus::Running);
        other.task_id = "task-2".to_string();
        state.upsert_agent_task(other).await;
        let waiting_state = Arc::clone(&state);
        let wait = tokio::spawn(async move {
            wait_for_delegated_agent_task(
                &waiting_state,
                "boss",
                "task-1",
                tokio_util::sync::CancellationToken::new(),
                1_000,
            )
            .await
        });
        tokio::task::yield_now().await;

        state
            .request_agent_task_input(
                "task-2",
                "worker",
                "request-2",
                "Need input for the other task",
                Vec::new(),
                true,
            )
            .await
            .unwrap();

        let (wake_reason, waited_task, attention) = wait
            .await
            .expect("wait task should join")
            .expect("delegator should remain authorized");
        assert_eq!(wake_reason, "input_requested");
        assert_eq!(waited_task.task_id, "task-1");
        assert_eq!(attention.unwrap().task_id, "task-2");
    }

    #[tokio::test]
    async fn wait_agent_task_wakes_for_another_tasks_report_after_registration() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let mut other = task(AgentTaskStatus::Running);
        other.task_id = "task-2".to_string();
        other.assignee_agent_id = Some("worker-2".to_string());
        other.assignee_agent_name = Some("Worker 2".to_string());
        state.upsert_agent_task(other).await;

        let waiting_state = Arc::clone(&state);
        let wait = tokio::spawn(async move {
            wait_for_delegated_agent_task(
                &waiting_state,
                "boss",
                "task-1",
                tokio_util::sync::CancellationToken::new(),
                1_000,
            )
            .await
        });
        tokio::task::yield_now().await;

        state
            .report_agent_task("task-2", "worker-2", candidate_report("completed"))
            .await
            .expect("second worker should submit its candidate result");

        let (wake_reason, waited_task, attention) = wait
            .await
            .expect("wait task should join")
            .expect("delegator should remain authorized");
        let attention = attention.expect("reported task should be returned for review");
        assert_eq!(wake_reason, "task_reported");
        assert_eq!(waited_task.task_id, "task-1");
        assert_eq!(waited_task.status, AgentTaskStatus::Running);
        assert_eq!(attention.task_id, "task-2");
        assert_eq!(attention.status, AgentTaskStatus::Reported);
        assert_eq!(
            attention
                .report
                .as_ref()
                .map(|report| report.summary.as_str()),
            Some("candidate result")
        );
    }

    #[tokio::test]
    async fn task_input_request_and_response_follow_current_task_relationship() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        assert!(state
            .request_agent_task_input(
                "task-1",
                "other",
                "request-denied",
                "Question",
                Vec::new(),
                true,
            )
            .await
            .is_err());

        state
            .request_agent_task_input(
                "task-1",
                "worker",
                "request-1",
                "Question",
                Vec::new(),
                true,
            )
            .await
            .expect("current assignee should be authorized");
        assert!(state
            .respond_agent_task_input("task-1", "request-1", "other", "Answer")
            .await
            .is_err());

        let (_, request) = state
            .respond_agent_task_input("task-1", "request-1", "boss", "Answer")
            .await
            .expect("delegator should be authorized");
        assert_eq!(request.status, AgentTaskInputRequestStatus::Answered);
        assert_eq!(request.answer.as_deref(), Some("Answer"));
        let task = state
            .set_agent_task_input_delivery("task-1", "request-1", "resumed")
            .await
            .expect("delivery status should remain attached to the request");
        assert_eq!(task.input_requests[0].delivery.as_deref(), Some("resumed"));
        assert!(state
            .respond_agent_task_input("task-1", "request-1", "boss", "Other")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn task_update_requires_delegator_and_increments_revision() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        assert!(state
            .update_agent_task("task-1", "other", "update-denied", "Changed", None, None,)
            .await
            .is_err());

        let (task, update) = state
            .update_agent_task(
                "task-1",
                "boss",
                "update-1",
                "Use the new target",
                Some("new objective".to_string()),
                Some(vec!["new acceptance".to_string()]),
            )
            .await
            .expect("delegator should be authorized");
        assert_eq!(task.revision, 2);
        assert_eq!(update.task_revision, 2);
        assert_eq!(task.objective, "new objective");
        assert_eq!(task.acceptance, vec!["new acceptance"]);
        let task = state
            .set_agent_task_update_delivery("task-1", "update-1", "queued_running")
            .await
            .expect("delivery status should remain attached to the update");
        assert_eq!(task.updates[0].delivery.as_deref(), Some("queued_running"));
    }

    fn candidate_report(report_type: &str) -> crate::conversation_state::AgentTaskReport {
        crate::conversation_state::AgentTaskReport {
            report_type: report_type.to_string(),
            summary: "candidate result".to_string(),
            result: serde_json::json!({"ok": true}),
            artifacts: Vec::new(),
            reported_at: "now".to_string(),
        }
    }

    #[tokio::test]
    async fn task_report_is_non_terminal_until_delegator_accepts_it() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let reported = state
            .report_agent_task("task-1", "worker", candidate_report("completed"))
            .await
            .expect("current assignee should submit a candidate result");
        assert_eq!(reported.status, AgentTaskStatus::Reported);
        assert!(!reported.status.is_terminal());

        let (wake_reason, waited, attention) = wait_for_delegated_agent_task(
            &state,
            "boss",
            "task-1",
            tokio_util::sync::CancellationToken::new(),
            1_000,
        )
        .await
        .unwrap();
        assert_eq!(wake_reason, "task_reported");
        assert_eq!(waited.status, AgentTaskStatus::Reported);
        assert_eq!(attention.unwrap().task_id, "task-1");

        assert!(state.complete_agent_task("task-1", "other").await.is_err());
        let completed = state
            .complete_agent_task("task-1", "boss")
            .await
            .expect("delegator should accept the report");
        assert_eq!(completed.status, AgentTaskStatus::Completed);
        assert!(completed.status.is_terminal());
    }

    #[tokio::test]
    async fn updating_reported_task_resumes_it_for_revision() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        state
            .report_agent_task("task-1", "worker", candidate_report("completed"))
            .await
            .unwrap();

        let (updated, _) = state
            .update_agent_task(
                "task-1",
                "boss",
                "revision-1",
                "Revise the conclusion",
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(updated.status, AgentTaskStatus::Running);
        assert!(
            updated.report.is_some(),
            "prior candidate remains auditable"
        );
    }

    #[tokio::test]
    async fn progress_stays_running_and_cancel_is_distinct_from_completion() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        let (progressed, progress) = state
            .report_agent_task_progress(
                "task-1",
                "worker",
                "progress-1",
                "research",
                "Research finished",
                serde_json::Value::Null,
                Vec::new(),
                Some("draft".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(progressed.status, AgentTaskStatus::Running);
        assert_eq!(progress.task_revision, 2);
        assert_eq!(progressed.progress.len(), 1);

        assert!(state.cancel_agent_task("task-1", "other").await.is_err());
        let canceled = state.cancel_agent_task("task-1", "boss").await.unwrap();
        assert_eq!(canceled.status, AgentTaskStatus::Canceled);
        assert_ne!(canceled.status, AgentTaskStatus::Completed);
    }

    #[tokio::test]
    async fn accepting_failed_report_finalizes_task_as_failed() {
        let state = conversation_state();
        state
            .upsert_agent_task(task(AgentTaskStatus::Running))
            .await;
        state
            .report_agent_task("task-1", "worker", candidate_report("failed"))
            .await
            .unwrap();
        let failed = state.complete_agent_task("task-1", "boss").await.unwrap();
        assert_eq!(failed.status, AgentTaskStatus::Failed);
    }
}

// ============================================================================
// RequestAgentTaskInput / RespondAgentTaskInput
// ============================================================================

#[define_operation(
    name = "RequestAgentTaskInput",
    display_name = "Request missing input for delegated task {task_id}: {question}; request {request_id}, blocking {blocking}, fields {required_fields}, status {status}",
    category = "Agent Collaboration",
    system_only,
    description = "Ask the delegator of the current background task for missing information. Routing is derived from task_id; no target Agent id is accepted. This is non-terminal and does not replace ReportAgentTask.",
    params {
        task_id:        "Task id assigned to the current background agent.",
        question:       "Concrete question for the task delegator.",
        required_fields:"Optional comma-separated names of required values.",
        blocking:       "Whether work on the task is blocked until the answer arrives. Optional; defaults to true."
    },
    outputs {
        task_id:        "Delegated task id.",
        request_id:     "Generated input request id.",
        blocking:       "Whether the request blocks further task work.",
        required_fields:"Requested value names.",
        status:         "pending when the request was accepted."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RequestAgentTaskInputSystem;

#[async_trait]
impl SystemOperation for RequestAgentTaskInputSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let question = match args.safe_require("question") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        if question.is_empty() {
            return Ok(AIOutput::error(400, "question must not be empty"));
        }
        let required_fields = split_csv(args.get("required_fields").unwrap_or(""));
        let blocking = args
            .get("blocking")
            .map(|_| args.get_bool("blocking"))
            .unwrap_or(true);
        let request_id = format!(
            "input_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let (_cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let requester_agent_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(
                404,
                format!("Delegated task '{}' does not exist.", task_id),
            ));
        };
        if task.status.is_terminal()
            || task.assignee_agent_id.as_deref() != Some(requester_agent_id.as_str())
        {
            return Ok(AIOutput::error(
                403,
                "RequestAgentTaskInput only accepts a non-terminal task currently assigned to the calling agent."
                    .to_string(),
            ));
        }

        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_INPUT_REQUESTED,
                serde_json::to_value(crate::events::AgentTaskInputRequestedPayload {
                    task_id: task_id.clone(),
                    request_id: request_id.clone(),
                    requester_agent_id: requester_agent_id.clone(),
                    question: question.clone(),
                    required_fields: required_fields.clone(),
                    blocking,
                })?,
            ))
            .await?;
        let accepted_task = state.agent_task(&task_id).await.filter(|task| {
            task.input_requests
                .iter()
                .any(|request| request.request_id == request_id)
        });
        let Some(accepted_task) = accepted_task else {
            return Ok(AIOutput::error(
                409,
                "The task input request was not accepted because the task changed.".to_string(),
            ));
        };

        let wake_result = async {
            let identity =
                crate::systems::agent_wake::AgentSystemInvocationIdentity::from_context(ctx)
                    .await?;
            crate::systems::agent_wake::invoke_wake_delegator_agent(
                crate::systems::agent_wake::WakeDelegatorAgentInput {
                    identity,
                    task_id: task_id.clone(),
                    reason: crate::systems::agent_wake::DelegatorWakeReason::InputRequested,
                    request_id: Some(request_id.clone()),
                    task_revision: accepted_task.revision,
                },
                ctx,
            )
            .await
        }
        .await;
        if let Err(error) = wake_result {
            // The request is already durable. A wake failure must remain visible
            // without pretending that the child request itself was rejected.
            tracing::warn!(
                task_id = %task_id,
                request_id = %request_id,
                caller_agent_id = %requester_agent_id,
                error = %error,
                "delegated-task input request was recorded but delegator wake failed"
            );
        }
        if blocking {
            ctx.cache
                .set(keys::PENDING_TOOLS_WAIT_FOR_INPUT, &true, None)
                .await?;
            ctx.cache
                .set(
                    keys::PENDING_TOOLS_STOP_REASON,
                    &"waiting".to_string(),
                    None,
                )
                .await?;
        }

        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "request_id": request_id,
                "blocking": blocking,
                "required_fields": required_fields,
                "status": "pending",
            }),
            "Requested missing input from the task delegator.".to_string(),
        ))
    }

    fn name(&self) -> &str {
        "RequestAgentTaskInput"
    }
}

#[define_operation(
    name = "RespondAgentTaskInput",
    display_name = "Respond to delegated task {task_id} input request {request_id} with {answer}; status {status}, delivery {delivery}",
    category = "Agent Collaboration",
    system_only,
    description = "Answer a pending input request for a task delegated by the current agent. The runtime resolves and injects the answer into the task's current assignee; no target Agent id is accepted.",
    params {
        task_id:   "Exact delegated task id.",
        request_id:"Exact pending request id returned by WaitAgentTask or shown in the delegated-task snapshot.",
        answer:    "Information or decision to inject into the assigned background agent."
    },
    outputs {
        task_id:   "Delegated task id.",
        request_id:"Answered request id.",
        status:    "answered when accepted.",
        delivery:  "Automatic delivery state for the assigned background agent."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RespondAgentTaskInputSystem;

#[async_trait]
impl SystemOperation for RespondAgentTaskInputSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let request_id = match args.safe_require("request_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let answer = match args.safe_require("answer") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        if answer.is_empty() {
            return Ok(AIOutput::error(400, "answer must not be empty"));
        }
        let (_cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let responder_agent_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(
                404,
                format!("Delegated task '{}' does not exist.", task_id),
            ));
        };
        let pending = task.input_requests.iter().any(|request| {
            request.request_id == request_id
                && request.status == crate::conversation_state::AgentTaskInputRequestStatus::Pending
        });
        if task.status.is_terminal() || task.delegator_agent_id != responder_agent_id || !pending {
            return Ok(AIOutput::error(
                403,
                "RespondAgentTaskInput only accepts a pending request from a non-terminal task delegated by the calling agent."
                    .to_string(),
            ));
        }

        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_INPUT_RESPONDED,
                serde_json::to_value(crate::events::AgentTaskInputRespondedPayload {
                    task_id: task_id.clone(),
                    request_id: request_id.clone(),
                    responder_agent_id,
                    answer,
                })?,
            ))
            .await?;
        let Some(updated_task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(409, "The delegated task disappeared."));
        };
        let Some(answered_request) = updated_task.input_requests.iter().find(|request| {
            request.request_id == request_id
                && request.status
                    == crate::conversation_state::AgentTaskInputRequestStatus::Answered
        }) else {
            return Ok(AIOutput::error(
                409,
                "The task input response was not accepted because the task changed.".to_string(),
            ));
        };
        let delivery = answered_request
            .delivery
            .as_deref()
            .unwrap_or("delivery_unknown");
        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "request_id": request_id,
                "status": "answered",
                "delivery": delivery,
            }),
            "Answered the delegated task input request and routed it to the assignee.".to_string(),
        ))
    }

    fn name(&self) -> &str {
        "RespondAgentTaskInput"
    }
}

#[define_operation(
    name = "UpdateAgentTask",
    display_name = "Update delegated task {task_id} with instruction {instruction}, objective {objective}, acceptance {acceptance}; update {update_id}, revision {task_revision}, delivery {delivery}",
    category = "Agent Collaboration",
    system_only,
    description = "Update a non-terminal task delegated by the current agent and automatically inject the change into that task's current assignee. Call once for each delegated task affected by a changed parent goal.",
    params {
        task_id:    "Exact delegated task id affected by the change.",
        instruction:"Concrete description of what changed and how the assignee should adapt.",
        objective:  "Optional replacement objective for this delegated task.",
        acceptance: "Optional replacement comma-separated acceptance checklist."
    },
    outputs {
        task_id:      "Updated delegated task id.",
        update_id:    "Generated update id.",
        task_revision:"New task revision.",
        delivery:     "Automatic delivery state for the assigned background agent."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct UpdateAgentTaskSystem;

#[async_trait]
impl SystemOperation for UpdateAgentTaskSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let instruction = match args.safe_require("instruction") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        if instruction.is_empty() {
            return Ok(AIOutput::error(400, "instruction must not be empty"));
        }
        let objective = args
            .get("objective")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let acceptance = args.get("acceptance").map(split_csv);
        let update_id = format!(
            "update_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let (_cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let updater_agent_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(
                404,
                format!("Delegated task '{}' does not exist.", task_id),
            ));
        };
        if task.status.is_terminal()
            || task.delegator_agent_id != updater_agent_id
            || task.assignee_agent_id.is_none()
        {
            return Ok(AIOutput::error(
                403,
                "UpdateAgentTask only accepts a non-terminal assigned task delegated by the calling agent."
                    .to_string(),
            ));
        }
        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_UPDATED,
                serde_json::to_value(crate::events::AgentTaskUpdatedPayload {
                    task_id: task_id.clone(),
                    update_id: update_id.clone(),
                    updater_agent_id,
                    instruction,
                    objective,
                    acceptance,
                })?,
            ))
            .await?;
        let Some(updated_task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(409, "The delegated task disappeared."));
        };
        let Some(update) = updated_task
            .updates
            .iter()
            .find(|update| update.update_id == update_id)
        else {
            return Ok(AIOutput::error(
                409,
                "The task update was not accepted because the task changed.".to_string(),
            ));
        };
        let delivery = update.delivery.as_deref().unwrap_or("delivery_unknown");
        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "update_id": update_id,
                "task_revision": update.task_revision,
                "delivery": delivery,
            }),
            "Updated the delegated task and routed the change to its assignee.".to_string(),
        ))
    }

    fn name(&self) -> &str {
        "UpdateAgentTask"
    }
}

// ============================================================================
// CompleteAgentTask / CancelAgentTask
// ============================================================================

#[define_operation(
    name = "CompleteAgentTask",
    display_name = "Accept reported delegated task {task_id}; status {status}, revision {task_revision}, retirement {retirement}",
    category = "Agent Collaboration",
    system_only,
    description = "Accept the current report for a task delegated by the calling agent, finalize it as completed or failed according to the report, and retire its background agent.",
    params {
        task_id: "Exact reported task id delegated by the current agent."
    },
    outputs {
        task_id: "Finalized delegated task id.",
        status: "completed or failed according to the accepted report.",
        task_revision: "Final task revision.",
        retirement: "retired or retirement_pending for the background agent."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct CompleteAgentTaskSystem;

#[async_trait]
impl SystemOperation for CompleteAgentTaskSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let (cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let completed_by_agent_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(
                404,
                format!("Delegated task '{}' does not exist.", task_id),
            ));
        };
        if task.delegator_agent_id != completed_by_agent_id
            || task.status != crate::conversation_state::AgentTaskStatus::Reported
        {
            return Ok(AIOutput::error(
                403,
                "CompleteAgentTask only accepts a reported task delegated by the calling agent."
                    .to_string(),
            ));
        }
        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_COMPLETED,
                serde_json::to_value(crate::events::AgentTaskCompletedPayload {
                    task_id: task_id.clone(),
                    completed_by_agent_id,
                })?,
            ))
            .await?;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(409, "The delegated task disappeared."));
        };
        if !task.status.is_terminal()
            || task.status == crate::conversation_state::AgentTaskStatus::Canceled
        {
            return Ok(AIOutput::error(409, "The reported task was not finalized."));
        }
        let retirement = match task.assignee_agent_id.as_deref() {
            Some(agent_id) if cluster.get(agent_id).await.is_some() => "retirement_pending",
            _ => "retired",
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task.task_id,
                "status": task.status.as_str(),
                "task_revision": task.revision,
                "retirement": retirement,
            }),
            format!(
                "Accepted task '{}' report and finalized it as {}.",
                task_id,
                task.status.as_str()
            ),
        ))
    }

    fn name(&self) -> &str {
        "CompleteAgentTask"
    }
}

#[define_operation(
    name = "CancelAgentTask",
    display_name = "Cancel delegated task {task_id} because {reason} with mode {mode}; status {status}, revision {task_revision}, retirement {retirement}",
    category = "Agent Collaboration",
    system_only,
    description = "Cancel a non-terminal task delegated by the calling agent and retire its current background agent. wait_for_tool preserves an in-flight tool result; detach_tool stops waiting and leaves the external outcome indeterminate.",
    params {
        task_id: "Exact non-terminal task id delegated by the current agent.",
        reason: "Concrete reason the delegated task is no longer required.",
        mode: "Optional wait_for_tool or detach_tool; defaults to wait_for_tool."
    },
    outputs {
        task_id: "Canceled delegated task id.",
        status: "canceled.",
        task_revision: "Final task revision.",
        retirement: "retired or retirement_pending for the background agent."
    },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct CancelAgentTaskSystem;

#[async_trait]
impl SystemOperation for CancelAgentTaskSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let reason = match args.safe_require("reason") {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            Ok(_) => return Ok(AIOutput::error(400, "reason must not be empty")),
            Err(error) => return Ok(error),
        };
        let mode = match crate::agent::AgentPauseMode::parse(&args.get_or("mode", "wait_for_tool"))
        {
            Ok(mode) => mode,
            Err(error) => return Ok(AIOutput::error(400, error)),
        };
        let (cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let canceled_by_agent_id = crate::agent::source_id_from_cache(&*ctx.cache).await;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(
                404,
                format!("Delegated task '{}' does not exist.", task_id),
            ));
        };
        if task.status.is_terminal() || task.delegator_agent_id != canceled_by_agent_id {
            return Ok(AIOutput::error(
                403,
                "CancelAgentTask only accepts a non-terminal task delegated by the calling agent."
                    .to_string(),
            ));
        }
        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_CANCELED,
                serde_json::to_value(crate::events::AgentTaskCanceledPayload {
                    task_id: task_id.clone(),
                    canceled_by_agent_id,
                    reason: reason.clone(),
                    mode,
                })?,
            ))
            .await?;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(409, "The delegated task disappeared."));
        };
        if task.status != crate::conversation_state::AgentTaskStatus::Canceled {
            return Ok(AIOutput::error(409, "The delegated task was not canceled."));
        }
        let retirement = match task.assignee_agent_id.as_deref() {
            Some(agent_id) if cluster.get(agent_id).await.is_some() => "retirement_pending",
            _ => "retired",
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task.task_id,
                "status": task.status.as_str(),
                "task_revision": task.revision,
                "retirement": retirement,
            }),
            format!("Canceled delegated task '{}': {}", task_id, reason),
        ))
    }

    fn name(&self) -> &str {
        "CancelAgentTask"
    }
}

// ============================================================================
// CreateBackgroundAgentTask
// ============================================================================

#[define_operation(
    name = "CreateBackgroundAgentTask",
    display_name = "为Agent {name}创建任务{task_id}，标题{title}、目标{objective}、验收{acceptance}",
    category = "Agent Collaboration",
    system_only,
    description = "Create a conversation-scoped delegated task, spawn a temporary background agent, and assign the task to it.",
    params {
        name:       "Registered agent profile id or profile display name.",
        title:      "Short task title.",
        objective:  "Concrete task objective for the background agent.",
        acceptance: "Optional comma-separated acceptance checklist.",
        task_id:    "Optional caller-provided task id."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct CreateBackgroundAgentTaskSystem;

#[async_trait]
impl SystemOperation for CreateBackgroundAgentTaskSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let requested_name = match args.safe_require("name") {
            Ok(v) => v.trim().to_string(),
            Err(e) => return Ok(e),
        };
        let Some(profile) = (match agent_resource_profile_by_name(ctx, &requested_name).await {
            Ok(profile) => profile,
            Err(error) => return Ok(AIOutput::error(500, error.to_string())),
        }) else {
            return Ok(AIOutput::error(
                404,
                format!("Agent profile '{}' does not exist.", requested_name),
            ));
        };
        let profile_id = profile.id.clone();
        let name = profile
            .name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| profile.id.clone());
        let objective = match args.safe_require("objective") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let title = args
            .get("title")
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| objective.chars().take(48).collect());
        let acceptance = split_csv(args.get("acceptance").unwrap_or(""));
        let task_id = args
            .get("task_id")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "task_{}",
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                )
            });

        let feature_skills = profile.skill_refs();
        if feature_skills.is_empty() {
            return Ok(AIOutput::error(
                400,
                format!(
                    "Agent profile '{}' must define a role or feature skills.",
                    profile_id
                ),
            ));
        }
        let (skills, mut tool_names) = match resolve_agent_skill_and_tools(&feature_skills).await {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        for tool in [
            "ReportAgentTask",
            "ReportAgentTaskProgress",
            "RequestAgentTaskInput",
        ] {
            if !tool_names.iter().any(|active| active == tool) {
                tool_names.push(tool.to_string());
            }
        }

        let (cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(shared_components) => shared_components,
                Err(e) => return Ok(e),
            };
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let (delegator_agent_id, delegator_agent_name) =
            crate::agent::source_meta_from_cache(&*ctx.cache).await;

        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_CREATED,
                serde_json::to_value(crate::events::AgentTaskCreatedPayload {
                    task_id: task_id.clone(),
                    title: title.clone(),
                    objective: objective.to_string(),
                    acceptance: acceptance.clone(),
                    delegator_agent_id: delegator_agent_id.clone(),
                    delegator_agent_name: delegator_agent_name.clone(),
                })?,
            ))
            .await?;

        let agent_id = format!(
            "bg_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let mut builder = build_agent_state_machine();
        if let Some(parent_unit) = ctx.execution_unit() {
            builder = builder.with_parent_unit(parent_unit);
        }
        let sm = Arc::new(
            builder
                .build()
                .await
                .map_err(|e| FrameworkError::SystemError(e.to_string()))?,
        );
        let cache = sm.unit().cache();
        cache.set(agent_keys::AGENT_ID, &agent_id, None).await?;
        cache.set(agent_keys::AGENT_NAME, &name, None).await?;
        cache
            .set(agent_keys::AGENT_CLASS, &"background".to_string(), None)
            .await?;
        if let Some(conversation_id) = crate::agent::conversation_id_from_cache(&*ctx.cache).await {
            crate::agent::set_conversation_id_in_cache(&*cache, &conversation_id).await?;
        }
        cache.set(keys::ACTIVE_TOOLS, &tool_names, None).await?;
        cache.set(keys::MAIN_SKILLS, &skills, None).await?;
        cache
            .set(
                keys::RETRIEVAL_CONFIG,
                &profile.retrieval.clone().unwrap_or_default(),
                None,
            )
            .await?;
        let task_contract = serde_json::json!({
            "task_id": task_id.clone(),
            "delegator_agent_id": delegator_agent_id.clone(),
            "delegator_agent_name": delegator_agent_name.clone(),
            "objective": objective,
            "acceptance": acceptance.clone(),
            "report_tool": "ReportAgentTask",
            "report_policy": "Call ReportAgentTask when a candidate final result is ready. This parks the task in reported until the delegator completes, updates, or cancels it.",
            "progress_tool": "ReportAgentTaskProgress",
            "progress_policy": "Use ReportAgentTaskProgress for durable stage progress that should not stop continued work.",
            "input_request_tool": "RequestAgentTaskInput",
            "input_request_policy": "When required information is missing, call RequestAgentTaskInput with this task_id. Do not guess or use ReportAgentTask for a non-terminal question. If blocking=true, stop the turn after requesting input and wait for the runtime-injected response."
        });
        let immutable_cache = BTreeMap::from([(
            "background_task_contract".to_string(),
            serde_json::to_string_pretty(&task_contract)
                .unwrap_or_else(|_| task_contract.to_string()),
        )]);
        cache
            .set(keys::IMMUTABLE_CACHE_ENTRIES, &immutable_cache, None)
            .await?;

        let task_prompt = format!(
            "You are assigned background task {task_id}.\n\nObjective:\n{objective}\n\nUse ReportAgentTaskProgress for durable stage progress and continue working afterward. If required information is missing, call RequestAgentTaskInput with task_id={task_id}; blocking=true parks you until the answer is injected. When a candidate final result is ready, call ReportAgentTask with task_id={task_id}; the runtime will park you while the delegator accepts, updates, or cancels the task. Do not hand off focus."
        );
        let event_bus = sm.unit().event_bus();
        AssistantContext::push_user_message_on_event_bus(&cache, &event_bus, &task_prompt).await?;
        sm.start()
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?;

        let runtime = Arc::new(crate::agent::AgentRuntime::new(
            agent_id.clone(),
            name.clone(),
            crate::agent::AgentKind::OneShot,
            Arc::clone(&sm),
            crate::agent::AgentPermissions {
                can_appoint: false,
                can_dismiss: false,
                allowed_report_targets: vec![delegator_agent_id.clone()],
                tools: tool_names.clone(),
                skills: skills.clone(),
            },
        ));
        runtime
            .set_conversation_id(state.conversation_id())
            .await
            .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
        cluster.register(Arc::clone(&runtime)).await;

        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_ASSIGNED,
                serde_json::to_value(crate::events::AgentTaskAssignedPayload {
                    task_id: task_id.clone(),
                    assignee_agent_id: agent_id.clone(),
                    assignee_agent_name: name.clone(),
                })?,
            ))
            .await?;

        // Once this agent delegates work it must retain deterministic wait and
        // control paths. Both tools enforce task ownership at execution time.
        let mut delegator_tools: Vec<String> =
            ctx.cache.get(keys::ACTIVE_TOOLS).await?.unwrap_or_default();
        let mut tools_changed = false;
        for tool in [
            "PauseAgent",
            "WaitAgentTask",
            "RespondAgentTaskInput",
            "UpdateAgentTask",
            "CompleteAgentTask",
            "CancelAgentTask",
        ] {
            if !delegator_tools.iter().any(|active| active == tool) {
                delegator_tools.push(tool.to_string());
                tools_changed = true;
            }
        }
        if tools_changed {
            ctx.cache
                .set(keys::ACTIVE_TOOLS, &delegator_tools, None)
                .await?;
        }

        cluster
            .schedule_agent_driver(Arc::clone(&runtime), false)
            .await
            .map_err(|error| FrameworkError::SystemError(error.to_string()))?;

        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "status": "running",
                "assignee_agent_id": agent_id,
                "assignee_agent_name": name.clone(),
                "profile": profile_id,
                "skills": skills,
                "pause_tool": "PauseAgent",
                "pause_modes": ["wait_for_tool", "detach_tool"],
                "wait_tool": "WaitAgentTask",
                "respond_input_tool": "RespondAgentTaskInput",
                "update_task_tool": "UpdateAgentTask",
                "complete_task_tool": "CompleteAgentTask",
                "cancel_task_tool": "CancelAgentTask",
            }),
            format!(
                "Background task '{}' was assigned to agent '{}'.",
                title, name
            ),
        ))
    }

    fn name(&self) -> &str {
        "CreateBackgroundAgentTask"
    }
}

// ============================================================================
// ReportAgentTask
// ============================================================================

#[define_operation(
    name = "ReportAgentTask",
    display_name = "Report task {task_id} as {report_type}: {summary}; result {result}, artifacts {artifacts}, reporter {reporter_agent_id}, status {status}, revision {task_revision}",
    category = "Agent Collaboration",
    system_only,
    description = "Submit a candidate final result for the background task assigned to the current agent. This changes the task to reported and parks the worker until its delegator completes, updates, or cancels the task.",
    params {
        task_id:     "Task id assigned by CreateBackgroundAgentTask.",
        report_type: "completed or failed candidate outcome.",
        summary:     "Concise result summary for the delegator.",
        result:      "Optional JSON result payload.",
        artifacts:   "Optional comma-separated artifact list."
    },
    outputs {
        task_id: "Reported delegated task id.",
        reporter_agent_id: "Current assigned background agent id.",
        report_type: "completed or failed candidate outcome.",
        status: "reported while awaiting delegator review.",
        task_revision: "Revision containing this report."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct ReportAgentTaskSystem;

#[async_trait]
impl SystemOperation for ReportAgentTaskSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let report_type = args.get_or("report_type", "completed").to_lowercase();
        if !matches!(report_type.as_str(), "completed" | "failed") {
            return Ok(AIOutput::error(
                400,
                "report_type must be completed or failed.".to_string(),
            ));
        }
        let summary = match args.safe_require("summary") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let result = args
            .get("result")
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .unwrap_or(serde_json::Value::Null);
        let artifacts = split_csv(args.get("artifacts").unwrap_or(""));

        let pending_tools: Vec<String> = ctx
            .cache
            .get(keys::PENDING_TOOLS)
            .await?
            .unwrap_or_default();
        if pending_tools.len() != 1 {
            return Ok(AIOutput::error(
                400,
                "ReportAgentTask must be the only tool in its execution batch.".to_string(),
            ));
        }
        let (_cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(shared_components) => shared_components,
                Err(e) => return Ok(e),
            };
        let (reporter_agent_id, reporter_agent_name) =
            crate::agent::source_meta_from_cache(&*ctx.cache).await;
        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_REPORTED,
                serde_json::to_value(crate::events::AgentTaskReportedPayload {
                    task_id: task_id.to_string(),
                    reporter_agent_id: reporter_agent_id.clone(),
                    reporter_agent_name: reporter_agent_name.clone(),
                    report_type: report_type.clone(),
                    summary: summary.to_string(),
                    result,
                    artifacts,
                })?,
            ))
            .await?;

        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let Some(reported_task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(409, "The delegated task disappeared."));
        };
        if reported_task.status != crate::conversation_state::AgentTaskStatus::Reported {
            return Ok(AIOutput::error(
                409,
                "The task report was not accepted because the task changed.".to_string(),
            ));
        }
        let wake_result = async {
            let identity =
                crate::systems::agent_wake::AgentSystemInvocationIdentity::from_context(ctx)
                    .await?;
            crate::systems::agent_wake::invoke_wake_delegator_agent(
                crate::systems::agent_wake::WakeDelegatorAgentInput {
                    identity,
                    task_id: task_id.to_string(),
                    reason: crate::systems::agent_wake::DelegatorWakeReason::TaskReported,
                    request_id: None,
                    task_revision: reported_task.revision,
                },
                ctx,
            )
            .await
        }
        .await;
        if let Err(error) = wake_result {
            tracing::warn!(
                task_id = %task_id,
                caller_agent_id = %reporter_agent_id,
                error = %error,
                "delegated-task report was recorded but delegator wake failed"
            );
        }
        ctx.cache
            .set(keys::PENDING_TOOLS_WAIT_FOR_INPUT, &true, None)
            .await?;
        ctx.cache
            .set(
                keys::PENDING_TOOLS_STOP_REASON,
                &"reported".to_string(),
                None,
            )
            .await?;

        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "reporter_agent_id": reporter_agent_id,
                "report_type": report_type,
                "status": "reported",
                "task_revision": reported_task.revision,
            }),
            format!(
                "Submitted task '{}' result as {}; it is awaiting delegator review.",
                task_id, report_type
            ),
        ))
    }

    fn name(&self) -> &str {
        "ReportAgentTask"
    }
}

#[define_operation(
    name = "ReportAgentTaskProgress",
    display_name = "Report task {task_id} stage {stage_id}: {summary}; result {result}, artifacts {artifacts}, progress {progress_id}, revision {task_revision}, status {status}, next {next_stage}",
    category = "Agent Collaboration",
    system_only,
    description = "Report durable non-terminal progress for the background task assigned to the current agent, then continue working. Use ReportAgentTask only for a candidate final result.",
    params {
        task_id: "Task id assigned by CreateBackgroundAgentTask.",
        stage_id: "Stable short identifier for the completed stage.",
        summary: "Concise stage result for the delegator.",
        result: "Optional JSON stage result payload.",
        artifacts: "Optional comma-separated artifact list.",
        next_stage: "Optional identifier or description of the next stage."
    },
    outputs {
        task_id: "Delegated task id.",
        progress_id: "Generated progress record id.",
        task_revision: "Revision containing the progress record.",
        status: "running.",
        next_stage: "Declared next stage when provided."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct ReportAgentTaskProgressSystem;

#[async_trait]
impl SystemOperation for ReportAgentTaskProgressSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let task_id = match args.safe_require("task_id") {
            Ok(value) => value.trim().to_string(),
            Err(error) => return Ok(error),
        };
        let stage_id = match args.safe_require("stage_id") {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            Ok(_) => return Ok(AIOutput::error(400, "stage_id must not be empty")),
            Err(error) => return Ok(error),
        };
        let summary = match args.safe_require("summary") {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            Ok(_) => return Ok(AIOutput::error(400, "summary must not be empty")),
            Err(error) => return Ok(error),
        };
        let result = args
            .get("result")
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .unwrap_or(serde_json::Value::Null);
        let artifacts = split_csv(args.get("artifacts").unwrap_or(""));
        let next_stage = args
            .get("next_stage")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let progress_id = format!(
            "progress_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let (_cluster, ledger) =
            match require_conversation_shared_components(ctx, "Conversation is not initialized") {
                Ok(components) => components,
                Err(error) => return Ok(error),
            };
        let (reporter_agent_id, reporter_agent_name) =
            crate::agent::source_meta_from_cache(&*ctx.cache).await;
        ledger
            .event_bus()
            .publish(BaseEvent::new(
                crate::events::types::AGENT_TASK_PROGRESS_REPORTED,
                serde_json::to_value(crate::events::AgentTaskProgressReportedPayload {
                    task_id: task_id.clone(),
                    progress_id: progress_id.clone(),
                    reporter_agent_id,
                    reporter_agent_name,
                    stage_id: stage_id.clone(),
                    summary,
                    result,
                    artifacts,
                    next_stage: next_stage.clone(),
                })?,
            ))
            .await?;
        let state = ledger
            .resolve_shared_component::<crate::conversation_state::ConversationState>()
            .ok_or_else(|| {
                FrameworkError::SystemError("ConversationState is not initialized".to_string())
            })?;
        let Some(task) = state.agent_task(&task_id).await else {
            return Ok(AIOutput::error(409, "The delegated task disappeared."));
        };
        let Some(progress) = task
            .progress
            .iter()
            .find(|item| item.progress_id == progress_id)
        else {
            return Ok(AIOutput::error(
                409,
                "The task progress report was not accepted.",
            ));
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "progress_id": progress_id,
                "task_revision": progress.task_revision,
                "status": task.status.as_str(),
                "next_stage": next_stage,
            }),
            format!(
                "Reported stage '{}' progress; the delegated task remains running.",
                stage_id
            ),
        ))
    }

    fn name(&self) -> &str {
        "ReportAgentTaskProgress"
    }
}
