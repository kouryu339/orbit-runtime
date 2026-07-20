//! 业务模块（简单封装 ExecutionUnit）
//!
//! Module 当前就是带权限管理的执行单元，用于：
//! - 访问 World 全局资源（带权限检查）
//! - 通过 EventBus 与其他模块通信
//! - 调用 Blueprint、StateMachine 或无状态 System
//! - 自己管理初始化和数据持久化
//!

pub use crate::execution_unit::{AccessMode, ExecutionUnit, UnitType};
pub use crate::world::FrameworkState;

use std::sync::Arc;

// 为了方便，提供类型别名
pub type Module = Arc<ExecutionUnit>;

/// 创建业务模块
///
/// # 示例
///
/// ```rust,ignore
/// let scores = create_module("scores")?;
///
/// // 声明资源权限
/// scores.declare_resource_access("scores:references", AccessMode::Owner)?;
/// scores.grant_access_to("scores:references", "*", AccessMode::ReadWrite)?;
///
/// // 使用资源
/// scores.set_resource("scores:references", &data, None)?;
/// let data: Vec<Item> = scores.get_resource("scores:references")?.unwrap_or_default();
/// ```
pub fn create_module(module_id: impl Into<String>) -> crate::error::Result<Module> {
    let _module_id = module_id.into();
    let framework = FrameworkState::initialize()?;
    Ok(Arc::new(ExecutionUnit::new_root(
        UnitType::Module,
        framework,
    )))
}

pub fn create_child_module(
    module_id: impl Into<String>,
    parent: &Arc<ExecutionUnit>,
) -> crate::error::Result<Module> {
    let _module_id = module_id.into();
    Ok(Arc::new(ExecutionUnit::new_child(
        UnitType::Module,
        parent,
    )?))
}

/// 创建业务模块（从指定的 Framework）
pub fn create_module_with_framework(
    module_id: impl Into<String>,
    framework: FrameworkState,
) -> Module {
    let _module_id = module_id.into();
    Arc::new(ExecutionUnit::new_root(UnitType::Module, framework))
}
