//! 工作流命令的共用响应类型（不含 Tauri 注册，注册在外层完成）

use serde::Serialize;
use std::collections::HashMap;

/// 蓝图加载响应
#[derive(Debug, Serialize)]
pub struct LoadBlueprintResponse {
    pub success: bool,
    pub key: Option<String>,
    pub error: Option<String>,
}

/// 蓝图执行响应
#[derive(Debug, Serialize)]
pub struct ExecuteBlueprintResponse {
    pub success: bool,
    pub outputs: HashMap<String, serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// 输入验证响应
#[derive(Debug, Serialize)]
pub struct ValidateInputsResponse {
    pub valid: bool,
    pub errors: Vec<String>,
}

// ── 草稿操作响应类型（供 Tauri commands 使用）────────────────────────────────

pub use crate::workflow::workflows::draft_ops::{DraftGetOutput, DraftOpOutput, NodeMoveItem};

// ── 节点注册表响应类型 ──────────────────────────────────────────────────────

/// 单个引脚信息（前端用）
#[derive(Debug, Serialize)]
pub struct PinInfo {
    pub name: String,
    pub kind: String, // "ExecInput" | "ExecOutput" | "DataInput" | "DataOutput"
    pub data_type: String,
    pub description: String,
    pub default_value: Option<serde_json::Value>,
}

/// 单个节点类型信息（前端用）
#[derive(Debug, Serialize)]
pub struct NodeTypeInfo {
    pub node_type: String,
    pub display_name: String,
    pub description: String,
    pub category: String,
    pub pins: Vec<PinInfo>,
    pub permissions: u8,
}

/// 节点分类信息
#[derive(Debug, Serialize)]
pub struct CategoryInfo {
    pub name: String,
    pub description: String,
    pub node_count: usize,
}

/// 节点目录响应（按分类分组）
#[derive(Debug, Serialize)]
pub struct NodeCatalogResponse {
    pub categories: Vec<CategoryInfo>,
    pub nodes: Vec<NodeTypeInfo>,
}

// ── 流程图响应类型 ──────────────────────────────────────────────────────────

pub use crate::workflow::workflows::flowchart::{
    FlowchartData, FlowchartEdge, FlowchartNode, FlowchartNodeType,
};
