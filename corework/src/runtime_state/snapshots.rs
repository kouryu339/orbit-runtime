use crate::ecs::ResourceAccessComponent;

pub type ResourceAccessSnapshot = ResourceAccessComponent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLineStateSnapshot {
    pub owner_unit_id: String,
    pub line_name: String,
    pub policy_name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedComponentStateSnapshot {
    pub unit_id: String,
    pub type_name: String,
}
