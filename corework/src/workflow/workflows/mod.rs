pub mod commands;
pub mod draft;
pub mod draft_ops;
pub mod executor;
pub mod flowchart;
pub mod script_tools;
pub mod snapshot;

#[cfg(feature = "workflow_llm_systems")]
pub mod recorder;
#[cfg(feature = "workflow_llm_systems")]
pub mod systems;

pub use executor::WorkflowsModule;

use std::sync::OnceLock;

type DraftExistsFn = dyn Fn(bool) + Send + Sync;

static DRAFT_EXISTS_CHANGED_CB: OnceLock<Box<DraftExistsFn>> = OnceLock::new();

pub fn set_draft_exists_callback<F>(f: F)
where
    F: Fn(bool) + Send + Sync + 'static,
{
    let _ = DRAFT_EXISTS_CHANGED_CB.set(Box::new(f));
}

pub(crate) fn notify_draft_exists(exists: bool) {
    if let Some(cb) = DRAFT_EXISTS_CHANGED_CB.get() {
        cb(exists);
    }
}
