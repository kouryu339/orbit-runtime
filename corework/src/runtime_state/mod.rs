//! Internal runtime state abstraction.
//!
//! This module is the migration boundary between the historical cache-oriented
//! runtime and future structured stores such as ECS or hybrid backends.

pub mod ecs_store;
pub mod factory;
pub mod hybrid_store;
pub mod map_store;
pub mod scope;
pub mod snapshots;
pub mod store;

pub use ecs_store::EcsRuntimeStateStore;
pub use factory::{create_runtime_state_store, RuntimeStateConfig};
pub use hybrid_store::HybridRuntimeStateStore;
pub use map_store::MapRuntimeStateStore;
pub use scope::StateScope;
pub use snapshots::{EventLineStateSnapshot, ResourceAccessSnapshot, SharedComponentStateSnapshot};
pub use store::{
    EventStateStore, KeyValueStateStore, ResourceStateStore, RuntimeStateBackendKind,
    RuntimeStateStore, SharedComponentStateStore, UnitStateStore,
};
