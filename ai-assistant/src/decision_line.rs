//! 行式输出协议解析器（Line-Oriented Decision Protocol）
//! 详见 `docs/03_EXEC行式协议与工具执行.md`。
//! ## 协议骨架
//! ```text
//! [<think>...</think>]
//! <DECISION>[ <subject>]
//! [<payload-line>...]
//! [--<param>
//! <value-line>...]
//! ```
//! - `<DECISION>` ∈ `{ASK, EXEC, RESULT}`，独占一行（容许装饰符 / 冒号）
//! - 多个 EXEC 块由"下一个决策头"自然切分
//! ## 核心入口
//! - [`parse_line_protocol`]：返回 [`LineParseResult`]，含决策列表 + outcome 三态
//! - [`LineParseOutcome`]：Strict / Recovered / Failed，灰度埋点用

use crate::decision::AIDecision;
use corework::workflow::syntax_lex;

// ============================================================================
// 公共类型
// ============================================================================

/// 解析结果三态——灰度埋点核心
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineParseOutcome {
    /// 完全合规：首行严格匹配、无装饰符、无空行干扰
    Strict,
    /// 容错命中：解析成功但触发了某些恢复规则（hints 列出原因）
    Recovered { hints: Vec<String> },
    /// 解析失败（首行不是合法决策头 / EXEC 缺工具名 / 等）
    Failed { reason: String },
}

impl LineParseOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Strict | Self::Recovered { .. })
    }
}

/// 单个 EXEC 块的结构化表示
/// 与现有 `"ToolName --k v"` 单行字符串相比，结构化表示能承载多行参数值。
/// 通过 [`Self::to_legacy_command`] 可降级回单行命令字符串（短参数兼容路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub name: String,
    /// 参数列表（保留顺序，允许同名后写覆盖前写——兜底容错）
    pub params: Vec<(String, String)>,
}

impl ParsedToolCall {
    /// 降级为现有单行 CLI 命令字符串：`ToolName --k1 v1 --k2 "v2 with space"`
    /// 长参数 / 含换行的参数会被引号包裹并做最小转义（`"` → `\"`、`\n` 保留为 `\\n` 字面量）。
    /// 注意：此降级路径**仍受单行 CLI 限制**，长字符串场景应走 [`Self::to_input_map`]。
    pub fn to_legacy_command(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.name);
        for (k, v) in &self.params {
            out.push_str(" --");
            out.push_str(k);
            out.push(' ');
            if v.is_empty() {
                out.push_str("\"\"");
            } else if v
                .chars()
                .any(|c| c.is_whitespace() || c == '"' || c == '\\')
            {
                out.push('"');
                for ch in v.chars() {
                    match ch {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => {}
                        _ => out.push(ch),
                    }
                }
                out.push('"');
            } else {
                out.push_str(v);
            }
        }
        out
    }
}

/// `parse_line_protocol` 返回值
#[derive(Debug, Clone)]
pub struct LineParseResult {
    /// 解析出的决策列表（通常长度 1；EXEC 多块时长度 > 1 但都是 Executing）
    /// 当前协议下：
    /// - ASK / RESULT / PLAN：必为单元素
    /// - EXEC：每个 EXEC 块对应一个 Executing 决策（多块合并为单个 Executing 也可，见调用约定）
    pub decisions: Vec<AIDecision>,
    /// 结构化的 EXEC 工具调用列表（与 `decisions` 中的 Executing 对应）
    /// 用于将来贯通"长参数原样传入 executor"路径；当前 `decisions` 里
    /// 仍存放降级后的单行命令字符串，便于零改动接入老 thinking 路径。
    pub tool_calls: Vec<ParsedToolCall>,
    /// 响应级变量表（解析完成后即失效，仅用于调试和测试可见性）
    pub var_table: std::collections::HashMap<String, String>,
    /// 推理块（`<think>...</think>` 内容，已剥除标签）
    pub reasoning: Option<String>,
    /// 解析三态
    pub outcome: LineParseOutcome,
}

// ============================================================================
// 主入口
// ============================================================================

/// 解析行式协议文本
/// 步骤：
/// 1. 剥离 `<think>...</think>` 块 → reasoning
/// 2. 扫描行流，找到第一个合法决策头
/// 3. 按"读到下一个决策头为止"切块
/// 5. 装配 `AIDecision` + 结构化 `ParsedToolCall`
pub fn parse_line_protocol(text: &str) -> LineParseResult {
    let mut hints: Vec<String> = Vec::new();

    // ---- Step 1: <think> 剥离 ----
    let (reasoning, body) = strip_and_capture_think(text);

    // ---- Step 2: 预处理逻辑行（合并 heredoc）----
    let lines = match preprocess_runtime_lines(&body) {
        Ok(lines) => lines,
        Err(reason) => {
            return LineParseResult {
                decisions: vec![],
                tool_calls: vec![],
                var_table: Default::default(),
                reasoning,
                outcome: LineParseOutcome::Failed { reason },
            };
        }
    };

    if let Some(reason) = find_disallowed_runtime_syntax(&lines) {
        return LineParseResult {
            decisions: vec![],
            tool_calls: vec![],
            var_table: Default::default(),
            reasoning,
            outcome: LineParseOutcome::Failed { reason },
        };
    }

    let mut variables: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    // 找出所有决策头行的下标
    let mut head_indices: Vec<(usize, DecisionHead)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(head) = match_decision_head(line, &mut hints) {
            head_indices.push((i, head));
        }
    }

    if head_indices.is_empty() {
        return LineParseResult {
            decisions: vec![],
            tool_calls: vec![],
            var_table: Default::default(),
            reasoning,
            outcome: LineParseOutcome::Failed {
                reason: "未找到任何合法决策头（ASK/EXEC/RESULT）".to_string(),
            },
        };
    }

    let has_non_exec = head_indices
        .iter()
        .any(|(_, head)| !matches!(head, DecisionHead::Exec(_)));
    if has_non_exec && head_indices.len() > 1 {
        return LineParseResult {
            decisions: vec![],
            tool_calls: vec![],
            var_table: variables.clone(),
            reasoning,
            outcome: LineParseOutcome::Failed {
                reason: "ASK/RESULT 必须单独作为一次响应，不能与 EXEC 或其它决策头混用".to_string(),
            },
        };
    }

    // 跳过首个决策头之前的内容（若有非空行，记 hint）
    let first_head_idx = head_indices[0].0;
    for line in &lines[..first_head_idx] {
        if let Some((name, value)) = parse_runtime_assignment(line, &variables) {
            variables.insert(name, value);
        } else if !line.trim().is_empty() {
            hints.push("决策头前存在前导文本，已忽略".to_string());
        }
    }

    // ---- Step 3: 按头切块 ----
    let mut decisions: Vec<AIDecision> = Vec::new();
    let mut tool_calls: Vec<ParsedToolCall> = Vec::new();

    for (idx, (head_line, head)) in head_indices.iter().enumerate() {
        let block_start = head_line + 1;
        let block_end = head_indices
            .get(idx + 1)
            .map(|(i, _)| *i)
            .unwrap_or(lines.len());
        let block_lines: Vec<&str> = lines[block_start..block_end]
            .iter()
            .map(|s| s.as_str())
            .collect();

        match head {
            DecisionHead::Ask => {
                let payload = join_payload(&block_lines);
                decisions.push(AIDecision::Asking {
                    reasoning: reasoning.clone(),
                    prompt: Some(payload),
                });
            }
            DecisionHead::Result => {
                let payload = join_payload(&block_lines);
                decisions.push(AIDecision::Result {
                    reasoning: reasoning.clone(),
                    result: payload,
                });
            }
            DecisionHead::Exec(subject) => {
                if subject.trim().is_empty() {
                    return LineParseResult {
                        decisions: vec![],
                        tool_calls: vec![],
                        var_table: variables.clone(),
                        reasoning,
                        outcome: LineParseOutcome::Failed {
                            reason: "EXEC 决策缺少工具名".to_string(),
                        },
                    };
                }
                for line in &block_lines {
                    if let Some((name, value)) = parse_runtime_assignment(line, &variables) {
                        variables.insert(name, value);
                    }
                }
                let non_assignment_block_line = block_lines.iter().find(|line| {
                    !line.trim().is_empty() && parse_runtime_assignment(line, &variables).is_none()
                });
                if let Some(line) = non_assignment_block_line {
                    return LineParseResult {
                        decisions: vec![],
                        tool_calls: vec![],
                        var_table: variables.clone(),
                        reasoning,
                        outcome: LineParseOutcome::Failed {
                            reason: format!(
                                "EXEC 参数必须写在同一行 `EXEC Tool --key value`；长文本请先声明变量再引用。游离行：`{}`",
                                line.trim()
                            ),
                        },
                    };
                }
                match parse_exec_subject(subject, &variables) {
                    Ok(parsed) => {
                        tool_calls.push(parsed);
                    }
                    Err(reason) => {
                        return LineParseResult {
                            decisions: vec![],
                            tool_calls: vec![],
                            var_table: variables.clone(),
                            reasoning,
                            outcome: LineParseOutcome::Failed { reason },
                        };
                    }
                }
            }
        }
    }

    // 多个 EXEC 块合并为单个 Executing 决策（沿用现有 PENDING_TOOLS 语义）
    if !tool_calls.is_empty() {
        let cmds: Vec<String> = tool_calls.iter().map(|t| t.to_legacy_command()).collect();
        decisions.push(AIDecision::Executing {
            reasoning: reasoning.clone(),
            tools: cmds,
        });
    }

    if decisions.is_empty() {
        return LineParseResult {
            decisions: vec![],
            tool_calls: vec![],
            var_table: variables.clone(),
            reasoning,
            outcome: LineParseOutcome::Failed {
                reason: "解析后无可用决策".to_string(),
            },
        };
    }

    let outcome = if hints.is_empty() {
        LineParseOutcome::Strict
    } else {
        LineParseOutcome::Recovered { hints }
    };

    LineParseResult {
        decisions,
        tool_calls,
        var_table: variables,
        reasoning,
        outcome,
    }
}

// ============================================================================
// 内部：决策头匹配
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum DecisionHead {
    Ask,
    Exec(String), // tool_name
    Result,
}

/// 决策头匹配（宽松模式）
/// 容许：
/// - 前后空格 / 制表符
/// - 尾部冒号（中英文）：`EXEC:` / `ASK：`
/// - 大小写不敏感
/// 触发任何容错时往 `hints` 写一条说明。
fn match_decision_head(line: &str, hints: &mut Vec<String>) -> Option<DecisionHead> {
    let raw = line;
    let trimmed_full = raw.trim();
    if trimmed_full.is_empty() {
        return None;
    }

    // 移除成对的 markdown bold 标记 `**` / 单星 `*`（仅当用作装饰时）
    let dedecorated_owned: String;
    let dedecorated: &str = if trimmed_full.contains('*') || trimmed_full.contains('`') {
        dedecorated_owned = trimmed_full
            .replace("**", "")
            .replace('*', "")
            .replace('`', "");
        dedecorated_owned.trim()
    } else {
        trimmed_full
    };

    let stripped =
        dedecorated.trim_start_matches(|c: char| matches!(c, '#' | '>' | '-' | ' ' | '\t'));

    // 尾部冒号 / 装饰
    let stripped = stripped.trim_end_matches(|c: char| matches!(c, ':' | '：' | ' ' | '\t'));

    if stripped.is_empty() {
        return None;
    }

    // 拆 head 关键字 + 可选 subject
    let mut iter = stripped.splitn(2, char::is_whitespace);
    let kw = iter.next().unwrap_or("");
    let subject = iter.next().unwrap_or("").trim();

    let kw_upper = kw.to_ascii_uppercase();
    let head = match kw_upper.as_str() {
        "ASK" => DecisionHead::Ask,
        "EXEC" => DecisionHead::Exec(subject.to_string()),
        "RESULT" => DecisionHead::Result,
        _ => return None,
    };

    // 装饰提示
    if raw != trimmed_full {
        hints.push(format!("决策头存在前后空白：{:?}", raw));
    } else if trimmed_full != stripped {
        hints.push(format!("决策头存在装饰符：{:?}", trimmed_full));
    }
    if kw != kw_upper {
        hints.push(format!("决策头大小写非标准：{:?}", kw));
    }
    // ASK / RESULT 不应有 subject
    if matches!(head, DecisionHead::Ask | DecisionHead::Result) && !subject.is_empty() {
        hints.push(format!(
            "{:?} 决策头不应带 subject，已忽略：{:?}",
            kw_upper, subject
        ));
    }

    Some(head)
}

// ============================================================================
// 内部：运行时单行 EXEC / 变量 / heredoc
// ============================================================================

fn preprocess_runtime_lines(text: &str) -> Result<Vec<String>, String> {
    syntax_lex::preprocess_lines(text, false)
        .map(|lines| lines.into_iter().map(|l| l.content).collect())
        .map_err(|err| format!("第 {} 行 {}", err.lineno, err.message))
}

fn parse_exec_subject(
    subject: &str,
    variables: &std::collections::HashMap<String, String>,
) -> Result<ParsedToolCall, String> {
    let chunks = tokenize_chunks(subject);
    if chunks.is_empty() {
        return Err("EXEC 决策缺少工具名".to_string());
    }
    let name = chunks[0].clone();
    if name.starts_with('-') || name.starts_with('$') {
        return Err(format!("EXEC 工具名非法：`{}`", name));
    }

    let mut params = Vec::new();
    let mut i = 1;
    while i < chunks.len() {
        let chunk = &chunks[i];
        if !chunk.starts_with("--") {
            return Err(format!(
                "EXEC 参数必须使用 `--name value` 形式，得到：`{}`",
                chunk
            ));
        }
        let key = chunk.trim_start_matches("--").trim();
        if key.is_empty() {
            return Err("EXEC 参数名为空".to_string());
        }

        i += 1;
        let mut value_parts = Vec::new();
        while i < chunks.len() && !chunks[i].starts_with("--") {
            value_parts.push(parse_runtime_param_value(key, &chunks[i], variables)?);
            i += 1;
        }
        params.push((key.to_string(), value_parts.join(" ")));
    }

    Ok(ParsedToolCall { name, params })
}

fn parse_runtime_assignment(
    line: &str,
    variables: &std::collections::HashMap<String, String>,
) -> Option<(String, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('$') {
        return None;
    }
    let eq = find_assignment_eq(trimmed)?;
    let lhs = trimmed[..eq].trim();
    let rhs = trimmed[eq + 1..].trim();
    let name = lhs.strip_prefix('$')?;
    if name.is_empty() || !is_valid_runtime_var_name(name) {
        return None;
    }
    parse_runtime_value_with_path_mode(rhs, variables, is_path_like_name(name))
        .ok()
        .map(|value| (name.to_string(), value))
}

fn parse_runtime_param_value(
    key: &str,
    raw: &str,
    variables: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    parse_runtime_value_with_path_mode(raw, variables, is_path_like_name(key))
}

fn parse_runtime_value_with_path_mode(
    raw: &str,
    variables: &std::collections::HashMap<String, String>,
    preserve_path_backslashes: bool,
) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    if s.starts_with("$(") {
        return Err("运行时 EXEC 协议不支持内联 pure 表达式 `$()`；请改用 RunWorkflow".to_string());
    }
    if looks_like_step_ref(s) {
        return Err(
            "运行时 EXEC 协议不支持 `N.pin` 步骤引用；需要多步依赖请改用 RunWorkflow".to_string(),
        );
    }
    if let Some(name) = s.strip_prefix('$') {
        if !is_valid_runtime_var_name(name) {
            return Err("变量引用 `$` 后缺少变量名".to_string());
        }
        return variables
            .get(name)
            .cloned()
            .ok_or_else(|| format!("变量未定义：`${}`", name));
    }
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        return Ok(unescape_string_literal_with_mode(
            &s[1..s.len() - 1],
            preserve_path_backslashes,
        ));
    }
    Ok(s.to_string())
}

fn is_path_like_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "path"
        || name == "paths"
        || name.ends_with("_path")
        || name.ends_with("_paths")
        || name.ends_with("path")
        || name.ends_with("paths")
        || name.contains("directory")
        || name.contains("folder")
}

fn is_valid_runtime_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn tokenize_chunks(line: &str) -> Vec<String> {
    syntax_lex::tokenize_chunks(line)
}

fn find_assignment_eq(line: &str) -> Option<usize> {
    let mut in_str: Option<char> = None;
    let mut prev_escape = false;
    for (idx, c) in line.char_indices() {
        if prev_escape {
            prev_escape = false;
            continue;
        }
        if c == '\\' && in_str.is_some() {
            prev_escape = true;
            continue;
        }
        match in_str {
            Some(q) if c == q => in_str = None,
            None if c == '"' || c == '\'' => in_str = Some(c),
            None if c == '=' => return Some(idx),
            _ => {}
        }
    }
    None
}

fn unescape_string_literal_with_mode(inner: &str, preserve_path_backslashes: bool) -> String {
    if preserve_path_backslashes {
        return unescape_path_literal(inner);
    }

    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') if !preserve_path_backslashes => out.push('\n'),
            Some('r') if !preserve_path_backslashes => out.push('\r'),
            Some('t') if !preserve_path_backslashes => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn unescape_path_literal(inner: &str) -> String {
    let mut out = String::new();
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let Some(&escaped) = chars.peek() else {
            out.push('\\');
            continue;
        };

        match escaped {
            '"' | '\'' | ' ' => {
                chars.next();
                out.push(escaped);
            }
            '\\' => {
                chars.next();
                if out.is_empty() {
                    out.push('\\');
                    out.push('\\');
                } else {
                    out.push('\\');
                }
            }
            _ => out.push('\\'),
        }
    }

    out
}

fn find_disallowed_runtime_syntax(lines: &[String]) -> Option<String> {
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('$') {
            continue;
        }
        if has_step_prefix(trimmed) {
            return Some(
                "运行时 EXEC 协议不支持 `N:` 步骤号；复杂编排请用 RunWorkflow".to_string(),
            );
        }
        let first = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(|c| c == ':' || c == '：');
        if matches!(
            first,
            "IF" | "ELIF" | "ELSE" | "FOR" | "BREAK" | "END" | "INPUT" | "RETURN"
        ) {
            return Some(format!(
                "运行时 EXEC 协议不支持 `{}` 控制流/工作流语法；复杂编排请用 RunWorkflow",
                first
            ));
        }
    }
    None
}

fn has_step_prefix(line: &str) -> bool {
    let Some((before, _)) = line.split_once(':') else {
        return false;
    };
    !before.is_empty() && before.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn looks_like_step_ref(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if !first.is_ascii_digit() {
        return false;
    }
    let Some(last_dot) = s.rfind('.') else {
        return false;
    };
    let step = &s[..last_dot];
    let pin = &s[last_dot + 1..];
    !step.is_empty()
        && !pin.is_empty()
        && step.chars().all(|c| c.is_ascii_digit() || c == '.')
        && pin
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
}

// ============================================================================
// 内部：辅助
// ============================================================================

/// 将载荷行原样拼接（保留换行），并 trim 首尾空白行
fn join_payload(lines: &[&str]) -> String {
    let mut start = 0;
    let mut end = lines.len();
    while start < end && lines[start].trim().is_empty() {
        start += 1;
    }
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    lines[start..end].join("\n")
}

/// 剥离 `<think>...</think>`，返回 (reasoning, 剩余文本)
/// 与 `decision::strip_think_tags` 行为基本一致，但额外捕获 think 内容供 reasoning 字段用。
fn strip_and_capture_think(text: &str) -> (Option<String>, String) {
    let trimmed = text.trim();
    if let Some(open) = trimmed.find("<think>") {
        if let Some(close) = trimmed[open..].find("</think>") {
            let inner_start = open + "<think>".len();
            let inner_end = open + close;
            let reasoning = trimmed[inner_start..inner_end].trim().to_string();
            let mut body = String::new();
            body.push_str(&trimmed[..open]);
            body.push_str(&trimmed[inner_end + "</think>".len()..]);
            return (
                if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                },
                body.trim().to_string(),
            );
        } else {
            // 截断的 think，丢弃整段
            return (None, String::new());
        }
    }
    (None, trimmed.to_string())
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn must_strict(r: &LineParseResult) {
        assert_eq!(
            r.outcome,
            LineParseOutcome::Strict,
            "expected Strict, got {:?}",
            r.outcome
        );
    }

    fn must_recovered(r: &LineParseResult) {
        assert!(
            matches!(r.outcome, LineParseOutcome::Recovered { .. }),
            "expected Recovered, got {:?}",
            r.outcome
        );
    }

    fn must_failed(r: &LineParseResult) {
        assert!(
            matches!(r.outcome, LineParseOutcome::Failed { .. }),
            "expected Failed, got {:?}",
            r.outcome
        );
    }

    // ---- ASK ----

    #[test]
    fn ask_simple() {
        let r = parse_line_protocol("ASK\n你想用什么颜色？");
        must_strict(&r);
        match &r.decisions[0] {
            AIDecision::Asking { prompt, .. } => {
                assert_eq!(prompt.as_deref(), Some("你想用什么颜色？"))
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn ask_multiline_with_widget() {
        let text = "ASK\n请选择背景：\n[select:single | label=\"背景\" | options=\"白色,黑色\"]";
        let r = parse_line_protocol(text);
        must_strict(&r);
        match &r.decisions[0] {
            AIDecision::Asking { prompt, .. } => {
                let p = prompt.as_deref().unwrap();
                assert!(p.contains("[select:"));
                assert!(p.contains("背景"));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn ask_with_quotes_and_newlines() {
        // 用户痛点：JSON 模式下这种"嵌套引号 + 多行"会破 JSON
        let text = "ASK\n用户说\"你好\"是什么意思？\n请详细描述场景。";
        let r = parse_line_protocol(text);
        must_strict(&r);
        match &r.decisions[0] {
            AIDecision::Asking { prompt, .. } => {
                let p = prompt.as_deref().unwrap();
                assert!(p.contains("\"你好\""));
                assert!(p.contains("\n"));
            }
            other => panic!("{:?}", other),
        }
    }

    // ---- RESULT ----

    #[test]
    fn result_long_markdown() {
        let text = "RESULT\n脚本已成功注入！共 4 个积木：\n1. 绿旗触发\n2. 说\"你好\" 2 秒\n3. 移动 50 步\n4. 说\"我走到这里啦\" 2 秒";
        let r = parse_line_protocol(text);
        must_strict(&r);
        match &r.decisions[0] {
            AIDecision::Result { result, .. } => {
                assert!(result.contains("绿旗触发"));
                assert!(result.contains("\"你好\""));
                assert_eq!(result.lines().count(), 5);
            }
            other => panic!("{:?}", other),
        }
    }

    // ---- EXEC ----

    #[test]
    fn exec_single_short_args() {
        let text = "EXEC BrowserOpen --url https://example.com";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "BrowserOpen");
        assert_eq!(
            r.tool_calls[0].params,
            vec![("url".to_string(), "https://example.com".to_string())]
        );
        match &r.decisions[0] {
            AIDecision::Executing { tools, .. } => {
                assert_eq!(tools.len(), 1);
                assert!(tools[0].starts_with("BrowserOpen"));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn exec_no_params() {
        let r = parse_line_protocol("EXEC GetSnapshot");
        must_strict(&r);
        assert_eq!(r.tool_calls[0].name, "GetSnapshot");
        assert!(r.tool_calls[0].params.is_empty());
    }

    #[test]
    fn exec_value_consumes_tokens_until_next_param() {
        let r = parse_line_protocol(
            "EXEC SearchDocs --query \"齐云山茶油 产品 卖点\" --label qiyun product --limit 5",
        );
        must_strict(&r);
        let tc = &r.tool_calls[0];
        assert_eq!(tc.name, "SearchDocs");
        assert_eq!(
            tc.params,
            vec![
                ("query".to_string(), "齐云山茶油 产品 卖点".to_string()),
                ("label".to_string(), "qiyun product".to_string()),
                ("limit".to_string(), "5".to_string()),
            ]
        );
    }

    #[test]
    fn exec_multiline_param_value() {
        // 核心场景：长字符串参数原样保留，零转义
        let text = "$script = \"\nwhen flag clicked\nsay \"hello world\" for 2 seconds\nmove 50 steps\n\"\nEXEC ScratchInject --script $script --target sprite1";
        let r = parse_line_protocol(text);
        must_strict(&r);
        let tc = &r.tool_calls[0];
        assert_eq!(tc.name, "ScratchInject");
        assert_eq!(tc.params.len(), 2);
        assert_eq!(tc.params[0].0, "script");
        let script = &tc.params[0].1;
        assert!(script.contains("when flag clicked"));
        assert!(script.contains("\"hello world\""));
        assert!(script.contains("move 50 steps"));
        assert!(script.lines().count() >= 3);
        assert_eq!(tc.params[1], ("target".to_string(), "sprite1".to_string()));
    }

    #[test]
    fn exec_multi_blocks() {
        let text = "EXEC GetSnapshot\nEXEC GetScriptDocs --page_id main";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].name, "GetSnapshot");
        assert_eq!(r.tool_calls[1].name, "GetScriptDocs");
        assert_eq!(
            r.tool_calls[1].params,
            vec![("page_id".to_string(), "main".to_string())]
        );
        match &r.decisions[0] {
            AIDecision::Executing { tools, .. } => assert_eq!(tools.len(), 2),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn exec_inline_value_then_continuation() {
        let text = "EXEC FooTool\n--note short note\nplus continuation line";
        let r = parse_line_protocol(text);
        must_failed(&r);
    }

    // ---- think 块 ----

    #[test]
    fn think_block_captured() {
        let text = "<think>\n先取快照\n</think>\nEXEC GetSnapshot";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.reasoning.as_deref(), Some("先取快照"));
        assert_eq!(r.tool_calls[0].name, "GetSnapshot");
    }

    #[test]
    fn think_truncated_dropped() {
        let text = "<think>\n截断了没结束";
        let r = parse_line_protocol(text);
        // 截断的 think 段被丢弃，body 为空 → Failed
        must_failed(&r);
    }

    // ---- 容错 ----

    #[test]
    fn recovered_decoration_marker() {
        let text = "**EXEC** GetSnapshot";
        let r = parse_line_protocol(text);
        must_recovered(&r);
        assert_eq!(r.tool_calls[0].name, "GetSnapshot");
    }

    #[test]
    fn recovered_lowercase() {
        let text = "exec GetSnapshot";
        let r = parse_line_protocol(text);
        must_recovered(&r);
        assert_eq!(r.tool_calls[0].name, "GetSnapshot");
    }

    #[test]
    fn recovered_chinese_colon() {
        let text = "ASK：\n你需要什么帮助？";
        let r = parse_line_protocol(text);
        must_recovered(&r);
        match &r.decisions[0] {
            AIDecision::Asking { prompt, .. } => {
                assert!(prompt.as_deref().unwrap().contains("帮助"));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn recovered_leading_garbage_before_head() {
        let text = "好的，我来思考一下\nEXEC GetSnapshot";
        let r = parse_line_protocol(text);
        must_recovered(&r);
        assert_eq!(r.tool_calls[0].name, "GetSnapshot");
    }

    // ---- 失败 ----

    #[test]
    fn failed_no_decision_head() {
        let r = parse_line_protocol("我觉得你应该用 ffmpeg 转换。");
        must_failed(&r);
    }

    #[test]
    fn failed_exec_no_tool_name() {
        let r = parse_line_protocol("EXEC\n--script abc");
        must_failed(&r);
    }

    #[test]
    fn failed_empty_input() {
        let r = parse_line_protocol("");
        must_failed(&r);
    }

    // ---- legacy command 降级 ----

    #[test]
    fn legacy_command_quotes_when_needed() {
        let tc = ParsedToolCall {
            name: "BrowserOpen".to_string(),
            params: vec![
                ("url".to_string(), "https://example.com".to_string()),
                ("title".to_string(), "Hello World".to_string()),
            ],
        };
        let cmd = tc.to_legacy_command();
        assert_eq!(
            cmd,
            "BrowserOpen --url https://example.com --title \"Hello World\""
        );
    }

    #[test]
    fn legacy_command_escapes_newlines() {
        let tc = ParsedToolCall {
            name: "Foo".to_string(),
            params: vec![("script".to_string(), "line1\nline2".to_string())],
        };
        let cmd = tc.to_legacy_command();
        assert!(cmd.contains("\\n"));
        assert!(cmd.starts_with("Foo --script \""));
    }

    #[test]
    fn combined_think_and_multi_exec() {
        let text = "<think>\n先取快照看当前状态\n</think>\nEXEC GetSnapshot\nEXEC GetScriptDocs";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.reasoning.as_deref(), Some("先取快照看当前状态"));
        assert_eq!(r.tool_calls.len(), 2);
    }

    #[test]
    fn ask_payload_preserves_blank_lines_inside() {
        let text = "ASK\n第一段。\n\n第二段（中间空行）。";
        let r = parse_line_protocol(text);
        must_strict(&r);
        match &r.decisions[0] {
            AIDecision::Asking { prompt, .. } => {
                let p = prompt.as_deref().unwrap();
                // 内部空行应保留
                assert!(p.contains("第一段"));
                assert!(p.contains("第二段"));
                assert_eq!(p.lines().count(), 3);
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn exec_param_then_next_exec_terminates_value() {
        // 关键：第二个 EXEC 应该终结上一个 EXEC 的最后 param value，不能被吞进去
        let text = "$script = \"\nline a\nline b\n\"\nEXEC FooTool --script $script\nEXEC BarTool";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.tool_calls.len(), 2);
        let foo_script = &r.tool_calls[0].params[0].1;
        assert!(foo_script.contains("line a"));
        assert!(foo_script.contains("line b"));
        assert!(!foo_script.contains("BarTool"));
    }

    #[test]
    fn exec_single_line_args() {
        let text = "EXEC BrowserOpen --url https://example.com --title \"Hello World\"";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.tool_calls.len(), 1);
        let tc = &r.tool_calls[0];
        assert_eq!(tc.name, "BrowserOpen");
        assert_eq!(
            tc.params[0],
            ("url".to_string(), "https://example.com".to_string())
        );
        assert_eq!(
            tc.params[1],
            ("title".to_string(), "Hello World".to_string())
        );
    }

    #[test]
    fn exec_variable_assignment_before_exec() {
        let text = "$url = \"https://example.com\"\nEXEC BrowserOpen --url $url";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(
            r.tool_calls[0].params[0],
            ("url".to_string(), "https://example.com".to_string())
        );
    }

    #[test]
    fn exec_path_param_preserves_windows_backslashes() {
        let text = r#"EXEC ReadTextFile --path "D:\new\raw\track.txt""#;
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(
            r.tool_calls[0].params[0],
            ("path".to_string(), r"D:\new\raw\track.txt".to_string())
        );
    }

    #[test]
    fn exec_path_variable_preserves_windows_backslashes() {
        let text = r#"$path = "D:\new\raw\track.txt"
EXEC ReadTextFile --path $path"#;
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(
            r.tool_calls[0].params[0],
            ("path".to_string(), r"D:\new\raw\track.txt".to_string())
        );
    }

    #[test]
    fn exec_path_param_preserves_unc_prefix() {
        let text = r#"EXEC ReadTextFile --path "\\server\share\track.txt""#;
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(
            r.tool_calls[0].params[0],
            ("path".to_string(), r"\\server\share\track.txt".to_string())
        );
    }

    #[test]
    fn exec_heredoc_variable() {
        let text = "$script = \"\nwhen flag clicked\nsay \"hello\" for 2 seconds\n\"\nEXEC ScratchInject --script $script --target sprite1";
        let r = parse_line_protocol(text);
        must_strict(&r);
        assert_eq!(r.tool_calls.len(), 1);
        let script = &r.tool_calls[0].params[0].1;
        assert!(script.contains("when flag clicked"));
        assert!(script.contains("say \"hello\""));
        assert_eq!(
            r.tool_calls[0].params[1],
            ("target".to_string(), "sprite1".to_string())
        );
    }

    #[test]
    fn runtime_rejects_inline_pure() {
        let text = "EXEC Foo --value $(Add --A 1 --B 2)";
        let r = parse_line_protocol(text);
        must_failed(&r);
        match r.outcome {
            LineParseOutcome::Failed { reason } => assert!(reason.contains("不支持内联 pure")),
            other => panic!("{:?}", other),
        }
    }

    // ---- fuzzing-style：随机噪声不应 panic ----

    #[test]
    fn runtime_rejects_control_flow() {
        let r = parse_line_protocol("IF $ok\nEXEC Foo\nEND");
        must_failed(&r);
    }

    #[test]
    fn runtime_rejects_step_ref() {
        let r = parse_line_protocol("EXEC Foo --value 1.title");
        must_failed(&r);
    }

    #[test]
    fn ask_result_cannot_mix_with_exec() {
        let r = parse_line_protocol("ASK\nNeed a choice\nEXEC Foo");
        must_failed(&r);
    }

    #[test]
    fn runtime_rejects_invalid_variable_name() {
        let r = parse_line_protocol("$1bad = \"x\"\nEXEC Foo --value $1bad");
        must_failed(&r);
    }

    #[test]
    fn var_table_keeps_assignments() {
        let r = parse_line_protocol("$url = \"https://example.com\"\nEXEC BrowserOpen --url $url");
        must_strict(&r);
        assert_eq!(
            r.var_table.get("url").map(String::as_str),
            Some("https://example.com")
        );
    }

    #[test]
    fn fuzz_no_panic_on_garbage() {
        let samples = [
            "",
            "\n\n\n",
            "ASK",
            "EXEC ",
            "RESULT\n",
            "<think></think>",
            "<think>only think</think>",
            "EXEC Foo\n--",
            "EXEC Foo\n----badparam value",
            "ASK\n```json\n{\"x\":1}\n```",
            "ResUlT\n大小写混用",
            "  > # EXEC   GetSnapshot   :  ",
        ];
        for s in samples {
            let _ = parse_line_protocol(s); // 只断言不 panic
        }
    }
}
