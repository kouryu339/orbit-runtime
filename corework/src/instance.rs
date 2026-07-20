//! RAII handle for orchestration instance lifetime and scoped contexts.
//!
//! `InstanceHandle` registers an instance when it is created and removes it
//! from the world when the handle is dropped.

use crate::event::EventBus;
use crate::monitoring::Telemetry;
use crate::orchestration::Context;
use crate::system::SystemRegistry;
use crate::world::OrchestrationWorld;
use std::marker::PhantomData;
use std::sync::Arc;

// ============================================================================
// ============================================================================

///
///
///
/// ```text
/// InstanceHandle::new()
/// World.register_instance()  // register instance
/// Drop
/// ```
///
/// ## Example
///
///
/// ```rust
/// # use corework::prelude::*;
/// # use std::sync::Arc;
/// let world = Arc::new(OrchestrationWorld::new());
/// let handle = InstanceHandle::new(world);
/// let _id = handle.id();
/// ```
///
///
/// ```rust
/// # use corework::prelude::*;
/// # use std::sync::Arc;
/// let world = Arc::new(OrchestrationWorld::new());
/// let handle = Arc::new(InstanceHandle::with_name(world, "saga_001"));
/// let event_bus = Arc::new(InMemoryEventBus::new());
/// let telemetry = Arc::new(NoopTelemetry);
///
/// let _step_1_context = handle.context(event_bus.clone(), telemetry.clone());
/// let _step_2_context = handle.context(event_bus, telemetry);
/// ```
pub struct InstanceHandle {
    /// Instance id.
    instance_id: String,
    world: Arc<OrchestrationWorld>,
    _phantom: PhantomData<*const ()>,
}

impl InstanceHandle {
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # use std::sync::Arc;
    /// let world = Arc::new(OrchestrationWorld::new());
    /// let handle = InstanceHandle::new(world);
    /// println!("Instance ID: {}", handle.id());
    /// ```
    pub fn new(world: Arc<OrchestrationWorld>) -> Self {
        let instance_id = uuid::Uuid::new_v4().to_string();

        world.register_instance(&instance_id);

        tracing::debug!(
            instance_id = %instance_id,
            "Instance created"
        );

        Self {
            instance_id,
            world,
            _phantom: PhantomData,
        }
    }

    ///
    ///
    ///
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # use std::sync::Arc;
    /// let world = Arc::new(OrchestrationWorld::new());
    /// let handle = InstanceHandle::with_name(world, "order_processing");
    /// // ID example: "order_processing:550e8400-e29b-41d4-a716-446655440000"
    /// ```
    pub fn with_name(world: Arc<OrchestrationWorld>, name: &str) -> Self {
        let instance_id = format!("{}:{}", name, uuid::Uuid::new_v4());

        world.register_instance(&instance_id);

        tracing::debug!(
            instance_id = %instance_id,
            name = name,
            "Named instance created"
        );

        Self {
            instance_id,
            world,
            _phantom: PhantomData,
        }
    }

    /// Instance id.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # use std::sync::Arc;
    /// # let world = Arc::new(OrchestrationWorld::new());
    /// let handle = InstanceHandle::with_name(world, "test");
    /// println!("Instance ID: {}", handle.id());
    /// ```
    pub fn id(&self) -> &str {
        &self.instance_id
    }

    pub fn world(&self) -> &Arc<OrchestrationWorld> {
        &self.world
    }

    ///
    ///
    /// - `event_bus`: event bus used by the created context.
    ///
    ///
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use corework::prelude::*;
    /// # use std::sync::Arc;
    /// let world = Arc::new(OrchestrationWorld::new());
    /// let handle = InstanceHandle::new(world);
    /// let event_bus = Arc::new(InMemoryEventBus::new());
    /// let telemetry = Arc::new(NoopTelemetry);
    ///
    /// let _ctx = handle.context(event_bus, telemetry);
    /// ```
    pub fn context(&self, event_bus: Arc<dyn EventBus>, telemetry: Arc<dyn Telemetry>) -> Context {
        // Create a scoped cache for this instance
        let scoped_cache = Arc::new(crate::scoped_cache::ScopedCache::new(
            Arc::new(crate::cache::InMemoryCache::new()),
            &self.instance_id,
        ));

        Context::new(scoped_cache, event_bus, telemetry)
    }

    ///
    /// ## Example
    ///
    /// ```rust
    /// # use corework::prelude::*;
    /// # use std::sync::Arc;
    /// let registry = Arc::new(SystemRegistry::new());
    /// let world = Arc::new(OrchestrationWorld::new());
    /// let handle = InstanceHandle::new(world);
    /// let event_bus = Arc::new(InMemoryEventBus::new());
    /// let telemetry = Arc::new(NoopTelemetry);
    ///
    /// let _ctx = handle.context_with_registry(event_bus, telemetry, registry);
    /// ```
    pub fn context_with_registry(
        &self,
        event_bus: Arc<dyn EventBus>,
        telemetry: Arc<dyn Telemetry>,
        registry: Arc<SystemRegistry>,
    ) -> Context {
        let scoped_cache = Arc::new(crate::scoped_cache::ScopedCache::new(
            Arc::new(crate::cache::InMemoryCache::new()),
            &self.instance_id,
        ));

        let world_event_bus = event_bus.clone();

        Context::with_world(
            scoped_cache,
            event_bus,
            telemetry,
            registry,
            self.world.clone(),
            world_event_bus,
        )
    }

    pub fn is_alive(&self) -> bool {
        self.world.is_instance_alive(&self.instance_id)
    }
}

impl Drop for InstanceHandle {
    fn drop(&mut self) {
        self.world.cleanup_instance(&self.instance_id);

        tracing::debug!(
            instance_id = %self.instance_id,
            "Instance dropped"
        );
    }
}

unsafe impl Send for InstanceHandle {}
unsafe impl Sync for InstanceHandle {}

// ============================================================================
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_creation() {
        let world = Arc::new(OrchestrationWorld::new());

        {
            let handle = InstanceHandle::new(world.clone());
            assert!(handle.is_alive());
            assert_eq!(world.instance_count(), 1);
        }

        assert_eq!(world.instance_count(), 0);
    }

    #[test]
    fn test_named_instance() {
        let world = Arc::new(OrchestrationWorld::new());

        let handle = InstanceHandle::with_name(world.clone(), "test");
        assert!(handle.id().starts_with("test:"));
        assert!(handle.is_alive());
    }

    #[test]
    fn test_shared_instance() {
        let world = Arc::new(OrchestrationWorld::new());

        let handle = Arc::new(InstanceHandle::new(world.clone()));

        let handle_clone = handle.clone();

        assert_eq!(world.instance_count(), 1);

        drop(handle);

        assert_eq!(world.instance_count(), 1);
        assert!(handle_clone.is_alive());

        drop(handle_clone);

        assert_eq!(world.instance_count(), 0);
    }

    #[test]
    fn test_instance_data_isolation() {
        let world = Arc::new(OrchestrationWorld::new());

        let handle1 = InstanceHandle::with_name(world.clone(), "instance1");
        let handle2 = InstanceHandle::with_name(world.clone(), "instance2");

        world
            .set_instance_data(handle1.id(), "key", &"value1")
            .unwrap();
        world
            .set_instance_data(handle2.id(), "key", &"value2")
            .unwrap();

        let val1: String = world
            .get_instance_data(handle1.id(), "key")
            .unwrap()
            .unwrap();
        let val2: String = world
            .get_instance_data(handle2.id(), "key")
            .unwrap()
            .unwrap();

        assert_eq!(val1, "value1");
        assert_eq!(val2, "value2");
    }
}
