use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 逻辑非节点
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotNode;

impl NotNode {
    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "bool"),
            Pin::data_out("Result", "bool"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>, String> {
        let value = inputs
            .get("Value")
            .ok_or("Missing input Value")?
            .as_bool()
            .ok_or("Invalid boolean value")?;

        let result = !value;

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
    fn test_not_true() {
        let node = NotNode;
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_bool(true));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_not_false() {
        let node = NotNode;
        let mut inputs = HashMap::new();
        inputs.insert("Value".to_string(), DataValue::from_bool(false));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_bool(), Some(true));
    }
}
