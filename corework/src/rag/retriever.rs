//! 检索器
//!
//! 基于查询文本检索相关文档

use super::types::{QueryResult, RagConfig};
use crate::error::Result;
use async_trait::async_trait;

/// 检索器 trait
#[async_trait]
pub trait Retriever: Send + Sync {
    /// 检索相关文档
    async fn retrieve(&self, query: &str, config: &RagConfig) -> Result<Vec<QueryResult>>;
}
