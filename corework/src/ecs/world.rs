use crate::ecs::command::{EcsCommand, SpawnUnitCommand};
use crate::ecs::components::{
    EventLineComponent, HierarchyComponent, LifecycleComponent, ResourceAccessComponent,
    ScopeComponent, SharedProviderComponent, UnitIdentityComponent, UnitLifecycleStatus,
};
use crate::ecs::entity::UnitEntityId;
use crate::ecs::storage::ComponentStore;
use crate::event_line::EventLinePolicy;
use crate::execution_unit::{AccessMode, UnitType};
use crate::runtime_state::snapshots::{EventLineStateSnapshot, SharedComponentStateSnapshot};
use dashmap::{DashMap, DashSet};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct EcsUnitSnapshot {
    pub entity: UnitEntityId,
    pub identity: UnitIdentityComponent,
    pub hierarchy: HierarchyComponent,
    pub scope: ScopeComponent,
    pub lifecycle: LifecycleComponent,
}

#[derive(Debug)]
pub struct EcsWorld {
    allocator: AtomicU64,
    index: DashMap<String, UnitEntityId>,
    reverse_index: DashMap<UnitEntityId, String>,
    identities: ComponentStore<UnitIdentityComponent>,
    hierarchies: ComponentStore<HierarchyComponent>,
    scopes: ComponentStore<ScopeComponent>,
    lifecycles: ComponentStore<LifecycleComponent>,
    resource_accesses: ComponentStore<ResourceAccessComponent>,
    event_lines: ComponentStore<EventLineComponent>,
    shared_providers: ComponentStore<SharedProviderComponent>,
    active_index: DashSet<UnitEntityId>,
    root_index: DashSet<UnitEntityId>,
    parent_index: DashMap<String, DashSet<UnitEntityId>>,
    ancestor_index: DashMap<String, DashSet<UnitEntityId>>,
    scope_index: DashMap<String, DashSet<UnitEntityId>>,
    conversation_index: DashMap<String, DashSet<UnitEntityId>>,
    type_index: DashMap<UnitType, DashSet<UnitEntityId>>,
    event_line_index: DashMap<String, DashSet<UnitEntityId>>,
    shared_type_index: DashMap<String, DashSet<UnitEntityId>>,
    // Structural writes are cold-path operations. Serializing writers keeps
    // multi-component mutations and their indexes from losing concurrent updates.
    mutation_lock: Mutex<()>,
    command_queue: Mutex<Vec<EcsCommand>>,
}

impl EcsWorld {
    pub fn new() -> Self {
        Self {
            allocator: AtomicU64::new(1),
            index: DashMap::new(),
            reverse_index: DashMap::new(),
            identities: ComponentStore::new(),
            hierarchies: ComponentStore::new(),
            scopes: ComponentStore::new(),
            lifecycles: ComponentStore::new(),
            resource_accesses: ComponentStore::new(),
            event_lines: ComponentStore::new(),
            shared_providers: ComponentStore::new(),
            active_index: DashSet::new(),
            root_index: DashSet::new(),
            parent_index: DashMap::new(),
            ancestor_index: DashMap::new(),
            scope_index: DashMap::new(),
            conversation_index: DashMap::new(),
            type_index: DashMap::new(),
            event_line_index: DashMap::new(),
            shared_type_index: DashMap::new(),
            mutation_lock: Mutex::new(()),
            command_queue: Mutex::new(Vec::new()),
        }
    }

    pub fn spawn_unit(&self, command: SpawnUnitCommand) -> UnitEntityId {
        let _mutation = self.mutation_lock.lock();
        if let Some(existing) = self.index.get(&command.unit_id) {
            return *existing;
        }

        let entity = UnitEntityId::new(self.allocator.fetch_add(1, Ordering::Relaxed));
        let depth = command.ancestor_ids.len();
        let identity = UnitIdentityComponent {
            unit_id: command.unit_id.clone(),
            unit_type: command.unit_type,
        };
        let hierarchy = HierarchyComponent {
            parent_id: command.parent_id,
            ancestor_ids: command.ancestor_ids,
            depth,
        };
        let scope = ScopeComponent {
            scope_id: command.scope_id,
            cache_scope_id: command.cache_scope_id,
            conversation_id: command.conversation_id,
        };

        self.index.insert(identity.unit_id.clone(), entity);
        self.reverse_index.insert(entity, identity.unit_id.clone());
        self.insert_indexes(entity, &identity, &hierarchy, &scope);
        self.identities.insert(entity, identity);
        self.hierarchies.insert(entity, hierarchy);
        self.scopes.insert(entity, scope);
        self.lifecycles.insert(entity, LifecycleComponent::active());
        self.resource_accesses
            .insert(entity, ResourceAccessComponent::default());
        self.event_lines
            .insert(entity, EventLineComponent::default());
        self.shared_providers
            .insert(entity, SharedProviderComponent::default());

        entity
    }

    pub fn enqueue(&self, command: EcsCommand) {
        self.command_queue.lock().push(command);
    }

    pub fn apply(&self, command: EcsCommand) {
        match command {
            EcsCommand::SpawnUnit(command) => {
                self.spawn_unit(command);
            }
            EcsCommand::MarkUnitDropped {
                unit_id,
                dropped_at,
            } => {
                self.mark_unit_dropped_at(&unit_id, dropped_at);
            }
            EcsCommand::UpdateScope { unit_id, scope } => {
                if let Some(entity) = self.unit_entity(&unit_id) {
                    self.update_scope(entity, scope);
                }
            }
        }
    }

    pub fn apply_queued(&self) {
        let commands = std::mem::take(&mut *self.command_queue.lock());
        for command in commands {
            self.apply(command);
        }
    }

    pub fn mark_unit_dropped(&self, unit_id: &str) -> bool {
        self.mark_unit_dropped_at(unit_id, SystemTime::now())
    }

    pub fn mark_unit_dropped_at(&self, unit_id: &str, dropped_at: SystemTime) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(unit_id) else {
            return false;
        };
        if self
            .lifecycles
            .modify(entity, |lifecycle| lifecycle.mark_dropped(dropped_at))
            .is_some()
        {
            self.active_index.remove(&entity);
            true
        } else {
            false
        }
    }

    pub fn update_scope(&self, entity: UnitEntityId, scope: ScopeComponent) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(existing) = self.scopes.get(entity).map(|entry| entry.value) else {
            return false;
        };
        self.remove_scope_indexes(entity, &existing);
        self.insert_scope_indexes(entity, &scope);
        self.scopes
            .modify(entity, |current| *current = scope)
            .is_some()
    }

    pub fn unit_entity(&self, unit_id: &str) -> Option<UnitEntityId> {
        self.index.get(unit_id).map(|entry| *entry)
    }

    pub fn unit(&self, unit_id: &str) -> Option<EcsUnitSnapshot> {
        let entity = self.unit_entity(unit_id)?;
        self.snapshot(entity)
    }

    pub fn snapshot(&self, entity: UnitEntityId) -> Option<EcsUnitSnapshot> {
        Some(EcsUnitSnapshot {
            entity,
            identity: self.identities.get(entity)?.value,
            hierarchy: self.hierarchies.get(entity)?.value,
            scope: self.scopes.get(entity)?.value,
            lifecycle: self.lifecycles.get(entity)?.value,
        })
    }

    pub fn active_units(&self) -> Vec<EcsUnitSnapshot> {
        self.snapshots_from_entities(self.active_index.iter().map(|entry| *entry))
    }

    pub fn all_units(&self) -> Vec<EcsUnitSnapshot> {
        self.identities
            .values()
            .into_iter()
            .filter_map(|(entity, _)| self.snapshot(entity))
            .collect()
    }

    pub fn children_of(&self, parent_id: &str) -> Vec<EcsUnitSnapshot> {
        self.snapshots_from_string_index(&self.parent_index, parent_id)
    }

    pub fn descendants_of(&self, ancestor_id: &str) -> Vec<EcsUnitSnapshot> {
        self.snapshots_from_string_index(&self.ancestor_index, ancestor_id)
    }

    pub fn root_units(&self) -> Vec<EcsUnitSnapshot> {
        self.snapshots_from_entities(self.root_index.iter().map(|entry| *entry))
    }

    pub fn units_in_scope(&self, scope_id: &str) -> Vec<EcsUnitSnapshot> {
        self.snapshots_from_string_index(&self.scope_index, scope_id)
    }

    pub fn units_in_conversation(&self, conversation_id: &str) -> Vec<EcsUnitSnapshot> {
        self.snapshots_from_string_index(&self.conversation_index, conversation_id)
    }

    pub fn units_by_type(&self, unit_type: UnitType) -> Vec<EcsUnitSnapshot> {
        self.type_index
            .get(&unit_type)
            .map(|entities| self.snapshots_from_entities(entities.iter().map(|entry| *entry)))
            .unwrap_or_default()
    }

    pub fn active_units_in_scope(&self, scope_id: &str) -> Vec<EcsUnitSnapshot> {
        self.units_in_scope(scope_id)
            .into_iter()
            .filter(|unit| unit.lifecycle.status == UnitLifecycleStatus::Active)
            .collect()
    }

    pub fn active_descendants_of(&self, ancestor_id: &str) -> Vec<EcsUnitSnapshot> {
        self.descendants_of(ancestor_id)
            .into_iter()
            .filter(|unit| unit.lifecycle.status == UnitLifecycleStatus::Active)
            .collect()
    }

    pub fn remove_unit(&self, unit_id: &str) -> Option<EcsUnitSnapshot> {
        let _mutation = self.mutation_lock.lock();
        let entity = self.unit_entity(unit_id)?;
        let snapshot = self.snapshot(entity)?;
        self.remove_indexes(
            entity,
            &snapshot.identity,
            &snapshot.hierarchy,
            &snapshot.scope,
        );
        self.index.remove(unit_id);
        self.reverse_index.remove(&entity);
        self.identities.remove(entity);
        self.hierarchies.remove(entity);
        self.scopes.remove(entity);
        self.lifecycles.remove(entity);
        self.resource_accesses.remove(entity);
        self.remove_entity_from_string_index(&self.event_line_index, entity);
        self.remove_entity_from_string_index(&self.shared_type_index, entity);
        self.event_lines.remove(entity);
        self.shared_providers.remove(entity);
        Some(snapshot)
    }

    pub fn prune_dropped(&self, dropped_before: SystemTime, limit: usize) -> usize {
        if limit == 0 {
            return 0;
        }
        let unit_ids = self
            .all_units()
            .into_iter()
            .filter(|unit| {
                unit.lifecycle.status == UnitLifecycleStatus::Dropped
                    && unit
                        .lifecycle
                        .dropped_at
                        .is_some_and(|dropped_at| dropped_at <= dropped_before)
            })
            .take(limit)
            .map(|unit| unit.identity.unit_id)
            .collect::<Vec<_>>();

        unit_ids
            .iter()
            .filter(|unit_id| self.remove_unit(unit_id).is_some())
            .count()
    }

    pub fn resource_access_of(&self, unit_id: &str) -> Option<ResourceAccessComponent> {
        let entity = self.unit_entity(unit_id)?;
        self.resource_accesses.get(entity).map(|entry| entry.value)
    }

    pub fn declare_resource_access(
        &self,
        owner_id: &str,
        resource_key: &str,
        access_mode: AccessMode,
    ) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(owner_id) else {
            return false;
        };
        self.resource_accesses
            .modify(entity, |access| {
                access.declare_owned(resource_key, access_mode)
            })
            .is_some()
    }

    pub fn grant_resource_access(
        &self,
        to_unit_id: &str,
        resource_key: &str,
        access_mode: AccessMode,
    ) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(to_unit_id) else {
            return false;
        };
        self.resource_accesses
            .modify(entity, |access| {
                access.grant_resource(resource_key, access_mode)
            })
            .is_some()
    }

    pub fn transfer_resource_ownership(
        &self,
        resource_key: &str,
        from_owner_id: &str,
        to_owner_id: &str,
    ) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(from_entity) = self.unit_entity(from_owner_id) else {
            return false;
        };
        let Some(to_entity) = self.unit_entity(to_owner_id) else {
            return false;
        };

        let Some(Some(access_mode)) = self
            .resource_accesses
            .modify(from_entity, |access| access.remove_owned(resource_key))
        else {
            return false;
        };
        self.resource_accesses
            .modify(to_entity, |access| {
                access.declare_owned(resource_key, access_mode);
                access.revoke_grant(resource_key);
            })
            .is_some()
    }

    pub fn revoke_unit_resource_access(&self, unit_id: &str) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(unit_id) else {
            return false;
        };
        self.resource_accesses
            .modify(entity, |access| {
                access.clear_owned();
                access.granted_resources.clear();
            })
            .is_some()
    }

    pub fn declare_event_line(
        &self,
        owner_unit_id: &str,
        line_name: &str,
        policy: EventLinePolicy,
    ) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(owner_unit_id) else {
            return false;
        };
        let changed = self.event_lines.modify(entity, |event_lines| {
            event_lines.declare_line(line_name, policy)
        });
        if changed.is_some() {
            self.event_line_index
                .entry(line_name.to_string())
                .or_default()
                .insert(entity);
        }
        changed.unwrap_or(false)
    }

    pub fn set_default_event_line(&self, owner_unit_id: &str, line_name: Option<&str>) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(owner_unit_id) else {
            return false;
        };
        self.event_lines
            .modify(entity, |event_lines| {
                event_lines.set_default_line(line_name)
            })
            .unwrap_or(false)
    }

    pub fn event_lines_of(&self, owner_unit_id: &str) -> Vec<EventLineStateSnapshot> {
        let Some(entity) = self.unit_entity(owner_unit_id) else {
            return Vec::new();
        };
        self.event_lines_of_entity(owner_unit_id, entity)
    }

    pub fn owners_of_event_line(&self, line_name: &str) -> Vec<EventLineStateSnapshot> {
        self.event_line_index
            .get(line_name)
            .map(|entities| {
                entities
                    .iter()
                    .filter_map(|entry| {
                        let entity = *entry;
                        let owner_unit_id = self.reverse_index.get(&entity)?.clone();
                        self.event_line_of_entity(&owner_unit_id, entity, line_name)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn event_lines_of_entity(
        &self,
        owner_unit_id: &str,
        entity: UnitEntityId,
    ) -> Vec<EventLineStateSnapshot> {
        self.event_lines
            .get(entity)
            .map(|entry| {
                entry
                    .value
                    .lines
                    .values()
                    .map(|line| EventLineStateSnapshot {
                        owner_unit_id: owner_unit_id.to_string(),
                        line_name: line.line_name.clone(),
                        policy_name: format!("{:?}", line.policy),
                        is_default: line.is_default,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn event_line_of_entity(
        &self,
        owner_unit_id: &str,
        entity: UnitEntityId,
        line_name: &str,
    ) -> Option<EventLineStateSnapshot> {
        self.event_lines
            .get(entity)?
            .value
            .lines
            .get(line_name)
            .map(|line| EventLineStateSnapshot {
                owner_unit_id: owner_unit_id.to_string(),
                line_name: line.line_name.clone(),
                policy_name: format!("{:?}", line.policy),
                is_default: line.is_default,
            })
    }

    pub fn provide_shared_component(&self, unit_id: &str, type_name: &'static str) -> bool {
        let _mutation = self.mutation_lock.lock();
        let Some(entity) = self.unit_entity(unit_id) else {
            return false;
        };
        let changed = self
            .shared_providers
            .modify(entity, |provider| provider.provide(type_name));
        if changed.is_some() {
            self.shared_type_index
                .entry(type_name.to_string())
                .or_default()
                .insert(entity);
        }
        changed.unwrap_or(false)
    }

    pub fn shared_components_of(&self, unit_id: &str) -> Vec<SharedComponentStateSnapshot> {
        let Some(entity) = self.unit_entity(unit_id) else {
            return Vec::new();
        };
        self.shared_components_of_entity(unit_id, entity)
    }

    pub fn owners_of_shared_component(&self, type_name: &str) -> Vec<SharedComponentStateSnapshot> {
        self.shared_type_index
            .get(type_name)
            .map(|entities| {
                entities
                    .iter()
                    .filter_map(|entry| {
                        let entity = *entry;
                        let unit_id = self.reverse_index.get(&entity)?.clone();
                        self.shared_component_of_entity(&unit_id, entity, type_name)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn shared_components_of_entity(
        &self,
        unit_id: &str,
        entity: UnitEntityId,
    ) -> Vec<SharedComponentStateSnapshot> {
        self.shared_providers
            .get(entity)
            .map(|entry| {
                entry
                    .value
                    .shared_components
                    .iter()
                    .map(|shared| SharedComponentStateSnapshot {
                        unit_id: unit_id.to_string(),
                        type_name: shared.type_name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn shared_component_of_entity(
        &self,
        unit_id: &str,
        entity: UnitEntityId,
        type_name: &str,
    ) -> Option<SharedComponentStateSnapshot> {
        self.shared_providers
            .get(entity)?
            .value
            .shared_components
            .iter()
            .find(|shared| shared.type_name == type_name)
            .map(|shared| SharedComponentStateSnapshot {
                unit_id: unit_id.to_string(),
                type_name: shared.type_name.clone(),
            })
    }

    fn insert_indexes(
        &self,
        entity: UnitEntityId,
        identity: &UnitIdentityComponent,
        hierarchy: &HierarchyComponent,
        scope: &ScopeComponent,
    ) {
        self.active_index.insert(entity);
        self.type_index
            .entry(identity.unit_type)
            .or_default()
            .insert(entity);
        if let Some(parent_id) = &hierarchy.parent_id {
            self.parent_index
                .entry(parent_id.clone())
                .or_default()
                .insert(entity);
        } else {
            self.root_index.insert(entity);
        }
        for ancestor_id in &hierarchy.ancestor_ids {
            self.ancestor_index
                .entry(ancestor_id.clone())
                .or_default()
                .insert(entity);
        }
        self.insert_scope_indexes(entity, scope);
    }

    fn remove_indexes(
        &self,
        entity: UnitEntityId,
        identity: &UnitIdentityComponent,
        hierarchy: &HierarchyComponent,
        scope: &ScopeComponent,
    ) {
        self.active_index.remove(&entity);
        if let Some(entities) = self.type_index.get(&identity.unit_type) {
            entities.remove(&entity);
        }
        if let Some(parent_id) = &hierarchy.parent_id {
            if let Some(entities) = self.parent_index.get(parent_id) {
                entities.remove(&entity);
            }
        } else {
            self.root_index.remove(&entity);
        }
        for ancestor_id in &hierarchy.ancestor_ids {
            if let Some(entities) = self.ancestor_index.get(ancestor_id) {
                entities.remove(&entity);
            }
        }
        self.remove_scope_indexes(entity, scope);
    }

    fn insert_scope_indexes(&self, entity: UnitEntityId, scope: &ScopeComponent) {
        self.scope_index
            .entry(scope.scope_id.clone())
            .or_default()
            .insert(entity);
        if let Some(conversation_id) = &scope.conversation_id {
            self.conversation_index
                .entry(conversation_id.clone())
                .or_default()
                .insert(entity);
        }
    }

    fn remove_scope_indexes(&self, entity: UnitEntityId, scope: &ScopeComponent) {
        if let Some(entities) = self.scope_index.get(&scope.scope_id) {
            entities.remove(&entity);
        }
        if let Some(conversation_id) = &scope.conversation_id {
            if let Some(entities) = self.conversation_index.get(conversation_id) {
                entities.remove(&entity);
            }
        }
    }

    fn remove_entity_from_string_index(
        &self,
        index: &DashMap<String, DashSet<UnitEntityId>>,
        entity: UnitEntityId,
    ) {
        for entities in index.iter() {
            entities.remove(&entity);
        }
    }

    fn snapshots_from_string_index(
        &self,
        index: &DashMap<String, DashSet<UnitEntityId>>,
        key: &str,
    ) -> Vec<EcsUnitSnapshot> {
        index
            .get(key)
            .map(|entities| self.snapshots_from_entities(entities.iter().map(|entry| *entry)))
            .unwrap_or_default()
    }

    fn snapshots_from_entities(
        &self,
        entities: impl IntoIterator<Item = UnitEntityId>,
    ) -> Vec<EcsUnitSnapshot> {
        entities
            .into_iter()
            .filter_map(|entity| self.snapshot(entity))
            .collect()
    }
}

impl Default for EcsWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn spawn_child_and_mark_drop() {
        let world = EcsWorld::new();
        let parent = world.spawn_unit(SpawnUnitCommand {
            unit_id: "module:parent".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope-a".to_string(),
            cache_scope_id: "scope-a".to_string(),
            conversation_id: None,
        });
        let child = world.spawn_unit(SpawnUnitCommand {
            unit_id: "statemachine:child".to_string(),
            unit_type: UnitType::StateMachine,
            parent_id: Some("module:parent".to_string()),
            ancestor_ids: vec!["module:parent".to_string()],
            scope_id: "scope-a".to_string(),
            cache_scope_id: "scope-a:unit:statemachine:child".to_string(),
            conversation_id: None,
        });

        assert_ne!(parent, child);
        assert_eq!(world.children_of("module:parent").len(), 1);
        assert_eq!(world.descendants_of("module:parent").len(), 1);
        assert_eq!(world.root_units().len(), 1);
        assert_eq!(world.active_units().len(), 2);
        assert_eq!(world.units_in_scope("scope-a").len(), 2);
        assert_eq!(world.units_by_type(UnitType::StateMachine).len(), 1);
        assert!(world.mark_unit_dropped("statemachine:child"));

        let snapshot = world.unit("statemachine:child").unwrap();
        assert_eq!(snapshot.lifecycle.status, UnitLifecycleStatus::Dropped);
        assert!(snapshot.lifecycle.dropped_at.is_some());
        assert_eq!(world.active_units().len(), 1);
        assert_eq!(world.active_descendants_of("module:parent").len(), 0);
    }

    #[test]
    fn scope_updates_keep_indexes_current() {
        let world = EcsWorld::new();
        let entity = world.spawn_unit(SpawnUnitCommand {
            unit_id: "module:unit".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope-a".to_string(),
            cache_scope_id: "scope-a".to_string(),
            conversation_id: Some("a".to_string()),
        });

        assert_eq!(world.units_in_scope("scope-a").len(), 1);
        assert_eq!(world.units_in_conversation("a").len(), 1);

        assert!(world.update_scope(
            entity,
            ScopeComponent {
                scope_id: "scope-b".to_string(),
                cache_scope_id: "scope-b".to_string(),
                conversation_id: Some("b".to_string()),
            },
        ));

        assert!(world.units_in_scope("scope-a").is_empty());
        assert!(world.units_in_conversation("a").is_empty());
        assert_eq!(world.units_in_scope("scope-b").len(), 1);
        assert_eq!(world.units_in_conversation("b").len(), 1);
    }

    #[test]
    fn resource_access_mirror_tracks_declares_grants_and_transfers() {
        let world = EcsWorld::new();
        world.spawn_unit(SpawnUnitCommand {
            unit_id: "module:owner".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope-a".to_string(),
            cache_scope_id: "scope-a".to_string(),
            conversation_id: None,
        });
        world.spawn_unit(SpawnUnitCommand {
            unit_id: "module:reader".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope-a".to_string(),
            cache_scope_id: "scope-a".to_string(),
            conversation_id: None,
        });

        assert!(world.declare_resource_access(
            "module:owner",
            "resource:config",
            AccessMode::Owner,
        ));
        assert!(world.grant_resource_access("module:reader", "resource:config", AccessMode::Read,));

        let owner_access = world.resource_access_of("module:owner").unwrap();
        assert_eq!(
            owner_access.owned_resources.get("resource:config"),
            Some(&AccessMode::Owner)
        );
        let reader_access = world.resource_access_of("module:reader").unwrap();
        assert_eq!(
            reader_access.granted_resources.get("resource:config"),
            Some(&AccessMode::Read)
        );

        assert!(world.transfer_resource_ownership(
            "resource:config",
            "module:owner",
            "module:reader",
        ));
        let owner_access = world.resource_access_of("module:owner").unwrap();
        assert!(!owner_access.owned_resources.contains_key("resource:config"));
        let reader_access = world.resource_access_of("module:reader").unwrap();
        assert_eq!(
            reader_access.owned_resources.get("resource:config"),
            Some(&AccessMode::Owner)
        );
        assert!(!reader_access
            .granted_resources
            .contains_key("resource:config"));

        assert!(world.revoke_unit_resource_access("module:reader"));
        let reader_access = world.resource_access_of("module:reader").unwrap();
        assert!(reader_access.owned_resources.is_empty());
        assert!(reader_access.granted_resources.is_empty());
    }

    #[test]
    fn event_lines_and_shared_components_are_structured_components() {
        let world = EcsWorld::new();
        world.spawn_unit(SpawnUnitCommand {
            unit_id: "module:owner".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope-a".to_string(),
            cache_scope_id: "scope-a".to_string(),
            conversation_id: None,
        });

        assert!(world.declare_event_line("module:owner", "updates", EventLinePolicy::subtree(),));
        assert!(world.set_default_event_line("module:owner", Some("updates")));
        assert!(world.provide_shared_component("module:owner", std::any::type_name::<String>()));

        let lines = world.event_lines_of("module:owner");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_name, "updates");
        assert!(lines[0].is_default);
        assert!(lines[0].policy_name.contains("Subtree"));

        let shared_components = world.shared_components_of("module:owner");
        assert_eq!(shared_components.len(), 1);
        assert_eq!(
            shared_components[0].type_name,
            std::any::type_name::<String>()
        );

        let owners = world.owners_of_event_line("updates");
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].owner_unit_id, "module:owner");

        let shared_component_owners =
            world.owners_of_shared_component(std::any::type_name::<String>());
        assert_eq!(shared_component_owners.len(), 1);
        assert_eq!(shared_component_owners[0].unit_id, "module:owner");

        assert!(!world.declare_event_line("module:owner", "updates", EventLinePolicy::subtree(),));
        assert!(!world.provide_shared_component("module:owner", std::any::type_name::<String>()));
        assert_eq!(world.owners_of_event_line("updates").len(), 1);
        assert_eq!(
            world
                .owners_of_shared_component(std::any::type_name::<String>())
                .len(),
            1
        );

        assert!(world.remove_unit("module:owner").is_some());
        assert!(world.owners_of_event_line("updates").is_empty());
        assert!(world
            .owners_of_shared_component(std::any::type_name::<String>())
            .is_empty());
    }

    #[test]
    fn concurrent_structural_writes_do_not_lose_updates() {
        let world = Arc::new(EcsWorld::new());
        let command = SpawnUnitCommand {
            unit_id: "module:concurrent".to_string(),
            unit_type: UnitType::Module,
            parent_id: None,
            ancestor_ids: Vec::new(),
            scope_id: "scope:concurrent".to_string(),
            cache_scope_id: "scope:concurrent".to_string(),
            conversation_id: None,
        };

        let handles = (0..16)
            .map(|index| {
                let world = Arc::clone(&world);
                let command = command.clone();
                std::thread::spawn(move || {
                    let entity = world.spawn_unit(command);
                    assert!(world.grant_resource_access(
                        "module:concurrent",
                        &format!("resource:{index}"),
                        AccessMode::Read,
                    ));
                    entity
                })
            })
            .collect::<Vec<_>>();

        let entities = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(entities.iter().all(|entity| *entity == entities[0]));
        assert_eq!(world.all_units().len(), 1);
        assert_eq!(
            world
                .resource_access_of("module:concurrent")
                .unwrap()
                .granted_resources
                .len(),
            16
        );
    }

    #[test]
    fn prune_dropped_is_bounded_and_preserves_active_units() {
        let world = EcsWorld::new();
        for unit_id in ["module:dropped-a", "module:dropped-b", "module:active"] {
            world.spawn_unit(SpawnUnitCommand {
                unit_id: unit_id.to_string(),
                unit_type: UnitType::Module,
                parent_id: None,
                ancestor_ids: Vec::new(),
                scope_id: "scope:prune".to_string(),
                cache_scope_id: "scope:prune".to_string(),
                conversation_id: None,
            });
        }
        let dropped_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
        assert!(world.mark_unit_dropped_at("module:dropped-a", dropped_at));
        assert!(world.mark_unit_dropped_at("module:dropped-b", dropped_at));

        assert_eq!(
            world.prune_dropped(dropped_at + std::time::Duration::from_secs(1), 1,),
            1
        );
        assert_eq!(world.all_units().len(), 2);
        assert!(world.unit("module:active").is_some());
        assert_eq!(world.prune_dropped(SystemTime::now(), usize::MAX), 1);
        assert_eq!(world.all_units().len(), 1);
    }
}
