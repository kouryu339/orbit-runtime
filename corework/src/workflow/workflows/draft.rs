//! 工作流草稿 —— 键常量 + 纯函数
//!
//! 本模块不做任何 I/O：
//!
//! 通过 ExecutionUnit 执行，保证调用链可溯源。

use std::collections::HashMap;

use corework::workflow::blueprint_json::{BlueprintJson, BlueprintVariable, PinMetadata};
use corework::workflow::chain_compiler::{ChainError, ChainErrorKind};
use corework::workflow::chain_compiler_v2::compile_chain_v2;
use corework::workflow::chain_decompiler::{decompile_chain, DecompileError};
use corework::workflow::registry::{NodePermissions, NodeRegistry, PinKind};
use serde::{Deserialize, Serialize};

// ============================================================================
// 存储键
// ============================================================================

pub mod keys {
    /// 当前草稿 WorkflowDraft（World 资源，JSON DAG 为主体，文本为临时 AI 视图）
    pub const DRAFT: &str = "wf_draft";
    /// 上一个追加节点的 id（用于自动连线，World 资源）
    pub const CURSOR: &str = "wf_cursor";
    /// 撤销历史栈 Vec<WorkflowDraft>（World 资源）
    pub const HISTORY: &str = "wf_history";
    /// 已使用节点类型 → 完整引脚描述（HashMap<node_type, detail_text>，World 资源）
    pub const USED_NODE_DETAILS: &str = "wf_used_node_details";
    /// 子 Agent 接收的用户目标描述
    pub const INTENT: &str = "wf_intent";
    /// 子 Agent 完成后写入的最终结果摘要
    pub const RESULT: &str = "wf_result";
    pub const SNAPSHOT: &str = "wf_draft_snapshot";
    /// 单调递增的快照版本号（`u64`，每次草稿变更 +1）
    pub const SNAPSHOT_VERSION: &str = "wf_draft_snap_ver";
}

// ============================================================================
// WorkflowDraft —— JSON DAG 主体与临时 AI 文本视图
// ============================================================================

/// 工作流草稿。`BlueprintJson` 是可持久化主体，操作链文本是临时 AI 视图。
///
/// - AI/Recorder 写文本 → `update_from_text` → 自动 compile 更新 JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDraft {
    pub blueprint: BlueprintJson,
    pub chain_text: String,
}

impl WorkflowDraft {
    /// 新建空草稿
    pub fn new(name: &str) -> Self {
        Self {
            blueprint: BlueprintJson::new(name),
            chain_text: String::new(),
        }
    }

    /// 从文本更新（AI/Recorder 写入路径）。
    pub fn update_from_text(&mut self, text: &str) -> Result<(), ChainError> {
        let saved_meta = self.blueprint.metadata.clone();
        let full_text = build_compilable_chain_v2(text, &saved_meta.inputs)?;
        let mut bp = match compile_chain_v2(&full_text) {
            Ok(bp) => bp,
            Err(mut e) => {
                if e.line > 1 {
                    e.line -= 1;
                }
                return Err(e);
            }
        };
        self.chain_text = text.to_string();
        bp.metadata = saved_meta;
        bp.variables = extract_variables_from_script(text);
        self.blueprint = bp;
        Ok(())
    }

    pub fn update_from_blueprint(&mut self, bp: BlueprintJson) -> Result<(), DecompileError> {
        let text = decompile_chain(&bp)?;
        self.blueprint = bp;
        self.chain_text = text;
        Ok(())
    }

    /// 从 BlueprintJson 更新，decompile 失败时标记文本为错误提示而非 panic。
    pub fn update_from_blueprint_lossy(&mut self, bp: BlueprintJson) {
        match decompile_chain(&bp) {
            Ok(text) => {
                self.blueprint = bp;
                self.chain_text = text;
            }
            Err(e) => {
                tracing::warn!("decompile 失败: {}, 文本标记为错误提示", e);
                self.blueprint = bp;
                self.chain_text = "# decompile error, please edit via DAG".to_string();
            }
        }
    }

    /// 顶层步骤数（从 chain_text 实时计算）
    pub fn top_level_step_count(&self) -> u32 {
        self.chain_text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with(' ') && !l.starts_with('\t'))
            .count() as u32
    }
}

fn build_compilable_chain_v2(
    script_text: &str,
    inputs: &[PinMetadata],
) -> Result<String, ChainError> {
    for (idx, line) in script_text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed == "INPUT" || trimmed.starts_with("INPUT ") || trimmed.starts_with("INPUT\t") {
            return Err(ChainError::of_kind(
                idx + 1,
                ChainErrorKind::Syntax,
                "`DraftWriteScript.text` 不应包含 INPUT；请使用 DraftDeclareInput 维护入参",
            ));
        }
        if trimmed == "RETURN" || trimmed.starts_with("RETURN ") || trimmed.starts_with("RETURN\t")
        {
            return Err(ChainError::of_kind(
                idx + 1,
                ChainErrorKind::Syntax,
                "`DraftWriteScript.text` 不应包含 RETURN；请使用 DraftDeclareReturn 维护返回契约",
            ));
        }
    }

    let mut full = String::from("INPUT");
    for input in inputs {
        full.push(' ');
        full.push_str(&input.name);
        full.push(':');
        full.push_str(&input.data_type);
        if let Some(default) = &input.default_value {
            full.push('=');
            full.push_str(&format_json_for_chain(default));
        }
    }
    full.push('\n');

    let body = script_text.trim_end();
    if !body.trim().is_empty() {
        full.push_str(body);
        full.push('\n');
    }
    full.push_str("RETURN\n");
    Ok(full)
}

fn extract_variables_from_script(script_text: &str) -> Vec<BlueprintVariable> {
    let mut variables = Vec::new();
    for line in script_text.lines() {
        let trimmed = line.trim();
        let Some((name, value)) = parse_var_decl_line(trimmed) else {
            continue;
        };
        if variables.iter().any(|v: &BlueprintVariable| v.name == name) {
            continue;
        }
        let (data_type, default_value) = infer_var_default(value);
        variables.push(BlueprintVariable {
            name,
            data_type,
            default_value,
            description: String::new(),
        });
    }
    variables
}

fn parse_var_decl_line(line: &str) -> Option<(String, &str)> {
    if !line.starts_with('$') {
        return None;
    }
    let eq = line.find('=')?;
    let name = line[1..eq].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((name.to_string(), line[eq + 1..].trim()))
}

fn infer_var_default(value: &str) -> (String, Option<serde_json::Value>) {
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return match serde_json::from_str::<serde_json::Value>(value) {
            Ok(v) => ("String".to_string(), Some(v)),
            Err(_) => (
                "String".to_string(),
                Some(serde_json::Value::String(
                    value[1..value.len() - 1].to_string(),
                )),
            ),
        };
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return (
            "String".to_string(),
            Some(serde_json::Value::String(
                value[1..value.len() - 1].to_string(),
            )),
        );
    }
    if value == "true" || value == "false" {
        return (
            "bool".to_string(),
            Some(serde_json::Value::Bool(value == "true")),
        );
    }
    if value == "null" {
        return ("Any".to_string(), Some(serde_json::Value::Null));
    }
    if let Ok(n) = value.parse::<i64>() {
        return ("num".to_string(), Some(serde_json::Value::Number(n.into())));
    }
    if let Ok(n) = value.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return ("num".to_string(), Some(serde_json::Value::Number(num)));
        }
    }
    if value.starts_with('[') || value.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
            let data_type = if v.is_array() { "Array<Any>" } else { "Any" };
            return (data_type.to_string(), Some(v));
        }
    }
    ("Any".to_string(), None)
}

fn format_json_for_chain(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
        }
        other => other.to_string(),
    }
}

// ============================================================================
// 节点目录（给 LLM 的提示词）
// ============================================================================

/// 将 permissions bits 转换为可读提示（供目录和详情使用）
fn permissions_hint(bits: u8) -> Option<String> {
    if bits == NodePermissions::NONE {
        return None;
    }
    let mut hints: Vec<&str> = Vec::new();
    if bits & NodePermissions::CAN_ADD_INPUT_PIN != 0 {
        hints.push("可添加输入引脚");
    }
    if bits & NodePermissions::CAN_REMOVE_INPUT_PIN != 0 {
        hints.push("可删除输入引脚");
    }
    if bits & NodePermissions::CAN_ADD_OUTPUT_PIN != 0 {
        hints.push("可添加输出引脚");
    }
    if bits & NodePermissions::CAN_REMOVE_OUTPUT_PIN != 0 {
        hints.push("可删除输出引脚");
    }
    if bits & NodePermissions::CAN_EDIT_PIN_TYPE != 0 {
        hints.push("可修改引脚类型");
    }
    if bits & NodePermissions::CAN_EDIT_PIN_NAME != 0 {
        hints.push("可修改引脚名称");
    }
    if hints.is_empty() {
        None
    } else {
        Some(format!("⚙ {}", hints.join("、")))
    }
}

pub fn describe_node_pins(node_type: &str) -> String {
    let Some(meta) = NodeRegistry::get(node_type) else {
        return format!("（节点 {} 未注册）", node_type);
    };
    let mut lines = Vec::new();
    for pin in meta.pins.iter() {
        match pin.kind {
            PinKind::ExecInput | PinKind::ExecOutput => {
                let dir = if matches!(pin.kind, PinKind::ExecInput) {
                    "执行入"
                } else {
                    "执行出"
                };
                if pin.description.is_empty() {
                    lines.push(format!("  [{}] `{}`", dir, pin.name));
                } else {
                    lines.push(format!("  [{}] `{}`: {}", dir, pin.name, pin.description));
                }
            }
            PinKind::DataInput | PinKind::DataOutput => {
                let dir = if matches!(pin.kind, PinKind::DataInput) {
                    "数据入"
                } else {
                    "数据出"
                };
                let public_type = crate::data_type::public_type_name(pin.data_type);
                let desc = if pin.description.is_empty() {
                    public_type.clone()
                } else {
                    pin.description.to_string()
                };
                // 精确匹配独立的 T（如 Array<T>、T），不误触 DateTime、Int 等
                let has_standalone_t = pin
                    .data_type
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|token| token == "T");
                let type_str = if has_standalone_t {
                    format!("{} [通配泛型]", public_type)
                } else {
                    public_type
                };
                let default_hint = if matches!(pin.kind, PinKind::DataInput) {
                    pin.default_value
                        .map(|v| format!("  「默认: {}」", v))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                lines.push(format!(
                    "  [{}] `{}`: {} ({}){}",
                    dir, pin.name, desc, type_str, default_hint
                ));
            }
        }
    }
    if let Some(hint) = permissions_hint(meta.permissions.bits) {
        lines.push(format!("  {}", hint));
    }
    lines.join("\n")
}

pub fn build_category_detail(category: &str) -> String {
    let mut nodes = NodeRegistry::by_category(category);
    nodes.retain(|m| m.node_type != "SequenceNode");
    if nodes.is_empty() {
        return format!("（类别 {} 中暂无节点）", category);
    }
    let mut lines = Vec::new();
    for meta in nodes {
        let perm = permissions_hint(meta.permissions.bits)
            .map(|h| format!("  ({})", h))
            .unwrap_or_default();
        lines.push(format!(
            "- `{}`{} — {}",
            meta.node_type, perm, meta.description
        ));
    }
    lines.join("\n")
}

/// 构建分层节点目录（Tier 1/Tier 2，类比主 Agent 的 skills catalog）
///
///
/// - **Tier 1（始终展示）**：`register_category!(always_visible = true)` 的分类，或未注册类别元数据的分类（向后兼容）
/// - **Tier 2（按需加载）**：`register_category!(always_visible = false)` 的分类
///   - 永远只显示摘要行，AI 调用 `WfQueryNodes` 后节点列表出现在**对话历史**中，不修改 system prompt
pub fn build_node_catalog() -> String {
    let nodes = NodeRegistry::all();
    if nodes.is_empty() {
        return "（暂无已注册节点，请先通过 register_node 宏注册节点）".to_string();
    }

    // 建立 category → node list 映射，排除隐藏节点
    let mut by_category: HashMap<&str, Vec<_>> = HashMap::new();
    for meta in &nodes {
        if meta.node_type == "SequenceNode" {
            continue;
        }
        by_category.entry(meta.category).or_default().push(*meta);
    }
    let mut sorted_cats: Vec<&&str> = by_category.keys().collect();
    sorted_cats.sort();

    // 按 always_visible 分区（未注册 CategoryMetadata 的分类默认 always_visible）
    let mut always_cats: Vec<&&str> = Vec::new();
    let mut ondemand_cats: Vec<&&str> = Vec::new();
    for cat in &sorted_cats {
        match NodeRegistry::category_meta(cat) {
            Some(m) if !m.always_visible => ondemand_cats.push(cat),
            _ => always_cats.push(cat),
        }
    }

    let mut sections: Vec<String> = Vec::new();

    // ── Tier 1：始终可见的分类 ───────────────────────────────────────────────
    if !always_cats.is_empty() {
        let mut lines = Vec::new();
        for cat in &always_cats {
            let metas = &by_category[*cat];
            let cat_desc = NodeRegistry::category_meta(cat)
                .map(|m| format!(" — {}", m.description))
                .unwrap_or_default();
            lines.push(format!("### {}{}", cat, cat_desc));
            for meta in metas {
                let perm = permissions_hint(meta.permissions.bits)
                    .map(|h| format!("  ({})", h))
                    .unwrap_or_default();
                lines.push(format!(
                    "- `{}`{} — {}",
                    meta.node_type, perm, meta.description
                ));
            }
            lines.push(String::new());
        }
        sections.push(lines.join("\n"));
    }

    // ── Tier 2：按需加载的业务节点（永远只显示摘要，节点详情进对话历史）──────
    if !ondemand_cats.is_empty() {
        let mut lines = Vec::new();
        lines.push("### 业务节点（按需加载）".to_string());
        lines.push(
            "调用 `WfQueryNodes --category <类别名>` 获取该类别完整节点列表（结果在对话中可见）。"
                .to_string(),
        );
        lines.push(String::new());
        for cat in &ondemand_cats {
            let count = by_category[*cat].len();
            let cat_meta = NodeRegistry::category_meta(cat);
            let desc = cat_meta.map(|m| m.description).unwrap_or("");
            lines.push(format!("- **{}**（{}个节点）— {}", cat, count, desc));
        }
        sections.push(lines.join("\n"));
    }

    sections.join("\n\n")
}

// ============================================================================
// 预览渲染（纯函数，接收 &BlueprintJson）
// ============================================================================

/// 渲染工作流为人类可读的文本摘要（按执行图 DFS 遍历）
pub fn render_preview(draft: &BlueprintJson) -> String {
    if draft.nodes.is_empty() {
        return format!("工作流「{}」\n（尚未添加任何节点）", draft.metadata.name);
    }

    // ── 判断节点类型 ──────────────────────────────────────────────────────────
    let is_pure_data = |node_id: &str| -> bool {
        if let Some(n) = draft.nodes.iter().find(|n| n.id == node_id) {
            !n.pins
                .iter()
                .any(|p| p.kind == "ExecInput" || p.kind == "ExecOutput")
        } else {
            false
        }
    };

    // ── 追溯 DataInput 的数据来源（递归展开纯数据节点）──────────────────────
    fn trace_data_source(
        source_node: &str,
        source_pin: &str,
        draft: &BlueprintJson,
        indent: &str,
        lines: &mut Vec<String>,
        depth: usize,
    ) {
        if depth > 8 {
            return;
        } // 防止循环
        let is_pure = !draft
            .nodes
            .iter()
            .find(|n| n.id == source_node)
            .map(|n| {
                n.pins
                    .iter()
                    .any(|p| p.kind == "ExecInput" || p.kind == "ExecOutput")
            })
            .unwrap_or(false);

        if is_pure {
            // 展开纯数据节点
            if let Some(src_node) = draft.nodes.iter().find(|n| n.id == source_node) {
                let src_display = src_node
                    .display_name
                    .as_deref()
                    .unwrap_or(&src_node.node_type);
                lines.push(format!(
                    "{}    ← [{src_node_id}] {src_display}.{source_pin} (数据节点)",
                    indent,
                    src_node_id = source_node
                ));
                // 展开该纯数据节点的 DataInput
                for p in src_node.pins.iter().filter(|p| p.kind == "DataInput") {
                    let p_desc = if p.description.is_empty() {
                        &p.data_type
                    } else {
                        &p.description
                    };
                    let upstream: Vec<_> = draft
                        .connections
                        .iter()
                        .filter(|c| c.target_node == source_node && c.target_pin == p.name)
                        .collect();
                    if !upstream.is_empty() {
                        for uc in &upstream {
                            trace_data_source(
                                &uc.source_node,
                                &uc.source_pin,
                                draft,
                                &format!("{}    ", indent),
                                lines,
                                depth + 1,
                            );
                        }
                    } else if let Some(val) = &p.default_value {
                        let v_str = match val {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        lines.push(format!("{}      .{} = {} (默认值)", indent, p.name, v_str));
                    } else {
                        lines.push(format!(
                            "{}      .{} ({}): {} 【⚠ 待完善】",
                            indent, p.name, p.data_type, p_desc
                        ));
                    }
                }
            }
        } else {
            lines.push(format!("{}    ← {}.{}", indent, source_node, source_pin));
        }
    }

    // ── 渲染单个节点的数据引脚状态 ────────────────────────────────────────────
    fn render_node_data_pins(
        node_id: &str,
        draft: &BlueprintJson,
        indent: &str,
        lines: &mut Vec<String>,
    ) {
        let node = match draft.nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n,
            None => return,
        };
        // DataOutput
        let outputs: Vec<_> = node
            .pins
            .iter()
            .filter(|p| p.kind == "DataOutput")
            .collect();
        if !outputs.is_empty() {
            for p in &outputs {
                let desc = if p.description.is_empty() {
                    &p.data_type
                } else {
                    &p.description
                };
                lines.push(format!(
                    "{}  ↳ {}.{} ({}): {}",
                    indent, node_id, p.name, p.data_type, desc
                ));
            }
        }
        // DataInput
        let inputs: Vec<_> = node.pins.iter().filter(|p| p.kind == "DataInput").collect();
        for p in &inputs {
            let desc = if p.description.is_empty() {
                &p.data_type
            } else {
                &p.description
            };
            let incomings: Vec<_> = draft
                .connections
                .iter()
                .filter(|c| c.target_node == node_id && c.target_pin == p.name)
                .collect();
            if !incomings.is_empty() {
                for conn in &incomings {
                    trace_data_source(&conn.source_node, &conn.source_pin, draft, indent, lines, 0);
                }
            } else if let Some(val) = &p.default_value {
                let v_str = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                lines.push(format!("{}  .{} = {} (默认值)", indent, p.name, v_str));
            } else {
                lines.push(format!(
                    "{}  .{} ({}): {} 【⚠ 待完善】",
                    indent, p.name, p.data_type, desc
                ));
            }
        }
    }

    // ── DFS 遍历执行图 ────────────────────────────────────────────────────────
    fn dfs(
        node_id: &str,
        draft: &BlueprintJson,
        indent: &str,
        visited: &mut std::collections::HashSet<String>,
        lines: &mut Vec<String>,
    ) {
        let node = match draft.nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n,
            None => return,
        };
        let display = node.display_name.as_deref().unwrap_or(&node.node_type);

        if visited.contains(node_id) {
            lines.push(format!(
                "{}[{}] {} ← (汇合，见上文)",
                indent, node_id, display
            ));
            return;
        }
        visited.insert(node_id.to_string());

        lines.push(format!(
            "{}[{}] {} ({})",
            indent, node_id, display, node.node_type
        ));
        render_node_data_pins(node_id, draft, indent, lines);

        // 找出所有 ExecOutput 引脚及其出向连线
        let exec_outs: Vec<_> = node
            .pins
            .iter()
            .filter(|p| p.kind == "ExecOutput")
            .collect();

        for exec_pin in &exec_outs {
            let targets: Vec<_> = draft
                .connections
                .iter()
                .filter(|c| c.source_node == node_id && c.source_pin == exec_pin.name)
                .collect();
            if targets.is_empty() {
                lines.push(format!("{}  └─[{}]→ (未连接)", indent, exec_pin.name));
            } else {
                for conn in &targets {
                    let child_indent = format!("{}  │  ", indent);
                    lines.push(format!("{}  └─[{}]→", indent, exec_pin.name));
                    dfs(&conn.target_node, draft, &child_indent, visited, lines);
                }
            }
        }
    }

    let mut lines = Vec::new();
    lines.push(format!("工作流：「{}」", draft.metadata.name));
    lines.push("─".repeat(40));

    // 从起始节点（无 ExecInput 入连线的执行节点）开始 DFS
    let entry_nodes: Vec<&str> = draft
        .nodes
        .iter()
        .filter(|n| {
            let has_exec_in = n.pins.iter().any(|p| p.kind == "ExecInput");
            let has_exec_out = n.pins.iter().any(|p| p.kind == "ExecOutput");
            let is_exec_node = has_exec_in || has_exec_out;
            if !is_exec_node {
                return false;
            }
            // 无任何 ExecInput 入连线 → 是入口
            !draft.connections.iter().any(|c| {
                c.target_node == n.id
                    && n.pins
                        .iter()
                        .any(|p| p.name == c.target_pin && p.kind == "ExecInput")
            })
        })
        .map(|n| n.id.as_str())
        .collect();

    let mut visited = std::collections::HashSet::new();
    if entry_nodes.is_empty() && !draft.nodes.is_empty() {
        // 无法判断入口，退化为顺序列表
        for node in &draft.nodes {
            if !is_pure_data(&node.id) {
                dfs(&node.id, draft, "", &mut visited, &mut lines);
            }
        }
    } else {
        for entry in entry_nodes {
            dfs(entry, draft, "", &mut visited, &mut lines);
        }
    }

    // ── 纯数据节点区块（不在执行树中，单独列出）────────────────────────────
    let pure_nodes: Vec<_> = draft.nodes.iter().filter(|n| is_pure_data(&n.id)).collect();
    if !pure_nodes.is_empty() {
        lines.push(String::new());
        lines.push("── 数据处理节点 ──".to_string());
        for node in pure_nodes {
            let display = node.display_name.as_deref().unwrap_or(&node.node_type);
            lines.push(format!("[{}] {} ({})", node.id, display, node.node_type));
            render_node_data_pins(&node.id, draft, "", &mut lines);
        }
    }

    lines.push("─".repeat(40));
    lines.push(format!(
        "共 {} 个节点，{} 条连线",
        draft.nodes.len(),
        draft.connections.len()
    ));
    lines.join("\n")
}

///
pub fn describe_pending_inputs(node_id: &str, draft: &BlueprintJson) -> Option<String> {
    let node = draft.nodes.iter().find(|n| n.id == node_id)?;

    // 收集已有传入连线的 DataInput 引脚名
    let connected: std::collections::HashSet<&str> = draft
        .connections
        .iter()
        .filter(|c| c.target_node == node_id)
        .map(|c| c.target_pin.as_str())
        .collect();

    // 找出既无连线也无默认值的 DataInput 引脚
    let pending: Vec<_> = node
        .pins
        .iter()
        .filter(|p| {
            p.kind == "DataInput"
                && p.default_value.is_none()
                && !connected.contains(p.name.as_str())
        })
        .collect();

    if pending.is_empty() {
        return None;
    }

    let display = node.display_name.as_deref().unwrap_or(&node.node_type);

    // 收集草稿中位于当前节点 **之前** 的节点的 DataOutput 引脚（上文可用，下文不可反向连接）
    let current_index = draft
        .nodes
        .iter()
        .position(|n| n.id == node_id)
        .unwrap_or(usize::MAX);
    let available_outputs: Vec<String> = draft
        .nodes
        .iter()
        .enumerate()
        .filter(|(idx, n)| *idx < current_index && n.id != node_id)
        .map(|(_, n)| n)
        .flat_map(|n| {
            let node_display = n.display_name.as_deref().unwrap_or(&n.node_type);
            n.pins
                .iter()
                .filter(|p| p.kind == "DataOutput")
                .map(move |p| {
                    let desc = if p.description.is_empty() {
                        &p.data_type
                    } else {
                        &p.description
                    };
                    format!(
                        "  • {}.{} ({}): {} [来自「{}」]",
                        n.id, p.name, p.data_type, desc, node_display
                    )
                })
        })
        .collect();

    let pending_lines: Vec<String> = pending
        .iter()
        .map(|p| {
            let desc = if p.description.is_empty() {
                &p.data_type
            } else {
                &p.description
            };
            format!("  • {}.{} ({}): {}", node_id, p.name, p.data_type, desc)
        })
        .collect();

    let available_section = if available_outputs.is_empty() {
        "  （当前节点之前暂无其他节点的 DataOutput，需要用 WfSetPinDefault 直接赋值）".to_string()
    } else {
        available_outputs.join("\n")
    };

    Some(format!(
        "⚠ 待完善：节点「{}」({}) 有 {} 个 DataInput 引脚尚未赋值：\n{}\n\
         \n上文节点可用的 DataOutput（根据引脚描述判断语义是否匹配，匹配则用 WfConnectPins 连线，否则用 WfSetPinDefault 赋值）：\n{}",
        display, node_id, pending.len(), pending_lines.join("\n"), available_section
    ))
}

// ============================================================================
// 连线规则执行（纯函数，修改传入的 draft）
// ============================================================================

/// 建立一条连线并执行引脚规则：
/// - 方向检查：来源节点必须在目标节点之前（下标更小），否则返回 Err
/// - ExecOutput 独占：来源引脚若为 ExecOutput，自动移除已有的出向连线，
///   并在返回值 replaced 中记录（供 to_ai 通知 AI）
/// - 其他引脚（ExecInput / DataOutput / DataInput）：无排他限制，直接追加
pub struct AddConnectionResult {
    /// 被自动替换掉的旧连线描述（格式："旧连线 {from} → {to} 已被替换"）
    pub replaced: Vec<String>,
}

pub fn add_connection_with_rules(
    draft: &mut BlueprintJson,
    source_node: String,
    source_pin: String,
    target_node: String,
    target_pin: String,
    connection_type: String,
) -> Result<AddConnectionResult, String> {
    // ── 1. 方向检查 ───────────────────────────────────────────────────────
    let src_idx = draft
        .nodes
        .iter()
        .position(|n| n.id == source_node)
        .ok_or_else(|| format!("节点不存在：{}", source_node))?;
    let tgt_idx = draft
        .nodes
        .iter()
        .position(|n| n.id == target_node)
        .ok_or_else(|| format!("节点不存在：{}", target_node))?;
    if src_idx >= tgt_idx {
        let src_display = draft.nodes[src_idx]
            .display_name
            .as_deref()
            .unwrap_or(&draft.nodes[src_idx].node_type);
        let tgt_display = draft.nodes[tgt_idx]
            .display_name
            .as_deref()
            .unwrap_or(&draft.nodes[tgt_idx].node_type);
        return Err(format!(
            "反向连线被拒绝：「{}」({}) 在草稿中位于「{}」({}) 之后，数据只能从上文流向下文。",
            source_node, src_display, target_node, tgt_display
        ));
    }

    // ── 2. ExecOutput 独占性：移除已有的出向连线 ─────────────────────────
    let mut replaced = Vec::new();
    let is_exec_output = draft.nodes[src_idx]
        .pins
        .iter()
        .any(|p| p.name == source_pin && p.kind == "ExecOutput");
    if is_exec_output {
        let old_conns: Vec<_> = draft
            .connections
            .iter()
            .filter(|c| c.source_node == source_node && c.source_pin == source_pin)
            .map(|c| {
                (
                    c.id.clone(),
                    format!(
                        "{}.{} → {}.{} 已被新连线替换",
                        c.source_node, c.source_pin, c.target_node, c.target_pin
                    ),
                )
            })
            .collect();
        for (id, desc) in old_conns {
            draft.connections.retain(|c| c.id != id);
            replaced.push(desc);
        }
    }

    // ── 3. 追加新连线 ─────────────────────────────────────────────────────
    let conn_id = format!("c{}", draft.connections.len() + 1);
    draft.add_connection(corework::workflow::blueprint_json::ConnectionJson {
        id: conn_id,
        source_node,
        source_pin,
        target_node,
        target_pin,
        connection_type,
    });

    Ok(AddConnectionResult { replaced })
}

// ============================================================================
// BuildWorkflowFromChain —— System Prompt 构建器
// ============================================================================

///
/// - `action_node_types`：主 Agent 声明的动作节点类型名列表（与工具同名）
/// - `workflow_name`：工作流名称
/// - `inputs_desc`：工作流入参描述（空字符串则不展示）
/// - `outputs_desc`：工作流出参描述（空字符串则不展示）
pub fn build_chain_compiler_prompt(
    action_node_types: &[&str],
    workflow_name: &str,
    inputs_desc: &str,
    outputs_desc: &str,
) -> String {
    // ── 基础节点目录（只取 always_visible 分类） ─────────────────────────
    let base_catalog = {
        let nodes = NodeRegistry::all();
        let mut by_cat: HashMap<&str, Vec<_>> = HashMap::new();
        for meta in &nodes {
            if meta.node_type == "SequenceNode" {
                continue;
            }
            let visible = match NodeRegistry::category_meta(meta.category) {
                Some(m) => m.always_visible,
                None => true,
            };
            if visible {
                by_cat.entry(meta.category).or_default().push(*meta);
            }
        }
        let mut sorted: Vec<_> = by_cat.keys().collect();
        sorted.sort();
        let mut lines = Vec::new();
        for cat in sorted {
            let metas = &by_cat[cat];
            let desc = NodeRegistry::category_meta(cat)
                .map(|m| format!(" — {}", m.description))
                .unwrap_or_default();
            lines.push(format!("### {}{}", cat, desc));
            for meta in metas {
                // 判断是否为 Pure（无任何 Exec 引脚）
                let is_pure = !meta
                    .pins
                    .iter()
                    .any(|p| matches!(p.kind, PinKind::ExecInput | PinKind::ExecOutput));
                let pure_tag = if is_pure { " [Pure]" } else { "" };
                // 权限提示
                let perm = permissions_hint(meta.permissions.bits)
                    .map(|h| format!("  ({})", h))
                    .unwrap_or_default();
                lines.push(format!(
                    "- `{}`{}{} — {}",
                    meta.node_type, pure_tag, perm, meta.description
                ));
                // 展示引脚摘要（最多前 8 个）
                for pin in meta.pins.iter().take(8) {
                    let dir = match pin.kind {
                        PinKind::ExecInput => "执行入",
                        PinKind::ExecOutput => "执行出",
                        PinKind::DataInput => "数据入",
                        PinKind::DataOutput => "数据出",
                    };
                    let type_info = if pin.data_type.is_empty() {
                        String::new()
                    } else {
                        // 标记泛型通配符
                        let has_t = pin
                            .data_type
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|tok| tok == "T");
                        if has_t {
                            format!(" ({}=泛型,按连线推断)", pin.data_type)
                        } else {
                            format!(" ({})", pin.data_type)
                        }
                    };
                    let default_hint = if matches!(pin.kind, PinKind::DataInput) {
                        pin.default_value
                            .map(|v| format!(" 默认:{}", v))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    lines.push(format!(
                        "  • [{}] `{}`{}{}",
                        dir, pin.name, type_info, default_hint
                    ));
                }
            }
            lines.push(String::new());
        }
        lines.join("\n")
    };

    // ── 动作节点完整引脚说明 ──────────────────────────────────────────────
    let action_section = if action_node_types.is_empty() {
        String::new()
    } else {
        let mut lines = vec![
            "### 此工作流专用动作节点（业务操作，均为 Impure，有执行引脚）".to_string(),
            String::new(),
            "这些节点与 AI 助手的同名工具一一对应，引脚定义来自节点注册表。".to_string(),
            String::new(),
        ];
        let mut missing_nodes = Vec::new();
        for node_type in action_node_types {
            if let Some(meta) = NodeRegistry::get(node_type) {
                let perm = permissions_hint(meta.permissions.bits)
                    .map(|h| format!("  ({})", h))
                    .unwrap_or_default();
                lines.push(format!(
                    "#### `{}`{} — {}",
                    meta.node_type, perm, meta.description
                ));
                lines.push(describe_node_pins(node_type));
                lines.push(String::new());
            } else {
                missing_nodes.push(*node_type);
                lines.push(format!("#### `{}` — ⚠️ 未在注册表中找到，该节点可能拼写有误。请严格按操作链中的参数名生成 DataInput/DataOutput 引脚。", node_type));
                lines.push(String::new());
            }
        }
        if !missing_nodes.is_empty() {
            tracing::warn!(
                "BuildWorkflowFromChain: 以下动作节点未在注册表中找到: {:?}",
                missing_nodes
            );
        }
        lines.join("\n")
    };

    // ── 工作流 IO ──────────────────────────────────────────────────────────
    let io_section = {
        let mut parts = Vec::new();
        if !inputs_desc.is_empty() {
            parts.push(format!(
                "## 工作流入参 → StartNode 的 DataOutput 引脚\n\
                 （在 StartNode 的 pins 数组中声明，kind=\"DataOutput\"）\n{}",
                inputs_desc
            ));
        }
        if !outputs_desc.is_empty() {
            parts.push(format!(
                "## 工作流出参 → EndNode 的 DataInput 引脚\n\
                 （在 EndNode 的 pins 数组中声明，kind=\"DataInput\"）\n{}",
                outputs_desc
            ));
        }
        parts.join("\n\n")
    };

    let _workflow_name = workflow_name; // 仅作上下文，不写入 JSON
    format!(
        r#"你是一个将操作链编译为 BlueprintJson 的编译器。

## 输出要求

- **只输出合法 JSON**，不加任何解释、不加 markdown 代码块、不加注释
- 输出 JSON 只包含 `nodes` 和 `connections` 两个字段，**不要写 `version`、`metadata`、`name`**（这些由主 Agent 负责设置）
- 所有节点 id 用 `n1`/`n2`/`n3`… 简洁形式，连线 id 用 `c1`/`c2`…

---

## ⚠️ 最关键区分：两种节点类型

### Impure 节点（有执行引脚）
- **特征**：pins 中含有 `"kind": "ExecInput"` 或 `"kind": "ExecOutput"` 的引脚
- **规则**：必须通过执行线串联在执行流中（ExecOutput → ExecInput）
- **例**：StartNode、EndNode、BranchNode、ForLoopNode、ForEachNode、BreakNode、所有动作节点

### Pure 节点（纯计算节点，无执行引脚）
- **特征**：pins 中完全没有任何 Exec 引脚，只有 `DataInput` / `DataOutput`
- **规则**：**不需要**接入执行流，直接用数据线连接；执行器按需自动计算
- **例**：AddNode、MultiplyNode、EqualNode、LessNode、GetArrayElementNode
- **在目录中标注 [Pure]**
- **注意**：Pure 节点不写 ExecInput/ExecOutput 引脚，写了也无效

---

## BlueprintJson Schema

```json
{{
  "nodes": [
    {{
      "id": "n1",
      "node_type": "StartNode",
      "display_name": "开始",
      "pins": [
        {{ "name": "Out", "kind": "ExecOutput", "data_type": "" }},
        {{ "name": "userId", "kind": "DataOutput", "data_type": "String", "description": "用户ID" }}
      ]
    }},
    {{
      "id": "n2",
      "node_type": "AddNode",
      "display_name": "计算总价",
      "pins": [
        {{ "name": "A", "kind": "DataInput", "data_type": "num", "default_value": 0.0 }},
        {{ "name": "B", "kind": "DataInput", "data_type": "num" }},
        {{ "name": "Result", "kind": "DataOutput", "data_type": "num" }}
      ]
    }},
    {{
      "id": "n3",
      "node_type": "EndNode",
      "display_name": "结束",
      "pins": [
        {{ "name": "In", "kind": "ExecInput", "data_type": "" }},
        {{ "name": "total", "kind": "DataInput", "data_type": "num", "description": "总价" }}
      ]
    }}
  ],
  "connections": [
    {{ "id": "c1", "source_node": "n1", "source_pin": "Out", "target_node": "n3", "target_pin": "In" }},
    {{ "id": "c2", "source_node": "n2", "source_pin": "Result", "target_node": "n3", "target_pin": "total" }}
  ]
}}
```

注意上面示例：n2（AddNode）是 Pure 节点，没有执行线连接，只通过数据线把 Result 送给 n3。

---

## StartNode / EndNode 动态引脚说明

**StartNode**（工作流入口）
- 固定引脚：`"name": "Out", "kind": "ExecOutput"` — 固定必须有
- 动态引脚：工作流的每个**入参**，对应一条 `"kind": "DataOutput"` 引脚
  - 引脚名 = 参数名，data_type = 参数类型
- 示例：工作流需要 userId(String) 和 amount(num) 两个入参：
  ```json
  "pins": [
    {{"name":"Out","kind":"ExecOutput","data_type":""}},
    {{"name":"userId","kind":"DataOutput","data_type":"String","description":"用户ID"}},
    {{"name":"amount","kind":"DataOutput","data_type":"num","description":"金额"}}
  ]
  ```

**EndNode**（工作流出口）
- 固定引脚：`"name": "In", "kind": "ExecInput"` — 固定必须有
- 动态引脚：工作流的每个**出参**，对应一条 `"kind": "DataInput"` 引脚
  - 引脚名 = 参数名，data_type = 参数类型
- 示例：工作流输出 result(String) 和 success(bool) 两个出参：
  ```json
  "pins": [
    {{"name":"In","kind":"ExecInput","data_type":""}},
    {{"name":"result","kind":"DataInput","data_type":"String","description":"结果文本"}},
    {{"name":"success","kind":"DataInput","data_type":"bool","description":"是否成功"}}
  ]
  ```

---

## 控制流节点引脚速查

### BranchNode（条件分支）— Impure
```
ExecInput:  In
DataInput:  Condition: bool
ExecOutput: True  （条件为真）
ExecOutput: False （条件为假）
```

### ForLoopNode（按范围循环）— Impure
```
ExecInput:  In
DataInput:  FirstIndex: num  （起始，含）
DataInput:  LastIndex: num   （结束，含）
ExecOutput: LoopBody  （每次迭代）
DataOutput: Index: num （当前索引）
ExecOutput: Completed （循环结束后）
```
- 循环 N 次时：FirstIndex=0, LastIndex=N-1

### ForEachNode（遍历数组）— Impure
```
ExecInput:  In
DataInput:  Array: Array<Any>  （要遍历的数组）
ExecOutput: LoopBody  （每次迭代）
DataOutput: Item: Any  （当前元素）
DataOutput: Index: num    （当前索引，从0开始）
ExecOutput: Completed  （遍历结束后）
```
- 操作链中 `FOR $item IN $array` → 用 ForEachNode
- 操作链中 `FOR $i FROM 0 TO N` → 用 ForLoopNode

### BreakNode（跳出循环）— Impure
```
ExecInput:  In
ExecOutput: Out
```
- 跳出最近一层循环（ForLoop 或 ForEach）
- 无任何 DataInput/DataOutput 引脚

## Pure 节点的正确用法

Pure 节点**不需要**在执行流中，只需数据连线：

```json
// ✅ 正确：Pure 节点 n_add 只连数据线，不连执行线
{{ "id": "c1", "source_node": "n_add", "source_pin": "Result", "target_node": "n_end", "target_pin": "total" }}

// ❌ 错误：不要给 Pure 节点加 ExecInput/ExecOutput 引脚
// 更不要给 Pure 节点连执行线
```

**常量值写法**：在目标节点的 DataInput 引脚上直接设 `"default_value": 42`；不要新建常量节点。

---

## 数据类型参考

| 类型名 | 说明 | JSON 示例 |
|--------|------|-----------|
| `String` | 字符串 | `"default_value": "hello"` |
| `num` | 数字 | `"default_value": 42` 或 `"default_value": 3.14` |
| `bool` | 布尔 | `"default_value": true` |
| `Array<String>` | 字符串数组 | `"default_value": ["a","b"]` |
| `Array<num>` | 数字数组 | `"default_value": [1,2.5,3]` |
| `Array<bool>` | 布尔数组 | `"default_value": [true,false]` |
| `Array<Any>` | 任意类型数组 | — |
| `Any` | 任意类型（通配） | — |

---

## 连线规则

1. **执行流**：Impure 节点的 ExecOutput → 下一个 Impure 节点的 ExecInput
2. **数据流**：任意节点的 DataOutput → 任意节点的 DataInput（Pure 节点可作为中间计算层）
3. **唯一入向**：同一 DataInput 引脚只能有一条入向连线；同一 ExecOutput 引脚只能有一条出向连线
4. **方向约束**：连线方向只能从上游节点（先出现）流向下游节点（后出现），不得反向

### 操作链语法 → 节点映射

| 操作链写法 | 对应节点 | 关键引脚 |
|-----------|---------|---------|
| `IF <cond>:` | `BranchNode` | Condition(bool) → True/False |
| `FOR $item IN $arr:` | `ForEachNode` | Array(Array<Any>) → LoopBody, Item |
| `FOR $i IN 0..N:` | `ForLoopNode` | FirstIndex/LastIndex(num) → LoopBody, Index |
| `BREAK` | `BreakNode` | — |
| `$var = <操作>` | 对应动作/计算节点 | 节点的 DataOutput 作为 $var 的来源 |
| 常量 `"abc"` / `123` | 目标引脚的 `default_value` | 直接写在引脚上，无需新建节点 |
| `RETURN $result` | `EndNode` 的 DataInput 接收连线 | — |

---

## 可用节点目录

（[Pure] = 纯计算节点，无执行引脚，不需要接入执行流）

{base}

{actions}

{io}

---

## 生成规则提示

- 若操作链引用的动作节点引脚与注册表不符，以注册表为准
- 遇到注册表中没有的操作，用最近似节点替代，并在 `display_name` 标注原意图
- 所有 Pure 节点的 pins 中**不得**出现 ExecInput/ExecOutput
- 所有 Impure 节点必须参与执行流（有执行线进出，除非是执行流终点/起点）
- 保持 JSON 完整，宁可简化逻辑也不输出残缺 JSON"#,
        base = base_catalog,
        actions = action_section,
        io = io_section,
    )
}

/// 渲染简短摘要（用于追加节点后的单行提示）
pub fn render_short_summary(draft: &BlueprintJson) -> String {
    let names: Vec<String> = draft
        .nodes
        .iter()
        .map(|n| {
            n.display_name
                .clone()
                .unwrap_or_else(|| n.node_type.clone())
        })
        .collect();
    format!("[{}]（{} 个节点）", names.join(" → "), draft.nodes.len())
}

// ============================================================================
// 工作流修改用 system prompt
// ============================================================================

pub fn build_revise_prompt(workflow_name: &str, workflow_desc: &str, feedback: &str) -> String {
    // 节点目录（同 build_chain_compiler_prompt 的 base_catalog 逻辑）
    let catalog = {
        let nodes = NodeRegistry::all();
        let mut by_cat: HashMap<&str, Vec<_>> = HashMap::new();
        for meta in &nodes {
            if meta.node_type == "SequenceNode" {
                continue;
            }
            let visible = match NodeRegistry::category_meta(meta.category) {
                Some(m) => m.always_visible,
                None => true,
            };
            if visible {
                by_cat.entry(meta.category).or_default().push(*meta);
            }
        }
        let mut sorted: Vec<_> = by_cat.keys().collect();
        sorted.sort();
        let mut lines = Vec::new();
        for cat in sorted {
            let metas = &by_cat[cat];
            lines.push(format!("### {}", cat));
            for meta in metas {
                let is_pure = !meta
                    .pins
                    .iter()
                    .any(|p| matches!(p.kind, PinKind::ExecInput | PinKind::ExecOutput));
                let pure_tag = if is_pure { " [Pure]" } else { "" };
                lines.push(format!(
                    "- `{}`{} — {}",
                    meta.node_type, pure_tag, meta.description
                ));
                for pin in meta.pins.iter().take(8) {
                    let dir = match pin.kind {
                        PinKind::ExecInput => "执行入",
                        PinKind::ExecOutput => "执行出",
                        PinKind::DataInput => "数据入",
                        PinKind::DataOutput => "数据出",
                    };
                    let type_info = if pin.data_type.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", pin.data_type)
                    };
                    let default_hint = if matches!(pin.kind, PinKind::DataInput) {
                        pin.default_value
                            .map(|v| format!(" 默认:{}", v))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    lines.push(format!(
                        "  • [{}] `{}`{}{}",
                        dir, pin.name, type_info, default_hint
                    ));
                }
            }
            lines.push(String::new());
        }
        lines.join("\n")
    };

    format!(
        r#"你是一个工作流修改助手。请根据用户的修改意见对工作流「{name}」（{desc}）进行调整，输出修改后完整的 nodes 和 connections JSON。

## 规则
1. 严格遵守节点目录中的引脚名与类型，不可臆造引脚
2. 执行流必须完整：StartNode.then → … → EndNode.exec
3. 只输出 JSON，格式：{{"nodes": [...], "connections": [...]}}，不要输出其他文字
4. 连线字段：source_node, source_pin, target_node, target_pin, connection_type("Exec"或"Data")
5. 节点字段：id, node_type, position{{x,y}}, pins(保留完整引脚), display_name(可选)

## 修改意见
{feedback}

## 节点目录
{catalog}
"#,
        name = workflow_name,
        desc = workflow_desc,
        feedback = feedback,
        catalog = catalog,
    )
}
