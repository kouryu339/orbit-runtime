//! Branch 节点 - 条件分支
//!
//! 根据条件执行不同的分支

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Branch 节点 - 条件分支
///
/// 根据 Condition 输入，执行 True 或 False 分支
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Control Flow",
    display_name = "Branch",
    description = "{{Condition}}为真走→{{True}}，否则走→{{False}}",
    permissions = 0,
    exec_in = ["In@接收执行流"],
    exec_out = ["True@条件为真时走此路径", "False@条件为假时走此路径"],
    data_in = ["Condition:bool@布尔条件表达式"]
)]
pub struct BranchNode;

impl Default for BranchNode {
    fn default() -> Self {
        Self
    }
}

impl BlueprintNode for BranchNode {
    fn name(&self) -> &str {
        "Branch"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("Condition", "bool"),
            Pin::exec_out("True"),
            Pin::exec_out("False"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Executes different branches based on condition")
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

impl BranchNode {
    pub async fn execute(
        &self,
        _ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let condition = inputs
            .get("Condition")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if condition {
            Ok(NodeOutput::ExecPin("True".to_string()))
        } else {
            Ok(NodeOutput::ExecPin("False".to_string()))
        }
    }
}
