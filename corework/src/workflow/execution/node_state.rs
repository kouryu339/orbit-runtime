//! 节点状态 - 保存需要跨调用保持的状态
//!
//! 与栈帧不同，节点状态在多次调用间持久化

/// 节点状态 - 跨调用持久化的状态
///
/// 用途：
/// - 循环计数器（ForLoop 的当前索引）
/// - DoOnce 的执行标记
/// - Delay 的开始时间
#[derive(Debug, Clone)]
pub enum NodeState {
    /// 循环状态
    Loop {
        current_index: usize,
        last_index: usize,
        iteration_count: usize,
    },

    /// DoOnce 状态
    DoOnce { has_executed: bool },

    /// DoN 状态
    DoN {
        execution_count: usize,
        max_count: usize,
    },

    /// Delay 状态
    Delay {
        start_time: std::time::Instant,
        duration: std::time::Duration,
    },

    /// 自定义状态（用 JSON 存储）
    Custom(serde_json::Value),
}

impl NodeState {
    /// 创建循环状态
    pub fn new_loop(current_index: usize, last_index: usize) -> Self {
        Self::Loop {
            current_index,
            last_index,
            iteration_count: 0,
        }
    }

    /// 创建 DoOnce 状态
    pub fn new_do_once() -> Self {
        Self::DoOnce {
            has_executed: false,
        }
    }

    /// 创建 DoN 状态
    pub fn new_do_n(max_count: usize) -> Self {
        Self::DoN {
            execution_count: 0,
            max_count,
        }
    }

    /// 创建 Delay 状态
    pub fn new_delay(duration: std::time::Duration) -> Self {
        Self::Delay {
            start_time: std::time::Instant::now(),
            duration,
        }
    }
}
