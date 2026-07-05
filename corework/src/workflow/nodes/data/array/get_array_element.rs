//! GetArrayElementNode - 数组元素获取节点
//!
//! Pure节点：根据索引获取数组元素

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetArrayElementNode;

impl GetArrayElementNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Array", "Array<Any>"),
            Pin::data_in("Index", "i64"),
            Pin::data_out("Element", "Any"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let array_value = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "GetArrayElement: Missing 'Array' input".to_string(),
            )
        })?;

        let index = inputs
            .get("Index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "GetArrayElement: Invalid 'Index' input".to_string(),
                )
            })?;

        if index < 0 {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "GetArrayElement: Index cannot be negative ({})",
                index
            )));
        }

        let element = array_value
            .get_array_element(index as usize)
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(format!(
                    "GetArrayElement: Index {} out of bounds",
                    index
                ))
            })?;

        let mut outputs = HashMap::new();
        outputs.insert("Element".to_string(), element);
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_array_element_success() {
        let node = GetArrayElementNode::new();

        let array = DataValue::from_array(
            vec![
                serde_json::json!(10),
                serde_json::json!(20),
                serde_json::json!(30),
            ],
            "i64",
        );

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(1));

        let outputs = node.evaluate(inputs).unwrap();
        let element = outputs.get("Element").unwrap();

        assert_eq!(element.as_i64(), Some(20));
    }

    #[test]
    fn test_get_array_element_out_of_bounds() {
        let node = GetArrayElementNode::new();

        let array = DataValue::from_array(vec![serde_json::json!(10)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(5));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_array_element_negative_index() {
        let node = GetArrayElementNode::new();

        let array = DataValue::from_array(vec![serde_json::json!(10)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(-1));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }
}
