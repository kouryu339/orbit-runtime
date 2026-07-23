//! ArrayConcatNode - 数组拼接节点
//!
//! Pure节点：连接两个数组

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Array",
    display_name = "{Array1}后拼接{Array2}",
    description = "→{{Result}}：拼接两个数组",
    permissions = 0,
    data_in = ["Array1:Array<Any>@前半数组", "Array2:Array<Any>@后半数组"],
    data_out = ["Result:Array<Any>@拼接后的数组"]
)]
pub struct ArrayConcatNode;

impl ArrayConcatNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Array1", "Array<Any>"),
            Pin::data_in("Array2", "Array<Any>"),
            Pin::data_out("Result", "Array<Any>"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let array1 = inputs.get("Array1").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "ArrayConcat: Missing 'Array1' input".to_string(),
            )
        })?;

        let array2 = inputs.get("Array2").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "ArrayConcat: Missing 'Array2' input".to_string(),
            )
        })?;

        let arr1 = array1.as_array().ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "ArrayConcat: 'Array1' is not an array".to_string(),
            )
        })?;

        let arr2 = array2.as_array().ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "ArrayConcat: 'Array2' is not an array".to_string(),
            )
        })?;

        // 合并两个数组
        let mut result_elements = arr1.clone();
        result_elements.extend_from_slice(arr2);

        let mut outputs = HashMap::new();
        outputs.insert(
            "Result".to_string(),
            DataValue::from_array(result_elements, "Any"),
        );
        Ok(outputs)
    }
}

impl BlueprintNode for ArrayConcatNode {
    fn name(&self) -> &str {
        "ArrayConcat"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        ArrayConcatNode::pins(self)
    }

    fn description(&self) -> Option<&str> {
        Some("Concatenates two arrays")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_concat_normal() {
        let node = ArrayConcatNode::new();

        let array1 = DataValue::from_array(vec![serde_json::json!(1), serde_json::json!(2)], "i64");

        let array2 = DataValue::from_array(vec![serde_json::json!(3), serde_json::json!(4)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array1".to_string(), array1);
        inputs.insert("Array2".to_string(), array2);

        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("Result").unwrap();

        assert_eq!(result.array_len(), Some(4));
        assert_eq!(result.get_array_element(0).unwrap().as_i64(), Some(1));
        assert_eq!(result.get_array_element(3).unwrap().as_i64(), Some(4));
    }

    #[test]
    fn test_array_concat_empty() {
        let node = ArrayConcatNode::new();

        let array1 = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");
        let array2 = DataValue::from_array(vec![serde_json::json!(1)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array1".to_string(), array1);
        inputs.insert("Array2".to_string(), array2);

        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("Result").unwrap();

        assert_eq!(result.array_len(), Some(1));
    }

    #[test]
    fn test_array_concat_both_empty() {
        let node = ArrayConcatNode::new();

        let array1 = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");
        let array2 = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");

        let mut inputs = HashMap::new();
        inputs.insert("Array1".to_string(), array1);
        inputs.insert("Array2".to_string(), array2);

        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("Result").unwrap();

        assert_eq!(result.array_len(), Some(0));
    }

    #[test]
    fn test_array_concat_invalid_input() {
        let node = ArrayConcatNode::new();

        let mut inputs = HashMap::new();
        inputs.insert("Array1".to_string(), DataValue::from_i64(123));
        inputs.insert(
            "Array2".to_string(),
            DataValue::from_array(Vec::<serde_json::Value>::new(), "Any"),
        );

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }
}
