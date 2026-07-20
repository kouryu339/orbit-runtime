//!
//! 两阶段流水线：
//! 1. **Parse**  — 操作链文本 → `chain_ast::Chain`
//! 2. **Compile** — `Chain` → `BlueprintJson`（含 CSE 去重）

use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};

use crate::rpc_tool::RuntimeToolMetadata;
use crate::workflow::blueprint_json::*;
use crate::workflow::chain_ast::*;
use crate::workflow::chain_id::HierarchicalIdGen;
use crate::workflow::core::builtin_types::is_builtin_type;
use crate::workflow::registry::{NodeRegistry, PinKind};

// ─────────────────────────────────────────────────────────────────────────────
// 错误
// ─────────────────────────────────────────────────────────────────────────────

/// 错误分类（供 agent 识别错误类型，选择合适的修复策略）
///
/// `Unclassified` 是 `ChainError::new` 默认值，向后兼容现有调用点；
/// 需要精确分类时调用 `ChainError::of_kind` 或 `with_kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainErrorKind {
    Syntax,
    /// 未知算子：节点类型不在注册表
    UnknownOperation,
    /// 未知引用：`$(x.y)` / `input.x` / `$var` 找不到目标
    UnknownReference,
    /// 类型不匹配：引脚类型与实际值冲突
    TypeMismatch,
    /// 引脚未连或未提供默认值
    DanglingPin,
    /// 未分类（兼容现有错误构造点）
    Unclassified,
}

impl ChainErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::UnknownOperation => "unknown_operation",
            Self::UnknownReference => "unknown_reference",
            Self::TypeMismatch => "type_mismatch",
            Self::DanglingPin => "dangling_pin",
            Self::Unclassified => "unclassified",
        }
    }
}

///
/// 字段约定：
/// - `line` / `message` 向后兼容，所有旧构造点使用
/// - `col` / `kind` / `suggestion` 为 Phase 2 扩展项，
///   通过 `with_col` / `with_kind` / `with_suggestion` 链式设置，
///   或由 `ChainError::new` 的自动推断填充
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChainError {
    pub line: usize,
    /// 列号（1-based），未知时为 0
    #[serde(default)]
    pub col: usize,
    /// 错误分类
    #[serde(default = "default_unclassified")]
    pub kind: ChainErrorKind,
    pub message: String,
    /// 修复建议（"did you mean X?" / 可用选项列表等）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

fn default_unclassified() -> ChainErrorKind {
    ChainErrorKind::Unclassified
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.col > 0 {
            write!(
                f,
                "[{}] line {}:{} — {}",
                self.kind.as_str(),
                self.line,
                self.col,
                self.message
            )?;
        } else {
            write!(
                f,
                "[{}] line {} — {}",
                self.kind.as_str(),
                self.line,
                self.message
            )?;
        }
        if let Some(s) = &self.suggestion {
            write!(f, "\n  💡 {}", s)?;
        }
        Ok(())
    }
}

impl std::error::Error for ChainError {}

impl ChainError {
    /// 兼容旧 API：自动从 message 内容推断 kind。
    pub fn new(line: usize, msg: impl Into<String>) -> Self {
        let message = msg.into();
        let kind = infer_kind_from_message(&message);
        Self {
            line,
            col: 0,
            kind,
            message,
            suggestion: None,
        }
    }

    /// 显式指定分类构造
    pub fn of_kind(line: usize, kind: ChainErrorKind, msg: impl Into<String>) -> Self {
        Self {
            line,
            col: 0,
            kind,
            message: msg.into(),
            suggestion: None,
        }
    }

    pub fn with_col(mut self, col: usize) -> Self {
        self.col = col;
        self
    }

    pub fn with_kind(mut self, kind: ChainErrorKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_suggestion(mut self, s: impl Into<String>) -> Self {
        self.suggestion = Some(s.into());
        self
    }

    /// 基于候选列表生成 "did you mean X?" 建议
    ///
    /// 使用简单 Levenshtein 距离，距离 ≤ 3 且 ≤ bad.len()/2 视为相近。
    pub fn with_suggest_from<I, S>(mut self, bad: &str, candidates: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let best = find_closest(bad, candidates);
        if let Some(hit) = best {
            self.suggestion = Some(format!("did you mean `{}`?", hit));
        }
        self
    }
}

/// 基于错误消息自动推断分类（仅用于兼容层）
///
/// 规则保守：命中关键词才打标签，不确定则归为 `Unclassified`。
fn infer_kind_from_message(msg: &str) -> ChainErrorKind {
    // 中文/英文关键词混合匹配
    if msg.contains("未定义")
        || msg.contains("未知")
        || msg.contains("没有引脚")
        || msg.contains("undefined")
        || msg.contains("unknown")
    {
        // 进一步区分 UnknownOperation vs UnknownReference
        if msg.contains("节点") && (msg.contains("引脚") || msg.contains("输入")) {
            return ChainErrorKind::UnknownReference;
        }
        if msg.contains("步骤") || msg.contains("变量") || msg.contains("引脚") {
            return ChainErrorKind::UnknownReference;
        }
        return ChainErrorKind::UnknownOperation;
    }
    if msg.contains("缺少")
        || msg.contains("unexpected")
        || msg.contains("语法")
        || msg.contains("冒号")
        || msg.contains("括号")
        || msg.contains("indent")
        || msg.contains("空行")
        || msg.contains("期望")
    {
        return ChainErrorKind::Syntax;
    }
    if msg.contains("类型") {
        return ChainErrorKind::TypeMismatch;
    }
    ChainErrorKind::Unclassified
}

/// Levenshtein 距离 —— 经典动态规划实现
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (m, n) = (a_chars.len(), b_chars.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = std::cmp::min(
                std::cmp::min(dp[i - 1][j] + 1, dp[i][j - 1] + 1),
                dp[i - 1][j - 1] + cost,
            );
        }
    }
    dp[m][n]
}

/// 从候选集中找与 `bad` 最接近的一个（ASCII 大小写不敏感）
pub fn find_closest<I, S>(bad: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let bad_lower = bad.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    for c in candidates {
        let cand = c.as_ref();
        let d = levenshtein(&bad_lower, &cand.to_lowercase());
        match &best {
            Some((bd, _)) if *bd <= d => {}
            _ => best = Some((d, cand.to_string())),
        }
    }
    let max_allowed = std::cmp::max(2, bad.chars().count() / 2).min(3);
    best.and_then(|(d, s)| if d <= max_allowed { Some(s) } else { None })
}

pub type ChainResult<T> = Result<T, ChainError>;

// ─────────────────────────────────────────────────────────────────────────────
// Compiler 状态
// ─────────────────────────────────────────────────────────────────────────────

///
/// 包含 parse + compile 两步，可单独调用也可用 `compile_chain` 一步到位。
pub struct ChainCompiler {
    id_gen: HierarchicalIdGen,
    conn_counter: usize,
    pure_counter: usize,
    var_init_counter: usize,

    /// Impure 节点的步骤映射：`step_id` → `(node_id, first_data_output_pin)`
    /// 兼容旧 $var 语法：`$var` → `(node_id, pin_name)`
    step_map: HashMap<String, (String, String)>,

    /// Variables that can be mutated by SetVarNode.
    writable_vars: HashSet<String>,

    pure_cache: HashMap<String, (String, String)>,

    /// Declared workflow variables emitted into BlueprintJson metadata.
    variables: Vec<BlueprintVariable>,

    /// Workflow variable references compiled as pure GetVarNode carriers.
    get_var_refs: HashMap<String, (String, String)>,

    /// 已生成的节点
    nodes: Vec<BlueprintNodeJson>,

    /// 已生成的连线
    connections: Vec<ConnectionJson>,

    /// exec 链栈：当前块里上一个 Impure 节点的 `(node_id, exec_out_pin)`
    /// 用于自动串联 Then→In
    exec_prev: Option<(String, String)>,

    /// 分支汇聚：IF/SWITCH 多分支的尾部 exec 点，下一个节点创建时会连过来
    pending_merges: Vec<(String, String)>,

    /// RETURN 绑定：(EndNode_pin_name, source_node_id, source_pin_name)
    return_bindings: Vec<(String, String, String)>,

    /// compile_step() 在处理每个步骤时更新此字段
    current_line: usize,
    runtime_tools: HashMap<String, RuntimeToolMetadata>,
}

impl Default for ChainCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainCompiler {
    pub fn new() -> Self {
        Self {
            id_gen: HierarchicalIdGen::new(),
            conn_counter: 0,
            pure_counter: 0,
            var_init_counter: 0,
            step_map: HashMap::new(),
            writable_vars: HashSet::new(),
            pure_cache: HashMap::new(),
            variables: Vec::new(),
            get_var_refs: HashMap::new(),
            nodes: Vec::new(),
            connections: Vec::new(),
            exec_prev: None,
            pending_merges: Vec::new(),
            return_bindings: Vec::new(),
            current_line: 0,
            runtime_tools: HashMap::new(),
        }
    }

    pub fn with_runtime_tools(runtime_tools: &[RuntimeToolMetadata]) -> Self {
        let mut compiler = Self::new();
        compiler.runtime_tools = runtime_tools
            .iter()
            .cloned()
            .map(|tool| (tool.name.clone(), tool))
            .collect();
        compiler
    }

    // ── ID 生成 ──────────────────────────────────────────────────────────

    fn next_conn_id(&mut self) -> String {
        self.conn_counter += 1;
        format!("c{}", self.conn_counter)
    }

    fn next_pure_node_id(&mut self) -> String {
        self.pure_counter += 1;
        format!("p{}", self.pure_counter)
    }

    fn next_var_init_id(&mut self) -> String {
        self.var_init_counter += 1;
        format!("v{}", self.var_init_counter)
    }

    // ── 连线辅助 ─────────────────────────────────────────────────────────

    fn add_connection(
        &mut self,
        src_node: &str,
        src_pin: &str,
        tgt_node: &str,
        tgt_pin: &str,
        conn_type: &str,
    ) {
        let id = self.next_conn_id();
        self.connections.push(ConnectionJson {
            id,
            source_node: src_node.to_string(),
            source_pin: src_pin.to_string(),
            target_node: tgt_node.to_string(),
            target_pin: tgt_pin.to_string(),
            connection_type: conn_type.to_string(),
        });
    }

    /// 将 exec_prev 连到 target 的 In 引脚，并处理 pending_merges
    fn wire_exec_to(&mut self, target_id: &str, target_pin: &str) {
        if let Some((prev_id, prev_pin)) = self.exec_prev.take() {
            self.add_connection(&prev_id, &prev_pin, target_id, target_pin, "Exec");
        }
        // 汇聚分支尾部
        for (merge_id, merge_pin) in std::mem::take(&mut self.pending_merges) {
            self.add_connection(&merge_id, &merge_pin, target_id, target_pin, "Exec");
        }
    }

    /// 设置下一个 exec 输出点
    fn set_exec_prev(&mut self, node_id: &str, pin: &str) {
        self.exec_prev = Some((node_id.to_string(), pin.to_string()));
    }

    // ── CSE canonical key ────────────────────────────────────────────────

    fn value_key(&self, val: &Value) -> String {
        match val {
            Value::Literal(v) => format!("L:{}", v),
            Value::StepRef { step_id, pin_name } => format!("{}.{}", step_id, pin_name),
            Value::InputRef(name) => format!("input.{}", name),
            Value::VarRef(name) => format!("${}", name),
            Value::Inline(expr) => self.inline_key(expr),
        }
    }

    fn inline_key(&self, expr: &InlineExpr) -> String {
        let mut pins: Vec<String> = expr
            .inputs
            .iter()
            .map(|(k, v)| format!("{}={}", k, self.value_key(v)))
            .collect();
        pins.sort(); // 引脚排序，确保参数顺序无关
        let sel = match &expr.output_pin {
            Some(p) => format!("[{}]", p),
            None => String::new(),
        };
        format!("{}({}){}", expr.node_type, pins.join(","), sel)
    }

    // ── 节点创建辅助 ─────────────────────────────────────────────────────

    /// 根据注册表元数据构建节点的 pins 数组
    fn build_pins_from_registry(&self, node_type: &str) -> Vec<NodePin> {
        if let Some(meta) = NodeRegistry::get(node_type) {
            return meta
                .pins
                .iter()
                .map(|p| NodePin {
                    name: p.name.to_string(),
                    kind: match p.kind {
                        PinKind::ExecInput => "ExecInput".to_string(),
                        PinKind::ExecOutput => "ExecOutput".to_string(),
                        PinKind::DataInput => "DataInput".to_string(),
                        PinKind::DataOutput => "DataOutput".to_string(),
                    },
                    data_type: p.data_type.to_string(),
                    description: p.description.to_string(),
                    default_value: p.default_value.and_then(|s| serde_json::from_str(s).ok()),
                    resolved_type: None,
                    split_config: None,
                })
                .collect();
        }

        let Some(tool) = self.runtime_tools.get(node_type) else {
            return Vec::new();
        };
        let mut pins = vec![NodePin {
            name: "In".to_string(),
            kind: "ExecInput".to_string(),
            data_type: String::new(),
            description: String::new(),
            default_value: None,
            resolved_type: None,
            split_config: None,
        }];
        pins.extend(tool.parameters.iter().map(|parameter| {
            NodePin {
                name: parameter.name.clone(),
                kind: "DataInput".to_string(),
                data_type: parameter.param_type.clone(),
                description: parameter.description.clone(),
                default_value: parameter
                    .default_value
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok()),
                resolved_type: None,
                split_config: None,
            }
        }));
        pins.extend(tool.outputs.iter().map(|output| NodePin {
            name: output.name.clone(),
            kind: "DataOutput".to_string(),
            data_type: output.field_type.clone(),
            description: output.description.clone(),
            default_value: None,
            resolved_type: None,
            split_config: None,
        }));
        pins.push(NodePin {
            name: "Then".to_string(),
            kind: "ExecOutput".to_string(),
            data_type: String::new(),
            description: String::new(),
            default_value: None,
            resolved_type: None,
            split_config: None,
        });
        pins
    }

    /// 查找节点元数据中第一个 DataOutput 的引脚名
    fn first_data_output(&self, node_type: &str) -> Option<String> {
        if let Some(meta) = NodeRegistry::get(node_type) {
            return meta
                .pins
                .iter()
                .find(|p| matches!(p.kind, PinKind::DataOutput))
                .map(|p| p.name.to_string());
        }
        self.runtime_tools
            .get(node_type)
            .and_then(|tool| tool.outputs.first())
            .map(|output| output.name.clone())
    }

    /// 判断节点类型是否为 Pure（无任何 Exec 引脚）
    fn is_pure(&self, node_type: &str) -> bool {
        match NodeRegistry::get(node_type) {
            Some(meta) => meta
                .pins
                .iter()
                .all(|p| p.kind != PinKind::ExecInput && p.kind != PinKind::ExecOutput),
            None => false,
        }
    }

    fn node_exists(&self, node_type: &str) -> bool {
        NodeRegistry::get(node_type).is_some() || self.runtime_tools.contains_key(node_type)
    }

    fn unknown_node_error(&self, node_type: &str) -> ChainError {
        let candidates = NodeRegistry::all()
            .into_iter()
            .map(|metadata| metadata.node_type.to_string())
            .chain(self.runtime_tools.keys().cloned())
            .collect::<Vec<_>>();
        ChainError::of_kind(
            self.current_line,
            ChainErrorKind::UnknownOperation,
            format!("Node `{node_type}` is not registered"),
        )
        .with_suggest_from(node_type, candidates)
    }

    // ── 入参自动推断 ─────────────────────────────────────────────────────

    /// 扫描 AST，返回"被引用但从未被赋值"的变量名列表（即工作流入参），保持首次出现顺序。
    #[allow(dead_code)]
    fn collect_input_vars(steps: &[Step]) -> Vec<String> {
        let mut assigned: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut referenced: Vec<String> = Vec::new(); // 保序，用于引脚顺序稳定
        let mut referenced_set: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        Self::scan_steps_vars(steps, &mut assigned, &mut referenced, &mut referenced_set);

        // 引用但未赋值 = 入参；保持 referenced 中的首次出现顺序
        referenced
            .into_iter()
            .filter(|v| !assigned.contains(v))
            .collect()
    }

    #[allow(dead_code)]
    fn scan_steps_vars(
        steps: &[Step],
        assigned: &mut std::collections::HashSet<String>,
        referenced: &mut Vec<String>,
        referenced_set: &mut std::collections::HashSet<String>,
    ) {
        for step in steps {
            match step {
                Step::Node {
                    step_id, inputs, ..
                } => {
                    if let Some(v) = step_id {
                        assigned.insert(v.clone());
                    }
                    for (_, val) in inputs {
                        Self::scan_value_vars(val, assigned, referenced, referenced_set);
                    }
                }
                Step::Call {
                    step_id, inputs, ..
                } => {
                    if let Some(v) = step_id {
                        assigned.insert(v.clone());
                    }
                    for (_, val) in inputs {
                        Self::scan_value_vars(val, assigned, referenced, referenced_set);
                    }
                }
                Step::If {
                    condition,
                    true_block,
                    false_block,
                    ..
                } => {
                    Self::scan_value_vars(condition, assigned, referenced, referenced_set);
                    Self::scan_steps_vars(true_block, assigned, referenced, referenced_set);
                    Self::scan_steps_vars(false_block, assigned, referenced, referenced_set);
                }
                Step::ForEach { array, body, .. } => {
                    // $item / $index 是隐式保留变量，自动标记为已赋值（不成为工作流入参）
                    assigned.insert("item".to_string());
                    assigned.insert("index".to_string());
                    Self::scan_value_vars(array, assigned, referenced, referenced_set);
                    Self::scan_steps_vars(body, assigned, referenced, referenced_set);
                }
                Step::ForLoop { from, to, body, .. } => {
                    // $index 是隐式保留变量，自动标记为已赋值（不成为工作流入参）
                    assigned.insert("index".to_string());
                    Self::scan_value_vars(from, assigned, referenced, referenced_set);
                    Self::scan_value_vars(to, assigned, referenced, referenced_set);
                    Self::scan_steps_vars(body, assigned, referenced, referenced_set);
                }
                Step::Return { assigns, .. } => {
                    for (_, val) in assigns {
                        Self::scan_value_vars(val, assigned, referenced, referenced_set);
                    }
                }
                Step::Input {
                    var_name, default, ..
                } => {
                    if !var_name.is_empty() {
                        assigned.insert(var_name.clone());
                    }
                    // 扫描默认值里可能引用的变量（理论上应为纯字面量，但防御性扫描）
                    if let Some(d) = default {
                        Self::scan_value_vars(d, assigned, referenced, referenced_set);
                    }
                }
                Step::Block(steps) => {
                    // 递归扫描块中的每个步骤
                    Self::scan_steps_vars(steps, assigned, referenced, referenced_set);
                }
                Step::VarInit { name, initial, .. } => {
                    assigned.insert(name.clone());
                    Self::scan_value_vars(initial, assigned, referenced, referenced_set);
                }
                Step::Break { .. } => {}
            }
        }
    }

    #[allow(dead_code)]
    fn scan_value_vars(
        val: &Value,
        assigned: &mut std::collections::HashSet<String>,
        referenced: &mut Vec<String>,
        referenced_set: &mut std::collections::HashSet<String>,
    ) {
        match val {
            Value::StepRef { step_id, pin_name } => {
                let key = format!("{}.{}", step_id, pin_name);
                if referenced_set.insert(key.clone()) {
                    referenced.push(key);
                }
            }
            Value::InputRef(name) | Value::VarRef(name) => {
                if referenced_set.insert(name.clone()) {
                    referenced.push(name.clone());
                }
            }
            Value::Inline(expr) => {
                for (_, v) in &expr.inputs {
                    Self::scan_value_vars(v, assigned, referenced, referenced_set);
                }
            }
            Value::Literal(_) => {}
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Part 2: Parser — 操作链文本 → Chain AST
// ═════════════════════════════════════════════════════════════════════════════

struct Line {
    /// 原始行号（1-based）
    lineno: usize,
    indent: usize,
    content: String,
}

/// 行信息：用于建立父子关系
#[derive(Debug, Clone)]
pub struct LineInfo {
    pub lineno: usize,
    pub indent: usize,
    pub content: String,
    pub keyword: KeywordType,
    pub parent_idx: Option<usize>,
    pub belongs_to: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeywordType {
    Input,
    Variable,
    If,
    Elif,
    Else,
    For,
    Break,
    Return,
    Node,
    Empty,
}

impl ChainCompiler {
    // ── 公开解析入口 ─────────────────────────────────────────────────────

    /// 解析操作链文本为 AST
    /// 格式要求：
    ///   第1行：INPUT 声明
    ///   第2行：可选的变量初始化
    ///   最后1行：RETURN 语句
    pub fn parse(text: &str) -> ChainResult<Chain> {
        // 去除首尾空白，避免多行字符串开头的空行被误解析
        let text = text.trim();
        let lines = Self::preprocess_lines(text);

        if lines.is_empty() {
            return Err(ChainError::new(1, "工作流不能为空"));
        }

        // 1. 解析第1行：必须是 INPUT
        let first_line = &lines[0];
        if !first_line.content.starts_with("INPUT") {
            return Err(ChainError::new(first_line.lineno, "第1行必须是 INPUT 声明"));
        }
        let input_step = Self::parse_input_declaration(&first_line.content, first_line.lineno)?;

        // 2. 解析最后1行：必须是 RETURN
        let last_line = &lines[lines.len() - 1];
        if !last_line.content.starts_with("RETURN") {
            return Err(ChainError::new(
                last_line.lineno,
                "最后1行必须是 RETURN 语句",
            ));
        }

        // 3. 建立行关系表（第一遍扫描）
        let line_info = Self::build_line_relation_table(&lines)?;

        // 4. 解析中间部分（使用关系表）
        let mut all_steps = vec![input_step];

        if lines.len() >= 3 {
            // 第2行：必须是变量初始化（$var = value）或空行
            let second_line = &lines[1];
            if !second_line.content.is_empty() {
                // 第二行必须是变量声明（$var = ...）或空行
                // 检查两种情况：
                // 1. $var = NodeCall(...) 形式（等号后非引号+括号）
                // 2. 直接节点调用 NodeCall(...)（无$开头）
                let is_node_call =
                    if second_line.content.starts_with('$') && second_line.content.contains('=') {
                        // 情况1: $var = NodeCall(...)
                        let after_eq = second_line
                            .content
                            .split_once('=')
                            .map(|x| x.1)
                            .unwrap_or("")
                            .trim();
                        let is_string_literal = after_eq.starts_with('"');
                        !is_string_literal && after_eq.contains('(')
                    } else {
                        // 情况2: 直接是节点调用（无$开头，包含括号）
                        second_line.content.contains('(')
                    };

                if is_node_call {
                    return Err(ChainError::new(
                        second_line.lineno,
                        "第2行不能是节点调用，必须是变量声明（如 $sum = 0）。\n\
                         提示：INPUT 声明后必须空出一行写变量声明，节点调用放在后面行。",
                    ));
                }

                let var_step =
                    Self::parse_var_init_or_node(&second_line.content, second_line.lineno)?;
                all_steps.push(var_step);
            }

            // 中间部分：用关系表解析
            let body_start = 2;
            let body_end = lines.len() - 1;
            let body_steps = Self::parse_body_with_table(&lines, &line_info, body_start, body_end)?;
            all_steps.extend(body_steps);
        } else if lines.len() == 2 {
            // 只有2行：INPUT + RETURN，中间可以没有变量初始化
        } else {
            // 只有1行：只有INPUT没有RETURN，报错
            return Err(ChainError::new(last_line.lineno, "缺少 RETURN 语句"));
        }

        // 5. 解析 RETURN
        let return_step = Self::parse_return_statement(&last_line.content, last_line.lineno)?;
        all_steps.push(return_step);

        Ok(Chain { steps: all_steps })
    }

    // ── 关系表解析 ───────────────────────────────────────────────────

    /// 第一遍扫描：建立行关系表
    fn build_line_relation_table(lines: &[Line]) -> ChainResult<Vec<LineInfo>> {
        let mut info = Vec::new();
        // 控制流栈：存储 (keyword_type, line_idx, indent)
        // 用于追踪当前在哪个控制流内部
        let mut control_stack: Vec<(KeywordType, usize, usize)> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            let keyword = Self::detect_keyword(&line.content);

            // 根据关键字类型处理栈
            match keyword {
                KeywordType::If | KeywordType::For => {
                    // 入栈：新的控制流开始
                    control_stack.push((keyword.clone(), i, line.indent));
                }
                KeywordType::Else | KeywordType::Elif => {
                    // ELSE/ELIF：找到对应的 IF 并更新其分支信息
                    // 从栈顶向下找最近的 IF，保持栈不变（ELSE/ELIF 与 IF 同级）
                    let _ = control_stack
                        .iter()
                        .rposition(|(k, _, _)| *k == KeywordType::If);
                }
                KeywordType::Break => {
                    // BREAK：必须在 FOR 的子链中
                    if !control_stack.iter().any(|(k, _, _)| *k == KeywordType::For) {
                        return Err(ChainError::new(line.lineno, "BREAK 必须写在 FOR 循环体内"));
                    }
                }
                _ => {}
            }

            // 确定父级：当前控制流栈顶的索引
            let parent_idx = control_stack.last().map(|(_, idx, _)| *idx);

            // 确定属于哪个控制流的子块
            let belongs_to = control_stack.last().map(|(_, idx, _)| *idx);

            info.push(LineInfo {
                lineno: line.lineno,
                indent: line.indent,
                content: line.content.clone(),
                keyword,
                parent_idx,
                belongs_to,
            });

            // 处理出栈：如果是控制流，在其子块结束后需要出栈
            // 但这里第一遍只建立关系，出栈在解析时处理
        }

        Ok(info)
    }

    /// 检测行关键字类型
    fn detect_keyword(content: &str) -> KeywordType {
        let c = content.trim();
        if c.is_empty() {
            return KeywordType::Empty;
        }
        let c = {
            let (sid, rest) = Self::split_step_prefix(c);
            if sid.is_some() {
                rest
            } else {
                c
            }
        };
        if c.starts_with("INPUT") {
            return KeywordType::Input;
        }
        if c.starts_with("RETURN") {
            return KeywordType::Return;
        }
        if c.starts_with("IF ") || c.starts_with("IF\t") {
            return KeywordType::If;
        }
        if c.starts_with("ELIF ") || c.starts_with("ELIF\t") {
            return KeywordType::Elif;
        }
        if c.starts_with("ELSE") {
            return KeywordType::Else;
        }
        if c.starts_with("FOR ") || c.starts_with("FOR\t") {
            return KeywordType::For;
        }
        if c == "BREAK" {
            return KeywordType::Break;
        }
        if c.starts_with('$') {
            return KeywordType::Variable;
        }
        KeywordType::Node
    }

    /// 第二遍解析：使用关系表解析
    fn parse_body_with_table(
        lines: &[Line],
        line_info: &[LineInfo],
        start: usize,
        end: usize,
    ) -> ChainResult<Vec<Step>> {
        let mut steps = Vec::new();
        let mut i = start;

        while i < end {
            let info = &line_info[i];

            match info.keyword {
                KeywordType::If => {
                    let (step, next_i) = Self::parse_if_with_table(lines, line_info, i, end)?;
                    steps.push(step);
                    i = next_i;
                }
                KeywordType::For => {
                    let (step, next_i) = Self::parse_for_with_table(lines, line_info, i, end)?;
                    steps.push(step);
                    i = next_i;
                }
                KeywordType::Break => {
                    let (break_sid, _) = Self::split_step_prefix(&info.content);
                    steps.push(Step::Break {
                        line: info.lineno,
                        step_id: break_sid,
                    });
                    i += 1;
                }
                KeywordType::Node | KeywordType::Variable => {
                    let step = Self::parse_node_statement(&info.content, info.lineno)
                        .or_else(|_| Self::parse_call_statement(&info.content, info.lineno))?;
                    steps.push(step);
                    i += 1;
                }
                KeywordType::Else | KeywordType::Elif => {
                    // ELSE/ELIF 应该在 IF 解析时处理，不应该出现在这里
                    return Err(ChainError::new(info.lineno, "ELSE/ELIF 必须紧跟在 IF 后面"));
                }
                KeywordType::Empty => {
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }

        Ok(steps)
    }

    /// 使用关系表解析 IF 块
    fn parse_if_with_table(
        lines: &[Line],
        line_info: &[LineInfo],
        pos: usize,
        end: usize,
    ) -> ChainResult<(Step, usize)> {
        let info = &line_info[pos];
        let content = &info.content;

        let (step_id, content_rest) = Self::split_step_prefix(content);

        // 解析条件（支持 IF 和 ELIF 关键字）
        let rest = content_rest
            .strip_prefix("IF ")
            .or_else(|| content_rest.strip_prefix("ELIF "))
            .unwrap_or("")
            .trim_end();
        let cond_str = rest
            .strip_suffix(':')
            .ok_or_else(|| ChainError::new(info.lineno, "IF 语句缺少结尾冒号"))?;
        let condition = Self::parse_value(cond_str.trim(), info.lineno)?;

        // 找 true_block 和 false_block 的范围
        let _if_idx = pos;
        let mut true_end = end;
        let mut else_idx = None;
        let _false_end = end;

        let if_indent = info.indent;
        for j in (pos + 1)..end {
            let j_info = &line_info[j];
            if j_info.indent == if_indent
                && (j_info.keyword == KeywordType::Else || j_info.keyword == KeywordType::Elif)
            {
                else_idx = Some(j);
                true_end = j;
                break;
            }
        }

        // 解析 true_block（IF 下面到 ELSE 之前的行）
        let true_block = if else_idx.is_some() {
            Self::parse_body_with_table(lines, line_info, pos + 1, true_end)?
        } else {
            Self::parse_body_with_table(lines, line_info, pos + 1, end)?
        };

        // 解析 false_block（如果有 ELSE）
        let (false_block, false_end_pos) = if let Some(else_pos) = else_idx {
            // 判断是 ELSE 还是 ELIF
            let else_info = &line_info[else_pos];
            if else_info.keyword == KeywordType::Elif {
                // ELIF：递归解析为新的 IF
                let (elif_step, elif_end) =
                    Self::parse_if_with_table(lines, line_info, else_pos, end)?;
                (vec![elif_step], elif_end)
            } else {
                // ELSE：解析 else 后面的行
                let mut else_block_end = end;
                // 找到 ELSE 块的结束位置
                for j in (else_pos + 1)..end {
                    if line_info[j].indent <= if_indent {
                        else_block_end = j;
                        break;
                    }
                }
                let fb =
                    Self::parse_body_with_table(lines, line_info, else_pos + 1, else_block_end)?;
                (fb, else_block_end)
            }
        } else {
            (Vec::new(), true_end)
        };

        let after_else = else_idx.map(|e| e + 1).unwrap_or(end);
        let next_pos = if after_else > false_end_pos {
            after_else
        } else {
            false_end_pos
        };
        Ok((
            Step::If {
                line: info.lineno,
                step_id,
                condition,
                true_block,
                false_block,
            },
            next_pos,
        ))
    }

    /// 使用关系表解析 FOR 块
    fn parse_for_with_table(
        lines: &[Line],
        line_info: &[LineInfo],
        pos: usize,
        end: usize,
    ) -> ChainResult<(Step, usize)> {
        let info = &line_info[pos];
        let content = &info.content;

        let (step_id, content_rest) = Self::split_step_prefix(content);

        // 解析 FOR 内容
        let rest = content_rest.strip_prefix("FOR ").unwrap_or("").trim_end();
        let rest = rest
            .strip_suffix(':')
            .ok_or_else(|| ChainError::new(info.lineno, "FOR 语句缺少结尾冒号"))?;

        let for_indent = info.indent;
        let mut for_end = end;
        for j in (pos + 1)..end {
            if line_info[j].indent <= for_indent {
                for_end = j;
                break;
            }
        }

        // 解析循环体
        let body = Self::parse_body_with_table(lines, line_info, pos + 1, for_end)?;

        // 判断是范围循环还是数组遍历
        if rest.contains(" TO ") {
            let (from, to) = Self::parse_for_range(rest, info.lineno)?;
            Ok((
                Step::ForLoop {
                    line: info.lineno,
                    step_id,
                    from,
                    to,
                    body,
                },
                for_end,
            ))
        } else {
            let array_val = Self::parse_value(rest, info.lineno)?;
            Ok((
                Step::ForEach {
                    line: info.lineno,
                    step_id,
                    array: array_val,
                    body,
                },
                for_end,
            ))
        }
    }

    // ── 预处理 ───────────────────────────────────────────────────────────

    fn preprocess_lines(text: &str) -> Vec<Line> {
        let mut result = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let trimmed = raw.trim();
            // 跳过注释行，但保留空行（保持行号对应）
            if trimmed.starts_with('#') {
                continue;
            }
            let indent_chars: usize = raw
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum();
            let indent = indent_chars / 4;
            result.push(Line {
                lineno: i + 1,
                indent,
                content: trimmed.to_string(),
            });
        }
        result
    }

    // ── 固定行解析 ───────────────────────────────────────────────────────

    /// 解析 INPUT 声明
    fn parse_input_declaration(content: &str, lineno: usize) -> ChainResult<Step> {
        let rest = content.strip_prefix("INPUT").unwrap_or("");
        let rest = rest.trim_start();

        if rest.is_empty() {
            return Ok(Step::Input {
                line: lineno,
                param_name: String::new(),
                var_name: String::new(),
                param_type: None,
                default: None,
            });
        }

        let parts: Vec<&str> = rest.split_whitespace().collect();
        let mut steps_out = Vec::new();

        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            // 必须包含冒号，格式 name:Type 或 name:Type=default
            let (param_name, rest_part) = if let Some((n, r)) = part.split_once(':') {
                (n.trim().to_string(), r.trim().to_string())
            } else {
                return Err(ChainError::new(lineno, format!(
                    "INPUT 参数 '{}' 缺少类型声明，正确格式：名称:类型（如 url:String、file:Path、count:i64=10）",
                    part
                )));
            };

            // 验证参数名合法性
            if param_name.is_empty() {
                return Err(ChainError::new(lineno, "INPUT 参数名不能为空"));
            }
            if !param_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(ChainError::new(
                    lineno,
                    format!(
                        "INPUT 参数名 '{}' 不合法，只能使用字母、数字和下划线",
                        param_name
                    ),
                ));
            }

            // 解析类型和可选默认值：rest_part = "Type" 或 "Type=default"
            let (type_str, default) = if let Some(eq_pos) = rest_part.find('=') {
                let type_part = rest_part[..eq_pos].trim();
                let default_part = rest_part[eq_pos + 1..].trim();
                let default_val = if default_part.is_empty() {
                    None
                } else {
                    Some(Self::parse_value(default_part, lineno)?)
                };
                (type_part.to_string(), default_val)
            } else {
                (rest_part.trim().to_string(), None)
            };

            // 验证类型是否合法
            if type_str.is_empty() {
                return Err(ChainError::new(lineno, format!(
                    "INPUT 参数 '{}' 的类型不能为空，可用类型：String、Path、Date、Time、i64、f64、bool、Array<T>、Object",
                    param_name
                )));
            }
            // 泛型数组 Array<T> / Vec<T> 单独处理
            let base_type = if type_str.starts_with("Array<") || type_str.starts_with("Vec<") {
                &type_str[..] // 交给 is_builtin_type 处理（它支持 Array< 前缀）
            } else {
                &type_str[..]
            };
            if !is_builtin_type(base_type) {
                return Err(ChainError::new(lineno, format!(
                    "INPUT 参数 '{}' 的类型 '{}' 不是有效类型。\n可用类型：String、Path（文件路径）、Date（日期）、Time（时间）、i64（整数）、f64（小数）、bool、Array<T>（列表）、Object",
                    param_name, type_str
                )));
            }
            Self::validate_input_default(&param_name, &type_str, default.as_ref(), lineno)?;

            let var_name = param_name.clone();
            steps_out.push(Step::Input {
                line: lineno,
                param_name,
                var_name,
                param_type: Some(type_str),
                default,
            });
        }

        if steps_out.is_empty() {
            return Err(ChainError::new(lineno, "INPUT 声明不能为空"));
        }
        if steps_out.len() == 1 {
            return Ok(steps_out.remove(0));
        }
        Ok(Step::Block(steps_out))
    }

    /// 解析变量初始化或节点调用（第2行）
    fn parse_var_init_or_node(content: &str, lineno: usize) -> ChainResult<Step> {
        let content = content.trim();
        if content.is_empty() {
            return Err(ChainError::new(lineno, "空行"));
        }

        // 变量初始化：$var = value
        if content.starts_with('$') {
            if let Some(eq_pos) = content.find('=') {
                let var_name = content[1..eq_pos].trim().to_string();
                let value_str = content[eq_pos + 1..].trim();
                let value = Self::parse_value(value_str, lineno)?;
                Self::validate_var_initializer(&var_name, &value, lineno)?;
                return Ok(Step::VarInit {
                    line: lineno,
                    name: var_name,
                    initial: value,
                });
            }
        }

        // 节点调用：NodeName(...) 或 $out = NodeName(...)
        // 先尝试解析为节点调用
        if let Ok(step) = Self::parse_node_statement(content, lineno) {
            return Ok(step);
        }
        // 再尝试解析为函数调用
        Self::parse_call_statement(content, lineno)
    }

    /// 解析 RETURN 语句
    fn parse_return_statement(content: &str, lineno: usize) -> ChainResult<Step> {
        let rest = content.strip_prefix("RETURN").unwrap_or("").trim_start();
        if rest.is_empty() {
            return Ok(Step::Return {
                line: lineno,
                assigns: Vec::new(),
            });
        }
        let assigns = Self::parse_assignments(rest, lineno)?;
        Ok(Step::Return {
            line: lineno,
            assigns,
        })
    }

    #[allow(dead_code)]
    fn parse_indent_block(
        lines: &[Line],
        start: usize,
        end: usize,
        parent_indent: usize,
    ) -> ChainResult<Vec<Step>> {
        let mut steps = Vec::new();
        let mut i = start;

        while i < end {
            let line = &lines[i];

            if line.indent < parent_indent {
                break;
            }

            if line.indent == parent_indent {
                let (step, next_i) = Self::parse_indent_statement(lines, i, end, parent_indent)?;
                steps.push(step);
                i = next_i;
                continue;
            }

            // 跳过
            i += 1;
        }

        Ok(steps)
    }

    /// 解析一条语句（包括其子块）
    #[allow(dead_code)]
    fn parse_indent_statement(
        lines: &[Line],
        pos: usize,
        end: usize,
        parent_indent: usize,
    ) -> ChainResult<(Step, usize)> {
        let line = &lines[pos];
        let content = &line.content;

        let (if_sid, content_kw) = Self::split_step_prefix(content);
        if let Some(rest) = content_kw.strip_prefix("IF ") {
            return Self::parse_if_block(if_sid, rest, lines, pos, end, parent_indent);
        }

        let (for_sid, content_kw2) = Self::split_step_prefix(content);
        if let Some(rest) = content_kw2.strip_prefix("FOR ") {
            return Self::parse_for_block(for_sid, rest, lines, pos, end, parent_indent);
        }

        // ELSE（需要找到对应的 IF）
        if content == "ELSE" || content.starts_with("ELSE ") {
            // ELSE 应该与 IF 一起解析，这里作为独立语句返回
            return Err(ChainError::new(line.lineno, "ELSE 没有匹配的 IF"));
        }

        let (break_sid, break_kw) = Self::split_step_prefix(content);
        if break_kw == "BREAK" {
            // 找子块
            let child_end = Self::find_child_block_end(lines, pos + 1, end, line.indent);
            let child_steps = Self::parse_indent_block(lines, pos + 1, child_end, line.indent + 1)?;
            if child_steps.is_empty() {
                return Ok((
                    Step::Break {
                        line: line.lineno,
                        step_id: break_sid,
                    },
                    child_end,
                ));
            }
            // BREAK 带子块的情况
            let mut result = vec![Step::Break {
                line: line.lineno,
                step_id: break_sid,
            }];
            result.extend(child_steps);
            return Ok((Step::Block(result), child_end));
        }

        // 节点调用或变量赋值
        let step = Self::parse_node_statement(content, line.lineno)
            .or_else(|_| Self::parse_call_statement(content, line.lineno))?;

        // 找子块
        let child_end = Self::find_child_block_end(lines, pos + 1, end, line.indent);
        let child_steps = Self::parse_indent_block(lines, pos + 1, child_end, line.indent + 1)?;

        if child_steps.is_empty() {
            return Ok((step, child_end));
        }

        // 如果有子块，将当前步骤和子块组合
        let mut result = vec![step];
        result.extend(child_steps);
        Ok((Step::Block(result), child_end))
    }

    /// 找子块的结束位置
    #[allow(dead_code)]
    fn find_child_block_end(
        lines: &[Line],
        start: usize,
        end: usize,
        parent_indent: usize,
    ) -> usize {
        let mut i = start;
        while i < end {
            if lines[i].indent <= parent_indent {
                return i;
            }
            i += 1;
        }
        end
    }

    /// 解析 IF 块
    #[allow(dead_code)]
    fn parse_if_block(
        step_id: Option<String>,
        rest: &str,
        lines: &[Line],
        pos: usize,
        end: usize,
        parent_indent: usize,
    ) -> ChainResult<(Step, usize)> {
        let rest = rest.trim_end();
        let cond_str = rest
            .strip_suffix(':')
            .ok_or_else(|| ChainError::new(lines[pos].lineno, "IF 语句缺少结尾冒号"))?;
        let condition = Self::parse_value(cond_str.trim(), lines[pos].lineno)?;

        let if_end = Self::find_child_block_end(lines, pos + 1, end, parent_indent);

        let else_pos = Self::find_else_or_elif(lines, pos + 1, if_end, lines[pos].indent);

        // 解析 true_block：从 IF 下一行到 ELSE 之前
        let if_body_indent = lines[pos].indent + 1;
        let true_block = if else_pos < if_end {
            Self::parse_indent_block(lines, pos + 1, else_pos, if_body_indent)?
        } else {
            Self::parse_indent_block(lines, pos + 1, if_end, if_body_indent)?
        };

        // 解析 ELSE 部分
        let (false_block, after_else) = if else_pos < if_end {
            let else_line = &lines[else_pos];
            let (elif_sid, else_kw) = Self::split_step_prefix(&else_line.content);
            if else_kw.starts_with("ELIF ") {
                // ELIF 递归处理
                let elif_rest = else_kw.strip_prefix("ELIF ").unwrap();
                let (elif_step, elif_end) = Self::parse_if_block(
                    elif_sid,
                    elif_rest,
                    lines,
                    else_pos,
                    if_end,
                    parent_indent,
                )?;
                (vec![elif_step], elif_end)
            } else if else_kw.starts_with("ELSE") {
                // ELSE 块
                let else_body_indent = if_body_indent;
                let parent_level = parent_indent;
                let mut else_end = else_pos + 1;
                while else_end < if_end {
                    if lines[else_end].indent <= parent_level {
                        break;
                    }
                    else_end += 1;
                }
                let fb = Self::parse_indent_block(lines, else_pos + 1, else_end, else_body_indent)?;
                (fb, else_end)
            } else {
                (Vec::new(), else_pos)
            }
        } else {
            (Vec::new(), if_end)
        };

        Ok((
            Step::If {
                line: lines[pos].lineno,
                step_id,
                condition,
                true_block,
                false_block,
            },
            after_else,
        ))
    }

    #[allow(dead_code)]
    fn find_else_or_elif(lines: &[Line], start: usize, end: usize, indent_level: usize) -> usize {
        for i in start..end {
            let line = &lines[i];
            if line.indent == indent_level {
                let (_, kw) = Self::split_step_prefix(line.content.trim());
                if kw.starts_with("ELSE") || kw.starts_with("ELIF") {
                    return i;
                }
            }
            if line.indent < indent_level {
                break;
            }
        }
        end
    }

    /// 解析 IF 的主体部分，找到 ELSE/ELIF 的位置
    #[allow(dead_code)]
    fn parse_if_body(
        lines: &[Line],
        start: usize,
        end: usize,
        base_indent: usize,
    ) -> ChainResult<(Vec<Step>, usize)> {
        let mut steps = Vec::new();
        let mut i = start;
        let mut else_pos = end;

        while i < end {
            let line = &lines[i];

            if line.indent < base_indent {
                break;
            }

            if line.indent > base_indent {
                i += 1;
                continue;
            }

            // 顶级语句
            if line.content.starts_with("ELIF ")
                || line.content == "ELSE"
                || line.content.starts_with("ELSE ")
            {
                else_pos = i;
                break;
            }

            let (step, next_i) = Self::parse_indent_statement(lines, i, end, base_indent)?;
            steps.push(step);
            i = next_i;
        }

        Ok((steps, else_pos))
    }

    /// 解析 FOR 块
    #[allow(dead_code)]
    fn parse_for_block(
        step_id: Option<String>,
        rest: &str,
        lines: &[Line],
        pos: usize,
        end: usize,
        parent_indent: usize,
    ) -> ChainResult<(Step, usize)> {
        let rest = rest.trim_end();
        let rest = rest
            .strip_suffix(':')
            .ok_or_else(|| ChainError::new(lines[pos].lineno, "FOR 语句缺少结尾冒号"))?;

        // 找 FOR 块的结束位置
        let for_end = Self::find_child_block_end(lines, pos + 1, end, parent_indent);

        // 解析循环体
        let body = Self::parse_indent_block(lines, pos + 1, for_end, parent_indent + 1)?;

        // 判断是范围循环还是数组遍历
        if rest.contains(" TO ") {
            let (from, to) = Self::parse_for_range(rest, lines[pos].lineno)?;
            Ok((
                Step::ForLoop {
                    line: lines[pos].lineno,
                    step_id,
                    from,
                    to,
                    body,
                },
                for_end,
            ))
        } else {
            let array = Self::parse_value(rest.trim(), lines[pos].lineno)?;
            Ok((
                Step::ForEach {
                    line: lines[pos].lineno,
                    step_id,
                    array,
                    body,
                },
                for_end,
            ))
        }
    }

    /// 解析 FOR 范围
    fn parse_for_range(rest: &str, lineno: usize) -> ChainResult<(Value, Value)> {
        // 支持格式：
        // 1. $var = 1 TO 10
        // 2. 1 TO 10
        let rest = rest.trim();

        let (from_str, to_str) = if let Some((f, t)) = rest.split_once(" TO ") {
            (f.trim(), t.trim())
        } else {
            return Err(ChainError::new(
                lineno,
                "FOR 语法错误，期望 'FOR start TO end:'",
            ));
        };

        let from_str = from_str.strip_prefix("$").unwrap_or(from_str);
        let from_str = from_str.strip_prefix("index = ").unwrap_or(from_str);
        let from_str = from_str.strip_prefix("index=").unwrap_or(from_str);

        let from = Self::parse_value(from_str.trim(), lineno)?;
        let to = Self::parse_value(to_str, lineno)?;

        Ok((from, to))
    }

    // ── 块解析 ───────────────────────────────────────────────────────────

    #[allow(dead_code)]
    fn parse_block(
        lines: &[Line],
        base_indent: usize,
        start: usize,
        end: usize,
    ) -> ChainResult<Vec<Step>> {
        let mut steps = Vec::new();
        let mut i = start;
        while i < end {
            let line = &lines[i];
            if line.indent < base_indent {
                break;
            }
            if line.indent > base_indent {
                // 已在控制流关键字中处理，独立出现说明格式错误
                return Err(ChainError::new(line.lineno, "unexpected indent"));
            }
            let (step, next_i) = Self::parse_statement(lines, base_indent, i, end)?;
            steps.push(step);
            i = next_i;
        }
        Ok(steps)
    }

    /// 解析一条语句（可能消费多行：控制流块）
    #[allow(dead_code)]
    fn parse_statement(
        lines: &[Line],
        base_indent: usize,
        pos: usize,
        end: usize,
    ) -> ChainResult<(Step, usize)> {
        let line = &lines[pos];
        let content = &line.content;

        // ── ────────────────────────────────────────── INPUT name=expr
        // 声明工作流入参：StartNode 的输出引脚可通过 input.name 引用
        // 支持格式（空格分隔多参数）：
        //   INPUT name:类型              - 声明入参，引用时用 input.name，必填
        //   INPUT name:类型=默认值       - 声明入参，带默认值，可选
        //   INPUT name1:类型 name2:类型  - 多参数
        //   INPUT()                      - 无参数
        // 识别方式：INPUT 关键字后跟空格/Tab/括号/无内容，才视为 INPUT 声明
        if content.starts_with("INPUT") {
            // 检查是否是有效的 INPUT 声明（关键字后跟空格、Tab、括号、或字符串结尾）
            let rest = content.strip_prefix("INPUT").unwrap_or("");
            let is_valid = rest.is_empty()
                || rest.starts_with(' ')
                || rest.starts_with('\t')
                || rest.starts_with('(');
            if is_valid {
                // 获取 INPUT 后面的内容
                let rest = if !rest.is_empty() {
                    rest.trim_start()
                } else {
                    ""
                };
                // 支持多个声明用空格分隔：INPUT url:String keyword:String
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let mut steps_out: Vec<Step> = Vec::new();

                for part in parts {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }

                    let (param_name, rest_part) = if let Some((n, r)) = part.split_once(':') {
                        (n.trim().to_string(), r.trim().to_string())
                    } else {
                        return Err(ChainError::new(line.lineno, format!(
                            "INPUT 参数 '{}' 缺少类型声明，正确格式：名称:类型（如 url:String、file:Path、count:i64=10）",
                            part
                        )));
                    };

                    let (type_str, default) = if let Some(eq_pos) = rest_part.find('=') {
                        let type_part = rest_part[..eq_pos].trim();
                        let default_part = rest_part[eq_pos + 1..].trim();
                        let default_val = if default_part.is_empty() {
                            None
                        } else {
                            Some(Self::parse_value(default_part, line.lineno)?)
                        };
                        (type_part.to_string(), default_val)
                    } else {
                        (rest_part.trim().to_string(), None)
                    };

                    let param_type = if type_str.is_empty() {
                        return Err(ChainError::new(line.lineno, format!(
                            "INPUT parameter '{}' is missing a type. Use INPUT name:Type or INPUT name:Type=default.",
                            param_name
                        )));
                    } else if !is_builtin_type(&type_str) {
                        return Err(ChainError::new(line.lineno, format!(
                            "INPUT 参数 '{}' 的类型 '{}' 不是有效类型。\n可用类型：String、Path（文件路径）、Date（日期）、Time（时间）、i64（整数）、f64（小数）、bool、Array<T>（列表）、Object",
                            param_name, type_str
                        )));
                    } else {
                        Some(type_str)
                    };

                    if let Some(ref type_name) = param_type {
                        Self::validate_input_default(
                            &param_name,
                            type_name,
                            default.as_ref(),
                            line.lineno,
                        )?;
                    }

                    let var_name = param_name.clone();
                    steps_out.push(Step::Input {
                        line: line.lineno,
                        param_name,
                        var_name,
                        param_type,
                        default,
                    });
                }

                // 返回多个 INPUT 声明
                if steps_out.is_empty() {
                    return Err(ChainError::new(line.lineno, "INPUT 声明不能为空"));
                }
                // 多声明返回一个 Block 或单个
                if steps_out.len() == 1 {
                    return Ok((steps_out.remove(0), pos + 1));
                }
                return Ok((Step::Block(steps_out), pos + 1));
            }
        }

        // 旧格式兼容处理（已废弃）
        let (break_sid2, break_kw2) = Self::split_step_prefix(content);
        if break_kw2 == "BREAK" {
            return Ok((
                Step::Break {
                    line: line.lineno,
                    step_id: break_sid2,
                },
                pos + 1,
            ));
        }

        // ── RETURN ───────────────────────────────────────────────────
        if let Some(rest) = content
            .strip_prefix("RETURN ")
            .or_else(|| content.strip_prefix("RETURN\t"))
        {
            let assigns = Self::parse_assignments(rest, line.lineno)?;
            return Ok((
                Step::Return {
                    line: line.lineno,
                    assigns,
                },
                pos + 1,
            ));
        }
        if content == "RETURN" {
            return Ok((
                Step::Return {
                    line: line.lineno,
                    assigns: Vec::new(),
                },
                pos + 1,
            ));
        }

        // ── IF / ELIF（脱糖为嵌套 IF）────────────────────────────────
        let (if_step_id, content_for_if) = Self::split_step_prefix(content);
        if let Some(rest) = content_for_if
            .strip_prefix("IF ")
            .or_else(|| content_for_if.strip_prefix("ELIF "))
        {
            let rest = rest.trim_end();
            let cond_str = rest
                .strip_suffix(':')
                .ok_or_else(|| ChainError::new(line.lineno, "IF/ELIF 语句缺少结尾冒号"))?;
            let condition = Self::parse_value(cond_str.trim(), line.lineno)?;

            // 收集 true block（到同级 ELIF/ELSE/块结束为止）
            let (true_end, elif_or_else) = Self::find_else_or_end(lines, base_indent, pos + 1, end);
            let true_block = Self::parse_block(lines, base_indent + 1, pos + 1, true_end)?;

            // 收集 false block：
            //   ELIF → 把 ELIF 行当作新 IF，递归 parse_statement（脱糖）
            //   ELSE → 按普通块解析
            //   无   → 空
            let (false_block, next_i) = if let Some(el) = elif_or_else {
                let el_content = lines[el].content.trim_start();
                let (_, el_kw) = Self::split_step_prefix(el_content);
                if el_kw.starts_with("ELIF ") {
                    // ELIF 脱糖：将此行以下当作独立的 IF 语句解析
                    let (nested_step, after) = Self::parse_statement(lines, base_indent, el, end)?;
                    (vec![nested_step], after)
                } else {
                    // ELSE
                    let false_end = Self::find_block_end(lines, base_indent, el + 1, end);
                    let fb = Self::parse_block(lines, base_indent + 1, el + 1, false_end)?;
                    (fb, false_end)
                }
            } else {
                (Vec::new(), true_end)
            };

            return Ok((
                Step::If {
                    line: line.lineno,
                    step_id: if_step_id,
                    condition,
                    true_block,
                    false_block,
                },
                next_i,
            ));
        }

        // ── FOR ... IN ... (ForEach) ─────────────────────────────────
        let (for_step_id, content_for_loop) = Self::split_step_prefix(content);
        if let Some(rest) = content_for_loop.strip_prefix("FOR ") {
            let rest = rest.trim_end();
            let rest_no_colon = rest
                .strip_suffix(':')
                .ok_or_else(|| ChainError::new(line.lineno, "FOR 语句缺少结尾冒号"))?;

            let body_end = Self::find_block_end(lines, base_indent, pos + 1, end);
            let body = Self::parse_block(lines, base_indent + 1, pos + 1, body_end)?;

            // FOR start TO end: → ForLoopNode（$index 隐式）
            if let Some((from_str, to_str)) = rest_no_colon.split_once(" TO ") {
                let from = Self::parse_value(from_str.trim(), line.lineno)?;
                let to = Self::parse_value(to_str.trim(), line.lineno)?;
                return Ok((
                    Step::ForLoop {
                        line: line.lineno,
                        step_id: for_step_id,
                        from,
                        to,
                        body,
                    },
                    body_end,
                ));
            }

            // FOR $array: → ForEachNode（$item / $index 隐式）
            let array = Self::parse_value(rest_no_colon.trim(), line.lineno)?;
            return Ok((
                Step::ForEach {
                    line: line.lineno,
                    step_id: for_step_id,
                    array,
                    body,
                },
                body_end,
            ));
        }

        // ── CALL ─────────────────────────────────────────────────────
        // $var = CALL name(inputs...)   或   CALL name(inputs...)
        if content.contains("CALL ") {
            return Self::parse_call_statement(content, line.lineno).map(|s| (s, pos + 1));
        }

        // ── 可变变量初始化：$var = <非节点调用表达式> ────────────────
        // 节点调用形如 `UpperName(...)`; VarInit 仅允许 `$var = literal`.
        if content.starts_with('$') {
            let (maybe_var, rest_after) = Self::split_assignment(content);
            if let Some(var_name) = maybe_var {
                // rest_after 不是 PascalCase 节点调用 → VarInit
                let is_node_call = rest_after
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false)
                    && rest_after.contains('(')
                    && !rest_after.starts_with("$(");
                if !is_node_call {
                    let initial = Self::parse_value(rest_after, line.lineno)?;
                    Self::validate_var_initializer(&var_name, &initial, line.lineno)?;
                    return Ok((
                        Step::VarInit {
                            line: line.lineno,
                            name: var_name,
                            initial,
                        },
                        pos + 1,
                    ));
                }
                // $var = NodeType(...) 是合法的节点调用语法
                // 直接交给 parse_node_statement 处理
            }
        }

        // ── 普通节点：[$var =] NodeType(inputs...) ───────────────────
        Self::parse_node_statement(content, line.lineno).map(|s| (s, pos + 1))
    }

    // ── 块边界查找 ───────────────────────────────────────────────────────

    /// 找 ELSE/ELIF 行或块结束位置，返回 (true_block_end, Option<else_or_elif_line_index>)
    #[allow(dead_code)]
    fn find_else_or_end(
        lines: &[Line],
        base_indent: usize,
        start: usize,
        end: usize,
    ) -> (usize, Option<usize>) {
        let mut i = start;
        while i < end {
            let l = &lines[i];
            if l.indent < base_indent {
                break;
            }
            if l.indent == base_indent {
                let c = l.content.trim_start();
                let (_, kw) = Self::split_step_prefix(c);
                if kw.starts_with("ELSE") || kw.starts_with("ELIF ") {
                    return (i, Some(i));
                }
            }
            i += 1;
        }
        (i, None)
    }

    #[allow(dead_code)]
    fn find_block_end(lines: &[Line], base_indent: usize, start: usize, end: usize) -> usize {
        let mut i = start;
        while i < end {
            if lines[i].indent <= base_indent {
                let c = lines[i].content.trim();
                if lines[i].indent == base_indent
                    && (c.starts_with("CASE ") || c.starts_with("DEFAULT") || c.starts_with("ELSE"))
                {
                    return i;
                }
                return i;
            }
            i += 1;
        }
        end
    }

    // ── 语句级解析 ───────────────────────────────────────────────────────

    /// 解析 `[N:] CALL name(inputs...)` 或 `$var = CALL name(inputs...)`
    fn parse_call_statement(content: &str, lineno: usize) -> ChainResult<Step> {
        Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::UnknownOperation,
            format!(
                "`CALL` subgraph syntax has been removed from workflow scripts: {}",
                content
            ),
        )
        .with_suggestion(
            "Use the runtime workflow test/run tool with either script text or a script file name instead.",
        ))
    }

    /// 解析 `[N:] NodeType(inputs...)` 或 `[$var =] NodeType(inputs...)`
    fn parse_node_statement(content: &str, lineno: usize) -> ChainResult<Step> {
        let (step_id, rest) = Self::split_step_prefix(content);
        // 尝试旧语法：$var = NodeType(...)
        let (old_var, rest2) = if step_id.is_none() {
            Self::split_assignment(content)
        } else {
            (None, rest)
        };
        let actual_rest = if step_id.is_some() { rest } else { rest2 };
        let final_step_id = step_id.or(old_var);
        let (node_type, inputs) = Self::parse_node_call(actual_rest, lineno)?;
        Ok(Step::Node {
            line: lineno,
            step_id: final_step_id,
            node_type,
            inputs,
        })
    }

    ///
    fn split_step_prefix(content: &str) -> (Option<String>, &str) {
        if let Some(colon_pos) = content.find(':') {
            let before = content[..colon_pos].trim();
            if !before.is_empty() && before.chars().all(|c| c.is_ascii_digit() || c == '.') {
                let rest = content[colon_pos + 1..].trim();
                return (Some(before.to_string()), rest);
            }
        }
        (None, content)
    }

    /// 分离 `$var = rest` 或 `&pin = rest` → (Some("name"), "rest")；无赋值则 (None, content)
    fn split_assignment(content: &str) -> (Option<String>, &str) {
        // 查找 `$xxx = ` 或 `&xxx = ` 模式（等号前后可有空格）
        if content.starts_with('$') || content.starts_with('&') {
            if let Some(eq_pos) = content.find('=') {
                // 确保等号前面是变量名部分（无括号等）
                let before_eq = &content[..eq_pos];
                if !before_eq.contains('(') && !before_eq.contains(')') {
                    let var_name = before_eq[1..].trim().to_string();
                    let rest = content[eq_pos + 1..].trim();
                    return (Some(var_name), rest);
                }
            }
        }
        (None, content)
    }

    /// DSL 节点名 → 注册表类型名，找不到则报错
    fn resolve_node_type(name: &str, lineno: usize) -> ChainResult<String> {
        if NodeRegistry::get(name).is_some() {
            return Ok(name.to_string());
        }
        let with_node = format!("{}Node", name);
        if NodeRegistry::get(&with_node).is_some() {
            return Ok(with_node);
        }
        // 从注册表收集候选名，生成 "did you mean?" 建议
        let candidates: Vec<String> = NodeRegistry::all()
            .into_iter()
            .map(|m| m.node_type.to_string())
            .collect();
        Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::UnknownOperation,
            format!(
                "未知节点类型 `{}`。\n提示：节点名必须是注册表中存在的类型，如 OpenBrowser、ClickElement。",
                name
            ),
        )
        .with_suggest_from(name, candidates))
    }

    /// 解析 `Name(pin=val, ...)` 或 `Name(val1, val2, ...)` → (resolved_type, inputs)
    ///
    /// 支持两种调用语法（可混用）：
    /// - **具名参数**：`AddNode(A=3.0, B=4.0)` — 明确指定引脚名
    /// - **位置参数**：`AddNode(3.0, 4.0)` — 按注册表 DataInput 顺序自动填名
    /// - **混合**：已命名的不占位置，剩余位置参数依次填入
    fn parse_node_call(s: &str, lineno: usize) -> ChainResult<(String, Vec<(String, Value)>)> {
        let paren_start = s
            .find('(')
            .ok_or_else(|| ChainError::new(lineno, format!("缺少左括号: {}", s)))?;
        let dsl_name = s[..paren_start].trim();
        let paren_end = Self::find_matching_paren(s, paren_start, lineno)?;
        let inner = &s[paren_start + 1..paren_end];

        // SetVar 特殊位置参数语法：SetVar($var, value)
        // DSL 关键字是 "SetVar"，但注册表节点类型是 "SetVarNode"
        if dsl_name == "SetVar" {
            let inputs = Self::parse_setvar_args(inner, lineno)?;
            return Ok(("SetVarNode".to_string(), inputs));
        }

        // 归一化：将 DSL 名解析为注册表类型名（如 "StringAppend" → "StringAppendNode"）
        let name = Self::resolve_node_type(dsl_name, lineno)?;

        // 检测是否存在位置参数（顶层没有 `=` 的分段）
        let parts = Self::split_top_level(inner.trim(), ',');
        let has_positional = parts
            .iter()
            .filter(|p| !p.trim().is_empty())
            .any(|p| !Self::has_top_level_eq(p.trim()));

        let inputs = if has_positional {
            let pin_names = Self::data_input_pin_names(&name);
            Self::parse_positional_or_named(&parts, &pin_names, lineno)?
        } else {
            Self::parse_pin_assignments(inner, lineno)?
        };
        Ok((name, inputs))
    }

    /// 判断字符串在顶层（深度=0）是否包含 `=`
    fn has_top_level_eq(s: &str) -> bool {
        let mut depth = 0i32;
        let mut in_str = false;
        for c in s.chars() {
            match c {
                '"' => in_str = !in_str,
                _ if in_str => {}
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                '=' if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    /// 从注册表返回节点所有 DataInput 引脚名（按声明顺序）
    fn data_input_pin_names(node_type: &str) -> Vec<String> {
        NodeRegistry::get(node_type)
            .map(|meta| {
                meta.pins
                    .iter()
                    .filter(|p| matches!(p.kind, PinKind::DataInput))
                    .map(|p| p.name.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 支持具名+位置混合解析：已命名的不占位置，剩余位置参数找下一个可用引脚
    fn parse_positional_or_named(
        parts: &[&str],
        pin_names: &[String],
        lineno: usize,
    ) -> ChainResult<Vec<(String, Value)>> {
        // 第一轮：收集已命名引脚
        let mut result: Vec<(String, Value)> = Vec::new();
        let mut positional_parts: Vec<&str> = Vec::new();

        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if Self::has_top_level_eq(part) {
                let eq = part.find('=').unwrap();
                let pin_name = part[..eq].trim().to_string();
                let val_str = part[eq + 1..].trim();
                let value = Self::parse_value(val_str, lineno)?;
                result.push((pin_name, value));
            } else {
                positional_parts.push(part);
            }
        }

        // 第二轮：按顺序填充位置参数到尚未使用的引脚
        let named: std::collections::HashSet<String> =
            result.iter().map(|(n, _)| n.clone()).collect();
        let mut remaining: Vec<&String> =
            pin_names.iter().filter(|n| !named.contains(*n)).collect();
        let mut rem_iter = remaining.drain(..);

        for part in positional_parts {
            let pin_name = rem_iter.next().ok_or_else(|| {
                ChainError::new(
                    lineno,
                    format!("位置参数超出 DataInput 引脚数量，无法分配: {}", part),
                )
            })?;
            let value = Self::parse_value(part, lineno)?;
            result.push((pin_name.clone(), value));
        }

        Ok(result)
    }

    /// 解析 `SetVar($var, value)` 的位置参数 → [("Name", Literal("var")), ("Value", val)]
    fn parse_setvar_args(inner: &str, lineno: usize) -> ChainResult<Vec<(String, Value)>> {
        let parts = Self::split_top_level(inner.trim(), ',');
        if parts.len() < 2 {
            return Err(ChainError::new(
                lineno,
                format!("SetVar 需要两个参数 ($var, value)，得到: {}。\n提示：正确格式：SetVar($变量名, 新值)", inner),
            ));
        }
        let var_part = parts[0].trim();
        let val_part = parts[1].trim();

        // 第一个参数必须是 $var_name → 提取名称作为字符串字面量写入 Name 引脚
        if !var_part.starts_with('$') || var_part.len() < 2 {
            return Err(ChainError::new(
                lineno,
                format!("SetVar 第一个参数必须是 $变量名，得到: {}。\n提示：第一个参数必须以 $ 开头，如 SetVar($sum, 10)", var_part),
            ));
        }
        let var_name = &var_part[1..];
        let value = Self::parse_value(val_part, lineno)?;

        Ok(vec![
            (
                "Name".to_string(),
                Value::Literal(serde_json::Value::String(var_name.to_string())),
            ),
            ("Value".to_string(), value),
        ])
    }

    // ── 值解析 ───────────────────────────────────────────────────────────

    /// 解析 `RETURN a=$x, b=$(Node(...))` 中的赋值列表
    fn parse_assignments(s: &str, lineno: usize) -> ChainResult<Vec<(String, Value)>> {
        Self::parse_pin_assignments(s, lineno)
    }

    /// 解析圆括号内的引脚赋值：`pin1=val1, pin2=val2`
    fn parse_pin_assignments(s: &str, lineno: usize) -> ChainResult<Vec<(String, Value)>> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(Vec::new());
        }
        let parts = Self::split_top_level(s, ',');
        let mut result = Vec::new();
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let eq_pos = part.find('=')
                .ok_or_else(|| ChainError::new(lineno, format!(
                    "引脚赋值缺少 '=': {}。\n提示：正确的格式是 pinName=值，如 url=\"https://example.com\"",
                    part
                )))?;
            let pin_name = part[..eq_pos].trim().to_string();
            let val_str = part[eq_pos + 1..].trim();
            let value = Self::parse_value(val_str, lineno)?;
            result.push((pin_name, value));
        }
        Ok(result)
    }

    /// 解析单个值：N.pin / input.pin / $var / $(Node(...)) / 字面量
    fn parse_value(s: &str, lineno: usize) -> ChainResult<Value> {
        let s = s.trim();

        // 内联 Pure 节点: $(NodeType(inputs...))[selector]
        if s.starts_with("$(") {
            return Self::parse_inline_expr(s, lineno);
        }

        // 步骤引用或输入引用：含有 `.` 的表达式
        let dot_count = s.matches('.').count();
        if dot_count >= 1 {
            if let Some((prefix, suffix)) = s.split_once('.') {
                // 工作流入参引用: input.pin_name
                if prefix.eq_ignore_ascii_case("input") {
                    if !suffix.is_empty() {
                        return Ok(Value::InputRef(suffix.to_string()));
                    }
                }
                // 新语法：数字开头 = StepRef（如 1.Body, 1.1.Result）
                // 需要找到最后一个 `.` 之后的部分作为 pin_name，前面作为 step_id
                else if prefix
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    // 找最后一个 `.` 来分割 step_id 和 pin_name
                    // 例如 "1.1.Result" → step_id="1.1", pin_name="Result"
                    // 例如 "1.Body" → step_id="1", pin_name="Body"
                    if let Some(last_dot) = s.rfind('.') {
                        let potential_pin = &s[last_dot + 1..];
                        let potential_step = &s[..last_dot];
                        // pin_name 应该以字母开头（不是数字），否则整个都是 step_id 的一部分
                        if !potential_pin.is_empty()
                            && potential_pin
                                .chars()
                                .next()
                                .map(|c| c.is_ascii_alphabetic())
                                .unwrap_or(false)
                        {
                            return Ok(Value::StepRef {
                                step_id: potential_step.to_string(),
                                pin_name: potential_pin.to_string(),
                            });
                        }
                    }
                }
                // 旧语法兼容: $var.pin_name -> StepRef
                else if prefix.starts_with('$') && prefix.len() > 1 {
                    let var_name = prefix[1..].to_string();
                    if !suffix.is_empty() {
                        return Ok(Value::StepRef {
                            step_id: var_name,
                            pin_name: suffix.to_string(),
                        });
                    }
                }
            }
        }

        // 旧语法兼容: &pin_name（已废弃，推荐用 N.pin 或 input.pin）
        // &x -> input.x (工作流入参引用)
        // &xxx -> NodeRef (需要根据上下文判断是入参还是节点输出，为简化暂时都当入参)
        if s.starts_with('&') && s.len() > 1 {
            let name = &s[1..];
            if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                // 暂时都转为 InputRef（向后兼容）
                return Ok(Value::InputRef(name.to_string()));
            }
        }

        // 变量引用: $var_name（可变）
        if s.starts_with('$')
            && s.len() > 1
            && s[1..].chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return Ok(Value::VarRef(s[1..].to_string()));
        }

        // 字符串字面量: "..." 或 '...'
        if (s.starts_with('"') && s.ends_with('"'))
            || (s.starts_with('\'') && s.ends_with('\'')) && s.len() >= 2
        {
            let inner = &s[1..s.len() - 1];
            return Ok(Value::Literal(JsonValue::String(inner.to_string())));
        }

        // bool
        if s == "true" {
            return Ok(Value::Literal(JsonValue::Bool(true)));
        }
        if s == "false" {
            return Ok(Value::Literal(JsonValue::Bool(false)));
        }

        // null
        if s == "null" {
            return Ok(Value::Literal(JsonValue::Null));
        }

        // 数字（整数或浮点）
        if let Ok(n) = s.parse::<i64>() {
            return Ok(Value::Literal(JsonValue::Number(n.into())));
        }
        if let Ok(n) = s.parse::<f64>() {
            if let Some(num) = serde_json::Number::from_f64(n) {
                return Ok(Value::Literal(JsonValue::Number(num)));
            }
        }

        if (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('{') && s.ends_with('}')) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(s) {
                return Ok(Value::Literal(value));
            }
        }

        Err(ChainError::new(lineno, format!(
            "无法解析值: '{}'。\n提示：字符串必须用双引号包围，如 url=\"https://example.com\"，不要使用单引号或转义符。",
            s
        )))
    }

    /// 解析 `$(NodeType(inputs...))[Selector]` 内联表达式
    fn validate_input_default(
        param_name: &str,
        type_name: &str,
        default: Option<&Value>,
        lineno: usize,
    ) -> ChainResult<()> {
        let Some(Value::Literal(value)) = default else {
            return Ok(());
        };
        if Self::literal_matches_type(value, type_name) {
            return Ok(());
        }
        Err(ChainError::new(
            lineno,
            format!(
                "INPUT parameter '{}' default does not match declared type '{}'.",
                param_name, type_name
            ),
        ))
    }

    fn validate_var_initializer(var_name: &str, value: &Value, lineno: usize) -> ChainResult<()> {
        let Value::Literal(literal) = value else {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                format!(
                    "Variable declaration '${}' must use a static literal default.",
                    var_name
                ),
            )
            .with_suggestion(
                "Use input.name, N.Pin, $var, or pure functions directly where the value is consumed. \
Use numbered `N: setvar name = expr` only when runtime mutable state is required.",
            ));
        };

        if let JsonValue::Array(items) = literal {
            if items.is_empty() {
                return Err(ChainError::new(
                    lineno,
                    format!(
                        "Variable '${}' uses an empty array literal. Add at least one element so its type can be inferred.",
                        var_name
                    ),
                ));
            }
        }
        Ok(())
    }

    fn literal_matches_type(value: &JsonValue, type_name: &str) -> bool {
        let type_name = type_name.trim();
        if type_name.eq_ignore_ascii_case("Any") {
            return true;
        }
        if let Some(inner) = Self::array_inner_type(type_name) {
            let JsonValue::Array(items) = value else {
                return false;
            };
            return items
                .iter()
                .all(|item| Self::literal_matches_type(item, inner));
        }
        match type_name {
            "String" | "Path" | "Date" | "Time" => value.is_string(),
            "bool" | "Boolean" => value.is_boolean(),
            "i64" | "int" | "integer" => value.as_i64().is_some(),
            "f64" | "float" | "Number" => value.as_f64().is_some(),
            "Object" | "KeyValuePair" => value.is_object(),
            "Null" => value.is_null(),
            _ => true,
        }
    }

    fn array_inner_type(type_name: &str) -> Option<&str> {
        let type_name = type_name.trim();
        let inner = type_name
            .strip_prefix("Array<")
            .or_else(|| type_name.strip_prefix("Vec<"))?
            .strip_suffix('>')?
            .trim();
        if inner.is_empty() {
            None
        } else {
            Some(inner)
        }
    }

    fn parse_inline_expr(s: &str, lineno: usize) -> ChainResult<Value> {
        // s = "$(NodeType(pin=val, ...))[Selector]" 或 "$(NodeType(pin=val, ...))"
        // 先找到 $( 后匹配的 )
        let inner_start = 2; // 跳过 "$("
        let close_paren = Self::find_matching_paren(s, 1, lineno)?; // 从 '(' at index 1 开始

        let inner = &s[inner_start..close_paren]; // "NodeType(pin=val, ...)"

        // 解析 NodeType(pin=val, ...)
        let (node_type, inputs) = Self::parse_node_call(inner, lineno)?;

        // 检查 )[Selector]
        let after = &s[close_paren + 1..];
        let output_pin = if after.starts_with('[') {
            let bracket_end = after
                .find(']')
                .ok_or_else(|| ChainError::new(lineno, "内联表达式的 [引脚名] 缺少 ']'"))?;
            Some(after[1..bracket_end].to_string())
        } else {
            None
        };

        Ok(Value::Inline(Box::new(InlineExpr {
            node_type,
            inputs,
            output_pin,
        })))
    }

    /// 提取 `$var_name` 中的变量名，或接受 `_` 作为丢弃变量
    #[allow(dead_code)]
    fn expect_var_name(s: &str, lineno: usize) -> ChainResult<String> {
        let s = s.trim();
        if s == "_" {
            return Ok("_".to_string()); // 丢弃变量，不注册到 step_map
        }
        if s.starts_with('$') && s.len() > 1 {
            Ok(s[1..].to_string())
        } else {
            Err(ChainError::new(
                lineno,
                format!("期望 $变量名 或 _，得到: {}", s),
            ))
        }
    }

    // ── 通用字符串工具 ───────────────────────────────────────────────────

    /// 在顶层（不进入括号内部）按分隔符分割
    fn split_top_level(s: &str, sep: char) -> Vec<&str> {
        let mut result = Vec::new();
        let mut depth = 0i32;
        let mut in_string = false;
        let mut start = 0;
        for (i, c) in s.char_indices() {
            match c {
                '"' if depth == 0 => in_string = !in_string,
                '(' | '[' if !in_string => depth += 1,
                ')' | ']' if !in_string => depth -= 1,
                c2 if c2 == sep && depth == 0 && !in_string => {
                    result.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        if start <= s.len() {
            result.push(&s[start..]);
        }
        result
    }

    /// 找到与 pos 处 `(` 匹配的 `)` 的位置
    fn find_matching_paren(s: &str, open_pos: usize, lineno: usize) -> ChainResult<usize> {
        let bytes = s.as_bytes();
        let mut depth = 0i32;
        let mut in_string = false;
        for i in open_pos..bytes.len() {
            match bytes[i] {
                b'"' => in_string = !in_string,
                b'(' if !in_string => depth += 1,
                b')' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(i);
                    }
                }
                _ => {}
            }
        }
        Err(ChainError::new(lineno, "括号不匹配"))
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Part 3: Compiler — Chain AST → BlueprintJson（含 CSE 去重）
// ═════════════════════════════════════════════════════════════════════════════

impl ChainCompiler {
    /// 递归收集所有 INPUT 声明（包括 Block 中的）
    fn collect_input_declarations(
        steps: &[Step],
    ) -> Vec<(String, String, Option<String>, Option<Value>)> {
        let mut input_decls = Vec::new();
        for s in steps {
            match s {
                Step::Input {
                    param_name,
                    var_name,
                    param_type,
                    default,
                    ..
                } => {
                    if param_name.is_empty() || var_name.is_empty() {
                        continue;
                    }
                    input_decls.push((
                        param_name.clone(),
                        var_name.clone(),
                        param_type.clone(),
                        default.clone(),
                    ));
                }
                Step::Block(block_steps) => {
                    input_decls.extend(Self::collect_input_declarations(block_steps));
                }
                _ => {}
            }
        }
        input_decls
    }

    fn collect_writable_variables(steps: &[Step]) -> HashSet<String> {
        let mut vars = HashSet::new();
        Self::collect_writable_variables_inner(steps, &mut vars);
        vars
    }

    fn collect_writable_variables_inner(steps: &[Step], vars: &mut HashSet<String>) {
        for step in steps {
            match step {
                Step::Input { var_name, .. } | Step::VarInit { name: var_name, .. } => {
                    if !var_name.is_empty() {
                        vars.insert(var_name.clone());
                    }
                }
                Step::If {
                    true_block,
                    false_block,
                    ..
                } => {
                    Self::collect_writable_variables_inner(true_block, vars);
                    Self::collect_writable_variables_inner(false_block, vars);
                }
                Step::ForEach { body, .. } | Step::ForLoop { body, .. } => {
                    Self::collect_writable_variables_inner(body, vars);
                }
                Step::Block(block_steps) => {
                    Self::collect_writable_variables_inner(block_steps, vars);
                }
                _ => {}
            }
        }
    }

    ///
    /// 自动生成 StartNode / EndNode，exec 线自动串联。
    /// 工作流入参必须通过 `INPUT name=$var` 显式声明。
    pub fn compile(&mut self, chain: &Chain) -> ChainResult<BlueprintJson> {
        // 0. 扫描顶层和嵌套 Block 中的 Step::Input 声明，收集工作流入参
        let input_decls = Self::collect_input_declarations(&chain.steps);
        self.writable_vars = Self::collect_writable_variables(&chain.steps);

        // 1. 创建 StartNode，含显式声明的入参引脚
        let start_id = self.id_gen.start_id();
        let mut start_pins = vec![NodePin {
            name: "Out".to_string(),
            kind: "ExecOutput".to_string(),
            data_type: String::new(),
            description: String::new(),
            default_value: None,
            resolved_type: None,
            split_config: None,
        }];
        for (param_name, var_name, param_type, default) in &input_decls {
            // 默认值只放在 DataInput 上，DataOutput 透传
            let default_json = match default {
                Some(Value::Literal(j)) => Some(j.clone()),
                _ => None,
            };
            // 实际类型：有声明用声明类型，否则退化为 Any
            let data_type = param_type.clone().unwrap_or_else(|| "Any".to_string());
            // DataInput 引脚：承载外部传入值及默认值
            start_pins.push(NodePin {
                name: param_name.clone(),
                kind: "DataInput".to_string(),
                data_type: data_type.clone(),
                description: String::new(),
                default_value: default_json,
                resolved_type: None,
                split_config: None,
            });
            start_pins.push(NodePin {
                name: param_name.clone(),
                kind: "DataOutput".to_string(),
                data_type,
                description: String::new(),
                default_value: None,
                resolved_type: None,
                split_config: None,
            });
            // $var_name → StartNode 的 param_name DataOutput 引脚
            self.step_map
                .insert(var_name.clone(), (start_id.clone(), param_name.clone()));
        }
        self.nodes.push(BlueprintNodeJson {
            id: start_id.clone(),
            node_type: "StartNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: start_pins,
            properties: HashMap::new(),
            display_name: Some("开始".to_string()),
            comment: None,
        });
        self.set_exec_prev(&start_id, "Out");

        self.compile_steps(&chain.steps)?;

        // 3. 创建 EndNode，把最后的 exec 连过来
        let end_id = self.id_gen.end_id();
        let end_pins = vec![NodePin {
            name: "In".to_string(),
            kind: "ExecInput".to_string(),
            data_type: String::new(),
            description: String::new(),
            default_value: None,
            resolved_type: None,
            split_config: None,
        }];
        // 如果有 RETURN 语句里的引脚，在这里已通过连线处理
        // EndNode 的 DataInput 引脚由 RETURN 语句动态生成
        self.nodes.push(BlueprintNodeJson {
            id: end_id.clone(),
            node_type: "EndNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: end_pins,
            properties: HashMap::new(),
            display_name: Some("结束".to_string()),
            comment: None,
        });
        self.wire_exec_to(&end_id, "In");

        // 处理 RETURN 绑定：为 EndNode 动态添加 DataInput 引脚 + 数据连线
        let bindings = std::mem::take(&mut self.return_bindings);
        for (pin_name, src_node, src_pin) in bindings {
            // 向 EndNode 追加 DataInput 引脚
            if let Some(node) = self.nodes.iter_mut().find(|n| n.id == end_id) {
                node.pins.push(NodePin {
                    name: pin_name.clone(),
                    kind: "DataInput".to_string(),
                    data_type: "Any".to_string(),
                    description: String::new(),
                    default_value: None,
                    resolved_type: None,
                    split_config: None,
                });
            }
            // 数据连线（仅当源不是字面量时）
            if src_node != "__literal__" {
                self.add_connection(&src_node, &src_pin, &end_id, &pin_name, "Data");
            }
        }
        let bp = BlueprintJson {
            format: WORKFLOW_FORMAT.to_string(),
            schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
            min_runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            metadata: crate::workflow::blueprint_json::BlueprintMetadata {
                id: String::new(),
                name: String::new(),
                created: String::new(),
                modified: String::new(),
                description: String::new(),
                author: String::new(),
                tags: Vec::new(),
                visibility: BlueprintVisibility::Private,
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
            nodes: std::mem::take(&mut self.nodes),
            connections: std::mem::take(&mut self.connections),
            variables: std::mem::take(&mut self.variables),
            comments: Vec::new(),
        };
        Ok(bp)
    }

    fn compile_steps(&mut self, steps: &[Step]) -> ChainResult<()> {
        for step in steps {
            self.compile_step(step)?;
        }
        Ok(())
    }

    fn compile_step(&mut self, step: &Step) -> ChainResult<()> {
        match step {
            Step::Node {
                line,
                step_id,
                node_type,
                inputs,
            } => {
                self.current_line = *line;
                self.compile_node_step(step_id.as_deref(), node_type, inputs)
            }
            Step::Call {
                line,
                step_id,
                name,
                inputs,
            } => {
                self.current_line = *line;
                self.compile_call_step(step_id.as_deref(), name, inputs)
            }
            Step::If {
                line,
                step_id,
                condition,
                true_block,
                false_block,
            } => {
                self.current_line = *line;
                self.compile_if(step_id.as_deref(), condition, true_block, false_block)
            }
            Step::ForEach {
                line,
                step_id,
                array,
                body,
            } => {
                self.current_line = *line;
                self.compile_for_each(step_id.as_deref(), array, body)
            }
            Step::ForLoop {
                line,
                step_id,
                from,
                to,
                body,
            } => {
                self.current_line = *line;
                self.compile_for_loop(step_id.as_deref(), from, to, body)
            }
            Step::Break { line, step_id } => {
                self.current_line = *line;
                self.compile_break(step_id.as_deref())
            }
            Step::Return { line, assigns } => {
                self.current_line = *line;
                self.compile_return(assigns)
            }
            Step::Input { .. } => {
                // INPUT 声明在 compile() 阶段已处理，运行时是 no-op
                Ok(())
            }
            Step::Block(steps) => self.compile_steps(steps),
            Step::VarInit {
                line,
                name,
                initial,
            } => {
                self.current_line = *line;
                self.compile_var_init(name, initial)
            }
        }
    }

    // ── 可变变量初始化 ────────────────────────────────────────────────────

    fn compile_var_init(&mut self, name: &str, initial: &Value) -> ChainResult<()> {
        Self::validate_var_initializer(name, initial, self.current_line)?;
        let default_value = match initial {
            Value::Literal(value) => Some(value.clone()),
            _ => None,
        };
        let data_type = default_value
            .as_ref()
            .map(Self::literal_data_type)
            .unwrap_or_else(|| "Any".to_string());
        if let Some(variable) = self
            .variables
            .iter_mut()
            .find(|variable| variable.name == name)
        {
            variable.default_value = default_value;
            variable.data_type = data_type;
        } else {
            self.variables.push(BlueprintVariable {
                name: name.to_string(),
                data_type,
                default_value,
                description: String::new(),
            });
        }
        Ok(())
    }

    fn literal_data_type(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(_) => "String",
            serde_json::Value::Bool(_) => "Boolean",
            serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "i64",
            serde_json::Value::Number(_) => "f64",
            serde_json::Value::Array(_) => "Array<Any>",
            serde_json::Value::Object(_) => "Object",
            serde_json::Value::Null => "Any",
        }
        .to_string()
    }

    fn ensure_get_var_ref(&mut self, name: &str) -> (String, String) {
        if let Some(existing) = self.get_var_refs.get(name) {
            return existing.clone();
        }
        let node_id = self.next_var_init_id();
        let data_type = self
            .variables
            .iter()
            .find(|variable| variable.name == name)
            .map(|variable| variable.data_type.clone())
            .unwrap_or_else(|| "Any".to_string());
        let mut properties = HashMap::new();
        properties.insert(
            "variable_name".to_string(),
            serde_json::Value::String(name.to_string()),
        );
        self.nodes.push(BlueprintNodeJson {
            id: node_id.clone(),
            node_type: "GetVarNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: vec![
                NodePin {
                    name: "Name".to_string(),
                    kind: "DataInput".to_string(),
                    data_type: "String".to_string(),
                    description: "Declared variable or input name".to_string(),
                    default_value: Some(serde_json::Value::String(name.to_string())),
                    resolved_type: None,
                    split_config: None,
                },
                NodePin {
                    name: "Value".to_string(),
                    kind: "DataOutput".to_string(),
                    data_type,
                    description: "Current variable value".to_string(),
                    default_value: None,
                    resolved_type: None,
                    split_config: None,
                },
            ],
            properties,
            display_name: None,
            comment: None,
        });
        let result = (node_id, "Value".to_string());
        self.get_var_refs.insert(name.to_string(), result.clone());
        result
    }

    // ── 普通 Impure 节点 ─────────────────────────────────────────────────

    fn compile_node_step(
        &mut self,
        step_id: Option<&str>,
        node_type: &str,
        inputs: &[(String, Value)],
    ) -> ChainResult<()> {
        if !self.node_exists(node_type) {
            return Err(self.unknown_node_error(node_type));
        }
        if node_type == "SetVarNode" {
            self.validate_setvar_target(inputs)?;
        }

        let node_id = if let Some(sid) = step_id {
            sid.to_string()
        } else {
            self.id_gen.next_impure_id()
        };
        let pins = self.build_pins_from_registry(node_type);

        // 节点在注册表中时，校验 input 引脚名是否合法
        if !pins.is_empty() {
            let valid_inputs: Vec<&str> = pins
                .iter()
                .filter(|p| p.kind == "DataInput")
                .map(|p| p.name.as_str())
                .collect();
            for (pin_name, _) in inputs {
                if !valid_inputs.contains(&pin_name.as_str()) {
                    let available = valid_inputs.join(", ");
                    return Err(ChainError::of_kind(
                        self.current_line,
                        ChainErrorKind::UnknownReference,
                        format!(
                            "节点 {} 没有输入引脚 `{}`。\n可用引脚：{}",
                            node_type, pin_name, available
                        ),
                    )
                    .with_suggest_from(pin_name, &valid_inputs));
                }
            }
        }

        self.nodes.push(BlueprintNodeJson {
            id: node_id.clone(),
            node_type: node_type.to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins,
            properties: HashMap::new(),
            display_name: None,
            comment: None,
        });

        // Pure 节点（无 Exec 引脚）不参与 Exec 链，只通过数据连线懒求值
        let pure = self.is_pure(node_type);

        if !pure {
            // exec 连线
            self.wire_exec_to(&node_id, "In");
        }

        // data_in 连线：将 inputs 中的值连接到节点的 DataInput 引脚
        self.wire_data_inputs(&node_id, inputs)?;

        // 设置默认字面量值到节点引脚的 default_value
        self.set_literal_defaults(&node_id, inputs);

        if !pure {
            // exec 输出前进
            self.set_exec_prev(&node_id, "Then");
        }

        // 步骤绑定：将 step_id 映射到 (node_id, first_data_output_pin)
        if let Some(sid) = step_id {
            // 查找第一个 DataOutput 引脚名
            if let Some(out_pin) = self.first_data_output(node_type) {
                self.step_map
                    .insert(sid.to_string(), (node_id.clone(), out_pin));
            }
        }

        Ok(())
    }

    fn validate_setvar_target(&self, inputs: &[(String, Value)]) -> ChainResult<()> {
        let Some((_, value)) = inputs.iter().find(|(key, _)| key == "Name") else {
            return Err(ChainError::of_kind(
                self.current_line,
                ChainErrorKind::Syntax,
                "SetVarNode requires a variable name",
            ));
        };
        let Value::Literal(serde_json::Value::String(name)) = value else {
            return Err(ChainError::of_kind(
                self.current_line,
                ChainErrorKind::Syntax,
                "SetVarNode variable name must be a static $variable reference",
            ));
        };
        if self.writable_vars.contains(name) {
            return Ok(());
        }
        Err(ChainError::of_kind(
            self.current_line,
            ChainErrorKind::UnknownReference,
            format!(
                "SetVarNode can only write declared workflow variables or inputs, not '${}'",
                name
            ),
        ))
    }

    // ── CALL (SubgraphNode) ──────────────────────────────────────────────

    fn compile_call_step(
        &mut self,
        _step_id: Option<&str>,
        name: &str,
        _inputs: &[(String, Value)],
    ) -> ChainResult<()> {
        Err(ChainError::of_kind(
            self.current_line,
            ChainErrorKind::UnknownOperation,
            format!("Subgraph workflow call '{}' is no longer a blueprint node.", name),
        )
        .with_suggestion(
            "Use the runtime workflow test/run tool with either script text or a script file name instead.",
        ))
    }

    // ── IF → BranchNode ──────────────────────────────────────────────────

    fn compile_if(
        &mut self,
        step_id: Option<&str>,
        condition: &Value,
        true_block: &[Step],
        false_block: &[Step],
    ) -> ChainResult<()> {
        let branch_id = if let Some(sid) = step_id {
            sid.to_string()
        } else {
            self.id_gen.next_impure_id()
        };
        self.nodes.push(BlueprintNodeJson {
            id: branch_id.clone(),
            node_type: "BranchNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: self.build_pins_from_registry("BranchNode"),
            properties: HashMap::new(),
            display_name: Some("条件分支".to_string()),
            comment: None,
        });

        // exec in
        self.wire_exec_to(&branch_id, "In");

        // condition 数据连线
        let (cond_node, cond_pin) = self.compile_value(condition)?;
        self.add_connection(&cond_node, &cond_pin, &branch_id, "Condition", "Data");

        // True 分支
        self.exec_prev = Some((branch_id.clone(), "True".to_string()));
        self.id_gen.push_scope(&format!("{}t", branch_id));
        self.compile_steps(true_block)?;
        self.id_gen.pop_scope();
        let true_tail = self.exec_prev.take();

        // False 分支
        self.exec_prev = Some((branch_id.clone(), "False".to_string()));
        self.id_gen.push_scope(&format!("{}f", branch_id));
        self.compile_steps(false_block)?;
        self.id_gen.pop_scope();
        let false_tail = self.exec_prev.take();

        // 汇聚：两个分支的尾部都指向下一个节点（通过 exec_prev 记录两个点）
        // 存为临时"多源"列表——但 exec_prev 只存一个，所以用简单策略：
        // 创建一个隐式汇聚点（下一个节点自己处理）
        // 这里先取 true_tail，false_tail 在下一个节点创建时再补连
        self.exec_prev = true_tail;
        // 保存 false_tail 用于 merge
        if let Some(ft) = false_tail {
            // 暂存到一个辅助字段——为了简单，直接在下一个节点创建前处理
            // 我们用一个 Vec 来收集待 merge 的 exec 尾部
            self.pending_merges.push(ft);
        }

        Ok(())
    }

    // ── FOR $array → ForEachNode ─────────────────────────────────────────

    fn compile_for_each(
        &mut self,
        step_id: Option<&str>,
        array: &Value,
        body: &[Step],
    ) -> ChainResult<()> {
        let node_id = if let Some(sid) = step_id {
            sid.to_string()
        } else {
            self.id_gen.next_impure_id()
        };
        self.nodes.push(BlueprintNodeJson {
            id: node_id.clone(),
            node_type: "ForEachNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: self.build_pins_from_registry("ForEachNode"),
            properties: HashMap::new(),
            display_name: Some("遍历数组".to_string()),
            comment: None,
        });

        self.wire_exec_to(&node_id, "In");

        // Array 数据连线
        let (arr_node, arr_pin) = self.compile_value(array)?;
        self.add_connection(&arr_node, &arr_pin, &node_id, "Array", "Data");

        // $item / $index 是隐式固定变量，保存外层绑定（支持嵌套 ForEach/ForLoop）
        let saved_item = self.step_map.remove("item");
        let saved_index = self.step_map.remove("index");
        self.step_map
            .insert("item".to_string(), (node_id.clone(), "Item".to_string()));
        self.step_map
            .insert("index".to_string(), (node_id.clone(), "Index".to_string()));

        self.exec_prev = Some((node_id.clone(), "LoopBody".to_string()));
        self.id_gen.push_scope(&node_id);
        self.compile_steps(body)?;
        self.id_gen.pop_scope();
        // 循环体尾部不需要连回（执行器自动迭代）
        self.exec_prev.take();

        // 恢复外层绑定
        match saved_item {
            Some(v) => {
                self.step_map.insert("item".to_string(), v);
            }
            None => {
                self.step_map.remove("item");
            }
        }
        match saved_index {
            Some(v) => {
                self.step_map.insert("index".to_string(), v);
            }
            None => {
                self.step_map.remove("index");
            }
        }

        self.set_exec_prev(&node_id, "Completed");

        // 注册 step_id → (node_id, "Item")，供外部引用 N.Item
        if let Some(sid) = step_id {
            self.step_map
                .insert(sid.to_string(), (node_id.clone(), "Item".to_string()));
        }

        Ok(())
    }

    fn compile_for_loop(
        &mut self,
        step_id: Option<&str>,
        from: &Value,
        to: &Value,
        body: &[Step],
    ) -> ChainResult<()> {
        let node_id = if let Some(sid) = step_id {
            sid.to_string()
        } else {
            self.id_gen.next_impure_id()
        };
        self.nodes.push(BlueprintNodeJson {
            id: node_id.clone(),
            node_type: "ForLoopNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: self.build_pins_from_registry("ForLoopNode"),
            properties: HashMap::new(),
            display_name: Some("范围循环".to_string()),
            comment: None,
        });

        self.wire_exec_to(&node_id, "In");

        // FirstIndex / LastIndex 连线（字面量用 default_value，变量引用用连接）
        let range_inputs: Vec<(String, Value)> = vec![
            ("FirstIndex".to_string(), from.clone()),
            ("LastIndex".to_string(), to.clone()),
        ];
        self.wire_data_inputs(&node_id, &range_inputs)?;
        self.set_literal_defaults(&node_id, &range_inputs);

        // $index 是隐式固定变量，保存外层绑定（支持嵌套）
        let saved_index = self.step_map.remove("index");
        self.step_map
            .insert("index".to_string(), (node_id.clone(), "Index".to_string()));

        self.exec_prev = Some((node_id.clone(), "LoopBody".to_string()));
        self.id_gen.push_scope(&node_id);
        self.compile_steps(body)?;
        self.id_gen.pop_scope();
        self.exec_prev.take();

        // 恢复外层 $index 绑定
        match saved_index {
            Some(v) => {
                self.step_map.insert("index".to_string(), v);
            }
            None => {
                self.step_map.remove("index");
            }
        }

        // Completed
        self.set_exec_prev(&node_id, "Completed");

        // 注册 step_id → (node_id, "Index")，供外部引用 N.Index
        if let Some(sid) = step_id {
            self.step_map
                .insert(sid.to_string(), (node_id.clone(), "Index".to_string()));
        }

        Ok(())
    }

    fn compile_break(&mut self, step_id: Option<&str>) -> ChainResult<()> {
        let node_id = if let Some(sid) = step_id {
            sid.to_string()
        } else {
            self.id_gen.next_impure_id()
        };
        self.nodes.push(BlueprintNodeJson {
            id: node_id.clone(),
            node_type: "BreakNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins: self.build_pins_from_registry("BreakNode"),
            properties: HashMap::new(),
            display_name: Some("跳出".to_string()),
            comment: None,
        });
        self.wire_exec_to(&node_id, "In");
        self.exec_prev = None;
        Ok(())
    }

    // ── RETURN → EndNode DataInput 连线 ──────────────────────────────────

    fn compile_return(&mut self, assigns: &[(String, Value)]) -> ChainResult<()> {
        // RETURN 的值连到 EndNode 的 DataInput 引脚
        // 此时 EndNode 尚未创建——我们先记录，最后在 compile() 中处理
        for (pin_name, val) in assigns {
            let (src_node, src_pin) = self.compile_value(val)?;
            self.return_bindings
                .push((pin_name.clone(), src_node, src_pin));
        }
        Ok(())
    }

    ///
    /// - Literal: 不创建节点，返回 special marker 并在上层处理 default_value
    /// - InputRef (input.pin): 查找工作流入参的输出引脚
    /// - VarRef: 从 step_map 查
    /// - Inline: CSE 查重，创建 Pure 节点
    fn compile_value(&mut self, val: &Value) -> ChainResult<(String, String)> {
        match val {
            Value::StepRef { step_id, pin_name } => {
                let (node_id, default_pin) = self.step_map.get(step_id)
                    .cloned()
                    .ok_or_else(|| ChainError::new(self.current_line, format!(
                        "未定义的步骤: {}。\n提示：步骤必须先声明，如 `1: HttpRequest(...)` 后用 `1.Body` 引用其输出。",
                        step_id
                    )))?;

                // 获取该节点（内部 invariant，正常情况不会触发）
                let node = self.nodes.iter()
                    .find(|n| n.id == node_id)
                    .ok_or_else(|| ChainError::new(self.current_line, format!(
                        "步骤 {} 的节点未能正确生成，这是编译器内部错误，请重新检查步骤 {} 的声明是否完整。",
                        step_id, step_id
                    )))?;

                let actual_pin = if pin_name.is_empty() {
                    default_pin
                } else {
                    let normalized_pin = if node.node_type == "ForEachNode" && pin_name == "Element"
                    {
                        "Item"
                    } else {
                        pin_name.as_str()
                    };
                    let has_pin = node
                        .pins
                        .iter()
                        .any(|p| p.name == normalized_pin && p.kind == "DataOutput");

                    if !has_pin {
                        let available_pins: Vec<String> = node
                            .pins
                            .iter()
                            .filter(|p| p.kind == "DataOutput")
                            .map(|p| p.name.clone())
                            .collect();

                        let suggestion = if available_pins.is_empty() {
                            format!(
                                "步骤 {} 对应的节点 ({}) 没有任何输出引脚。",
                                step_id, node.node_type
                            )
                        } else if available_pins.len() == 1 {
                            format!("可用引脚：{}.{}", step_id, available_pins[0])
                        } else {
                            format!(
                                "可用引脚：{}",
                                available_pins
                                    .iter()
                                    .map(|p| format!("{}.{}", step_id, p))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        };

                        return Err(ChainError::of_kind(
                            self.current_line,
                            ChainErrorKind::UnknownReference,
                            format!("步骤 {} 没有引脚 `{}`。\n{}", step_id, pin_name, suggestion),
                        )
                        .with_suggest_from(pin_name, &available_pins));
                    }
                    normalized_pin.to_string()
                };

                Ok((node_id, actual_pin))
            }
            Value::InputRef(name) => {
                // 工作流入参引用，查找 start 节点的输出引脚
                let start_node = self.nodes.iter()
                    .find(|n| n.node_type == "StartNode")
                    .ok_or_else(|| ChainError::new(self.current_line,
                        "工作流缺少 INPUT 声明。\n提示：操作链第一行必须是 INPUT 声明，如 INPUT url:String。".to_string()
                    ))?;
                let start_id = start_node.id.clone();
                let has_pin = start_node
                    .pins
                    .iter()
                    .any(|p| p.name == *name && p.kind == "DataOutput");
                if !has_pin {
                    return Err(ChainError::new(self.current_line, format!(
                        "工作流入参 '{}' 不存在。\n提示：用 INPUT 声明的入参才能用 input.{} 引用，如 INPUT url:String 后用 input.url。",
                        name, name
                    )));
                }
                Ok((start_id, name.clone()))
            }
            Value::VarRef(name) => {
                if self.variables.iter().any(|variable| variable.name == *name) {
                    return Ok(self.ensure_get_var_ref(name));
                }

                // 循环隐式变量等作用域绑定已经明确到具体输出引脚。
                // 例如 $item -> ForEachNode.Item、$index -> ForEachNode.Index。
                self.step_map.get(name)
                    .cloned()
                    .ok_or_else(|| ChainError::new(self.current_line, format!(
                        "未定义的变量: ${}。\n提示：变量必须在 INPUT 声明后的第二行先初始化，如 $sum = 0.0，或使用 SetVar($var, value) 赋值。",
                        name
                    )))
            }
            Value::Inline(expr) => self.compile_inline_pure(expr),
            Value::Literal(_) => {
                // 字面量没有源节点，返回一个特殊标记
                // 上层 wire_data_inputs 会检测到这个并改用 default_value
                Ok(("__literal__".to_string(), String::new()))
            }
        }
    }

    fn compile_inline_pure(&mut self, expr: &InlineExpr) -> ChainResult<(String, String)> {
        if !self.node_exists(&expr.node_type) {
            return Err(self.unknown_node_error(&expr.node_type));
        }
        let key = self.inline_key(expr);

        // CSE 命中
        if let Some(cached) = self.pure_cache.get(&key) {
            return Ok(cached.clone());
        }

        // 未命中 → 创建节点
        let node_id = self.next_pure_node_id();
        let pins = self.build_pins_from_registry(&expr.node_type);

        self.nodes.push(BlueprintNodeJson {
            id: node_id.clone(),
            node_type: expr.node_type.clone(),
            position: NodePosition::default(),
            size: NodeSize::default(),
            pins,
            properties: HashMap::new(),
            display_name: None,
            comment: None,
        });

        // 递归连接 inputs
        self.wire_data_inputs(&node_id, &expr.inputs)?;
        self.set_literal_defaults(&node_id, &expr.inputs);

        // 确定输出引脚
        let out_pin = match &expr.output_pin {
            Some(name) => name.clone(),
            None => self.first_data_output(&expr.node_type).ok_or_else(|| {
                ChainError::of_kind(
                    self.current_line,
                    ChainErrorKind::UnknownReference,
                    format!("Node `{}` has no data output", expr.node_type),
                )
            })?,
        };

        let result = (node_id, out_pin);
        self.pure_cache.insert(key, result.clone());
        Ok(result)
    }

    // ── 数据连线辅助 ─────────────────────────────────────────────────────

    /// 将 inputs 列表中的值连到目标节点的 DataInput 引脚
    fn wire_data_inputs(
        &mut self,
        target_node: &str,
        inputs: &[(String, Value)],
    ) -> ChainResult<()> {
        for (pin_name, val) in inputs {
            match val {
                Value::Literal(_) => {
                    // 字面量通过 set_literal_defaults 处理，不需要连线
                }
                _ => {
                    let (src_node, src_pin) = self.compile_value(val)?;
                    if src_node != "__literal__" {
                        self.add_connection(&src_node, &src_pin, target_node, pin_name, "Data");
                    }
                }
            }
        }
        Ok(())
    }

    /// 将字面量值写入目标节点引脚的 default_value
    fn set_literal_defaults(&mut self, target_node: &str, inputs: &[(String, Value)]) {
        for (pin_name, val) in inputs {
            if let Value::Literal(json_val) = val {
                // 找到目标节点，设置引脚的 default_value
                if let Some(node) = self.nodes.iter_mut().find(|n| n.id == target_node) {
                    if let Some(pin) = node.pins.iter_mut().find(|p| p.name == *pin_name) {
                        pin.default_value = Some(json_val.clone());
                    }
                }
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Part 4: 公开接口
// ═════════════════════════════════════════════════════════════════════════════

impl ChainCompiler {
    /// 补全 RETURN 绑定：在 EndNode 上创建 DataInput 引脚并连线
    fn finalize_return_bindings(&mut self) {
        if self.return_bindings.is_empty() {
            return;
        }
        // 找到 EndNode
        let end_id = self
            .nodes
            .iter()
            .find(|n| n.node_type == "EndNode")
            .map(|n| n.id.clone());
        let Some(end_id) = end_id else { return };

        let bindings = std::mem::take(&mut self.return_bindings);
        for (pin_name, src_node, src_pin) in &bindings {
            // 在 EndNode 上追加 DataInput 引脚
            if let Some(end_node) = self.nodes.iter_mut().find(|n| n.id == end_id) {
                if !end_node.pins.iter().any(|p| p.name == *pin_name) {
                    end_node.pins.push(NodePin {
                        name: pin_name.clone(),
                        kind: "DataInput".to_string(),
                        data_type: "Any".to_string(),
                        description: String::new(),
                        default_value: None,
                        resolved_type: None,
                        split_config: None,
                    });
                }
            }
            // 连线
            self.add_connection(src_node, src_pin, &end_id, pin_name, "Data");
        }
    }
}

/// 一步到位：操作链文本 → BlueprintJson
///
/// ```rust,ignore
/// let bp = compile_chain(r#"
///     1. $page = OpenBrowser(url="https://example.com")
///     2. $title = GetPageTitle(page=$page)
///     RETURN title=$title
/// "#)?;
/// ```
pub fn compile_chain(text: &str) -> ChainResult<BlueprintJson> {
    let chain = ChainCompiler::parse(text)?;
    compile_chain_from_ast(&chain)
}

/// 从已解析的 Chain AST 一步到位生成 BlueprintJson
///
/// 暴露此入口供其他前端复用同一份 AST → BlueprintJson 流水线
/// （例如 `chain_compiler_v2` 用单行 CLI 语法解析后调用本函数）。
pub fn compile_chain_from_ast(chain: &Chain) -> ChainResult<BlueprintJson> {
    let mut compiler = ChainCompiler::new();
    compile_chain_from_ast_with_compiler(chain, &mut compiler)
}

pub fn compile_chain_from_ast_with_runtime_tools(
    chain: &Chain,
    runtime_tools: &[RuntimeToolMetadata],
) -> ChainResult<BlueprintJson> {
    let mut compiler = ChainCompiler::with_runtime_tools(runtime_tools);
    compile_chain_from_ast_with_compiler(chain, &mut compiler)
}

fn compile_chain_from_ast_with_compiler(
    chain: &Chain,
    compiler: &mut ChainCompiler,
) -> ChainResult<BlueprintJson> {
    // compile() 通过 std::mem::take 把 nodes/connections 移入 bp
    let mut bp = compiler.compile(chain)?;
    // finalize_return_bindings 会修补 EndNode 引脚并追加 RETURN 数据连线
    compiler.finalize_return_bindings();
    // 将 finalize 产生的额外节点/连线合并进 bp（而非替换，避免覆盖 compile 已生成内容）
    bp.nodes.extend(std::mem::take(&mut compiler.nodes));
    bp.connections
        .extend(std::mem::take(&mut compiler.connections));
    Ok(bp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_simple_node() {
        // 新语法：N: NodeType(...)
        let chain = ChainCompiler::parse(
            r#"
INPUT

            1: SomeNode(url="https://example.com", count=3)
RETURN result=1.Body
        "#,
        )
        .unwrap();
        assert_eq!(chain.steps.len(), 3);
        match &chain.steps[1] {
            Step::Node {
                step_id,
                node_type,
                inputs,
                ..
            } => {
                assert_eq!(step_id.as_deref(), Some("1"));
                assert_eq!(node_type, "SomeNode");
                assert_eq!(inputs.len(), 2);
                assert_eq!(inputs[0].0, "url");
                assert_eq!(inputs[1].0, "count");
            }
            _ => panic!("expected Step::Node"),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_simple_node_old_syntax() {
        // 旧语法兼容：$var = NodeType(...)
        let chain = ChainCompiler::parse(
            r#"
INPUT

            $result = SomeNode(url="https://example.com", count=3)
RETURN result=$result
        "#,
        )
        .unwrap();
        assert_eq!(chain.steps.len(), 3);
        match &chain.steps[1] {
            Step::Node {
                step_id,
                node_type,
                inputs,
                ..
            } => {
                assert_eq!(step_id.as_deref(), Some("result"));
                assert_eq!(node_type, "SomeNode");
                assert_eq!(inputs.len(), 2);
            }
            _ => panic!("expected Step::Node"),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_inline_pure() {
        let chain = ChainCompiler::parse(
            r#"
INPUT

            DoSomething(x=$(AddNode(A=$price, B=10.0)))
RETURN result=1
        "#,
        )
        .unwrap();
        assert_eq!(chain.steps.len(), 3);
        match &chain.steps[1] {
            Step::Node { inputs, .. } => match &inputs[0].1 {
                Value::Inline(expr) => {
                    assert_eq!(expr.node_type, "AddNode");
                    assert_eq!(expr.inputs.len(), 2);
                }
                _ => panic!("expected Value::Inline"),
            },
            _ => panic!("expected Step::Node"),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_inline_with_selector() {
        let chain = ChainCompiler::parse(
            r#"
INPUT

            DoSomething(x=$(DivModNode(A=$a, B=$b))[Quotient])
RETURN result=1
        "#,
        )
        .unwrap();
        match &chain.steps[1] {
            Step::Node { inputs, .. } => match &inputs[0].1 {
                Value::Inline(expr) => {
                    assert_eq!(expr.node_type, "DivModNode");
                    assert_eq!(expr.output_pin.as_deref(), Some("Quotient"));
                }
                _ => panic!("expected Value::Inline"),
            },
            _ => panic!("expected Step::Node"),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_if_else() {
        let chain = ChainCompiler::parse(
            r#"
INPUT

IF $flag:
    DoA(x=1)
ELSE:
    DoB(x=2)
RETURN result=1
        "#,
        )
        .unwrap();
        assert_eq!(chain.steps.len(), 3);
        match &chain.steps[1] {
            Step::If {
                true_block,
                false_block,
                ..
            } => {
                assert_eq!(true_block.len(), 1);
                assert_eq!(false_block.len(), 1);
            }
            _ => panic!("expected Step::If"),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_for_each() {
        let chain = ChainCompiler::parse(
            r#"
INPUT

FOR $urls:
    ProcessNode(url=$item)
RETURN result=1
        "#,
        )
        .unwrap();
        match &chain.steps[1] {
            Step::ForEach { body, .. } => {
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected Step::ForEach"),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_for_loop() {
        let chain = ChainCompiler::parse(
            r#"
INPUT

FOR 0 TO 10:
    DoWork()
RETURN result=1
        "#,
        )
        .unwrap();
        match &chain.steps[1] {
            Step::ForLoop { body, .. } => {
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected Step::ForLoop"),
        }
    }

    #[test]
    fn test_parse_return() {
        let chain = ChainCompiler::parse(
            r#"
INPUT

            RETURN title=$title, count=$(AddNode(A=1, B=2))
        "#,
        )
        .unwrap();
        match &chain.steps[1] {
            Step::Return { assigns, .. } => {
                assert_eq!(assigns.len(), 2);
                assert_eq!(assigns[0].0, "title");
                assert_eq!(assigns[1].0, "count");
            }
            _ => panic!("expected Step::Return"),
        }
    }

    #[test]
    fn test_cse_key_order_invariant() {
        let compiler = ChainCompiler::new();
        let expr1 = InlineExpr {
            node_type: "AddNode".to_string(),
            inputs: vec![
                ("A".to_string(), Value::VarRef("x".to_string())),
                ("B".to_string(), Value::Literal(JsonValue::from(10))),
            ],
            output_pin: None,
        };
        let expr2 = InlineExpr {
            node_type: "AddNode".to_string(),
            inputs: vec![
                ("B".to_string(), Value::Literal(JsonValue::from(10))),
                ("A".to_string(), Value::VarRef("x".to_string())),
            ],
            output_pin: None,
        };
        assert_eq!(compiler.inline_key(&expr1), compiler.inline_key(&expr2));
    }

    #[test]
    fn test_parse_step_ref() {
        // 新语法：1.Body
        let val = ChainCompiler::parse_value("1.Body", 1).unwrap();
        match val {
            Value::StepRef { step_id, pin_name } => {
                assert_eq!(step_id, "1");
                assert_eq!(pin_name, "Body");
            }
            _ => panic!("expected StepRef, got {:?}", val),
        }

        // 层级步骤引用：1.1.Result
        let val = ChainCompiler::parse_value("1.1.Result", 1).unwrap();
        match val {
            Value::StepRef { step_id, pin_name } => {
                assert_eq!(step_id, "1.1");
                assert_eq!(pin_name, "Result");
            }
            _ => panic!("expected StepRef, got {:?}", val),
        }

        // 旧语法兼容：$var.pin
        let val = ChainCompiler::parse_value("$result.Body", 1).unwrap();
        match val {
            Value::StepRef { step_id, pin_name } => {
                assert_eq!(step_id, "result");
                assert_eq!(pin_name, "Body");
            }
            _ => panic!("expected StepRef, got {:?}", val),
        }
    }

    #[test]
    #[should_panic(expected = "removed")]
    fn test_parse_step_prefix() {
        // N: NodeType(...)
        let chain = ChainCompiler::parse(
            r#"
INPUT

1: SomeNode(url="test")
2: OtherNode(data=1.Body)
RETURN result=2.Body
        "#,
        )
        .unwrap();
        assert_eq!(chain.steps.len(), 4);
        match &chain.steps[1] {
            Step::Node {
                step_id, node_type, ..
            } => {
                assert_eq!(step_id.as_deref(), Some("1"));
                assert_eq!(node_type, "SomeNode");
            }
            _ => panic!("expected Step::Node"),
        }
        match &chain.steps[2] {
            Step::Node {
                step_id,
                node_type,
                inputs,
                ..
            } => {
                assert_eq!(step_id.as_deref(), Some("2"));
                assert_eq!(node_type, "OtherNode");
                match &inputs[0].1 {
                    Value::StepRef { step_id, pin_name } => {
                        assert_eq!(step_id, "1");
                        assert_eq!(pin_name, "Body");
                    }
                    _ => panic!("expected StepRef"),
                }
            }
            _ => panic!("expected Step::Node"),
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Phase 2: 错误消息升级测试
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn err_new_autoclassifies_syntax() {
        let e = ChainError::new(3, "IF 语句缺少结尾冒号");
        assert_eq!(e.kind, ChainErrorKind::Syntax);
        assert_eq!(e.line, 3);
    }

    #[test]
    fn err_new_autoclassifies_unknown_reference_step() {
        let e = ChainError::new(5, "未定义的步骤: 2");
        assert_eq!(e.kind, ChainErrorKind::UnknownReference);
    }

    #[test]
    fn err_new_autoclassifies_unknown_reference_pin() {
        let e = ChainError::new(5, "步骤 1 没有引脚 `foo`");
        assert_eq!(e.kind, ChainErrorKind::UnknownReference);
    }

    #[test]
    fn err_new_autoclassifies_unknown_operation_when_node_missing() {
        // "未知节点类型" 走显式 of_kind 路径；这里测兜底推断
        let e = ChainError::new(1, "未知算子 foo");
        // 推断为 UnknownOperation（未包含 "引脚/步骤/变量/节点" 关键字）
        assert_eq!(e.kind, ChainErrorKind::UnknownOperation);
    }

    #[test]
    fn err_of_kind_overrides_inference() {
        let e = ChainError::of_kind(1, ChainErrorKind::TypeMismatch, "whatever text");
        assert_eq!(e.kind, ChainErrorKind::TypeMismatch);
    }

    #[test]
    fn err_with_col_sets_column() {
        let e = ChainError::new(2, "bad").with_col(12);
        assert_eq!(e.col, 12);
        assert!(format!("{}", e).contains("line 2:12"));
    }

    #[test]
    fn err_display_without_col_omits_column() {
        let e = ChainError::new(2, "bad");
        let s = format!("{}", e);
        assert!(s.contains("line 2 "));
        assert!(!s.contains(":0"));
    }

    #[test]
    fn err_display_includes_suggestion() {
        let e = ChainError::new(1, "bad").with_suggestion("try X");
        let s = format!("{}", e);
        assert!(s.contains("💡 try X"), "got: {}", s);
    }

    #[test]
    fn levenshtein_sanity() {
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("click", "clik"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn find_closest_picks_similar() {
        let cands = vec!["ClickElement", "FillInput", "OpenBrowser"];
        let hit = find_closest("ClickElment", cands); // 少了一个字母 e
        assert_eq!(hit.as_deref(), Some("ClickElement"));
    }

    #[test]
    fn find_closest_rejects_too_far() {
        let cands = vec!["ClickElement"];
        let hit = find_closest("xyz", cands);
        assert_eq!(hit, None);
    }

    #[test]
    fn with_suggest_from_appends_hint() {
        let e = ChainError::new(1, "bad").with_suggest_from("Clik", vec!["Click", "Close", "Copy"]);
        assert!(
            e.suggestion.as_deref().unwrap().contains("Click"),
            "got: {:?}",
            e.suggestion
        );
    }

    #[test]
    fn compile_unknown_node_reports_unknown_operation_with_suggestion() {
        let res = compile_chain(
            r#"
INPUT url:String

SetVarNod($x, 1)
RETURN result=$x
        "#,
        );
        let err = res.unwrap_err();
        // NodeRegistry 里有 SetVar / SetVarNode 之类的近似名才能命中 Levenshtein
        assert_eq!(err.kind, ChainErrorKind::UnknownOperation);
        // 就算 registry 里没有近似词，也至少不会 panic；suggestion 可为 None
    }

    #[test]
    fn compile_bad_step_ref_reports_unknown_reference() {
        // 引用不存在的步骤号
        let res = compile_chain(
            r#"
INPUT a:String
RETURN result=9.Body
        "#,
        );
        let err = res.unwrap_err();
        assert_eq!(err.kind, ChainErrorKind::UnknownReference);
    }

    #[test]
    fn chain_error_serializes_to_json() {
        // 确保 ChainError 能序列化（Tauri command 需要）
        let e = ChainError::of_kind(3, ChainErrorKind::UnknownOperation, "未知节点 Clik")
            .with_col(5)
            .with_suggestion("did you mean `Click`?");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["line"], 3);
        assert_eq!(json["col"], 5);
        assert_eq!(json["kind"], "unknown_operation");
        assert_eq!(json["suggestion"], "did you mean `Click`?");
    }
}
