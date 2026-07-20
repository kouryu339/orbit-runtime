//! Neg 节点 - 数值取反

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Neg 节点 - 数值取反
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Math",
    display_name = "-{Value}",
    description = "→{{Negated}} = -{{Value}}",
    permissions = 0,
    data_in = ["Value:f64@输入值"],
    data_out = ["Negated:f64@取反结果"]
)]
pub struct NegNode;

impl Default for NegNode {
    fn default() -> Self {
        Self
    }
}

impl NegNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for NegNode {
    fn name(&self) -> &str {
        "Neg"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "f64"),
            Pin::data_out("Negated", "f64"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Negate value: -A")
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

impl NegNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let value = inputs
            .get("Value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Missing or invalid input Value".to_string(),
                )
            })?;

        let result = -value;

        let mut outputs = HashMap::new();
        outputs.insert("Negated".to_string(), DataValue::from_f64(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_neg_positive() {
        let node = NegNode;
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(5.0));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Negated").unwrap().as_f64(), Some(-5.0));
    }

    #[test]
    fn test_neg_negative() {
        let node = NegNode;
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(-3.0));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Negated").unwrap().as_f64(), Some(3.0));
    }
}
