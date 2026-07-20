//! Greater 节点 - 大于比较

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Greater 节点 - 大于比较
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Logic",
    display_name = "{A}>{B}",
    description = "→{{IsGreater}} = {{A}} > {{B}}",
    permissions = 0,
    data_in = ["A:f64@左操作数", "B:f64@右操作数"],
    data_out = ["IsGreater:bool@比较结果，A>B时为true"]
)]
pub struct GreaterNode;

impl Default for GreaterNode {
    fn default() -> Self {
        Self
    }
}

impl GreaterNode {
    pub fn new() -> Self {
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
        Some("Compare if A > B")
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

impl GreaterNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Greater: 输入 A 无效或缺失".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Greater: 输入 B 无效或缺失".to_string())
        })?;

        let result = a > b;

        let mut outputs = HashMap::new();
        outputs.insert("IsGreater".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_greater_true() {
        let node = GreaterNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(5.0));
        inputs.insert("B".to_string(), DataValue::from_f64(3.0));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsGreater").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_greater_false() {
        let node = GreaterNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(5.0));
        inputs.insert("B".to_string(), DataValue::from_f64(10.0));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsGreater").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_greater_equal() {
        let node = GreaterNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(5.0));
        inputs.insert("B".to_string(), DataValue::from_f64(5.0));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsGreater").unwrap().as_bool(), Some(false));
    }
}
