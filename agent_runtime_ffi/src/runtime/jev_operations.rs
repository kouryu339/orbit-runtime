use super::*;
use corework::jev::{HttpJevClient, JevDefinition, JevManager, RegistryJevToolExecutor};

impl RuntimeFacade {
    pub fn configure_jev(
        &mut self,
        api_key: String,
        endpoint: Option<String>,
    ) -> Result<Value, RuntimeError> {
        if self.started {
            return Err(RuntimeError::InvalidConfig(
                "jev.configure must be called before runtime.start".to_string(),
            ));
        }
        if api_key.trim().is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "payload.api_key must not be empty".to_string(),
            ));
        }
        self.jev_api_key = Some(api_key);
        if let Some(endpoint) = endpoint {
            if endpoint.trim().is_empty() {
                return Err(RuntimeError::InvalidConfig(
                    "payload.endpoint must not be empty".to_string(),
                ));
            }
            self.jev_endpoint = endpoint;
        }
        Ok(json!({"configured": true, "endpoint": self.jev_endpoint}))
    }

    pub fn register_jev(&mut self, definition: JevDefinition) -> Result<Value, RuntimeError> {
        definition
            .validate()
            .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        if self
            .jev_definitions
            .iter()
            .any(|existing| existing.name == definition.name)
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "Jev definition '{}' is already registered",
                definition.name
            )));
        }
        if let Some(manager) = &self.jev_manager {
            manager
                .register(definition.clone())
                .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        }
        self.jev_definitions.push(definition.clone());
        Ok(json!({"registered": true, "jev": definition}))
    }

    pub fn list_jev(&self) -> Value {
        let definitions = self
            .jev_manager
            .as_ref()
            .map(|manager| manager.list())
            .unwrap_or_else(|| self.jev_definitions.clone());
        json!({"schema":"agent-runtime-jev-list/v1", "jev": definitions})
    }

    pub fn run_jev(
        &self,
        name: &str,
        task: &str,
        conversation_id: Option<String>,
        agent_id: Option<String>,
        parent_run_id: Option<String>,
    ) -> Result<Value, RuntimeError> {
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        let manager = self.jev_manager.as_ref().ok_or_else(|| {
            RuntimeError::InvalidConfig(
                "Jev is not configured; call jev.configure before runtime.start".to_string(),
            )
        })?;
        let outcome = self
            .rt
            .block_on(manager.run(name, task, conversation_id, agent_id, parent_run_id, None))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        serde_json::to_value(outcome).map_err(|error| RuntimeError::Internal(error.to_string()))
    }

    pub(super) fn install_jev_manager(&mut self) -> Result<(), RuntimeError> {
        let Some(api_key) = self.jev_api_key.clone() else {
            return Ok(());
        };
        let owner = self
            .conversation_manager
            .as_ref()
            .ok_or(RuntimeError::NotStarted)?
            .unit()
            .clone();
        let client = Arc::new(
            HttpJevClient::with_endpoint(api_key, self.jev_endpoint.clone())
                .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?,
        );
        let executor = Arc::new(RegistryJevToolExecutor);
        let event_bus: Arc<dyn EventBus> = self.event_bus.clone();
        let manager = Arc::new(JevManager::new(owner, client, executor, event_bus));
        for definition in self.jev_definitions.clone() {
            manager
                .register(definition)
                .map_err(|error| RuntimeError::InvalidConfig(error.to_string()))?;
        }
        self.jev_manager = Some(manager);
        Ok(())
    }
}
