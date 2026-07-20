use crate::ecs::{EcsUnitSnapshot, SpawnUnitCommand, UnitEntityId};
use crate::error::Result;
use crate::event_line::EventLinePolicy;
use crate::execution_unit::AccessMode;
use crate::runtime_state::scope::StateScope;
use crate::runtime_state::snapshots::{
    EventLineStateSnapshot, ResourceAccessSnapshot, SharedComponentStateSnapshot,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStateBackendKind {
    Map,
    Ecs,
    Hybrid,
}

pub trait RuntimeStateStore: Send + Sync {
    fn backend_kind(&self) -> RuntimeStateBackendKind;
    fn units(&self) -> &dyn UnitStateStore;
    fn resources(&self) -> &dyn ResourceStateStore;
    fn events(&self) -> &dyn EventStateStore;
    fn shared_components(&self) -> &dyn SharedComponentStateStore;
    fn kv(&self) -> &dyn KeyValueStateStore;
    fn maintain(&self) {}
}

pub trait UnitStateStore: Send + Sync {
    fn spawn_unit(&self, command: SpawnUnitCommand) -> UnitEntityId;
    fn mark_dropped(&self, unit_id: &str) -> bool;
    fn remove_unit(&self, _unit_id: &str) -> Option<EcsUnitSnapshot> {
        None
    }
    fn prune_dropped(&self, _dropped_before: SystemTime, _limit: usize) -> usize {
        0
    }
    fn unit_entity(&self, unit_id: &str) -> Option<UnitEntityId>;
    fn unit(&self, unit_id: &str) -> Option<EcsUnitSnapshot>;
    fn children_of(&self, unit_id: &str) -> Vec<EcsUnitSnapshot>;
    fn descendants_of(&self, unit_id: &str) -> Vec<EcsUnitSnapshot>;
    fn units_in_scope(&self, scope_id: &str) -> Vec<EcsUnitSnapshot>;
    fn active_units(&self) -> Vec<EcsUnitSnapshot>;
}

pub trait ResourceStateStore: Send + Sync {
    fn declare(&self, owner_id: &str, resource_key: &str, mode: AccessMode) -> bool;
    fn grant(&self, to_unit_id: &str, resource_key: &str, mode: AccessMode) -> bool;
    fn transfer(&self, resource_key: &str, from_owner_id: &str, to_owner_id: &str) -> bool;
    fn revoke_unit(&self, unit_id: &str) -> bool;
    fn access_of(&self, unit_id: &str) -> Option<ResourceAccessSnapshot>;
}

pub trait EventStateStore: Send + Sync {
    fn declare_line(&self, owner_unit_id: &str, line_name: &str, policy: EventLinePolicy) -> bool;
    fn set_default_line(&self, owner_unit_id: &str, line_name: Option<&str>) -> bool;

    fn lines_of(&self, _unit_id: &str) -> Vec<EventLineStateSnapshot> {
        Vec::new()
    }

    fn owners_of_line(&self, _line_name: &str) -> Vec<EventLineStateSnapshot> {
        Vec::new()
    }
}

pub trait SharedComponentStateStore: Send + Sync {
    fn provide(&self, unit_id: &str, type_name: &'static str) -> bool;

    fn components_of(&self, _unit_id: &str) -> Vec<SharedComponentStateSnapshot> {
        Vec::new()
    }

    fn owners_of(&self, _type_name: &str) -> Vec<SharedComponentStateSnapshot> {
        Vec::new()
    }
}

#[async_trait]
pub trait KeyValueStateStore: Send + Sync {
    async fn get_raw(&self, scope: &StateScope, key: &str) -> Result<Option<Value>>;

    async fn set_raw(
        &self,
        scope: &StateScope,
        key: &str,
        value: Value,
        ttl: Option<Duration>,
    ) -> Result<()>;

    async fn delete(&self, scope: &StateScope, key: &str) -> Result<()>;

    async fn dump_scope(&self, scope: &StateScope) -> Result<HashMap<String, Value>>;
}
