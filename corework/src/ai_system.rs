//!

use crate::error::{FrameworkError, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AIParameter {
    /// 参数名称
    pub name: &'static str,
    /// 参数类型（String, i32, i64, f32, f64, bool等）
    pub param_type: &'static str,
    /// 是否必填
    pub required: bool,
    /// 默认值（字符串表示）
    pub default_value: Option<&'static str>,
    /// 详细描述（应包含示例、有效值范围等）
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct AIOutputField {
    /// 输出字段名称
    pub name: &'static str,
    /// 字段类型
    pub field_type: &'static str,
    /// 字段描述
    pub description: &'static str,
}

/// 参照 Model Context Protocol (MCP) 的 ToolMetadata 设计
#[derive(Debug, Clone)]
pub struct AISystemMetadata {
    pub name: &'static str,
    /// Human-readable name for UI/display surfaces. Tool lookup and EXEC still use `name`.
    pub display_name: &'static str,
    pub description: &'static str,
    /// Tool implementation origin. Static Rust systems registered by macros
    /// are `local`; runtime stubs registered for external tool providers are
    /// `rpc`.
    pub tool_kind: &'static str,
    pub parameters: &'static [AIParameter],
    /// 输出字段列表（对应 Node 的 data_out）
    pub outputs: &'static [AIOutputField],

    // 行为元数据（参照MCP）
    /// 是否会执行破坏性修改（删除/覆盖现有资源）
    /// 默认 true（安全优先：假设危险，除非明确标记安全）
    pub destructive: bool,

    /// 是否只读操作（不会创建/更新/删除数据）
    /// 默认 false（假设有写操作）
    pub readonly: bool,

    /// 是否幂等（相同参数重复调用产生相同结果）
    /// 默认 false（假设非幂等）
    pub idempotent: bool,

    /// 是否与开放世界交互（动态/不可预测的外部实体）
    /// 默认 true（假设与外部交互）
    pub open_world: bool,

    /// 是否处理敏感信息（密钥/凭据/秘密等）
    /// 默认 false（假设无敏感数据）
    pub secret: bool,
}

pub struct AISystemFactory {
    pub metadata: AISystemMetadata,
    pub constructor: fn() -> Arc<dyn std::any::Any + Send + Sync>,
}

inventory::collect!(AISystemFactory);

#[async_trait]
pub trait AICallableSystem: crate::system::SystemOperation {
    /// 从字符串参数解析输入
    fn parse_args(args: &str) -> Result<Self::Input>;

    /// 将输出转换为AI友好的文本
    fn to_text(output: &Self::Output) -> String;
}

/// 简单的参数解析器（CLI 风格）
///
/// 支持的格式：
/// - `--key value`           键值对
/// - `--key "hello world"`   带引号的值（支持空格）
/// - `--key 'hello world'`   单引号同理
/// - `--flag`                布尔标志（值为 "true"）
/// - `key=value`             等号格式
/// - `key="hello world"`    等号格式带引号
/// - `--tags a,b,c`          逗号分隔 → 通过 get_list() 获取
/// - `--desc "hello \"world\""` 引号内仅处理必要的引号/反斜杠转义
pub struct SimpleArgs {
    args: HashMap<String, String>,
}

impl SimpleArgs {
    ///
    /// 规则：
    /// 1. 已经是 -- 开头的保持不变
    /// 2. 已经是 = 格式的保持不变（会自动处理）
    /// 3. 其他情况：如果后面跟着空格+值，自动添加 --
    ///
    #[allow(dead_code)]
    fn normalize_args(input: &str) -> String {
        input.trim().to_string()
    }

    /// 词法分析：双指针版本，正确处理嵌套的引号和方括号
    ///
    /// 使用 start 和 end 双指针，start 指向 token 开头，end 向前扫描找分隔符
    /// 分隔符必须是：
    /// - 空格，且不在引号内（' 或 "），且不在方括号内
    #[allow(dead_code)]
    fn tokenize(input: &str) -> Result<Vec<String>> {
        let mut tokens = Vec::new();
        let chars: Vec<char> = input.chars().collect();
        let mut start = 0;

        while start < chars.len() {
            // 跳过前导空白
            while start < chars.len() && chars[start].is_whitespace() {
                start += 1;
            }
            if start >= chars.len() {
                break;
            }

            let mut end = start;
            let mut in_quote: Option<char> = None;
            let mut bracket_depth = 0;

            while end < chars.len() {
                let ch = chars[end];

                // 在方括号内，跳过引号检查（CSS 选择器内部）
                if bracket_depth > 0 {
                    if ch == '[' {
                        bracket_depth += 1;
                    } else if ch == ']' {
                        bracket_depth -= 1;
                    }
                    end += 1;
                    continue;
                }

                match ch {
                    // 遇到引号：找到匹配的结束引号
                    '"' | '\'' if in_quote.is_none() => {
                        in_quote = Some(ch);
                        end += 1;
                    }
                    q if in_quote.is_some() && Some(q) == in_quote => {
                        // 找到匹配的结束引号
                        in_quote = None;
                        end += 1;
                    }
                    // 遇到方括号：进入括号块
                    '[' => {
                        bracket_depth += 1;
                        end += 1;
                    }
                    // 遇到空白且不在引号内和方括号内：分隔符
                    c if c.is_whitespace() && in_quote.is_none() && bracket_depth == 0 => {
                        break;
                    }
                    _ => {
                        end += 1;
                    }
                }
            }

            // 提取 token
            let token: String = chars[start..end].iter().collect();
            if !token.is_empty() {
                // 处理 token 内部的转义字符
                tokens.push(Self::unescape_token(&token));
            }

            start = end;
        }

        Ok(tokens)
    }

    /// 处理 token 中的转义字符
    fn unescape_token(token: &str) -> String {
        Self::unescape_token_with_mode(token, false)
    }

    /// 处理路径类 token 中的转义字符。
    ///
    /// 不能按 C/JSON 字符串语义反转义成制表符或换行。
    fn unescape_path_token(token: &str) -> String {
        let mut result = String::new();
        let mut chars = token.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch != '\\' {
                result.push(ch);
                continue;
            }

            let Some(&escaped) = chars.peek() else {
                result.push('\\');
                continue;
            };

            match escaped {
                '"' | '\'' | ' ' => {
                    chars.next();
                    result.push(escaped);
                }
                '\\' => {
                    chars.next();
                    if result.is_empty() {
                        result.push('\\');
                        result.push('\\');
                    } else {
                        result.push('\\');
                    }
                }
                _ => result.push('\\'),
            }
        }

        result
    }

    fn unescape_token_with_mode(token: &str, preserve_path_backslashes: bool) -> String {
        let mut result = String::new();
        let mut chars = token.chars().peekable();

        while let Some(&ch) = chars.peek() {
            if ch == '\\' {
                chars.next(); // 消费 '\'
                if let Some(&escaped) = chars.peek() {
                    chars.next();
                    match escaped {
                        'n' if !preserve_path_backslashes => result.push('\n'),
                        't' if !preserve_path_backslashes => result.push('\t'),
                        'r' if !preserve_path_backslashes => result.push('\r'),
                        '\\' => result.push('\\'),
                        '"' => result.push('"'),
                        '\'' => result.push('\''),
                        ' ' => result.push(' '),
                        other => {
                            result.push('\\');
                            result.push(other);
                        }
                    }
                } else {
                    result.push('\\');
                }
            } else {
                chars.next();
                result.push(ch);
            }
        }

        result
    }

    fn is_path_like_key(key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        key == "path"
            || key == "paths"
            || key.ends_with("_path")
            || key.ends_with("_paths")
            || key.ends_with("path")
            || key.ends_with("paths")
            || key.contains("directory")
            || key.contains("folder")
    }

    /// 解析 CLI 风格参数字符串
    ///
    /// 解析规则：
    /// 1. `--` 开头的 token 是参数名
    /// 2. 参数值：等下一个 `--`
    ///    - 中间无内容 → "true"
    ///    - 中间有内容 → 那个字符串
    ///    - 没等到就结束了 → "true"
    ///
    /// # 示例
    /// ```rust,ignore
    /// let args = SimpleArgs::parse(r#"--name "hello world" --count 5 --verbose"#)?;
    /// assert_eq!(args.get("name"), Some("hello world"));
    /// assert_eq!(args.get_i64("count"), Some(5));
    /// assert_eq!(args.get_bool("verbose"), true);
    /// ```
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let mut args = HashMap::new();

        // 直接在原始字符串上解析，不先 tokenize
        let mut i = 0;
        let bytes = input.as_bytes();

        while i < bytes.len() {
            // 跳过空格
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }

            // 检查是否是 --
            if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                // 找到 --，解析参数名
                i += 2;
                let key_start = i;

                // 读取参数名（直到空格、=、或字符串结束）
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
                    i += 1;
                }
                let key = &input[key_start..i];

                if key.is_empty() {
                    // 只有一个 -，跳过
                    continue;
                }

                // 跳过等号和空格
                while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
                    if bytes[i] == b'=' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }

                // 解析值：直接提取到下一个 -- 之前的原始内容（不解析内部结构）
                let value_start = i;
                let mut in_quote: Option<u8> = None;
                let mut escaped = false;
                while i < bytes.len() {
                    let b = bytes[i];
                    if escaped {
                        escaped = false;
                        i += 1;
                        continue;
                    }
                    if in_quote.is_some() && b == b'\\' {
                        escaped = true;
                        i += 1;
                        continue;
                    }
                    if b == b'"' || b == b'\'' {
                        match in_quote {
                            Some(q) if q == b => in_quote = None,
                            None => in_quote = Some(b),
                            _ => {}
                        }
                        i += 1;
                        continue;
                    }
                    if in_quote.is_none()
                        && b == b'-'
                        && i + 1 < bytes.len()
                        && bytes[i + 1] == b'-'
                    {
                        break;
                    }
                    i += 1;
                }
                // 找到前一个非空字符
                let mut end = i;
                while end > value_start && bytes[end - 1].is_ascii_whitespace() {
                    end -= 1;
                }
                let value = &input[value_start..end];

                let value = if value.is_empty() {
                    "true".to_string()
                } else if (value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))
                {
                    let inner = &value[1..value.len() - 1];
                    if Self::is_path_like_key(key) {
                        Self::unescape_path_token(inner)
                    } else {
                        Self::unescape_token(inner)
                    }
                } else {
                    value.to_string()
                };

                args.insert(key.to_string(), value);
            } else {
                // 不是 -- 开头，跳过
                i += 1;
            }
        }

        Ok(Self { args })
    }

    /// 获取字符串值
    pub fn get(&self, key: &str) -> Option<&str> {
        self.args.get(key).map(|s| s.as_str())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.args.keys().map(|k| k.as_str())
    }

    /// 获取必填字符串值，缺失时返回 `AIOutput::error` 而非 `FrameworkError`。
    ///
    /// ```rust,ignore
    /// let path = match args.safe_require("path") { Ok(v) => v, Err(e) => return Ok(e) };
    /// ```
    pub fn safe_require(&self, key: &str) -> std::result::Result<String, AIOutput> {
        self.get_required(key)
            .map_err(|e| AIOutput::error(400, e.to_string()))
    }

    /// 获取必填字符串值，缺失时返回友好错误（包含已提供的参数名辅助诊断）
    pub fn get_required(&self, key: &str) -> Result<String> {
        self.get(key).map(|s| s.to_string()).ok_or_else(|| {
            let provided: Vec<String> = self.args.keys().map(|k| format!("--{}", k)).collect();
            let provided_str = if provided.is_empty() {
                "无".to_string()
            } else {
                provided.join(", ")
            };
            FrameworkError::InvalidOperation(format!(
                "缺失必填参数: --{}。你实际提供的参数: [{}]，请严格使用工具定义中的参数名",
                key, provided_str
            ))
        })
    }

    /// 获取布尔值（支持 true/1/yes）
    pub fn get_bool(&self, key: &str) -> bool {
        self.get(key)
            .map(|v| v == "true" || v == "1" || v == "yes")
            .unwrap_or(false)
    }

    /// 获取 i64 值
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// 获取 f64 值
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// 获取逗号分隔的列表值
    ///
    /// `--tags a,b,c` → `vec!["a", "b", "c"]`
    pub fn get_list(&self, key: &str) -> Vec<&str> {
        self.get(key)
            .map(|v| v.split(',').map(|s| s.trim()).collect())
            .unwrap_or_default()
    }

    /// 获取带默认值的字符串
    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
    }

    /// 获取带默认值的 i64
    pub fn get_i64_or(&self, key: &str, default: i64) -> i64 {
        self.get_i64(key).unwrap_or(default)
    }

    /// 获取所有参数的键名（用于调试/验证）
    pub fn keys(&self) -> Vec<&str> {
        self.args.keys().map(|k| k.as_str()).collect()
    }

    /// 检查某个参数是否存在
    pub fn has(&self, key: &str) -> bool {
        self.args.contains_key(key)
    }

    /// 根据 AIParameter 元数据验证参数完整性
    ///
    /// 检查所有 required 参数是否都提供了
    pub fn validate(&self, params: &[AIParameter]) -> Result<()> {
        let missing: Vec<&str> = params
            .iter()
            .filter(|p| p.required && !self.has(p.name))
            .map(|p| p.name)
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(FrameworkError::InvalidOperation(format!(
                "缺失必填参数：{}",
                missing
                    .iter()
                    .map(|n| format!("--{}", n))
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    }
}

///
/// 再用 `args.get_required("param")` / `args.get("param")` 按名提取参数。
///
/// # 示例
/// ```rust,ignore
/// async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
///     let args = input.parse_args()?;
///     let path = args.get_required("path")?;
///     // ...
/// }
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AIInput {
    /// CLI 参数原始字符串，如 "--path /tmp/file --format mp3"
    #[serde(default)]
    pub input: String,
}

impl AIInput {
    /// 解析 CLI 参数为 `SimpleArgs` 键值对
    pub fn parse_args(&self) -> Result<SimpleArgs> {
        SimpleArgs::parse(&self.input)
    }

    /// 解析参数，失败时返回 `AIOutput::error` 而非 `FrameworkError`。
    ///
    /// ```rust,ignore
    /// let args = match input.safe_parse_args() { Ok(a) => a, Err(e) => return Ok(e) };
    /// ```
    pub fn safe_parse_args(&self) -> std::result::Result<SimpleArgs, AIOutput> {
        self.parse_args()
            .map_err(|e| AIOutput::error(400, format!("参数解析失败: {}", e)))
    }

    pub fn from_args(args: HashMap<String, String>) -> Self {
        let input = args
            .into_iter()
            .map(|(k, v)| {
                // 如果值包含空格或特殊字符，用引号包裹
                if v.contains(' ') || v.contains('"') || v.contains('\\') {
                    format!(
                        "--{} \"{}\"",
                        k,
                        v.replace('\\', "\\\\").replace('"', "\\\"")
                    )
                } else {
                    format!("--{} {}", k, v)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        Self { input }
    }
}

///
/// - `to_ai`：给 AI 看的文本摘要，插入到对话历史
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AIOutput {
    /// 真实结果（结构化 JSON）
    pub result: serde_json::Value,
    /// 给 AI 看的文本摘要
    pub to_ai: String,
    /// 错误码：0 = 成功，非 0 = 失败
    pub error_code: i32,
}

impl AIOutput {
    /// 构造成功输出
    pub fn success(result: impl serde::Serialize, to_ai: impl Into<String>) -> Self {
        Self {
            result: serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
            to_ai: to_ai.into(),
            error_code: 0,
        }
    }

    /// 构造失败输出
    pub fn error(code: i32, to_ai: impl Into<String>) -> Self {
        Self {
            result: serde_json::Value::Null,
            to_ai: to_ai.into(),
            error_code: code,
        }
    }

    /// 是否成功
    pub fn is_ok(&self) -> bool {
        self.error_code == 0
    }
}

#[cfg(test)]
mod tests {
    use super::SimpleArgs;

    #[test]
    fn path_arguments_keep_windows_backslashes_literal() {
        let args = SimpleArgs::parse(r#"--path "D:\Desktop\text.mp4""#).unwrap();
        assert_eq!(args.get("path"), Some(r"D:\Desktop\text.mp4"));
    }

    #[test]
    fn path_arguments_keep_newline_like_segments_literal() {
        let args = SimpleArgs::parse(r#"--output_path "D:\new\raw\track.mp4""#).unwrap();
        assert_eq!(args.get("output_path"), Some(r"D:\new\raw\track.mp4"));
    }

    #[test]
    fn path_arguments_decode_doubled_backslashes_after_drive_prefix() {
        let args = SimpleArgs::parse(r#"--file_path "D:\\Desktop\\text.mp4""#).unwrap();
        assert_eq!(args.get("file_path"), Some(r"D:\Desktop\text.mp4"));
    }

    #[test]
    fn path_arguments_preserve_unc_prefix() {
        let args = SimpleArgs::parse(r#"--path "\\server\share\text.mp4""#).unwrap();
        assert_eq!(args.get("path"), Some(r"\\server\share\text.mp4"));
    }

    #[test]
    fn non_path_arguments_keep_legacy_escape_sequences() {
        let args = SimpleArgs::parse(r#"--message "hello\tworld""#).unwrap();
        assert_eq!(args.get("message"), Some("hello\tworld"));
    }
}
