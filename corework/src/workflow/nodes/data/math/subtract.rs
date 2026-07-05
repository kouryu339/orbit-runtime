//! SubtractNode - 减法节点
//!
//! Pure节点：计算 A - B

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubtractNode;

impl SubtractNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "f64"),
            Pin::data_in("B", "f64"),
            Pin::data_out("Result", "f64"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Subtract: Invalid input 'A'".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Subtract: Invalid input 'B'".to_string())
        })?;

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_f64(a - b));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtract() {
        let node = SubtractNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(10.0));
        inputs.insert("B".to_string(), DataValue::from_f64(3.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 7.0).abs() < 1e-10);
    }

    #[test]
    fn test_subtract_negative() {
        let node = SubtractNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(3.0));
        inputs.insert("B".to_string(), DataValue::from_f64(10.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - (-7.0)).abs() < 1e-10);
    }
}
