//! ReplaceNode - 字符串替换节点
//!
//! Pure节点：替换字符串中的子串

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplaceNode;

impl ReplaceNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("String", "String"),
            Pin::data_in("From", "String"),
            Pin::data_in("To", "String"),
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
                    "Replace: Invalid input 'String'".to_string(),
                )
            })?;

        let from = inputs.get("From").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Replace: Invalid input 'From'".to_string())
        })?;

        let to = inputs.get("To").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Replace: Invalid input 'To'".to_string())
        })?;

        let result = string.replace(from, to);

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_string(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace() {
        let node = ReplaceNode::new();
        let mut inputs = HashMap::new();
        inputs.insert(
            "String".to_string(),
            DataValue::from_string("Hello, World!"),
        );
        inputs.insert("From".to_string(), DataValue::from_string("World"));
        inputs.insert("To".to_string(), DataValue::from_string("Rust"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(
            outputs.get("Result").unwrap().as_str(),
            Some("Hello, Rust!")
        );
    }

    #[test]
    fn test_replace_multiple() {
        let node = ReplaceNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("foo bar foo"));
        inputs.insert("From".to_string(), DataValue::from_string("foo"));
        inputs.insert("To".to_string(), DataValue::from_string("baz"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("baz bar baz"));
    }
}
