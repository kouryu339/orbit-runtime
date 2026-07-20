//! Shared orchestration world and framework-level singletons.

use crate::ecs::EcsWorld;
use crate::error::{FrameworkError, Result};
use dashmap::DashMap;
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// ============================================================================

#[derive(Clone)]
struct ResourceEntry {
    value: serde_json::Value,
    version: u64,
    expires_at: Option<Instant>,
}

impl ResourceEntry {
    fn new(value: serde_json::Value, ttl: Option<Duration>) -> Self {
        Self {
            value,
            version: 0,
            expires_at: ttl.map(|d| Instant::now() + d),
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Instant::now() > exp)
            .unwrap_or(false)
    }
}

struct InstanceState {
    /// Per-instance data isolated by orchestration instance id.
    data: DashMap<String, serde_json::Value>,
    generation: u64,
    created_at: Instant,
}

impl InstanceState {
    fn new() -> Self {
        Self {
            data: DashMap::new(),
            generation: 0,
            created_at: Instant::now(),
        }
    }
}

// ============================================================================
// ============================================================================

/// Central store for shared resources and per-instance data.
///
/// Resources are global across orchestration instances. Instance data is scoped
/// by instance id and is cleaned up with the owning `InstanceHandle`.
/// ```rust,no_run
/// use corework::prelude::*;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<()> {
/// let world = Arc::new(OrchestrationWorld::new());
///
/// world.set_resource("config", &MyConfig::default(), None)?;
///
/// let instance_1 = "workflow_001";
/// let config: MyConfig = world.get_resource("config")?.unwrap();
///
/// let instance_2 = "workflow_002";
/// # Ok(())
/// # }
/// # #[derive(serde::Serialize, serde::Deserialize, Default)]
/// # struct MyConfig;
/// ```
pub struct OrchestrationWorld {
    resources: Arc<DashMap<String, ResourceEntry>>,
    instances: Arc<DashMap<String, InstanceState>>,
    ecs: Arc<EcsWorld>,
}

impl OrchestrationWorld {
    pub fn new() -> Self {
        Self {
            resources: Arc::new(DashMap::new()),
            instances: Arc::new(DashMap::new()),
            ecs: Arc::new(EcsWorld::new()),
        }
    }

    pub fn ecs(&self) -> Arc<EcsWorld> {
        self.ecs.clone()
    }

    // ========================================================================
    // ========================================================================

    /// Get a global resource if it exists and has not expired.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # async fn example(world: &OrchestrationWorld) -> Result<()> {
    /// let user: User = world.get_resource("user:123")?.unwrap();
    /// # Ok(())
    /// # }
    /// # #[derive(serde::Deserialize)]
    /// # struct User;
    /// ```
    pub fn get_resource<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        if let Some(entry) = self.resources.get(key) {
            if entry.is_expired() {
                drop(entry);
                self.resources.remove(key);
                return Ok(None);
            }

            let value: T = serde_json::from_value(entry.value.clone())
                .map_err(FrameworkError::SerializationError)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Set a global resource with an optional TTL.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # use std::time::Duration;
    /// # async fn example(world: &OrchestrationWorld) -> Result<()> {
    /// # let user = User;
    /// world.set_resource("user:123", &user, Some(Duration::from_secs(600)))?;
    /// # Ok(())
    /// # }
    /// # #[derive(serde::Serialize)]
    /// # struct User;
    /// ```
    pub fn set_resource<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let json = serde_json::to_value(value).map_err(FrameworkError::SerializationError)?;

        let entry = ResourceEntry::new(json, ttl);
        self.resources.insert(key.to_string(), entry);
        Ok(())
    }

    pub fn delete_resource(&self, key: &str) -> bool {
        self.resources.remove(key).is_some()
    }

    /// Update a resource only when its version matches `expected_version`.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # async fn example(world: &OrchestrationWorld) -> Result<()> {
    /// world.update_resource("counter", 0, |count: i32| count + 1)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn update_resource<T, F>(
        &self,
        key: &str,
        expected_version: u64,
        updater: F,
    ) -> Result<bool>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce(T) -> T,
    {
        if let Some(mut entry) = self.resources.get_mut(key) {
            if entry.version == expected_version {
                let old_value: T = serde_json::from_value(entry.value.clone())
                    .map_err(FrameworkError::SerializationError)?;

                let new_value = updater(old_value);

                entry.value =
                    serde_json::to_value(new_value).map_err(FrameworkError::SerializationError)?;
                entry.version += 1;

                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn get_resource_version(&self, key: &str) -> Option<u64> {
        self.resources.get(key).map(|entry| entry.version)
    }

    // ========================================================================
    // ========================================================================

    pub(crate) fn register_instance(&self, instance_id: &str) {
        self.instances
            .insert(instance_id.to_string(), InstanceState::new());
    }

    /// Get data scoped to one orchestration instance.
    pub fn get_instance_data<T: DeserializeOwned>(
        &self,
        instance_id: &str,
        key: &str,
    ) -> Result<Option<T>> {
        if let Some(instance) = self.instances.get(instance_id) {
            if let Some(value) = instance.data.get(key) {
                let result: T = serde_json::from_value(value.clone())
                    .map_err(FrameworkError::SerializationError)?;
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    pub fn set_instance_data<T: Serialize>(
        &self,
        instance_id: &str,
        key: &str,
        value: &T,
    ) -> Result<()> {
        let json = serde_json::to_value(value).map_err(FrameworkError::SerializationError)?;

        if let Some(instance) = self.instances.get(instance_id) {
            instance.data.insert(key.to_string(), json);
            Ok(())
        } else {
            Err(FrameworkError::InvalidOperation(format!(
                "Instance {} not found",
                instance_id
            )))
        }
    }

    pub fn delete_instance_data(&self, instance_id: &str, key: &str) -> bool {
        self.instances
            .get(instance_id)
            .and_then(|instance| instance.data.remove(key))
            .is_some()
    }

    pub(crate) fn cleanup_instance(&self, instance_id: &str) {
        if let Some((_, state)) = self.instances.remove(instance_id) {
            let lifetime = state.created_at.elapsed();
            tracing::debug!(
                instance_id = instance_id,
                lifetime_ms = lifetime.as_millis(),
                data_count = state.data.len(),
                "Instance cleaned up"
            );
        }
    }

    pub fn is_instance_alive(&self, instance_id: &str) -> bool {
        self.instances.contains_key(instance_id)
    }

    /// Return the current generation for one orchestration instance.
    pub fn get_instance_generation(&self, instance_id: &str) -> Option<u64> {
        self.instances
            .get(instance_id)
            .map(|state| state.generation)
    }

    // ========================================================================
    // ========================================================================

    /// Query instance data first, then fall back to global resources.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use corework::prelude::*;
    /// # async fn example(world: &OrchestrationWorld) -> Result<()> {
    /// let value: MyData = world.query("workflow_001", "some_key")?.unwrap();
    /// # Ok(())
    /// # }
    /// # #[derive(serde::Deserialize)]
    /// # struct MyData;
    /// ```
    pub fn query<T: DeserializeOwned>(&self, instance_id: &str, key: &str) -> Result<Option<T>> {
        if let Some(value) = self.get_instance_data(instance_id, key)? {
            return Ok(Some(value));
        }

        self.get_resource(key)
    }

    // ========================================================================
    // ========================================================================

    /// Return the number of global resources.
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Return the number of live orchestration instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    pub fn cleanup_expired_resources(&self) -> usize {
        let mut removed = 0;
        self.resources.retain(|_, entry| {
            let expired = entry.is_expired();
            if expired {
                removed += 1;
            }
            !expired
        });
        removed
    }
}

impl Default for OrchestrationWorld {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestData {
        value: String,
    }

    #[test]
    fn test_resource_operations() {
        let world = OrchestrationWorld::new();

        let data = TestData {
            value: "test".to_string(),
        };
        world.set_resource("key1", &data, None).unwrap();

        let retrieved: TestData = world.get_resource("key1").unwrap().unwrap();
        assert_eq!(retrieved, data);

        assert!(world.delete_resource("key1"));
        assert!(world.get_resource::<TestData>("key1").unwrap().is_none());
    }

    #[test]
    fn test_instance_isolation() {
        let world = OrchestrationWorld::new();

        world.register_instance("instance1");
        world.register_instance("instance2");

        let data1 = TestData {
            value: "data1".to_string(),
        };
        let data2 = TestData {
            value: "data2".to_string(),
        };

        world.set_instance_data("instance1", "key", &data1).unwrap();
        world.set_instance_data("instance2", "key", &data2).unwrap();

        let retrieved1: TestData = world
            .get_instance_data("instance1", "key")
            .unwrap()
            .unwrap();
        let retrieved2: TestData = world
            .get_instance_data("instance2", "key")
            .unwrap()
            .unwrap();

        assert_eq!(retrieved1.value, "data1");
        assert_eq!(retrieved2.value, "data2");
    }

    #[test]
    fn test_query_fallback() {
        let world = OrchestrationWorld::new();
        world.register_instance("instance1");

        let global_data = TestData {
            value: "global".to_string(),
        };
        let instance_data = TestData {
            value: "instance".to_string(),
        };

        world.set_resource("key1", &global_data, None).unwrap();

        let result: TestData = world.query("instance1", "key1").unwrap().unwrap();
        assert_eq!(result.value, "global");

        world
            .set_instance_data("instance1", "key1", &instance_data)
            .unwrap();

        let result: TestData = world.query("instance1", "key1").unwrap().unwrap();
        assert_eq!(result.value, "instance");
    }

    #[test]
    fn test_ttl() {
        let world = OrchestrationWorld::new();

        let data = TestData {
            value: "test".to_string(),
        };

        world
            .set_resource("key1", &data, Some(Duration::from_millis(10)))
            .unwrap();

        assert!(world.get_resource::<TestData>("key1").unwrap().is_some());

        std::thread::sleep(Duration::from_millis(20));

        assert!(world.get_resource::<TestData>("key1").unwrap().is_none());
    }

    #[test]
    fn test_atomic_update() {
        let world = OrchestrationWorld::new();

        world.set_resource("counter", &0i32, None).unwrap();

        let version = world.get_resource_version("counter").unwrap();

        let success = world
            .update_resource("counter", version, |count: i32| count + 1)
            .unwrap();
        assert!(success);

        let count: i32 = world.get_resource("counter").unwrap().unwrap();
        assert_eq!(count, 1);

        let success = world
            .update_resource("counter", version, |count: i32| count + 1)
            .unwrap();
        assert!(!success);
    }

    #[tokio::test]
    async fn framework_context_exposes_runtime_state_store() {
        let framework = FrameworkState::initialize().unwrap();
        let ctx = framework.create_context();
        let state = ctx.state().expect("framework context should expose state");
        let scope = crate::runtime_state::StateScope::local(
            "unit:runtime-state-test",
            "scope:runtime-state-test",
        );

        state
            .kv()
            .set_raw(&scope, "value", serde_json::json!({"ok": true}), None)
            .await
            .unwrap();

        assert_eq!(
            state.kv().get_raw(&scope, "value").await.unwrap(),
            Some(serde_json::json!({"ok": true}))
        );
    }
}
// ============================================================================

use crate::cache::{create_cache_backend, Cache, CacheBackendConfig};
use crate::event::{EventBus, InMemoryEventBus};
use crate::monitoring::{NoopTelemetry, Telemetry};
use crate::orchestration::Context;
use crate::runtime_state::{create_runtime_state_store, RuntimeStateConfig, RuntimeStateStore};
use crate::system::SystemRegistry;

static GLOBAL_WORLD: std::sync::OnceLock<Arc<OrchestrationWorld>> = std::sync::OnceLock::new();
static GLOBAL_REGISTRY: std::sync::OnceLock<Arc<SystemRegistry>> = std::sync::OnceLock::new();
static GLOBAL_EVENT_BUS: std::sync::OnceLock<Arc<InMemoryEventBus>> = std::sync::OnceLock::new();
static GLOBAL_TELEMETRY: std::sync::OnceLock<Arc<NoopTelemetry>> = std::sync::OnceLock::new();
static GLOBAL_CACHE: std::sync::OnceLock<Arc<dyn Cache>> = std::sync::OnceLock::new();
static GLOBAL_RUNTIME_STATE: std::sync::OnceLock<(RuntimeStateConfig, Arc<dyn RuntimeStateStore>)> =
    std::sync::OnceLock::new();

/// Framework-level handles shared by integrations such as Tauri or Axum.
#[derive(Clone)]
pub struct FrameworkState {
    pub world: Arc<OrchestrationWorld>,
    pub registry: Arc<SystemRegistry>,
    pub event_bus: Arc<InMemoryEventBus>,
    pub telemetry: Arc<NoopTelemetry>,
    pub cache: Arc<dyn Cache>,
    /// Internal runtime state store.
    pub runtime_state: Arc<dyn RuntimeStateStore>,
}

impl FrameworkState {
    /// Initialize the global framework state.
    /// ```rust
    /// use corework::prelude::*;
    ///
    /// let _framework = FrameworkState::initialize().unwrap();
    /// ```
    pub fn initialize() -> Result<Self> {
        Self::initialize_with_cache_backend(CacheBackendConfig::default())
    }

    pub fn initialize_with_cache_backend(cache_config: CacheBackendConfig) -> Result<Self> {
        Self::initialize_with_runtime_state_config(cache_config, RuntimeStateConfig::default())
    }

    pub fn initialize_with_runtime_state_config(
        cache_config: CacheBackendConfig,
        runtime_state_config: RuntimeStateConfig,
    ) -> Result<Self> {
        let world = GLOBAL_WORLD
            .get_or_init(|| Arc::new(OrchestrationWorld::new()))
            .clone();

        let registry = GLOBAL_REGISTRY
            .get_or_init(|| {
                let reg = Arc::new(SystemRegistry::new());
                reg.auto_register_all();
                reg
            })
            .clone();

        let event_bus = GLOBAL_EVENT_BUS
            .get_or_init(|| Arc::new(InMemoryEventBus::new()))
            .clone();
        let telemetry = GLOBAL_TELEMETRY
            .get_or_init(|| Arc::new(NoopTelemetry))
            .clone();
        let cache = match GLOBAL_CACHE.get() {
            Some(cache) => cache.clone(),
            None => {
                let cache = create_cache_backend(cache_config)?;
                let _ = GLOBAL_CACHE.set(cache.clone());
                cache
            }
        };
        if let Some((active_config, _)) = GLOBAL_RUNTIME_STATE.get() {
            if *active_config != runtime_state_config {
                return Err(FrameworkError::ConfigError(format!(
                    "runtime state backend is already initialized as {:?}, requested {:?}",
                    active_config.backend, runtime_state_config.backend
                )));
            }
        }
        let candidate =
            create_runtime_state_store(runtime_state_config, cache.clone(), world.ecs());
        let _ = GLOBAL_RUNTIME_STATE.set((runtime_state_config, candidate));
        let (active_config, runtime_state) = GLOBAL_RUNTIME_STATE
            .get()
            .expect("runtime state must be initialized");
        if *active_config != runtime_state_config {
            return Err(FrameworkError::ConfigError(format!(
                "runtime state backend initialized concurrently as {:?}, requested {:?}",
                active_config.backend, runtime_state_config.backend
            )));
        }
        let runtime_state = runtime_state.clone();

        Ok(Self {
            world,
            registry,
            event_bus,
            telemetry,
            cache,
            runtime_state,
        })
    }

    /// Create a standard execution context backed by the global framework state.
    ///
    /// # Example
    /// ```rust
    /// use corework::prelude::*;
    ///
    /// let framework = FrameworkState::initialize().unwrap();
    /// let _ctx = framework.create_context();
    /// ```
    pub fn create_context(&self) -> Context {
        use crate::instance::InstanceHandle;

        let handle = InstanceHandle::new(self.world.clone());
        handle
            .context_with_registry(
                self.event_bus.clone() as Arc<dyn EventBus>,
                self.telemetry.clone() as Arc<dyn Telemetry>,
                self.registry.clone(),
            )
            .with_runtime_state(self.runtime_state())
    }

    /// Return the shared world.
    pub fn world(&self) -> Arc<OrchestrationWorld> {
        self.world.clone()
    }

    /// Return the shared system registry.
    pub fn registry(&self) -> Arc<SystemRegistry> {
        self.registry.clone()
    }

    /// Return the shared event bus.
    pub fn event_bus(&self) -> Arc<InMemoryEventBus> {
        self.event_bus.clone()
    }

    /// Return the shared telemetry handle.
    pub fn telemetry(&self) -> Arc<dyn Telemetry> {
        self.telemetry.clone() as Arc<dyn Telemetry>
    }

    /// Return the shared cache backend.
    pub fn cache(&self) -> Arc<dyn Cache> {
        self.cache.clone()
    }

    pub fn runtime_state(&self) -> Arc<dyn RuntimeStateStore> {
        self.runtime_state.clone()
    }

    pub fn shutdown(&self) {
        self.world.instances.clear();
        tracing::debug!("[FrameworkState] cleared all instances");
    }
}

impl Default for FrameworkState {
    fn default() -> Self {
        Self::initialize().expect("framework initialization failed")
    }
}
