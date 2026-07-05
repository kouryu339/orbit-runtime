//! RegexMatch 节点 - 正则表达式匹配

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// RegexMatch 节点 - 正则表达式匹配
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "String",
    display_name = "RegexMatch",
    description = "→{{IsMatch}}：{{Value}}是否匹配{{Pattern}}正则",
    permissions = 0,
    data_in = ["Value:String@要匹配的字符串", "Pattern:String@正则表达式"],
    data_out = ["IsMatch:bool@是否匹配成功"]
)]
pub struct RegexMatchNode;

impl Default for RegexMatchNode {
    fn default() -> Self {
        Self
    }
}

impl RegexMatchNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for RegexMatchNode {
    fn name(&self) -> &str {
        "RegexMatch"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "String"),
            Pin::data_in("Pattern", "String"),
            Pin::data_out("IsMatch", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Match string with regex pattern")
    }

    fn category(&self) -> Option<&str> {
        Some("String")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::workflow::core::NodeOutput>> + Send + 'a,
        >,
    > {
        self.__execute_node_impl(ctx, inputs)
    }
}

impl RegexMatchNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let value = inputs
            .get("Value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Missing or invalid input Value".to_string(),
                )
            })?;

        let pattern = inputs
            .get("Pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "Missing or invalid input Pattern".to_string(),
                )
            })?;

        let result = match regex::Regex::new(pattern) {
            Ok(re) => re.is_match(value),
            Err(_) => false,
        };

        let mut outputs = HashMap::new();
        outputs.insert("IsMatch".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_regex_match() {
        let node = RegexMatchNode;
        let mut inputs = HashMap::new();
        inputs.insert(
            "Value".to_string(),
            DataValue::from_string("hello123".to_string()),
        );
        inputs.insert(
            "Pattern".to_string(),
            DataValue::from_string(r"\d+".to_string()),
        );

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsMatch").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_regex_no_match() {
        let node = RegexMatchNode;
        let mut inputs = HashMap::new();
        inputs.insert(
            "Value".to_string(),
            DataValue::from_string("hello".to_string()),
        );
        inputs.insert(
            "Pattern".to_string(),
            DataValue::from_string(r"\d+".to_string()),
        );

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsMatch").unwrap().as_bool(), Some(false));
    }
}
