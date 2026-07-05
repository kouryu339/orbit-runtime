use crate::workflow::core::{DataValue, Pin};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 条件选择节点（三元运算符）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SelectNode;

impl SelectNode {
    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Condition", "bool"),
            Pin::data_in("TrueValue", "Any"),
            Pin::data_in("FalseValue", "Any"),
            Pin::data_out("Result", "Any"),
        ]
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>, String> {
        let condition = inputs
            .get("Condition")
            .ok_or("Missing input Condition")?
            .as_bool()
            .ok_or("Invalid boolean value")?;
        let true_value = inputs.get("TrueValue").ok_or("Missing input TrueValue")?;
        let false_value = inputs.get("FalseValue").ok_or("Missing input FalseValue")?;

        let result = if condition {
            true_value.clone()
        } else {
            false_value.clone()
        };

        let mut outputs = HashMap::new();
        outputs.insert("Result".to_string(), result);
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_select_true() {
        let node = SelectNode;
        let mut inputs = HashMap::new();
        inputs.insert("Condition".to_string(), DataValue::from_bool(true));
        inputs.insert("TrueValue".to_string(), DataValue::from_i64(100));
        inputs.insert("FalseValue".to_string(), DataValue::from_i64(200));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_i64(), Some(100));
    }

    #[test]
    fn test_select_false() {
        let node = SelectNode;
        let mut inputs = HashMap::new();
        inputs.insert("Condition".to_string(), DataValue::from_bool(false));
        inputs.insert("TrueValue".to_string(), DataValue::from_string("yes"));
        inputs.insert("FalseValue".to_string(), DataValue::from_string("no"));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("Result").unwrap().as_str(), Some("no"));
    }
}
