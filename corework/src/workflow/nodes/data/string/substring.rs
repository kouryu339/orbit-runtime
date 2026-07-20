//! SubstringNode - 子字符串节点
//!
//! Pure节点：提取子字符串

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubstringNode;

impl SubstringNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("String", "String"),
            Pin::data_in("Start", "i64"),
            Pin::data_in("Length", "i64"),
            Pin::data_out("Result", "String"),
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
                    "Substring: Invalid input 'String'".to_string(),
                )
            })?;

        let start = inputs
            .get("Start")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Substring: Invalid input 'Start'".to_string(),
                )
            })? as usize;

        let length = inputs
            .get("Length")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Substring: Invalid input 'Length'".to_string(),
                )
            })? as usize;

        let end = start.saturating_add(length).min(string.len());
        let result = if start < string.len() {
            &string[start..end]
        } else {
            ""
        };

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_string(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substring() {
        let node = SubstringNode::new();
        let mut inputs = HashMap::new();
        inputs.insert(
            "String".to_string(),
            DataValue::from_string("Hello, World!"),
        );
        inputs.insert("Start".to_string(), DataValue::from_i64(7));
        inputs.insert("Length".to_string(), DataValue::from_i64(5));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("World"));
    }

    #[test]
    fn test_substring_out_of_bounds() {
        let node = SubstringNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("test"));
        inputs.insert("Start".to_string(), DataValue::from_i64(10));
        inputs.insert("Length".to_string(), DataValue::from_i64(5));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some(""));
    }
}
