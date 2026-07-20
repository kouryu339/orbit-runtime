//! DebugPrint 节点 - 调试打印节点
//!
//! 打印值到调试输出并将其传递下去

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// 调试打印节点 - 打印值并将其传递下去
/// 用于调试数据流（模拟Print节点功能）
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Placeholder",
    display_name = "打印{Value}",
    description = "打印{{Value}}到调试输出，→{{Value}}传递",
    permissions = 0,
    exec_in = ["In"],
    exec_out = ["Out"],
    data_in = ["Value:Any@需要打印的值"],
    data_out = ["Value:Any@传递的值（同输入）"]
)]
#[derive(Default)]
pub struct DebugPrintNode {
    /// 可选的标签，用于标识打印输出
    pub label: Option<String>,
}

impl BlueprintNode for DebugPrintNode {
    fn name(&self) -> &str {
        "DebugPrint"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("Value", "Any"),
            Pin::exec_out("Out"),
            Pin::data_out("Value", "Any"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Prints input value and passes it through (for debugging)")
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

impl DebugPrintNode {
    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
        }
    }

    pub async fn execute(
        &self,
        _ctx: &mut crate::workflow::execution::ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<crate::workflow::core::NodeOutput> {
        let value = inputs
            .get("Value")
            .cloned()
            .unwrap_or_else(|| DataValue::new("null", serde_json::Value::Null));

        let label_str = self
            .label
            .as_ref()
            .map(|l| format!("[{}] ", l))
            .unwrap_or_default();

        tracing::info!("{}DebugPrint: {:?}", label_str, value);

        Ok(crate::workflow::core::NodeOutput::ExecPin(
            "Out".to_string(),
        ))
    }
}
