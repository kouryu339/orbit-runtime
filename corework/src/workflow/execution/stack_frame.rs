//! 栈帧 - 保存函数/节点调用的局部状态
//!

use crate::workflow::core::DataValue;
use std::collections::HashMap;

/// 栈帧类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameType {
    /// 函数调用
    Function,
    /// 循环迭代
    LoopIteration { current: usize, total: usize },
    /// 事件处理
    Event,
    /// 宏展开
    Macro,
}

/// 栈帧 - 保存节点执行的瞬时状态
///
/// 每次进入节点时创建，离开节点时销毁
///
/// 用途：
/// - 函数参数传递
/// - 局部变量存储
/// - 返回地址记录
#[derive(Debug, Clone)]
pub struct StackFrame {
    /// 节点名称
    pub node_name: String,

    /// 局部变量（生命周期仅限本栈帧）
    pub local_vars: HashMap<String, DataValue>,

    /// 返回地址（执行完成后回到哪个节点）
    pub return_address: Option<String>,

    /// 栈帧类型
    pub frame_type: FrameType,
}

impl StackFrame {
    /// 创建函数栈帧
    pub fn new_function(node_name: impl Into<String>) -> Self {
        Self {
            node_name: node_name.into(),
            local_vars: HashMap::new(),
            return_address: None,
            frame_type: FrameType::Function,
        }
    }

    /// 创建循环迭代栈帧
    pub fn new_loop_iteration(node_name: impl Into<String>, current: usize, total: usize) -> Self {
        Self {
            node_name: node_name.into(),
            local_vars: HashMap::new(),
            return_address: None,
            frame_type: FrameType::LoopIteration { current, total },
        }
    }

    /// 创建事件栈帧
    pub fn new_event(node_name: impl Into<String>) -> Self {
        Self {
            node_name: node_name.into(),
            local_vars: HashMap::new(),
            return_address: None,
            frame_type: FrameType::Event,
        }
    }

    /// 设置返回地址
    pub fn with_return_address(mut self, return_to: impl Into<String>) -> Self {
        self.return_address = Some(return_to.into());
        self
    }

    /// 添加局部变量
    pub fn with_local_var(mut self, key: impl Into<String>, value: DataValue) -> Self {
        self.local_vars.insert(key.into(), value);
        self
    }
}
