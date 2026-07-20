//! ClampNode - 钳制节点
//!
//! Pure节点：将值限制在 [Min, Max] 范围内

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClampNode;

impl ClampNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "f64"),
            Pin::data_in("Min", "f64"),
            Pin::data_in("Max", "f64"),
            Pin::data_out("Result", "f64"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let value = inputs
            .get("Value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Clamp: Invalid input 'Value'".to_string(),
                )
            })?;

        let min = inputs.get("Min").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Clamp: Invalid input 'Min'".to_string())
        })?;

        let max = inputs.get("Max").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Clamp: Invalid input 'Max'".to_string())
        })?;

        // 确保 Min <= Max
        if min > max {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "Clamp: Min ({}) must be <= Max ({})",
                min, max
            )));
        }

        let clamped = value.max(min).min(max);

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_f64(clamped));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_within_range() {
        let node = ClampNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(5.0));
        inputs.insert("Min".to_string(), DataValue::from_f64(0.0));
        inputs.insert("Max".to_string(), DataValue::from_f64(10.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_clamp_below_min() {
        let node = ClampNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(-5.0));
        inputs.insert("Min".to_string(), DataValue::from_f64(0.0));
        inputs.insert("Max".to_string(), DataValue::from_f64(10.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_clamp_above_max() {
        let node = ClampNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(15.0));
        inputs.insert("Min".to_string(), DataValue::from_f64(0.0));
        inputs.insert("Max".to_string(), DataValue::from_f64(10.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_clamp_invalid_range() {
        let node = ClampNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_f64(5.0));
        inputs.insert("Min".to_string(), DataValue::from_f64(10.0));
        inputs.insert("Max".to_string(), DataValue::from_f64(0.0));

        assert!(node.evaluate(inputs).is_err());
    }
}
