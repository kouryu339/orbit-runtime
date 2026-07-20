//! ArrayLengthNode - 数组长度节点
//!
//! Pure节点：获取数组的长度

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArrayLengthNode;

impl ArrayLengthNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Array", "Array<Any>"),
            Pin::data_out("Length", "i64"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let array_value = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "ArrayLength: Missing 'Array' input".to_string(),
            )
        })?;

        let length = array_value.array_len().ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "ArrayLength: Input is not an array".to_string(),
            )
        })?;

        let mut outputs = HashMap::new();
        outputs.insert("Length".to_string(), DataValue::from_i64(length as i64));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_length_normal() {
        let node = ArrayLengthNode::new();

        let array = DataValue::from_array(
            vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3),
            ],
            "i64",
        );

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        let length = outputs.get("Length").unwrap();

        assert_eq!(length.as_i64(), Some(3));
    }

    #[test]
    fn test_array_length_empty() {
        let node = ArrayLengthNode::new();

        let array = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        let length = outputs.get("Length").unwrap();

        assert_eq!(length.as_i64(), Some(0));
    }

    #[test]
    fn test_array_length_not_array() {
        let node = ArrayLengthNode::new();

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), DataValue::from_i64(123));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }
}
