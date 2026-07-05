//!
//! Pure节点：计算 |A|

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AbsNode;

impl AbsNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![Pin::data_in("A", "f64"), Pin::data_out("Result", "f64")]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Abs: Invalid input 'A'".to_string())
        })?;

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_f64(a.abs()));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abs_positive() {
        let node = AbsNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(5.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_abs_negative() {
        let node = AbsNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(-5.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_abs_zero() {
        let node = AbsNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(0.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 0.0).abs() < 1e-10);
    }
}
