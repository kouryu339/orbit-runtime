//! StringAppendNode - 字符串拼接节点
//!
//! Pure节点：连接两个字符串

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// 字符串拼接节点 - 将 A 和 B 两个字符串连接起来
#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "String",
    display_name = "StringAppend",
    description = "→{{Joined}} = {{A}} + {{B}} 字符串连接",
    permissions = 0,
    data_in = ["A:String@左侧字符串", "B:String@右侧字符串"],
    data_out = ["Joined:String@连接结果"]
)]
pub struct StringAppendNode;

impl BlueprintNode for StringAppendNode {
    fn name(&self) -> &str {
        "StringAppend"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "String"),
            Pin::data_in("B", "String"),
            Pin::data_out("Joined", "String"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Concatenates two strings: Result = A + B")
    }

    fn category(&self) -> Option<&str> {
        Some("String")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: std::collections::HashMap<String, DataValue>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::workflow::core::NodeOutput>> + Send + 'a,
        >,
    > {
        self.__execute_node_impl(ctx, inputs)
    }
}

impl StringAppendNode {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("StringAppend: Invalid input 'A'".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("StringAppend: Invalid input 'B'".to_string())
        })?;

        let result = format!("{}{}", a, b);

        let mut outputs = HashMap::new();
        outputs.insert("Joined".to_string(), DataValue::from_string(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_append() {
        let node = StringAppendNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_string("Hello, "));
        inputs.insert("B".to_string(), DataValue::from_string("World!"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(
            outputs.get("Joined").unwrap().as_str(),
            Some("Hello, World!")
        );
    }

    #[test]
    fn test_string_append_empty() {
        let node = StringAppendNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_string(""));
        inputs.insert("B".to_string(), DataValue::from_string("test"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Joined").unwrap().as_str(), Some("test"));
    }
}
