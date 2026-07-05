use crate::cache::Cache;
use crate::ecs::{EcsUnitSnapshot, EcsWorld, SpawnUnitCommand, UnitEntityId, UnitLifecycleStatus};
use crate::error::Result;
use crate::event_line::EventLinePolicy;
use crate::execution_unit::AccessMode;
use crate::runtime_state::scope::StateScope;
use crate::runtime_state::snapshots::{
    EventLineStateSnapshot, ResourceAccessSnapshot, SharedComponentStateSnapshot,
};
use crate::runtime_state::store::{
    EventStateStore, KeyValueStateStore, ResourceStateStore, RuntimeStateBackendKind,
    RuntimeStateStore, SharedComponentStateStore, UnitStateStore,
};
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

pub struct MapRuntimeStateStore {
    cache: Arc<dyn Cache>,
    ecs: Arc<EcsWorld>,
    event_lines: DashMap<String, MapEventProvider>,
    shared_components: DashMap<String, Vec<&'static str>>,
    dropped_retention: Duration,
    maintenance_interval: usize,
    prune_batch_size: usize,
    drops_since_maintenance: AtomicUsize,
}

#[derive(Debug, Default)]
struct MapEventProvider {
    lines: HashMap<String, EventLineStateSnapshot>,
}

impl MapRuntimeStateStore {
    pub fn new(cache: Arc<dyn Cache>, ecs: Arc<EcsWorld>) -> Self {
        Self::with_lifecycle_policy(cache, ecs, Duration::from_secs(60 * 60), 64, 256)
    }

    pub fn with_lifecycle_policy(
        cache: Arc<dyn Cache>,
        ecs: Arc<EcsWorld>,
        dropped_retention: Duration,
        maintenance_interval: usize,
        prune_batch_size: usize,
    ) -> Self {
        Self {
            cache,
            ecs,
            event_lines: DashMap::new(),
            shared_components: DashMap::new(),
            dropped_retention,
            maintenance_interval: maintenance_interval.max(1),
            prune_batch_size: prune_batch_size.max(1),
            drops_since_maintenance: AtomicUsize::new(0),
        }
    }
}

impl RuntimeStateStore for MapRuntimeStateStore {
    fn backend_kind(&self) -> RuntimeStateBackendKind {
        RuntimeStateBackendKind::Map
    }

    fn units(&self) -> &dyn UnitStateStore {
        self
    }

    fn resources(&self) -> &dyn ResourceStateStore {
        self
    }

    fn events(&self) -> &dyn EventStateStore {
        self
    }

    fn shared_components(&self) -> &dyn SharedComponentStateStore {
        self
    }

    fn kv(&self) -> &dyn KeyValueStateStore {
        self
    }

    fn maintain(&self) {
        let drops = self.drops_since_maintenance.fetch_add(1, Ordering::Relaxed) + 1;
        if drops < self.maintenance_interval {
            return;
        }
        if self.drops_since_maintenance.swap(0, Ordering::AcqRel) < self.maintenance_interval {
            return;
        }
        let Some(dropped_before) = SystemTime::now().checked_sub(self.dropped_retention) else {
            return;
        };
        let pruned = self.prune_dropped(dropped_before, self.prune_batch_size);
        if pruned > 0 {
            tracing::debug!(
                pruned_units = pruned,
                retention_seconds = self.dropped_retention.as_secs(),
                "runtime state pruned dropped execution units"
            );
        }
    }
}

impl UnitStateStore for MapRuntimeStateStore {
    fn spawn_unit(&self, command: SpawnUnitCommand) -> UnitEntityId {
        self.ecs.spawn_unit(command)
    }

    fn mark_dropped(&self, unit_id: &str) -> bool {
        self.ecs.mark_unit_dropped(unit_id)
    }

    fn remove_unit(&self, unit_id: &str) -> Option<EcsUnitSnapshot> {
        let snapshot = self.ecs.remove_unit(unit_id)?;
        self.event_lines.remove(unit_id);
        self.shared_components.remove(unit_id);
        Some(snapshot)
    }

    fn prune_dropped(&self, dropped_before: SystemTime, limit: usize) -> usize {
        if limit == 0 {
            return 0;
        }
        let unit_ids = self
            .ecs
            .all_units()
            .into_iter()
            .filter(|unit| {
                unit.lifecycle.status == UnitLifecycleStatus::Dropped
                    && unit
                        .lifecycle
                        .dropped_at
                        .is_some_and(|dropped_at| dropped_at <= dropped_before)
            })
            .take(limit)
            .map(|unit| unit.identity.unit_id)
            .collect::<Vec<_>>();

        unit_ids
            .iter()
            .filter(|unit_id| self.remove_unit(unit_id).is_some())
            .count()
    }

    fn unit_entity(&self, unit_id: &str) -> Option<UnitEntityId> {
        self.ecs.unit_entity(unit_id)
    }

    fn unit(&self, unit_id: &str) -> Option<EcsUnitSnapshot> {
        self.ecs.unit(unit_id)
    }

    fn children_of(&self, unit_id: &str) -> Vec<EcsUnitSnapshot> {
        self.ecs.children_of(unit_id)
    }

    fn descendants_of(&self, unit_id: &str) -> Vec<EcsUnitSnapshot> {
        self.ecs.descendants_of(unit_id)
    }

    fn units_in_scope(&self, scope_id: &str) -> Vec<EcsUnitSnapshot> {
        self.ecs.units_in_scope(scope_id)
    }

    fn active_units(&self) -> Vec<EcsUnitSnapshot> {
        self.ecs.active_units()
    }
}

impl ResourceStateStore for MapRuntimeStateStore {
    fn declare(&self, owner_id: &str, resource_key: &str, mode: AccessMode) -> bool {
        self.ecs
            .declare_resource_access(owner_id, resource_key, mode)
    }

    fn grant(&self, to_unit_id: &str, resource_key: &str, mode: AccessMode) -> bool {
        self.ecs
            .grant_resource_access(to_unit_id, resource_key, mode)
    }

    fn transfer(&self, resource_key: &str, from_owner_id: &str, to_owner_id: &str) -> bool {
        self.ecs
            .transfer_resource_ownership(resource_key, from_owner_id, to_owner_id)
    }

    fn revoke_unit(&self, unit_id: &str) -> bool {
        self.ecs.revoke_unit_resource_access(unit_id)
    }

    fn access_of(&self, unit_id: &str) -> Option<ResourceAccessSnapshot> {
        self.ecs.resource_access_of(unit_id)
    }
}

impl EventStateStore for MapRuntimeStateStore {
    fn declare_line(&self, owner_unit_id: &str, line_name: &str, policy: EventLinePolicy) -> bool {
        if self.ecs.unit_entity(owner_unit_id).is_none() {
            return false;
        }
        let mut provider = self
            .event_lines
            .entry(owner_unit_id.to_string())
            .or_default();
        let existed = provider.lines.contains_key(line_name);
        provider.lines.insert(
            line_name.to_string(),
            EventLineStateSnapshot {
                owner_unit_id: owner_unit_id.to_string(),
                line_name: line_name.to_string(),
                policy_name: format!("{policy:?}"),
                is_default: false,
            },
        );
        !existed
    }

    fn set_default_line(&self, owner_unit_id: &str, line_name: Option<&str>) -> bool {
        let mut matched = false;
        let Some(mut provider) = self.event_lines.get_mut(owner_unit_id) else {
            return line_name.is_none();
        };
        for (current_line_name, snapshot) in provider.lines.iter_mut() {
            let is_default = line_name == Some(current_line_name.as_str());
            matched |= is_default;
            snapshot.is_default = is_default;
        }
        line_name.is_none() || matched
    }

    fn lines_of(&self, unit_id: &str) -> Vec<EventLineStateSnapshot> {
        self.event_lines
            .get(unit_id)
            .map(|provider| provider.lines.values().cloned().collect())
            .unwrap_or_default()
    }

    fn owners_of_line(&self, line_name: &str) -> Vec<EventLineStateSnapshot> {
        self.event_lines
            .iter()
            .filter_map(|provider| provider.lines.get(line_name).cloned())
            .collect()
    }
}

impl SharedComponentStateStore for MapRuntimeStateStore {
    fn provide(&self, unit_id: &str, type_name: &'static str) -> bool {
        if self.ecs.unit_entity(unit_id).is_none() {
            return false;
        }
        let mut type_names = self
            .shared_components
            .entry(unit_id.to_string())
            .or_default();
        if type_names.contains(&type_name) {
            return false;
        }
        type_names.push(type_name);
        true
    }

    fn components_of(&self, unit_id: &str) -> Vec<SharedComponentStateSnapshot> {
        self.shared_components
            .get(unit_id)
            .map(|type_names| {
                type_names
                    .iter()
                    .map(|type_name| SharedComponentStateSnapshot {
                        unit_id: unit_id.to_string(),
                        type_name: (*type_name).to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn owners_of(&self, type_name: &str) -> Vec<SharedComponentStateSnapshot> {
        self.shared_components
            .iter()
            .filter(|provider| provider.value().contains(&type_name))
            .map(|provider| SharedComponentStateSnapshot {
                unit_id: provider.key().clone(),
                type_name: type_name.to_string(),
            })
            .collect()
    }
}

#[async_trait]
impl KeyValueStateStore for MapRuntimeStateStore {
    async fn get_raw(&self, scope: &StateScope, key: &str) -> Result<Option<Value>> {
        self.cache.get_raw(&scope.key(key)).await
    }

    async fn set_raw(
        &self,
        scope: &StateScope,
        key: &str,
        value: Value,
        ttl: Option<Duration>,
    ) -> Result<()> {
        self.cache.set_raw(&scope.key(key), value, ttl).await
    }

    async fn delete(&self, scope: &StateScope, key: &str) -> Result<()> {
        self.cache.delete(&scope.key(key)).await
    }

    async fn dump_scope(&self, scope: &StateScope) -> Result<HashMap<String, Value>> {
        let snapshot = self.cache.dump_raw().await?;
        let Some(prefix) = scope.key_prefix() else {
            return Ok(snapshot);
        };

        Ok(snapshot
            .into_iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(&prefix)
                    .map(|logical_key| (logical_key.to_string(), value))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;
    use crate::execution_unit::UnitType;
    use std::sync::Barrier;

    #[test]
    fn empty_shared_component_mirror_has_no_heap_capacity() {
        let store =
            MapRuntimeStateStore::new(Arc::new(InMemoryCache::new()), Arc::new(EcsWorld::new()));

        assert!(store.shared_components.is_empty());
    }

    fn spawn_test_units(store: &MapRuntimeStateStore, owner_count: usize) {
        for index in 0..owner_count {
            store.units().spawn_unit(SpawnUnitCommand {
                unit_id: format!("unit:{index}"),
                unit_type: UnitType::Module,
                parent_id: None,
                ancestor_ids: Vec::new(),
                scope_id: "scope:concurrent-map".to_string(),
                cache_scope_id: format!("scope:concurrent-map:{index}"),
                conversation_id: None,
            });
        }
    }

    #[test]
    fn concurrent_owners_preserve_same_named_event_and_shared_component_writes() {
        const OWNER_COUNT: usize = 32;
        let store = Arc::new(MapRuntimeStateStore::new(
            Arc::new(InMemoryCache::new()),
            Arc::new(EcsWorld::new()),
        ));
        spawn_test_units(&store, OWNER_COUNT);

        let barrier = Arc::new(Barrier::new(OWNER_COUNT));
        let handles = (0..OWNER_COUNT)
            .map(|index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let unit_id = format!("unit:{index}");
                    assert!(store.events().declare_line(
                        &unit_id,
                        "updates",
                        EventLinePolicy::subtree(),
                    ));
                    assert!(store
                        .shared_components()
                        .provide(&unit_id, std::any::type_name::<String>()));
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(store.events().owners_of_line("updates").len(), OWNER_COUNT);
        assert_eq!(
            store
                .shared_components()
                .owners_of(std::any::type_name::<String>())
                .len(),
            OWNER_COUNT
        );
    }

    #[test]
    fn concurrent_event_line_declarations_for_one_owner_do_not_lose_updates() {
        const LINE_COUNT: usize = 32;
        let store = Arc::new(MapRuntimeStateStore::new(
            Arc::new(InMemoryCache::new()),
            Arc::new(EcsWorld::new()),
        ));
        spawn_test_units(&store, 1);

        let barrier = Arc::new(Barrier::new(LINE_COUNT));
        let handles = (0..LINE_COUNT)
            .map(|index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    assert!(store.events().declare_line(
                        "unit:0",
                        &format!("line:{index}"),
                        EventLinePolicy::private(),
                    ));
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        let mut line_names = store
            .events()
            .lines_of("unit:0")
            .into_iter()
            .map(|snapshot| snapshot.line_name)
            .collect::<Vec<_>>();
        line_names.sort();
        let mut expected = (0..LINE_COUNT)
            .map(|index| format!("line:{index}"))
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(line_names, expected);
    }

    #[tokio::test]
    async fn kv_scope_uses_explicit_runtime_state_scope() {
        let cache = Arc::new(InMemoryCache::new());
        let store = MapRuntimeStateStore::new(cache, Arc::new(EcsWorld::new()));
        let scope = StateScope::local("unit:a", "scope:a");

        store
            .kv()
            .set_raw(&scope, "answer", serde_json::json!(42), None)
            .await
            .unwrap();

        assert_eq!(
            store.kv().get_raw(&scope, "answer").await.unwrap(),
            Some(serde_json::json!(42))
        );
    }

    #[test]
    fn mirrors_event_lines_and_shared_components() {
        let cache = Arc::new(InMemoryCache::new());
        let store = MapRuntimeStateStore::new(cache, Arc::new(EcsWorld::new()));
        store.units().spawn_unit(SpawnUnitCommand {
            unit_id: "unit:a".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope:a".to_string(),
            cache_scope_id: "scope:a".to_string(),
            conversation_id: None,
        });

        assert!(store
            .events()
            .declare_line("unit:a", "updates", EventLinePolicy::subtree()));
        assert!(store.events().set_default_line("unit:a", Some("updates")));
        assert!(store
            .shared_components()
            .provide("unit:a", std::any::type_name::<String>()));

        let lines = store.events().lines_of("unit:a");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_name, "updates");
        assert!(lines[0].is_default);

        let shared_components = store.shared_components().components_of("unit:a");
        assert_eq!(shared_components.len(), 1);
        assert_eq!(
            shared_components[0].type_name,
            std::any::type_name::<String>()
        );

        let line_owners = store.events().owners_of_line("updates");
        assert_eq!(line_owners.len(), 1);
        assert_eq!(line_owners[0].owner_unit_id, "unit:a");

        let shared_component_owners = store
            .shared_components()
            .owners_of(std::any::type_name::<String>());
        assert_eq!(shared_component_owners.len(), 1);
        assert_eq!(shared_component_owners[0].unit_id, "unit:a");
    }
}
