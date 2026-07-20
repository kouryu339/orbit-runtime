//! chain_text → 流程图结构转换
//!
//! 将操作链文本解析为口语化的流程图节点和连线，供普通模式前端渲染。

use corework::workflow::chain_ast::{InlineExpr, Step, Value};
use corework::workflow::chain_compiler_v2::parse_v2;
use corework::workflow::registry::node_registry::PinKind;
use corework::workflow::registry::NodeRegistry;
use serde::{Deserialize, Serialize};

// ============================================================================
// 流程图数据结构
// ============================================================================

/// 流程图节点类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowchartNodeType {
    /// 开始节点
    Start,
    /// 结束节点
    End,
    /// 普通动作步骤
    Action,
    /// 条件判断（菱形）
    Decision,
    /// 循环
    Loop,
    /// 跳出循环
    Break,
    /// 变量设置
    Variable,
    /// 子流程调用
    SubProcess,
}

/// 流程图节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowchartNode {
    /// 唯一 ID
    pub id: String,
    /// 节点类型
    pub node_type: FlowchartNodeType,
    /// 口语化标题（如 "打开浏览器"）
    pub label: String,
    /// 详细描述（如 "访问 https://baidu.com"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    /// 本步骤产出的输出引脚名（如 ["page_id", "title"]）
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub outputs: Vec<String>,
    /// 来自前序步骤的数据引用（如 ["url ← 步骤1.url"]）
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub data_from: Vec<String>,
}
/// 流程图连线
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowchartEdge {
    /// 源节点 ID
    pub source: String,
    /// 目标节点 ID
    pub target: String,
    /// 连线标签（如 "是"、"否"、"循环体"、"完成后"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// 流程图解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowchartData {
    pub nodes: Vec<FlowchartNode>,
    pub edges: Vec<FlowchartEdge>,
}

// ============================================================================
// 转换逻辑
// ============================================================================

struct FlowchartBuilder {
    nodes: Vec<FlowchartNode>,
    edges: Vec<FlowchartEdge>,
    id_counter: usize,
}

impl FlowchartBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            id_counter: 0,
        }
    }

    fn next_id(&mut self) -> String {
        self.id_counter += 1;
        format!("fc_{}", self.id_counter)
    }

    fn add_node(
        &mut self,
        node_type: FlowchartNodeType,
        label: String,
        detail: Option<String>,
        depth: usize,
        step_id: Option<String>,
        outputs: Vec<String>,
        data_from: Vec<String>,
    ) -> String {
        let id = self.next_id();
        self.nodes.push(FlowchartNode {
            id: id.clone(),
            node_type,
            label,
            detail,
            depth,
            step_id,
            outputs,
            data_from,
        });
        id
    }

    fn add_edge(&mut self, source: &str, target: &str, label: Option<&str>) {
        self.edges.push(FlowchartEdge {
            source: source.to_string(),
            target: target.to_string(),
            label: label.map(|s| s.to_string()),
        });
    }

    /// 处理步骤列表，返回 (first_node_id, last_node_id)
    /// prev_id: 上一个节点的 ID，用于连线
    fn process_steps(
        &mut self,
        steps: &[Step],
        depth: usize,
        prev_id: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let mut first_id: Option<String> = None;
        let mut current_prev: Option<String> = prev_id.map(|s| s.to_string());

        for step in steps {
            let (step_first, step_last) = self.process_step(step, depth);

            if let Some(ref sf) = step_first {
                if first_id.is_none() {
                    first_id = Some(sf.clone());
                }
                if let Some(ref prev) = current_prev {
                    self.add_edge(prev, sf, None);
                }
            }

            if step_last.is_some() {
                current_prev = step_last;
            }
        }

        (first_id, current_prev)
    }
    /// 处理单个步骤，返回 (first_node_id, last_node_id)
    fn process_step(&mut self, step: &Step, depth: usize) -> (Option<String>, Option<String>) {
        match step {
            Step::Input { .. } => {
                // 入参声明 — 不生成流程图节点（已在 Start 节点中体现）
                (None, None)
            }

            Step::VarInit { name, initial, .. } => {
                let detail = format!("{} = {}", name, value_to_label(initial));
                let id = self.add_node(
                    FlowchartNodeType::Variable,
                    format!("设置变量 {}", name),
                    Some(detail),
                    depth,
                    None,
                    vec![],
                    vec![],
                );
                (Some(id.clone()), Some(id))
            }

            Step::Node {
                step_id,
                node_type,
                inputs,
                ..
            } => {
                let detail = generate_node_detail(node_type, inputs);
                let label = detail
                    .clone()
                    .unwrap_or_else(|| node_type_to_label(node_type));

                // outputs: DataOutput 引脚名
                let outputs: Vec<String> = NodeRegistry::get(node_type)
                    .map(|m| {
                        m.pins
                            .iter()
                            .filter(|p| matches!(p.kind, PinKind::DataOutput))
                            .map(|p| p.name.to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                // data_from: StepRef 类型的输入
                let data_from: Vec<String> = inputs
                    .iter()
                    .filter_map(|(name, val)| {
                        if let Value::StepRef {
                            step_id: src_id,
                            pin_name,
                        } = val
                        {
                            Some(format!("{} ← 步骤{}.{}", name, src_id, pin_name))
                        } else {
                            None
                        }
                    })
                    .collect();

                let id = self.add_node(
                    FlowchartNodeType::Action,
                    label,
                    None,
                    depth,
                    step_id.clone(),
                    outputs,
                    data_from,
                );
                (Some(id.clone()), Some(id))
            }

            Step::Call {
                step_id,
                name,
                inputs,
                ..
            } => {
                let detail = if inputs.is_empty() {
                    None
                } else {
                    Some(inputs_to_detail(inputs))
                };
                let id = self.add_node(
                    FlowchartNodeType::SubProcess,
                    format!("调用 {}", name),
                    detail,
                    depth,
                    step_id.clone(),
                    vec![],
                    vec![],
                );
                (Some(id.clone()), Some(id))
            }
            Step::If {
                step_id,
                condition,
                true_block,
                false_block,
                ..
            } => {
                let cond_label = condition_to_label(condition);
                let decision_id = self.add_node(
                    FlowchartNodeType::Decision,
                    cond_label,
                    None,
                    depth,
                    step_id.clone(),
                    vec![],
                    vec![],
                );

                // 处理 true 分支
                let (true_first, true_last) = self.process_steps(true_block, depth + 1, None);
                if let Some(ref tf) = true_first {
                    self.add_edge(&decision_id, tf, Some("是"));
                }

                // 处理 false 分支
                let (false_first, false_last) = self.process_steps(false_block, depth + 1, None);
                if let Some(ref ff) = false_first {
                    self.add_edge(&decision_id, ff, Some("否"));
                }

                // 返回 decision 作为 first，分支末尾作为 last（由调用方处理汇合）
                // 如果只有 true 分支，false 直接从 decision 出去
                let last = if false_last.is_some() {
                    // 两个分支都有内容，需要汇合 — 返回两个 last
                    // 简化处理：创建一个隐式汇合点
                    let merge_id = self.add_node(
                        FlowchartNodeType::Action,
                        "".to_string(),
                        None,
                        depth,
                        None,
                        vec![],
                        vec![],
                    );
                    // 标记为空节点（前端可以隐藏或显示为小圆点）
                    if let Some(ref tl) = true_last {
                        self.add_edge(tl, &merge_id, None);
                    }
                    if let Some(ref fl) = false_last {
                        self.add_edge(fl, &merge_id, None);
                    }
                    // 如果 true 分支为空，decision 直接连汇合
                    if true_first.is_none() {
                        self.add_edge(&decision_id, &merge_id, Some("是"));
                    }
                    Some(merge_id)
                } else if true_last.is_some() {
                    // 只有 true 分支
                    true_last
                } else {
                    Some(decision_id.clone())
                };

                (Some(decision_id), last)
            }

            Step::ForLoop {
                step_id,
                from,
                to,
                body,
                ..
            } => {
                let label = format!("循环 {} 到 {}", value_to_label(from), value_to_label(to));
                let loop_id = self.add_node(
                    FlowchartNodeType::Loop,
                    label,
                    None,
                    depth,
                    step_id.clone(),
                    vec![],
                    vec![],
                );

                let (bf, bl) = self.process_steps(body, depth + 1, None);
                if let Some(ref f) = bf {
                    self.add_edge(&loop_id, f, Some("循环体"));
                }
                // 循环体末尾回到循环头
                if let Some(ref l) = bl {
                    self.add_edge(l, &loop_id, None);
                }

                (Some(loop_id.clone()), Some(loop_id))
            }

            Step::ForEach {
                step_id,
                array,
                body,
                ..
            } => {
                let label = format!("遍历 {}", value_to_label(array));
                let loop_id = self.add_node(
                    FlowchartNodeType::Loop,
                    label,
                    None,
                    depth,
                    step_id.clone(),
                    vec![],
                    vec![],
                );

                let (bf, bl) = self.process_steps(body, depth + 1, None);
                if let Some(ref f) = bf {
                    self.add_edge(&loop_id, f, Some("每个元素"));
                }
                if let Some(ref l) = bl {
                    self.add_edge(l, &loop_id, None);
                }

                (Some(loop_id.clone()), Some(loop_id))
            }

            Step::Break { .. } => {
                let id = self.add_node(
                    FlowchartNodeType::Break,
                    "跳出循环".to_string(),
                    None,
                    depth,
                    None,
                    vec![],
                    vec![],
                );
                (Some(id.clone()), Some(id))
            }

            Step::Return { .. } => {
                // Return 不在这里处理，由顶层 build 添加 End 节点
                (None, None)
            }

            Step::Block(steps) => self.process_steps(steps, depth, None),
        }
    }
}
// ============================================================================
// 口语化转换辅助函数
// ============================================================================

/// 节点类型名 → 口语化标签
fn node_type_to_label(node_type: &str) -> String {
    // 优先从 NodeRegistry 取 display_name
    if let Some(meta) = NodeRegistry::get(node_type) {
        if !meta.display_name.is_empty() {
            return meta.display_name.to_string();
        }
    }
    let name = node_type.strip_suffix("Node").unwrap_or(node_type);
    name.to_string()
}

/// 条件表达式 → 口语化标签
fn condition_to_label(value: &Value) -> String {
    match value {
        Value::Inline(expr) => inline_to_label(expr),
        Value::VarRef(name) => format!("{}？", name),
        Value::InputRef(name) => format!("input.{}？", name),
        Value::StepRef { step_id, pin_name } => format!("{}.{}？", step_id, pin_name),
        Value::Literal(v) => format!("{}？", v),
    }
}

/// 内联 Pure 表达式 → 口语化
fn inline_to_label(expr: &InlineExpr) -> String {
    let nt = &expr.node_type;
    let args: Vec<String> = expr.inputs.iter().map(|(_, v)| value_to_label(v)).collect();

    match nt.as_str() {
        "EqualNode" if args.len() == 2 => format!("{} 等于 {}？", args[0], args[1]),
        "NotEqualNode" if args.len() == 2 => format!("{} 不等于 {}？", args[0], args[1]),
        "GreaterNode" if args.len() == 2 => format!("{} 大于 {}？", args[0], args[1]),
        "GreaterOrEqualNode" if args.len() == 2 => format!("{} ≥ {}？", args[0], args[1]),
        "LessNode" if args.len() == 2 => format!("{} 小于 {}？", args[0], args[1]),
        "LessOrEqualNode" if args.len() == 2 => format!("{} ≤ {}？", args[0], args[1]),
        "ContainsNode" if args.len() == 2 => format!("{} 包含 {}？", args[0], args[1]),
        "NotNode" if args.len() == 1 => format!("非 {}？", args[0]),
        "AndNode" if args.len() == 2 => format!("{} 且 {}？", args[0], args[1]),
        "OrNode" if args.len() == 2 => format!("{} 或 {}？", args[0], args[1]),
        "AddNode" if args.len() == 2 => format!("{} + {}", args[0], args[1]),
        "SubtractNode" if args.len() == 2 => format!("{} - {}", args[0], args[1]),
        "MultiplyNode" if args.len() == 2 => format!("{} × {}", args[0], args[1]),
        "DivideNode" if args.len() == 2 => format!("{} ÷ {}", args[0], args[1]),
        "StringAppendNode" if args.len() == 2 => format!("{} + {}", args[0], args[1]),
        "TrimNode" if args.len() == 1 => format!("去空白({})", args[0]),
        _ => {
            let label = node_type_to_label(nt);
            if args.is_empty() {
                label
            } else {
                format!("{}({})", label, args.join(", "))
            }
        }
    }
}

/// Value → 简短标签
fn value_to_label(value: &Value) -> String {
    match value {
        Value::Literal(v) => match v {
            serde_json::Value::String(s) => {
                if s.len() > 30 {
                    format!("\"{}...\"", &s[..27])
                } else {
                    format!("\"{}\"", s)
                }
            }
            other => other.to_string(),
        },
        Value::StepRef { step_id, .. } => format!("步骤{}的结果", step_id),
        Value::InputRef(name) => format!("输入.{}", name),
        Value::VarRef(name) => name.clone(),
        Value::Inline(expr) => inline_to_label(expr),
    }
}

/// 输入参数列表 → 详细描述
fn inputs_to_detail(inputs: &[(String, Value)]) -> String {
    inputs
        .iter()
        .map(|(name, val)| format!("{}: {}", name, value_to_label(val)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 使用节点描述模板 + 实际输入值生成 label
///
/// 替换 {{input_name}} 为实际值，然后清除残留的 →{{xxx}} 和 {{xxx}}（输出引脚占位符），
/// 最后清理多余标点，返回干净的口语化描述。
fn generate_node_detail(node_type: &str, inputs: &[(String, Value)]) -> Option<String> {
    let meta = NodeRegistry::get(node_type);
    let desc = meta.map(|m| m.description).unwrap_or("");

    if desc.contains("{{") {
        let input_map: std::collections::HashMap<&str, String> = inputs
            .iter()
            .map(|(name, val)| (name.as_str(), value_to_label(val)))
            .collect();

        let mut result = desc.to_string();
        // 替换输入占位符
        for (name, label) in &input_map {
            let placeholder = format!("{{{{{}}}}}", name);
            result = result.replace(&placeholder, label);
        }
        // 清除残留的 →{{xxx}} 和 {{xxx}}（输出引脚，未被替换）
        let result = regex_remove_output_placeholders(&result);

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    } else if inputs.is_empty() {
        None
    } else {
        Some(inputs_to_detail(inputs))
    }
}

/// 删除描述中残留的 `→{{xxx}}`、`{{xxx}}` 及其前后多余的分隔符
fn regex_remove_output_placeholders(s: &str) -> String {
    // 逐字符扫描，删除 →{{...}} 和 {{...}} 片段
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // 检测 → 后跟 {{
        if i + 1 < len && bytes[i] == 0xe2 {
            // → 是 UTF-8 三字节 e2 86 92
            if i + 2 < len && bytes[i + 1] == 0x86 && bytes[i + 2] == 0x92 {
                let after = i + 3;
                if after + 1 < len && bytes[after] == b'{' && bytes[after + 1] == b'{' {
                    // 跳过 →{{...}}
                    if let Some(end) = find_close(s, after + 2) {
                        i = end;
                        continue;
                    }
                }
            }
        }
        // 检测 {{
        if i + 1 < len && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = find_close(s, i + 2) {
                i = end;
                continue;
            }
        }
        // 普通字符
        let ch_len = utf8_char_len(bytes[i]);
        result.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    clean_separators(&result)
}

fn find_close(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i + 2);
        }
        i += 1;
    }
    None
}

fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b < 0xe0 {
        2
    } else if b < 0xf0 {
        3
    } else {
        4
    }
}

fn clean_separators(s: &str) -> String {
    let mut result = String::new();
    let mut prev_sep = true; // 开头视为已有分隔符，避免前导逗号
    for ch in s.chars() {
        if matches!(ch, '，' | ',' | '、' | ' ') {
            if !prev_sep && !result.is_empty() {
                result.push(ch);
                prev_sep = true;
            }
        } else {
            prev_sep = false;
            result.push(ch);
        }
    }
    // 去掉尾部分隔符
    result.trim_end_matches(['，', ',', '、', ' ']).to_string()
}

// ============================================================================
// 公共 API
// ============================================================================

/// 将 chain_text 解析为流程图结构
pub fn parse_chain_to_flowchart(chain_text: &str) -> Result<FlowchartData, String> {
    let chain = parse_v2(chain_text).map_err(|e| format!("解析失败: {}", e))?;

    let mut builder = FlowchartBuilder::new();

    // 收集入参信息
    let mut input_params: Vec<String> = Vec::new();
    let mut return_outputs: Vec<String> = Vec::new();
    let mut body_steps: Vec<&Step> = Vec::new();

    for step in &chain.steps {
        match step {
            Step::Input { param_name, .. } => {
                input_params.push(param_name.clone());
            }
            Step::Block(inner) => {
                for s in inner {
                    if let Step::Input { param_name, .. } = s {
                        input_params.push(param_name.clone());
                    }
                }
            }
            Step::Return { assigns, .. } => {
                for (name, _) in assigns {
                    return_outputs.push(name.clone());
                }
            }
            _ => {
                body_steps.push(step);
            }
        }
    }

    // Start 节点
    let start_detail = if input_params.is_empty() {
        None
    } else {
        Some(format!("参数: {}", input_params.join(", ")))
    };
    let start_id = builder.add_node(
        FlowchartNodeType::Start,
        "开始".to_string(),
        start_detail,
        0,
        None,
        vec![],
        vec![],
    );

    // 处理主体步骤
    let (_body_first, body_last) = builder.process_steps(
        &body_steps.iter().map(|s| (*s).clone()).collect::<Vec<_>>(),
        0,
        Some(&start_id),
    );

    // 如果主体为空，start 直接连 end
    let last_before_end = body_last.unwrap_or(start_id.clone());

    // End 节点
    let end_detail = if return_outputs.is_empty() {
        None
    } else {
        Some(format!("返回: {}", return_outputs.join(", ")))
    };
    let end_id = builder.add_node(
        FlowchartNodeType::End,
        "结束".to_string(),
        end_detail,
        0,
        None,
        vec![],
        vec![],
    );
    builder.add_edge(&last_before_end, &end_id, None);

    Ok(FlowchartData {
        nodes: builder.nodes,
        edges: builder.edges,
    })
}
