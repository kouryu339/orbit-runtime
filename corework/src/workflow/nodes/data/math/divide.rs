//! DivideNode - 除法节点
//!
//! Pure节点：计算 A / B（带除零检查）

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DivideNode;

impl DivideNode {
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
            crate::error::FrameworkError::SystemError("Divide: Invalid input 'A'".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_f64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Divide: Invalid input 'B'".to_string())
        })?;

        // 除零检查
        if b.abs() < 1e-10 {
            return Err(crate::error::FrameworkError::SystemError(
                "Divide: Division by zero".to_string(),
            ));
        }

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_f64(a / b));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divide() {
        let node = DivideNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(10.0));
        inputs.insert("B".to_string(), DataValue::from_f64(2.0));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_f64().unwrap();
        assert!((output - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_divide_by_zero() {
        let node = DivideNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_f64(10.0));
        inputs.insert("B".to_string(), DataValue::from_f64(0.0));

        assert!(node.evaluate(inputs).is_err());
    }
}
