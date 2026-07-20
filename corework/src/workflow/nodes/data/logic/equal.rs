//! Equal 节点 - 相等比较（支持 Any 类型）

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, Pin};
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Equal 节点 - 相等比较（支持 Any 类型：整数、浮点、字符串、bool）
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Logic",
    display_name = "{A}等于{B}",
    description = "→{{IsEqual}} = {{A}} == {{B}}",
    permissions = 0,
    data_in = ["A:Any@左操作数", "B:Any@右操作数"],
    data_out = ["IsEqual:bool@比较结果，相等为true"]
)]
pub struct EqualNode;

impl Default for EqualNode {
    fn default() -> Self {
        Self
    }
}

impl EqualNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for EqualNode {
    fn name(&self) -> &str {
        "Equal"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("A", "Any"),
            Pin::data_in("B", "Any"),
            Pin::data_out("IsEqual", "bool"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("Compare if A == B (supports int, float, string, bool)")
    }

    fn category(&self) -> Option<&str> {
        Some("Logic")
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

/// 比较两个 DataValue 是否相等（支持整数、浮点、字符串、bool）
fn values_equal(a: &DataValue, b: &DataValue) -> bool {
    // 尝试字符串比较
    if let (Some(a_str), Some(b_str)) = (a.as_str(), b.as_str()) {
        return a_str == b_str;
    }
    // 尝试 bool 比较
    if let (Some(a_bool), Some(b_bool)) = (a.as_bool(), b.as_bool()) {
        return a_bool == b_bool;
    }
    // 尝试整数比较
    if let (Some(a_i64), Some(b_i64)) = (a.as_i64(), b.as_i64()) {
        return a_i64 == b_i64;
    }
    // 尝试浮点比较
    if let (Some(a_f64), Some(b_f64)) = (a.as_f64(), b.as_f64()) {
        return (a_f64 - b_f64).abs() < f64::EPSILON;
    }
    // 类型不匹配
    false
}

impl EqualNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let a = inputs.get("A").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing input A".to_string())
        })?;

        let b = inputs.get("B").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("Missing input B".to_string())
        })?;

        let result = values_equal(a, b);

        let mut outputs = HashMap::new();
        outputs.insert("IsEqual".to_string(), DataValue::from_bool(result));
        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::core::DataValue;

    #[test]
    fn test_equal_same_values() {
        let node = EqualNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(42));
        inputs.insert("B".to_string(), DataValue::from_i64(42));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsEqual").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_equal_different_values() {
        let node = EqualNode;
        let mut inputs = HashMap::new();
        inputs.insert("A".to_string(), DataValue::from_i64(42));
        inputs.insert("B".to_string(), DataValue::from_i64(43));

        let outputs = node.evaluate(inputs).unwrap();
        assert_eq!(outputs.get("IsEqual").unwrap().as_bool(), Some(false));
    }
}
