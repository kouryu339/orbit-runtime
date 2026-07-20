//! DivMod 节点 - 除法和取模运算
//!
//! 同时返回商和余数

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// DivMod 节点 - 同时返回商和余数
///
/// 演示如何返回多个输出值
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Math",
    display_name = "{Dividend}除以{Divisor}的商和余数",
    description = "{{Dividend}} ÷ {{Divisor}}，→{{Quotient}}商 →{{Remainder}}余数",
    permissions = 0,
    data_in = ["Dividend:i64@被除数", "Divisor:i64@除数"],
    data_out = ["Quotient:i64@商", "Remainder:i64@余数"]
)]
pub struct DivModNode;

impl Default for DivModNode {
    fn default() -> Self {
        Self
    }
}

impl BlueprintNode for DivModNode {
    fn name(&self) -> &str {
        "DivMod"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Dividend", "i64"),
            Pin::data_in("Divisor", "i64"),
            Pin::data_out("Quotient", "i64"),
            Pin::data_out("Remainder", "i64"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Division with remainder: Quotient = Dividend / Divisor, Remainder = Dividend % Divisor")
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

impl DivModNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let dividend = inputs
            .get("Dividend")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "DivMod: Missing or invalid input 'Dividend'".into(),
                )
            })?;

        let divisor = inputs
            .get("Divisor")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "DivMod: Missing or invalid input 'Divisor'".into(),
                )
            })?;

        if divisor == 0 {
            return Err(crate::error::FrameworkError::SystemError(
                "DivMod: Division by zero".into(),
            ));
        }

        let quotient = dividend / divisor;
        let remainder = dividend % divisor;

        let mut outputs = HashMap::new();
        outputs.insert("Quotient".to_string(), DataValue::from_i64(quotient));
        outputs.insert("Remainder".to_string(), DataValue::from_i64(remainder));

        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divmod_node() {
        let node = DivModNode;
        let mut inputs = HashMap::new();
        inputs.insert("Dividend".to_string(), DataValue::from_i64(17));
        inputs.insert("Divisor".to_string(), DataValue::from_i64(5));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Quotient").unwrap().as_i64().unwrap(), 3);
        assert_eq!(outputs.get("Remainder").unwrap().as_i64().unwrap(), 2);
    }

    #[test]
    fn test_divmod_zero_divisor() {
        let node = DivModNode;
        let mut inputs = HashMap::new();
        inputs.insert("Dividend".to_string(), DataValue::from_i64(10));
        inputs.insert("Divisor".to_string(), DataValue::from_i64(0));

        assert!(node.evaluate(inputs).is_err());
    }
}
