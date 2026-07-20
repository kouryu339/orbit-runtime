//! LLM 响应解析器——两个原子无状态纯函数
//! 职责：
//! 1. [`is_tool_call`]：判断 LLM 文本是否包含工具调用（thinking 结束后决定 → ask 还是 executing）
//! 2. [`parse_tool_calls`]：将 LLM 文本解析为工具调用数组（进入 executing 时调用）
//! **不做状态转换、不执行工具、不改对话历史。**

use crate::decision_line;
pub use crate::decision_line::ParsedToolCall;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallKind {
    None,
    ToolOnly,
    /// A response that contains protocol lines and user-visible text outside them.
    Mixed {
        assistant_content: String,
    },
}

// ============================================================================
// 函数 1：判断是不是工具调用
// ============================================================================

/// 判断 LLM 响应文本是否包含工具调用（EXEC 决策头）
/// - `true`  → 状态机应进入 executing
/// - `false` → 非工具调用（普通对话 / ASK / RESULT / 空 / 格式错误等）
pub fn is_tool_call(text: &str) -> bool {
    !matches!(classify_tool_call(text), ToolCallKind::None)
}

pub fn classify_tool_call(text: &str) -> ToolCallKind {
    let normalized = normalize_protocol_text(text);
    if !contains_exec_keyword(&normalized) {
        return ToolCallKind::None;
    }

    match visible_assistant_content(&normalized) {
        Some(assistant_content) => ToolCallKind::Mixed { assistant_content },
        None => ToolCallKind::ToolOnly,
    }
}

/// Normalize minor model quoting mistakes before validation and ledger writes.
/// Some models wrap the whole response, or a standalone protocol line, in
/// single/double quotes. The runtime treats that as accidental presentation
/// quoting and stores/parses the unquoted line.
pub fn normalize_response_for_ledger(text: &str) -> String {
    let whole_unquoted = strip_matching_outer_quotes(text);
    if !std::ptr::eq(whole_unquoted, text) {
        return whole_unquoted.to_string();
    }
    text.lines()
        .map(|line| {
            let unquoted = strip_matching_outer_quotes(line);
            let trimmed = unquoted.trim();
            if is_exec_protocol_line(trimmed) || is_runtime_assignment_start(trimmed) {
                unquoted
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Validate response-level protocol structure before any ledger write.
pub fn validate_response_shape(text: &str) -> Result<(), String> {
    let stripped = normalize_protocol_text(text);
    let mut in_multiline_assignment = false;
    let mut in_code_fence = false;
    let mut has_assignment = false;
    let mut has_exec = false;

    for line in stripped.lines() {
        let trimmed = line.trim();
        if in_multiline_assignment {
            if crate::decision::contains_widget_tag(trimmed) {
                return Err(
                    "widget tags are not allowed inside a response-level variable".to_string(),
                );
            }
            if trimmed == "\"" || trimmed == "'" {
                in_multiline_assignment = false;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence {
            continue;
        }
        if is_runtime_assignment_start(trimmed) {
            has_assignment = true;
            in_multiline_assignment = opens_multiline_assignment(trimmed);
            continue;
        }
        if trimmed.starts_with('$') && trimmed.contains('=') {
            return Err(
                "响应级变量声明格式无效，变量声明必须单独成行并使用 `$name = value`".to_string(),
            );
        }
        if is_exec_protocol_line(trimmed) {
            has_exec = true;
            continue;
        }
        if crate::decision::contains_widget_tag(trimmed) && !is_standalone_widget_line(trimmed) {
            return Err("widget tags must occupy standalone lines".to_string());
        }
    }

    if in_multiline_assignment {
        return Err("响应级变量声明存在未闭合的多行值".to_string());
    }
    if has_assignment && !has_exec {
        return Err("响应级变量声明只能与至少一个 EXEC 工具调用同时出现".to_string());
    }
    if has_exec {
        parse_tool_calls(text).map_err(|reason| format!("invalid EXEC protocol: {reason}"))?;
        if let Some(tail) = trailing_visible_content_after_last_exec(text) {
            if crate::response_guard::contains_strong_action_claim(&tail) {
                return Err(
                    "visible text after the final EXEC promises another unsupported action"
                        .to_string(),
                );
            }
            if crate::response_guard::contains_completion_claim(&tail) {
                return Err(
                    "visible text after the final EXEC must not claim that execution already completed".to_string(),
                );
            }
        }
    } else if crate::response_guard::contains_strong_action_claim(&stripped) {
        return Err("an action promise requires a corresponding standalone EXEC line".to_string());
    }
    Ok(())
}

// ============================================================================
// 函数 2：解析工具调用数组
// ============================================================================

/// 解析 LLM 响应文本为工具调用数组
/// 返回 `Ok(Vec<ParsedToolCall>)` —— 每个元素包含 `name` 和 `params: Vec<(key, value)>`。
/// 多个 EXEC 调用会产生多个元素，由调用方并发执行。
/// 返回 `Err(String)` —— 解析失败时的错误原因。
pub fn parse_tool_calls(text: &str) -> Result<Vec<ParsedToolCall>, String> {
    if normalize_protocol_text(text)
        .trim()
        .eq_ignore_ascii_case("EXEC")
    {
        return Err("EXEC 决策缺少工具名".to_string());
    }
    let normalized = protocol_content(text);
    let r = decision_line::parse_line_protocol(&normalized);
    if !r.outcome.is_ok() {
        let reason = match r.outcome {
            decision_line::LineParseOutcome::Failed { reason } => reason,
            _ => "解析失败".to_string(),
        };
        return Err(reason);
    }
    if r.tool_calls.is_empty() {
        return Err("未找到 EXEC 工具调用".to_string());
    }
    Ok(r.tool_calls)
}

pub fn display_exec_commands(text: &str) -> Vec<String> {
    protocol_content(text)
        .lines()
        .map(str::trim)
        .filter(|line| is_exec_protocol_line(line))
        .map(str::to_string)
        .collect()
}

/// Build frontend-visible text while preserving the location of each tool call.
/// Response-level variables are runtime plumbing and are omitted. Each EXEC
/// line becomes a status widget bound to its preallocated runtime call id.
pub fn frontend_projection(text: &str, call_ids: &[String]) -> String {
    let stripped = normalize_protocol_text(text);
    let mut visible = Vec::new();
    let mut in_multiline_assignment = false;
    let mut in_code_fence = false;
    let mut call_index = 0usize;

    for line in stripped.lines() {
        let trimmed = line.trim();
        if in_multiline_assignment {
            if trimmed == "\"" || trimmed == "'" {
                in_multiline_assignment = false;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            visible.push(line.to_string());
            continue;
        }
        if in_code_fence {
            visible.push(line.to_string());
            continue;
        }
        if is_runtime_assignment_start(trimmed) {
            in_multiline_assignment = opens_multiline_assignment(trimmed);
            continue;
        }
        if is_exec_protocol_line(trimmed) {
            if let Some(call_id) = call_ids.get(call_index) {
                visible.push(format!("[tool:status | call_id=\"{}\"]", call_id));
            }
            call_index += 1;
            continue;
        }
        visible.push(line.to_string());
    }

    visible.join("\n").trim().to_string()
}

pub fn visible_assistant_content(text: &str) -> Option<String> {
    let stripped = normalize_protocol_text(text);
    let content = extract_protocol_and_visible_lines(&stripped)
        .1
        .join("\n")
        .trim()
        .to_string();
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

pub fn trailing_visible_content_after_last_exec(text: &str) -> Option<String> {
    let stripped = normalize_protocol_text(text);
    let lines = stripped.lines().collect::<Vec<_>>();
    let last_exec = lines
        .iter()
        .rposition(|line| is_exec_protocol_line(line.trim()))?;
    let tail = lines[last_exec + 1..].join("\n");
    visible_assistant_content(&tail)
}

fn contains_exec_keyword(text: &str) -> bool {
    let mut in_code_fence = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if !in_code_fence && is_exec_protocol_line(trimmed) {
            return true;
        }
    }
    false
}

fn protocol_content(text: &str) -> String {
    let stripped = normalize_protocol_text(text);
    extract_protocol_and_visible_lines(&stripped).0.join("\n")
}

fn extract_protocol_and_visible_lines(text: &str) -> (Vec<String>, Vec<String>) {
    let mut protocol = Vec::new();
    let mut visible = Vec::new();
    let mut in_multiline_assignment = false;
    let mut in_code_fence = false;
    let mut saw_exec = false;

    for line in text.lines() {
        let unquoted_line = strip_matching_outer_quotes(line);
        let trimmed = unquoted_line.trim();
        if in_multiline_assignment {
            protocol.push(line.to_string());
            if trimmed == "\"" || trimmed == "'" {
                in_multiline_assignment = false;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            visible.push(line.to_string());
        } else if in_code_fence {
            visible.push(line.to_string());
        } else if is_runtime_assignment_start(trimmed) {
            protocol.push(unquoted_line.to_string());
            in_multiline_assignment = opens_multiline_assignment(trimmed);
        } else if is_exec_protocol_line(trimmed) {
            protocol.push(trimmed.to_string());
            saw_exec = true;
        } else if saw_exec && trimmed.starts_with("--") {
            // Keep malformed legacy continuation lines in the protocol slice
            // so the strict parser rejects them rather than executing a
            // truncated call and displaying the discarded arguments as text.
            protocol.push(line.to_string());
        } else {
            visible.push(line.to_string());
        }
    }

    (protocol, visible)
}

fn opens_multiline_assignment(line: &str) -> bool {
    line.ends_with("= \"") || line.ends_with("= '")
}

fn is_exec_protocol_line(line: &str) -> bool {
    let Some(prefix) = line.get(..4) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case("EXEC") {
        return false;
    }
    line.get(4..)
        .is_some_and(|rest| rest.starts_with(char::is_whitespace) && !rest.trim().is_empty())
}

fn strip_matching_outer_quotes(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() < 2 {
        return value;
    }
    let bytes = trimmed.as_bytes();
    let quote = bytes[0];
    if !matches!(quote, b'\'' | b'"') || bytes[bytes.len() - 1] != quote {
        return value;
    }
    &trimmed[1..trimmed.len() - 1]
}

fn is_standalone_widget_line(line: &str) -> bool {
    line.ends_with(']')
        && (line.starts_with("[input:")
            || line.starts_with("[select:")
            || line.starts_with("[confirm"))
}

fn is_runtime_assignment_start(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('$') else {
        return false;
    };
    let Some(eq_pos) = rest.find('=') else {
        return false;
    };
    let name = rest[..eq_pos].trim();
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn normalize_protocol_text(text: &str) -> String {
    strip_protocol_fences(&strip_think_blocks(text))
}

fn strip_protocol_fences(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index];
        if !line.trim().starts_with("```") {
            output.push(line.to_string());
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < lines.len() && !lines[index].trim().starts_with("```") {
            index += 1;
        }
        let has_closing_fence = index < lines.len();
        let content_end = index;
        let content = &lines[start + 1..content_end];
        let contains_protocol = content.iter().any(|line| {
            let trimmed = line.trim();
            is_exec_protocol_line(trimmed) || is_runtime_assignment_start(trimmed)
        });

        if contains_protocol {
            output.extend(content.iter().map(|line| (*line).to_string()));
        } else {
            output.push(line.to_string());
            output.extend(content.iter().map(|line| (*line).to_string()));
            if has_closing_fence {
                output.push(lines[index].to_string());
            }
        }

        if has_closing_fence {
            index += 1;
        }
    }

    output.join("\n")
}

/// 剔除 Markdown 代码围栏（``` 或 ```text 等语言标签），仅去掉独立的围栏行，不动表体。
/// 这是对模型把 `EXEC` 包进代码块的兼容：只要 EXEC 周围裸露在本体里，
fn strip_think_blocks(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();

    while let Some(open) = rest.find("<think>") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + "<think>".len()..];
        if let Some(close) = after_open.find("</think>") {
            rest = &after_open[close + "</think>".len()..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_tool_call ----

    #[test]
    fn detect_single_exec() {
        assert!(is_tool_call("EXEC BrowserOpen --url https://example.com"));
    }

    #[test]
    fn bare_exec_is_plain_text() {
        assert!(!is_tool_call("EXEC"));
        assert!(validate_response_shape("EXEC").is_ok());
        assert_eq!(normalize_response_for_ledger("EXEC"), "EXEC");
    }

    #[test]
    fn quoted_exec_line_is_normalized_for_protocol() {
        let text = "\"EXEC BrowserOpen --url https://example.com\"";
        let normalized = normalize_response_for_ledger(text);
        assert_eq!(normalized, "EXEC BrowserOpen --url https://example.com");
        assert!(is_tool_call(&normalized));
        assert!(validate_response_shape(&normalized).is_ok());
    }

    #[test]
    fn ordinary_quoted_visible_text_is_preserved() {
        let text = "He said \"EXEC\".";
        assert_eq!(normalize_response_for_ledger(text), text);
        assert!(!is_tool_call(text));
    }

    #[test]
    fn detect_multi_exec() {
        assert!(is_tool_call(
            "EXEC GetSnapshot\nEXEC GetScriptDocs --page_id main"
        ));
    }

    #[test]
    fn route_legacy_block_exec_to_executing() {
        assert!(is_tool_call("EXEC BrowserOpen\n--url https://example.com"));
        assert!(parse_tool_calls("EXEC BrowserOpen\n--url https://example.com").is_err());
    }

    #[test]
    fn detect_with_think() {
        assert!(is_tool_call(
            "<think>\n分析一下\n</think>\nEXEC GetSnapshot"
        ));
    }

    #[test]
    fn classify_mixed_preface_and_exec() {
        let kind = classify_tool_call("ok, I will create it.\nEXEC CreateFile --path out.txt");
        assert_eq!(
            kind,
            ToolCallKind::Mixed {
                assistant_content: "ok, I will create it.".to_string()
            }
        );
    }

    #[test]
    fn preserve_text_before_and_after_standalone_exec() {
        let kind = classify_tool_call(
            "好的，我来执行。\nEXEC CreateFile --path out.txt\n文件创建后我会继续说明。",
        );
        assert_eq!(
            kind,
            ToolCallKind::Mixed {
                assistant_content: "好的，我来执行。\n文件创建后我会继续说明。".to_string()
            }
        );
    }

    #[test]
    fn parse_exec_with_surrounding_visible_text() {
        let calls = parse_tool_calls(
            "好的，我来执行。\nEXEC CreateFile --path out.txt\n执行结果返回后我再说明。",
        )
        .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "CreateFile");
        assert_eq!(calls[0].params[0], ("path".into(), "out.txt".into()));
    }

    #[test]
    fn display_standalone_exec_only() {
        let commands = display_exec_commands("好的，我来执行。\nEXEC CreateFile --path out.txt");
        assert_eq!(commands, vec!["EXEC CreateFile --path out.txt"]);
    }

    #[test]
    fn frontend_projection_hides_variables_and_replaces_exec_in_place() {
        let text = "我先读取文件。\n$path = \"a.docx\"\nEXEC ReadFile --path $path\n请选择格式：\n[select:single | label=\"格式\" | options=\"PDF,DOCX\"]";
        let projected = frontend_projection(text, &["boss:2:0".to_string()]);
        assert!(!projected.contains("$path"));
        assert!(!projected.contains("EXEC ReadFile"));
        assert!(projected.contains("[tool:status | call_id=\"boss:2:0\"]"));
        assert!(projected.contains("[select:single"));
    }

    #[test]
    fn response_shape_rejects_assignment_without_exec() {
        let error = validate_response_shape("$path = \"a.docx\"\n我稍后处理。").unwrap_err();
        assert!(error.contains("EXEC"));
    }

    #[test]
    fn response_shape_rejects_inline_widget() {
        let error =
            validate_response_shape("请选择 [select:single | options=\"A,B\"]").unwrap_err();
        assert!(error.contains("widget"));
    }

    #[test]
    fn response_shape_accepts_mixed_exec_and_standalone_widget() {
        let text = "我先读取。\n$path = \"a.docx\"\nEXEC ReadFile --path $path\n请选择：\n[select:single | options=\"A,B\"]";
        assert!(validate_response_shape(text).is_ok());
    }

    #[test]
    fn response_shape_accepts_fenced_exec_for_compatibility() {
        let text = "```text\nEXEC ReadFile --path a.docx\n```";
        assert!(validate_response_shape(text).is_ok());
        assert!(is_tool_call(text));
        let calls = parse_tool_calls(text).unwrap();
        assert_eq!(calls[0].name, "ReadFile");
        assert_eq!(calls[0].params[0], ("path".into(), "a.docx".into()));
    }

    #[test]
    fn response_shape_keeps_inline_exec_as_visible_text() {
        let text = "命令示例是 EXEC ReadFile --path a.docx";
        assert!(validate_response_shape(text).is_ok());
        assert!(!is_tool_call(text));
    }

    #[test]
    fn response_shape_rejects_widget_inside_variable() {
        let error = validate_response_shape(
            "$script = \"\n[select:single | label=\"格式\" | options=\"A,B\"]\n\"\nEXEC Run --script $script",
        )
        .unwrap_err();
        assert!(error.contains("inside"));
    }

    #[test]
    fn response_shape_rejects_unbacked_action_without_exec() {
        let error = validate_response_shape("我将为你执行转换。").unwrap_err();
        assert!(error.contains("EXEC"));
    }

    #[test]
    fn response_shape_rejects_unbacked_action_after_final_exec() {
        let error =
            validate_response_shape("我先读取。\nEXEC ReadFile --path a.docx\n然后我马上修改它。")
                .unwrap_err();
        assert!(error.contains("final EXEC"));
    }

    #[test]
    fn response_shape_rejects_completion_claim_while_executing() {
        let error = validate_response_shape(
            "我先读取。\nEXEC ReadFile --path a.docx\n我已经为你读取完成。",
        )
        .unwrap_err();
        assert!(error.contains("completed"));
    }

    #[test]
    fn response_shape_allows_prior_completion_before_new_exec() {
        assert!(
            validate_response_shape("上一项已为你读取完成。\nEXEC WriteFile --path b.docx").is_ok()
        );
    }

    #[test]
    fn frontend_projection_preserves_visible_markdown_fences() {
        let projected = frontend_projection(
            "参考示例：\n```text\nnot protocol\n```\nEXEC ReadFile --path a.docx",
            &["boss:2:0".to_string()],
        );
        assert!(projected.contains("```text\nnot protocol\n```"));
    }

    #[test]
    fn frontend_projection_replaces_fenced_exec() {
        let projected = frontend_projection(
            "```text\nEXEC ReadFile --path a.docx\n```",
            &["boss:2:0".to_string()],
        );
        assert_eq!(projected, "[tool:status | call_id=\"boss:2:0\"]");
    }

    #[test]
    fn inline_exec_wording_is_not_an_action_call() {
        assert!(!is_tool_call(
            "好的，我来执行 EXEC CreateFile --path out.txt"
        ));
        assert!(!is_tool_call("I will execute this next."));
    }

    #[test]
    fn returns_visible_tail_after_last_exec() {
        let tail = trailing_visible_content_after_last_exec(
            "先读取文件。\nEXEC ReadFile --path a.txt\n然后我马上修改它。",
        );
        assert_eq!(tail.as_deref(), Some("然后我马上修改它。"));
    }

    #[test]
    fn classify_think_plus_exec_as_tool_only() {
        let kind =
            classify_tool_call("<think>\nneed a file\n</think>\nEXEC CreateFile --path out.txt");
        assert_eq!(kind, ToolCallKind::ToolOnly);
    }

    #[test]
    fn not_tool_call_plain_text() {
        assert!(!is_tool_call("我觉得你应该用 ffmpeg 转换。"));
    }

    #[test]
    fn not_tool_call_ask() {
        assert!(!is_tool_call("ASK\n你想用什么颜色？"));
    }

    #[test]
    fn not_tool_call_result() {
        assert!(!is_tool_call("RESULT\n任务完成。"));
    }

    #[test]
    fn not_tool_call_empty() {
        assert!(!is_tool_call(""));
    }

    #[test]
    fn route_malformed_exec_as_tool_call() {
        assert!(is_tool_call(
            "EXEC SearchDocs --query \"齐云山茶油 产品 卖点\" --label qiyun product"
        ));
    }

    // ---- parse_tool_calls：正确解析 ----

    #[test]
    fn parse_single_line_args() {
        let calls =
            parse_tool_calls("EXEC BrowserOpen --url https://example.com --title \"Hello World\"")
                .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "BrowserOpen");
        assert_eq!(
            calls[0].params[0],
            ("url".into(), "https://example.com".into())
        );
        assert_eq!(calls[0].params[1], ("title".into(), "Hello World".into()));
    }

    #[test]
    fn parse_multiple_concurrent_exec() {
        let calls =
            parse_tool_calls("EXEC GetSnapshot\nEXEC GetScriptDocs --page_id main").unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "GetSnapshot");
        assert_eq!(calls[1].name, "GetScriptDocs");
        assert_eq!(calls[1].params[0], ("page_id".into(), "main".into()));
    }

    #[test]
    fn parse_variable_substitution() {
        let text = "$url = \"https://example.com\"\nEXEC BrowserOpen --url $url";
        let calls = parse_tool_calls(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].params[0],
            ("url".into(), "https://example.com".into())
        );
    }

    #[test]
    fn parse_heredoc_variable() {
        let text = "$script = \"\nwhen flag clicked\nsay hello\n\"\nEXEC ScratchInject --script $script --target sprite1";
        let calls = parse_tool_calls(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].params[0].1.contains("when flag clicked"));
        assert!(calls[0].params[0].1.contains("say hello"));
        assert_eq!(calls[0].params[1], ("target".into(), "sprite1".into()));
    }

    #[test]
    fn reject_legacy_block_style() {
        let text = "EXEC FooTool\n--script\nline a\nline b";
        let err = parse_tool_calls(text).unwrap_err();
        assert!(err.contains("EXEC") || err.contains("参数"), "got: {}", err);
    }

    #[test]
    fn parse_with_think_block() {
        let text = "<think>\n先取快照\n</think>\nEXEC GetSnapshot";
        let calls = parse_tool_calls(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "GetSnapshot");
    }

    #[test]
    fn parse_no_params() {
        let calls = parse_tool_calls("EXEC GetSnapshot").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "GetSnapshot");
        assert!(calls[0].params.is_empty());
    }

    // ---- parse_tool_calls：错误提示 ----

    #[test]
    fn err_no_exec_head() {
        let e = parse_tool_calls("我觉得你应该用 ffmpeg。").unwrap_err();
        assert!(e.contains("决策头"), "got: {}", e);
    }

    #[test]
    fn err_bare_exec_has_no_tool_call() {
        let e = parse_tool_calls("EXEC ").unwrap_err();
        assert!(e.contains("工具名"), "got: {}", e);
    }

    #[test]
    fn err_empty_input() {
        let e = parse_tool_calls("").unwrap_err();
        assert!(!e.is_empty());
    }

    #[test]
    fn err_inline_pure_rejected() {
        let e = parse_tool_calls("EXEC Foo --value $(Add --A 1 --B 2)").unwrap_err();
        assert!(e.contains("不支持"), "got: {}", e);
    }

    #[test]
    fn err_undefined_variable() {
        let e = parse_tool_calls("EXEC Foo --bar $undefined_var").unwrap_err();
        assert!(e.contains("未定义"), "got: {}", e);
    }

    #[test]
    fn err_unclosed_heredoc() {
        let text = "$code = \"\nunclosed content\nEXEC Foo --x 1";
        let e = parse_tool_calls(text).unwrap_err();
        assert!(e.contains("未闭合"), "got: {}", e);
    }

    // ---- legacy CLI 字符串生成 ----

    #[test]
    fn legacy_command_generation() {
        let calls = parse_tool_calls("EXEC Foo --bar baz --msg \"hello world\"").unwrap();
        let cmd = calls[0].to_legacy_command();
        assert!(cmd.starts_with("Foo"));
        assert!(cmd.contains("--bar"));
        assert!(cmd.contains("--msg"));
    }
}
