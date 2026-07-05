use crate::ecs::components::ScopeComponent;
use crate::execution_unit::UnitType;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct SpawnUnitCommand {
    pub unit_id: String,
    pub unit_type: UnitType,
    pub parent_id: Option<String>,
    pub ancestor_ids: Vec<String>,
    pub scope_id: String,
    pub cache_scope_id: String,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum EcsCommand {
    SpawnUnit(SpawnUnitCommand),
    MarkUnitDropped {
        unit_id: String,
        dropped_at: SystemTime,
    },
    UpdateScope {
        unit_id: String,
        scope: ScopeComponent,
    },
}
