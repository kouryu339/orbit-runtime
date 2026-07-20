//! ToFloatNode - 转换为浮点数节点
//!
//! Pure节点：将值转换为 f64

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToFloatNode;

impl ToFloatNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![Pin::data_in("Value", "Any"), Pin::data_out("Result", "f64")]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let value = inputs.get("Value").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("ToFloat: Missing 'Value' input".to_string())
        })?;

        let float_value = value.to_f64()?;

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_f64(float_value));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_float_from_int() {
        let node = ToFloatNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_i64(42));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_f64(), Some(42.0));
    }

    #[test]
    fn test_to_float_from_string() {
        let node = ToFloatNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("2.5"));

        let outputs = node.evaluate(inputs).unwrap();
        let result = outputs.get("Result").unwrap().as_f64().unwrap();
        assert!((result - 2.5).abs() < 1e-6);
    }

    #[test]
    fn test_to_float_from_bool() {
        let node = ToFloatNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_f64(), Some(1.0));
    }

    #[test]
    fn test_to_float_invalid() {
        let node = ToFloatNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("not_a_number"));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }
}
