//! 错误类型定义

use thiserror::Error;

/// AI助手错误类型
#[derive(Error, Debug)]
pub enum Error {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("执行错误: {0}")]
    Execution(String),

    #[error("状态机错误: {0}")]
    StateMachine(String),

    #[error("RAG 检索错误: {0}")]
    Rag(String),

    #[error("框架错误: {0}")]
    Framework(#[from] corework::error::FrameworkError),

    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("持久化错误: {0}")]
    Persistence(String),

    #[error("其他错误: {0}")]
    Other(#[from] anyhow::Error),
}

/// 结果类型别名
pub type Result<T> = std::result::Result<T, Error>;
