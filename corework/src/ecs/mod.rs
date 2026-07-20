//! Lightweight ECS mirror for corework execution units.
//!
//! The first ECS boundary is intentionally narrow: every `ExecutionUnit` is
//! mirrored as an entity with structural components. Existing cache, resource,
//! event, and system paths remain authoritative.

pub mod command;
pub mod components;
pub mod entity;
pub mod storage;
pub mod world;

pub use command::{EcsCommand, SpawnUnitCommand};
pub use components::{
    EventLineComponent, EventLineEntry, HierarchyComponent, LifecycleComponent,
    ResourceAccessComponent, ScopeComponent, SharedComponentEntry, SharedProviderComponent,
    UnitIdentityComponent, UnitLifecycleStatus,
};
pub use entity::UnitEntityId;
pub use storage::{ComponentStore, Versioned};
pub use world::{EcsUnitSnapshot, EcsWorld};
