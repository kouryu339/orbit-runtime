//! 操作链 AST 定义
//!
//! 对应 SKILL.md 中描述的操作链文本格式。

use serde_json::Value as JsonValue;

// ─────────────────────────────────────────────────────────────────────────────
// 值
// ─────────────────────────────────────────────────────────────────────────────

/// 出现在引脚槽中的表达式
#[derive(Debug, Clone)]
pub enum Value {
    /// 字面量：字符串 / 数字 / bool / null
    Literal(JsonValue),
    /// 例如：`1.Body` 表示引用步骤 1 的 Body 引脚
    StepRef { step_id: String, pin_name: String },
    /// 工作流入参引用：`input.pin_name`（来自 INPUT 声明）
    InputRef(String),
    /// 可变变量引用：`$var_name`（通过 SetVar 更新，用于循环累积等）
    VarRef(String),
    /// 内联 Pure 节点，由脚本函数表达式（如 `add(a, b)`）生成。
    Inline(Box<InlineExpr>),
}

/// 内联 Pure 节点表达式
#[derive(Debug, Clone)]
pub struct InlineExpr {
    /// 节点类型，如 `"AddNode"`
    pub node_type: String,
    pub inputs: Vec<(String, Value)>,
    /// 输出引脚选择器：None = 取第一个 DataOutput，Some(name) = `[name]`
    pub output_pin: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 步骤
// ─────────────────────────────────────────────────────────────────────────────

/// 一条执行步骤
#[derive(Debug, Clone)]
pub enum Step {
    /// 普通节点：`[N:] NodeType(inputs...)`
    Node {
        /// 源码行号（用于错误定位）
        line: usize,
        step_id: Option<String>,
        node_type: String,
        inputs: Vec<(String, Value)>,
    },
    /// 子图调用：`[N:] CALL name(inputs...)`
    Call {
        line: usize,
        step_id: Option<String>,
        name: String,
        inputs: Vec<(String, Value)>,
    },
    /// 条件分支：`[N:] IF cond: ... [ELSE: ...]`
    If {
        line: usize,
        step_id: Option<String>,
        condition: Value,
        true_block: Vec<Step>,
        false_block: Vec<Step>,
    },
    /// For-each 循环：`[N:] FOR $array:`（`$item` / `$index` 为循环体内隐式固定变量）
    ForEach {
        line: usize,
        step_id: Option<String>,
        array: Value,
        body: Vec<Step>,
    },
    /// For-range 循环：`[N:] FOR start TO end:` — `$index` 为循环体内隐式固定变量
    ForLoop {
        line: usize,
        step_id: Option<String>,
        from: Value,
        to: Value,
        body: Vec<Step>,
    },
    /// 跳出循环：`[N:] BREAK`
    Break {
        line: usize,
        step_id: Option<String>,
    },
    /// 返回：`RETURN pin=val, pin2=val2`
    Return {
        line: usize,
        assigns: Vec<(String, Value)>,
    },
    /// 可变变量初始化：`$var = literal` / `$var = &pin` / `$var = $(Inline)`
    ///
    VarInit {
        line: usize,
        name: String,
        initial: Value,
    },
    /// 工作流入参声明：`INPUT name:Type` 或 `INPUT name:Type=default`
    ///
    /// `param_name` 是 StartNode 的输出引脚名，`var_name` 是链内使用的变量名。
    /// `param_type` 是声明的类型（如 `"String"`、`"Path"`、`"i64"` 等），None 时退化为 `"Any"`。
    /// `default` 是可选的默认值（字面量）。
    Input {
        line: usize,
        param_name: String,
        var_name: String,
        param_type: Option<String>,
        default: Option<Value>,
    },
    /// 步骤块：用于多个 INPUT 声明在一行
    Block(Vec<Step>),
}

// ─────────────────────────────────────────────────────────────────────────────
// 链
// ─────────────────────────────────────────────────────────────────────────────

/// 完整操作链（顶层步骤序列）
#[derive(Debug, Clone)]
pub struct Chain {
    pub steps: Vec<Step>,
}
