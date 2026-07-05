//! 执行流 - 管理节点间的执行顺序
//!
//! 跟踪当前执行位置，防止无限循环

/// 执行流 - 管理执行路径
#[derive(Debug, Clone)]
pub struct ExecutionFlow {
    /// 当前节点
    current_node: Option<String>,

    /// 当前执行引脚
    current_exec_pin: Option<String>,

    /// 已访问的节点（用于循环检测）
    visited_nodes: Vec<String>,

    /// 最大执行步数（防止无限循环）
    max_steps: usize,

    /// 当前步数
    current_step: usize,
}

impl ExecutionFlow {
    /// 创建新的执行流
    pub fn new() -> Self {
        Self {
            current_node: None,
            current_exec_pin: None,
            visited_nodes: Vec::new(),
            max_steps: 10000, // 默认最多执行 10000 步
            current_step: 0,
        }
    }

    /// 设置最大步数
    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// 移动到下一个节点
    pub fn move_to(&mut self, node: String, exec_pin: String) {
        self.visited_nodes.push(node.clone());
        self.current_node = Some(node);
        self.current_exec_pin = Some(exec_pin);
        self.current_step += 1;
    }

    /// 获取当前节点
    pub fn current_node(&self) -> Option<&String> {
        self.current_node.as_ref()
    }

    /// 获取当前执行引脚
    pub fn current_exec_pin(&self) -> Option<&String> {
        self.current_exec_pin.as_ref()
    }

    /// 检查是否达到最大步数
    pub fn is_max_steps_reached(&self) -> bool {
        self.current_step >= self.max_steps
    }

    /// 获取当前步数
    pub fn current_step(&self) -> usize {
        self.current_step
    }

    /// 检查节点是否已访问（简单的循环检测）
    pub fn has_visited(&self, node: &str) -> bool {
        self.visited_nodes.contains(&node.to_string())
    }

    /// 获取访问历史
    pub fn visited_nodes(&self) -> &[String] {
        &self.visited_nodes
    }

    /// 重置执行流
    pub fn reset(&mut self) {
        self.current_node = None;
        self.current_exec_pin = None;
        self.visited_nodes.clear();
        self.current_step = 0;
    }
}

impl Default for ExecutionFlow {
    fn default() -> Self {
        Self::new()
    }
}
