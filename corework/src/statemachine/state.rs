//! 状态定义 - 基于函数回调的设计
//!
//! 状态的进入、退出、执行、转移判断都由函数实现

use crate::error::Result;
use crate::execution_unit::ExecutionUnit;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ==================== 函数类型定义 ====================

/// 状态处理函数（进入/退出/执行回调）
/// 直接接收 Arc<ExecutionUnit>，可访问 cache()、world()、get_resource() 等全部能力
pub type StateFn = Box<
    dyn Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

/// 条件判断函数（转移条件检查）
pub type ConditionFn = Box<
    dyn Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> + Send + Sync,
>;

/// 转移判断函数（返回目标状态名，None 表示不转移）
pub type TransitionFn = Box<
    dyn Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send>>
        + Send
        + Sync,
>;

// ==================== 核心 Trait ====================

/// 状态 trait - 框架层抽象
///
/// 每个状态通过函数回调定义行为，函数内部可自由调用工作流或其他逻辑。
pub trait State: Send + Sync {
    /// 状态名称（唯一标识）
    fn name(&self) -> &str;

    /// 显示名称
    fn display_name(&self) -> &str {
        self.name()
    }

    /// 描述
    fn description(&self) -> Option<&str> {
        None
    }

    /// 是否为终止状态
    fn is_terminal(&self) -> bool {
        false
    }

    /// 进入回调（状态进入时执行）
    fn on_enter(&self) -> Option<&StateFn> {
        None
    }

    /// 退出回调（状态退出时执行）
    fn on_exit(&self) -> Option<&StateFn> {
        None
    }

    /// 执行回调（tick 时调用，可循环执行）
    fn on_execute(&self) -> Option<&StateFn> {
        None
    }

    /// 转移判断回调（tick 后调用，返回目标状态名触发自动转移）
    fn on_transition(&self) -> Option<&TransitionFn> {
        None
    }

    /// 获取该状态的所有转移（事件名 -> 转移）
    fn transitions(&self) -> &HashMap<String, Box<dyn Transition>>;
}

/// 转移 trait - 框架层抽象
pub trait Transition: Send + Sync {
    /// 事件名称
    fn event(&self) -> &str;

    /// 目标状态名称
    fn target_state(&self) -> &str;

    /// 条件检查函数（可选，返回 true 允许转移）
    fn condition(&self) -> Option<&ConditionFn> {
        None
    }
}

// ==================== 函数状态实现（Builder 风格）====================

/// 函数状态实现 - 通过闭包定义行为
///
/// 简单场景直接用闭包，复杂场景在闭包内调用工作流。
///
/// ```rust,ignore
/// use corework::statemachine::FnState;
///
/// let state = FnState::new("thinking")
///     .with_on_enter(|ctx| Box::pin(async move {
///         ctx.cache.set("entered", &true).await?;
///         Ok(())
///     }))
///     .with_on_execute(|ctx| Box::pin(async move {
///         Ok(())
///     }));
/// ```
pub struct FnState {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub is_terminal: bool,

    /// 进入回调
    pub on_enter_fn: Option<StateFn>,
    /// 退出回调
    pub on_exit_fn: Option<StateFn>,
    /// 执行回调
    pub on_execute_fn: Option<StateFn>,
    /// 转移判断回调
    pub on_transition_fn: Option<TransitionFn>,

    pub transitions: HashMap<String, Box<dyn Transition>>,
}

impl FnState {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            display_name: name.clone(),
            name,
            description: None,
            is_terminal: false,
            on_enter_fn: None,
            on_exit_fn: None,
            on_execute_fn: None,
            on_transition_fn: None,
            transitions: HashMap::new(),
        }
    }

    pub fn terminal(name: impl Into<String>) -> Self {
        let mut s = Self::new(name);
        s.is_terminal = true;
        s
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// 设置进入回调
    pub fn with_on_enter(
        mut self,
        f: impl Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.on_enter_fn = Some(Box::new(f));
        self
    }

    /// 设置退出回调
    pub fn with_on_exit(
        mut self,
        f: impl Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.on_exit_fn = Some(Box::new(f));
        self
    }

    /// 设置执行回调（tick 时调用）
    pub fn with_on_execute(
        mut self,
        f: impl Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.on_execute_fn = Some(Box::new(f));
        self
    }

    /// 设置转移判断回调（tick 后调用）
    pub fn with_on_transition(
        mut self,
        f: impl Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.on_transition_fn = Some(Box::new(f));
        self
    }

    /// 添加事件转移
    pub fn add_transition(
        mut self,
        event: impl Into<String>,
        transition: Box<dyn Transition>,
    ) -> Self {
        self.transitions.insert(event.into(), transition);
        self
    }
}

impl State for FnState {
    fn name(&self) -> &str {
        &self.name
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn is_terminal(&self) -> bool {
        self.is_terminal
    }

    fn on_enter(&self) -> Option<&StateFn> {
        self.on_enter_fn.as_ref()
    }

    fn on_exit(&self) -> Option<&StateFn> {
        self.on_exit_fn.as_ref()
    }

    fn on_execute(&self) -> Option<&StateFn> {
        self.on_execute_fn.as_ref()
    }

    fn on_transition(&self) -> Option<&TransitionFn> {
        self.on_transition_fn.as_ref()
    }

    fn transitions(&self) -> &HashMap<String, Box<dyn Transition>> {
        &self.transitions
    }
}

/// 简单转移实现（无条件转移）
pub struct SimpleTransition {
    pub event: String,
    pub target_state: String,
}

impl SimpleTransition {
    pub fn new(event: impl Into<String>, target_state: impl Into<String>) -> Self {
        Self {
            event: event.into(),
            target_state: target_state.into(),
        }
    }
}

impl Transition for SimpleTransition {
    fn event(&self) -> &str {
        &self.event
    }

    fn target_state(&self) -> &str {
        &self.target_state
    }
}

/// 带条件的转移实现
pub struct ConditionalTransition {
    pub event: String,
    pub target_state: String,
    pub condition_fn: ConditionFn,
}

impl ConditionalTransition {
    pub fn new(
        event: impl Into<String>,
        target_state: impl Into<String>,
        condition: impl Fn(Arc<ExecutionUnit>) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            event: event.into(),
            target_state: target_state.into(),
            condition_fn: Box::new(condition),
        }
    }
}

impl Transition for ConditionalTransition {
    fn event(&self) -> &str {
        &self.event
    }

    fn target_state(&self) -> &str {
        &self.target_state
    }

    fn condition(&self) -> Option<&ConditionFn> {
        Some(&self.condition_fn)
    }
}
