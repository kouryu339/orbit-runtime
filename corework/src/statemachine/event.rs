use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// 状态事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEvent {
    /// 事件类型（如 "timer", "user_action"）
    pub event_type: String,

    /// 目标状态（由工作流返回）
    pub target_state: Option<String>,

    /// 事件携带的数据
    pub data: HashMap<String, serde_json::Value>,
}

impl StateEvent {
    pub fn new(event_type: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            target_state: None,
            data: HashMap::new(),
        }
    }

    pub fn with_target(mut self, target_state: impl Into<String>) -> Self {
        self.target_state = Some(target_state.into());
        self
    }

    pub fn with_data(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.data.insert(key.into(), value);
        self
    }
}

/// 状态机独立事件队列
pub struct StateMachineEventQueue {
    queue: Arc<RwLock<VecDeque<StateEvent>>>,
}

impl StateMachineEventQueue {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// 发送事件
    pub fn send(&self, event: StateEvent) {
        self.queue.write().push_back(event);
    }

    /// 接收事件（非阻塞）
    pub fn receive(&self) -> Option<StateEvent> {
        self.queue.write().pop_front()
    }

    /// 查看队列长度
    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.queue.read().is_empty()
    }

    /// 清空队列
    pub fn clear(&self) {
        self.queue.write().clear();
    }
}

impl Default for StateMachineEventQueue {
    fn default() -> Self {
        Self::new()
    }
}
