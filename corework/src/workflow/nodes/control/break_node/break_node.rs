//! Break 节点 - 提前退出循环
//!
//! 模拟 UE 的 Break 节点，通过返回 NodeOutput::Break 信号退出循环

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Break 节点 - 提前退出循环
///
/// 当 Condition 为 true 时，返回 Break 信号，通知执行器停止当前循环。
/// 配合 ForLoop 使用，可以实现类似 WhileLoop 的效果。
///
/// # 设计原理
///
/// UE 蓝图中的 Break：
/// - 使用字节码 JUMP 指令直接跳到循环结束
/// - 无需状态标记，编译时绑定跳转地址
/// - 自动处理嵌套循环（每个循环有自己的 LOOP_END）
///
/// 我们的实现：
/// - 返回 NodeOutput::Break 作为特殊信号
/// - 执行器捕获并停止当前循环迭代
/// - 通过返回值传播自动处理嵌套（内层 Break 不影响外层）
///
/// # 示例
///
/// ```text
/// ForLoop(0-99)
///   → Print(Index)
///   → Break(Index >= 10)
///   
/// 效果：只打印 0-9，到 10 时退出循环
/// ```
///
/// 嵌套循环：
/// ```text
/// ForLoop A(0-3)
///   → ForLoop B(0-3)
///       → Break(B.Index > 1)  ← 只退出 B，不退出 A
///   → Print("A={A.Index}")
///   
/// 输出：A=0, A=1, A=2, A=3（B 每次都在 Index=2 时退出）
/// ```
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Control Flow",
    display_name = "Break",
    description = "{{Condition}}为真时中断当前循环",
    permissions = 0,
    exec_in = ["In@执行输入"],
    data_in = ["Condition:bool@是否中断循环"]
)]
pub struct BreakNode;

impl Default for BreakNode {
    fn default() -> Self {
        Self
    }
}

impl BreakNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for BreakNode {
    fn name(&self) -> &str {
        "Break"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("Condition", "bool"),
            // ⚠️ 注意：Break 节点没有 exec_out
            // 这是因为 Break 直接中断执行流，不会继续往下执行
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Break out of loop when condition is true (UE style)")
    }

    fn category(&self) -> Option<&str> {
        Some("Control Flow")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::workflow::core::NodeOutput>> + Send + 'a,
        >,
    > {
        self.__execute_node_impl(ctx, inputs)
    }
}

impl BreakNode {
    pub async fn execute(
        &self,
        _ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let condition = inputs
            .get("Condition")
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // 默认为 true

        if condition {
            tracing::debug!("   🛑 [Break] Condition=true, 发出 Break 信号");
            Ok(NodeOutput::Break)
        } else {
            tracing::debug!("   ➡️  [Break] Condition=false, 继续执行");
            // 条件不满足，相当于空节点
            Ok(NodeOutput::Complete)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::FrameworkState;

    fn execution_context() -> crate::error::Result<ExecutionContext> {
        Ok(ExecutionContext::from_context(
            FrameworkState::initialize()?.create_context(),
        ))
    }

    #[tokio::test]
    async fn test_break_with_true() {
        let node = BreakNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Condition".to_string(), DataValue::from_bool(true));

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        assert!(matches!(output, NodeOutput::Break));
    }

    #[tokio::test]
    async fn test_break_with_false() {
        let node = BreakNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Condition".to_string(), DataValue::from_bool(false));

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        assert!(matches!(output, NodeOutput::Complete));
    }

    #[tokio::test]
    async fn test_break_default_true() {
        let node = BreakNode::new();
        let inputs = HashMap::new(); // 没有提供 Condition

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        assert!(matches!(output, NodeOutput::Break));
    }
}
