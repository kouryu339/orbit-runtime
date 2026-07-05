//! GreaterOrEqual 节点 - 大于等于比较

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// GreaterOrEqual 节点 - 大于等于比较
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Logic",
    display_name = "GreaterOrEqual",
    description = "→{{IsGreaterOrEqual}} = {{A}} >= {{B}}",
    permissions = 0,
    data_in = ["A:i64@左操作数", "B:i64@右操作数"],
    data_out = ["IsGreaterOrEqual:bool@比较结果，A>=B时为true"]
)]
pub struct GreaterOrEqualNode;

impl Default for GreaterOrEqualNode {
    fn default() -> Self {
        Self
    }
}

impl GreaterOrEqualNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for GreaterOrEqualNode {
    fn name(&self) -> &str {
        "GreaterOrEqual"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "i64"),
            Pin::data_in("B", "i64"),
            Pin::data_out("IsGreaterOrEqual", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Compare if A >= B")
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

impl GreaterOrEqualNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_i64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input A".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_i64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input B".to_string())
        })?;

        let result = a >= b;

        let mut outputs = HashMap::new();
        outputs.insert("IsGreaterOrEqual".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_greater_or_equal_greater() {
        let node = GreaterOrEqualNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(5));
        inputs.insert("B".to_string(), DataValue::from_i64(3));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(
            outputs.get("IsGreaterOrEqual").unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_greater_or_equal_equal() {
        let node = GreaterOrEqualNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(5));
        inputs.insert("B".to_string(), DataValue::from_i64(5));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(
            outputs.get("IsGreaterOrEqual").unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_greater_or_equal_false() {
        let node = GreaterOrEqualNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(3));
        inputs.insert("B".to_string(), DataValue::from_i64(5));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(
            outputs.get("IsGreaterOrEqual").unwrap().as_bool(),
            Some(false)
        );
    }
}
