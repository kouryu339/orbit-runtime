//! 文档存储
//!
//! 管理文档的存储和索引

use super::types::Document;
use crate::error::Result;
use async_trait::async_trait;

/// 文档存储 trait
#[async_trait]
pub trait DocumentStore: Send + Sync {
    /// 添加文档
    async fn add_document(&self, doc: Document) -> Result<()>;

    /// 批量添加文档
    async fn add_documents(&self, docs: Vec<Document>) -> Result<()>;

    /// 按 ID 获取文档
    async fn get_document(&self, id: &str) -> Result<Option<Document>>;

    /// 删除文档
    async fn remove_document(&self, id: &str) -> Result<()>;

    /// 文档数量
    async fn count(&self) -> Result<usize>;
}
