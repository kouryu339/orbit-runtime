//! TrimNode - 字符串修剪节点
//!
//! Pure节点：去除字符串首尾空白

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// 字符串修剪节点 - 去除字符串首尾空白
#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "String",
    display_name = "Trim",
    description = "→{{Trimmed}}：去除{{Value}}首尾空白",
    permissions = 0,
    data_in = ["Value:String@要修剪的字符串"],
    data_out = ["Trimmed:String@修剪后的字符串"]
)]
pub struct TrimNode;

impl TrimNode {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let s = inputs
            .get("Value")
            .or_else(|| inputs.get("String")) // 兼容旧引脚名
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut outputs = HashMap::new();
        outputs.insert("Trimmed".to_string(), DataValue::from_string(s.trim()));
        Ok(outputs)
    }
}

impl BlueprintNode for TrimNode {
    fn name(&self) -> &str {
        "Trim"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "String"),
            Pin::data_out("Trimmed", "String"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Trims leading and trailing whitespace from a string")
    }

    fn category(&self) -> Option<&str> {
        Some("String")
    }

    fn execute_node<'a>(
        &'a self,
        _ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move {
            let outputs = self.evaluate(inputs)?;
            Ok(NodeOutput::Data(outputs))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim() {
        let node = TrimNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("  hello  "));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Trimmed").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn test_trim_newlines() {
        let node = TrimNode::new();
        let mut inputs = HashMap::new();
        inputs.insert(
            "String".to_string(),
            DataValue::from_string("\n\thello\t\n"),
        );

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Trimmed").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn test_trim_no_whitespace() {
        let node = TrimNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("hello"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Trimmed").unwrap().as_str(), Some("hello"));
    }
}
