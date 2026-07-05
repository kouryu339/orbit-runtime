use crate::cache::Cache;
use crate::ecs::EcsWorld;
use crate::event_line::EventLinePolicy;
use crate::runtime_state::map_store::MapRuntimeStateStore;
use crate::runtime_state::snapshots::{EventLineStateSnapshot, SharedComponentStateSnapshot};
use crate::runtime_state::store::{
    EventStateStore, KeyValueStateStore, ResourceStateStore, RuntimeStateBackendKind,
    RuntimeStateStore, SharedComponentStateStore, UnitStateStore,
};
use std::sync::Arc;
use std::time::Duration;

pub struct HybridRuntimeStateStore {
    inner: MapRuntimeStateStore,
    ecs: Arc<EcsWorld>,
}

impl HybridRuntimeStateStore {
    pub fn new(cache: Arc<dyn Cache>, ecs: Arc<EcsWorld>) -> Self {
        Self {
            inner: MapRuntimeStateStore::new(cache, ecs.clone()),
            ecs,
        }
    }

    pub fn with_lifecycle_policy(
        cache: Arc<dyn Cache>,
        ecs: Arc<EcsWorld>,
        dropped_retention: Duration,
        maintenance_interval: usize,
        prune_batch_size: usize,
    ) -> Self {
        Self {
            inner: MapRuntimeStateStore::with_lifecycle_policy(
                cache,
                ecs.clone(),
                dropped_retention,
                maintenance_interval,
                prune_batch_size,
            ),
            ecs,
        }
    }
}

impl RuntimeStateStore for HybridRuntimeStateStore {
    fn backend_kind(&self) -> RuntimeStateBackendKind {
        RuntimeStateBackendKind::Hybrid
    }

    fn units(&self) -> &dyn UnitStateStore {
        self.inner.units()
    }

    fn resources(&self) -> &dyn ResourceStateStore {
        self.inner.resources()
    }

    fn events(&self) -> &dyn EventStateStore {
        self
    }

    fn shared_components(&self) -> &dyn SharedComponentStateStore {
        self
    }

    fn kv(&self) -> &dyn KeyValueStateStore {
        self.inner.kv()
    }

    fn maintain(&self) {
        self.inner.maintain()
    }
}

impl EventStateStore for HybridRuntimeStateStore {
    fn declare_line(&self, owner_unit_id: &str, line_name: &str, policy: EventLinePolicy) -> bool {
        self.ecs
            .declare_event_line(owner_unit_id, line_name, policy)
    }

    fn set_default_line(&self, owner_unit_id: &str, line_name: Option<&str>) -> bool {
        self.ecs.set_default_event_line(owner_unit_id, line_name)
    }

    fn lines_of(&self, unit_id: &str) -> Vec<EventLineStateSnapshot> {
        self.ecs.event_lines_of(unit_id)
    }

    fn owners_of_line(&self, line_name: &str) -> Vec<EventLineStateSnapshot> {
        self.ecs.owners_of_event_line(line_name)
    }
}

impl SharedComponentStateStore for HybridRuntimeStateStore {
    fn provide(&self, unit_id: &str, type_name: &'static str) -> bool {
        self.ecs.provide_shared_component(unit_id, type_name)
    }

    fn components_of(&self, unit_id: &str) -> Vec<SharedComponentStateSnapshot> {
        self.ecs.shared_components_of(unit_id)
    }

    fn owners_of(&self, type_name: &str) -> Vec<SharedComponentStateSnapshot> {
        self.ecs.owners_of_shared_component(type_name)
    }
}
