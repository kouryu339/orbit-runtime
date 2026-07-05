//! Shared line-oriented syntax helpers for workflow scripts and runtime EXEC.
//!
//! This module intentionally stays semantic-light: it knows how to detect
//! heredoc-style open strings and how to split a logical line into chunks while
//! respecting quotes, parentheses, and brackets. Workflow and runtime parsers
//! layer their own allowed syntax on top.

/// A preprocessed logical line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalLine {
    /// Original starting line number, 1-based.
    pub lineno: usize,
    /// Line content after heredoc merging.
    pub content: String,
}

/// Lexing/preprocess error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxLexError {
    pub lineno: usize,
    pub message: String,
}

impl SyntaxLexError {
    pub fn new(lineno: usize, message: impl Into<String>) -> Self {
        Self {
            lineno,
            message: message.into(),
        }
    }
}

/// Preprocess text into logical lines.
///
/// - Merges heredoc strings that start with an open quote and end with an
///   independent line containing only `"`.
/// - When `skip_empty_and_comments` is true, empty lines and lines beginning
///   with `#` are omitted. Workflow scripts use this mode.
/// - When false, blank lines are preserved as empty logical lines. Runtime EXEC
///   parsing uses this mode so ASK/RESULT payload layout remains intact.
pub fn preprocess_lines(
    text: &str,
    skip_empty_and_comments: bool,
) -> Result<Vec<LogicalLine>, SyntaxLexError> {
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < raw_lines.len() {
        let raw = raw_lines[i];
        let trimmed = raw.trim();

        if trimmed.is_empty() {
            if !skip_empty_and_comments {
                result.push(LogicalLine {
                    lineno: i + 1,
                    content: String::new(),
                });
            }
            i += 1;
            continue;
        }
        if skip_empty_and_comments && trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        if line_ends_in_open_string(trimmed) {
            let start_lineno = i + 1;
            let mut combined = String::from(trimmed);
            combined.push('\n');
            i += 1;
            let mut closed = false;
            while i < raw_lines.len() {
                let line = raw_lines[i];
                if line.trim() == "\"" {
                    combined.push('"');
                    closed = true;
                    i += 1;
                    break;
                }
                combined.push_str(line);
                combined.push('\n');
                i += 1;
            }
            if !closed {
                return Err(SyntaxLexError::new(
                    start_lineno,
                    "heredoc 字符串未闭合（缺少独占一行的 `\"`）",
                ));
            }
            result.push(LogicalLine {
                lineno: start_lineno,
                content: combined,
            });
        } else {
            result.push(LogicalLine {
                lineno: i + 1,
                content: trimmed.to_string(),
            });
            i += 1;
        }
    }

    Ok(result)
}

/// Detect whether a line ends while still inside a string literal.
pub fn line_ends_in_open_string(line: &str) -> bool {
    let mut in_str: Option<char> = None;
    let mut prev_escape = false;
    for c in line.chars() {
        if prev_escape {
            prev_escape = false;
            continue;
        }
        if c == '\\' {
            prev_escape = true;
            continue;
        }
        match in_str {
            Some(q) if c == q => in_str = None,
            None if c == '"' || c == '\'' => in_str = Some(c),
            _ => {}
        }
    }
    in_str.is_some()
}

/// Split a line into whitespace-delimited chunks, preserving quoted strings,
/// parenthesized expressions, and bracketed JSON/selector chunks.
pub fn tokenize_chunks(line: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        let start = i;
        let mut paren_depth: i32 = 0;
        let mut bracket_depth: i32 = 0;
        let mut brace_depth: i32 = 0;
        let mut in_str: Option<char> = None;
        let mut prev_escape = false;

        while i < chars.len() {
            let c = chars[i];
            if prev_escape {
                prev_escape = false;
                i += 1;
                continue;
            }
            if c == '\\' && in_str.is_some() {
                prev_escape = true;
                i += 1;
                continue;
            }
            if let Some(q) = in_str {
                if c == q {
                    in_str = None;
                }
                i += 1;
                continue;
            }
            match c {
                '"' | '\'' => {
                    in_str = Some(c);
                    i += 1;
                }
                '(' => {
                    paren_depth += 1;
                    i += 1;
                }
                ')' => {
                    paren_depth -= 1;
                    i += 1;
                }
                '[' => {
                    bracket_depth += 1;
                    i += 1;
                }
                ']' => {
                    bracket_depth -= 1;
                    i += 1;
                }
                '{' => {
                    brace_depth += 1;
                    i += 1;
                }
                '}' => {
                    brace_depth -= 1;
                    i += 1;
                }
                _ if c.is_whitespace()
                    && paren_depth == 0
                    && bracket_depth == 0
                    && brace_depth == 0 =>
                {
                    break
                }
                _ => i += 1,
            }
        }

        if i > start {
            chunks.push(chars[start..i].iter().collect());
        }
    }
    chunks
}
