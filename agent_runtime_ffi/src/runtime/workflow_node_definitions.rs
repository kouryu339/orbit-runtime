use super::*;

impl RuntimeFacade {
    pub fn workflow_reference_node(
        &self,
        workflow_id: &str,
        node_id: &str,
    ) -> Result<Value, RuntimeError> {
        if node_id.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "node_id must not be empty".into(),
            ));
        }
        self.workflow_module()?
            .workflow_reference_definition(workflow_id, node_id)
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))
    }

    pub fn workflow_node_definitions(&self) -> Result<String, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }

        let mut nodes = workflow_node_definition_values(&self.runtime_tools);
        if let Some(module) = &self.workflow_module {
            for definition in module
                .workflow_reference_definitions()
                .map_err(|error| RuntimeError::Internal(error.to_string()))?
            {
                let mut node = definition["node"].clone();
                node["category"] = json!("workflow/reference");
                node["source"] = json!("workflow");
                node["pure"] = json!(false);
                node["name"] = node["node_type"].clone();
                node["editor_callable"] = json!(true);
                node["native_category"] = json!("Workflow Reference");
                nodes.push(node);
            }
        }
        serde_json::to_string(&json!({
            "schema": "agent-runtime-workflow-node-definitions/v1",
            "catalog_scope": "runtime_registered",
            "nodes": nodes
        }))
        .map_err(|error| {
            RuntimeError::Internal(format!(
                "serialize workflow node definitions failed: {error}"
            ))
        })
    }
}

pub(crate) fn workflow_node_definition_values(runtime_tools: &[RuntimeToolMetadata]) -> Vec<Value> {
    corework::workflow::node_definitions::workflow_node_definition_values(
        runtime_tools,
        &AssistantContext::all_registered_tool_names(),
    )
}
