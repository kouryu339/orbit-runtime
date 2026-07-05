//! 控制流节点模块
//!
//! 从 control_nodes.rs 迁移

pub mod branch;
pub mod break_node;
pub mod for_loop;
pub mod start_end;

// 重新导出常用节点
pub use branch::BranchNode;
pub use break_node::BreakNode;
pub use for_loop::ForLoopNode;
pub use start_end::{EndNode, PinMapping, StartNode};
