//!
//!
//! ## 设计理念
//!
//! 框架提供抽象（State trait、Transition trait），业务实现具体逻辑。
//! 这样设计的优点：
//! - 清晰的职责分离：框架管理状态转移，业务管理状态行为
//!
//! ## 示例
//!
//! ```rust,ignore
//! use corework::statemachine::FnState;
//!
//! // 用函数回调定义状态行为（简单场景直接用闭包，复杂场景在闭包内调用工作流）
//! let pending = FnState::new("待支付")
//!     .with_on_enter(|ctx| Box::pin(async move {
//!         ctx.cache.set("status", &"验证中").await?;
//!         Ok(())
//!     }))
//!     .with_on_execute(|ctx| Box::pin(async move {
//!         Ok(())
//!     }))
//!     .add_transition("pay", Box::new(SimpleTransition::new("pay", "已支付")));
//!
//! let sm = StateMachine::builder("订单")
//!     .add_state(Box::new(pending))
//!     .initial_state("待支付")
//!     .build()
//!     .await?;
//! ```

pub mod event;
pub mod state;
pub mod statemachine;
pub mod transition;

// 重新导出核心类型
pub use event::{StateEvent, StateMachineEventQueue};
pub use state::{ConditionFn, StateFn, TransitionFn};
pub use state::{ConditionalTransition, FnState, SimpleTransition, State, Transition};
pub use statemachine::{StateMachine, StateMachineBuilder, SM_STATE_ENTER, SM_STATE_EXIT};
pub use transition::TransitionRecord;
