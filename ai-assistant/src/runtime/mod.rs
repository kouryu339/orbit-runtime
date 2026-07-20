//! 运行时协议层——LLM 响应解析 & 行式协议抽象
//! 本模块将 LLM 返回的原始文本解析为结构化的 [`parser::ParsedResponse`]，
//! 供 thinking / executing 等状态机状态消费。
//! **职责边界**：仅负责解析，不执行工具、不修改 cache / 对话历史。

pub mod parser;
