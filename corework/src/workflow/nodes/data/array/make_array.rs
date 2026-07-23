//! MakeArrayNode - 数组构造节点
//!
//! Pure节点：从多个元素构造数组

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Array",
    display_name = "[{Element0},{Element1},{Element2},{Element3},{Element4}]",
    description = "→{{Array}}：按顺序构造数组",
    permissions = 0,
    data_in = [
        "Element0:Any@第1项",
        "Element1:Any@第2项",
        "Element2:Any@第3项",
        "Element3:Any@第4项",
        "Element4:Any@第5项"
    ],
    data_out = ["Array:Array<Any>@构造后的数组"]
)]
pub struct MakeArrayNode {
    /// 数组元素类型提示
    pub element_type: String,
}

impl Default for MakeArrayNode {
    fn default() -> Self {
        Self::new()
    }
}

impl BlueprintNode for MakeArrayNode {
    fn name(&self) -> &str {
        "MakeArray"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        MakeArrayNode::pins(self)
    }

    fn description(&self) -> Option<&str> {
        Some("Constructs an array from up to five values")
    }

    fn category(&self) -> Option<&str> {
        Some("Array")
    }

    fn execute_node<'a>(
        &'a self,
        _ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move { Ok(NodeOutput::Data(self.evaluate(inputs)?)) })
    }
}

impl MakeArrayNode {
    pub fn new() -> Self {
        Self {
            element_type: "Any".to_string(),
        }
    }

    pub fn with_element_type(mut self, element_type: impl Into<String>) -> Self {
        self.element_type = element_type.into();
        self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Element0", "Any"),
            Pin::data_in("Element1", "Any"),
            Pin::data_in("Element2", "Any"),
            Pin::data_in("Element3", "Any"),
            Pin::data_in("Element4", "Any"),
            Pin::data_out("Array", "Array<Any>"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let mut elements = Vec::new();

        // 收集所有提供的元素（跳过未连接的输入）
        for i in 0..5 {
            let key = format!("Element{}", i);
            if let Some(value) = inputs.get(&key) {
                elements.push(value.json_value().clone());
            }
        }

        let mut outputs = HashMap::new();
        outputs.insert(
            "Array".to_string(),
            DataValue::from_array(elements, &self.element_type),
        );
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_array_from_numbers() {
        let node = MakeArrayNode::new().with_element_type("f64");
        let mut inputs = HashMap::new();
        inputs.insert("Element0".to_string(), DataValue::from_f64(1.0));
        inputs.insert("Element1".to_string(), DataValue::from_f64(2.0));
        inputs.insert("Element2".to_string(), DataValue::from_f64(3.0));

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(3));
    }

    #[test]
    fn test_make_array_from_strings() {
        let node = MakeArrayNode::new().with_element_type("String");
        let mut inputs = HashMap::new();
        inputs.insert("Element0".to_string(), DataValue::from_string("hello"));
        inputs.insert("Element1".to_string(), DataValue::from_string("world"));

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(2));
    }

    #[test]
    fn test_make_array_empty() {
        let node = MakeArrayNode::new();
        let inputs = HashMap::new();

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(0));
    }

    #[test]
    fn test_make_array_sparse() {
        let node = MakeArrayNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Element0".to_string(), DataValue::from_i64(10));
        inputs.insert("Element3".to_string(), DataValue::from_i64(40));

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(2)); // 只有提供的元素
    }
}
