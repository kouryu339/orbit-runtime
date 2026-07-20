//! 状态机核心实现 - 基于函数回调
//!
//! 状态的进入、退出、执行都由函数实现，函数内部可自由调用工作流或其他逻辑

use super::transition::TransitionRecord;
use super::{State, StateEvent, StateMachineEventQueue};
use crate::cache::CacheExt;
use crate::error::{FrameworkError, Result};
use crate::event::BaseEvent;
use crate::execution_unit::{ExecutionUnit, UnitType};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// 状态机转换事件类型常量
// ============================================================================

/// 状态机进入某个状态时向全局 EventBus 发布的事件
pub const SM_STATE_ENTER: &str = "sm:state-enter";
/// 状态机离开某个状态时向全局 EventBus 发布的事件
pub const SM_STATE_EXIT: &str = "sm:state-exit";

/// 状态机 - 基于函数回调的状态管理
pub struct StateMachine {
    /// 状态机名称
    name: String,

    /// 所有状态（状态名 -> 状态对象）
    states: HashMap<String, Box<dyn State>>,

    /// 初始状态名称
    initial_state: String,

    /// 当前状态名称
    current_state: Arc<RwLock<String>>,

    unit: Arc<ExecutionUnit>,

    /// 转移历史
    history: Arc<RwLock<Vec<TransitionRecord>>>,

    /// 独立事件队列
    event_queue: Arc<StateMachineEventQueue>,
}

impl StateMachine {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 创建构建器
    pub fn builder(name: impl Into<String>) -> StateMachineBuilder {
        StateMachineBuilder::new(name)
    }

    /// 启动状态机（进入初始状态）
    pub async fn start(&self) -> Result<()> {
        let initial = self.initial_state.clone();

        // 写入 cache: current_state
        self.unit
            .cache()
            .set("current_state", &initial, None)
            .await?;

        // 执行初始状态的进入回调
        self.enter_state(&initial).await
    }

    /// 异步发送事件（**推荐在并发场景使用**）
    ///
    /// 与 [`send_event`](Self::send_event) 的根本区别：
    /// - `send_event`：**立即同步执行转移**，若当前状态的 `on_enter` 尚在 `await`（如 LLM/IO），
    ///   目标状态的 `on_enter` 会和源状态的 `on_enter` **并发运行**，造成 cache 写入污染
    ///   （例如 thinking 还在写 PENDING_TOOL_CALLS，suspended on_enter 又在清理它）。
    /// - `post_event`：**仅入队**，等 [`tick`](Self::tick) 或 [`process_events`](Self::process_events)
    ///   消费。此时当前状态函数已返回，enter/exit 顺序由 transition_to 串行保证，**无竞态**。
    ///
    /// 典型场景：外部任务（Tauri command、监控 task）收到用户暂停信号，向当前正在跑
    /// `on_enter`（比如 thinking 的 LLM 调用）的状态机派送 `PAUSE` 事件。
    ///
    /// 注意：`post_event` 不报错。若消费时当前状态无该事件的转移，**静默忽略**
    /// （区别于 `send_event` 的 `InvalidOperation` 硬报错），这对"对不确定状态广播暂停信号"
    /// 这种用例是必要语义。
    pub fn post_event(&self, event: impl Into<String>) {
        self.event_queue.send(StateEvent::new(event));
    }

    /// 发送事件，尝试触发状态转移（**立即同步**，慎用）
    ///
    /// 仅在已知当前状态稳定（非 `on_enter` 进行中）时使用。并发场景优先用
    /// [`post_event`](Self::post_event)。
    pub async fn send_event(&self, event: &str) -> Result<()> {
        let current = self.current_state.read().clone();

        // 获取当前状态
        let state = self.states.get(&current).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("当前状态 '{}' 不存在", current))
        })?;

        // 查找匹配的转移
        let transition = state.transitions().get(event).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "状态 '{}' 没有事件 '{}' 的转移",
                current, event
            ))
        })?;

        // 获取条件函数和目标状态（避免借用冲突）
        let has_condition = transition.condition().is_some();
        let target = transition.target_state().to_string();

        // 检查转移条件（如果有条件函数）
        if has_condition {
            let condition_fn = transition.condition().unwrap();
            let allowed = condition_fn(self.unit.clone()).await?;
            if !allowed {
                return Err(FrameworkError::InvalidOperation("转移条件不满足".into()));
            }
        }

        // 执行转移
        self.transition_to(event, &target).await
    }

    /// 获取当前状态名称
    pub fn current_state(&self) -> String {
        self.current_state.read().clone()
    }

    /// 获取历史记录
    pub fn history(&self) -> Vec<TransitionRecord> {
        self.history.read().clone()
    }

    /// 处理事件队列中的事件
    ///
    /// 两类事件来源：
    /// 1. `tick` 内部 `on_transition` 返回的自动转移（`target_state = Some(...)`）
    /// 2. 外部 [`post_event`](Self::post_event) 送入的具名事件（`target_state = None`，
    ///    需查当前状态的 transitions 表解析目标）
    ///
    /// 对第 2 类事件，若当前状态未登记该事件的转移，**静默忽略并记 debug 日志**，
    /// 不抛错。这是为了支持 "对不确定状态广播信号" 的语义（如 PAUSE 事件打给可能处于
    /// asking 的状态机——asking 不需要响应，直接忽略即可）。
    pub async fn process_events(&self) -> Result<()> {
        while let Some(event) = self.event_queue.receive() {
            // 第 1 优先级：事件自带 target_state（tick 内 on_transition 的自动转移路径）
            let target = if let Some(t) = event.target_state.clone() {
                Some(t)
            } else {
                // 第 2 路径：按 event_type 查当前状态的 transitions 表
                let current = self.current_state.read().clone();
                let resolved = self
                    .states
                    .get(&current)
                    .and_then(|s| s.transitions().get(&event.event_type))
                    .map(|t| (t.target_state().to_string(), t.condition().is_some()));

                match resolved {
                    Some((tgt, has_cond)) => {
                        if has_cond {
                            // 有条件转移：重新走 send_event 分支以复用条件判断代码路径
                            let cond_fn = self
                                .states
                                .get(&current)
                                .and_then(|s| s.transitions().get(&event.event_type))
                                .and_then(|t| t.condition());
                            if let Some(f) = cond_fn {
                                let allowed = f(self.unit.clone()).await?;
                                if !allowed {
                                    tracing::debug!(
                                        "process_events: 事件 '{}' 在状态 '{}' 条件不满足，忽略",
                                        event.event_type,
                                        current
                                    );
                                    continue;
                                }
                            }
                        }
                        Some(tgt)
                    }
                    None => {
                        tracing::debug!(
                            "process_events: 事件 '{}' 在状态 '{}' 无转移定义，静默忽略",
                            event.event_type,
                            current
                        );
                        None
                    }
                }
            };

            if let Some(target_state) = target {
                let current = self.current_state.read().clone();
                if target_state == current {
                    tracing::debug!("目标状态与当前状态相同，不执行转移");
                    continue;
                }
                self.transition_to(&event.event_type, &target_state).await?;
            }
        }
        Ok(())
    }

    /// 执行状态的 on_execute（tick 时调用）
    pub async fn tick(&self) -> Result<()> {
        let current = self.current_state.read().clone();

        let state = self.states.get(&current).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("当前状态 '{}' 不存在", current))
        })?;

        // 执行状态的执行回调
        if let Some(execute_fn) = state.on_execute() {
            execute_fn(self.unit.clone()).await?;
        }

        // 执行转移判断回调
        if let Some(transition_fn) = state.on_transition() {
            if let Some(target_state) = transition_fn(self.unit.clone()).await? {
                // 发送事件到队列
                self.event_queue
                    .send(StateEvent::new("auto_transition").with_target(target_state));
            }
        }

        // 处理事件队列
        self.process_events().await
    }

    /// 获取执行单元
    pub fn unit(&self) -> &Arc<ExecutionUnit> {
        &self.unit
    }

    /// 强制切换到指定状态（仅供错误恢复用，跳过 on_enter/on_exit 回调）
    ///
    /// **不执行任何回调**，仅更新内部状态标记和 cache。
    /// 调用方需确保业务状态一致（如写入 WAITING_FOR_INPUT 等）。
    ///
    /// 典型场景：tick 循环 on_enter 抛错后状态机卡死，需要强制拉回 asking。
    pub async fn force_state(&self, state_name: &str) -> Result<()> {
        if !self.states.contains_key(state_name) {
            return Err(FrameworkError::InvalidOperation(format!(
                "force_state: 状态 '{}' 不存在",
                state_name
            )));
        }
        tracing::warn!(
            "force_state: {} → {}",
            self.current_state.read().clone(),
            state_name
        );
        *self.current_state.write() = state_name.to_string();
        self.unit
            .cache()
            .set("current_state", &state_name.to_string(), None)
            .await?;
        Ok(())
    }

    /// 内部方法：进入状态（执行 on_enter 回调）
    /// Enter a recovered state and run its `on_enter` callback without emitting
    /// a normal transition from the previous process lifetime.
    pub async fn recover_enter_state(&self, state_name: &str) -> Result<()> {
        tracing::warn!(
            "recover_enter_state: {} -> {}",
            self.current_state.read().clone(),
            state_name
        );
        self.enter_state(state_name).await
    }

    async fn enter_state(&self, state_name: &str) -> Result<()> {
        // 获取进入回调
        let has_enter = {
            let state = self.states.get(state_name).ok_or_else(|| {
                FrameworkError::InvalidOperation(format!("状态 '{}' 不存在", state_name))
            })?;
            state.on_enter().is_some()
        };

        // 执行进入回调
        if has_enter {
            let state = self.states.get(state_name).unwrap();
            let enter_fn = state.on_enter().unwrap();
            enter_fn(self.unit.clone()).await?;
        }

        // 更新当前状态
        *self.current_state.write() = state_name.to_string();
        self.unit
            .cache()
            .set("current_state", &state_name.to_string(), None)
            .await?;

        Ok(())
    }

    /// 内部方法：退出状态（执行 on_exit 回调）
    async fn exit_state(&self, state_name: &str) -> Result<()> {
        // 获取退出回调
        let has_exit = {
            let state = self.states.get(state_name).ok_or_else(|| {
                FrameworkError::InvalidOperation(format!("状态 '{}' 不存在", state_name))
            })?;
            state.on_exit().is_some()
        };

        // 执行退出回调
        if has_exit {
            let state = self.states.get(state_name).unwrap();
            let exit_fn = state.on_exit().unwrap();
            exit_fn(self.unit.clone()).await?;
        }

        Ok(())
    }

    /// 内部方法：执行状态转移
    ///
    /// 转移顺序：
    /// 1. 先执行目标状态的 on_enter
    /// 2. 后执行当前状态的 on_exit
    async fn transition_to(&self, event: &str, target_state: &str) -> Result<()> {
        let from_state = self.current_state.read().clone();

        tracing::debug!(
            from_state = %from_state,
            event = %event,
            target_state = %target_state,
            "state transition"
        );

        let unit_id = self.unit.id().to_string();
        let bus = self.unit.event_bus();

        // 1. 先进入目标状态（on_enter）
        self.enter_state(target_state).await?;

        // 发布 sm:state-enter 事件（on_enter 完成后，新状态已激活）
        let enter_payload = serde_json::json!({
            "unit_id": unit_id,
            "state": target_state,
            "from": from_state,
        });
        let _ = bus
            .publish(BaseEvent::new(SM_STATE_ENTER, enter_payload))
            .await;

        // 2. 后退出当前状态（on_exit）
        self.exit_state(&from_state).await?;

        // 发布 sm:state-exit 事件（on_exit 完成后，旧状态已离开）
        let exit_payload = serde_json::json!({
            "unit_id": unit_id,
            "state": from_state,
            "to": target_state,
        });
        let _ = bus
            .publish(BaseEvent::new(SM_STATE_EXIT, exit_payload))
            .await;

        // 3. 记录历史
        self.history.write().push(TransitionRecord {
            from_state,
            to_state: target_state.to_string(),
            event: event.to_string(),
            timestamp: chrono::Utc::now(),
        });

        Ok(())
    }
}

/// 状态机构建器
pub struct StateMachineBuilder {
    name: String,
    states: Vec<Box<dyn State>>,
    initial_state: Option<String>,
    framework: Option<crate::world::FrameworkState>,
    /// Real parent execution unit for nested state machines.
    parent_unit: Option<Arc<ExecutionUnit>>,
}

impl StateMachineBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            states: Vec::new(),
            initial_state: None,
            framework: None,
            parent_unit: None,
        }
    }

    /// 添加状态
    pub fn add_state(mut self, state: Box<dyn State>) -> Self {
        self.states.push(state);
        self
    }

    /// 设置初始状态
    pub fn initial_state(mut self, state_name: impl Into<String>) -> Self {
        self.initial_state = Some(state_name.into());
        self
    }

    /// 使用自定义 FrameworkState
    pub fn with_framework(mut self, framework: crate::world::FrameworkState) -> Self {
        self.framework = Some(framework);
        self
    }

    pub fn with_parent_unit(mut self, parent_unit: Arc<ExecutionUnit>) -> Self {
        self.parent_unit = Some(parent_unit);
        self
    }

    /// 构建状态机
    pub async fn build(self) -> Result<StateMachine> {
        use crate::world::FrameworkState;

        let initial_state = self
            .initial_state
            .ok_or_else(|| FrameworkError::InvalidOperation("未设置初始状态".into()))?;

        // 验证：初始状态必须存在
        if !self.states.iter().any(|s| s.name() == initial_state) {
            return Err(FrameworkError::InvalidOperation(format!(
                "初始状态 '{}' 不存在",
                initial_state
            )));
        }

        // 初始化框架
        let framework = if let Some(fw) = self.framework {
            fw
        } else {
            FrameworkState::initialize()?
        };

        // 创建执行单元
        let unit = match self.parent_unit {
            Some(parent) => Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &parent)?),
            None => Arc::new(ExecutionUnit::new_root(UnitType::StateMachine, framework)),
        };

        let current_state = Arc::new(RwLock::new(initial_state.clone()));

        Ok(StateMachine {
            name: self.name,
            states: self
                .states
                .into_iter()
                .map(|s| (s.name().to_string(), s))
                .collect(),
            initial_state,
            current_state,
            unit,
            history: Arc::new(RwLock::new(Vec::new())),
            event_queue: Arc::new(StateMachineEventQueue::new()),
        })
    }
}
