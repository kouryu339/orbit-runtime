//! GetLast 节点 - 获取数组最后一个元素
//!
//! Pure 节点：返回数组末尾元素，IsValid 指示数组是否非空

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// GetLast 节点 - 获取数组最后一个元素
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Array",
    display_name = "Get Last",
    description = "获取{{Array}}最后一个元素，→{{Element}}，→{{IsValid}}",
    permissions = 0,
    data_in = ["Array:Array<Any>@输入数组"],
    data_out = ["Element:Any@最后一个元素", "IsValid:bool@数组是否非空（false 表示数组为空）"]
)]
pub struct GetLastNode;

impl Default for GetLastNode {
    fn default() -> Self {
        Self
    }
}

impl GetLastNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for GetLastNode {
    fn name(&self) -> &str {
        "GetLast"
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
        Some("获取数组最后一个元素，IsValid=false 时数组为空")
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

impl GetLastNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let array = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("GetLast: 缺少输入 'Array'".to_string())
        })?;

        let mut outputs = HashMap::new();
        let len = array.array_len().unwrap_or(0);
        if len == 0 {
            outputs.insert("IsValid".to_string(), DataValue::from_bool(false));
        } else {
            match array.get_array_element(len - 1) {
                Some(elem) => {
                    outputs.insert("Element".to_string(), elem);
                    outputs.insert("IsValid".to_string(), DataValue::from_bool(true));
                }
                None => {
                    outputs.insert("IsValid".to_string(), DataValue::from_bool(false));
                }
            }
        }
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_last_success() {
        let node = GetLastNode::new();
        let array = DataValue::from_array(
            vec![serde_json::json!("first"), serde_json::json!("last")],
            "String",
        );
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsValid").unwrap().as_bool(), Some(true));
        assert_eq!(outputs.get("Element").unwrap().as_str(), Some("last"));
    }

    #[test]
    fn test_get_last_single_element() {
        let node = GetLastNode::new();
        let array = DataValue::from_array(vec![serde_json::json!(42)], "i64");
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsValid").unwrap().as_bool(), Some(true));
        assert_eq!(outputs.get("Element").unwrap().as_i64(), Some(42));
    }

    #[test]
    fn test_get_last_empty_array() {
        let node = GetLastNode::new();
        let array = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsValid").unwrap().as_bool(), Some(false));
    }
}
