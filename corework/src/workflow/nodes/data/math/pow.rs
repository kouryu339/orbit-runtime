//! Pow 节点 - 幂运算

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Pow 节点 - 幂运算
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Math",
    display_name = "Pow",
    description = "→{{Power}} = {{Base}} ^ {{Exponent}}",
    permissions = 0,
    data_in = ["Base:f64@底数", "Exponent:f64@指数"],
    data_out = ["Power:f64@运算结果"]
)]
pub struct PowNode;

impl Default for PowNode {
    fn default() -> Self {
        Self
    }
}

impl PowNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for PowNode {
    fn name(&self) -> &str {
        "Pow"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Base", "f64"),
            Pin::data_in("Exponent", "f64"),
            Pin::data_out("Power", "f64"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Power: Base^Exponent")
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

impl PowNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let base = inputs.get("Base").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing or invalid input Base".to_string())
        })?;

        let exponent = inputs
            .get("Exponent")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Missing or invalid input Exponent".to_string(),
                )
            })?;

        let result = base.powf(exponent);

        let mut outputs = HashMap::new();
        outputs.insert("Power".to_string(), DataValue::from_f64(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_pow_square() {
        let node = PowNode;
        let mut inputs = HashMap::new();
        inputs.insert("Base".to_string(), DataValue::from_f64(2.0));
        inputs.insert("Exponent".to_string(), DataValue::from_f64(3.0));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Power").unwrap().as_f64(), Some(8.0));
    }

    #[test]
    fn test_pow_sqrt() {
        let node = PowNode;
        let mut inputs = HashMap::new();
        inputs.insert("Base".to_string(), DataValue::from_f64(16.0));
        inputs.insert("Exponent".to_string(), DataValue::from_f64(0.5));

        let outputs = node.evaluate(inputs).unwrap();
        assert!((outputs.get("Power").unwrap().as_f64().unwrap() - 4.0).abs() < 0.0001);
    }
}
