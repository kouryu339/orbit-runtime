//! ToStringNode - 转换为字符串节点
//!
//! Pure节点：将值转换为 String

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToStringNode;

impl ToStringNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "Any"),
            Pin::data_out("Result", "String"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let value = inputs.get("Value").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("ToString: Missing 'Value' input".to_string())
        })?;

        let string_value = value.to_string_value();

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_string(string_value));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_string_from_int() {
        let node = ToStringNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_i64(42));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("42"));
    }

    #[test]
    fn test_to_string_from_float() {
        let node = ToStringNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(2.5));

        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("Result").unwrap().as_str().unwrap();
        assert!(result.starts_with("2.5"));
    }

    #[test]
    fn test_to_string_from_bool() {
        let node = ToStringNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("true"));
    }

    #[test]
    fn test_to_string_from_string() {
        let node = ToStringNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("hello"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("hello"));
    }
}
