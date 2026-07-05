//! Embedded adversarial testing studio.
//!
//! The supervisor and adversary agents use immutable, conversation-scoped
//! composite role skills. Runtime snapshots contain only compact mutable state.

pub mod conclusion;
pub mod controller;
pub mod event_observer;
pub mod pair_builder;
pub mod pair_runtime;
pub mod role_contract;
pub mod server;
pub mod systems;
pub mod tool_runtime;
pub mod tools;
