//! RAG 核心类型定义

use serde::{Deserialize, Serialize};

/// 文档
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// 文档ID
    pub id: String,
    /// 文档内容
    pub content: String,
    /// 元数据
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    /// 来源标识
    pub source: Option<String>,
}

/// 查询结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// 匹配的文档
    pub document: Document,
    /// 相关性分数（0.0 - 1.0）
    pub score: f64,
}

/// RAG 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    /// 最大检索数量
    pub top_k: usize,
    pub min_score: f64,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            top_k: 5,
            min_score: 0.3,
        }
    }
}
