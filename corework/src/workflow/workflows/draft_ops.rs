//!
//! 通过 `#[buns_system]` 宏 + `inventory` 自动注册到 `SystemRegistry`，
//! `ctx.get_dynamic_system("name")` 或 `ctx.system::<T>("name")` 调用。

use std::collections::HashMap;

use async_trait::async_trait;
use corework::buns_system;
use corework::orchestration::Context;
use corework::prelude::{FrameworkError, SystemOperation};
use corework::workflow::blueprint_json::{
    BlueprintJson, BlueprintNodeJson, BlueprintVariable, CommentBox, CommentSize, NodePin,
    NodePosition, NodeSize, PinMetadata,
};
use corework::workflow::registry::{NodePermissions, NodeRegistry, PinKind};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::workflow::workflows::draft::{self, keys, WorkflowDraft};

// ============================================================================
// 公共输出类型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftOpOutput {
    pub success: bool,
    pub message: String,
    /// 操作产生的 ID（如新节点 ID、新连线 ID）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl DraftOpOutput {
    fn ok(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: msg.into(),
            id: None,
        }
    }
    fn ok_with_id(msg: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            success: true,
            message: msg.into(),
            id: Some(id.into()),
        }
    }
    fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            message: msg.into(),
            id: None,
        }
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

const MAX_HISTORY: usize = 50;

fn read_draft(ctx: &Context) -> Result<WorkflowDraft, FrameworkError> {
    let world = ctx.get_world_cache()?;
    world
        .get_resource::<WorkflowDraft>(keys::DRAFT)?
        .ok_or_else(|| FrameworkError::WorkflowError("草稿不存在，请先创建".into()))
}

fn write_draft_with_undo(ctx: &Context, draft: &WorkflowDraft) -> Result<(), FrameworkError> {
    let world = ctx.get_world_cache()?;
    let is_new = world.get_resource::<WorkflowDraft>(keys::DRAFT)?.is_none();
    // 推入 undo 栈
    if let Some(current) = world.get_resource::<WorkflowDraft>(keys::DRAFT)? {
        let mut history: Vec<WorkflowDraft> =
            world.get_resource(keys::HISTORY)?.unwrap_or_default();
        history.push(current);
        if history.len() > MAX_HISTORY {
            history.remove(0);
        }
        world.set_resource(keys::HISTORY, &history, None)?;
    }
    world.set_resource(keys::DRAFT, draft, None)?;
    // 草稿从无到有时通知外部
    if is_new {
        crate::workflow::workflows::notify_draft_exists(true);
    }
    // 刷新快照（bump version + 重建渲染），供 agent 上下文和前端共用
    let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(ctx);
    Ok(())
}

#[allow(dead_code)]
fn write_draft_no_undo(ctx: &Context, draft: &WorkflowDraft) -> Result<(), FrameworkError> {
    let world = ctx.get_world_cache()?;
    let is_new = world.get_resource::<WorkflowDraft>(keys::DRAFT)?.is_none();
    world.set_resource(keys::DRAFT, draft, None)?;
    if is_new {
        crate::workflow::workflows::notify_draft_exists(true);
    }
    Ok(())
}

fn next_node_id(draft: &WorkflowDraft) -> String {
    let max = draft
        .blueprint
        .nodes
        .iter()
        .filter_map(|n| n.id.strip_prefix('n').and_then(|s| s.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("n{}", max + 1)
}

#[allow(dead_code)]
fn next_conn_id(draft: &WorkflowDraft) -> String {
    let max = draft
        .blueprint
        .connections
        .iter()
        .filter_map(|c| c.id.strip_prefix('c').and_then(|s| s.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("c{}", max + 1)
}

fn next_comment_id(draft: &WorkflowDraft) -> String {
    let max = draft
        .blueprint
        .comments
        .iter()
        .filter_map(|c| c.id.strip_prefix("cm").and_then(|s| s.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("cm{}", max + 1)
}

// ============================================================================
// ============================================================================

// ── DraftNew ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftNewInput {
    pub name: String,
}

#[buns_system("DraftNew")]
pub struct DraftNewSystem;

#[async_trait]
impl SystemOperation for DraftNewSystem {
    type Input = DraftNewInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let draft = WorkflowDraft::new(&input.name);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已创建草稿「{}」", input.name)))
    }

    fn name(&self) -> &str {
        "DraftNew"
    }
}

// ── DraftGet ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftGetInput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftGetOutput {
    pub draft: Option<WorkflowDraft>,
}

#[buns_system("DraftGet")]
pub struct DraftGetSystem;

#[async_trait]
impl SystemOperation for DraftGetSystem {
    type Input = DraftGetInput;
    type Output = DraftGetOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let world = ctx.get_world_cache()?;
        let draft = world.get_resource::<WorkflowDraft>(keys::DRAFT)?;
        Ok(DraftGetOutput { draft })
    }

    fn name(&self) -> &str {
        "DraftGet"
    }
}

// ── DraftFromText ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftFromTextInput {
    pub text: String,
}

#[buns_system("DraftFromText")]
pub struct DraftFromTextSystem;

#[async_trait]
impl SystemOperation for DraftFromTextSystem {
    type Input = DraftFromTextInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = match read_draft(ctx) {
            Ok(d) => d,
            Err(_) => WorkflowDraft::new("untitled"),
        };
        if let Err(e) = draft.update_from_text(&input.text) {
            return Ok(DraftOpOutput::err(format!("编译失败: {}", e)));
        }
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("已从文本更新草稿"))
    }

    fn name(&self) -> &str {
        "DraftFromText"
    }
}

// ── DraftFromJson ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftFromJsonInput {
    pub json: String,
}

#[buns_system("DraftFromJson")]
pub struct DraftFromJsonSystem;

#[async_trait]
impl SystemOperation for DraftFromJsonSystem {
    type Input = DraftFromJsonInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let bp = match BlueprintJson::from_json_str(&input.json) {
            Ok(bp) => bp,
            Err(e) => return Ok(DraftOpOutput::err(format!("JSON 解析失败: {}", e))),
        };
        let mut draft = WorkflowDraft::new(&bp.metadata.name);
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("已从 JSON 导入草稿"))
    }

    fn name(&self) -> &str {
        "DraftFromJson"
    }
}

// ── DraftCommit ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftCommitInput;

#[buns_system("DraftCommit")]
pub struct DraftCommitSystem;

#[async_trait]
impl SystemOperation for DraftCommitSystem {
    type Input = DraftCommitInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let draft = read_draft(ctx)?;
        if let Err(e) = draft.blueprint.validate() {
            return Ok(DraftOpOutput::err(format!("蓝图验证失败: {}", e)));
        }
        let name = draft.blueprint.metadata.name.clone();
        // 注册到 World 的工作流注册表
        let world = ctx.get_world_cache()?;
        let json_str = draft
            .blueprint
            .to_json_string()
            .map_err(FrameworkError::SerializationError)?;
        let key = format!("wf:{}", name);
        world.set_resource(&key, &json_str, None)?;
        // 提交后删除草稿并通知窗口关闭
        world.delete_resource(keys::DRAFT);
        world.delete_resource(keys::HISTORY);
        crate::workflow::workflows::notify_draft_exists(false);
        Ok(DraftOpOutput::ok_with_id(
            format!("草稿「{}」已提交", name),
            key,
        ))
    }

    fn name(&self) -> &str {
        "DraftCommit"
    }
}

// ── DraftClose ───────────────────────────────────────────────────────────────
// 放弃草稿（不提交，直接丢弃）

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftCloseInput;

#[buns_system("DraftClose")]
pub struct DraftCloseSystem;

#[async_trait]
impl SystemOperation for DraftCloseSystem {
    type Input = DraftCloseInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let world = ctx.get_world_cache()?;
        let existed = world.get_resource::<WorkflowDraft>(keys::DRAFT)?.is_some();
        world.delete_resource(keys::DRAFT);
        world.delete_resource(keys::HISTORY);
        if existed {
            crate::workflow::workflows::notify_draft_exists(false);
        }
        Ok(DraftOpOutput::ok("草稿已关闭"))
    }

    fn name(&self) -> &str {
        "DraftClose"
    }
}

// ── DraftUndo ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftUndoInput;

#[buns_system("DraftUndo")]
pub struct DraftUndoSystem;

#[async_trait]
impl SystemOperation for DraftUndoSystem {
    type Input = DraftUndoInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let world = ctx.get_world_cache()?;
        let mut history: Vec<WorkflowDraft> =
            world.get_resource(keys::HISTORY)?.unwrap_or_default();
        if history.is_empty() {
            return Ok(DraftOpOutput::err("没有可撤销的操作"));
        }
        let prev = history.pop().unwrap();
        world.set_resource(keys::HISTORY, &history, None)?;
        world.set_resource(keys::DRAFT, &prev, None)?;
        // P0 同步：撤销也是 mutation，必须刷新快照让前端/agent 看到
        let _ = crate::workflow::workflows::snapshot::refresh_world_snapshot(ctx);
        Ok(DraftOpOutput::ok("已撤销"))
    }

    fn name(&self) -> &str {
        "DraftUndo"
    }
}

// ── DraftSaveToFile ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSaveToFileInput {
    pub file_path: String,
}

#[buns_system("DraftSaveToFile")]
pub struct DraftSaveToFileSystem;

#[async_trait]
impl SystemOperation for DraftSaveToFileSystem {
    type Input = DraftSaveToFileInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let draft = read_draft(ctx)?;
        let json_str = draft
            .blueprint
            .to_json_string()
            .map_err(FrameworkError::SerializationError)?;
        tokio::fs::write(&input.file_path, json_str)
            .await
            .map_err(|e| FrameworkError::WorkflowError(format!("写入文件失败: {}", e)))?;
        Ok(DraftOpOutput::ok(format!("已保存到 {}", input.file_path)))
    }

    fn name(&self) -> &str {
        "DraftSaveToFile"
    }
}

// ============================================================================
// ============================================================================

// ── DraftSetName ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetNameInput {
    pub name: String,
}

#[buns_system("DraftSetName")]
pub struct DraftSetNameSystem;

#[async_trait]
impl SystemOperation for DraftSetNameSystem {
    type Input = DraftSetNameInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.name = input.name.clone();
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("名称已设为「{}」", input.name)))
    }

    fn name(&self) -> &str {
        "DraftSetName"
    }
}

// ── DraftSetDescription ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetDescriptionInput {
    pub description: String,
}

#[buns_system("DraftSetDescription")]
pub struct DraftSetDescriptionSystem;

#[async_trait]
impl SystemOperation for DraftSetDescriptionSystem {
    type Input = DraftSetDescriptionInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.description = input.description;
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("描述已更新"))
    }

    fn name(&self) -> &str {
        "DraftSetDescription"
    }
}

// ── DraftSetAuthor ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetAuthorInput {
    pub author: String,
}

#[buns_system("DraftSetAuthor")]
pub struct DraftSetAuthorSystem;

#[async_trait]
impl SystemOperation for DraftSetAuthorSystem {
    type Input = DraftSetAuthorInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.author = input.author;
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("作者已更新"))
    }

    fn name(&self) -> &str {
        "DraftSetAuthor"
    }
}

// ── DraftSetVisibility ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetVisibilityInput {
    pub visibility: String,
}

#[buns_system("DraftSetVisibility")]
pub struct DraftSetVisibilitySystem;

#[async_trait]
impl SystemOperation for DraftSetVisibilitySystem {
    type Input = DraftSetVisibilityInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        use corework::workflow::blueprint_json::BlueprintVisibility;
        let vis = match input.visibility.as_str() {
            "private" => BlueprintVisibility::Private,
            "public" => BlueprintVisibility::Public,
            other => return Ok(DraftOpOutput::err(format!("无效的可见性: {}", other))),
        };
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.visibility = vis;
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "可见性已设为 {}",
            input.visibility
        )))
    }

    fn name(&self) -> &str {
        "DraftSetVisibility"
    }
}

// ── DraftSetTags ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetTagsInput {
    pub tags: Vec<String>,
}

#[buns_system("DraftSetTags")]
pub struct DraftSetTagsSystem;

#[async_trait]
impl SystemOperation for DraftSetTagsSystem {
    type Input = DraftSetTagsInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.tags = input.tags;
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("标签已更新"))
    }

    fn name(&self) -> &str {
        "DraftSetTags"
    }
}

// ── DraftSetInputPinsMeta ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetInputPinsMetaInput {
    pub pins: Vec<PinMetadata>,
}

#[buns_system("DraftSetInputPinsMeta")]
pub struct DraftSetInputPinsMetaSystem;

#[async_trait]
impl SystemOperation for DraftSetInputPinsMetaSystem {
    type Input = DraftSetInputPinsMetaInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.inputs = input.pins;
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("输入引脚元数据已更新"))
    }

    fn name(&self) -> &str {
        "DraftSetInputPinsMeta"
    }
}

// ── DraftSetOutputPinsMeta ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetOutputPinsMetaInput {
    pub pins: Vec<PinMetadata>,
}

#[buns_system("DraftSetOutputPinsMeta")]
pub struct DraftSetOutputPinsMetaSystem;

#[async_trait]
impl SystemOperation for DraftSetOutputPinsMetaSystem {
    type Input = DraftSetOutputPinsMetaInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        draft.blueprint.metadata.outputs = input.pins;
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok("输出引脚元数据已更新"))
    }

    fn name(&self) -> &str {
        "DraftSetOutputPinsMeta"
    }
}

// ============================================================================
// ============================================================================

// ── DraftAddNode ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftAddNodeInput {
    pub node_type: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[buns_system("DraftAddNode")]
pub struct DraftAddNodeSystem;

#[async_trait]
impl SystemOperation for DraftAddNodeSystem {
    type Input = DraftAddNodeInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let node_id = next_node_id(&draft);

        // 从 NodeRegistry 填充引脚
        let mut pins = Vec::new();
        if let Some(meta) = NodeRegistry::get(&input.node_type) {
            for pin in meta.pins.iter() {
                pins.push(NodePin {
                    name: pin.name.to_string(),
                    kind: match pin.kind {
                        PinKind::ExecInput => "ExecInput",
                        PinKind::ExecOutput => "ExecOutput",
                        PinKind::DataInput => "DataInput",
                        PinKind::DataOutput => "DataOutput",
                    }
                    .to_string(),
                    data_type: pin.data_type.to_string(),
                    description: pin.description.to_string(),
                    default_value: pin.default_value.and_then(|v| serde_json::from_str(v).ok()),
                    resolved_type: None,
                    split_config: None,
                });
            }
        }

        let node = BlueprintNodeJson {
            id: node_id.clone(),
            node_type: input.node_type.clone(),
            position: NodePosition {
                x: input.x,
                y: input.y,
            },
            size: NodeSize::from_pins(&pins),
            pins,
            properties: HashMap::new(),
            display_name: input.display_name,
            comment: None,
        };

        draft.blueprint.add_node(node);
        // 结构性操作 → 同步 chain_text
        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok_with_id(
            format!("已添加节点 {} ({})", node_id, input.node_type),
            &node_id,
        ))
    }

    fn name(&self) -> &str {
        "DraftAddNode"
    }
}

// ── DraftRemoveNode ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRemoveNodeInput {
    pub node_id: String,
}

#[buns_system("DraftRemoveNode")]
pub struct DraftRemoveNodeSystem;

#[async_trait]
impl SystemOperation for DraftRemoveNodeSystem {
    type Input = DraftRemoveNodeInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        if draft.blueprint.find_node(&input.node_id).is_none() {
            return Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id)));
        }
        draft.blueprint.remove_node(&input.node_id);
        draft.blueprint.remove_connections_for_node(&input.node_id);
        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已删除节点 {}", input.node_id)))
    }

    fn name(&self) -> &str {
        "DraftRemoveNode"
    }
}

// ── DraftMoveNode ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftMoveNodeInput {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

#[buns_system("DraftMoveNode")]
pub struct DraftMoveNodeSystem;

#[async_trait]
impl SystemOperation for DraftMoveNodeSystem {
    type Input = DraftMoveNodeInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        match draft.blueprint.find_node_mut(&input.node_id) {
            Some(node) => {
                node.position = NodePosition {
                    x: input.x,
                    y: input.y,
                };
                draft.blueprint.update_modified_time();
                write_draft_with_undo(ctx, &draft)?;
                Ok(DraftOpOutput::ok(format!("节点 {} 已移动", input.node_id)))
            }
            None => Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
        }
    }

    fn name(&self) -> &str {
        "DraftMoveNode"
    }
}

// ── DraftBatchMoveNodes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMoveItem {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftBatchMoveNodesInput {
    pub moves: Vec<NodeMoveItem>,
}

#[buns_system("DraftBatchMoveNodes")]
pub struct DraftBatchMoveNodesSystem;

#[async_trait]
impl SystemOperation for DraftBatchMoveNodesSystem {
    type Input = DraftBatchMoveNodesInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let mut moved = 0usize;
        for m in &input.moves {
            if let Some(node) = draft.blueprint.find_node_mut(&m.node_id) {
                node.position = NodePosition { x: m.x, y: m.y };
                moved += 1;
            }
        }
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已移动 {} 个节点", moved)))
    }

    fn name(&self) -> &str {
        "DraftBatchMoveNodes"
    }
}

// ── DraftSetNodeDisplayName ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetNodeDisplayNameInput {
    pub node_id: String,
    pub display_name: Option<String>,
}

#[buns_system("DraftSetNodeDisplayName")]
pub struct DraftSetNodeDisplayNameSystem;

#[async_trait]
impl SystemOperation for DraftSetNodeDisplayNameSystem {
    type Input = DraftSetNodeDisplayNameInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        match draft.blueprint.find_node_mut(&input.node_id) {
            Some(node) => {
                node.display_name = input.display_name;
                draft.blueprint.update_modified_time();
                write_draft_with_undo(ctx, &draft)?;
                Ok(DraftOpOutput::ok(format!(
                    "节点 {} 显示名已更新",
                    input.node_id
                )))
            }
            None => Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
        }
    }

    fn name(&self) -> &str {
        "DraftSetNodeDisplayName"
    }
}

// ── DraftSetNodeComment ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetNodeCommentInput {
    pub node_id: String,
    pub comment: Option<String>,
}

#[buns_system("DraftSetNodeComment")]
pub struct DraftSetNodeCommentSystem;

#[async_trait]
impl SystemOperation for DraftSetNodeCommentSystem {
    type Input = DraftSetNodeCommentInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        match draft.blueprint.find_node_mut(&input.node_id) {
            Some(node) => {
                node.comment = input.comment;
                draft.blueprint.update_modified_time();
                write_draft_with_undo(ctx, &draft)?;
                Ok(DraftOpOutput::ok(format!(
                    "节点 {} 注释已更新",
                    input.node_id
                )))
            }
            None => Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
        }
    }

    fn name(&self) -> &str {
        "DraftSetNodeComment"
    }
}

// ── DraftSetNodeProperty ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetNodePropertyInput {
    pub node_id: String,
    pub key: String,
    pub value: JsonValue,
}

#[buns_system("DraftSetNodeProperty")]
pub struct DraftSetNodePropertySystem;

#[async_trait]
impl SystemOperation for DraftSetNodePropertySystem {
    type Input = DraftSetNodePropertyInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        match draft.blueprint.find_node_mut(&input.node_id) {
            Some(node) => {
                node.properties.insert(input.key.clone(), input.value);
                draft.blueprint.update_modified_time();
                write_draft_with_undo(ctx, &draft)?;
                Ok(DraftOpOutput::ok(format!(
                    "节点 {} 属性 {} 已更新",
                    input.node_id, input.key
                )))
            }
            None => Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
        }
    }

    fn name(&self) -> &str {
        "DraftSetNodeProperty"
    }
}

// ============================================================================
// ============================================================================

// ── DraftAddPin ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftAddPinInput {
    pub node_id: String,
    pub pin_name: String,
    pub kind: String, // "ExecInput" | "ExecOutput" | "DataInput" | "DataOutput"
    pub data_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,
}

#[buns_system("DraftAddPin")]
pub struct DraftAddPinSystem;

#[async_trait]
impl SystemOperation for DraftAddPinSystem {
    type Input = DraftAddPinInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let node = match draft.blueprint.find_node_mut(&input.node_id) {
            Some(n) => n,
            None => return Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
        };

        // 权限检查
        let required = match input.kind.as_str() {
            "DataInput" | "ExecInput" => NodePermissions::CAN_ADD_INPUT_PIN,
            "DataOutput" | "ExecOutput" => NodePermissions::CAN_ADD_OUTPUT_PIN,
            other => return Ok(DraftOpOutput::err(format!("无效的引脚类型: {}", other))),
        };
        if let Some(meta) = NodeRegistry::get(&node.node_type) {
            if !meta.permissions.has(required) {
                return Ok(DraftOpOutput::err(format!(
                    "节点类型 {} 不允许添加此类引脚",
                    node.node_type
                )));
            }
        }

        // 检查重名
        if node.pins.iter().any(|p| p.name == input.pin_name) {
            return Ok(DraftOpOutput::err(format!(
                "引脚名已存在: {}",
                input.pin_name
            )));
        }

        node.pins.push(NodePin {
            name: input.pin_name.clone(),
            kind: input.kind,
            data_type: input.data_type,
            description: input.description,
            default_value: input.default_value,
            resolved_type: None,
            split_config: None,
        });

        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已添加引脚 {}", input.pin_name)))
    }

    fn name(&self) -> &str {
        "DraftAddPin"
    }
}

// ── DraftRemovePin ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRemovePinInput {
    pub node_id: String,
    pub pin_name: String,
}

#[buns_system("DraftRemovePin")]
pub struct DraftRemovePinSystem;

#[async_trait]
impl SystemOperation for DraftRemovePinSystem {
    type Input = DraftRemovePinInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;

        // 查找引脚并检查权限
        let (pin_kind, node_type) = {
            let node = match draft.blueprint.find_node(&input.node_id) {
                Some(n) => n,
                None => return Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
            };
            let pin = match node.pins.iter().find(|p| p.name == input.pin_name) {
                Some(p) => p,
                None => {
                    return Ok(DraftOpOutput::err(format!(
                        "引脚不存在: {}",
                        input.pin_name
                    )))
                }
            };
            (pin.kind.clone(), node.node_type.clone())
        };

        let required = match pin_kind.as_str() {
            "DataInput" | "ExecInput" => NodePermissions::CAN_REMOVE_INPUT_PIN,
            _ => NodePermissions::CAN_REMOVE_OUTPUT_PIN,
        };
        if let Some(meta) = NodeRegistry::get(&node_type) {
            if !meta.permissions.has(required) {
                return Ok(DraftOpOutput::err(format!(
                    "节点类型 {} 不允许删除此类引脚",
                    node_type
                )));
            }
        }

        // 删除引脚
        if let Some(node) = draft.blueprint.find_node_mut(&input.node_id) {
            node.pins.retain(|p| p.name != input.pin_name);
        }

        // 删除关联连线
        let nid = &input.node_id;
        let pname = &input.pin_name;
        draft.blueprint.connections.retain(|c| {
            !((c.source_node == *nid && c.source_pin == *pname)
                || (c.target_node == *nid && c.target_pin == *pname))
        });

        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已删除引脚 {}", input.pin_name)))
    }

    fn name(&self) -> &str {
        "DraftRemovePin"
    }
}

// ── DraftSetPinDefault ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetPinDefaultInput {
    pub node_id: String,
    pub pin_name: String,
    pub value: Option<JsonValue>,
}

#[buns_system("DraftSetPinDefault")]
pub struct DraftSetPinDefaultSystem;

#[async_trait]
impl SystemOperation for DraftSetPinDefaultSystem {
    type Input = DraftSetPinDefaultInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let node = match draft.blueprint.find_node_mut(&input.node_id) {
            Some(n) => n,
            None => return Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
        };
        let pin = match node.pins.iter_mut().find(|p| p.name == input.pin_name) {
            Some(p) => p,
            None => {
                return Ok(DraftOpOutput::err(format!(
                    "引脚不存在: {}",
                    input.pin_name
                )))
            }
        };
        pin.default_value = input.value;

        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "引脚 {} 默认值已更新",
            input.pin_name
        )))
    }

    fn name(&self) -> &str {
        "DraftSetPinDefault"
    }
}

// ── DraftSetPinType ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetPinTypeInput {
    pub node_id: String,
    pub pin_name: String,
    pub data_type: String,
}

#[buns_system("DraftSetPinType")]
pub struct DraftSetPinTypeSystem;

#[async_trait]
impl SystemOperation for DraftSetPinTypeSystem {
    type Input = DraftSetPinTypeInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;

        // 权限检查
        {
            let node = match draft.blueprint.find_node(&input.node_id) {
                Some(n) => n,
                None => return Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
            };
            if let Some(meta) = NodeRegistry::get(&node.node_type) {
                if !meta.permissions.has(NodePermissions::CAN_EDIT_PIN_TYPE) {
                    return Ok(DraftOpOutput::err(format!(
                        "节点类型 {} 不允许修改引脚类型",
                        node.node_type
                    )));
                }
            }
        }

        let node = draft.blueprint.find_node_mut(&input.node_id).unwrap();
        let pin = match node.pins.iter_mut().find(|p| p.name == input.pin_name) {
            Some(p) => p,
            None => {
                return Ok(DraftOpOutput::err(format!(
                    "引脚不存在: {}",
                    input.pin_name
                )))
            }
        };
        pin.data_type = input.data_type.clone();

        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "引脚 {} 类型已设为 {}",
            input.pin_name, input.data_type
        )))
    }

    fn name(&self) -> &str {
        "DraftSetPinType"
    }
}

// ── DraftSetPinName ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSetPinNameInput {
    pub node_id: String,
    pub old_name: String,
    pub new_name: String,
}

#[buns_system("DraftSetPinName")]
pub struct DraftSetPinNameSystem;

#[async_trait]
impl SystemOperation for DraftSetPinNameSystem {
    type Input = DraftSetPinNameInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;

        // 权限检查
        {
            let node = match draft.blueprint.find_node(&input.node_id) {
                Some(n) => n,
                None => return Ok(DraftOpOutput::err(format!("节点不存在: {}", input.node_id))),
            };
            if let Some(meta) = NodeRegistry::get(&node.node_type) {
                if !meta.permissions.has(NodePermissions::CAN_EDIT_PIN_NAME) {
                    return Ok(DraftOpOutput::err(format!(
                        "节点类型 {} 不允许修改引脚名称",
                        node.node_type
                    )));
                }
            }
            if node.pins.iter().any(|p| p.name == input.new_name) {
                return Ok(DraftOpOutput::err(format!(
                    "引脚名已存在: {}",
                    input.new_name
                )));
            }
        }

        // 重命名引脚
        let node = draft.blueprint.find_node_mut(&input.node_id).unwrap();
        match node.pins.iter_mut().find(|p| p.name == input.old_name) {
            Some(p) => p.name = input.new_name.clone(),
            None => {
                return Ok(DraftOpOutput::err(format!(
                    "引脚不存在: {}",
                    input.old_name
                )))
            }
        }

        // 更新引用该引脚的连线
        let nid = &input.node_id;
        for conn in &mut draft.blueprint.connections {
            if conn.source_node == *nid && conn.source_pin == input.old_name {
                conn.source_pin = input.new_name.clone();
            }
            if conn.target_node == *nid && conn.target_pin == input.old_name {
                conn.target_pin = input.new_name.clone();
            }
        }

        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "引脚 {} 已重命名为 {}",
            input.old_name, input.new_name
        )))
    }

    fn name(&self) -> &str {
        "DraftSetPinName"
    }
}

// ============================================================================
// ============================================================================

// ── DraftConnect ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftConnectInput {
    pub source_node: String,
    pub source_pin: String,
    pub target_node: String,
    pub target_pin: String,
}

#[buns_system("DraftConnect")]
pub struct DraftConnectSystem;

#[async_trait]
impl SystemOperation for DraftConnectSystem {
    type Input = DraftConnectInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;

        // 推断 connection_type
        let conn_type = {
            let src_node = draft.blueprint.find_node(&input.source_node);
            if let Some(n) = src_node {
                if n.pins
                    .iter()
                    .any(|p| p.name == input.source_pin && p.kind == "ExecOutput")
                {
                    "Exec"
                } else {
                    "Data"
                }
            } else {
                "Data"
            }
        }
        .to_string();

        match draft::add_connection_with_rules(
            &mut draft.blueprint,
            input.source_node.clone(),
            input.source_pin.clone(),
            input.target_node.clone(),
            input.target_pin.clone(),
            conn_type,
        ) {
            Ok(result) => {
                let conn_id = draft
                    .blueprint
                    .connections
                    .last()
                    .map(|c| c.id.clone())
                    .unwrap_or_default();
                let mut msg = format!(
                    "已连线 {}.{} → {}.{}",
                    input.source_node, input.source_pin, input.target_node, input.target_pin
                );
                for r in &result.replaced {
                    msg.push_str(&format!("\n  {}", r));
                }
                let bp = draft.blueprint.clone();
                draft.update_from_blueprint_lossy(bp);
                write_draft_with_undo(ctx, &draft)?;
                Ok(DraftOpOutput::ok_with_id(msg, conn_id))
            }
            Err(e) => Ok(DraftOpOutput::err(e)),
        }
    }

    fn name(&self) -> &str {
        "DraftConnect"
    }
}

// ── DraftDisconnect ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftDisconnectInput {
    pub source_node: String,
    pub source_pin: String,
    pub target_node: String,
    pub target_pin: String,
}

#[buns_system("DraftDisconnect")]
pub struct DraftDisconnectSystem;

#[async_trait]
impl SystemOperation for DraftDisconnectSystem {
    type Input = DraftDisconnectInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let before = draft.blueprint.connections.len();
        draft.blueprint.connections.retain(|c| {
            !(c.source_node == input.source_node
                && c.source_pin == input.source_pin
                && c.target_node == input.target_node
                && c.target_pin == input.target_pin)
        });
        let removed = before - draft.blueprint.connections.len();
        if removed == 0 {
            return Ok(DraftOpOutput::err("未找到匹配的连线"));
        }
        draft.blueprint.update_modified_time();
        let bp = draft.blueprint.clone();
        draft.update_from_blueprint_lossy(bp);
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已断开 {} 条连线", removed)))
    }

    fn name(&self) -> &str {
        "DraftDisconnect"
    }
}

// ============================================================================
// ============================================================================

// ── DraftAddComment ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftAddCommentInput {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[buns_system("DraftAddComment")]
pub struct DraftAddCommentSystem;

#[async_trait]
impl SystemOperation for DraftAddCommentSystem {
    type Input = DraftAddCommentInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let id = next_comment_id(&draft);
        draft.blueprint.comments.push(CommentBox {
            id: id.clone(),
            text: input.text,
            position: NodePosition {
                x: input.x,
                y: input.y,
            },
            size: CommentSize {
                width: input.width,
                height: input.height,
            },
            color: input.color,
        });
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok_with_id("已添加注释", &id))
    }

    fn name(&self) -> &str {
        "DraftAddComment"
    }
}

// ── DraftRemoveComment ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRemoveCommentInput {
    pub comment_id: String,
}

#[buns_system("DraftRemoveComment")]
pub struct DraftRemoveCommentSystem;

#[async_trait]
impl SystemOperation for DraftRemoveCommentSystem {
    type Input = DraftRemoveCommentInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let before = draft.blueprint.comments.len();
        draft
            .blueprint
            .comments
            .retain(|c| c.id != input.comment_id);
        if draft.blueprint.comments.len() == before {
            return Ok(DraftOpOutput::err(format!(
                "注释不存在: {}",
                input.comment_id
            )));
        }
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "已删除注释 {}",
            input.comment_id
        )))
    }

    fn name(&self) -> &str {
        "DraftRemoveComment"
    }
}

// ── DraftUpdateComment ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftUpdateCommentInput {
    pub comment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[buns_system("DraftUpdateComment")]
pub struct DraftUpdateCommentSystem;

#[async_trait]
impl SystemOperation for DraftUpdateCommentSystem {
    type Input = DraftUpdateCommentInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let comment = match draft
            .blueprint
            .comments
            .iter_mut()
            .find(|c| c.id == input.comment_id)
        {
            Some(c) => c,
            None => {
                return Ok(DraftOpOutput::err(format!(
                    "注释不存在: {}",
                    input.comment_id
                )))
            }
        };
        if let Some(text) = input.text {
            comment.text = text;
        }
        if let Some(x) = input.x {
            comment.position.x = x;
        }
        if let Some(y) = input.y {
            comment.position.y = y;
        }
        if let Some(w) = input.width {
            comment.size.width = w;
        }
        if let Some(h) = input.height {
            comment.size.height = h;
        }
        if input.color.is_some() {
            comment.color = input.color;
        }
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "注释 {} 已更新",
            input.comment_id
        )))
    }

    fn name(&self) -> &str {
        "DraftUpdateComment"
    }
}

// ── DraftAddVariable ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftAddVariableInput {
    pub name: String,
    pub data_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,
    #[serde(default)]
    pub description: String,
}

#[buns_system("DraftAddVariable")]
pub struct DraftAddVariableSystem;

#[async_trait]
impl SystemOperation for DraftAddVariableSystem {
    type Input = DraftAddVariableInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        if draft
            .blueprint
            .variables
            .iter()
            .any(|v| v.name == input.name)
        {
            return Ok(DraftOpOutput::err(format!("变量已存在: {}", input.name)));
        }
        draft.blueprint.variables.push(BlueprintVariable {
            name: input.name.clone(),
            data_type: input.data_type,
            default_value: input.default_value,
            description: input.description,
        });
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok_with_id(
            format!("已添加变量 {}", input.name),
            input.name,
        ))
    }

    fn name(&self) -> &str {
        "DraftAddVariable"
    }
}

// ── DraftRemoveVariable ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRemoveVariableInput {
    pub var_name: String,
}

#[buns_system("DraftRemoveVariable")]
pub struct DraftRemoveVariableSystem;

#[async_trait]
impl SystemOperation for DraftRemoveVariableSystem {
    type Input = DraftRemoveVariableInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let before = draft.blueprint.variables.len();
        draft
            .blueprint
            .variables
            .retain(|v| v.name != input.var_name);
        if draft.blueprint.variables.len() == before {
            return Ok(DraftOpOutput::err(format!(
                "变量不存在: {}",
                input.var_name
            )));
        }
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("已删除变量 {}", input.var_name)))
    }

    fn name(&self) -> &str {
        "DraftRemoveVariable"
    }
}

// ── DraftUpdateVariable ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftUpdateVariableInput {
    pub var_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[buns_system("DraftUpdateVariable")]
pub struct DraftUpdateVariableSystem;

#[async_trait]
impl SystemOperation for DraftUpdateVariableSystem {
    type Input = DraftUpdateVariableInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let var = match draft
            .blueprint
            .variables
            .iter_mut()
            .find(|v| v.name == input.var_name)
        {
            Some(v) => v,
            None => {
                return Ok(DraftOpOutput::err(format!(
                    "变量不存在: {}",
                    input.var_name
                )))
            }
        };
        if let Some(dt) = input.data_type {
            var.data_type = dt;
        }
        if input.default_value.is_some() {
            var.default_value = input.default_value;
        }
        if let Some(desc) = input.description {
            var.description = desc;
        }
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!("变量 {} 已更新", input.var_name)))
    }

    fn name(&self) -> &str {
        "DraftUpdateVariable"
    }
}

// ============================================================================
//
// 对外一律用 `kind: "input" | "return" | "var"`；内部 outputs 字段在
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContractKind {
    Input,
    Return,
    Var,
}

impl ContractKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Return => "return",
            Self::Var => "var",
        }
    }
}

fn contract_name_exists(draft: &WorkflowDraft, name: &str) -> Option<&'static str> {
    if draft
        .blueprint
        .metadata
        .inputs
        .iter()
        .any(|p| p.name == name)
    {
        return Some("input");
    }
    if draft
        .blueprint
        .metadata
        .outputs
        .iter()
        .any(|p| p.name == name)
    {
        return Some("return");
    }
    if draft.blueprint.variables.iter().any(|v| v.name == name) {
        return Some("var");
    }
    None
}

// ── DraftDeclareInput ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftDeclareInputInput {
    pub name: String,
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[buns_system("DraftDeclareInput")]
pub struct DraftDeclareInputSystem;

#[async_trait]
impl SystemOperation for DraftDeclareInputSystem {
    type Input = DraftDeclareInputInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        if let Some(existing) = contract_name_exists(&draft, &input.name) {
            return Ok(DraftOpOutput::err(format!(
                "契约项 `{}` 已存在于 {}，不能重复声明",
                input.name, existing
            )));
        }
        draft.blueprint.metadata.inputs.push(PinMetadata {
            name: input.name.clone(),
            data_type: input.data_type,
            description: input.comment.unwrap_or_default(),
            default_value: None,
        });
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok_with_id(
            format!("已声明 input `{}`", input.name),
            input.name,
        ))
    }

    fn name(&self) -> &str {
        "DraftDeclareInput"
    }
}

// ── DraftDeclareReturn ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftDeclareReturnInput {
    pub name: String,
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[buns_system("DraftDeclareReturn")]
pub struct DraftDeclareReturnSystem;

#[async_trait]
impl SystemOperation for DraftDeclareReturnSystem {
    type Input = DraftDeclareReturnInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        if let Some(existing) = contract_name_exists(&draft, &input.name) {
            return Ok(DraftOpOutput::err(format!(
                "契约项 `{}` 已存在于 {}，不能重复声明",
                input.name, existing
            )));
        }
        // 内部字段仍叫 outputs，对外称 return
        draft.blueprint.metadata.outputs.push(PinMetadata {
            name: input.name.clone(),
            data_type: input.data_type,
            description: input.comment.unwrap_or_default(),
            default_value: None,
        });
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok_with_id(
            format!("已声明 return `{}`", input.name),
            input.name,
        ))
    }

    fn name(&self) -> &str {
        "DraftDeclareReturn"
    }
}

// ── DraftDeclareVar ──────────────────────────────────────────────────────────
//
// 语义等价于 DraftAddVariable，额外限制 name 在契约全局唯一（不与 input/return 撞名）。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftDeclareVarInput {
    pub name: String,
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[buns_system("DraftDeclareVar")]
pub struct DraftDeclareVarSystem;

#[async_trait]
impl SystemOperation for DraftDeclareVarSystem {
    type Input = DraftDeclareVarInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        if let Some(existing) = contract_name_exists(&draft, &input.name) {
            return Ok(DraftOpOutput::err(format!(
                "契约项 `{}` 已存在于 {}，不能重复声明",
                input.name, existing
            )));
        }
        draft.blueprint.variables.push(BlueprintVariable {
            name: input.name.clone(),
            data_type: input.data_type,
            default_value: input.default_value,
            description: input.comment.unwrap_or_default(),
        });
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok_with_id(
            format!("已声明 var `{}`", input.name),
            input.name,
        ))
    }

    fn name(&self) -> &str {
        "DraftDeclareVar"
    }
}

// ── DraftUpdateContract ──────────────────────────────────────────────────────
//
// 按 name 在三类中定位后，更新 data_type / default_value（仅 var）/ comment。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftUpdateContractInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// 仅对 var 有效；input/return 收到 default_value 时返回错误提示
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[buns_system("DraftUpdateContract")]
pub struct DraftUpdateContractSystem;

#[async_trait]
impl SystemOperation for DraftUpdateContractSystem {
    type Input = DraftUpdateContractInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let Some(kind) = contract_name_exists(&draft, &input.name) else {
            return Ok(DraftOpOutput::err(format!(
                "契约项 `{}` 不存在",
                input.name
            )));
        };

        match kind {
            "input" => {
                if input.default_value.is_some() {
                    return Ok(DraftOpOutput::err(
                        "input 不支持 default_value；如需默认值请改为 var".to_string(),
                    ));
                }
                let pin = draft
                    .blueprint
                    .metadata
                    .inputs
                    .iter_mut()
                    .find(|p| p.name == input.name)
                    .expect("exists checked");
                if let Some(dt) = input.data_type {
                    pin.data_type = dt;
                }
                if let Some(c) = input.comment {
                    pin.description = c;
                }
            }
            "return" => {
                if input.default_value.is_some() {
                    return Ok(DraftOpOutput::err(
                        "return 不支持 default_value".to_string(),
                    ));
                }
                let pin = draft
                    .blueprint
                    .metadata
                    .outputs
                    .iter_mut()
                    .find(|p| p.name == input.name)
                    .expect("exists checked");
                if let Some(dt) = input.data_type {
                    pin.data_type = dt;
                }
                if let Some(c) = input.comment {
                    pin.description = c;
                }
            }
            "var" => {
                let v = draft
                    .blueprint
                    .variables
                    .iter_mut()
                    .find(|v| v.name == input.name)
                    .expect("exists checked");
                if let Some(dt) = input.data_type {
                    v.data_type = dt;
                }
                if input.default_value.is_some() {
                    v.default_value = input.default_value;
                }
                if let Some(c) = input.comment {
                    v.description = c;
                }
            }
            _ => unreachable!(),
        }

        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "契约项 `{}` ({}) 已更新",
            input.name, kind
        )))
    }

    fn name(&self) -> &str {
        "DraftUpdateContract"
    }
}

// ── DraftRemoveContract ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRemoveContractInput {
    pub name: String,
}

#[buns_system("DraftRemoveContract")]
pub struct DraftRemoveContractSystem;

#[async_trait]
impl SystemOperation for DraftRemoveContractSystem {
    type Input = DraftRemoveContractInput;
    type Output = DraftOpOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;
        let Some(kind) = contract_name_exists(&draft, &input.name) else {
            return Ok(DraftOpOutput::err(format!(
                "契约项 `{}` 不存在",
                input.name
            )));
        };
        match kind {
            "input" => draft
                .blueprint
                .metadata
                .inputs
                .retain(|p| p.name != input.name),
            "return" => draft
                .blueprint
                .metadata
                .outputs
                .retain(|p| p.name != input.name),
            "var" => draft.blueprint.variables.retain(|v| v.name != input.name),
            _ => unreachable!(),
        }
        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;
        Ok(DraftOpOutput::ok(format!(
            "已删除契约项 `{}` ({})",
            input.name, kind
        )))
    }

    fn name(&self) -> &str {
        "DraftRemoveContract"
    }
}

// ── DraftRenameContract ──────────────────────────────────────────────────────
//
// 采用"报告模式"：扫描 chain_text 找出所有受影响的 `$(input.old)` / `$(var.old)` / `$old`
// 引用位置，交由 agent 决定是否同步改脚本。
//

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRenameContractInput {
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefLocation {
    /// 行号（1-based）
    pub line: u32,
    /// 引用语法样例，便于 agent 搜索替换
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRenameContractOutput {
    pub success: bool,
    pub message: String,
    /// 受影响的脚本引用位置（调用方按需同步 WriteScript 修改）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_refs: Vec<RefLocation>,
}

#[buns_system("DraftRenameContract")]
pub struct DraftRenameContractSystem;

#[async_trait]
impl SystemOperation for DraftRenameContractSystem {
    type Input = DraftRenameContractInput;
    type Output = DraftRenameContractOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;

        let Some(kind) = contract_name_exists(&draft, &input.old) else {
            return Ok(DraftRenameContractOutput {
                success: false,
                message: format!("契约项 `{}` 不存在", input.old),
                affected_refs: Vec::new(),
            });
        };
        if contract_name_exists(&draft, &input.new).is_some() {
            return Ok(DraftRenameContractOutput {
                success: false,
                message: format!("目标名 `{}` 已被占用", input.new),
                affected_refs: Vec::new(),
            });
        }

        // 先扫脚本收集受影响引用（改名前的快照）
        let affected = scan_refs(&draft.chain_text, kind, &input.old);

        // 改契约项 name
        match kind {
            "input" => {
                if let Some(p) = draft
                    .blueprint
                    .metadata
                    .inputs
                    .iter_mut()
                    .find(|p| p.name == input.old)
                {
                    p.name = input.new.clone();
                }
            }
            "return" => {
                if let Some(p) = draft
                    .blueprint
                    .metadata
                    .outputs
                    .iter_mut()
                    .find(|p| p.name == input.old)
                {
                    p.name = input.new.clone();
                }
            }
            "var" => {
                if let Some(v) = draft
                    .blueprint
                    .variables
                    .iter_mut()
                    .find(|v| v.name == input.old)
                {
                    v.name = input.new.clone();
                }
            }
            _ => unreachable!(),
        }

        draft.blueprint.update_modified_time();
        write_draft_with_undo(ctx, &draft)?;

        let msg = if affected.is_empty() {
            format!(
                "已改名 `{}` → `{}`（无脚本引用需同步）",
                input.old, input.new
            )
        } else {
            format!(
                "已改名 `{}` → `{}`，脚本中有 {} 处引用需手动同步，请调用 WriteScript 修正",
                input.old,
                input.new,
                affected.len()
            )
        };

        Ok(DraftRenameContractOutput {
            success: true,
            message: msg,
            affected_refs: affected,
        })
    }

    fn name(&self) -> &str {
        "DraftRenameContract"
    }
}

/// 扫描 chain_text 里对契约项 `name` 的所有引用位置。
///
/// - `input`  → 匹配 `input.{name}`
/// - `return` → return 只在 `RETURN result=...` 左侧出现，这里也匹配 `result=` 形式
/// - `var`    → 匹配 `${name}` 和 `{name}` 在表达式中（保守：至少匹配 `${name}`）
fn scan_refs(chain_text: &str, kind: &str, name: &str) -> Vec<RefLocation> {
    let mut out = Vec::new();
    let pat_owned: Vec<String> = match kind {
        "input" => vec![format!("input.{}", name)],
        "return" => vec![format!("{}=", name), format!("{} =", name)],
        "var" => vec![format!("${}", name), format!("$({}.", name)],
        _ => Vec::new(),
    };
    for (idx, line) in chain_text.lines().enumerate() {
        for pat in &pat_owned {
            if line.contains(pat) {
                out.push(RefLocation {
                    line: (idx + 1) as u32,
                    snippet: line.trim().to_string(),
                });
                break;
            }
        }
    }
    out
}

// ── DraftWriteScript ─────────────────────────────────────────────────────────
//
// 保留 `base_version` 可做乐观锁（未来 Phase 5 引入）。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftWriteScriptInput {
    pub text: String,
    /// 乐观锁基准版本号；为 0 跳过校验
    #[serde(default)]
    pub base_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftWriteScriptOutput {
    pub success: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<corework::workflow::chain_compiler::ChainError>,
    /// 成功后的新快照文本（方便 agent 下一轮直接用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_text: Option<String>,
    /// 成功后的新快照版本号
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

#[buns_system("DraftWriteScript")]
pub struct DraftWriteScriptSystem;

#[async_trait]
impl SystemOperation for DraftWriteScriptSystem {
    type Input = DraftWriteScriptInput;
    type Output = DraftWriteScriptOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &Context,
    ) -> Result<Self::Output, Self::Error> {
        let mut draft = read_draft(ctx)?;

        // 乐观锁检查（base_version == 0 跳过）
        if input.base_version > 0 {
            let cur_ver = crate::workflow::workflows::snapshot::current_version(ctx);
            if cur_ver != input.base_version {
                return Ok(DraftWriteScriptOutput {
                    success: false,
                    message: format!(
                        "版本冲突：提交基于 v{}, 当前 v{}，请重读快照后重试",
                        input.base_version, cur_ver
                    ),
                    errors: Vec::new(),
                    snapshot_text: None,
                    version: None,
                });
            }
        }

        match draft.update_from_text(&input.text) {
            Ok(_) => {
                draft.blueprint.update_modified_time();
                write_draft_with_undo(ctx, &draft)?;
                // write_draft_with_undo 内部已 refresh_world_snapshot
                let snap = crate::workflow::workflows::snapshot::current_snapshot(ctx);
                Ok(DraftWriteScriptOutput {
                    success: true,
                    message: "脚本已更新".to_string(),
                    errors: Vec::new(),
                    snapshot_text: snap.as_ref().map(|s| s.text.clone()),
                    version: snap.map(|s| s.version),
                })
            }
            Err(e) => Ok(DraftWriteScriptOutput {
                success: false,
                message: "脚本编译失败，详见 errors".to_string(),
                errors: vec![e],
                snapshot_text: None,
                version: None,
            }),
        }
    }

    fn name(&self) -> &str {
        "DraftWriteScript"
    }
}
