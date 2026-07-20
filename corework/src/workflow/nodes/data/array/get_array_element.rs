//! GetArrayElementNode - 数组元素获取节点
//!
//! Pure节点：根据索引获取数组元素

use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Array",
    display_name = "获取{Array}中索引为{Index}的项",
    description = "获取{{Array}}中索引为{{Index}}的元素；负索引从末尾开始，-1 为最后一项",
    permissions = 0,
    data_in = ["Array:Array<Any>@输入数组", "Index:i64@元素索引，负数从末尾开始"],
    data_out = ["Element:Any@指定索引对应的元素"]
)]
pub struct GetArrayElementNode;

impl GetArrayElementNode {
    pub fn new() -> Self {
        Self
    }

    pub fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Array", "Array<Any>"),
            Pin::data_in("Index", "i64"),
            Pin::data_out("Element", "Any"),
        ]
    }

    fn resolve_index(index: i64, len: usize) -> Option<usize> {
        let len = i64::try_from(len).ok()?;
        let resolved = if index < 0 {
            len.checked_add(index)?
        } else {
            index
        };

        (resolved >= 0 && resolved < len).then_some(resolved as usize)
    }

    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        let array_value = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "GetArrayElement: Missing 'Array' input".to_string(),
            )
        })?;

        let index = inputs
            .get("Index")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "GetArrayElement: Invalid 'Index' input".to_string(),
                )
            })?;

        let len = array_value.array_len().unwrap_or(0);
        let resolved_index = Self::resolve_index(index, len).ok_or_else(|| {
            crate::error::FrameworkError::SystemError(format!(
                "GetArrayElement: Index {} out of bounds for array of length {}",
                index, len
            ))
        })?;

        let element = array_value
            .get_array_element(resolved_index)
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(format!(
                    "GetArrayElement: Index {} out of bounds",
                    index
                ))
            })?;

        let mut outputs = HashMap::new();
        outputs.insert("Element".to_string(), element);
        Ok(outputs)
    }
}

impl BlueprintNode for GetArrayElementNode {
    fn name(&self) -> &str {
        "GetArrayElement"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        GetArrayElementNode::pins(self)
    }

    fn description(&self) -> Option<&str> {
        Some("按索引获取数组元素；-1 为最后一项，-2 为倒数第二项")
    }

    fn category(&self) -> Option<&str> {
        Some("Array")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = crate::error::Result<crate::workflow::core::NodeOutput>,
                > + Send
                + 'a,
        >,
    > {
        self.__execute_node_impl(ctx, inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_array_element_success() {
        let node = GetArrayElementNode::new();

        let array = DataValue::from_array(
            vec![
                serde_json::json!(10),
                serde_json::json!(20),
                serde_json::json!(30),
            ],
            "i64",
        );

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(1));

        let outputs = node.evaluate(inputs).unwrap();
        let element = outputs.get("Element").unwrap();

        assert_eq!(element.as_i64(), Some(20));
    }

    #[test]
    fn test_get_array_element_out_of_bounds() {
        let node = GetArrayElementNode::new();

        let array = DataValue::from_array(vec![serde_json::json!(10)], "i64");

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(5));

        let result = node.evaluate(inputs);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_array_element_negative_indexes() {
        let node = GetArrayElementNode::new();

        let array = DataValue::from_array(
            vec![
                serde_json::json!(10),
                serde_json::json!(20),
                serde_json::json!(30),
            ],
            "i64",
        );

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array.clone());
        inputs.insert("Index".to_string(), DataValue::from_i64(-1));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs["Element"].as_i64(), Some(30));

        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);
        inputs.insert("Index".to_string(), DataValue::from_i64(-2));
        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs["Element"].as_i64(), Some(20));
    }
}
