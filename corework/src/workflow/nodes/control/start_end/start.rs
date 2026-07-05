//! Start 节点 - 执行流入口

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use crate::workflow::workflow::{WorkflowState, WORKFLOW_STATE_KEY};
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PinMapping {
    pub pin_name: String,
    pub cache_key: String,
    pub type_name: String,
}

impl PinMapping {
    pub fn new(
        pin_name: impl Into<String>,
        cache_key: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        Self {
            pin_name: pin_name.into(),
            cache_key: cache_key.into(),
            type_name: type_name.into(),
        }
    }
}

/// Start 节点 - 执行流入口
#[derive(Debug, Clone)]
#[register_node(
    node_type = "Impure",
    version = "1.0.0",
    category = "Control Flow",
    display_name = "Start",
    description = "工作流入口，→{{Out}}触发执行",
    permissions = 0b00001100,  // CAN_ADD_OUTPUT_PIN | CAN_REMOVE_OUTPUT_PIN
    exec_out = ["Out@触发工作流开始执行"]
)]
#[derive(Default)]
pub struct StartNode {
    outputs: Vec<PinMapping>,
}

impl StartNode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_outputs(outputs: Vec<PinMapping>) -> Self {
        Self { outputs }
    }
}

impl BlueprintNode for StartNode {
    fn name(&self) -> &str {
        "Start"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::exec_out("Out")];
        for mapping in &self.outputs {
            pins.push(Pin::data_out(&mapping.pin_name, &mapping.type_name));
        }
        pins
    }

    fn description(&self) -> Option<&str> {
        Some("Start node - execution entry")
    }

    fn category(&self) -> Option<&str> {
        Some("Flow")
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

impl StartNode {
    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        _inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        ctx.set_cached(WORKFLOW_STATE_KEY, &WorkflowState::Running)
            .await?;

        for mapping in &self.outputs {
            if let Ok(Some(value)) = ctx.get_cached::<DataValue>(&mapping.cache_key).await {
                let key = format!("Start:{}", mapping.pin_name);
                ctx.set_cached(&key, &value).await?;
            }
        }
        Ok(NodeOutput::ExecPin("Out".to_string()))
    }
}
