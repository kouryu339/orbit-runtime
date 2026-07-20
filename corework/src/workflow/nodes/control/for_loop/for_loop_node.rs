//! ForLoop 节点 - 固定次数循环
//!
//! 从 FirstIndex 到 LastIndex 循环执行

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, LoopIteration, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// ForLoop 节点 - 固定次数循环
///
/// 从 FirstIndex 到 LastIndex（包含）循环执行 LoopBody
/// 每次迭代输出当前的 Index 值
/// 所有迭代完成后执行 Completed 分支
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Control Flow",
    display_name = "从{FirstIndex}到{LastIndex}循环，当前索引为{Index}",
    description = "从{{FirstIndex}}到{{LastIndex}}循环，每次→{{LoopBody}}输出→{{Index}}，完成后→{{Completed}}",
    permissions = 0,
    exec_in = ["In@启动循环"],
    exec_out = ["LoopBody@每次迭代时触发，可从 Index 获取当前索引", "Completed@所有迭代完成后触发"],
    data_in = ["FirstIndex:i64@起始索引（含）", "LastIndex:i64@结束索引（含）"],
    data_out = ["Index:i64@当前循环索引"]
)]
pub struct ForLoopNode;

impl Default for ForLoopNode {
    fn default() -> Self {
        Self
    }
}

impl ForLoopNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for ForLoopNode {
    fn name(&self) -> &str {
        "ForLoop"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("FirstIndex", "i64"),
            Pin::data_in("LastIndex", "i64"),
            Pin::exec_out("LoopBody"),
            Pin::data_out("Index", "i64"),
            Pin::exec_out("Completed"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Executes loop body for each index from FirstIndex to LastIndex")
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

impl ForLoopNode {
    pub async fn execute(
        &self,
        _ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let first = inputs
            .get("FirstIndex")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let last = inputs
            .get("LastIndex")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "ForLoop: Missing LastIndex input".to_string(),
                )
            })?;

        tracing::debug!("🔁 [ForLoop] 循环范围: {} 到 {}", first, last);

        // 检查是否为有效范围
        if first > last {
            tracing::debug!("⚠️  [ForLoop] 起始索引大于结束索引，不执行循环");
            // 范围无效，直接跳到 Completed
            return Ok(NodeOutput::ExecPin("Completed".to_string()));
        }

        // 生成所有迭代的数据
        let iterations: Vec<LoopIteration> = (first..=last)
            .map(|i| {
                let mut outputs = HashMap::new();
                outputs.insert("Index".to_string(), DataValue::from_i64(i));
                LoopIteration { outputs }
            })
            .collect();

        tracing::debug!("🔁 [ForLoop] 准备执行 {} 次迭代", iterations.len());

        Ok(NodeOutput::Loop {
            body_pin: "LoopBody".to_string(),
            completed_pin: "Completed".to_string(),
            iterations,
        })
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
    async fn test_for_loop_basic() {
        let node = ForLoopNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("FirstIndex".to_string(), DataValue::from_i64(0));
        inputs.insert("LastIndex".to_string(), DataValue::from_i64(2));

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        match output {
            NodeOutput::Loop {
                body_pin,
                completed_pin,
                iterations,
            } => {
                assert_eq!(body_pin, "LoopBody");
                assert_eq!(completed_pin, "Completed");
                assert_eq!(iterations.len(), 3);

                // 检查第一次迭代
                let first_iter = &iterations[0];
                assert_eq!(first_iter.outputs.get("Index").unwrap().as_i64(), Some(0));

                // 检查最后一次迭代
                let last_iter = &iterations[2];
                assert_eq!(last_iter.outputs.get("Index").unwrap().as_i64(), Some(2));
            }
            _ => panic!("Expected Loop output"),
        }
    }

    #[tokio::test]
    async fn test_for_loop_single_iteration() {
        let node = ForLoopNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("FirstIndex".to_string(), DataValue::from_i64(5));
        inputs.insert("LastIndex".to_string(), DataValue::from_i64(5));

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        match output {
            NodeOutput::Loop { iterations, .. } => {
                assert_eq!(iterations.len(), 1);
                assert_eq!(
                    iterations[0].outputs.get("Index").unwrap().as_i64(),
                    Some(5)
                );
            }
            _ => panic!("Expected Loop output"),
        }
    }

    #[tokio::test]
    async fn test_for_loop_invalid_range() {
        let node = ForLoopNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("FirstIndex".to_string(), DataValue::from_i64(5));
        inputs.insert("LastIndex".to_string(), DataValue::from_i64(2));

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        // 无效范围应该直接跳到 Completed
        match output {
            NodeOutput::ExecPin(pin) => assert_eq!(pin, "Completed"),
            _ => panic!("Expected ExecPin(Completed) for invalid range"),
        }
    }

    #[tokio::test]
    async fn test_for_loop_large_range() {
        let node = ForLoopNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("FirstIndex".to_string(), DataValue::from_i64(10));
        inputs.insert("LastIndex".to_string(), DataValue::from_i64(20));

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        match output {
            NodeOutput::Loop { iterations, .. } => {
                assert_eq!(iterations.len(), 11); // 10 to 20 inclusive
            }
            _ => panic!("Expected Loop output"),
        }
    }
}
