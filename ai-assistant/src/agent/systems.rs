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
        let (skills, tool_names) = match resolve_agent_skill_and_tools(&feature_skills).await {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };

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
            "report_policy": "Call ReportAgentTask exactly once when the task is complete, failed, or canceled."
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
            "You are assigned background task {task_id}.\n\nObjective:\n{objective}\n\nWhen finished, call ReportAgentTask with task_id={task_id}. Do not hand off focus."
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

        let driver = Arc::clone(&runtime);
        tokio::spawn(async move {
            if let Err(error) = driver.drive(None).await {
                tracing::warn!(
                    agent_id = %driver.id,
                    "background agent task drive failed: {}",
                    error
                );
            }
        });

        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "status": "running",
                "assignee_agent_id": agent_id,
                "assignee_agent_name": name.clone(),
                "profile": profile_id,
                "skills": skills,
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
    display_name = "报告任务{task_id}的状态{report_type}、摘要{summary}、结果{result}和产物{artifacts}",
    category = "Agent Collaboration",
    system_only,
    description = "Report the result of the background task assigned to the current agent.",
    params {
        task_id:     "Task id assigned by CreateBackgroundAgentTask.",
        report_type: "completed, failed, or canceled.",
        summary:     "Concise result summary for the delegator.",
        result:      "Optional JSON result payload.",
        artifacts:   "Optional comma-separated artifact list."
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
        if !matches!(report_type.as_str(), "completed" | "failed" | "canceled") {
            return Ok(AIOutput::error(
                400,
                "report_type must be completed, failed, or canceled.".to_string(),
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

        Ok(AIOutput::success(
            serde_json::json!({
                "task_id": task_id,
                "reporter_agent_id": reporter_agent_id,
                "report_type": report_type,
            }),
            format!("Reported task '{}' as {}.", task_id, report_type),
        ))
    }

    fn name(&self) -> &str {
        "ReportAgentTask"
    }
}
