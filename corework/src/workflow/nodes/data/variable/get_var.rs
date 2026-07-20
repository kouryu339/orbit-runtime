//! GetVarNode - variable reference node.
//!
//! Pure data source node. `Name` can be edited or connected; runtime reads remain
//! restricted to declared workflow variables and inputs.

use crate::error::Result;
use crate::register_node;
use crate::workflow::core::{DataValue, NodeOutput, Pin, PinCacheMapping};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::traits::{BlueprintNode, NodeType};
use std::collections::HashMap;

/// Reads the current value from the workflow variable scope.
#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Pure",
    version = "1.0.0",
    category = "Variable",
    display_name = "读取变量{Name}",
    description = "Read declared variable {{Name}}, -> {{Value}}",
    permissions = 0,
    data_in = ["Name:String@Declared variable or input name"],
    data_out = ["Value:Any@Current variable value"]
)]
pub struct GetVarNode {
    instance_key: String,
    default_name: String,
}

impl BlueprintNode for GetVarNode {
    fn name(&self) -> &str {
        "GetVar"
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Name", "String"),
            Pin::data_out("Value", "Any"),
        ]
    }

    fn pin_cache_mappings(&self) -> (Vec<PinCacheMapping>, Vec<PinCacheMapping>) {
        (
            vec![PinCacheMapping::new(
                "Name",
                format!("{}:defaults:Name", self.instance_key),
                "String",
            )],
            vec![PinCacheMapping::new(
                "Value",
                format!("{}:Value", self.instance_key),
                "Any",
            )],
        )
    }

    fn description(&self) -> Option<&str> {
        Some("Reads current value from a validated workflow variable slot")
    }

    fn category(&self) -> Option<&str> {
        Some("Variable")
    }

    fn execute_node<'a>(
        &'a self,
        ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move { self.execute(ctx, inputs).await })
    }
}

impl GetVarNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            instance_key: format!("__getvar__{}", name),
            default_name: name,
        }
    }

    pub fn with_instance(instance_key: impl Into<String>, default_name: impl Into<String>) -> Self {
        Self {
            instance_key: instance_key.into(),
            default_name: default_name.into(),
        }
    }

    pub fn evaluate(
        &self,
        _inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let mut outputs = HashMap::new();
        outputs.insert("Value".to_string(), DataValue::from_string(""));
        Ok(outputs)
    }

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let name = inputs
            .get("Name")
            .and_then(DataValue::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(self.default_name.as_str());
        if !ctx.is_workflow_variable_declared(name) {
            return Err(crate::error::FrameworkError::WorkflowError(format!(
                "GetVarNode can only read declared workflow variables or inputs, not '${}'",
                name
            )));
        }
        let value = ctx
            .get_workflow_variable(name)
            .await?
            .unwrap_or_else(|| DataValue::from_string(""));

        // 通过 NodeOutput::Data 输出，executor 写入 GetVar_nX:Value
        let mut outputs = HashMap::new();
        outputs.insert("Value".to_string(), value);
        Ok(NodeOutput::Data(outputs))
    }
}
