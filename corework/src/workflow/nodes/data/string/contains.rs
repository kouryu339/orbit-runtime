//! Contains 节点 - 字符串包含判断

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Contains 节点 - 字符串包含判断
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "String",
    display_name = "{Value}是否包含{Pattern}",
    description = "→{{Found}}：{{Value}}是否包含{{Pattern}}",
    permissions = 0,
    data_in = ["Value:String@主字符串", "Pattern:String@要查找的子串"],
    data_out = ["Found:bool@是否包含"]
)]
pub struct ContainsNode;

impl Default for ContainsNode {
    fn default() -> Self {
        Self
    }
}

impl ContainsNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for ContainsNode {
    fn name(&self) -> &str {
        "Contains"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "String"),
            Pin::data_in("Pattern", "String"),
            Pin::data_out("Found", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Check if string contains substring")
    }

    fn category(&self) -> Option<&str> {
        Some("String")
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

impl ContainsNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let value = inputs
            .get("Value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Missing or invalid input Value".to_string(),
                )
            })?;

        let pattern = inputs
            .get("Pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Missing or invalid input Pattern".to_string(),
                )
            })?;

        let result = value.contains(pattern);

        let mut outputs = HashMap::new();
        outputs.insert("Found".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_contains_true() {
        let node = ContainsNode;
        let mut inputs = HashMap::new();
        inputs.insert(
            "Value".to_string(),
            DataValue::from_string("Hello World".to_string()),
        );
        inputs.insert(
            "Pattern".to_string(),
            DataValue::from_string("World".to_string()),
        );

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Found").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_contains_false() {
        let node = ContainsNode;
        let mut inputs = HashMap::new();
        inputs.insert(
            "Value".to_string(),
            DataValue::from_string("Hello World".to_string()),
        );
        inputs.insert(
            "Pattern".to_string(),
            DataValue::from_string("Python".to_string()),
        );

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Found").unwrap().as_bool(), Some(false));
    }
}
