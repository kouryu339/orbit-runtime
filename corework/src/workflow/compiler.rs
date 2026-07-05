//!
//! 参考 UE 的 FKismetCompiler 实现
//! 关键特性：数据流环检测（DAG验证）

use crate::error::{FrameworkError, Result};
use crate::workflow::core::Connection;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileErrorType {
    TypeError,
    ConnectionError,
    UnresolvedWildcard,
    MissingNode,
    InvalidPin,
    CircularDependency,
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub node_name: Option<String>,
    pub message: String,
    pub error_type: CompileErrorType,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(node) = &self.node_name {
            write!(f, "[{}] {}", node, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    pub fn type_error(node: Option<String>, msg: impl Into<String>) -> Self {
        Self {
            node_name: node,
            message: msg.into(),
            error_type: CompileErrorType::TypeError,
        }
    }

    pub fn connection_error(msg: impl Into<String>) -> Self {
        Self {
            node_name: None,
            message: msg.into(),
            error_type: CompileErrorType::ConnectionError,
        }
    }

    pub fn unresolved_wildcard(wildcard_id: impl Into<String>) -> Self {
        Self {
            node_name: None,
            message: format!("Unresolved wildcard type: {}", wildcard_id.into()),
            error_type: CompileErrorType::UnresolvedWildcard,
        }
    }

    pub fn circular_dependency(cycle: Vec<String>) -> Self {
        Self {
            node_name: cycle.first().cloned(),
            message: format!(
                "Circular data dependency detected: {} → (循环)",
                cycle.join(" → ")
            ),
            error_type: CompileErrorType::CircularDependency,
        }
    }
}

///
pub struct BlueprintCompiler;

impl BlueprintCompiler {
    /// 检测数据流中的环状依赖（DAG验证）
    ///
    /// UE机制说明：
    /// - **执行流（Exec引脚）**：允许回路（ForLoop等循环节点）
    /// - **数据流（Data引脚）**：必须是DAG，不允许循环
    ///
    /// 检测策略：
    /// 1. 只检查数据引脚连接
    /// 2. 使用拓扑排序（Kahn算法）
    /// 3. 如果无法完成排序，说明存在环
    pub fn detect_data_cycles(node_names: &[String], connections: &[Connection]) -> Result<()> {
        // 1. 过滤出所有数据流连接（非Exec引脚）
        let data_connections: Vec<_> = connections
            .iter()
            .filter(|conn| {
                // 简化判断：引脚名不是常见的执行引脚名
                !is_exec_pin(&conn.from_pin) && !is_exec_pin(&conn.to_pin)
            })
            .collect();

        if data_connections.is_empty() {
            return Ok(()); // 没有数据连接，无环
        }

        // 2. 构建邻接表（数据流图）
        let mut graph: HashMap<&String, Vec<&String>> = HashMap::new();
        let mut in_degree: HashMap<&String, usize> = HashMap::new();

        // 初始化所有节点的入度为0
        for node in node_names {
            in_degree.insert(node, 0);
            graph.insert(node, Vec::new());
        }

        // 构建图：from_node → to_node
        for conn in &data_connections {
            graph
                .entry(&conn.from_node)
                .or_default()
                .push(&conn.to_node);
            *in_degree.entry(&conn.to_node).or_insert(0) += 1;
        }

        // 3. Kahn算法：拓扑排序
        let mut queue: Vec<&String> = in_degree
            .iter()
            .filter(|(_, &degree)| degree == 0)
            .map(|(node, _)| *node)
            .collect();

        let mut sorted_count = 0;
        let mut visited: HashSet<&String> = HashSet::new();

        while let Some(node) = queue.pop() {
            if !visited.insert(node) {
                continue; // 已访问，跳过
            }
            sorted_count += 1;

            // 遍历当前节点的所有出边
            if let Some(neighbors) = graph.get(node) {
                for &neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(neighbor);
                        }
                    }
                }
            }
        }

        // 4. 检查是否所有节点都被排序
        if sorted_count < node_names.len() {
            // 存在环，找出环中的节点
            let cycle_nodes: Vec<String> = node_names
                .iter()
                .filter(|node| !visited.contains(node))
                .cloned()
                .collect();

            return Err(FrameworkError::CompileError(
                CompileError::circular_dependency(cycle_nodes),
            ));
        }

        Ok(())
    }
}

/// 判断引脚名是否是执行引脚
fn is_exec_pin(pin_name: &str) -> bool {
    matches!(
        pin_name,
        "In" | "Out"
            | "Then"
            | "Else"
            | "True"
            | "False"
            | "LoopBody"
            | "Completed"
            | "Break"
            | "Continue"
    ) || pin_name.starts_with("Then ")
        || pin_name.starts_with("Case ")
}

// 导出类型推导相关类型
pub use crate::workflow::type_inference::{BlueprintTypeInference, TypeConstraint};
