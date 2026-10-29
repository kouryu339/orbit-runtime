use super::*;
use corework::system::SystemRegistry;

impl RuntimeFacade {
    pub fn tool_definitions(&self) -> Result<String, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }

        let mut tools = BTreeMap::<String, Value>::new();
        for metadata in SystemRegistry::list_ai_systems() {
            tools.insert(metadata.name.to_string(), static_tool_definition(metadata));
        }
        // RPC discovery is append-only; these names cannot replace built-ins.
        for metadata in &self.runtime_tools {
            tools.insert(metadata.name.clone(), runtime_tool_definition(metadata));
        }

        serde_json::to_string(&json!({
            "schema": "agent-runtime-tool-definitions/v1",
            "catalog_scope": "runtime_registered",
            "authorization_scope": "definitions_only",
            "catalog_revision": self.tool_registry.runtime_catalog_revision(),
            "tools": tools.into_values().collect::<Vec<_>>()
        }))
        .map_err(|error| {
            RuntimeError::Internal(format!("serialize tool definitions failed: {error}"))
        })
    }
}

use corework::tool_definitions::{runtime_tool_definition, static_tool_definition};
