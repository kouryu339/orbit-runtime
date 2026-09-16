//! Agent-owned plan state. Cache snapshots and agent_plan.set use CurrentPlan.
use crate::context::{AssistantContext, CurrentPlan, PlanStep};
use crate::events::{types as ev_types, PlanChangedPayload};
use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput, SimpleArgs};
use corework::define_operation;
use corework::error::FrameworkError;
use corework::event::BaseEvent;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

// A weak lock per cache serializes read/check/write without retaining agents.
fn plan_lock(ctx: &Context) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<usize, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let key = Arc::as_ptr(&ctx.cache) as *const () as usize;
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn validate_steps(steps: &[PlanStep]) -> Result<(), AIOutput> {
    let mut ids = HashSet::new();
    let mut running = 0;
    for step in steps {
        if step.id.trim().is_empty() || step.text.trim().is_empty() || !ids.insert(&step.id) {
            return Err(AIOutput::error(
                400,
                "Steps require unique non-empty id and non-empty text.".to_string(),
            ));
        }
        if !matches!(
            step.status.as_str(),
            "pending" | "in_progress" | "completed" | "blocked" | "canceled"
        ) {
            return Err(AIOutput::error(400, "Invalid step status.".to_string()));
        }
        running += usize::from(step.status == "in_progress");
    }
    if running > 1 {
        return Err(AIOutput::error(
            400,
            "Only one step may be in_progress.".to_string(),
        ));
    }
    Ok(())
}

fn parse_steps(args: &SimpleArgs) -> Result<Option<Vec<PlanStep>>, AIOutput> {
    args.get("steps")
        .map(|text| {
            let steps: Vec<PlanStep> = serde_json::from_str(text)
                .map_err(|e| AIOutput::error(400, format!("Invalid steps: {e}")))?;
            if steps.is_empty() {
                return Err(AIOutput::error(400, "steps cannot be empty.".to_string()));
            }
            validate_steps(&steps)?;
            Ok(steps)
        })
        .transpose()
}

fn check_revision(args: &SimpleArgs, plan: &CurrentPlan) -> Result<(), AIOutput> {
    // Legacy Markdown plans remain editable; new plans require optimistic concurrency.
    if plan.plan_id.is_empty() && args.get("revision").is_none() {
        return Ok(());
    }
    if args.get("plan_id") != Some(plan.plan_id.as_str())
        || args.get("revision").and_then(|v| v.parse::<u64>().ok()) != Some(plan.revision)
    {
        return Err(AIOutput::error(
            409,
            format!(
                "Plan changed. Use plan_id={} revision={}.",
                plan.plan_id, plan.revision
            ),
        ));
    }
    Ok(())
}

async fn commit(
    ctx: &Context,
    event_type: &str,
    plan: CurrentPlan,
) -> Result<AIOutput, FrameworkError> {
    AssistantContext::set_current_plan(&ctx.cache, &plan).await?;
    // The owning Agent's normal cache checkpoint persists this state. Never
    // write to the process-global default Agent's legacy SessionMeta field.
    let (agent_id, agent_name) = crate::agent::source_meta_from_cache(&*ctx.cache).await;
    let payload = PlanChangedPayload {
        agent_id,
        agent_name,
        plan: plan.clone(),
    };
    let event = BaseEvent::new(event_type, serde_json::to_value(payload)?);
    // State is committed even if a subscriber fails; snapshots repair projection.
    if let Err(error) = ctx.event_bus.publish(event).await {
        tracing::warn!(%error, plan_id = %plan.plan_id, revision = plan.revision, "plan committed; event delivery failed");
    }
    Ok(AIOutput::success(
        serde_json::to_value(&plan)?,
        format!(
            "Plan '{}' {}. plan_id={} revision={}.",
            plan.title, plan.status, plan.plan_id, plan.revision
        ),
    ))
}

#[define_operation(
    name = "PlanWrite", display_name = "建立计划{title}，目标{summary}，步骤{steps}，详情{content}", category = "Planning", system_only,
    description = "Create one agent-owned plan. Supply steps with id, text, status (pending/in_progress/completed/blocked/canceled). An active plan cannot be overwritten. Legacy Markdown content is accepted.",
    params {
        title: "String@Required plan title.",
        steps: "Array@Structured steps [{id,text,status}]. Required unless legacy content is supplied.",
        content: "String@Optional Markdown details, or legacy plan body.",
        summary: "String@Optional concise objective."
    },
    destructive = false, readonly = false, idempotent = false, open_world = false
)]
pub struct PlanWriteSystem;

#[async_trait]
impl SystemOperation for PlanWriteSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let title = match args.safe_require("title") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let steps = match parse_steps(&args) {
            Ok(v) => v.unwrap_or_default(),
            Err(e) => return Ok(e),
        };
        let content = args.get("content").unwrap_or("").to_string();
        if title.trim().is_empty() || (steps.is_empty() && content.trim().is_empty()) {
            return Ok(AIOutput::error(
                400,
                "Supply a title and steps (or legacy content).".to_string(),
            ));
        }
        let lock = plan_lock(ctx);
        let _guard = lock.lock().await;
        if let Some(plan) = AssistantContext::get_current_plan(&ctx.cache).await? {
            if plan.is_active() {
                return Ok(AIOutput::error(
                    409,
                    format!(
                        "Active plan already exists: {} revision={}. Use PlanUpdate or cancel it.",
                        plan.plan_id, plan.revision
                    ),
                ));
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        commit(
            ctx,
            ev_types::PLAN_WRITTEN,
            CurrentPlan {
                plan_id: format!("plan_{}", uuid::Uuid::new_v4()),
                revision: 1,
                steps,
                title,
                summary: args.get("summary").unwrap_or("").into(),
                content,
                status: CurrentPlan::STATUS_ACTIVE.into(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .await
    }
    fn name(&self) -> &str {
        "PlanWrite"
    }
}

#[define_operation(
    name = "PlanUpdate", display_name = "更新计划{plan_id}版本{revision}，标题{title}，摘要{summary}，步骤{steps}，单步{step_id}状态{step_status}，详情{content}", category = "Planning", system_only,
    description = "Update the current plan using its plan_id and revision. Patch one step with step_id/step_status, or replace steps to revise the plan. Preserve stable step ids.",
    params {
        plan_id: "String@Required current plan id.",
        revision: "u64@Required current revision.",
        step_id: "String@Optional step id to update.",
        step_status: "String@Optional pending/in_progress/completed/blocked/canceled.",
        steps: "Array@Optional complete revised steps [{id,text,status}].",
        title: "String@Optional revised title.",
        summary: "String@Optional revised objective.",
        content: "String@Optional revised Markdown details."
    },
    destructive = false, readonly = false, idempotent = false, open_world = false
)]
pub struct PlanUpdateSystem;

#[async_trait]
impl SystemOperation for PlanUpdateSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let steps = match parse_steps(&args) {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let lock = plan_lock(ctx);
        let _guard = lock.lock().await;
        let Some(mut plan) = AssistantContext::get_current_plan(&ctx.cache).await? else {
            return Ok(AIOutput::error(
                404,
                "No plan. Call PlanWrite first.".to_string(),
            ));
        };
        if let Err(e) = check_revision(&args, &plan) {
            return Ok(e);
        }
        if !plan.is_active() {
            return Ok(AIOutput::error(409, "Plan is already closed.".to_string()));
        }
        if let Some(steps) = steps {
            plan.steps = steps;
        }
        match (args.get("step_id"), args.get("step_status")) {
            (Some(id), Some(status)) => {
                let Some(step) = plan.steps.iter_mut().find(|s| s.id == id) else {
                    return Ok(AIOutput::error(404, "Unknown step id.".to_string()));
                };
                step.status = status.into();
            }
            (None, None) => {}
            _ => {
                return Ok(AIOutput::error(
                    400,
                    "Provide both step_id and step_status.".to_string(),
                ))
            }
        }
        if let Err(e) = validate_steps(&plan.steps) {
            return Ok(e);
        }
        if let Some(title) = args.get("title") {
            if title.trim().is_empty() {
                return Ok(AIOutput::error(400, "Title cannot be empty.".to_string()));
            }
            plan.title = title.into();
        }
        if let Some(summary) = args.get("summary") {
            plan.summary = summary.into();
        }
        if let Some(content) = args.get("content") {
            plan.content = content.into();
        }
        plan.revision += 1;
        plan.updated_at = chrono::Utc::now().to_rfc3339();
        commit(ctx, ev_types::PLAN_UPDATED, plan).await
    }
    fn name(&self) -> &str {
        "PlanUpdate"
    }
}

#[define_operation(
    name = "PlanFinish", display_name = "结束计划{plan_id}版本{revision}，状态{status}，说明{note}", category = "Planning", system_only,
    description = "Finish or explicitly cancel the current plan. Finishing requires all structured steps completed. The closed state remains available for audit and is no longer injected into context.",
    params {
        plan_id: "String@Required current plan id.", revision: "u64@Required current revision.",
        status: "String@Optional finished (default) or canceled.",
        note: "String@Optional completion or cancellation note."
    },
    destructive = false, readonly = false, idempotent = false, open_world = false
)]
pub struct PlanFinishSystem;

#[async_trait]
impl SystemOperation for PlanFinishSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;
    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let lock = plan_lock(ctx);
        let _guard = lock.lock().await;
        let Some(mut plan) = AssistantContext::get_current_plan(&ctx.cache).await? else {
            return Ok(AIOutput::error(404, "No current plan.".to_string()));
        };
        if let Err(e) = check_revision(&args, &plan) {
            return Ok(e);
        }
        if !plan.is_active() {
            return Ok(AIOutput::success(
                serde_json::to_value(&plan)?,
                "Plan already closed.".to_string(),
            ));
        }
        let status = args.get("status").unwrap_or("finished");
        if !matches!(status, "finished" | "canceled") {
            return Ok(AIOutput::error(
                400,
                "status must be finished or canceled.".to_string(),
            ));
        }
        if status == "finished" && plan.steps.iter().any(|s| s.status != "completed") {
            return Ok(AIOutput::error(
                409,
                "Complete every step before finishing, or explicitly cancel the plan.".to_string(),
            ));
        }
        plan.status = status.into();
        if status == "canceled" {
            for step in &mut plan.steps {
                if step.status != "completed" {
                    step.status = "canceled".to_string();
                }
            }
        }
        plan.revision += 1;
        plan.updated_at = chrono::Utc::now().to_rfc3339();
        if let Some(note) = args.get("note") {
            plan.content.push_str(&format!("\n\n{note}"));
        }
        commit(ctx, ev_types::PLAN_FINISHED, plan).await
    }
    fn name(&self) -> &str {
        "PlanFinish"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::cache::InMemoryCache;
    use corework::event::{EventHandler, InMemoryEventBus};
    use corework::monitoring::NoopTelemetry;

    #[derive(Default)]
    struct Capture(Mutex<Vec<serde_json::Value>>);
    #[async_trait]
    impl EventHandler for Capture {
        async fn handle(&self, event: &BaseEvent) -> corework::error::Result<()> {
            self.0.lock().unwrap().push(event.payload.clone());
            Ok(())
        }
    }
    fn context() -> Context {
        Context::new(
            Arc::new(InMemoryCache::new()),
            Arc::new(InMemoryEventBus::new()),
            Arc::new(NoopTelemetry),
        )
    }
    fn args(values: &[(&str, &str)]) -> AIInput {
        AIInput::from_args(
            values
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
    async fn create(ctx: &Context) -> CurrentPlan {
        let result = PlanWriteSystem
            .execute(
                args(&[
                    ("title", "Ship"),
                    (
                        "steps",
                        r#"[{"id":"s1","text":"Verify","status":"in_progress"}]"#,
                    ),
                ]),
                ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.error_code, 0, "{}", result.to_ai);
        serde_json::from_value(result.result).unwrap()
    }
    #[tokio::test]
    async fn plan_event_roundtrip_lifecycle_and_agent_isolation() {
        let ctx = context();
        let capture = Arc::new(Capture::default());
        ctx.event_bus
            .subscribe(ev_types::PLAN_WRITTEN.into(), capture.clone())
            .await
            .unwrap();
        let plan = create(&ctx).await;
        let emitted = capture.0.lock().unwrap()[0].clone();
        // Gateway puts this real event body into delta.plan; recovery reads CurrentPlan.
        let restored: CurrentPlan = serde_json::from_value(emitted).unwrap();
        assert_eq!(restored.created_at, plan.created_at);
        assert_eq!(restored.steps, plan.steps);
        let other = context();
        assert!(AssistantContext::get_current_plan(&other.cache)
            .await
            .unwrap()
            .is_none());
        // Cache checkpoint restores the very same state without SessionMeta.
        let dump = ctx.cache.dump_raw().await.unwrap();
        for (key, value) in dump {
            other.cache.set_raw(&key, value, None).await.unwrap();
        }
        assert_eq!(
            AssistantContext::get_current_plan(&other.cache)
                .await
                .unwrap()
                .unwrap()
                .plan_id,
            plan.plan_id
        );
        assert_eq!(
            PlanWriteSystem
                .execute(args(&[("title", "Overwrite"), ("content", "bad")]), &ctx)
                .await
                .unwrap()
                .error_code,
            409
        );
        let finish = args(&[("plan_id", &plan.plan_id), ("revision", "1")]);
        assert_eq!(
            PlanFinishSystem
                .execute(finish, &ctx)
                .await
                .unwrap()
                .error_code,
            409
        );
        let update = args(&[
            ("plan_id", &plan.plan_id),
            ("revision", "1"),
            ("step_id", "s1"),
            ("step_status", "completed"),
        ]);
        let (a, b) = tokio::join!(
            PlanUpdateSystem.execute(update.clone(), &ctx),
            PlanUpdateSystem.execute(update, &ctx)
        );
        let mut codes = [a.unwrap().error_code, b.unwrap().error_code];
        codes.sort();
        assert_eq!(codes, [0, 409]);
        let finished = PlanFinishSystem
            .execute(args(&[("plan_id", &plan.plan_id), ("revision", "2")]), &ctx)
            .await
            .unwrap();
        assert_eq!(finished.error_code, 0);
        assert!(!serde_json::from_value::<CurrentPlan>(finished.result)
            .unwrap()
            .is_active());
        assert!(AssistantContext::get_current_plan(&other.cache)
            .await
            .unwrap()
            .unwrap()
            .is_active());
    }

    #[tokio::test]
    async fn plan_cancel_and_invalid_steps() {
        let ctx = context();
        let plan = create(&ctx).await;
        let result = PlanFinishSystem
            .execute(
                args(&[
                    ("plan_id", &plan.plan_id),
                    ("revision", "1"),
                    ("status", "canceled"),
                ]),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.result["status"], "canceled");
        assert_eq!(result.result["steps"][0]["status"], "canceled");
        let bad = PlanWriteSystem.execute(args(&[("title","Bad"), ("steps",r#"[{"id":"a","text":"a","status":"pending"},{"id":"a","text":"b","status":"pending"}]"#)]), &ctx).await.unwrap();
        assert_eq!(bad.error_code, 400);
    }
}
