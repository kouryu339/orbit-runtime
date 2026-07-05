//! ModuloNode - 取模节点
//!
//! Pure节点：计算 A % B（求余数）

use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModuloNode;

impl ModuloNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "i64"),
            Pin::data_in("B", "i64"),
            Pin::data_out("Result", "i64"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").and_then(|v| v.as_i64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Modulo: Invalid input 'A'".to_string())
        })?;

        let b = inputs.get("B").and_then(|v| v.as_i64()).ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Modulo: Invalid input 'B'".to_string())
        })?;

        // 除零检查
        if b == 0 {
            return Err(crate::error::FrameworkError::SystemError(
                "Modulo: Division by zero".to_string(),
            ));
        }

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_i64(a % b));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modulo() {
        let node = ModuloNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(10));
        inputs.insert("B".to_string(), DataValue::from_i64(3));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_i64().unwrap();
        assert_eq!(output, 1);
    }

    #[test]
    fn test_modulo_exact_division() {
        let node = ModuloNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(10));
        inputs.insert("B".to_string(), DataValue::from_i64(5));

        let result = node.evaluate(inputs).unwrap();
        let output = result.get("Result").unwrap().as_i64().unwrap();
        assert_eq!(output, 0);
    }

    #[test]
    fn test_modulo_by_zero() {
        let node = ModuloNode::new();
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(10));
        inputs.insert("B".to_string(), DataValue::from_i64(0));

        assert!(node.evaluate(inputs).is_err());
    }
}
