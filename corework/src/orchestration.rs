//! Execution context and orchestrator helpers.

use crate::cache::Cache;
use crate::error::{FrameworkError, Result};
use crate::event::EventBus;
use crate::event_line::{EventLineHandle, EventLinePolicy};
use crate::execution_unit::ExecutionUnit;
use crate::hierarchical_cache::HierarchicalCache;
use crate::monitoring::Telemetry;
use crate::runtime_state::RuntimeStateStore;
use crate::system::SystemRegistry;
use crate::world::OrchestrationWorld;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use uuid::Uuid;

#[derive(Clone)]
pub struct Context {
    pub request_id: String,
    pub scope_id: Option<String>,
    /// Current execution unit cache namespace, if the ctx was created by an ExecutionUnit.
    pub cache_scope_id: Option<String>,
    /// Conversation id, if this context belongs to a conversation-scoped execution unit.
    pub conversation_id: Option<String>,
    /// Current execution unit, used when systems create real child units.
    execution_unit: Option<Weak<ExecutionUnit>>,
    pub data: Arc<parking_lot::RwLock<HashMap<String, serde_json::Value>>>,
    pub cache: Arc<dyn Cache>,
    /// Explicit execution-unit cache hierarchy, when bound to an ExecutionUnit.
    pub hierarchical_cache: Option<Arc<HierarchicalCache>>,
    runtime_state: Option<Arc<dyn RuntimeStateStore>>,
    /// Event bus for this context.
    pub event_bus: Arc<dyn EventBus>,
    pub world_event_bus: Arc<dyn EventBus>,
    /// Return one value from local context data.
    pub telemetry: Arc<dyn Telemetry>,
    registry: Arc<SystemRegistry>,
    world: Option<Arc<OrchestrationWorld>>,
}

impl Context {
    pub fn new(
        cache: Arc<dyn Cache>,
        event_bus: Arc<dyn EventBus>,
        telemetry: Arc<dyn Telemetry>,
    ) -> Self {
        let world_event_bus = event_bus.clone();
        Self {
            request_id: Uuid::new_v4().to_string(),
            scope_id: None,
            cache_scope_id: None,
            conversation_id: None,
            execution_unit: None,
            data: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            cache,
            hierarchical_cache: None,
            runtime_state: None,
            event_bus,
            world_event_bus,
            telemetry,
            registry: Arc::new(SystemRegistry::new()),
            world: None,
        }
    }

    pub fn with_registry(
        cache: Arc<dyn Cache>,
        event_bus: Arc<dyn EventBus>,
        telemetry: Arc<dyn Telemetry>,
        registry: Arc<SystemRegistry>,
    ) -> Self {
        let world_event_bus = event_bus.clone();
        Self {
            request_id: Uuid::new_v4().to_string(),
            scope_id: None,
            cache_scope_id: None,
            conversation_id: None,
            execution_unit: None,
            data: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            cache,
            hierarchical_cache: None,
            runtime_state: None,
            event_bus,
            world_event_bus,
            telemetry,
            registry,
            world: None,
        }
    }

    pub fn set_registry(mut self, registry: Arc<SystemRegistry>) -> Self {
        self.registry = registry;
        self
    }

    pub fn with_scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = Some(scope_id.into());
        self
    }

    /// Bind the cache namespace for diagnostics and explicit state ownership.
    pub fn with_cache_scope_id(mut self, cache_scope_id: impl Into<String>) -> Self {
        self.cache_scope_id = Some(cache_scope_id.into());
        self
    }

    /// Bind the current conversation id.
    pub fn with_conversation_id(mut self, conversation_id: impl Into<String>) -> Self {
        self.conversation_id = Some(conversation_id.into());
        self
    }

    /// Bind the current conversation id when available.
    pub fn with_optional_conversation_id(mut self, conversation_id: Option<String>) -> Self {
        self.conversation_id = conversation_id;
        self
    }

    pub fn with_execution_unit(mut self, unit: &Arc<ExecutionUnit>) -> Self {
        self.execution_unit = Some(Arc::downgrade(unit));
        self
    }

    pub fn execution_unit(&self) -> Option<Arc<ExecutionUnit>> {
        self.execution_unit.as_ref().and_then(Weak::upgrade)
    }

    pub fn ecs_world(&self) -> Option<Arc<crate::ecs::EcsWorld>> {
        self.world.as_ref().map(|world| world.ecs())
    }

    pub fn resolve_shared_component<T>(&self) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.execution_unit()
            .and_then(|unit| unit.resolve_shared_component::<T>())
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "shared component '{}' is not available from the current execution unit",
                    std::any::type_name::<T>()
                ))
            })
    }

    pub fn create_event_line(
        &self,
        name: impl Into<String>,
        policy: EventLinePolicy,
    ) -> Result<EventLineHandle> {
        self.execution_unit()
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(
                    "context is not bound to an execution unit".to_string(),
                )
            })?
            .create_event_line(name, policy)
    }

    pub fn event_line(&self, name: &str) -> Result<EventLineHandle> {
        self.execution_unit()
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(
                    "context is not bound to an execution unit".to_string(),
                )
            })?
            .event_line(name)
    }

    pub fn with_hierarchical_cache(mut self, cache: Arc<HierarchicalCache>) -> Self {
        self.hierarchical_cache = Some(cache);
        self
    }

    pub fn with_runtime_state(mut self, state: Arc<dyn RuntimeStateStore>) -> Self {
        self.runtime_state = Some(state);
        self
    }

    pub fn state(&self) -> Option<Arc<dyn RuntimeStateStore>> {
        self.runtime_state.clone()
    }

    pub fn with_world(
        cache: Arc<dyn Cache>,
        event_bus: Arc<dyn EventBus>,
        telemetry: Arc<dyn Telemetry>,
        registry: Arc<SystemRegistry>,
        world: Arc<OrchestrationWorld>,
        world_event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            scope_id: None,
            cache_scope_id: None,
            conversation_id: None,
            execution_unit: None,
            data: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            cache,
            hierarchical_cache: None,
            runtime_state: None,
            event_bus,
            world_event_bus,
            telemetry,
            registry,
            world: Some(world),
        }
    }

    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let save_op: Arc<SaveOrderOperation> = ctx.system("save_order")?;
    /// let result = save_op.execute(order, &ctx).await?;
    /// ```
    pub fn system<T>(&self, name: &str) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.registry.get(name).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("System operation '{}' not found", name))
        })
    }

    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let save_op = ctx.system_by_type::<SaveOrderOperation>()?;
    /// let result = save_op.execute(order, &ctx).await?;
    /// ```
    pub fn system_by_type<T>(&self) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.registry.get_by_type().ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "System operation '{}' not found",
                std::any::type_name::<T>()
            ))
        })
    }
    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let system = ctx.get_dynamic_system("ValidateOrder")?;
    /// let output = system.execute_dynamic(input_map, &ctx).await?;
    /// ```
    pub fn get_dynamic_system(
        &self,
        name: &str,
    ) -> Result<Arc<dyn crate::workflow::dynamic_node::DynamicExecute>> {
        self.registry.get_dynamic(name).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("System operation '{}' not found", name))
        })
    }

    pub(crate) fn get_registry(&self) -> Arc<SystemRegistry> {
        self.registry.clone()
    }
    pub fn create_node_by_type(
        &self,
        node_type: &str,
        name: impl Into<String>,
    ) -> Result<Arc<dyn crate::workflow::nodes::traits::BlueprintNode + Send + Sync>> {
        crate::workflow::registry::node_registry::NodeRegistry::create_node_internal(
            node_type, name,
        )
        .ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("Node type {} not registered", node_type))
        })
    }

    pub fn set<T: serde::Serialize>(&self, key: impl Into<String>, value: T) -> Result<()> {
        let json_value = serde_json::to_value(value)?;
        self.data.write().insert(key.into(), json_value);
        Ok(())
    }

    /// Return one value from local context data.
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let data = self.data.read();
        if let Some(value) = data.get(key) {
            let result = serde_json::from_value(value.clone())?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.data.read().contains_key(key)
    }

    /// Return one value from local context data.
    pub fn remove(&self, key: &str) -> Option<serde_json::Value> {
        self.data.write().remove(key)
    }

    pub fn clear(&self) {
        self.data.write().clear();
    }

    pub async fn reset(&self) -> Result<()> {
        self.clear();
        self.cache.flush().await?;
        Ok(())
    }

    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let cache = ctx.get_cache();
    /// cache.get("temp_result").await?;
    /// cache.set("temp_result", &value, ttl).await?;
    /// ```
    pub fn get_cache(&self) -> &Arc<dyn Cache> {
        &self.cache
    }

    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let world = ctx.get_world_cache()?;
    /// world.get_resource::<Config>("app_config")?;
    /// world.set_resource("user:123", &user, ttl)?;
    /// ```
    pub fn get_world_cache(&self) -> Result<&Arc<OrchestrationWorld>> {
        self.world.as_ref().ok_or_else(|| {
            FrameworkError::InvalidOperation("World not available in this context".to_string())
        })
    }
}

pub struct Orchestrator {
    cache: Arc<dyn Cache>,
    event_bus: Arc<dyn EventBus>,
    telemetry: Arc<dyn Telemetry>,
}

impl Orchestrator {
    pub fn builder() -> OrchestratorBuilder {
        OrchestratorBuilder::new()
    }

    pub fn create_context(&self) -> Context {
        Context::new(
            self.cache.clone(),
            self.event_bus.clone(),
            self.telemetry.clone(),
        )
    }
}

pub struct OrchestratorBuilder {
    cache: Option<Arc<dyn Cache>>,
    event_bus: Option<Arc<dyn EventBus>>,
    telemetry: Option<Arc<dyn Telemetry>>,
}

impl OrchestratorBuilder {
    pub fn new() -> Self {
        Self {
            cache: None,
            event_bus: None,
            telemetry: None,
        }
    }

    pub fn with_cache(mut self, cache: impl Cache + 'static) -> Self {
        self.cache = Some(Arc::new(cache));
        self
    }

    pub fn with_event_bus(mut self, event_bus: impl EventBus + 'static) -> Self {
        self.event_bus = Some(Arc::new(event_bus));
        self
    }

    pub fn with_telemetry(mut self, telemetry: impl Telemetry + 'static) -> Self {
        self.telemetry = Some(Arc::new(telemetry));
        self
    }

    pub fn build(self) -> Orchestrator {
        let cache = self
            .cache
            .unwrap_or_else(|| Arc::new(crate::cache::InMemoryCache::new()));
        let event_bus = self
            .event_bus
            .unwrap_or_else(|| Arc::new(crate::event::InMemoryEventBus::new()));
        let telemetry = self
            .telemetry
            .unwrap_or_else(|| Arc::new(crate::monitoring::NoopTelemetry));

        Orchestrator {
            cache,
            event_bus,
            telemetry,
        }
    }
}

impl Default for OrchestratorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_data() {
        let cache = Arc::new(crate::cache::InMemoryCache::new());
        let event_bus = Arc::new(crate::event::InMemoryEventBus::new());
        let telemetry = Arc::new(crate::monitoring::NoopTelemetry);

        let ctx = Context::new(cache, event_bus, telemetry);

        ctx.set("key1", "value1").unwrap();
        let value: Option<String> = ctx.get("key1").unwrap();
        assert_eq!(value, Some("value1".to_string()));

        assert!(ctx.contains("key1"));
        assert!(!ctx.contains("key2"));

        ctx.remove("key1");
        assert!(!ctx.contains("key1"));
    }

    #[test]
    fn test_orchestrator_builder() {
        let cache = crate::cache::InMemoryCache::new();
        let orchestrator = Orchestrator::builder().with_cache(cache).build();

        let ctx = orchestrator.create_context();
        assert!(!ctx.request_id.is_empty());
    }
}
