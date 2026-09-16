//! Literal multi-separator splitting. Empty fields are preserved; longest match wins.
use crate::error::{FrameworkError, Result};
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Pure", version = "1.0.0", category = "String",
    display_name = "按{Separators}分割{Value}",
    description = "按多个字面分隔符分割字符串；保留空项和空白，重叠时优先最长匹配；空分隔符无效",
    permissions = 0,
    data_in = ["Value:String@输入字符串", "Separators:Array<String>@自定义字面分隔符数组"],
    data_out = ["Parts:Array<String>@分割后的字符串数组"]
)]
pub struct SplitNode;

impl SplitNode {
    pub fn new() -> Self {
        Self
    }
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let invalid = |message: &str| FrameworkError::InvalidData(format!("Split: {message}"));
        let value = inputs
            .get("Value")
            .and_then(DataValue::as_str)
            .ok_or_else(|| invalid("Value must be a string"))?;
        let separators = inputs
            .get("Separators")
            .and_then(DataValue::as_array)
            .ok_or_else(|| invalid("Separators must be an array of strings"))?;
        let mut separators = separators
            .iter()
            .map(|separator| {
                separator
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("each separator must be a non-empty string"))
            })
            .collect::<Result<Vec<_>>>()?;
        separators.sort_unstable_by_key(|s| std::cmp::Reverse(s.len()));
        separators.dedup();
        let mut parts = Vec::new();
        let mut start = 0;
        let mut cursor = 0;
        while cursor < value.len() {
            let rest = &value[cursor..];
            if let Some(separator) = separators.iter().find(|s| rest.starts_with(**s)) {
                parts.push(value[start..cursor].to_string());
                cursor += separator.len();
                start = cursor;
            } else {
                cursor += rest.chars().next().unwrap().len_utf8();
            }
        }
        parts.push(value[start..].to_string());
        Ok(HashMap::from([(
            "Parts".to_string(),
            DataValue::from_array(parts, "String"),
        )]))
    }
}

impl BlueprintNode for SplitNode {
    fn name(&self) -> &str {
        "Split"
    }
    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }
    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Value", "String"),
            Pin::data_in("Separators", "Array<String>"),
            Pin::data_out("Parts", "Array<String>"),
        ]
    }
    fn category(&self) -> Option<&str> {
        Some("String")
    }
    fn execute_node<'a>(
        &'a self,
        _ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move { Ok(NodeOutput::Data(self.evaluate(inputs)?)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_unicode_overlapping_and_empty_fields() {
        for (value, separators, expected) in [
            ("aa-bb-cc", vec!["-", ","], vec!["aa", "bb", "cc"]),
            ("aa-c,i", vec!["-", ","], vec!["aa", "c", "i"]),
            (
                "甲，乙--丙-",
                vec!["-", "--", "，"],
                vec!["甲", "乙", "丙", ""],
            ),
            ("-a,, b", vec!["-", ","], vec!["", "a", "", " b"]),
            ("a.*b", vec![".*"], vec!["a", "b"]),
            ("", vec![], vec![""]),
            ("abc", vec![], vec!["abc"]),
        ] {
            let output = SplitNode
                .evaluate(HashMap::from([
                    ("Value".into(), DataValue::from_string(value)),
                    (
                        "Separators".into(),
                        DataValue::from_array(separators, "String"),
                    ),
                ]))
                .unwrap();
            assert_eq!(output["Parts"].extract_array::<String>().unwrap(), expected);
        }
        assert!(SplitNode
            .evaluate(HashMap::from([
                ("Value".into(), DataValue::from_string("abc")),
                (
                    "Separators".into(),
                    DataValue::from_array(vec![""], "String")
                ),
            ]))
            .is_err());
    }
}
