//! Views — AI 向用户提问时的交互载体
//! ## 新模式（内联 widgets）
//! AI 在 `asking` 决策的 `prompt` 字段中直接嵌入控件标签，前端解析渲染。
//! ## Widget 标签格式
//! `[widget_type | key=value | key=value]`，每个标签独占一行，可与自然语言混排。
//! 支持的控件：
//! - `[input:text | label="说明"]`                          — 纯文本输入框
//! - `[input:path | label="说明" | accept=".mp4,.mov"]`     — 路径/文件选择
//! - `[input:date | label="说明"]`                          — 日期选择（YYYY-MM-DD）
//! - `[input:time | label="说明"]`                          — 时间选择（HH:MM）
//! - `[select:single | label="说明" | options="A,B,C"]`     — 单选
//! - `[select:multi  | label="说明" | options="A,B,C"]`     — 多选
//! - `[confirm | label="操作说明"]`                         — 确认/取消

use serde::{Deserialize, Serialize};

// ============================================================================
// ViewPayload — ASK 决策在 `saying` 稳态里的展示载体
// ============================================================================

/// AI 向用户提问时携带的数据
/// - `prompt`：对用户说的话，可内嵌 `[widget...]` 标签
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewPayload {
    /// 视图类型，固定为 "widgets"
    pub view: String,
    /// 提示文案，可包含内嵌控件标签
    pub prompt: String,
}

impl ViewPayload {
    /// 创建 widgets 视图（唯一构造方式）
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            view: "widgets".into(),
            prompt: prompt.into(),
        }
    }

    /// 向下兼容别名
    pub fn text(prompt: impl Into<String>) -> Self {
        Self::new(prompt)
    }

    /// 向下兼容别名
    pub fn widgets(prompt: impl Into<String>) -> Self {
        Self::new(prompt)
    }
}

// ============================================================================
// prompt 写作指导（写入 system prompt）
// ============================================================================

/// 生成"向用户提问"章节，写入 AI system prompt
pub fn format_asking_section() -> String {
    crate::prompt_assets::template("asking_section.md")
        .trim()
        .to_string()
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_payload_new() {
        let p = ViewPayload::new("请填写名称");
        assert_eq!(p.view, "widgets");
        assert_eq!(p.prompt, "请填写名称");
    }

    #[test]
    fn test_view_payload_compat() {
        let p = ViewPayload::text("你好");
        assert_eq!(p.view, "widgets");

        let p2 = ViewPayload::widgets("请填写\n[input:text | label=\"名称\"]");
        assert_eq!(p2.view, "widgets");
        assert!(p2.prompt.contains("[input:text"));
    }

    #[test]
    fn test_view_payload_serde() {
        let p = ViewPayload::new("请选择格式");
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"view\":\"widgets\""));

        let back: ViewPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.view, "widgets");
    }

    #[test]
    fn test_format_asking_section() {
        let section = format_asking_section();
        assert!(section.contains("向用户提问"));
        assert!(section.contains("input:path"));
        assert!(section.contains("select:single"));
        assert!(section.contains("confirm"));
    }
}
