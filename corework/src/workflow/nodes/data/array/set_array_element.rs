//! SetArrayElementNode - 数组元素设置节点
//!
//! Pure节点：设置指定索引的数组元素（返回新数组）

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetArrayElementNode;

impl SetArrayElementNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Array", "Array<Any>"),
            Pin::data_in("Index", "i64"),
            Pin::data_in("Value", "Any"),
            Pin::data_out("Array", "Array<Any>"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let array_value = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "SetArrayElement: Missing 'Array' input".to_string(),
            )
        })?;

        let index = inputs
            .get("Index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "SetArrayElement: Invalid 'Index' input".to_string(),
                )
            })?;

        let new_value = inputs.get("Value").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "SetArrayElement: Missing 'Value' input".to_string(),
            )
        })?;

        if index < 0 {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "SetArrayElement: Index cannot be negative ({})",
                index
            )));
        }

        // 深拷贝数组
        let mut new_array = array_value.deep_clone();

        // 设置元素
        new_array.set_array_element(index as usize, new_value.clone())?;

        let mut outputs = HashMap::new();
        outputs.insert("Array".to_string(), new_array);
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_array_element_success() {
        let node = SetArrayElementNode::new();

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
        inputs.insert("Value".to_string(), DataValue::from_i64(99));

        let outputs = node.evaluate(inputs).unwrap();
        let new_array = outputs.get("Array").unwrap();

        assert_eq!(new_array.get_array_element(1).unwrap().as_i64(), Some(99));
        assert_eq!(new_array.get_array_element(0).unwrap().as_i64(), Some(10));
        assert_eq!(new_array.get_array_element(2).unwrap().as_i64(), Some(30));
    }

    #[test]
    fn test_set_array_element_out_of_bounds() {
        let node = SetArrayElementNode::new();

        let array = DataValue::from_array(vec![serde_json::json!(10)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(5));
        inputs.insert("Value".to_string(), DataValue::from_i64(99));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_array_element_negative_index() {
        let node = SetArrayElementNode::new();

        let array = DataValue::from_array(vec![serde_json::json!(10)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(-1));
        inputs.insert("Value".to_string(), DataValue::from_i64(99));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }
}
