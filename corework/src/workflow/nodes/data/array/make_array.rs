//! MakeArrayNode - 数组构造节点
//!
//! Pure节点：从多个元素构造数组

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MakeArrayNode {
    /// 数组元素类型提示
    pub element_type: String,
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
