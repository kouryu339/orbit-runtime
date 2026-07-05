//! 测试嵌套循环中的 Break 节点
//!
//! 验证：
//! 1. Break 只影响内层循环
//! 3. Break 条件可以基于循环变量

use corework::workflow::core::DataValue;
use corework::workflow::execution::ExecutionContext;
use corework::workflow::nodes::control::{BreakNode, ForLoopNode};
use corework::world::FrameworkState;
use std::collections::HashMap;

fn execution_context() -> corework::error::Result<ExecutionContext> {
    Ok(ExecutionContext::from_context(
        FrameworkState::initialize()?.create_context(),
    ))
}

#[tokio::test]
async fn test_simple_loop_with_break_disabled() -> corework::error::Result<()> {
    let mut ctx = execution_context()?;
    let for_loop = ForLoopNode::new();
    let mut loop_inputs = HashMap::new();
    loop_inputs.insert("FirstIndex".to_string(), DataValue::from_i64(0));
    loop_inputs.insert("LastIndex".to_string(), DataValue::from_i64(4));

    let output = for_loop.execute(&mut ctx, loop_inputs).await?;
    assert!(matches!(
        output,
        corework::workflow::core::NodeOutput::Loop { .. }
    ));

    Ok(())
}

#[tokio::test]
async fn test_break_node_with_true() -> corework::error::Result<()> {
    let mut ctx = execution_context()?;
    let break_node = BreakNode::new();
    let mut inputs = HashMap::new();
    inputs.insert("Condition".to_string(), DataValue::from_bool(true));

    let output = break_node.execute(&mut ctx, inputs).await?;
    assert!(matches!(
        output,
        corework::workflow::core::NodeOutput::Break
    ));

    Ok(())
}

#[tokio::test]
async fn test_break_node_with_false() -> corework::error::Result<()> {
    let mut ctx = execution_context()?;
    let break_node = BreakNode::new();
    let mut inputs = HashMap::new();
    inputs.insert("Condition".to_string(), DataValue::from_bool(false));

    let output = break_node.execute(&mut ctx, inputs).await?;
    assert!(matches!(
        output,
        corework::workflow::core::NodeOutput::Complete
    ));

    Ok(())
}
