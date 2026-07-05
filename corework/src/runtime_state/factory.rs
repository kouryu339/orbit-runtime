use crate::cache::Cache;
use crate::ecs::EcsWorld;
use crate::runtime_state::ecs_store::EcsRuntimeStateStore;
use crate::runtime_state::hybrid_store::HybridRuntimeStateStore;
use crate::runtime_state::map_store::MapRuntimeStateStore;
use crate::runtime_state::store::{RuntimeStateBackendKind, RuntimeStateStore};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStateConfig {
    pub backend: RuntimeStateBackendKind,
    pub dropped_retention: Duration,
    pub maintenance_interval: usize,
    pub prune_batch_size: usize,
}

impl RuntimeStateConfig {
    pub fn map() -> Self {
        Self {
            backend: RuntimeStateBackendKind::Map,
            ..Self::production_defaults()
        }
    }

    pub fn ecs() -> Self {
        Self {
            backend: RuntimeStateBackendKind::Ecs,
            ..Self::production_defaults()
        }
    }

    pub fn hybrid() -> Self {
        Self {
            backend: RuntimeStateBackendKind::Hybrid,
            ..Self::production_defaults()
        }
    }

    fn production_defaults() -> Self {
        Self {
            backend: RuntimeStateBackendKind::Map,
            dropped_retention: Duration::from_secs(60 * 60),
            maintenance_interval: 64,
            prune_batch_size: 256,
        }
    }
}

impl Default for RuntimeStateConfig {
    fn default() -> Self {
        Self::map()
    }
}

pub fn create_runtime_state_store(
    config: RuntimeStateConfig,
    cache: Arc<dyn Cache>,
    ecs: Arc<EcsWorld>,
) -> Arc<dyn RuntimeStateStore> {
    match config.backend {
        RuntimeStateBackendKind::Map => Arc::new(MapRuntimeStateStore::with_lifecycle_policy(
            cache,
            ecs,
            config.dropped_retention,
            config.maintenance_interval,
            config.prune_batch_size,
        )),
        RuntimeStateBackendKind::Ecs => Arc::new(EcsRuntimeStateStore::with_lifecycle_policy(
            cache,
            ecs,
            config.dropped_retention,
            config.maintenance_interval,
            config.prune_batch_size,
        )),
        RuntimeStateBackendKind::Hybrid => {
            Arc::new(HybridRuntimeStateStore::with_lifecycle_policy(
                cache,
                ecs,
                config.dropped_retention,
                config.maintenance_interval,
                config.prune_batch_size,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;
    use crate::ecs::SpawnUnitCommand;
    use crate::event_line::EventLinePolicy;
    use crate::execution_unit::UnitType;
    use std::time::SystemTime;

    #[test]
    fn factory_selects_runtime_state_backend_kind() {
        assert_eq!(
            RuntimeStateConfig::default().backend,
            RuntimeStateBackendKind::Map
        );
        let cases = [
            (RuntimeStateConfig::map(), RuntimeStateBackendKind::Map),
            (RuntimeStateConfig::ecs(), RuntimeStateBackendKind::Ecs),
            (
                RuntimeStateConfig::hybrid(),
                RuntimeStateBackendKind::Hybrid,
            ),
        ];

        for (config, expected) in cases {
            let store = create_runtime_state_store(
                config,
                Arc::new(InMemoryCache::new()),
                Arc::new(EcsWorld::new()),
            );
            assert_eq!(store.backend_kind(), expected);
        }
    }

    #[test]
    fn ecs_backed_runtime_state_stores_structured_event_and_shared_component_state() {
        for config in [RuntimeStateConfig::ecs(), RuntimeStateConfig::hybrid()] {
            let store = create_runtime_state_store(
                config,
                Arc::new(InMemoryCache::new()),
                Arc::new(EcsWorld::new()),
            );
            store.units().spawn_unit(SpawnUnitCommand {
                unit_id: "unit:structured".to_string(),
                unit_type: UnitType::Module,
                parent_id: None,
                ancestor_ids: Vec::new(),
                scope_id: "scope:structured".to_string(),
                cache_scope_id: "scope:structured".to_string(),
                conversation_id: None,
            });

            assert!(store.events().declare_line(
                "unit:structured",
                "updates",
                EventLinePolicy::subtree(),
            ));
            assert!(store
                .events()
                .set_default_line("unit:structured", Some("updates")));
            assert!(store
                .shared_components()
                .provide("unit:structured", std::any::type_name::<String>()));

            let lines = store.events().lines_of("unit:structured");
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].line_name, "updates");
            assert!(lines[0].is_default);

            let shared_components = store.shared_components().components_of("unit:structured");
            assert_eq!(shared_components.len(), 1);
            assert_eq!(
                shared_components[0].type_name,
                std::any::type_name::<String>()
            );

            let line_owners = store.events().owners_of_line("updates");
            assert_eq!(line_owners.len(), 1);
            assert_eq!(line_owners[0].owner_unit_id, "unit:structured");

            let shared_component_owners = store
                .shared_components()
                .owners_of(std::any::type_name::<String>());
            assert_eq!(shared_component_owners.len(), 1);
            assert_eq!(shared_component_owners[0].unit_id, "unit:structured");
        }
    }

    #[test]
    fn shared_component_contract_is_consistent_across_backends() {
        for config in [
            RuntimeStateConfig::map(),
            RuntimeStateConfig::ecs(),
            RuntimeStateConfig::hybrid(),
        ] {
            let store = create_runtime_state_store(
                config,
                Arc::new(InMemoryCache::new()),
                Arc::new(EcsWorld::new()),
            );
            for unit_id in ["unit:owner", "unit:peer"] {
                store.units().spawn_unit(SpawnUnitCommand {
                    unit_id: unit_id.to_string(),
                    unit_type: UnitType::Module,
                    parent_id: None,
                    ancestor_ids: Vec::new(),
                    scope_id: "scope:shared-components".to_string(),
                    cache_scope_id: "scope:shared-components".to_string(),
                    conversation_id: None,
                });
            }

            let string_type = std::any::type_name::<String>();
            let number_type = std::any::type_name::<u64>();
            assert!(!store
                .shared_components()
                .provide("unit:missing", string_type));
            assert!(store.shared_components().provide("unit:owner", string_type));
            assert!(!store.shared_components().provide("unit:owner", string_type));
            assert!(store.shared_components().provide("unit:owner", number_type));
            assert!(store.shared_components().provide("unit:peer", string_type));

            let mut owner_types = store
                .shared_components()
                .components_of("unit:owner")
                .into_iter()
                .map(|snapshot| snapshot.type_name)
                .collect::<Vec<_>>();
            owner_types.sort();
            let mut expected_types = vec![string_type.to_string(), number_type.to_string()];
            expected_types.sort();
            assert_eq!(owner_types, expected_types);

            let mut string_owners = store
                .shared_components()
                .owners_of(string_type)
                .into_iter()
                .map(|snapshot| snapshot.unit_id)
                .collect::<Vec<_>>();
            string_owners.sort();
            assert_eq!(
                string_owners,
                vec!["unit:owner".to_string(), "unit:peer".to_string()]
            );

            assert!(store.units().mark_dropped("unit:owner"));
            assert!(store
                .units()
                .active_units()
                .iter()
                .all(|snapshot| snapshot.identity.unit_id != "unit:owner"));
            assert_eq!(
                store.shared_components().components_of("unit:owner").len(),
                2,
                "dropped units retain shared-component declarations for audit"
            );
            assert_eq!(store.units().prune_dropped(SystemTime::now(), 1), 1);
            assert!(store.units().unit("unit:owner").is_none());
            assert!(store
                .shared_components()
                .components_of("unit:owner")
                .is_empty());
        }
    }

    #[test]
    fn maintenance_applies_bounded_tombstone_policy() {
        let store = create_runtime_state_store(
            RuntimeStateConfig {
                backend: RuntimeStateBackendKind::Ecs,
                dropped_retention: Duration::ZERO,
                maintenance_interval: 1,
                prune_batch_size: 1,
            },
            Arc::new(InMemoryCache::new()),
            Arc::new(EcsWorld::new()),
        );
        store.units().spawn_unit(SpawnUnitCommand {
            unit_id: "unit:maintenance".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope:maintenance".to_string(),
            cache_scope_id: "scope:maintenance".to_string(),
            conversation_id: None,
        });
        assert!(store.units().mark_dropped("unit:maintenance"));

        store.maintain();

        assert!(store.units().unit("unit:maintenance").is_none());
    }
}
