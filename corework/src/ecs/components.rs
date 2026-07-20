use crate::event_line::EventLinePolicy;
use crate::execution_unit::{AccessMode, UnitType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitIdentityComponent {
    pub unit_id: String,
    pub unit_type: UnitType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyComponent {
    pub parent_id: Option<String>,
    pub ancestor_ids: Vec<String>,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeComponent {
    pub scope_id: String,
    pub cache_scope_id: String,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitLifecycleStatus {
    Active,
    Dropped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleComponent {
    pub created_at: SystemTime,
    pub status: UnitLifecycleStatus,
    pub dropped_at: Option<SystemTime>,
}

impl LifecycleComponent {
    pub fn active() -> Self {
        Self {
            created_at: SystemTime::now(),
            status: UnitLifecycleStatus::Active,
            dropped_at: None,
        }
    }

    pub fn mark_dropped(&mut self, dropped_at: SystemTime) {
        self.status = UnitLifecycleStatus::Dropped;
        self.dropped_at = Some(dropped_at);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceAccessComponent {
    pub owned_resources: HashMap<String, AccessMode>,
    pub granted_resources: HashMap<String, AccessMode>,
}

impl ResourceAccessComponent {
    pub fn declare_owned(&mut self, resource_key: impl Into<String>, access_mode: AccessMode) {
        self.owned_resources
            .insert(resource_key.into(), access_mode);
    }

    pub fn grant_resource(&mut self, resource_key: impl Into<String>, access_mode: AccessMode) {
        self.granted_resources
            .insert(resource_key.into(), access_mode);
    }

    pub fn remove_owned(&mut self, resource_key: &str) -> Option<AccessMode> {
        self.owned_resources.remove(resource_key)
    }

    pub fn revoke_grant(&mut self, resource_key: &str) -> Option<AccessMode> {
        self.granted_resources.remove(resource_key)
    }

    pub fn clear_owned(&mut self) {
        self.owned_resources.clear();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventLineComponent {
    pub lines: HashMap<String, EventLineEntry>,
    pub default_line: Option<String>,
}

impl EventLineComponent {
    pub fn declare_line(&mut self, line_name: impl Into<String>, policy: EventLinePolicy) -> bool {
        let line_name = line_name.into();
        let existed = self.lines.contains_key(&line_name);
        self.lines.insert(
            line_name.clone(),
            EventLineEntry {
                line_name,
                policy,
                is_default: false,
            },
        );
        !existed
    }

    pub fn set_default_line(&mut self, line_name: Option<&str>) -> bool {
        self.default_line = line_name.map(str::to_string);
        let mut matched = line_name.is_none();
        for line in self.lines.values_mut() {
            let is_default = line_name == Some(line.line_name.as_str());
            matched |= is_default;
            line.is_default = is_default;
        }
        matched
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLineEntry {
    pub line_name: String,
    pub policy: EventLinePolicy,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedProviderComponent {
    pub shared_components: Vec<SharedComponentEntry>,
}

impl SharedProviderComponent {
    pub fn provide(&mut self, type_name: impl Into<String>) -> bool {
        let type_name = type_name.into();
        if self
            .shared_components
            .iter()
            .any(|entry| entry.type_name == type_name)
        {
            return false;
        }
        self.shared_components
            .push(SharedComponentEntry { type_name });
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedComponentEntry {
    pub type_name: String,
}
