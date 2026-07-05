//! ToBoolNode - 转换为布尔值节点
//!
//! Pure节点：将值转换为 bool

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToBoolNode;

impl ToBoolNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "Any"),
            Pin::data_out("Result", "bool"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let value = inputs.get("Value").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("ToBool: Missing 'Value' input".to_string())
        })?;

        let bool_value = value.to_bool();

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_bool(bool_value));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_bool_from_int() {
        let node = ToBoolNode::new();

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_i64(0));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(false));

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_i64(42));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_to_bool_from_float() {
        let node = ToBoolNode::new();

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(0.0));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(false));

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(2.5));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_to_bool_from_string() {
        let node = ToBoolNode::new();

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string(""));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(false));

        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("hello"));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_to_bool_from_bool() {
        let node = ToBoolNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }
}
