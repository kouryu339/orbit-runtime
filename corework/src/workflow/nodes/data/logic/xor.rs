//! Xor 节点 - 逻辑异或

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Xor 节点 - 逻辑异或
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Logic",
    display_name = "{A}与{B}异或",
    description = "→{{IsXor}} = {{A}} XOR {{B}}",
    permissions = 0,
    data_in = ["A:bool@左操作数", "B:bool@右操作数"],
    data_out = ["IsXor:bool@异或结果"]
)]
pub struct XorNode;

impl Default for XorNode {
    fn default() -> Self {
        Self
    }
}

impl XorNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for XorNode {
    fn name(&self) -> &str {
        "Xor"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "bool"),
            Pin::data_in("B", "bool"),
            Pin::data_out("IsXor", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Logic XOR: returns true if A and B are different")
    }

    fn category(&self) -> Option<&str> {
        Some("Logic")
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

impl XorNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_bool()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input A".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_bool()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input B".to_string())
        })?;

        let result = a ^ b;

        let mut outputs = HashMap::new();
        outputs.insert("IsXor".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_xor_true_false() {
        let node = XorNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_bool(true));
        inputs.insert("B".to_string(), DataValue::from_bool(false));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsXor").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_xor_false_true() {
        let node = XorNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_bool(false));
        inputs.insert("B".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsXor").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_xor_true_true() {
        let node = XorNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_bool(true));
        inputs.insert("B".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsXor").unwrap().as_bool(), Some(false));
    }
}
