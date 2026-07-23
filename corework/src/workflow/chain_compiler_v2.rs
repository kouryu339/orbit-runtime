//!
//! 与 `chain_compiler.rs` 共享同一份 AST (`chain_ast::Chain`) 和后端
//! (`compile_chain_from_ast`)，只替换前端语法。
//!
//! ## 与 v1 的差异
//!
//! - **节点调用**：`1: EXEC NodeType --A val --B val`
//! - **Pure 表达式**：`add(a, b)`、`gt(a, b)`、`text_concat(a, b)`
//! - **变量声明**：`$var = literal`（仅允许静态字面量默认值，不进入 Exec 链）
//! - **状态更新**：`1: setvar var = expr`（必须带步骤号，进入 Exec 链）
//! - **`IF` / `ELIF` / `FOR`**：去掉结尾冒号
//! - **`RETURN`**：空格分隔，例 `RETURN a=1.value b=2.total`（取代逗号分隔）
//! - **多行字符串**：起始 `"` 后直接换行 → heredoc，直到独占一行的 `"` 结束
//!
//! ## 复用
//!
//! 解析完成后调用 [`compile_chain_from_ast`] 走和 v1 完全相同的
//! AST → BlueprintJson 流水线，确保两个前端产生**语义等价**的蓝图。

use serde_json::Value as JsonValue;

use crate::workflow::blueprint_json::BlueprintJson;
use crate::workflow::chain_ast::*;
use crate::workflow::chain_compiler::{
    compile_chain_from_ast, compile_chain_from_ast_with_runtime_tools, ChainError, ChainErrorKind,
    ChainResult,
};
use crate::workflow::pure_function_codec;
use crate::workflow::syntax_lex::{self, LogicalLine};

// ─────────────────────────────────────────────────────────────────────────────
// 公开入口
// ─────────────────────────────────────────────────────────────────────────────

/// 一步到位：v2 语法文本 → BlueprintJson
pub fn compile_chain_v2(text: &str) -> ChainResult<BlueprintJson> {
    let chain = parse_v2(text)?;
    compile_chain_from_ast(&chain)
}

pub fn compile_chain_v2_with_runtime_tools(
    text: &str,
    runtime_tools: &[crate::rpc_tool::RuntimeToolMetadata],
) -> ChainResult<BlueprintJson> {
    let chain = parse_v2(text)?;
    compile_chain_from_ast_with_runtime_tools(&chain, runtime_tools)
}

/// 解析 v2 语法文本为 Chain AST
pub fn parse_v2(text: &str) -> ChainResult<Chain> {
    let lines = preprocess(text)?;
    if lines.is_empty() {
        return Err(ChainError::new(1, "工作流不能为空"));
    }

    let mut parser = Parser {
        lines,
        pos: 0,
        loop_depth: 0,
    };
    parser.parse_chain()
}

// ─────────────────────────────────────────────────────────────────────────────
// 预处理：行与 heredoc
// ─────────────────────────────────────────────────────────────────────────────

/// 把原始文本切成逻辑行：
/// - 跳过注释 `#`
/// - 跳过空行
/// - 合并 heredoc（起始 `"` 后直接换行 → 直到独占一行的 `"`）
fn preprocess(text: &str) -> ChainResult<Vec<LogicalLine>> {
    syntax_lex::preprocess_lines(text, true)
        .map_err(|err| ChainError::of_kind(err.lineno, ChainErrorKind::Syntax, err.message))
}

// ─────────────────────────────────────────────────────────────────────────────
// 词法：把一行切分成 chunk（以空白分隔，但保留引号 / 括号 / 方括号内的整体）
// ─────────────────────────────────────────────────────────────────────────────

/// 把单行（可能含 heredoc 合并的多行字符串）切分为 chunk：
/// - 字符串 `"..."` / `'...'` 视为整体，可包含换行
/// - `(...)` / `[...]` 内部不切分
/// - 多个空白 / 制表符视作分隔
fn tokenize_chunks(line: &str) -> Vec<String> {
    syntax_lex::tokenize_chunks(line)
}

fn starts_with_keyword(content: &str, keyword: &str) -> bool {
    strip_keyword(content, keyword).is_some()
}

fn strip_keyword<'a>(content: &'a str, keyword: &str) -> Option<&'a str> {
    let prefix = content.get(..keyword.len())?;
    if !prefix.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = &content[keyword.len()..];
    (rest.is_empty()
        || rest
            .chars()
            .next()
            .map(char::is_whitespace)
            .unwrap_or(false))
    .then_some(rest)
}

// ─────────────────────────────────────────────────────────────────────────────
// Parser
// ─────────────────────────────────────────────────────────────────────────────

struct Parser {
    lines: Vec<LogicalLine>,
    pos: usize,
    loop_depth: usize,
}

impl Parser {
    fn parse_chain(&mut self) -> ChainResult<Chain> {
        let mut steps: Vec<Step> = Vec::new();

        // 1. 第一行必须是 INPUT
        let first = self
            .lines
            .first()
            .ok_or_else(|| ChainError::new(1, "工作流不能为空"))?;
        if !starts_with_keyword(&first.content, "input") {
            return Err(ChainError::of_kind(
                first.lineno,
                ChainErrorKind::Syntax,
                "第1行必须是 INPUT 声明",
            ));
        }
        let input_step = parse_input_decl(&first.content, first.lineno)?;
        steps.push(input_step);
        self.pos = 1;

        // 2. 循环解析中段，直到 RETURN
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            if starts_with_keyword(&line.content, "return") {
                break;
            }
            let step = self.parse_statement()?;
            steps.push(step);
        }

        // 3. 最后必须是 RETURN
        if self.pos >= self.lines.len() {
            let last_lineno = self.lines.last().map(|l| l.lineno).unwrap_or(1);
            return Err(ChainError::of_kind(
                last_lineno,
                ChainErrorKind::Syntax,
                "缺少 RETURN 语句",
            ));
        }
        let ret_line = &self.lines[self.pos];
        let ret_step = parse_return(&ret_line.content, ret_line.lineno)?;
        steps.push(ret_step);
        self.pos += 1;

        if self.pos < self.lines.len() {
            let extra = &self.lines[self.pos];
            return Err(ChainError::of_kind(
                extra.lineno,
                ChainErrorKind::Syntax,
                "RETURN 之后不应再有其它语句",
            ));
        }

        Ok(Chain { steps })
    }

    /// 解析任意语句（消费一行或多行 IF/FOR 块）
    fn parse_statement(&mut self) -> ChainResult<Step> {
        let line = self.lines[self.pos].clone();
        let lineno = line.lineno;
        let content = line.content.as_str();

        let (step_id, rest) = split_step_prefix(content);

        if starts_with_keyword(rest, "input") {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                format!(
                    "第 {lineno} 行不能再次声明 INPUT。Workflow 只能有一条 INPUT 逻辑行；多个输入请写在同一行，例如：`input video_path:String title:String=\"\" description:String=\"\"`"
                ),
            ));
        }

        // 关键字优先
        if rest == "BREAK" {
            if self.loop_depth == 0 {
                return Err(ChainError::of_kind(
                    lineno,
                    ChainErrorKind::Syntax,
                    "BREAK 必须写在 FOR 循环体内",
                ));
            }
            self.pos += 1;
            return Ok(Step::Break {
                line: lineno,
                step_id,
            });
        }
        if rest == "ELSE" || rest.starts_with("ELSE ") {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "ELSE 必须紧跟在 IF 后面（应在 IF 块解析时消费）",
            ));
        }
        if rest == "ELIF" || rest.starts_with("ELIF ") {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "ELIF 必须紧跟在 IF 后面",
            ));
        }
        if rest == "END" {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "END 没有对应的 IF/FOR",
            ));
        }
        if rest.starts_with("IF ") || rest == "IF" {
            return self.parse_if(step_id, rest, lineno);
        }
        if rest.starts_with("FOR ") || rest == "FOR" {
            return self.parse_for(step_id, rest, lineno);
        }

        // 赋值 `$var = expr`
        if rest.starts_with('$') && contains_assign_eq(rest) {
            self.pos += 1;
            return parse_assignment(step_id, rest, lineno);
        }

        // 普通节点调用 / setvar
        self.pos += 1;
        parse_node_call_line(step_id, rest, lineno)
    }

    fn parse_if(
        &mut self,
        step_id: Option<String>,
        rest: &str,
        lineno: usize,
    ) -> ChainResult<Step> {
        // 读 condition：IF cond  / IF
        let cond_str = rest
            .strip_prefix("IF")
            .ok_or_else(|| ChainError::of_kind(lineno, ChainErrorKind::Syntax, "IF 关键字缺失"))?
            .trim();
        if cond_str.is_empty() {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "IF 缺少条件表达式",
            ));
        }
        let condition = parse_value(cond_str, lineno)?;

        self.pos += 1;
        let mut true_block = Vec::new();
        let mut false_block: Vec<Step> = Vec::new();

        // true 块：解析直到 ELIF / ELSE / END
        while self.pos < self.lines.len() {
            let l = self.lines[self.pos].clone();
            let l_lineno = l.lineno;
            let (_, l_rest) = split_step_prefix(&l.content);
            if l_rest == "END" {
                self.pos += 1;
                return Ok(Step::If {
                    line: lineno,
                    step_id,
                    condition,
                    true_block,
                    false_block,
                });
            }
            if l_rest.starts_with("ELIF ") || l_rest == "ELIF" {
                // 把 ELIF 转成嵌套 IF
                let (elif_sid, _) = split_step_prefix(&l.content);
                let elif_rest = format!("IF{}", &l_rest["ELIF".len()..]);
                let elif_step = self.parse_if(elif_sid, &elif_rest, l_lineno)?;
                false_block = vec![elif_step];
                return Ok(Step::If {
                    line: lineno,
                    step_id,
                    condition,
                    true_block,
                    false_block,
                });
            }
            if l_rest == "ELSE" {
                self.pos += 1;
                while self.pos < self.lines.len() {
                    let m = self.lines[self.pos].clone();
                    let (_, m_rest) = split_step_prefix(&m.content);
                    if m_rest == "END" {
                        self.pos += 1;
                        return Ok(Step::If {
                            line: lineno,
                            step_id,
                            condition,
                            true_block,
                            false_block,
                        });
                    }
                    let s = self.parse_statement()?;
                    false_block.push(s);
                }
                return Err(ChainError::of_kind(
                    l_lineno,
                    ChainErrorKind::Syntax,
                    "ELSE 块缺少 END",
                ));
            }
            let s = self.parse_statement()?;
            true_block.push(s);
        }
        Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "IF 块缺少 END",
        ))
    }

    fn parse_for(
        &mut self,
        step_id: Option<String>,
        rest: &str,
        lineno: usize,
    ) -> ChainResult<Step> {
        let header = rest
            .strip_prefix("FOR")
            .ok_or_else(|| ChainError::of_kind(lineno, ChainErrorKind::Syntax, "FOR 关键字缺失"))?
            .trim();
        if header.is_empty() {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "FOR 缺少迭代源 / 范围",
            ));
        }

        // 判断是 range（含 " TO "）还是 foreach
        let is_range = header.split_whitespace().any(|w| w == "TO");

        self.pos += 1;
        self.loop_depth += 1;
        let body = (|| {
            let mut body: Vec<Step> = Vec::new();
            loop {
                if self.pos >= self.lines.len() {
                    return Err(ChainError::of_kind(
                        lineno,
                        ChainErrorKind::Syntax,
                        "FOR 块缺少 END",
                    ));
                }
                let line = self.lines[self.pos].clone();
                let (_, l_rest) = split_step_prefix(&line.content);
                if l_rest == "END" {
                    self.pos += 1;
                    return Ok(body);
                }
                let s = self.parse_statement()?;
                body.push(s);
            }
        })();
        self.loop_depth -= 1;
        let body = body?;

        if is_range {
            let (from, to) = parse_for_range(header, lineno)?;
            Ok(Step::ForLoop {
                line: lineno,
                step_id,
                from,
                to,
                body,
            })
        } else {
            let array = parse_value(header, lineno)?;
            Ok(Step::ForEach {
                line: lineno,
                step_id,
                array,
                body,
            })
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 行级解析
// ─────────────────────────────────────────────────────────────────────────────

/// 解析 INPUT 声明：`INPUT name[=default] [name:Type[=default] ...]`
fn parse_input_decl(content: &str, lineno: usize) -> ChainResult<Step> {
    let rest = strip_keyword(content, "input").unwrap_or("").trim_start();

    if rest.is_empty() {
        return Ok(Step::Input {
            line: lineno,
            param_name: String::new(),
            var_name: String::new(),
            param_type: None,
            default: None,
        });
    }

    let chunks = normalize_input_chunks(tokenize_chunks(rest), lineno)?;
    let mut steps_out = Vec::new();
    for raw in chunks {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        let (name, type_name, default) = if let Some((name, type_part)) = part.split_once(':') {
            if let Some(eq) = type_part.find('=') {
                let type_name = crate::data_type::public_type_name(&type_part[..eq]);
                let default = parse_value(type_part[eq + 1..].trim(), lineno)?;
                (name, Some(type_name), Some(default))
            } else {
                (
                    name,
                    Some(crate::data_type::public_type_name(type_part)),
                    None,
                )
            }
        } else if let Some((name, default)) = part.split_once('=') {
            (name, None, Some(parse_value(default.trim(), lineno)?))
        } else {
            (part, None, None)
        };
        steps_out.push(Step::Input {
            line: lineno,
            param_name: name.trim().to_string(),
            var_name: name.trim().to_string(),
            param_type: type_name,
            default,
        });
    }

    if steps_out.is_empty() {
        return Ok(Step::Input {
            line: lineno,
            param_name: String::new(),
            var_name: String::new(),
            param_type: None,
            default: None,
        });
    }
    if steps_out.len() == 1 {
        return Ok(steps_out.into_iter().next().unwrap());
    }
    Ok(Step::Block(steps_out))
}

fn normalize_input_chunks(chunks: Vec<String>, lineno: usize) -> ChainResult<Vec<String>> {
    let mut normalized = Vec::new();
    let mut index = 0;
    while index < chunks.len() {
        let mut current = chunks[index].clone();
        if current == "=" {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "INPUT 默认值的 `=` 前缺少字段声明",
            ));
        }

        if current.ends_with('=') {
            let value = chunks.get(index + 1).ok_or_else(|| {
                ChainError::of_kind(
                    lineno,
                    ChainErrorKind::Syntax,
                    "INPUT 默认值的 `=` 后缺少值",
                )
            })?;
            current.push_str(value);
            index += 2;
        } else if chunks.get(index + 1).is_some_and(|next| next == "=") {
            let value = chunks.get(index + 2).ok_or_else(|| {
                ChainError::of_kind(
                    lineno,
                    ChainErrorKind::Syntax,
                    "INPUT 默认值的 `=` 后缺少值",
                )
            })?;
            current.push('=');
            current.push_str(value);
            index += 3;
        } else if chunks
            .get(index + 1)
            .is_some_and(|next| next.starts_with('='))
        {
            current.push_str(&chunks[index + 1]);
            index += 2;
        } else {
            index += 1;
        }
        normalized.push(current);
    }
    Ok(normalized)
}

/// 解析 RETURN：`RETURN [field=val ...]`（空格分隔）
fn parse_return(content: &str, lineno: usize) -> ChainResult<Step> {
    let rest = strip_keyword(content, "return").unwrap_or("").trim_start();
    if rest.is_empty() {
        return Ok(Step::Return {
            line: lineno,
            assigns: Vec::new(),
        });
    }
    let chunks = tokenize_chunks(rest);
    let mut assigns = Vec::new();
    for chunk in chunks {
        if chunk.is_empty() {
            continue;
        }
        let eq = chunk.find('=').ok_or_else(|| {
            ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                format!("RETURN 项缺少 `=`：`{}`，正确形式：`field=value`", chunk),
            )
        })?;
        let pin_name = chunk[..eq].trim().to_string();
        let value = parse_value(chunk[eq + 1..].trim(), lineno)?;
        assigns.push((pin_name, value));
    }
    Ok(Step::Return {
        line: lineno,
        assigns,
    })
}

/// 解析 `$var = expr` → VarInit
fn parse_assignment(step_id: Option<String>, rest: &str, lineno: usize) -> ChainResult<Step> {
    if step_id.is_some() {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "运行时赋值语法已禁用；请使用显式 `N: SetVar $var value`",
        ));
    }

    let eq = rest
        .find('=')
        .ok_or_else(|| ChainError::of_kind(lineno, ChainErrorKind::Syntax, "赋值缺少 `=`"))?;
    let lhs = rest[..eq].trim();
    let rhs = rest[eq + 1..].trim();

    if !lhs.starts_with('$') || lhs.len() < 2 {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("赋值左侧必须是 `$varname`，得到：`{}`", lhs),
        ));
    }
    let var_name = lhs[1..].to_string();
    let value = parse_value(rhs, lineno)?;

    Ok(Step::VarInit {
        line: lineno,
        name: var_name,
        initial: value,
    })
}

/// 解析普通节点调用：`N: EXEC NodeType --A val --B val ...`
fn parse_node_call_line(step_id: Option<String>, rest: &str, lineno: usize) -> ChainResult<Step> {
    if let Some(setvar) = strip_keyword(rest, "setvar") {
        let step_id = step_id.ok_or_else(|| {
            ChainError::of_kind(lineno, ChainErrorKind::Syntax, "`setvar` 必须带步骤号")
        })?;
        let inputs = parse_setvar_args(setvar.trim(), lineno)?;
        return Ok(Step::Node {
            line: lineno,
            step_id: Some(step_id),
            node_type: "SetVarNode".to_string(),
            inputs,
        });
    }

    if let Some((alias, expression)) = rest.split_once('=') {
        if strip_keyword(expression.trim(), "exec").is_some() {
            let step = step_id.as_deref().unwrap_or("N");
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                format!(
                    "外部工具结果不能赋值给别名 `{}`。请直接写 `{step}: EXEC Tool --param value`，并通过 `{step}.pin` 引用该步骤的输出",
                    alias.trim()
                ),
            ));
        }
    }

    let call = strip_keyword(rest, "exec").ok_or_else(|| {
        ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "外部工具步骤必须使用 `N: EXEC Tool --param value`；请确认该工具已激活，并保持工具声明中的参数名不变",
        )
    })?;
    let step_id = step_id.ok_or_else(|| {
        ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "外部工具步骤缺少编号。请在完整 `EXEC` 调用前添加顺序编号，例如：`1: EXEC Tool --param value`",
        )
    })?;
    let chunks = tokenize_chunks(call);
    if chunks.is_empty() {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "节点调用为空",
        ));
    }

    let node_name = chunks[0].clone();
    if node_name.starts_with('-') || node_name.starts_with('$') {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("节点名缺失或非法：`{}`", node_name),
        ));
    }

    let inputs = parse_call_args(&chunks[1..], &node_name, lineno)?;
    Ok(Step::Node {
        line: lineno,
        step_id: Some(step_id),
        node_type: node_name,
        inputs,
    })
}

/// 解析 `--key val`/位置参数混合（与 inline 共享逻辑）
fn parse_call_args(
    chunks: &[String],
    node_type: &str,
    lineno: usize,
) -> ChainResult<Vec<(String, Value)>> {
    parse_inline_args(chunks, node_type, lineno)
}

/// 状态更新：`setvar name = value`
fn parse_setvar_args(text: &str, lineno: usize) -> ChainResult<Vec<(String, Value)>> {
    let Some((name, value)) = text.split_once('=') else {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "`setvar` 语法应为 `N: setvar name = value`",
        ));
    };
    let name = name.trim().trim_start_matches('$');
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("非法变量名：`{}`", name),
        ));
    }
    let value = parse_value(value.trim(), lineno)?;
    Ok(vec![
        (
            "Name".to_string(),
            Value::Literal(JsonValue::String(name.to_string())),
        ),
        ("Value".to_string(), value),
    ])
}

// ─────────────────────────────────────────────────────────────────────────────
// 值解析
// ─────────────────────────────────────────────────────────────────────────────

/// 解析单个值 chunk
pub(crate) fn parse_value(s: &str, lineno: usize) -> ChainResult<Value> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            "值为空",
        ));
    }

    if s.starts_with('[') {
        return parse_array_expression(s, lineno);
    }

    // Pure 函数：add(a, b)、gt(a, b)、text_concat(a, b)
    if s.ends_with(')')
        && s.chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
    {
        return parse_pure_function(s, lineno);
    }

    // 字符串字面量
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
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
    if s == "null" {
        return Ok(Value::Literal(JsonValue::Null));
    }

    // 数字
    if let Ok(n) = s.parse::<i64>() {
        return Ok(Value::Literal(JsonValue::Number(n.into())));
    }
    if let Ok(n) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return Ok(Value::Literal(JsonValue::Number(num)));
        }
    }

    // input.xxx
    if let Some(rest) = s.strip_prefix("input.") {
        if !rest.is_empty() {
            return Ok(Value::InputRef(rest.to_string()));
        }
    }

    // 步骤引用：`N.field` 或 `N.M.field`
    if let Some(first_char) = s.chars().next() {
        if first_char.is_ascii_digit() {
            if let Some(last_dot) = s.rfind('.') {
                let pin = &s[last_dot + 1..];
                let step = &s[..last_dot];
                if !pin.is_empty()
                    && pin
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphabetic())
                        .unwrap_or(false)
                {
                    return Ok(Value::StepRef {
                        step_id: step.to_string(),
                        pin_name: pin.to_string(),
                    });
                }
            }
        }
    }

    // $var
    if let Some(rest) = s.strip_prefix('$') {
        if rest.is_empty() {
            return Err(ChainError::of_kind(
                lineno,
                ChainErrorKind::Syntax,
                "`$` 后缺少变量名",
            ));
        }
        if rest.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Ok(Value::VarRef(rest.to_string()));
        }
    }

    Err(ChainError::of_kind(
        lineno,
        ChainErrorKind::Syntax,
        format!(
            "无法解析值：`{}`。提示：字符串需用 `\"...\"`，引用需用 `$var` / `N.pin` / `input.x`，pure 使用 `add(...)` 这类函数",
            s
        ),
    ))
}

fn parse_array_expression(s: &str, lineno: usize) -> ChainResult<Value> {
    if !s.ends_with(']') {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("数组表达式缺少结束的 `]`：`{s}`"),
        ));
    }
    let inner = &s[1..s.len() - 1];
    let values = if inner.trim().is_empty() {
        Vec::new()
    } else {
        split_top_level(inner, ',')
            .into_iter()
            .map(|item| {
                let item = item.trim();
                if item.is_empty() {
                    return Err(ChainError::of_kind(
                        lineno,
                        ChainErrorKind::Syntax,
                        format!("数组表达式存在空项：`{s}`"),
                    ));
                }
                parse_value(item, lineno)
            })
            .collect::<ChainResult<Vec<_>>>()?
    };

    if values
        .iter()
        .all(|value| matches!(value, Value::Literal(_)))
    {
        let items = values
            .into_iter()
            .map(|value| match value {
                Value::Literal(item) => item,
                _ => unreachable!("literal array checked above"),
            })
            .collect();
        return Ok(Value::Literal(JsonValue::Array(items)));
    }

    let mut chunks = values.chunks(5).map(make_array_inline);
    let Some(mut result) = chunks.next() else {
        return Ok(Value::Literal(JsonValue::Array(Vec::new())));
    };
    for next in chunks {
        result = Value::Inline(Box::new(InlineExpr {
            node_type: "ArrayConcatNode".to_string(),
            inputs: vec![("Array1".to_string(), result), ("Array2".to_string(), next)],
            output_pin: Some("Result".to_string()),
        }));
    }
    Ok(result)
}

fn make_array_inline(values: &[Value]) -> Value {
    Value::Inline(Box::new(InlineExpr {
        node_type: "MakeArrayNode".to_string(),
        inputs: values
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, value)| (format!("Element{index}"), value))
            .collect(),
        output_pin: Some("Array".to_string()),
    }))
}

fn parse_pure_function(text: &str, lineno: usize) -> ChainResult<Value> {
    let open = text
        .find('(')
        .ok_or_else(|| ChainError::of_kind(lineno, ChainErrorKind::Syntax, "pure 函数缺少 `(`"))?;
    let close = find_matching_paren(text, open, lineno)?;
    if close + 1 != text.len() {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("pure 函数后存在多余内容：`{}`", &text[close + 1..]),
        ));
    }
    let name = text[..open].trim();
    let spec = pure_function_codec::by_function_name(name).ok_or_else(|| {
        ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("未知 pure 函数：`{}`", name),
        )
    })?;
    let args = split_top_level(&text[open + 1..close], ',')
        .into_iter()
        .filter(|arg| !arg.trim().is_empty())
        .map(|arg| parse_value(arg.trim(), lineno))
        .collect::<ChainResult<Vec<_>>>()?;
    if args.len() != spec.input_pins.len() {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!(
                "pure 函数 `{}` 需要 {} 个参数，得到 {} 个",
                name,
                spec.input_pins.len(),
                args.len()
            ),
        ));
    }
    Ok(Value::Inline(Box::new(pure_function_codec::inline_expr(
        spec, args,
    ))))
}

/// 内联节点的参数解析：支持 `--key val` 和位置参数
fn parse_inline_args(
    chunks: &[String],
    node_type: &str,
    lineno: usize,
) -> ChainResult<Vec<(String, Value)>> {
    let mut named: Vec<(String, Value)> = Vec::new();
    let mut positional: Vec<Value> = Vec::new();

    let mut i = 0;
    while i < chunks.len() {
        let c = &chunks[i];
        if let Some(key) = strip_flag(c) {
            if i + 1 >= chunks.len() {
                return Err(ChainError::of_kind(
                    lineno,
                    ChainErrorKind::Syntax,
                    format!("`--{}` 缺少值", key),
                ));
            }
            let v = parse_value(&chunks[i + 1], lineno)?;
            named.push((key, v));
            i += 2;
        } else {
            positional.push(parse_value(c, lineno)?);
            i += 1;
        }
    }

    if positional.is_empty() {
        return Ok(named);
    }

    // 用 NodeRegistry 把位置参数对应到引脚名
    let pin_names = data_input_pin_names(node_type);
    let used: std::collections::HashSet<String> = named.iter().map(|(n, _)| n.clone()).collect();
    let remaining: Vec<&String> = pin_names.iter().filter(|n| !used.contains(*n)).collect();
    if positional.len() > remaining.len() {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!(
                "节点 `{}` 位置参数 {} 个，超出可用 DataInput 引脚 {} 个",
                node_type,
                positional.len(),
                remaining.len()
            ),
        ));
    }

    let mut result = named;
    for (pin, val) in remaining.iter().zip(positional.into_iter()) {
        result.push((pin.to_string(), val));
    }
    Ok(result)
}

fn data_input_pin_names(node_type: &str) -> Vec<String> {
    use crate::workflow::registry::{NodeRegistry, PinKind};
    let lookup = |name: &str| -> Option<Vec<String>> {
        NodeRegistry::get(name).map(|meta| {
            meta.pins
                .iter()
                .filter(|p| matches!(p.kind, PinKind::DataInput))
                .map(|p| p.name.to_string())
                .collect()
        })
    };
    if let Some(v) = lookup(node_type) {
        return v;
    }
    let with_node = format!("{}Node", node_type);
    if let Some(v) = lookup(&with_node) {
        return v;
    }
    Vec::new()
}

// ─────────────────────────────────────────────────────────────────────────────
// 辅助
// ─────────────────────────────────────────────────────────────────────────────

fn strip_flag(s: &str) -> Option<String> {
    if let Some(rest) = s.strip_prefix("--") {
        if !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Some(rest.to_string());
        }
    }
    None
}

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

/// 判断字符串顶层是否包含赋值用 `=`
fn contains_assign_eq(s: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_str: Option<char> = None;
    let mut prev_escape = false;
    for c in s.chars() {
        if prev_escape {
            prev_escape = false;
            continue;
        }
        if c == '\\' && in_str.is_some() {
            prev_escape = true;
            continue;
        }
        if let Some(q) = in_str {
            if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_str = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '=' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// FOR range header：`X TO Y` / `$var TO $var2`
fn parse_for_range(s: &str, lineno: usize) -> ChainResult<(Value, Value)> {
    let chunks = tokenize_chunks(s);
    // 期望形式：[FROM, "TO", TO]
    if chunks.len() != 3 || chunks[1] != "TO" {
        return Err(ChainError::of_kind(
            lineno,
            ChainErrorKind::Syntax,
            format!("FOR range 语法应为 `X TO Y`，得到：`{}`", s),
        ));
    }
    let from = parse_value(&chunks[0], lineno)?;
    let to = parse_value(&chunks[2], lineno)?;
    Ok((from, to))
}

/// 找匹配的 `)`：从字符串中第一个 `(` 的位置开始（参数 `open_pos` 指 `(` 字符索引）
fn find_matching_paren(s: &str, open_pos: usize, lineno: usize) -> ChainResult<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;
    let mut prev_escape = false;
    for i in open_pos..bytes.len() {
        let c = bytes[i];
        if prev_escape {
            prev_escape = false;
            continue;
        }
        if c == b'\\' && in_str.is_some() {
            prev_escape = true;
            continue;
        }
        if let Some(q) = in_str {
            if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => in_str = Some(c),
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err(ChainError::of_kind(
        lineno,
        ChainErrorKind::Syntax,
        "括号不匹配",
    ))
}

fn split_top_level(s: &str, separator: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (index, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string.is_some() {
            escaped = true;
            continue;
        }
        if let Some(quote) = in_string {
            if ch == quote {
                in_string = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            _ if ch == separator && paren_depth == 0 && bracket_depth == 0 => {
                result.push(&s[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::Context;
    use crate::rpc_tool::{RuntimeAIOutputField, RuntimeAIParameter, RuntimeToolMetadata};
    use crate::workflow::blueprint_loader::BlueprintLoader;
    use crate::workflow::dynamic_node::DynamicExecute;
    use crate::workflow::execution::{ExecutionContext, WorkflowNodeStatus};
    use crate::world::FrameworkState;
    use async_trait::async_trait;
    use std::collections::HashMap;

    fn browser_tool_metadata(name: &str) -> RuntimeToolMetadata {
        RuntimeToolMetadata {
            name: name.to_string(),
            display_name: "Browser Open Page".to_string(),
            description: "Open a browser page".to_string(),
            tool_kind: "rpc".to_string(),
            parameters: vec![RuntimeAIParameter {
                name: "url".to_string(),
                param_type: "String".to_string(),
                required: true,
                default_value: None,
                description: String::new(),
            }],
            outputs: vec![
                RuntimeAIOutputField {
                    name: "page_id".to_string(),
                    field_type: "String".to_string(),
                    description: String::new(),
                },
                RuntimeAIOutputField {
                    name: "url".to_string(),
                    field_type: "String".to_string(),
                    description: String::new(),
                },
            ],
            destructive: false,
            readonly: false,
            idempotent: false,
            open_world: true,
            secret: false,
            required_capabilities: Vec::new(),
            endpoint_id: "browser".to_string(),
            service: "browser.Browser".to_string(),
            method: "OpenPage".to_string(),
        }
    }

    fn workflow_value_tool_metadata(name: &str) -> RuntimeToolMetadata {
        RuntimeToolMetadata {
            name: name.to_string(),
            display_name: "Workflow value test".to_string(),
            description: "Accept workflow value forms".to_string(),
            tool_kind: "rpc".to_string(),
            parameters: vec![
                RuntimeAIParameter {
                    name: "page_id".to_string(),
                    param_type: "String".to_string(),
                    required: true,
                    default_value: None,
                    description: String::new(),
                },
                RuntimeAIParameter {
                    name: "value".to_string(),
                    param_type: "String".to_string(),
                    required: true,
                    default_value: None,
                    description: String::new(),
                },
                RuntimeAIParameter {
                    name: "files".to_string(),
                    param_type: "Array<String>".to_string(),
                    required: true,
                    default_value: None,
                    description: String::new(),
                },
                RuntimeAIParameter {
                    name: "checked".to_string(),
                    param_type: "bool".to_string(),
                    required: true,
                    default_value: None,
                    description: String::new(),
                },
            ],
            outputs: vec![],
            destructive: false,
            readonly: false,
            idempotent: false,
            open_world: true,
            secret: false,
            required_capabilities: Vec::new(),
            endpoint_id: "workflow-value-test".to_string(),
            service: "test.WorkflowValue".to_string(),
            method: "Apply".to_string(),
        }
    }

    struct BrowserOpenPageTestSystem;

    #[async_trait]
    impl DynamicExecute for BrowserOpenPageTestSystem {
        async fn execute_dynamic(
            &self,
            input: HashMap<String, JsonValue>,
            _ctx: &Context,
        ) -> crate::error::Result<JsonValue> {
            Ok(serde_json::json!({
                "result": {
                    "page_id": "page-42",
                    "url": input.get("url").cloned().unwrap_or(JsonValue::Null)
                },
                "to_ai": "opened page-42",
                "error_code": 0
            }))
        }
    }

    #[test]
    fn runtime_tool_outputs_are_compiled_as_named_pins() {
        let tool = browser_tool_metadata("BrowserOpenPageCompileTest");
        let blueprint = compile_chain_v2_with_runtime_tools(
            r#"
input
1: EXEC BrowserOpenPageCompileTest --url "https://example.com"
return page_id=1.page_id url=1.url
"#,
            &[tool],
        )
        .unwrap();

        let node = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "BrowserOpenPageCompileTest")
            .unwrap();
        let outputs = node
            .pins
            .iter()
            .filter(|pin| pin.kind == "DataOutput")
            .map(|pin| pin.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(outputs, vec!["page_id", "url"]);
        assert!(!node.pins.iter().any(|pin| pin.name == "Result"));
        let end_node = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "EndNode")
            .unwrap();
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "1"
                && connection.source_pin == "page_id"
                && connection.target_node == end_node.id
                && connection.target_pin == "page_id"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "1"
                && connection.source_pin == "url"
                && connection.target_node == end_node.id
                && connection.target_pin == "url"
        }));
    }

    #[test]
    fn unknown_exec_node_reports_unknown_operation() {
        let error = compile_chain_v2(
            r#"
input
1: EXEC MissingRuntimeNode --url "https://example.com"
return
"#,
        )
        .unwrap_err();

        assert_eq!(error.kind, ChainErrorKind::UnknownOperation);
        assert!(error.message.contains("MissingRuntimeNode"));
        assert!(!error.message.contains("Result"));
    }

    #[tokio::test]
    async fn runtime_tool_ai_output_result_is_expanded_to_named_outputs() {
        let tool_name = "BrowserOpenPageExecutionTest";
        let framework = FrameworkState::initialize().unwrap();
        framework.registry().register_dynamic_with_metadata(
            browser_tool_metadata(tool_name),
            std::sync::Arc::new(BrowserOpenPageTestSystem),
        );
        let mut blueprint = compile_chain_v2_with_runtime_tools(
            &format!(
                "input\n1: EXEC {tool_name} --url \"https://example.com\"\nreturn page_id=1.page_id url=1.url"
            ),
            &[browser_tool_metadata(tool_name)],
        )
        .unwrap();
        blueprint.metadata.name = "runtime_result_expansion".to_string();

        let ctx = framework.create_context();
        let loaded = BlueprintLoader::new()
            .load_from_blueprint_json(blueprint, &ctx)
            .unwrap();
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        loaded
            .compiled
            .initialize_defaults(&mut exec_ctx)
            .await
            .unwrap();
        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec_ctx, HashMap::new())
            .await
            .unwrap();

        assert_eq!(
            outputs.get("page_id").and_then(|value| value.as_str()),
            Some("page-42")
        );
        assert_eq!(
            outputs.get("url").and_then(|value| value.as_str()),
            Some("https://example.com")
        );
        assert!(!outputs.contains_key("Result"));
    }

    #[test]
    fn tokenize_basic() {
        let chunks = tokenize_chunks("ReadFile --path \"C:\\out.txt\" --encoding utf-8");
        assert_eq!(
            chunks,
            vec![
                "ReadFile",
                "--path",
                "\"C:\\out.txt\"",
                "--encoding",
                "utf-8"
            ]
        );
    }

    #[test]
    fn tokenize_pure_function() {
        let chunks = tokenize_chunks("add(1.0, mul(2.0, 3.0))");
        assert_eq!(chunks, vec!["add(1.0, mul(2.0, 3.0))"]);
    }

    #[test]
    fn parse_nested_pure_function() {
        let value = parse_value("add(1.0, mul(2.0, 3.0))", 1).unwrap();
        assert!(matches!(value, Value::Inline(_)));
    }

    #[test]
    fn parse_simple_chain() {
        let chain = parse_v2(
            r#"
INPUT

RETURN result=add(3.0, 4.0)
"#,
        )
        .unwrap();
        // 至少 3 个 step：Input, Return, 中间没有
        assert!(chain.steps.len() >= 2, "{:#?}", chain);
    }

    #[test]
    fn repeated_input_reports_the_single_line_rule() {
        let error =
            parse_v2("input video_path:String\ninput title:String=\"\"\nreturn").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("不能再次声明 INPUT"), "{message}");
        assert!(message.contains("只能有一条 INPUT 逻辑行"), "{message}");
        assert!(message.contains("title:String=\"\""), "{message}");
    }

    #[test]
    fn external_tool_requires_a_step_number() {
        let error = parse_v2("input\nEXEC BrowserOpenPage --url \"https://example.com\"\nreturn")
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("缺少编号"), "{message}");
        assert!(message.contains("1: EXEC Tool"), "{message}");
    }

    #[test]
    fn external_tool_result_cannot_be_assigned_to_an_alias() {
        let error = parse_v2(
            "input\n1: upload_page = EXEC BrowserOpenPage --url \"https://example.com\"\nreturn",
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("不能赋值给别名 `upload_page`"),
            "{message}"
        );
        assert!(
            message.contains("1.page_id") || message.contains("1.pin"),
            "{message}"
        );
    }

    #[test]
    fn input_defaults_allow_spaces_around_equals() {
        let blueprint = compile_chain_v2(
            "input title:String = \"\" description:String= \"demo\" visibility:String =\"private\"\nreturn",
        )
        .unwrap();
        let start = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "StartNode")
            .unwrap();
        let output_names = start
            .pins
            .iter()
            .filter(|pin| pin.kind == "DataOutput")
            .map(|pin| pin.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(output_names, vec!["title", "description", "visibility"]);
    }

    #[test]
    fn quoted_string_defaults_preserve_escaped_inner_quotes() {
        let blueprint = compile_chain_v2(
            r#"input selector:String="button[aria-label=\"发布\"]" visibility:String="好友可见"
return selector=input.selector visibility=input.visibility"#,
        )
        .unwrap();

        let start = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "StartNode")
            .unwrap();
        assert!(start.pins.iter().any(|pin| pin.name == "selector"));
        assert!(start.pins.iter().any(|pin| pin.name == "visibility"));
    }

    #[test]
    fn dynamic_array_parses_as_inline_pure_expression() {
        let chain = parse_v2(
            "input video_path:String\n1: EXEC BrowserSetInputFiles --files [input.video_path]\nreturn",
        )
        .unwrap();
        let Step::Node { inputs, .. } = &chain.steps[1] else {
            panic!("expected workflow tool node");
        };
        assert!(matches!(
            inputs.first().map(|(_, value)| value),
            Some(Value::Inline(expr)) if expr.node_type == "MakeArrayNode"
        ));
    }

    #[test]
    fn dynamic_values_create_connections_and_bool_one_is_normalized() {
        let browser = browser_tool_metadata("BrowserOpenPageValueSyntaxTest");
        let form = workflow_value_tool_metadata("WorkflowValueSyntaxTest");
        let blueprint = compile_chain_v2_with_runtime_tools(
            r#"
input title:String video_path:String
1: EXEC BrowserOpenPageValueSyntaxTest --url "https://example.com"
2: EXEC WorkflowValueSyntaxTest --page_id 1.page_id --value input.title --files [input.video_path] --checked 1
return
"#,
            &[browser, form],
        )
        .unwrap();

        let start_id = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "StartNode")
            .map(|node| node.id.as_str())
            .unwrap();
        let array_id = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "MakeArrayNode")
            .map(|node| node.id.as_str())
            .unwrap();
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "1"
                && connection.source_pin == "page_id"
                && connection.target_node == "2"
                && connection.target_pin == "page_id"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == start_id
                && connection.source_pin == "title"
                && connection.target_node == "2"
                && connection.target_pin == "value"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == start_id
                && connection.source_pin == "video_path"
                && connection.target_node == array_id
                && connection.target_pin == "Element0"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == array_id
                && connection.source_pin == "Array"
                && connection.target_node == "2"
                && connection.target_pin == "files"
        }));
        let checked = blueprint
            .nodes
            .iter()
            .find(|node| node.id == "2")
            .and_then(|node| node.pins.iter().find(|pin| pin.name == "checked"))
            .and_then(|pin| pin.default_value.as_ref());
        assert_eq!(checked, Some(&JsonValue::Bool(true)));
    }

    #[test]
    fn literal_array_stays_a_literal_default() {
        let form = workflow_value_tool_metadata("WorkflowLiteralArrayTest");
        let blueprint = compile_chain_v2_with_runtime_tools(
            r#"
input
1: EXEC WorkflowLiteralArrayTest --page_id "page-1" --value "title" --files ["a.mp4", "b.mp4"] --checked false
return
"#,
            &[form],
        )
        .unwrap();
        assert!(!blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "MakeArrayNode"));
        let files = blueprint
            .nodes
            .iter()
            .find(|node| node.id == "1")
            .and_then(|node| node.pins.iter().find(|pin| pin.name == "files"))
            .and_then(|pin| pin.default_value.as_ref());
        assert_eq!(files, Some(&serde_json::json!(["a.mp4", "b.mp4"])));
    }

    #[tokio::test]
    async fn dynamic_array_over_five_items_executes_in_source_order() {
        let mut blueprint = compile_chain_v2(
            r#"
input a:String b:String c:String d:String e:String f:String
return files=[input.a, input.b, input.c, input.d, input.e, input.f]
"#,
        )
        .unwrap();
        blueprint.metadata.name = "dynamic_array_over_five_items".to_string();

        assert_eq!(
            blueprint
                .nodes
                .iter()
                .filter(|node| node.node_type == "MakeArrayNode")
                .count(),
            2
        );
        assert_eq!(
            blueprint
                .nodes
                .iter()
                .filter(|node| node.node_type == "ArrayConcatNode")
                .count(),
            1
        );

        let framework = FrameworkState::initialize().unwrap();
        let ctx = framework.create_context();
        let loaded = BlueprintLoader::new()
            .load_from_blueprint_json(blueprint, &ctx)
            .unwrap();
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        loaded
            .compiled
            .initialize_defaults(&mut exec_ctx)
            .await
            .unwrap();
        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(
                &mut exec_ctx,
                HashMap::from([
                    (
                        "a".to_string(),
                        crate::workflow::core::DataValue::from_string("a"),
                    ),
                    (
                        "b".to_string(),
                        crate::workflow::core::DataValue::from_string("b"),
                    ),
                    (
                        "c".to_string(),
                        crate::workflow::core::DataValue::from_string("c"),
                    ),
                    (
                        "d".to_string(),
                        crate::workflow::core::DataValue::from_string("d"),
                    ),
                    (
                        "e".to_string(),
                        crate::workflow::core::DataValue::from_string("e"),
                    ),
                    (
                        "f".to_string(),
                        crate::workflow::core::DataValue::from_string("f"),
                    ),
                ]),
            )
            .await
            .unwrap();

        assert_eq!(
            outputs.get("files").map(|value| value.json_value()),
            Some(&serde_json::json!(["a", "b", "c", "d", "e", "f"]))
        );
    }

    #[test]
    fn quoted_reference_shapes_remain_fixed_strings_without_connections() {
        let browser = browser_tool_metadata("BrowserOpenPageQuotedValueTest");
        let form = workflow_value_tool_metadata("WorkflowQuotedValueTest");
        let blueprint = compile_chain_v2_with_runtime_tools(
            r#"
input title:String video_path:String
1: EXEC BrowserOpenPageQuotedValueTest --url "https://example.com"
2: EXEC WorkflowQuotedValueTest --page_id "1.page_id" --value "input.title" --files [input.video_path] --checked true
return
"#,
            &[browser, form],
        )
        .unwrap();
        let node = blueprint.nodes.iter().find(|node| node.id == "2").unwrap();
        assert_eq!(
            node.pins
                .iter()
                .find(|pin| pin.name == "page_id")
                .and_then(|pin| pin.default_value.as_ref()),
            Some(&serde_json::json!("1.page_id"))
        );
        assert_eq!(
            node.pins
                .iter()
                .find(|pin| pin.name == "value")
                .and_then(|pin| pin.default_value.as_ref()),
            Some(&serde_json::json!("input.title"))
        );
        assert!(!blueprint.connections.iter().any(|connection| {
            connection.target_node == "2"
                && matches!(connection.target_pin.as_str(), "page_id" | "value")
        }));
    }

    #[test]
    fn quoted_array_and_bool_report_target_aware_fixes() {
        let browser = browser_tool_metadata("BrowserOpenPageQuotedTypeTest");
        let form = workflow_value_tool_metadata("WorkflowQuotedTypeTest");
        let cases = [
            (
                r#"--page_id 1.page_id --value input.title --files "[input.video_path]" --checked true"#,
                "改为 `[input.video_path]`",
            ),
            (
                r#"--page_id 1.page_id --value input.title --files [input.video_path] --checked "true""#,
                "请去掉引号，写成 `true`",
            ),
        ];
        for (args, expected) in cases {
            let script = format!(
                "input title:String video_path:String\n1: EXEC BrowserOpenPageQuotedTypeTest --url \"https://example.com\"\n2: EXEC WorkflowQuotedTypeTest {args}\nreturn"
            );
            let error =
                compile_chain_v2_with_runtime_tools(&script, &[browser.clone(), form.clone()])
                    .unwrap_err();
            assert!(error.to_string().contains(expected), "{}", error);
        }
    }

    #[test]
    fn compile_division_and_remainder_to_distinct_outputs() {
        let blueprint = compile_chain_v2(
            r#"
input dividend:num divisor:num
return quotient=div(input.dividend, input.divisor) remainder=mod(input.dividend, input.divisor)
"#,
        )
        .unwrap();

        let end_id = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "EndNode")
            .map(|node| node.id.as_str())
            .unwrap();
        let start = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "StartNode")
            .unwrap();
        assert!(start
            .pins
            .iter()
            .filter(|pin| pin.kind == "DataOutput")
            .all(|pin| pin.data_type == "num"));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_pin == "Quotient"
                && connection.target_node == end_id
                && connection.target_pin == "quotient"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_pin == "Remainder"
                && connection.target_node == end_id
                && connection.target_pin == "remainder"
        }));
    }

    #[test]
    fn numeric_input_aliases_are_canonicalized_to_num() {
        let blueprint =
            compile_chain_v2("input a:i64 b:f64 c:Number d:Integer values:Array<i64>\nreturn")
                .unwrap();
        let start = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "StartNode")
            .unwrap();
        let types = start
            .pins
            .iter()
            .filter(|pin| pin.kind == "DataOutput")
            .map(|pin| pin.data_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(types, vec!["num", "num", "num", "num", "Array<num>"]);
    }

    #[tokio::test]
    async fn num_inputs_keep_internal_numeric_transfer_implicit() {
        let mut blueprint = compile_chain_v2(
            "input dividend:num=17 divisor:num=5\nreturn quotient=div(input.dividend, input.divisor) remainder=mod(input.dividend, input.divisor)",
        )
        .unwrap();
        blueprint.metadata.name = "num_divmod".to_string();

        let framework = FrameworkState::initialize().unwrap();
        let ctx = framework.create_context();
        let loaded = BlueprintLoader::new()
            .load_from_blueprint_json(blueprint, &ctx)
            .unwrap();
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        loaded
            .compiled
            .initialize_defaults(&mut exec_ctx)
            .await
            .unwrap();
        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec_ctx, HashMap::new())
            .await
            .unwrap();

        assert_eq!(outputs["quotient"].as_i64(), Some(3));
        assert_eq!(outputs["remainder"].as_i64(), Some(2));
    }

    #[test]
    fn parse_var_init() {
        let chain = parse_v2(
            r#"
INPUT
$num = 0.0
RETURN last=$num
"#,
        )
        .unwrap();
        assert!(matches!(chain.steps[1], Step::VarInit { .. }));
    }

    #[test]
    fn parse_for_range() {
        let chain = parse_v2(
            r#"
INPUT
$num = 0.0
FOR 1 TO 10
    $num = mul($index, 1.0)
END
RETURN last=$num
"#,
        )
        .unwrap();
        // 找到 ForLoop step
        let for_step = chain
            .steps
            .iter()
            .find(|s| matches!(s, Step::ForLoop { .. }));
        assert!(for_step.is_some(), "ForLoop missing in {:#?}", chain);
    }

    #[test]
    fn reject_break_outside_for() {
        let error = parse_v2("input\n1: BREAK\nreturn").unwrap_err();
        assert_eq!(error.kind, ChainErrorKind::Syntax);
        assert!(error.message.contains("FOR"));
    }

    #[test]
    fn allow_break_inside_for() {
        parse_v2("input items:Array<String>\n1: FOR input.items\n1.1: BREAK\nEND\nreturn").unwrap();
    }

    #[test]
    fn parse_if_else() {
        let chain = parse_v2(
            r#"
INPUT x:Any
$result = 0.0
1: IF eq(input.x, 0.0)
    $result = 0.0
ELSE
    $result = input.x
END
RETURN result=$result
"#,
        )
        .unwrap();
        let has_if = chain.steps.iter().any(|s| matches!(s, Step::If { .. }));
        assert!(has_if, "If missing in {:#?}", chain);
    }

    #[test]
    fn compile_literal_declaration_with_get_var_reference() {
        let blueprint = compile_chain_v2(
            r#"
input
$label = "aa"
1: EXEC DebugPrintNode --Value $label
return
"#,
        )
        .unwrap();
        assert!(
            blueprint
                .nodes
                .iter()
                .all(|node| node.node_type != "SetVarNode"),
            "declaration must not generate SetVarNode"
        );
        assert_eq!(
            blueprint
                .nodes
                .iter()
                .filter(|node| node.node_type == "GetVarNode")
                .count(),
            1,
            "referencing the declared variable must generate one GetVarNode"
        );
    }

    #[test]
    fn compile_explicit_setvar_as_exec_node() {
        let blueprint = compile_chain_v2(
            r#"
input
$total = 0
1: setvar total = 0
return
"#,
        )
        .unwrap();
        assert!(
            blueprint
                .nodes
                .iter()
                .any(|node| node.node_type == "SetVarNode"),
            "explicit setvar must generate SetVarNode"
        );
        assert_eq!(blueprint.variables[0].name, "total");
        assert_eq!(blueprint.variables[0].data_type, "num");
        let get_var = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "GetVarNode");
        assert!(get_var.is_none(), "an unread variable needs no GetVarNode");
    }

    #[test]
    fn numeric_variable_reads_are_publicly_typed_as_num() {
        let blueprint = compile_chain_v2(
            "input\n$total = 1\n1: setvar total = add($total, 2)\nreturn result=$total",
        )
        .unwrap();
        assert_eq!(blueprint.variables[0].data_type, "num");
        let get_var = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "GetVarNode")
            .unwrap();
        assert!(get_var.pins.iter().any(|pin| {
            pin.name == "Value" && pin.kind == "DataOutput" && pin.data_type == "num"
        }));
        let set_var = blueprint
            .nodes
            .iter()
            .find(|node| node.node_type == "SetVarNode")
            .unwrap();
        assert!(!set_var
            .pins
            .iter()
            .any(|pin| pin.name == "Value" && pin.kind == "DataOutput"));

        let error =
            compile_chain_v2("input\n$total = 1\n1: setvar total = 2\nreturn result=1.Value")
                .expect_err("setvar must not expose a step output");
        assert_eq!(error.kind, ChainErrorKind::UnknownReference);
    }

    #[test]
    fn reject_legacy_var_pin_alias() {
        let error = parse_v2(
            r#"
input
1: EXEC Echo --text $result.Body
return
"#,
        )
        .expect_err("legacy `$var.pin` alias must be rejected");
        assert_eq!(error.kind, ChainErrorKind::Syntax);
    }

    #[test]
    fn compile_foreach_implicit_item_binding() {
        let blueprint = compile_chain_v2(
            r#"
input strings:Array[String]
$result = ""
$total = 0
2: FOR input.strings
    2.1: setvar result = text_concat($result, $item)
    2.2: setvar total = add($total, $index)
END
return result=$result total=$total
"#,
        )
        .unwrap();

        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "ForEachNode"));
        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "StringAppendNode"));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "2" && connection.source_pin == "Item"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "2" && connection.source_pin == "Index"
        }));

        let legacy = compile_chain_v2(
            r#"
input strings:Array[String]
2: FOR input.strings
END
return item=2.Element
"#,
        )
        .unwrap();
        assert!(legacy.connections.iter().any(|connection| {
            connection.source_node == "2" && connection.source_pin == "Item"
        }));
    }

    #[test]
    fn nested_foreach_requires_explicit_outer_item_promotion() {
        let error = compile_chain_v2(
            r#"
input groups:Array<Any>
1: FOR input.groups
    1.1: FOR $item
        1.1.1: EXEC DebugPrintNode --Value 1.Item
    END
END
return
"#,
        )
        .unwrap_err();
        assert_eq!(error.kind, ChainErrorKind::UnknownReference);
        assert!(error.message.contains("未定义的步骤: 1"), "{error}");

        let blueprint = compile_chain_v2(
            r#"
input groups:Array<Any>
$outer_item = null
1: FOR input.groups
    1.1: setvar outer_item = $item
    1.2: FOR $item
        1.2.1: EXEC DebugPrintNode --Value $item
        1.2.2: EXEC DebugPrintNode --Value $outer_item
    END
END
return
"#,
        )
        .unwrap();

        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "1"
                && connection.source_pin == "Item"
                && connection.target_node == "1.1"
                && connection.target_pin == "Value"
        }));
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == "1.2"
                && connection.source_pin == "Item"
                && connection.target_node == "1.2.1"
                && connection.target_pin == "Value"
        }));
        let promoted_read = blueprint
            .nodes
            .iter()
            .find(|node| {
                node.node_type == "GetVarNode" && node.properties["variable_name"] == "outer_item"
            })
            .unwrap();
        assert!(blueprint.connections.iter().any(|connection| {
            connection.source_node == promoted_read.id
                && connection.source_pin == "Value"
                && connection.target_node == "1.2.2"
                && connection.target_pin == "Value"
        }));
    }

    #[tokio::test]
    async fn execute_setvar_with_trace_without_ai_envelope() {
        let mut blueprint = compile_chain_v2(
            r#"
input
$total = 0
1: setvar total = 7
return result=$total
"#,
        )
        .unwrap();
        blueprint.metadata.name = "setvar_trace".to_string();

        let framework = FrameworkState::initialize().unwrap();
        let ctx = framework.create_context();
        let loaded = BlueprintLoader::new()
            .load_from_blueprint_json(blueprint, &ctx)
            .unwrap();
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        exec_ctx.enable_trace("setvar_trace", loaded.compiled.source_map.clone());
        loaded
            .compiled
            .initialize_defaults(&mut exec_ctx)
            .await
            .unwrap();

        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec_ctx, HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            outputs.get("result").and_then(|value| value.as_i64()),
            Some(7)
        );
        let trace = exec_ctx.take_trace().unwrap();
        let setvar = trace
            .nodes
            .iter()
            .find(|node| node.status == WorkflowNodeStatus::Succeeded)
            .expect("successful trace entry missing");
        assert_eq!(setvar.status, WorkflowNodeStatus::Succeeded);
        assert_eq!(setvar.output_pin.as_deref(), Some("Out"));
        assert_eq!(setvar.to_ai, None);
        assert_eq!(setvar.error_code, None);
        assert!(setvar.input_preview.is_some());
    }

    #[tokio::test]
    async fn execute_if_for_setvar_trace() {
        let mut blueprint = compile_chain_v2(
            r#"
input
$total = 0
1: setvar total = 0
2: IF gt(3, 0)
    2.1: FOR 1 TO 3
        2.1.1: setvar total = add($total, $index)
    END
END
return result=$total
"#,
        )
        .unwrap();
        blueprint.metadata.name = "if_for_setvar_trace".to_string();

        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "BranchNode"));
        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "ForLoopNode"));
        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "SetVarNode"));
        assert!(blueprint
            .nodes
            .iter()
            .any(|node| node.node_type == "AddNode"));

        let framework = FrameworkState::initialize().unwrap();
        let ctx = framework.create_context();
        let loaded = BlueprintLoader::new()
            .load_from_blueprint_json(blueprint, &ctx)
            .unwrap();
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        exec_ctx.enable_trace("if_for_setvar_trace", loaded.compiled.source_map.clone());
        loaded
            .compiled
            .initialize_defaults(&mut exec_ctx)
            .await
            .unwrap();

        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec_ctx, HashMap::new())
            .await
            .unwrap();
        let trace = exec_ctx.take_trace().unwrap();
        let result = outputs.get("result").expect("missing result output");
        assert!(
            result.as_i64() == Some(6) || result.as_f64() == Some(6.0),
            "unexpected result output: {result:?}"
        );

        assert!(
            trace
                .nodes
                .iter()
                .any(|node| node.status == WorkflowNodeStatus::Succeeded),
            "trace should record at least one successful node"
        );
        let body_runs = trace
            .nodes
            .iter()
            .filter(|node| {
                node.status == WorkflowNodeStatus::Succeeded
                    && node.to_ai.is_none()
                    && node.error_code.is_none()
                    && node.input_preview.is_some()
            })
            .count();
        assert!(
            body_runs >= 3,
            "expected successful trace entries for the loop body"
        );
    }
}
