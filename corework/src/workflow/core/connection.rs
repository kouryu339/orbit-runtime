//! Connection - 引脚间的连接
//!
//! 从 blueprint.rs 提取

use serde::{Deserialize, Serialize};

/// Connection between pins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub from_node: String,
    pub from_pin: String,
    pub to_node: String,
    pub to_pin: String,
}

impl Connection {
    pub fn new(
        from_node: impl Into<String>,
        from_pin: impl Into<String>,
        to_node: impl Into<String>,
        to_pin: impl Into<String>,
    ) -> Self {
        Self {
            from_node: from_node.into(),
            from_pin: from_pin.into(),
            to_node: to_node.into(),
            to_pin: to_pin.into(),
        }
    }
}
