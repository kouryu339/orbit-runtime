//! ToIntNode - 转换为整数节点
//!
//! Pure节点：将值转换为 i64

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToIntNode;

impl ToIntNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![Pin::data_in("Value", "Any"), Pin::data_out("Result", "i64")]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let value = inputs.get("Value").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("ToInt: Missing 'Value' input".to_string())
        })?;

        let int_value = value.to_i64()?;

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_i64(int_value));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_int_from_float() {
        let node = ToIntNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(42.7));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_i64(), Some(42));
    }

    #[test]
    fn test_to_int_from_string() {
        let node = ToIntNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("123"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_i64(), Some(123));
    }

    #[test]
    fn test_to_int_from_bool() {
        let node = ToIntNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn test_to_int_invalid() {
        let node = ToIntNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_string("abc"));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }
}
