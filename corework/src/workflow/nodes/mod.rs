//!
//! 定义各种节点 trait 和分类

pub mod control;
pub mod data;
pub mod node_wrapper;
pub mod traits;

// 重新导出
pub use node_wrapper::NodeWrapper;
pub use traits::{BlueprintNode, NodeType};
