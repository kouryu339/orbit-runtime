//! ToLowerNode - 转小写节点
//!
//! Pure节点：将字符串转为小写

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToLowerNode;

impl ToLowerNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("String", "String"),
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
                    "ToLower: Invalid input 'String'".to_string(),
                )
            })?;

        let result = string.to_lowercase();

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_string(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_lower() {
        let node = ToLowerNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("HELLO"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn test_to_lower_mixed() {
        let node = ToLowerNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("Hello World!"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(
            outputs.get("Result").unwrap().as_str(),
            Some("hello world!")
        );
    }
}
