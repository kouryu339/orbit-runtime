//! ArrayConcatNode - 数组拼接节点
//!
//! Pure节点：连接两个数组

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
