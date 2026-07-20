//! IndexOfNode - 字符串查找节点
//!
//! Pure节点：查找子字符串位置

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexOfNode;

impl IndexOfNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("String", "String"),
            Pin::data_in("SearchString", "String"),
            Pin::data_out("Index", "i64"),
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
                    "IndexOf: Invalid input 'String'".to_string(),
                )
            })?;

        let search_string = inputs
            .get("SearchString")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "IndexOf: Invalid input 'SearchString'".to_string(),
                )
            })?;

        let index = string.find(search_string).map(|i| i as i64).unwrap_or(-1);

        let mut outputs = HashMap::new();
        outputs.insert("Index".to_string(), DataValue::from_i64(index));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_of_found() {
        let node = IndexOfNode::new();
        let mut inputs = HashMap::new();
        inputs.insert(
            "String".to_string(),
            DataValue::from_string("Hello, World!"),
        );
        inputs.insert("SearchString".to_string(), DataValue::from_string("World"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Index").unwrap().as_i64(), Some(7));
    }

    #[test]
    fn test_index_of_not_found() {
        let node = IndexOfNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("String".to_string(), DataValue::from_string("Hello"));
        inputs.insert("SearchString".to_string(), DataValue::from_string("xyz"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Index").unwrap().as_i64(), Some(-1));
    }
}
