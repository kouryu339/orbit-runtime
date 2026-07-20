//! 转移历史记录

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 转移历史记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRecord {
    /// 源状态
    pub from_state: String,

    /// 目标状态
    pub to_state: String,

    /// 触发事件
    pub event: String,

    /// 转移时间
    pub timestamp: DateTime<Utc>,
}
