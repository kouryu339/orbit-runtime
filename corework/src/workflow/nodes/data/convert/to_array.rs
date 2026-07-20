//! ToArrayNode - 转换为数组节点
//!
//! Pure节点：将单个值包装为数组

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToArrayNode;

impl ToArrayNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "Any"),
            Pin::data_out("Array", "Array<Any>"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let value = inputs.get("Value").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("ToArray: Missing 'Value' input".to_string())
        })?;

        // 如果已经是数组，直接返回
        if value.is_array() {
            let mut outputs = HashMap::new();
            outputs.insert("Array".to_string(), value.clone());
            return Ok(outputs);
        }

        // 否则将值包装为单元素数组
        let array = DataValue::from_array(vec![value.json_value().clone()], "Any");

        let mut outputs = HashMap::new();
        outputs.insert("Array".to_string(), array);
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_array_from_int() {
        let node = ToArrayNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_i64(42));

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(1));
        assert_eq!(array.get_array_element(0).unwrap().as_i64(), Some(42));
    }

    #[test]
    fn test_to_array_from_string() {
        let node = ToArrayNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("hello"));

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(1));
    }

    #[test]
    fn test_to_array_from_array() {
        let node = ToArrayNode::new();
        let existing_array =
            DataValue::from_array(vec![serde_json::json!(1), serde_json::json!(2)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), existing_array);

        let outputs = node.evaluate(inputs).unwrap();
        let array = outputs.get("Array").unwrap();

        assert!(array.is_array());
        assert_eq!(array.array_len(), Some(2)); // 保持原样，不重复包装
    }
}
