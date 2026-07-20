//! 占位节点模块
//!
//! 提供基础的占位节点，用于框架验证和快速原型开发。
//!
//! ✅ 这些节点会被正常导出到 nodes.json，这是预期行为：
//! - DebugPrintNode: 用于调试，打印值并传递
//! - NoOpNode: 空操作节点，仅流程控制
//! - IdentityNode, PassThroughNode, ConstantNode: 基础数据流节点
//!

pub mod debug_print;
pub mod noop;

// 重新导出所有占位节点
pub use debug_print::DebugPrintNode;
pub use noop::NoOpNode;
