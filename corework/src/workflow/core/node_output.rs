//! NodeOutput - 节点执行结果
//!
//! 从 blueprint.rs 提取

use super::DataValue;
use std::collections::HashMap;

/// 循环迭代数据
#[derive(Debug, Clone)]
pub struct LoopIteration {
    /// 当前迭代的输出数据（如 Index、IterationCount）
    pub outputs: HashMap<String, DataValue>,
}

/// 节点执行输出
#[derive(Debug, Clone)]
pub enum NodeOutput {
    /// 执行下一个引脚（白色线）
    ExecPin(String),

    /// 输出数据（彩色线）
    Data(HashMap<String, DataValue>),

    /// 多个执行输出（如 Sequence 节点）
    Multiple(Vec<String>),

    /// 循环执行（ForLoop、WhileLoop）
    Loop {
        /// 循环体执行引脚名称
        body_pin: String,
        /// 循环完成后执行的引脚名称
        completed_pin: String,
        /// 所有迭代的数据
        iterations: Vec<LoopIteration>,
    },

    /// 中断当前循环（Break 节点）
    Break,

    /// 执行完成
    Complete,
}
