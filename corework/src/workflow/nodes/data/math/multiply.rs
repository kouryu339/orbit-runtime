//! Multiply 节点 - 乘法运算
//!
//! 计算 A * B

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// 乘法节点 - 计算 A * B
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Math",
    display_name = "{A}×{B}",
    description = "→{{Product}} = {{A}} * {{B}}",
    permissions = 0,
    data_in = ["A:f64@第一个乘数", "B:f64@第二个乘数"],
    data_out = ["Product:f64@相乘结果"]
)]
pub struct MultiplyNode;

impl Default for MultiplyNode {
    fn default() -> Self {
        Self
    }
}

impl BlueprintNode for MultiplyNode {
    fn name(&self) -> &str {
        "Multiply"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "f64"),
            Pin::data_in("B", "f64"),
            Pin::data_out("Product", "f64"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Multiplies two numbers: Result = A * B")
    }

    fn category(&self) -> Option<&str> {
        Some("Math")
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

impl MultiplyNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input A".into())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input B".into())
        })?;

        let result = a * b;
        let mut outputs = HashMap::new();
        outputs.insert("Product".to_string(), DataValue::from_f64(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiply_node() {
        let node = MultiplyNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(5.0));
        inputs.insert("B".to_string(), DataValue::from_f64(6.0));
        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("Product").unwrap().as_f64().unwrap();
        assert_eq!(result, 30.0);
    }
}
