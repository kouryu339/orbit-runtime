//! - `GetSkillsList`：获取可用 Skills 列表，返回 to_ai 格式文本
//! - `UpdateSkills`：更新 imported_skills，同步 imported_tools
//! - `ComposeMessages`：组装完整消息序列（system prompt + 历史对话，token 预算内截断）
//! ## to_ai 设计
//! 自动作为 tool role 消息插入对话历史（conversation cache key）。

pub mod agent_route;
pub mod history;
pub mod ledger;
pub mod plan_file;
pub mod prompt;
pub mod workflow_studio;

use async_trait::async_trait;

use corework::ai_system::{AIInput, AIOutput};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use crate::context::{keys, AssistantContext};
use crate::skills::{systems::mgr, SkillMetadata};

pub mod system_names {
    pub const GET_SKILLS_LIST: &str = "GetSkillsList";
    pub const UPDATE_SKILLS: &str = "UpdateSkills";
}

// ============================================================================
// 1. GetSkillsList — 获取可用 Skills 列表
// ============================================================================

#[define_operation(
    name = "GetSkillsList",
    display_name = "Get Skills List",
    category = "Assistant",
    system_only,
    description = "获取所有可用的 Skills 列表。返回每个 Skill 的名称和描述，供 AI 判断需要激活哪些技能。",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct GetSkillsListSystem;

#[async_trait]
impl SystemOperation for GetSkillsListSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, _input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        // 从 SkillManager 获取 feature 层元数据（排除 system/main 层）
        let m = mgr().read().await;
        let skills: Vec<SkillMetadata> = m.feature_metadata().into_iter().cloned().collect();
        let active = AssistantContext::all_active_skills(&ctx.cache).await?;

        // 构造 to_ai：name description [已激活]
        let body = if skills.is_empty() {
            crate::prompt_assets::template("get_skills_list_empty.md")
                .trim()
                .to_string()
        } else {
            let mut lines = Vec::with_capacity(skills.len() + 1);
            lines.push(
                crate::prompt_assets::template("get_skills_list_header.md")
                    .trim()
                    .to_string(),
            );
            let active_suffix = crate::prompt_assets::template("skill_active_suffix.md")
                .trim_end()
                .to_string();
            for meta in &skills {
                let status = if active.contains(&meta.name) {
                    active_suffix.as_str()
                } else {
                    ""
                };
                lines.push(format!("{} {}{}", meta.name, meta.description, status));
            }
            lines.join("\n")
        };
        Ok(AIOutput::success(
            serde_json::json!({ "skills": skills, "active": active }),
            body,
        ))
    }

    fn name(&self) -> &str {
        system_names::GET_SKILLS_LIST
    }
}

// ============================================================================
// 2. UpdateSkills — 更新 imported_skills（全量替换语义）
// ============================================================================

#[define_operation(
    name = "UpdateSkills",
    display_name = "Update Skills",
    category = "Assistant",
    system_only,
    description = "全量替换当前激活的 Skills。传入的列表将直接覆盖 imported_skills，不在列表中的 Skill 会被移除，同时同步关联工具到 imported_tools。",
    params {
        skills: "期望激活的 Skill 名称数组（必填，例如：[\"rust-coding\", \"example-skill\"]）"
    },
    destructive = false,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct UpdateSkillsSystem;

#[async_trait]
impl SystemOperation for UpdateSkillsSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        // 兼容两种格式：
        // 1. 逗号分隔: --skills browser,fileops
        // 2. JSON 数组: --skills ["browser","fileops"]
        let raw = args.get("skills").unwrap_or("");
        let desired: Vec<String> = if raw.trim_start().starts_with('[') {
            // 尝试解析 JSON 数组
            serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| {
                // JSON 解析失败时 fallback：去掉括号后逗号分隔
                raw.trim_matches(|c| c == '[' || c == ']')
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
        } else {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        // 全量替换 imported_skills（仅 feature 层）
        ctx.cache.set(keys::IMPORTED_SKILLS, &desired, None).await?;

        // 加载 Tier 2（指导内容）并同步 active_tools
        let mut m = mgr().write().await;
        let name_refs: Vec<&str> = desired.iter().map(|s| s.as_str()).collect();
        if let Err(e) = m.load_many(&name_refs).await {
            tracing::warn!("UpdateSkills 加载部分 Skills 失败: {}", e);
        }

        // 同步 active_tools：main skills + feature skills 的工具合并（去重）
        let mut new_tools: Vec<String> = Vec::new();

        // 先注入 main skills 的工具（始终存在）
        let main_names: Vec<String> = ctx
            .cache
            .get::<Vec<String>>(keys::MAIN_SKILLS)
            .await?
            .unwrap_or_default();
        let main_refs: Vec<&str> = main_names.iter().map(|s| s.as_str()).collect();
        m.inject_tools_for_skills(&main_refs, &mut new_tools);

        // 再注入 feature skills 的工具
        m.inject_tools_for_skills(&name_refs, &mut new_tools);

        // thinking system Skill 的工具属于每个 Agent 的基础白名单。
        m.inject_tools_for_state(crate::state::states::THINKING, &mut new_tools);

        ctx.cache.set(keys::ACTIVE_TOOLS, &new_tools, None).await?;
        let (event_agent_id, event_agent_name) =
            crate::agent::source_meta_from_cache(&*ctx.cache).await;
        let mut event = corework::event::BaseEvent::new(
            crate::events::types::AGENT_SKILLS_CHANGED,
            serde_json::json!({
                "conversation_id": ctx.conversation_id.clone().unwrap_or_default(),
                "agent_id": event_agent_id,
                "agent_name": event_agent_name,
                "main_skills": main_names,
                "imported_skills": desired.clone(),
                "active_tools": new_tools.clone(),
            }),
        );
        if let Some(conversation_id) = ctx.conversation_id.clone() {
            event = event.with_conversation_id(conversation_id);
        }
        ctx.event_bus.publish(event).await?;

        // 持久化 imported_skills 到 session 索引（Boss 或子 Agent）
        {
            let agent_id: Option<String> = ctx
                .cache
                .get(crate::state_machine::agent_keys::AGENT_ID)
                .await
                .ok()
                .flatten();
            let skills_clone = desired.clone();
            tokio::spawn(async move {
                let default_agent_id = crate::persistence::current_default_agent_id();
                let result = if let Some(aid) = agent_id.filter(|aid| aid != &default_agent_id) {
                    crate::persistence::save_agent_skill_state(&aid, skills_clone).await
                } else {
                    crate::persistence::save_default_agent_skill_state(skills_clone).await
                };
                if let Err(e) = result {
                    tracing::warn!("持久化 skill 状态失败: {}", e);
                }
            });
        }

        // to_ai：显示当前激活的 skills
        let body = if desired.is_empty() {
            crate::prompt_assets::template("update_skills_empty.md")
                .trim()
                .to_string()
        } else {
            let skills_text = desired.join(", ");
            crate::prompt_assets::render(
                "update_skills_success.md",
                &[("{{SKILLS}}", &skills_text)],
            )
        };
        Ok(AIOutput::success(
            serde_json::json!({ "active_skills": desired }),
            body,
        ))
    }

    fn name(&self) -> &str {
        system_names::UPDATE_SKILLS
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_names() {
        assert_eq!(system_names::GET_SKILLS_LIST, "GetSkillsList");
        assert_eq!(system_names::UPDATE_SKILLS, "UpdateSkills");
    }
}
