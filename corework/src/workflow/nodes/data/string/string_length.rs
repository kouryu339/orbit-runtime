//! StringLengthNode - 字符串长度节点
//!
//! Pure节点：获取字符串长度

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StringLengthNode;

impl StringLengthNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("String", "String"),
            Pin::data_out("Length", "i64"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let string = inputs
            .get("String")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "StringLength: Invalid input 'String'".to_string(),
                )
            })?;

        let length = string.len() as i64;

        let mut outputs = HashMap::new();
        outputs.insert("Length".to_string(), DataValue::from_i64(length));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_length() {
        let node = StringLengthNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("Hello"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Length").unwrap().as_i64(), Some(5));
    }

    #[test]
    fn test_string_length_empty() {
        let node = StringLengthNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string(""));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Length").unwrap().as_i64(), Some(0));
    }
}
