//! ForEach 节点 - 数组遍历
//!
//! Impure 节点：遍历数组每个元素，每次迭代输出当前元素和索引

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, LoopIteration, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// ForEach 节点 - 遍历数组每个元素
///
/// 与 ForLoop 的区别：ForLoop 是按索引范围循环，ForEach 是遍历数组元素
/// 每次迭代通过 LoopBody 分支输出当前 Item 和 Index
/// 遍历完成后触发 Completed 分支
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Array",
    display_name = "ForEach",
    description = "遍历{{Array}}，每次→{{LoopBody}}输出→{{Item}}和→{{Index}}，完成后→{{Completed}}",
    permissions = 0,
    exec_in = ["In@启动遍历"],
    exec_out = ["LoopBody@每次迭代时触发，可从 Item 和 Index 获取当前元素", "Completed@所有元素遍历完成后触发"],
    data_in = ["Array:Array<Any>@要遍历的数组"],
    data_out = ["Item:Any@当前迭代的元素", "Index:i64@当前元素的索引（从0开始）"]
)]
pub struct ForEachNode;

impl Default for ForEachNode {
    fn default() -> Self {
        Self
    }
}

impl ForEachNode {
    pub fn new() -> Self {
        Self
    }
}

impl BlueprintNode for ForEachNode {
    fn name(&self) -> &str {
        "ForEach"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("In"),
            Pin::data_in("Array", "Array<Any>"),
            Pin::exec_out("LoopBody"),
            Pin::data_out("Item", "Any"),
            Pin::data_out("Index", "i64"),
            Pin::exec_out("Completed"),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("遍历数组每个元素，每次迭代输出 Item 和 Index，结束后触发 Completed")
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

impl ForEachNode {
    pub async fn execute(
        &self,
        _ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let array = inputs.get("Array").ok_or_else(|| {
            crate::error::FrameworkError::SystemError("ForEach: 缺少输入 'Array'".to_string())
        })?;

        let len = array.array_len().unwrap_or(0);

        // 数组为空，直接跳到 Completed
        if len == 0 {
            return Ok(NodeOutput::ExecPin("Completed".to_string()));
        }

        // 构建每次迭代的数据
        let mut iterations = Vec::with_capacity(len);
        for i in 0..len {
            let element = array.get_array_element(i).ok_or_else(|| {
                crate::error::FrameworkError::SystemError(format!(
                    "ForEach: 无法获取索引 {} 处的元素",
                    i
                ))
            })?;

            let mut iter_outputs = HashMap::new();
            iter_outputs.insert("Item".to_string(), element.clone());
            iter_outputs.insert("Element".to_string(), element);
            iter_outputs.insert("Index".to_string(), DataValue::from_i64(i as i64));
            iterations.push(LoopIteration {
                outputs: iter_outputs,
            });
        }

        Ok(NodeOutput::Loop {
            body_pin: "LoopBody".to_string(),
            completed_pin: "Completed".to_string(),
            iterations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::FrameworkState;

    fn execution_context() -> crate::error::Result<ExecutionContext> {
        Ok(ExecutionContext::from_context(
            FrameworkState::initialize()?.create_context(),
        ))
    }

    #[tokio::test]
    async fn test_for_each_basic() {
        let node = ForEachNode::new();
        let array = DataValue::from_array(
            vec![
                serde_json::json!("a"),
                serde_json::json!("b"),
                serde_json::json!("c"),
            ],
            "String",
        );
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        match output {
            NodeOutput::Loop {
                body_pin,
                completed_pin,
                iterations,
            } => {
                assert_eq!(body_pin, "LoopBody");
                assert_eq!(completed_pin, "Completed");
                assert_eq!(iterations.len(), 3);

                assert_eq!(
                    iterations[0].outputs.get("Index").unwrap().as_i64(),
                    Some(0)
                );
                assert_eq!(
                    iterations[0].outputs.get("Item").unwrap().as_str(),
                    Some("a")
                );

                assert_eq!(
                    iterations[2].outputs.get("Index").unwrap().as_i64(),
                    Some(2)
                );
                assert_eq!(
                    iterations[2].outputs.get("Item").unwrap().as_str(),
                    Some("c")
                );
            }
            _ => panic!("Expected Loop output"),
        }
    }

    #[tokio::test]
    async fn test_for_each_empty_array() {
        let node = ForEachNode::new();
        let array = DataValue::from_array(Vec::<serde_json::Value>::new(), "Any");
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        match output {
            NodeOutput::ExecPin(pin) => assert_eq!(pin, "Completed"),
            _ => panic!("Expected ExecPin(Completed) for empty array"),
        }
    }

    #[tokio::test]
    async fn test_for_each_int_array() {
        let node = ForEachNode::new();
        let array =
            DataValue::from_array(vec![serde_json::json!(10), serde_json::json!(20)], "i64");
        let mut inputs = HashMap::new();
        inputs.insert("Array".to_string(), array);

        let mut ctx = execution_context().unwrap();
        let output = node.execute(&mut ctx, inputs).await.unwrap();

        match output {
            NodeOutput::Loop { iterations, .. } => {
                assert_eq!(iterations.len(), 2);
                assert_eq!(
                    iterations[0].outputs.get("Item").unwrap().as_i64(),
                    Some(10)
                );
                assert_eq!(
                    iterations[1].outputs.get("Item").unwrap().as_i64(),
                    Some(20)
                );
            }
            _ => panic!("Expected Loop output"),
        }
    }
}
