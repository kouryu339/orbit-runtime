//!
//!
//! |------|------|
//! | `RecorderStartSystem`   | 启动录制会话，切换 recorder skills |
//! | `RecorderWriteSystem`   | 全量覆盖写入操作链文本，同步做语法合法性验证 |
//! | `RecorderUndoSystem`    | 撤销最后录入的一步 |
//! | `RecorderShowSystem`    | 显示当前完整操作链 |
//!
//! ## 关键约束
//!
//! - **不依赖 ai-assistant crate**（单向依赖关系）。
//!   与 Agent cache 键使用字符串字面量，与 `ai_assistant::context::keys` 中的常量保持一致。
//! - `RecorderWriteSystem` 在每次写入时调用 `compile_chain` 做语法合法性校验。
//!   若工作流实例生成成功，则整个工作流无误，随即保存并退出录制模式。

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::buns_system;
use corework::cache::CacheExt;
use corework::error::FrameworkError;
use corework::event::BaseEvent;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use corework::workflow::blueprint_json::{BlueprintJson, BlueprintVisibility, PinMetadata};
use corework::workflow::BlueprintLoader;

// ============================================================================
// Cache 键常量（与 ai-assistant::context::keys 中的字符串保持同步）
// ============================================================================

mod keys {
    // ---- 录制会话专用 ----

    /// 录制模式激活标志 — `bool`
    pub const RECORDER_ACTIVE: &str = "recorder_active";
    pub const RECORDER_CHAIN: &str = "recorder_chain";

    /// 录制前的 imported_skills 快照 — `Vec<String>`
    pub const RECORDER_SAVED_SKILLS: &str = "recorder_saved_skills";

    /// 录制前的 imported_tools 快照 — `Vec<String>`
    pub const RECORDER_SAVED_TOOLS: &str = "recorder_saved_tools";

    /// 运行时导入的 Skills 名称列表（同 ai_assistant::context::keys::IMPORTED_SKILLS）
    pub const IMPORTED_SKILLS: &str = "imported_skills";

    /// 运行时导入的工具名称列表（同 ai_assistant::context::keys::IMPORTED_TOOLS）
    pub const IMPORTED_TOOLS: &str = "imported_tools";
}

// ============================================================================
// RecorderStartSystem
// ============================================================================

#[buns_system(
    "RecorderStartSystem",
    description = "启动工作流录制，激活{{activate_skills}}领域技能",
    params {
        activate_skills: "录制期间需要额外激活的领域技能（可选），逗号分隔。\
                          示例：browser（网页自动化）、fileops（文件操作）、audioconv（音频处理）。\
                          recorder skill 始终自动激活，无需填写。"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RecorderStartSystem;

#[async_trait]
impl SystemOperation for RecorderStartSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let activate_skills_raw = args.get_or("activate_skills", "");

        // ── 1. 保存当前状态快照 ─────────────
        let saved_default_skills: Vec<String> =
            ctx.cache.get("default_skills").await?.unwrap_or_default();
        let saved_imported_skills: Vec<String> = ctx
            .cache
            .get(keys::IMPORTED_SKILLS)
            .await?
            .unwrap_or_default();
        let saved_imported_tools: Vec<String> = ctx
            .cache
            .get(keys::IMPORTED_TOOLS)
            .await?
            .unwrap_or_default();
        let saved_imported_views: Vec<String> =
            ctx.cache.get("imported_views").await?.unwrap_or_default();

        ctx.cache
            .set(keys::RECORDER_SAVED_SKILLS, &saved_imported_skills, None)
            .await?;
        ctx.cache
            .set(keys::RECORDER_SAVED_TOOLS, &saved_imported_tools, None)
            .await?;
        ctx.cache
            .set("recorder_saved_default_skills", &saved_default_skills, None)
            .await?;
        ctx.cache
            .set("recorder_saved_imported_views", &saved_imported_views, None)
            .await?;

        // ── 2. 替换 default_skills 中的 navigation 为 recorder（幂等）─────
        let mut new_default_skills = saved_default_skills.clone();
        if let Some(pos) = new_default_skills.iter().position(|s| s == "navigation") {
            new_default_skills[pos] = "recorder".to_string();
        } else if !new_default_skills.contains(&"recorder".to_string()) {
            new_default_skills.push("recorder".to_string());
        }
        ctx.cache
            .set("default_skills", &new_default_skills, None)
            .await?;

        // ── 3. 设置 imported_skills 为额外激活的领域 skills ─────────
        let mut new_imported_skills: Vec<String> = Vec::new();
        for s in activate_skills_raw.split(',') {
            let s = s.trim();
            if !s.is_empty() && !new_imported_skills.contains(&s.to_string()) {
                new_imported_skills.push(s.to_string());
            }
        }
        ctx.cache
            .set(keys::IMPORTED_SKILLS, &new_imported_skills, None)
            .await?;

        // ── 4. 从 SkillManager 收集所有激活 skills 的 tools 和 views ─────────
        // 注意: 这里我们需要通过 World cache 访问 SkillManager
        // 因为 workflows crate 不直接依赖 ai-assistant crate
        // 所以我们使用一个简化的方法: 直接从 recorder skill 的 metadata 读取

        // recorder skill 的 tools 和 views (从 SKILL.md 读取)
        let mut new_tools: Vec<String> = vec![
            "RecorderWriteSystem".to_string(),
            "RecorderUndoSystem".to_string(),
            "RecorderAnnotateSystem".to_string(),
            "GetSkillsList".to_string(),
            "UpdateSkills".to_string(),
        ];

        let mut new_views: Vec<String> = vec![
            "text".to_string(),
            "confirm".to_string(),
            "select".to_string(),
            "file_input".to_string(),
        ];

        // Future: 从额外激活的 skills 中收集 tools 和 views
        // 这需要通过 World cache 访问 SkillManager,或者通过其他方式
        // 暂时先使用 recorder 的 tools 和 views

        ctx.cache
            .set(keys::IMPORTED_TOOLS, &new_tools, None)
            .await?;
        ctx.cache.set("imported_views", &new_views, None).await?;

        ctx.cache.set(keys::RECORDER_ACTIVE, &true, None).await?;

        // 创建 WorkflowDraft 存入 World cache（替代旧的 recorder_chain + recorder_step_num）
        if let Ok(world) = ctx.get_world_cache() {
            let draft = crate::workflow::workflows::draft::WorkflowDraft::new("");
            let _ =
                world.set_resource(crate::workflow::workflows::draft::keys::DRAFT, &draft, None);
            // P0 同步：新草稿出生时刷新快照
            let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(ctx);
        }

        // Clear the internal recorder chain for a fresh recording session.
        {
            let _ = ctx
                .cache
                .set(keys::RECORDER_CHAIN, &String::new(), None)
                .await;
        }

        let mut all_active_skills = new_default_skills.clone();
        all_active_skills.extend(new_imported_skills.clone());
        let active_list = all_active_skills.join(", ");

        tracing::debug!(
            "RecorderStart: 默认技能 = {:?}, 导入技能 = {:?}, 工具 = {:?}, 视图 = {:?}",
            new_default_skills,
            new_imported_skills,
            new_tools,
            new_views
        );

        let open_window_event = BaseEvent::new(
            "ui:open-window",
            serde_json::json!({
                "window_type": "workflow-editor",
                "params": null
            }),
        );
        if let Err(e) = ctx.world_event_bus.publish(open_window_event).await {
            tracing::warn!("发布 ui:open-window 事件失败: {}", e);
        }

        Ok(AIOutput::success(
            serde_json::json!({
                "default_skills":   new_default_skills,
                "imported_skills":  new_imported_skills,
                "active_tools":     new_tools,
                "active_views":     new_views,
                "recorder_active":  true,
            }),
            format!(
                "录制会话已启动，已激活技能：{}。\n\
                 请回顾用户的原始消息，如果用户已经描述了要执行的操作，直接开始执行第一步；\
                 否则询问用户第一步要做什么。",
                active_list
            ),
        ))
    }

    fn name(&self) -> &str {
        "RecorderStartSystem"
    }
}

// ============================================================================
// RecorderWriteSystem
// ============================================================================

#[buns_system(
    "RecorderWriteSystem",
    description = "全量更新操作链为{{chain}}并验证语法",
    params {
        chain: "完整的操作链文本（必填）。包含所有步骤的全量文本，将直接覆盖当前链。\
                多行内容须用英文双引号包裹；若通过 CLI 字符串传递，换行用 \\n 表示，\
                系统会自动将 \\n 还原为真实换行。\
                示例：\"1. $page_id = OpenBrowser(url=\\\"https://example.com\\\")\\n\
                2. ClickElement(page_id=$page_id, selector=\\\"#btn\\\")\""
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RecorderWriteSystem;

#[async_trait]
impl SystemOperation for RecorderWriteSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let chain = match args.safe_require("chain") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        if chain.trim().is_empty() {
            return Ok(AIOutput::error(400, "操作链文本不能为空。".to_string()));
        }

        // ── 将字面 \n 还原为真实换行 ────────────────────────────────────────
        let new_chain = chain
            .replace(r"\n", "\n")
            .replace(r#"\""#, "\"")
            .trim_end()
            .to_string();

        // ── 从 World cache 读取 WorkflowDraft ────────────────────────────────
        let world = ctx.get_world_cache()?;
        let mut draft: crate::workflow::workflows::draft::WorkflowDraft = world
            .get_resource(crate::workflow::workflows::draft::keys::DRAFT)?
            .unwrap_or_else(|| crate::workflow::workflows::draft::WorkflowDraft::new(""));

        if let Err(e) = draft.update_from_text(&new_chain) {
            return Ok(AIOutput::error(
                422,
                format!(
                    "❌ 操作链编译失败（第 {} 行）：{}\n\n请直接修正操作链后重新调用 RecorderWriteSystem：\n```\n{}\n```",
                    e.line, e.message, new_chain
                ),
            ));
        }

        // ── 写回 World cache ───────────────────────────────────────────────
        let _ = world.set_resource(crate::workflow::workflows::draft::keys::DRAFT, &draft, None);
        // P0 同步：RecorderWrite 是 agent 录制主路径，必须刷新快照
        let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(ctx);

        // Keep recorder state separate from host-owned dynamic context fields.
        {
            let _ = ctx.cache.set(keys::RECORDER_CHAIN, &new_chain, None).await;
        }

        let top_level_count = draft.top_level_step_count();
        let total_lines = new_chain.lines().filter(|l| !l.trim().is_empty()).count();

        tracing::debug!(
            "RecorderWrite: {} 个顶层步骤，共 {} 行（已通过语法验证）",
            top_level_count,
            total_lines
        );

        Ok(AIOutput::success(
            serde_json::json!({
                "top_level_steps": top_level_count,
                "total_lines":     total_lines,
                "validated":       true,
                "chain":           new_chain,
            }),
            format!(
                "✓ 操作链已更新并通过语法验证（{} 个顶层步骤，共 {} 行）\n\n当前完整操作链：\n```\n{}\n```",
                top_level_count, total_lines, new_chain
            ),
        ))
    }

    fn name(&self) -> &str {
        "RecorderWriteSystem"
    }
}

// ============================================================================
// RecorderUndoSystem
// ============================================================================

#[buns_system(
    "RecorderUndoSystem",
    description = "撤销操作链最后一行",
    params {},
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RecorderUndoSystem;

#[async_trait]
impl SystemOperation for RecorderUndoSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, _input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let world = ctx.get_world_cache()?;
        let mut draft: crate::workflow::workflows::draft::WorkflowDraft = world
            .get_resource(crate::workflow::workflows::draft::keys::DRAFT)?
            .unwrap_or_else(|| crate::workflow::workflows::draft::WorkflowDraft::new(""));

        let current_chain = &draft.chain_text;

        if current_chain.trim().is_empty() {
            return Ok(AIOutput::error(
                400,
                "操作链为空，没有可撤销的步骤。".to_string(),
            ));
        }

        let lines: Vec<&str> = current_chain.lines().collect();

        // 找到最后一个非空行的索引
        let last_idx = match lines.iter().rposition(|l| !l.trim().is_empty()) {
            Some(i) => i,
            None => return Ok(AIOutput::error(400, "操作链全为空行。".to_string())),
        };

        let removed_line = lines[last_idx].to_string();
        let new_chain = lines[..last_idx].join("\n");

        // 通过 update_from_text 同步 JSON（空文本时直接清空）
        if new_chain.trim().is_empty() {
            draft.chain_text = String::new();
            draft.blueprint = BlueprintJson::new(&draft.blueprint.metadata.name);
        } else if let Err(e) = draft.update_from_text(&new_chain) {
            tracing::warn!("RecorderUndo: 撤销后编译失败: {}, 仅更新文本", e);
            draft.chain_text = new_chain.clone();
        }

        let _ = world.set_resource(crate::workflow::workflows::draft::keys::DRAFT, &draft, None);
        // P0 同步：RecorderUndo 也是 mutation
        let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(ctx);

        let top_level_count = draft.top_level_step_count();
        let remaining_lines = draft
            .chain_text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count();

        Ok(AIOutput::success(
            serde_json::json!({
                "removed_line":    removed_line.trim(),
                "top_level_steps": top_level_count,
                "remaining_lines": remaining_lines,
            }),
            format!(
                "已撤销：「{}」\n当前有 {} 个顶层步骤，{} 行。",
                removed_line.trim(),
                top_level_count,
                remaining_lines
            ),
        ))
    }

    fn name(&self) -> &str {
        "RecorderUndoSystem"
    }
}

// ============================================================================
// RecorderShowSystem
// ============================================================================

#[buns_system(
    "RecorderShowSystem",
    description = "显示当前操作链文本和步骤统计",
    params {},
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct RecorderShowSystem;

#[async_trait]
impl SystemOperation for RecorderShowSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, _input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        // 从 World cache 读取 WorkflowDraft
        let step_num = if let Ok(world) = ctx.get_world_cache() {
            let draft: Option<crate::workflow::workflows::draft::WorkflowDraft> = world
                .get_resource(crate::workflow::workflows::draft::keys::DRAFT)
                .unwrap_or(None);
            draft.map(|d| d.top_level_step_count()).unwrap_or(0)
        } else {
            0
        };
        Ok(AIOutput::success(
            serde_json::json!({"top_level_steps": step_num}),
            format!(
                "当前有 {} 个顶层步骤，操作链已显示在系统上下文中。",
                step_num
            ),
        ))
    }

    fn name(&self) -> &str {
        "RecorderShowSystem"
    }
}

// ============================================================================
// RecorderAnnotateSystem
// ============================================================================

#[buns_system(
    "RecorderAnnotateSystem",
    description = "编译操作链并保存为{{name}}工作流（{{description}}），填写{{inputs}}和{{outputs}}引脚说明",
    params {
        name:        "工作流名称（必填），将作为文件名和注册键",
        description: "工作流功能描述（必填），说明此工作流能做什么",
        inputs:      "INPUT 引脚说明（可选），JSON 数组，格式：\
                      [{\"name\":\"引脚名\",\"description\":\"说明\"}]。\
                      data_type 由编译结果自动填入，无需手动指定",
        outputs:     "RETURN 引脚说明（可选），格式同 inputs"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RecorderAnnotateSystem;

#[async_trait]
impl SystemOperation for RecorderAnnotateSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let workflow_name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let workflow_desc = match args.safe_require("description") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let inputs_desc = args.get_or("inputs", "");
        let outputs_desc = args.get_or("outputs", "");

        // ── 1. 从 World cache 读取 WorkflowDraft ──────────────────────────
        let world = ctx.get_world_cache()?;
        let draft: crate::workflow::workflows::draft::WorkflowDraft =
            match world.get_resource(crate::workflow::workflows::draft::keys::DRAFT)? {
                Some(d) => d,
                None => {
                    return Ok(AIOutput::error(
                        400,
                        "当前无草稿，请先录入步骤。".to_string(),
                    ))
                }
            };

        let chain = &draft.chain_text;
        if chain.trim().is_empty() {
            return Ok(AIOutput::error(
                400,
                "操作链为空，请先录入步骤后再调用本系统。".to_string(),
            ));
        }

        let mut blueprint = draft.blueprint.clone();

        // ── 3. BlueprintLoader：生成工作流实例（完整可执行性验证）─────────
        //       实例生成成功 = 整个工作流无误
        if let Err(e) = BlueprintLoader::new()
            .load_workflow_from_blueprint_json(blueprint.clone())
            .await
        {
            return Ok(AIOutput::error(
                422,
                format!(
                    "工作流实例生成失败：{}\n\n操作链：\n```\n{}\n```\n\n\
                     请调用 RecorderWriteSystem 修正操作链后重试。",
                    e, chain
                ),
            ));
        }

        let (actual_inputs, actual_outputs) = extract_pins_from_blueprint(&blueprint);

        // ── 5. 解析用户提供的引脚说明（只取 name → description 映射）──────
        let user_input_descs = parse_user_pin_descs(&inputs_desc);
        let user_output_descs = parse_user_pin_descs(&outputs_desc);

        blueprint.metadata.inputs = merge_pin_metadata(&actual_inputs, &user_input_descs);
        blueprint.metadata.outputs = merge_pin_metadata(&actual_outputs, &user_output_descs);
        blueprint.metadata.name = workflow_name.to_string();
        if blueprint.metadata.id.is_empty() {
            blueprint.metadata.id = workflow_name.to_string();
        }
        blueprint.metadata.description = workflow_desc.to_string();
        blueprint.metadata.visibility = BlueprintVisibility::Private;
        blueprint.normalize_node_sizes();

        let node_count = blueprint.nodes.len();
        let conn_count = blueprint.connections.len();

        // ── 7. 保存到本地目录并注册 ─────────────────────────────────────────
        let file_path_str = {
            let world = ctx.get_world_cache()?;
            let workflows_dir: String = world
                .get_resource("wf:workflows_dir")
                .unwrap_or(None)
                .unwrap_or_else(|| "workflows".to_string());

            let safe_name =
                workflow_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            let file_name = format!("{}.workflow.json", safe_name);
            let save_path = std::path::Path::new(&workflows_dir).join(&file_name);

            if let Some(parent) = save_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }

            let json_pretty = serde_json::to_string_pretty(&blueprint)
                .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
            let path_str = save_path.to_string_lossy().to_string();

            if let Err(e) = tokio::fs::write(&save_path, &json_pretty).await {
                tracing::warn!("RecorderAnnotate 保存文件失败 (non-fatal): {}", e);
            }

            // 注册到 World registry
            let mut registry: Vec<crate::workflow::workflows::executor::BlueprintEntry> = world
                .get_resource(crate::workflow::workflows::executor::REGISTRY)
                .unwrap_or(Some(Vec::new()))
                .unwrap_or_default();
            registry.retain(|e| e.metadata.name != workflow_name);
            registry.push(crate::workflow::workflows::executor::BlueprintEntry {
                metadata: blueprint.metadata.clone(),
                file_path: path_str.clone(),
                key: format!("{}:recorder", workflow_name),
                source: crate::workflow::workflows::executor::WorkflowSource::Local,
            });
            let _ = world.set_resource(
                crate::workflow::workflows::executor::REGISTRY,
                &registry,
                None,
            );

            path_str
        };

        // ── 9. 恢复正常 Agent 模式 ─────────────────────────────────────────
        let saved_default_skills: Vec<String> = ctx
            .cache
            .get("recorder_saved_default_skills")
            .await?
            .unwrap_or_default();
        let saved_imported_skills: Vec<String> = ctx
            .cache
            .get(keys::RECORDER_SAVED_SKILLS)
            .await?
            .unwrap_or_default();
        let saved_imported_tools: Vec<String> = ctx
            .cache
            .get(keys::RECORDER_SAVED_TOOLS)
            .await?
            .unwrap_or_default();
        let saved_imported_views: Vec<String> = ctx
            .cache
            .get("recorder_saved_imported_views")
            .await?
            .unwrap_or_default();

        ctx.cache
            .set("default_skills", &saved_default_skills, None)
            .await?;
        ctx.cache
            .set(keys::IMPORTED_SKILLS, &saved_imported_skills, None)
            .await?;
        ctx.cache
            .set(keys::IMPORTED_TOOLS, &saved_imported_tools, None)
            .await?;
        ctx.cache
            .set("imported_views", &saved_imported_views, None)
            .await?;

        ctx.cache.set(keys::RECORDER_ACTIVE, &false, None).await?;
        ctx.cache.delete(keys::RECORDER_SAVED_SKILLS).await?;
        ctx.cache.delete(keys::RECORDER_SAVED_TOOLS).await?;
        ctx.cache.delete("recorder_saved_default_skills").await?;
        ctx.cache.delete("recorder_saved_imported_views").await?;

        // 清理 World cache 中的草稿
        if let Ok(world) = ctx.get_world_cache() {
            let _ = world.delete_resource(crate::workflow::workflows::draft::keys::DRAFT);
        }

        tracing::debug!(
            "RecorderAnnotate: 工作流「{}」已保存 ({} 节点, {} 连线) → {}",
            workflow_name,
            node_count,
            conn_count,
            file_path_str
        );

        Ok(AIOutput::success(
            serde_json::json!({
                "name":             workflow_name,
                "file_path":        file_path_str,
                "node_count":       node_count,
                "connection_count": conn_count,
            }),
            format!(
                "🎉 工作流「{}」已定稿保存！共 {} 个节点、{} 条连线。\n\
                 文件：{}\n\n\
                 录制模式已退出，已恢复正常助手模式。可以说「运行 {}」来执行这个工作流。",
                workflow_name, node_count, conn_count, file_path_str, workflow_name
            ),
        ))
    }

    fn name(&self) -> &str {
        "RecorderAnnotateSystem"
    }
}

// ============================================================================
// 内部工具函数
// ============================================================================

///
/// - `StartNode` 的 DataOutput 引脚 → 工作流 INPUT 参数
/// - `EndNode` 的 DataInput 引脚   → 工作流 RETURN 参数
///
/// 返回 `(input_pins, output_pins)`，每个元素为 `{"name": ..., "data_type": ...}`
fn extract_pins_from_blueprint(
    blueprint: &BlueprintJson,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut input_pins = Vec::new();
    let mut output_pins = Vec::new();

    let Ok(value) = serde_json::to_value(blueprint) else {
        return (input_pins, output_pins);
    };

    let Some(nodes) = value.get("nodes").and_then(|v| v.as_array()) else {
        return (input_pins, output_pins);
    };

    for node in nodes {
        let node_type = node.get("node_type").and_then(|v| v.as_str()).unwrap_or("");
        let Some(pins) = node.get("pins").and_then(|v| v.as_array()) else {
            continue;
        };

        for pin in pins {
            let pin_name = pin.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let data_type = pin
                .get("data_type")
                .and_then(|v| v.as_str())
                .unwrap_or("Any");
            let direction = pin.get("direction").and_then(|v| v.as_str()).unwrap_or("");

            match (node_type, direction) {
                ("StartNode", "DataOutput") => {
                    input_pins
                        .push(serde_json::json!({ "name": pin_name, "data_type": data_type }));
                }
                ("EndNode", "DataInput") => {
                    output_pins
                        .push(serde_json::json!({ "name": pin_name, "data_type": data_type }));
                }
                _ => {}
            }
        }
    }

    (input_pins, output_pins)
}

/// 解析用户提供的引脚说明 JSON，返回 `name → description` 映射。
///
/// 期望格式：`[{"name": "...", "description": "..."}]`
/// 若解析失败则返回空 Map，不报错。
fn parse_user_pin_descs(json_str: &str) -> std::collections::HashMap<String, String> {
    if json_str.trim().is_empty() {
        return std::collections::HashMap::new();
    }
    serde_json::from_str::<Vec<serde_json::Value>>(json_str)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            let name = v.get("name").and_then(|n| n.as_str())?.to_string();
            let desc = v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            Some((name, desc))
        })
        .collect()
}

///
/// - `description`  来自用户输入（若未提供则为空字符串）
/// - `default_value` 暂不设置
fn merge_pin_metadata(
    actual: &[serde_json::Value],
    descs: &std::collections::HashMap<String, String>,
) -> Vec<PinMetadata> {
    actual
        .iter()
        .map(|pin| {
            let name = pin
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let data_type = pin
                .get("data_type")
                .and_then(|v| v.as_str())
                .unwrap_or("Any")
                .to_string();
            let description = descs.get(&name).cloned().unwrap_or_default();
            PinMetadata {
                name,
                data_type,
                description,
                default_value: None,
            }
        })
        .collect()
}
