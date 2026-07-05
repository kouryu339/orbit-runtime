//! GetFirst 节点 - 获取数组第一个元素
//!
//! Pure 节点：返回数组首个元素，IsValid 指示数组是否非空

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// GetFirst 节点 - 获取数组第一个元素
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Array",
    display_name = "Get First",
    description = "获取{{Array}}第一个元素，→{{Element}}，→{{IsValid}}",
    permissions = 0,
    data_in = ["Array:Array<Any>@输入数组"],
    data_out = ["Element:Any@第一个元素", "IsValid:bool@数组是否非空（false 表示数组为空）"]
)]
pub struct GetFirstNode;

impl Default for GetFirstNode {
    fn default() -> Self {
        Self
    }
}

impl GetFirstNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for GetFirstNode {
    fn name(&self) -> &str {
        "GetFirst"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Array", "Array<Any>"),
            Pin::data_out("Element", "Any"),
            Pin::data_out("IsValid", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("获取数组第一个元素，IsValid=false 时数组为空")
    }

    fn category(&self) -> Option<&str> {
        Some("Array")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut crate::workflow::execution::ExecutionContext,
        inputs: std::collections::HashMap<String, crate::workflow::core::DataValue>,
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

impl GetFirstNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let array = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("GetFirst: 缺少输入 'Array'".to_string())
        })?;

        let mut outputs = HashMap::new();
        match array.get_array_element(0) {
            Some(elem) => {
                outputs.insert("Element".to_string(), elem);
                outputs.insert("IsValid".to_string(), DataValue::from_bool(true));
            }
            None => {
                outputs.insert("IsValid".to_string(), DataValue::from_bool(false));
            }
        }
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_first_success() {
        let node = GetFirstNode::new();
        let array = DataValue::from_array(
            vec![serde_json::json!("hello"), serde_json::json!("world")],
            "String",
        );
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsValid").unwrap().as_bool(), Some(true));
        assert_eq!(outputs.get("Element").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn test_get_first_empty_array() {
        let node = GetFirstNode::new();
        let array = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsValid").unwrap().as_bool(), Some(false));
        assert!(!outputs.contains_key("Element"));
    }
}
