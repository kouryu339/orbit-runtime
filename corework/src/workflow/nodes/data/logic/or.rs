use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 逻辑或节点
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrNode;

impl OrNode {
    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "bool"),
            Pin::data_in("B", "bool"),
            Pin::data_out("Result", "bool"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>, String> {
        let a = inputs
            .get("A")
            .ok_or("Missing input A")?
            .as_bool()
            .ok_or("Invalid boolean value for A")?;
        let b = inputs
            .get("B")
            .ok_or("Missing input B")?
            .as_bool()
            .ok_or("Invalid boolean value for B")?;

        let result = a || b;

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_or_true_true() {
        let node = OrNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_bool(true));
        inputs.insert("B".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_or_true_false() {
        let node = OrNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_bool(true));
        inputs.insert("B".to_string(), DataValue::from_bool(false));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_or_false_false() {
        let node = OrNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_bool(false));
        inputs.insert("B".to_string(), DataValue::from_bool(false));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(false));
    }
}
