//! 执行引擎模块
//!
//! 管理蓝图执行流程、栈帧、节点状态

pub mod execution_context;
pub mod execution_flow;
pub mod executor;
pub mod node_state;
pub mod stack_frame;
pub mod trace;

// 重新导出
pub use execution_context::{ExecutionContext, WorkflowTraceEntry};
pub use execution_flow::ExecutionFlow;
pub use executor::BlueprintExecutor;
pub use node_state::NodeState;
pub use stack_frame::{FrameType, StackFrame};
pub use trace::{
    WorkflowExecutionReport, WorkflowExecutionTrace, WorkflowNodeStatus, WorkflowNodeTrace,
    WorkflowSourceRef, WorkflowToAiMode, WorkflowTraceRecorder,
};
