//! 变量节点库 (Variable Nodes)
//!
//! 支持多种基本类型的变量节点，用于存储和管理工作流执行过程中的数据值
//!
//! 新增通用可变变量节点：
//! - `SetVarNode`：将值写入具名变量槽（支持跨迭代状态）
//! - `GetVarNode`：从具名变量槽读取当前值

pub mod get_var;
pub mod set_var;

pub use get_var::GetVarNode;
pub use set_var::SetVarNode;
