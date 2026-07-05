//! AI 决策类型
//! 行式输出协议（feat/line-protocol 起）下，LLM 的所有决策由
//! [`crate::decision_line::parse_line_protocol`] 解析为 `AIDecision`。
//! 本模块只保留：
//! - `AIDecision` 枚举：状态机驱动用
//! - `ToolResult`：工具执行结果
//! - `strip_think_tags_pub` / `contains_widget_tag` 等纯工具函数
//! 旧的 JSON 解析路径（`parse_ai_response` 7 层修复链）已整体删除——
//! 协议层只剩行式协议，FC 模式（`assistant_decide` 伪工具）也一并废弃。

use serde::{Deserialize, Serialize};

use crate::views::ViewPayload;

/// AI 决策 — 由 thinking 状态解析 LLM 响应得到
/// 三种变体对应三个目标状态：
/// - `Executing` → 执行工具命令
/// - `Asking` → 向用户提问
/// - `Result` → 给出最终结论
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AIDecision {
    /// AI 决定执行工具
    Executing {
        /// AI 的推理过程（不展示给用户，用于调试和日志）
        #[serde(default)]
        reasoning: Option<String>,
        /// 待执行的命令列表（按序执行，CLI 字符串 `ToolName --arg val ...`）
        tools: Vec<String>,
    },

    /// AI 决定向用户提问
    Asking {
        #[serde(default)]
        reasoning: Option<String>,
        /// 向用户说的话，可内嵌控件标签（如 `[input:path | label="文件"]`）
        #[serde(default)]
        prompt: Option<String>,
    },

    /// AI 认为任务完成，给出最终结论
    Result {
        #[serde(default)]
        reasoning: Option<String>,
        result: String,
    },
}

impl AIDecision {
    /// 将 Asking 变体转换为 ViewPayload
    /// **仅当 prompt 中包含 widget 标签时**才返回 ViewPayload。
    /// 纯文本提问不需要 widgets 视图（否则前端会多渲染一遍文本 + 不必要的"确认提交"按钮）。
    pub fn to_view_payload(&self) -> Option<ViewPayload> {
        match self {
            AIDecision::Asking { prompt, .. } => {
                let text = prompt.clone().unwrap_or_default();
                if contains_widget_tag(&text) {
                    Some(ViewPayload::new(text))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 获取提问文本（用于插入对话历史）
    pub fn asking_text(&self) -> Option<String> {
        match self {
            AIDecision::Asking { prompt, .. } => {
                let text = prompt.clone().unwrap_or_default();
                if text.trim().is_empty() {
                    tracing::warn!("AIDecision::asking_text: prompt 字段为 None 或空字符串。");
                }
                Some(text)
            }
            _ => None,
        }
    }

    /// 获取 AI 的推理过程文本
    pub fn reasoning(&self) -> Option<&str> {
        match self {
            AIDecision::Executing { reasoning, .. } => reasoning.as_deref(),
            AIDecision::Asking { reasoning, .. } => reasoning.as_deref(),
            AIDecision::Result { reasoning, .. } => reasoning.as_deref(),
        }
    }
}

/// 工具执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// 原始命令
    pub command: String,
    /// 是否成功（error_code == 0）
    pub success: bool,
    /// 给 AI 看的文本摘要（插入对话历史）
    pub to_ai: String,
    /// 错误码：0 = 成功，非 0 = 失败
    pub error_code: i32,
    pub result: serde_json::Value,
}

/// 检查 prompt 文本中是否包含 widget 标签
/// 匹配模式：`[input:...`、`[select:...`、`[confirm`
pub fn contains_widget_tag(text: &str) -> bool {
    text.contains("[input:") || text.contains("[select:") || text.contains("[confirm")
}

/// 剥离 `<think>...</think>` 思考块，返回剩余文本（pub 版，供外部模块使用）
pub fn strip_think_tags_pub(text: &str) -> &str {
    strip_think_tags(text)
}

/// 剥离 `<think>...</think>` 思考块，返回剩余文本
fn strip_think_tags(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(end) = trimmed.rfind("</think>") {
        trimmed[end + "</think>".len()..].trim()
    } else if trimmed.starts_with("<think>") {
        ""
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_tag_detection() {
        assert!(contains_widget_tag(
            "请选择 [select:single | options=\"a,b\"]"
        ));
        assert!(contains_widget_tag("[input:path | label=\"文件\"]"));
        assert!(contains_widget_tag("[confirm | label=\"确定？\"]"));
        assert!(!contains_widget_tag("纯文本提问没有控件"));
    }

    #[test]
    fn strip_think_basic() {
        assert_eq!(strip_think_tags("<think>推理</think>正文"), "正文");
        assert_eq!(strip_think_tags("无 think 标签"), "无 think 标签");
        assert_eq!(strip_think_tags("<think>截断"), "");
    }

    #[test]
    fn asking_to_view_payload_only_with_widget() {
        let with_widget = AIDecision::Asking {
            reasoning: None,
            prompt: Some("请选择 [select:single | options=\"a,b\"]".to_string()),
        };
        assert!(with_widget.to_view_payload().is_some());

        let plain = AIDecision::Asking {
            reasoning: None,
            prompt: Some("你好吗？".to_string()),
        };
        assert!(plain.to_view_payload().is_none());
    }

    #[test]
    fn reasoning_accessor() {
        let d = AIDecision::Executing {
            reasoning: Some("测试推理".to_string()),
            tools: vec![],
        };
        assert_eq!(d.reasoning(), Some("测试推理"));
    }
}
