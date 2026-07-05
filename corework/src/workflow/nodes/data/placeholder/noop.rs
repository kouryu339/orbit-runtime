//! NoOp 节点 - 空操作节点
//!
//! 什么都不做，只进行流控制

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// 空操作节点 - 什么都不做，只进行流控制
/// 用于占位和测试流程
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Placeholder",
    display_name = "NoOp",
    description = "空操作，{{In}}直通→{{Out}}",
    permissions = 0,
    exec_in = ["In@执行输入"],
    exec_out = ["Out@执行输出"]
)]
pub struct NoOpNode;

impl Default for NoOpNode {
    fn default() -> Self {
        Self
    }
}

impl NoOpNode {
    pub async fn execute(
        &self,
        _ctx: &mut crate::workflow::execution::ExecutionContext,
        _inputs: HashMap<String, DataValue>,
    ) -> Result<crate::workflow::core::NodeOutput> {
        Ok(crate::workflow::core::NodeOutput::ExecPin(
            "Out".to_string(),
        ))
    }
}

impl BlueprintNode for NoOpNode {
    fn name(&self) -> &str {
        "NoOp"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![Pin::exec_in("In"), Pin::exec_out("Out")]
    }

    fn description(&self) -> Option<&str> {
        Some("No operation - placeholder for flow control")
    }

    fn category(&self) -> Option<&str> {
        Some("Placeholder")
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
