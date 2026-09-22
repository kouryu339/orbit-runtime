//! Jev finite-state decision runs.
//!
//! A [`JevManager`] owns reusable definitions and creates one child
//! [`ExecutionUnit`] per run. Tool authorization remains the responsibility of
//! the embedding layer through [`JevToolExecutor`].

mod client;
mod executor;
mod manager;
mod snapshot;
mod types;

pub use client::{HttpJevClient, JevClient, JevDecision};
pub use executor::RegistryJevToolExecutor;
pub use manager::JevManager;
pub use snapshot::apply_snapshot_update;
pub use types::{
    JevAction, JevArgumentBinding, JevDefinition, JevExecutionContext, JevRunOutcome, JevRunStatus,
    JevSnapshotUpdate, JevToolExecutor, JevToolResult,
};

pub const JEV_RUN_STARTED_EVENT: &str = "jev.execution_started";
pub const JEV_DECISION_EVENT: &str = "jev.decision";
pub const JEV_TOOL_STARTED_EVENT: &str = "jev.tool_started";
pub const JEV_TOOL_COMPLETED_EVENT: &str = "jev.tool_completed";
pub const JEV_SNAPSHOT_UPDATED_EVENT: &str = "jev.snapshot_updated";
pub const JEV_RUN_COMPLETED_EVENT: &str = "jev.execution_completed";
