//! Greater 节点 - 大于比较
//!
//! 比较 A > B

use std::collections::HashMap;
use crate::workflow::core::{Pin, DataValue};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use crate::error::Result;
use crate::register_node;

use crate::workflow::registry as __NodeRegistry;

/// 大于比较节点 - 判断 A > B
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Math",
    display_name = "Greater",
    description = "→{{IsGreater}} = {{A}} > {{B}}",
    permissions = 0,
    data_in = ["A:f64@左操作数", "B:f64@右操作数"],
    data_out = ["IsGreater:bool@A是否大于B"]
)]
pub struct GreaterNode;

impl Default for GreaterNode {
    fn default() -> Self {
        Self
    }
}

impl BlueprintNode for GreaterNode {
    fn name(&self) -> &str {
        "Greater"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "f64"),
            Pin::data_in("B", "f64"),
            Pin::data_out("IsGreater", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Compares two numbers: Result = (A > B)")
    }

    fn category(&self) -> Option<&str> {
        Some("Math/Comparison")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<crate::workflow::core::NodeOutput>> + Send + 'a>> {
        self.__execute_node_impl(ctx, inputs)
    }
}

impl GreaterNode {
    pub fn evaluate(&self, inputs: HashMap<String, DataValue>) -> Result<HashMap<String, DataValue>> {
        let a = inputs.get("A")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::FrameworkError::SystemError("Missing or invalid input A".into()))?;
        
        let b = inputs.get("B")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::FrameworkError::SystemError("Missing or invalid input B".into()))?;

        let result = a > b;
        let mut outputs = HashMap::new();
        outputs.insert("IsGreater".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greater_node() {
        let node = GreaterNode::default();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(10.0));
        inputs.insert("B".to_string(), DataValue::from_f64(5.0));
        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("IsGreater").unwrap().as_bool().unwrap();
        assert!(result);
    }
}
