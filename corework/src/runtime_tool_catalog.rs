//! Append-only metadata and executor publication for one Runtime instance.
use crate::{
    error::{FrameworkError, Result},
    rpc_tool::{validate_runtime_tool_metadata, RuntimeToolMetadata},
    workflow::dynamic_node::DynamicExecute,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub type RuntimeToolEntry = (RuntimeToolMetadata, Arc<dyn DynamicExecute>);

#[derive(Default)]
struct Snapshot {
    revision: u64,
    entries: HashMap<String, RuntimeToolEntry>,
}

#[derive(Default)]
pub struct RuntimeToolCatalog {
    snapshot: parking_lot::RwLock<Snapshot>,
}

impl RuntimeToolCatalog {
    pub fn append(&self, entries: Vec<RuntimeToolEntry>) -> Result<u64> {
        let mut names = HashSet::new();
        for (metadata, _) in &entries {
            validate_runtime_tool_metadata(metadata)?;
            if !names.insert(metadata.name.clone()) {
                return Err(FrameworkError::InvalidOperation(format!(
                    "tool_name_conflict: {}",
                    metadata.name
                )));
            }
        }
        let mut snapshot = self.snapshot.write();
        let conflicts: Vec<_> = names
            .iter()
            .filter(|name| snapshot.entries.contains_key(*name))
            .cloned()
            .collect();
        if !conflicts.is_empty() {
            return Err(FrameworkError::InvalidOperation(format!(
                "tool_name_conflict: {}",
                conflicts.join(", ")
            )));
        }
        if entries.is_empty() {
            return Ok(snapshot.revision);
        }
        let revision = snapshot.revision.checked_add(1).ok_or_else(|| {
            FrameworkError::InvalidOperation("tool catalog revision exhausted".into())
        })?;
        for (metadata, executor) in entries {
            snapshot
                .entries
                .insert(metadata.name.clone(), (metadata, executor));
        }
        snapshot.revision = revision;
        Ok(revision)
    }

    pub fn metadata(&self, name: &str) -> Option<RuntimeToolMetadata> {
        self.snapshot
            .read()
            .entries
            .get(name)
            .map(|entry| entry.0.clone())
    }

    pub fn executor(&self, name: &str) -> Option<Arc<dyn DynamicExecute>> {
        self.snapshot
            .read()
            .entries
            .get(name)
            .map(|entry| entry.1.clone())
    }

    pub fn tools(&self) -> Vec<RuntimeToolMetadata> {
        let mut tools: Vec<_> = self
            .snapshot
            .read()
            .entries
            .values()
            .map(|entry| entry.0.clone())
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    pub fn revision(&self) -> u64 {
        self.snapshot.read().revision
    }
}
