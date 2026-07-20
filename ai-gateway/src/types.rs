//! 共享响应类型
//!
//! 纯数据结构，无业务逻辑。下游 crate 可自行转换为 buns_model。

use serde::{Deserialize, Serialize};

// ============================================================================
// 通用
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// 对话消息
///
/// # 调用范式规范
///
/// 本结构同时承载两类上游调用范式：
///
/// 1. 纯文本范式
///    - `system` / `user` / `assistant`
///    - 不应携带 `tool_call_id` / `tool_calls`
///
/// 2. Function Calling 范式
///    - 工具请求消息必须为 `assistant(tool_calls)`
///    - 工具结果消息必须为 `tool(tool_call_id=...)`
///
/// 下游 crate 应尽量通过本类型提供的构造函数创建消息，避免手写字段导致协议不一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 角色: "system" | "user" | "assistant" | "tool"
    pub role: String,
    /// 文本内容
    pub content: String,
    #[serde(default)]
    pub cache_control: bool,
    /// tool 消息的配对 ID（对应 ToolCall.id）
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
    /// tool 消息的函数名
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    /// assistant 消息携带的 tool_calls（function calling 模式下模型返回）
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// 思考模型返回的 reasoning_content（DeepSeek 等要求下一轮原样传回）
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning_content: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            cache_control: false,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
    pub fn system_cached(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            cache_control: true,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            cache_control: false,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            cache_control: false,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
    /// 创建 assistant 消息，携带文本内容与 tool_calls（兼容需要保留少量说明文本的 FC 轮次）
    pub fn assistant_with_content_and_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            cache_control: false,
            tool_call_id: None,
            name: None,
            tool_calls: Some(tool_calls),
            reasoning_content: None,
        }
    }
    /// 创建 assistant 消息，携带 tool_calls（function calling 模式）
    pub fn assistant_with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self::assistant_with_content_and_tool_calls(String::new(), tool_calls)
    }
    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            cache_control: false,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }
    }
    /// 创建带配对 ID 的 tool 消息（function calling 模式）
    pub fn tool_with_id(
        content: impl Into<String>,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            cache_control: false,
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
            tool_calls: None,
            reasoning_content: None,
        }
    }
}

/// 图片尺寸
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

impl From<(u32, u32)> for ImageSize {
    fn from(t: (u32, u32)) -> Self {
        Self {
            width: t.0,
            height: t.1,
        }
    }
}

// ============================================================================
// OCR
// ============================================================================

/// 2D 矩形边界框
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BBox2D {
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
}

/// OCR 单项识别结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResultItem {
    /// 识别文本
    pub text: String,
    /// 2D 边界框
    pub bbox_2d: Option<BBox2D>,
    /// 置信度 (0.0‑1.0)
    pub confidence: Option<f32>,
}

/// OCR 完整识别结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    /// 图片尺寸
    pub image_size: ImageSize,
    /// 结果列表
    pub items: Vec<OcrResultItem>,
    /// 识别耗时（毫秒）
    pub elapsed_ms: Option<u64>,
}

// ============================================================================
// VLM
// ============================================================================

/// VLM 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VlmResponse {
    pub content: String,
    pub tokens: Option<TokenUsage>,
}

// ============================================================================
// ASR（语音识别）
// ============================================================================

/// ASR 分段（含时间戳）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrSegment {
    /// 分段起始时间（毫秒）
    pub start_ms: u64,
    /// 分段结束时间（毫秒）
    pub end_ms: u64,
    /// 分段文本
    pub text: String,
}

/// ASR 转录响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrResponse {
    /// 转录全文
    pub text: String,
    /// 检测到的语言（如 "zh" / "en"）
    pub language: Option<String>,
    /// 分段列表（含时间戳，部分提供商返回）
    pub segments: Option<Vec<AsrSegment>>,
    /// 识别耗时（毫秒）
    pub elapsed_ms: Option<u64>,
    /// Token 用量（部分提供商返回）
    pub tokens: Option<TokenUsage>,
}

// ============================================================================
// LLM
// ============================================================================

/// LLM 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tokens: Option<TokenUsage>,
    #[serde(default)]
    pub cached_tokens: u32,
    /// 模型返回的 tool_calls（function calling 模式）
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// 思考模型返回的 reasoning_content（需要随下一轮 assistant 历史传回）
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

// ============================================================================
// Function Calling
// ============================================================================

/// Function Calling 工具定义（传给 DashScope API 的 tools 参数）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// 固定为 "function"
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

/// 函数定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    /// 函数描述
    pub description: String,
    /// 参数 JSON Schema（type: "object", properties: {...}, required: [...]）
    pub parameters: serde_json::Value,
}

/// 模型返回的 tool_call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// 调用 ID（用于 tool 结果消息的配对）
    #[serde(default)]
    pub id: String,
    /// 固定为 "function"
    #[serde(rename = "type", default)]
    pub call_type: Option<String>,
    /// 函数调用信息
    pub function: FunctionCall,
}

impl ToolCall {
    /// 创建标准 function tool_call。
    ///
    /// - `arguments` 必须是 JSON 字符串而不是对象字面量
    pub fn function(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            call_type: Some("function".into()),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

/// 函数调用（模型返回的具体调用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// 函数名
    pub name: String,
    /// 参数 JSON 字符串（DashScope 返回的是字符串，需要再解析）
    pub arguments: String,
}
