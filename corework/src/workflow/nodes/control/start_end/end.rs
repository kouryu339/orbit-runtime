//! End 节点 - 执行流终止点

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use crate::workflow::workflow::{WorkflowState, WORKFLOW_STATE_KEY};
use std::collections::HashMap;

use super::start::PinMapping;

/// End 节点 - 执行流终止点
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Control Flow",
    display_name = "End",
    description = "工作流出口，{{In}}接收后结束",
    permissions = 0,
    exec_in = ["In@接收执行流，进入此节点后工作流结束"]
)]
pub struct EndNode {
    inputs: Vec<PinMapping>,
}

impl Default for EndNode {
    fn default() -> Self {
        Self {
            inputs: vec![PinMapping::new("Result", "end::result", "Any")],
        }
    }
}

impl EndNode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cache_key(
        cache_key: impl Into<String>,
        result_pin: impl Into<String>,
        result_type: impl Into<String>,
    ) -> Self {
        Self {
            inputs: vec![PinMapping::new(result_pin, cache_key, result_type)],
        }
    }

    pub fn with_inputs(inputs: Vec<PinMapping>) -> Self {
        // 规范化所有 cache_key 为 "End:{pin_name}" 格式
        // 确保与 pin_cache_mappings() 的默认实现保持一致
        let normalized_inputs = inputs
            .into_iter()
            .map(|m| {
                PinMapping::new(
                    m.pin_name.clone(),
                    format!("End:{}", m.pin_name), // 使用标准格式
                    m.type_name.clone(),
                )
            })
            .collect();

        Self {
            inputs: normalized_inputs,
        }
    }
}

impl BlueprintNode for EndNode {
    fn name(&self) -> &str {
        "End"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::exec_in("In")];
        for mapping in &self.inputs {
            pins.push(Pin::data_in(&mapping.pin_name, &mapping.type_name));
        }
        pins
    }

    fn description(&self) -> Option<&str> {
        Some("End node - execution terminator")
    }

    fn category(&self) -> Option<&str> {
        Some("Flow")
    }

    // 注意：不需要 override pin_cache_mappings()
    // 默认实现会自动生成 "End:{pin_name}" 格式的 cache_key
    // 这与 with_inputs() 中规范化的格式完全一致

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

impl EndNode {
    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 将收到的输入数据存储到 cache
        // self.inputs 中的 cache_key 已经在构造时规范化为 "End:{pin_name}" 格式
        for mapping in &self.inputs {
            if let Some(value) = inputs.get(&mapping.pin_name) {
                ctx.set_cached(&mapping.cache_key, value).await?;
            }
        }

        ctx.set_cached(WORKFLOW_STATE_KEY, &WorkflowState::Idle)
            .await?;
        Ok(NodeOutput::Complete)
    }
}
