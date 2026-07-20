//! 工具仅写入状态机 cache key [`crate::context::keys::CURRENT_PLAN`]，
//! 并发送计划事件给前端。
//! ## 工具列表
//! | 工具 | 功能 |
//! |------|------|
//! | `PlanWrite`  | 首次建立当前计划（全量写入） |
//! | `PlanUpdate` | 覆盖当前计划内容（保留 created_at） |
//! | `PlanFinish` | 标记当前计划完成（status=finished） |
//! **不提供** `PlanRead`——thinking 会自动把 active 计划注入 prompt，模型不需要主动读。
//! ## 事件
//! 每次工具执行成功后同时发 `plan:written` / `plan:updated` / `plan:finished`，
//! 前端据此实时刷新计划面板、在节点完成时做高亮或归档动画。

use async_trait::async_trait;

use corework::ai_system::{AIInput, AIOutput};
use corework::define_operation;
use corework::error::FrameworkError;
use corework::event::BaseEvent;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use crate::context::{AssistantContext, CurrentPlan};
use crate::events::{types as ev_types, PlanChangedPayload};

// ============================================================================
// 辅助工具
// ============================================================================

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

async fn publish_plan_event(
    ctx: &Context,
    event_type: &str,
    plan: &CurrentPlan,
) -> Result<(), FrameworkError> {
    let (agent_id, agent_name) = crate::agent::source_meta_from_cache(&*ctx.cache).await;
    let payload = PlanChangedPayload {
        agent_id,
        agent_name,
        title: plan.title.clone(),
        summary: plan.summary.clone(),
        content: plan.content.clone(),
        status: plan.status.clone(),
        updated_at: plan.updated_at.clone(),
    };
    let value = serde_json::to_value(&payload).unwrap_or(serde_json::json!({}));
    ctx.event_bus
        .publish(BaseEvent::new(event_type, value))
        .await
}

// ============================================================================
// PlanWrite —— 首次建立当前执行计划
// ============================================================================

#[define_operation(
    name = "PlanWrite",
    display_name = "建立标题{title}、摘要{summary}和正文{content}的执行计划",
    category = "Planning",
    system_only,
    description = "建立当前执行计划：\
将计划全量写入状态机内部（cache key: current_plan）。\
建立后模型必须按该计划执行，除非用户明确变更目标。",
    params {
        title: "计划标题（一句话，必填，供前端面板展示）",
        content: "完整计划 Markdown 正文（必填，建议包含：目标、分步骤、预估步骤数）",
        summary: "计划摘要（可选，供前端 badge / 通知）"
    },
    destructive = false,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct PlanWriteSystem;

#[async_trait]
impl SystemOperation for PlanWriteSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let title = match args.safe_require("title") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let content = match args.safe_require("content") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let summary = args.get("summary").unwrap_or("").to_string();

        let now = now_rfc3339();
        let plan = CurrentPlan {
            title: title.clone(),
            summary: summary.clone(),
            content: content.clone(),
            status: CurrentPlan::STATUS_ACTIVE.to_string(),
            created_at: now.clone(),
            updated_at: now,
        };

        AssistantContext::set_current_plan(&ctx.cache, &plan).await?;
        crate::persistence::save_default_agent_current_plan(Some(plan.clone()))
            .await
            .map_err(|e| FrameworkError::SystemError(format!("保存当前计划失败: {}", e)))?;
        publish_plan_event(ctx, ev_types::PLAN_WRITTEN, &plan).await?;

        let lines = plan.content.lines().count();
        tracing::debug!(title = %title, lines, "plan written");

        Ok(AIOutput::success(
            serde_json::json!({
                "status": "active",
                "title": plan.title,
                "summary": plan.summary,
                "lines": lines,
            }),
            format!(
                "[成功] 计划「{}」已建立（{} 行，状态=active）。后续 thinking 将按计划执行。",
                title, lines
            ),
        ))
    }

    fn name(&self) -> &str {
        "PlanWrite"
    }
}

// ============================================================================
// PlanUpdate —— 覆盖当前执行计划的正文 / 摘要 / 标题
// ============================================================================

#[define_operation(
    name = "PlanUpdate",
    display_name = "将执行计划更新为标题{title}、摘要{summary}和正文{content}",
    category = "Planning",
    system_only,
    description = "更新当前执行计划：全量替换 content（以及可选的 title / summary），\
保留原有 created_at；并同步通知前端。\
适用于：执行过程中发现计划需要调整，或某一步已完成后修改剩余步骤。",
    params {
        content: "新的完整计划 Markdown 正文（必填）",
        title: "新的计划标题（可选，不传则保留原值）",
        summary: "新的计划摘要（可选，不传则保留原值）"
    },
    destructive = false,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct PlanUpdateSystem;

#[async_trait]
impl SystemOperation for PlanUpdateSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let content = match args.safe_require("content") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let new_title = args.get("title").map(|s| s.to_string());
        let new_summary = args.get("summary").map(|s| s.to_string());

        let mut plan = match AssistantContext::get_current_plan(&ctx.cache).await? {
            Some(p) => p,
            None => {
                return Ok(AIOutput::error(
                    404,
                    "当前没有已建立的计划。请先调用 PlanWrite 建立计划。".to_string(),
                ));
            }
        };

        if !plan.is_active() {
            return Ok(AIOutput::error(
                409,
                format!(
                    "当前计划状态为 {}，已完成的计划不能再更新。如需新计划，请调用 PlanWrite。",
                    plan.status
                ),
            ));
        }

        if let Some(t) = new_title {
            if !t.is_empty() {
                plan.title = t;
            }
        }
        if let Some(s) = new_summary {
            plan.summary = s;
        }
        plan.content = content.clone();
        plan.updated_at = now_rfc3339();

        AssistantContext::set_current_plan(&ctx.cache, &plan).await?;
        crate::persistence::save_default_agent_current_plan(Some(plan.clone()))
            .await
            .map_err(|e| FrameworkError::SystemError(format!("保存当前计划失败: {}", e)))?;
        publish_plan_event(ctx, ev_types::PLAN_UPDATED, &plan).await?;

        let lines = plan.content.lines().count();
        tracing::debug!(title = %plan.title, lines, "plan updated");

        Ok(AIOutput::success(
            serde_json::json!({
                "status": plan.status,
                "title": plan.title,
                "summary": plan.summary,
                "lines": lines,
            }),
            format!("[成功] 计划「{}」已更新（{} 行）。", plan.title, lines),
        ))
    }

    fn name(&self) -> &str {
        "PlanUpdate"
    }
}

// ============================================================================
// PlanFinish —— 标记当前执行计划完成
// ============================================================================

#[define_operation(
    name = "PlanFinish",
    display_name = "完成当前执行计划并记录说明{note}",
    category = "Planning",
    system_only,
    description = "将当前执行计划标记为 finished：\
thinking 从此停止对该计划的强注入，让对话回到无计划的自由推进状态。\
数据本身保留在状态机中，供审计或前端归档展示。用户明确表示任务完成 / 目标达成时调用。",
    params {
        note: "完成说明（可选，记录本次计划的最终状态或遗留事项）"
    },
    destructive = false,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct PlanFinishSystem;

#[async_trait]
impl SystemOperation for PlanFinishSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let note = args.get("note").unwrap_or("").to_string();

        let mut plan = match AssistantContext::get_current_plan(&ctx.cache).await? {
            Some(p) => p,
            None => {
                return Ok(AIOutput::error(
                    404,
                    "当前没有已建立的计划，无需结束。".to_string(),
                ));
            }
        };

        if !plan.is_active() {
            return Ok(AIOutput::success(
                serde_json::json!({ "status": plan.status }),
                format!("[跳过] 当前计划状态已为 {}，无需重复结束。", plan.status),
            ));
        }

        plan.status = CurrentPlan::STATUS_FINISHED.to_string();
        plan.updated_at = now_rfc3339();
        if !note.is_empty() {
            plan.content.push_str("\n\n---\n");
            plan.content
                .push_str(&format!("**[完成记录 @ {}]** {}", plan.updated_at, note));
        }

        AssistantContext::set_current_plan(&ctx.cache, &plan).await?;
        crate::persistence::save_default_agent_current_plan(Some(plan.clone()))
            .await
            .map_err(|e| FrameworkError::SystemError(format!("保存当前计划失败: {}", e)))?;
        publish_plan_event(ctx, ev_types::PLAN_FINISHED, &plan).await?;

        tracing::debug!(title = %plan.title, "plan finished");

        Ok(AIOutput::success(
            serde_json::json!({
                "status": plan.status,
                "title": plan.title,
            }),
            format!("[成功] 计划「{}」已标记为完成。", plan.title),
        ))
    }

    fn name(&self) -> &str {
        "PlanFinish"
    }
}
