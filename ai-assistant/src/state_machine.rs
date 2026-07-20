//! AI助手状态机 —— 组装器
//! 各状态的业务逻辑在 `state/` 目录下各自实现。
//! 本模块仅将它们组装为一个 `StateMachineBuilder`。

use crate::state;
use corework::statemachine::StateMachineBuilder;

// 重新导出，方便外部使用
pub use crate::state::{agent_keys, events, states};

/// 所有持久 Agent 与默认 Agent 共用同一套核心状态：
/// `suspended / thinking / executing / saying`。
pub fn build_agent_state_machine_with_initial(initial: &str) -> StateMachineBuilder {
    corework::statemachine::StateMachine::builder("agent")
        .add_state(Box::new(state::suspended::build()))
        .add_state(Box::new(state::saying::build()))
        .add_state(Box::new(state::thinking::build()))
        .add_state(Box::new(state::executing::build()))
        .initial_state(initial)
}

/// 构建默认 AI 助手状态机。
/// 兼容旧入口，初始状态仍为 `suspended`。
pub fn build_assistant_state_machine() -> StateMachineBuilder {
    build_agent_state_machine_with_initial(states::SUSPENDED)
}

/// 构建临时/子 Agent 状态机。
/// 兼容旧调用，临时 OneShot 创建时已经注入 intent，所以从 `thinking` 开始。
pub fn build_agent_state_machine() -> StateMachineBuilder {
    build_agent_state_machine_with_initial(states::THINKING)
}
