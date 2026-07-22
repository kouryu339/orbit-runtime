//! SetVarNode - 可变变量写节点
//!
//! Impure 节点：把 Value 写入当前 execution scope 的 workflow variable 槽。
//! 是实现循环累加、跨迭代状态传递的基础节点

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// SetVar 节点 - 可变变量写节点
///
/// 把 Value 写入以 Name 命名的声明变量槽。
#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Variable",
    display_name = "将变量{Name}设为{Value}",
    description = "将{{Value}}写入变量{{Name}}",
    permissions = 0,
    exec_in  = ["In@执行输入"],
    exec_out = ["Then@执行后继续"],
    data_in  = ["Name:String@变量槽名称", "Value:Any@要写入的值"],
    data_out = []
)]
pub struct SetVarNode;

impl BlueprintNode for SetVarNode {
    fn name(&self) -> &str {
        "SetVar"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("Name", "String"),
            Pin::data_in("Value", "Any"),
            Pin::exec_out("Then"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Writes a value to a named variable slot, enabling mutable state across loop iterations")
    }

    fn category(&self) -> Option<&str> {
        Some("Variable")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move { self.execute(ctx, inputs).await })
    }
}

impl SetVarNode {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let name = inputs.get("Name").and_then(|v| v.as_str()).unwrap_or("");

        let value = inputs
            .get("Value")
            .cloned()
            .unwrap_or_else(|| DataValue::from_string(""));

        ctx.set_workflow_variable(name, &value).await?;

        Ok(NodeOutput::ExecPin("Then".to_string()))
    }
}
