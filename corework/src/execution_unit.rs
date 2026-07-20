//! Execution units own scoped runtime state, event lines, hierarchy metadata,
//! and explicit resource permissions.
//! ```rust,ignore
//! unit.declare_resource_access("product:123", AccessMode::ReadWrite)?;
//!
//! unit.declare_resource_access("scores:config", AccessMode::Owner)?;
//! ```

use crate::cache::Cache;
use crate::ecs::{SpawnUnitCommand, UnitEntityId};
use crate::error::{FrameworkError, Result};
use crate::event::EventBus;
use crate::event_line::{EventLine, EventLineHandle, EventLinePolicy};
use crate::hierarchical_cache::HierarchicalCache;
use crate::monitoring::Telemetry;
use crate::orchestration::Context;
use crate::runtime_state::RuntimeStateStore;
use crate::scoped_cache::ScopedCache;
use crate::system::SystemRegistry;
use crate::world::{FrameworkState, OrchestrationWorld};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessMode {
    Read,
    ReadWrite,
    Owner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceClaim {
    pub resource_key: String,
    pub owner_id: String,
    pub access_mode: AccessMode,
    pub claimed_at: std::time::SystemTime,
}

pub struct ResourceRegistry {
    // Claims and grants are the authoritative in-process access table. The
    // runtime-state store is updated separately as an observable mirror.
    claims: RwLock<HashMap<String, ResourceClaim>>,
    grants: RwLock<HashMap<String, HashMap<String, AccessMode>>>,
}

impl ResourceRegistry {
    pub fn new() -> Self {
        Self {
            claims: RwLock::new(HashMap::new()),
            grants: RwLock::new(HashMap::new()),
        }
    }

    pub fn global() -> &'static ResourceRegistry {
        static REGISTRY: std::sync::OnceLock<ResourceRegistry> = std::sync::OnceLock::new();
        REGISTRY.get_or_init(ResourceRegistry::new)
    }

    pub fn declare(
        &self,
        resource_key: &str,
        owner_id: &str,
        access_mode: AccessMode,
    ) -> Result<()> {
        let mut claims = self.claims.write();

        if let Some(existing) = claims.get(resource_key) {
            if existing.owner_id != owner_id {
                return Err(FrameworkError::InvalidOperation(format!(
                    "resource '{}' is already owned by '{}'",
                    resource_key, existing.owner_id
                )));
            }
        }

        claims.insert(
            resource_key.to_string(),
            ResourceClaim {
                resource_key: resource_key.to_string(),
                owner_id: owner_id.to_string(),
                access_mode,
                claimed_at: std::time::SystemTime::now(),
            },
        );

        tracing::debug!(
            resource_key = %resource_key,
            owner_id = %owner_id,
            access_mode = ?access_mode,
            "resource declared"
        );

        Ok(())
    }

    pub fn grant_access(
        &self,
        resource_key: &str,
        from_owner: &str,
        to_unit: &str,
        mode: AccessMode,
    ) -> Result<()> {
        let claims = self.claims.read();

        let claim = claims.get(resource_key).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("resource '{}' is not declared", resource_key))
        })?;

        if claim.owner_id != from_owner {
            return Err(FrameworkError::InvalidOperation(format!(
                "owner '{}' cannot grant resource '{}'",
                claim.owner_id, resource_key
            )));
        }

        if mode == AccessMode::Owner {
            return Err(FrameworkError::InvalidOperation(
                "owner access cannot be granted; use transfer_ownership".to_string(),
            ));
        }

        drop(claims);

        let mut grants = self.grants.write();
        grants
            .entry(resource_key.to_string())
            .or_default()
            .insert(to_unit.to_string(), mode);

        tracing::debug!(
            resource_key = %resource_key,
            from = %from_owner,
            to = %to_unit,
            mode = ?mode,
            "resource access granted"
        );

        Ok(())
    }

    pub fn transfer_ownership(
        &self,
        resource_key: &str,
        from_owner: &str,
        to_owner: &str,
    ) -> Result<()> {
        let mut claims = self.claims.write();

        let claim = claims.get_mut(resource_key).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("resource '{}' is not declared", resource_key))
        })?;

        if claim.owner_id != from_owner {
            return Err(FrameworkError::InvalidOperation(format!(
                "owner '{}' cannot transfer resource '{}'",
                claim.owner_id, resource_key
            )));
        }

        claim.owner_id = to_owner.to_string();

        tracing::info!(
            resource_key = %resource_key,
            from = %from_owner,
            to = %to_owner,
            "resource ownership transferred"
        );

        Ok(())
    }

    /// Check access permission.
    pub fn check_access(&self, resource_key: &str, unit_id: &str, mode: AccessMode) -> bool {
        let claims = self.claims.read();
        let grants = self.grants.read();

        if let Some(claim) = claims.get(resource_key) {
            if claim.owner_id == unit_id {
                return true;
            }

            if let Some(granted_units) = grants.get(resource_key) {
                if let Some(granted_mode) = granted_units.get(unit_id) {
                    return match mode {
                        AccessMode::Read => true,
                        AccessMode::ReadWrite => {
                            *granted_mode == AccessMode::ReadWrite
                                && claim.access_mode != AccessMode::Read
                        }
                        AccessMode::Owner => false,
                    };
                }
            }
        }

        false
    }

    /// Revoke resource declarations for a unit.
    pub fn revoke(&self, owner_id: &str) {
        let mut claims = self.claims.write();
        claims.retain(|_, claim| claim.owner_id != owner_id);

        let mut grants = self.grants.write();
        grants.retain(|_, units| {
            units.remove(owner_id);
            !units.is_empty()
        });

        tracing::debug!(owner_id = %owner_id, "resource declarations revoked");
    }

    /// Return resources owned by a unit.
    pub fn get_owned_resources(&self, owner_id: &str) -> Vec<String> {
        let claims = self.claims.read();
        claims
            .iter()
            .filter(|(_, claim)| claim.owner_id == owner_id)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn owner_of(&self, resource_key: &str) -> Option<String> {
        self.claims
            .read()
            .get(resource_key)
            .map(|claim| claim.owner_id.clone())
    }

    /// List all declared resources.
    pub fn list_all_resources(&self) -> Vec<String> {
        let claims = self.claims.read();
        claims.keys().cloned().collect()
    }

    /// Return grants for a resource.
    pub fn get_grants(&self, resource_key: &str) -> Vec<String> {
        let grants = self.grants.read();
        grants
            .get(resource_key)
            .map(|units| units.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Return grants held by a unit.
    pub fn get_unit_grants(&self, unit_id: &str) -> Vec<String> {
        let grants = self.grants.read();
        grants
            .iter()
            .filter(|(_, units)| units.contains_key(unit_id))
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn grant_access_batch(
        &self,
        resource_key: &str,
        from_owner: &str,
        to_units: &[String],
        mode: AccessMode,
    ) -> Result<()> {
        for unit_id in to_units {
            self.grant_access(resource_key, from_owner, unit_id, mode)?;
        }
        Ok(())
    }
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnitType {
    Blueprint,
    StateMachine,
    Module,
}

pub const MAX_EXECUTION_UNIT_DEPTH: usize = 16;
const DEFAULT_EVENT_LINE: &str = "__corework_default";

///
pub struct ExecutionUnit {
    unit_id: String,

    /// Parent execution unit. Weak ownership keeps the hierarchy acyclic.
    parent: Option<Weak<ExecutionUnit>>,

    /// Immutable root-to-parent lineage used for authorization and diagnostics.
    ancestor_unit_ids: Vec<String>,

    scope_id: String,

    /// Cache namespace. StateMachine units that share an event scope still keep
    /// their agent-local state in a per-unit cache namespace.
    cache_scope_id: String,

    /// Conversation id parsed from the event scope, when this unit belongs to a conversation.
    conversation_id: Option<String>,

    /// ECS mirror entity id.
    entity_id: Option<UnitEntityId>,

    /// Execution unit kind.
    unit_type: UnitType,

    framework: FrameworkState,

    /// Local cache visible only to this execution unit.
    scoped_cache: Arc<ScopedCache>,

    /// Explicit subtree/global cache views bound to the execution-unit hierarchy.
    hierarchical_cache: Arc<HierarchicalCache>,

    /// Named event lines owned by this execution unit.
    event_lines: RwLock<HashMap<String, Arc<EventLine>>>,

    /// Event line inherited by this unit's subtree as the default EventBus.
    default_event_line_name: RwLock<Option<String>>,

    /// Shared liveness token used by handles without retaining the owner.
    event_line_liveness: Arc<()>,
    // Shared components are intentionally a tiny cold-path collection. A Vec
    // keeps empty units allocation-free and avoids HashMap bucket overhead.
    shared_components: RwLock<Vec<(TypeId, Arc<dyn Any + Send + Sync>)>>,

    resource_registry: &'static ResourceRegistry,
}

impl ExecutionUnit {
    fn generate_unit_id(unit_type: UnitType) -> String {
        use uuid::Uuid;
        format!(
            "{}:{}",
            match unit_type {
                UnitType::Blueprint => "blueprint",
                UnitType::Module => "module",
                UnitType::StateMachine => "statemachine",
            },
            Uuid::new_v4()
        )
    }

    ///
    /// Root units use `new_root` or `new_root_in_scope`.
    /// Child units must use `new_child` with a real parent unit.
    ///
    /// ```rust,ignore
    /// let parent = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    ///
    /// // Create a child.
    /// let child = ExecutionUnit::new_child(UnitType::StateMachine, &parent)?;
    /// ```
    pub fn new_root(unit_type: UnitType, framework: FrameworkState) -> Self {
        Self::new_internal(unit_type, framework, None, None, Vec::new())
    }

    pub fn new_root_in_scope(
        unit_type: UnitType,
        framework: FrameworkState,
        scope_id: impl Into<String>,
    ) -> Self {
        Self::new_internal(
            unit_type,
            framework,
            Some(scope_id.into()),
            None,
            Vec::new(),
        )
    }

    /// Create a real child execution unit.
    ///
    /// Unlike `new(..., Some(scope_id))`, this records the parent identity and
    /// lineage in addition to inheriting the parent's event scope.
    pub fn new_child(unit_type: UnitType, parent: &Arc<ExecutionUnit>) -> Result<Self> {
        Self::validate_parent(parent)?;
        let mut ancestor_unit_ids = parent.ancestor_unit_ids.clone();
        ancestor_unit_ids.push(parent.unit_id.clone());
        Ok(Self::new_internal(
            unit_type,
            parent.framework.clone(),
            Some(parent.scope_id.clone()),
            Some(Arc::downgrade(parent)),
            ancestor_unit_ids,
        ))
    }

    fn validate_parent(parent: &ExecutionUnit) -> Result<()> {
        if parent.depth() >= MAX_EXECUTION_UNIT_DEPTH {
            return Err(FrameworkError::ValidationError(format!(
                "execution unit hierarchy exceeds maximum depth {MAX_EXECUTION_UNIT_DEPTH}"
            )));
        }
        Self::validate_lineage(parent.id(), &parent.ancestor_unit_ids)
    }

    fn validate_lineage(unit_id: &str, ancestor_unit_ids: &[String]) -> Result<()> {
        if ancestor_unit_ids
            .iter()
            .any(|ancestor_id| ancestor_id == unit_id)
        {
            return Err(FrameworkError::ValidationError(format!(
                "execution unit hierarchy contains a cycle at {}",
                unit_id
            )));
        }
        let unique_count = ancestor_unit_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        if unique_count != ancestor_unit_ids.len() {
            return Err(FrameworkError::ValidationError(
                "execution unit hierarchy contains duplicate ancestors".to_string(),
            ));
        }
        Ok(())
    }

    fn new_internal(
        unit_type: UnitType,
        framework: FrameworkState,
        explicit_scope_id: Option<String>,
        parent: Option<Weak<ExecutionUnit>>,
        ancestor_unit_ids: Vec<String>,
    ) -> Self {
        let unit_id = Self::generate_unit_id(unit_type);
        let is_child = parent.is_some();
        let scope_id = explicit_scope_id.unwrap_or_else(|| unit_id.clone());

        let cache_scope_id = if is_child {
            format!("{}:unit:{}", scope_id, unit_id)
        } else {
            scope_id.clone()
        };
        let scoped_cache = Arc::new(ScopedCache::new(framework.cache(), cache_scope_id.clone()));
        let hierarchical_cache = Arc::new(HierarchicalCache::new(
            framework.cache(),
            scoped_cache.clone(),
            unit_id.clone(),
            &ancestor_unit_ids,
        ));
        let conversation_id = conversation_id_from_scope(&scope_id);
        let parent_id = ancestor_unit_ids.last().cloned();
        let entity_id = Some(
            framework
                .runtime_state()
                .units()
                .spawn_unit(SpawnUnitCommand {
                    unit_id: unit_id.clone(),
                    unit_type,
                    parent_id,
                    ancestor_ids: ancestor_unit_ids.clone(),
                    scope_id: scope_id.clone(),
                    cache_scope_id: cache_scope_id.clone(),
                    conversation_id: conversation_id.clone(),
                }),
        );

        tracing::debug!(
            unit_id = %unit_id,
            scope_id = %scope_id,
            unit_type = ?unit_type,
            "execution unit created"
        );

        Self {
            unit_id,
            parent,
            ancestor_unit_ids,
            scope_id,
            cache_scope_id,
            conversation_id,
            entity_id,
            unit_type,
            framework,
            scoped_cache,
            hierarchical_cache,
            event_lines: RwLock::new(HashMap::new()),
            default_event_line_name: RwLock::new(
                (!is_child).then(|| DEFAULT_EVENT_LINE.to_string()),
            ),
            event_line_liveness: Arc::new(()),
            shared_components: RwLock::new(Vec::new()),
            resource_registry: ResourceRegistry::global(),
        }
    }

    pub fn id(&self) -> &str {
        &self.unit_id
    }

    pub fn entity_id(&self) -> Option<UnitEntityId> {
        self.entity_id
    }

    /// Return the live parent unit, if this unit was created with `new_child`.
    pub fn parent(&self) -> Option<Arc<ExecutionUnit>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }

    /// Return the parent identity even if the parent has already been dropped.
    pub fn parent_id(&self) -> Option<&str> {
        self.ancestor_unit_ids.last().map(String::as_str)
    }

    /// Return the immutable root-to-parent execution-unit identity chain.
    pub fn ancestor_ids(&self) -> &[String] {
        &self.ancestor_unit_ids
    }

    /// Return this unit's hierarchy depth. Root units have depth zero.
    pub fn depth(&self) -> usize {
        self.ancestor_unit_ids.len()
    }

    /// Check whether this unit belongs to the given ancestor's subtree.
    pub fn is_descendant_of(&self, ancestor: &ExecutionUnit) -> bool {
        self.ancestor_unit_ids
            .iter()
            .any(|unit_id| unit_id == ancestor.id())
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    /// Return cache scope id.
    pub fn cache_scope_id(&self) -> &str {
        &self.cache_scope_id
    }

    /// Return conversation id.
    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    pub fn unit_type(&self) -> UnitType {
        self.unit_type
    }

    pub fn world(&self) -> Arc<OrchestrationWorld> {
        self.framework.world()
    }

    pub fn registry(&self) -> Arc<SystemRegistry> {
        self.framework.registry()
    }

    pub fn event_bus(&self) -> Arc<dyn EventBus> {
        Arc::new(
            self.default_event_line()
                .expect("execution unit must have a default event line"),
        ) as Arc<dyn EventBus>
    }

    pub fn global_event_bus(&self) -> Arc<dyn EventBus> {
        self.framework.event_bus() as Arc<dyn EventBus>
    }

    pub fn telemetry(&self) -> Arc<dyn Telemetry> {
        self.framework.telemetry() as Arc<dyn Telemetry>
    }

    /// Return local cache.
    pub fn cache(&self) -> Arc<dyn Cache> {
        self.scoped_cache.clone() as Arc<dyn Cache>
    }

    pub fn hierarchical_cache(&self) -> Arc<HierarchicalCache> {
        self.hierarchical_cache.clone()
    }

    pub fn state(&self) -> Arc<dyn RuntimeStateStore> {
        self.framework.runtime_state()
    }

    pub fn attach_shared_component<T>(&self, component: Arc<T>) -> Result<()>
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let type_name = std::any::type_name::<T>();
        let mut shared_components = self.shared_components.write();
        if shared_components
            .iter()
            .any(|(attached_type_id, _)| *attached_type_id == type_id)
        {
            return Err(FrameworkError::InvalidOperation(format!(
                "shared component '{}' is already attached to execution unit '{}'",
                type_name, self.unit_id
            )));
        }
        let state = self.state();
        if !state.shared_components().provide(&self.unit_id, type_name) {
            tracing::warn!(
                unit_id = %self.unit_id,
                component_type = type_name,
                backend = ?state.backend_kind(),
                "shared component mirror rejected provider declaration"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "shared component '{}' could not be mirrored for execution unit '{}'",
                type_name, self.unit_id
            )));
        }
        shared_components.push((type_id, component));
        tracing::debug!(
            unit_id = %self.unit_id,
            component_type = type_name,
            component_count = shared_components.len(),
            backend = ?state.backend_kind(),
            "shared component attached"
        );
        Ok(())
    }

    pub fn resolve_shared_component<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let type_name = std::any::type_name::<T>();
        if let Some(component) = self
            .shared_components
            .read()
            .iter()
            .find(|(attached_type_id, _)| *attached_type_id == type_id)
            .map(|(_, component)| Arc::clone(component))
        {
            tracing::trace!(
                unit_id = %self.unit_id,
                provider_unit_id = %self.unit_id,
                component_type = type_name,
                hierarchy_hops = 0usize,
                "shared component resolved"
            );
            return Arc::downcast::<T>(component).ok();
        }

        let mut parent = self.parent();
        let mut hierarchy_hops = 1usize;
        while let Some(unit) = parent {
            if let Some(component) = unit
                .shared_components
                .read()
                .iter()
                .find(|(attached_type_id, _)| *attached_type_id == type_id)
                .map(|(_, component)| Arc::clone(component))
            {
                tracing::trace!(
                    unit_id = %self.unit_id,
                    provider_unit_id = %unit.unit_id,
                    component_type = type_name,
                    hierarchy_hops,
                    "shared component resolved"
                );
                return Arc::downcast::<T>(component).ok();
            }
            parent = unit.parent();
            hierarchy_hops += 1;
        }
        tracing::trace!(
            unit_id = %self.unit_id,
            component_type = type_name,
            hierarchy_depth = self.depth(),
            "shared component resolution missed"
        );
        None
    }

    pub fn create_event_line(
        &self,
        name: impl Into<String>,
        policy: EventLinePolicy,
    ) -> Result<EventLineHandle> {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() {
            return Err(FrameworkError::ValidationError(
                "event line name must not be empty".to_string(),
            ));
        }

        let mut lines = self.event_lines.write();
        if lines.contains_key(name) {
            return Err(FrameworkError::InvalidOperation(format!(
                "event line '{}' already exists on execution unit '{}'",
                name, self.unit_id
            )));
        }

        let state = self.state();
        if !state.events().declare_line(&self.unit_id, name, policy) {
            tracing::error!(
                unit_id = %self.unit_id,
                event_line = name,
                backend = ?state.backend_kind(),
                "runtime state rejected event line declaration"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "event line '{}' could not be mirrored for execution unit '{}'",
                name, self.unit_id
            )));
        }
        let line = Arc::new(EventLine::new(name.to_string(), self, policy));
        lines.insert(name.to_string(), line.clone());
        if self.default_event_line_name.read().as_deref() == Some(name)
            && !state.events().set_default_line(&self.unit_id, Some(name))
        {
            lines.remove(name);
            tracing::error!(
                unit_id = %self.unit_id,
                event_line = name,
                backend = ?state.backend_kind(),
                "runtime state rejected default event line"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "default event line '{}' could not be mirrored for execution unit '{}'",
                name, self.unit_id
            )));
        }
        Ok(EventLineHandle::new(line, self))
    }

    pub(crate) fn event_line_liveness(&self) -> &Arc<()> {
        &self.event_line_liveness
    }

    pub fn set_default_event_line(&self, name: &str) -> Result<()> {
        if !self.event_lines.read().contains_key(name) {
            return Err(FrameworkError::InvalidOperation(format!(
                "execution unit '{}' does not own event line '{}'",
                self.unit_id, name
            )));
        }
        let state = self.state();
        if !state.events().set_default_line(&self.unit_id, Some(name)) {
            tracing::error!(
                unit_id = %self.unit_id,
                event_line = name,
                backend = ?state.backend_kind(),
                "runtime state rejected default event line"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "default event line '{}' could not be mirrored for execution unit '{}'",
                name, self.unit_id
            )));
        }
        *self.default_event_line_name.write() = Some(name.to_string());
        Ok(())
    }

    pub fn clear_default_event_line(&self) {
        let default_line = self
            .parent
            .is_none()
            .then(|| DEFAULT_EVENT_LINE.to_string());
        let state = self.state();
        if state
            .events()
            .set_default_line(&self.unit_id, default_line.as_deref())
        {
            *self.default_event_line_name.write() = default_line;
        } else {
            tracing::error!(
                unit_id = %self.unit_id,
                backend = ?state.backend_kind(),
                "runtime state rejected clearing default event line"
            );
        }
    }

    pub fn default_event_line_name(&self) -> Option<String> {
        if let Some(name) = self.default_event_line_name.read().clone() {
            return Some(name);
        }

        let mut parent = self.parent();
        while let Some(unit) = parent {
            if let Some(name) = unit.default_event_line_name.read().clone() {
                return Some(name);
            }
            parent = unit.parent();
        }
        None
    }

    fn default_event_line(&self) -> Result<EventLineHandle> {
        if let Some(name) = self.default_event_line_name.read().clone() {
            if let Some(line) = self.event_lines.read().get(&name).cloned() {
                return Ok(EventLineHandle::new(line, self));
            }
            match self.create_event_line(name.clone(), EventLinePolicy::subtree()) {
                Ok(line) => return Ok(line),
                Err(_) => {
                    if let Some(line) = self.event_lines.read().get(&name).cloned() {
                        return Ok(EventLineHandle::new(line, self));
                    }
                }
            }
            return Err(FrameworkError::InvalidOperation(format!(
                "default event line '{}' is unavailable",
                name
            )));
        }

        let mut parent = self.parent();
        while let Some(unit) = parent {
            if let Some(name) = unit.default_event_line_name.read().clone() {
                if !unit.event_lines.read().contains_key(&name) {
                    let _ = unit.create_event_line(name.clone(), EventLinePolicy::subtree());
                }
                return unit
                    .event_lines
                    .read()
                    .get(&name)
                    .cloned()
                    .map(|line| EventLineHandle::new(line, self))
                    .ok_or_else(|| {
                        FrameworkError::InvalidOperation(format!(
                            "default event line '{}' is unavailable",
                            name
                        ))
                    });
            }
            parent = unit.parent();
        }

        *self.default_event_line_name.write() = Some(DEFAULT_EVENT_LINE.to_string());
        match self.create_event_line(DEFAULT_EVENT_LINE, EventLinePolicy::subtree()) {
            Ok(line) => Ok(line),
            Err(_) => self
                .event_lines
                .read()
                .get(DEFAULT_EVENT_LINE)
                .cloned()
                .map(|line| EventLineHandle::new(line, self))
                .ok_or_else(|| {
                    FrameworkError::InvalidOperation(
                        "failed to create the execution unit default event line".to_string(),
                    )
                }),
        }
    }

    pub fn event_line(&self, name: &str) -> Result<EventLineHandle> {
        self.try_event_line(name).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "event line '{}' is not visible from execution unit '{}'",
                name, self.unit_id
            ))
        })
    }

    pub fn try_event_line(&self, name: &str) -> Option<EventLineHandle> {
        if let Some(line) = self.event_lines.read().get(name).cloned() {
            let handle = EventLineHandle::new(line, self);
            return handle.has_access().then_some(handle);
        }

        let mut parent = self.parent();
        while let Some(unit) = parent {
            if let Some(line) = unit.event_lines.read().get(name).cloned() {
                let handle = EventLineHandle::new(line, self);
                return handle.has_access().then_some(handle);
            }
            parent = unit.parent();
        }
        None
    }

    /// Create a standard context.
    pub fn create_context(self: &Arc<Self>) -> Context {
        Context::with_world(
            self.cache(),
            self.event_bus(),
            self.telemetry(),
            self.registry(),
            self.world(),
            self.event_bus(),
        )
        .with_scope_id(self.scope_id.clone())
        .with_cache_scope_id(self.cache_scope_id.clone())
        .with_optional_conversation_id(self.conversation_id.clone())
        .with_hierarchical_cache(self.hierarchical_cache.clone())
        .with_runtime_state(self.state())
        .with_execution_unit(self)
    }

    ///
    ///
    /// ```rust,ignore
    ///
    /// // Declare user data access.
    /// unit.declare_resource_access("user:123", AccessMode::ReadWrite)?;
    /// ```
    pub fn declare_resource_access(
        &self,
        resource_key: &str,
        access_mode: AccessMode,
    ) -> Result<()> {
        self.resource_registry
            .declare(resource_key, &self.unit_id, access_mode)?;
        let state = self.framework.runtime_state();
        if !state
            .resources()
            .declare(&self.unit_id, resource_key, access_mode)
        {
            tracing::error!(
                unit_id = %self.unit_id,
                resource_key,
                backend = ?state.backend_kind(),
                "runtime state rejected resource declaration"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "resource '{}' declaration could not be mirrored for execution unit '{}'",
                resource_key, self.unit_id
            )));
        }
        Ok(())
    }

    ///
    /// ```rust,ignore
    ///     "scores:config",
    ///     "module:browser_buns",
    ///     AccessMode::Read,
    /// )?;
    /// ```
    pub fn grant_access_to(
        &self,
        resource_key: &str,
        to_unit_id: &str,
        mode: AccessMode,
    ) -> Result<()> {
        self.resource_registry
            .grant_access(resource_key, &self.unit_id, to_unit_id, mode)?;
        // Wildcard grants are resolved by ResourceRegistry and do not map to a
        // concrete execution-unit entity in the runtime-state mirror.
        if to_unit_id == "*" {
            return Ok(());
        }
        let state = self.framework.runtime_state();
        if !state.resources().grant(to_unit_id, resource_key, mode) {
            tracing::error!(
                unit_id = %self.unit_id,
                target_unit_id = to_unit_id,
                resource_key,
                backend = ?state.backend_kind(),
                "runtime state rejected resource grant"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "resource '{}' grant to '{}' could not be mirrored",
                resource_key, to_unit_id
            )));
        }
        Ok(())
    }

    pub fn transfer_ownership(&self, resource_key: &str, to_unit_id: &str) -> Result<()> {
        self.resource_registry
            .transfer_ownership(resource_key, &self.unit_id, to_unit_id)?;
        let state = self.framework.runtime_state();
        if !state
            .resources()
            .transfer(resource_key, &self.unit_id, to_unit_id)
        {
            tracing::error!(
                unit_id = %self.unit_id,
                target_unit_id = to_unit_id,
                resource_key,
                backend = ?state.backend_kind(),
                "runtime state rejected resource ownership transfer"
            );
            return Err(FrameworkError::InvalidOperation(format!(
                "resource '{}' transfer to '{}' could not be mirrored",
                resource_key, to_unit_id
            )));
        }
        Ok(())
    }

    /// Check access permission.
    pub fn check_access(&self, resource_key: &str, mode: AccessMode) -> bool {
        self.resource_registry
            .check_access(resource_key, &self.unit_id, mode)
    }

    /// Return owned resources.
    pub fn owned_resources(&self) -> Vec<String> {
        self.resource_registry.get_owned_resources(&self.unit_id)
    }

    /// Return all registered resources.
    pub fn list_all_resources(&self) -> Vec<String> {
        self.resource_registry.list_all_resources()
    }

    /// Return resource grants.
    pub fn get_resource_grants(&self, resource_key: &str) -> Vec<String> {
        self.resource_registry.get_grants(resource_key)
    }

    /// Return grants for this unit.
    pub fn my_grants(&self) -> Vec<String> {
        self.resource_registry.get_unit_grants(&self.unit_id)
    }

    pub fn grant_access_batch(
        &self,
        resource_key: &str,
        to_unit_ids: &[String],
        mode: AccessMode,
    ) -> Result<()> {
        self.resource_registry.grant_access_batch(
            resource_key,
            &self.unit_id,
            to_unit_ids,
            mode,
        )?;
        let state = self.framework.runtime_state();
        for to_unit_id in to_unit_ids {
            if to_unit_id == "*" {
                continue;
            }
            if !state.resources().grant(to_unit_id, resource_key, mode) {
                tracing::error!(
                    unit_id = %self.unit_id,
                    target_unit_id = to_unit_id,
                    resource_key,
                    backend = ?state.backend_kind(),
                    "runtime state rejected batched resource grant"
                );
                return Err(FrameworkError::InvalidOperation(format!(
                    "resource '{}' grant to '{}' could not be mirrored",
                    resource_key, to_unit_id
                )));
            }
        }
        Ok(())
    }

    pub fn get_resource<T: for<'de> Deserialize<'de>>(
        &self,
        resource_key: &str,
    ) -> Result<Option<T>> {
        if !self.check_access(resource_key, AccessMode::Read) {
            return Err(FrameworkError::InvalidOperation(format!(
                "execution unit '{}' cannot read resource '{}'",
                self.unit_id, resource_key
            )));
        }

        self.world().get_resource(resource_key)
    }

    pub fn set_resource<T: Serialize>(
        &self,
        resource_key: &str,
        value: &T,
        ttl: Option<std::time::Duration>,
    ) -> Result<()> {
        if !self.check_access(resource_key, AccessMode::ReadWrite) {
            return Err(FrameworkError::InvalidOperation(format!(
                "execution unit '{}' cannot write resource '{}'",
                self.unit_id, resource_key
            )));
        }

        self.world().set_resource(resource_key, value, ttl)
    }

    pub async fn get_resource_cached<T: for<'de> Deserialize<'de>>(
        &self,
        resource_key: &str,
    ) -> Result<Option<T>> {
        if !self.check_access(resource_key, AccessMode::Read) {
            return Err(FrameworkError::InvalidOperation(format!(
                "execution unit '{}' cannot read cached resource '{}'",
                self.unit_id, resource_key
            )));
        }

        let owner_id = self
            .resource_registry
            .owner_of(resource_key)
            .ok_or_else(|| {
                FrameworkError::NotFoundError(format!("resource '{}' has no owner", resource_key))
            })?;
        self.hierarchical_cache
            .get_subtree_from_raw(&owner_id, resource_key)
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(FrameworkError::SerializationError)
    }

    pub async fn set_resource_cached<T: Serialize>(
        &self,
        resource_key: &str,
        value: &T,
        ttl: Option<std::time::Duration>,
    ) -> Result<()> {
        if !self.check_access(resource_key, AccessMode::ReadWrite) {
            return Err(FrameworkError::InvalidOperation(format!(
                "execution unit '{}' cannot write cached resource '{}'",
                self.unit_id, resource_key
            )));
        }

        let owner_id = self
            .resource_registry
            .owner_of(resource_key)
            .ok_or_else(|| {
                FrameworkError::NotFoundError(format!("resource '{}' has no owner", resource_key))
            })?;
        let value = serde_json::to_value(value).map_err(FrameworkError::SerializationError)?;
        self.hierarchical_cache
            .set_subtree_for_raw(&owner_id, resource_key, value, ttl)
            .await
    }
}

fn conversation_id_from_scope(scope_id: &str) -> Option<String> {
    if let Some(conversation_id) = scope_id.strip_prefix("conversation:") {
        if !conversation_id.is_empty() {
            return Some(conversation_id.to_string());
        }
    }
    scope_id
        .rsplit_once(":conversation:")
        .and_then(|(_, conversation_id)| {
            if conversation_id.is_empty() {
                None
            } else {
                Some(conversation_id.to_string())
            }
        })
}

impl Drop for ExecutionUnit {
    fn drop(&mut self) {
        let state = self.framework.runtime_state();
        if !state.units().mark_dropped(&self.unit_id) {
            tracing::warn!(
                unit_id = %self.unit_id,
                backend = ?state.backend_kind(),
                "runtime state could not mark dropped execution unit"
            );
        }
        if !state.resources().revoke_unit(&self.unit_id) {
            tracing::warn!(
                unit_id = %self.unit_id,
                backend = ?state.backend_kind(),
                "runtime state could not revoke dropped unit resources"
            );
        }
        state.maintain();

        self.resource_registry.revoke(&self.unit_id);

        tracing::debug!(
            unit_id = %self.unit_id,
            "execution unit dropped"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BaseEvent, EventHandler};
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[test]
    fn test_resource_registry() {
        let registry = ResourceRegistry::new();

        registry
            .declare("user:123", "module:scores", AccessMode::Owner)
            .unwrap();

        assert!(registry.check_access("user:123", "module:scores", AccessMode::ReadWrite));

        registry
            .grant_access(
                "user:123",
                "module:scores",
                "module:browser",
                AccessMode::Read,
            )
            .unwrap();
        assert!(registry.check_access("user:123", "module:browser", AccessMode::Read));

        registry
            .transfer_ownership("user:123", "module:scores", "module:browser")
            .unwrap();
        assert!(registry.check_access("user:123", "module:browser", AccessMode::Owner));
    }

    #[tokio::test]
    async fn test_execution_unit() {
        let framework = FrameworkState::initialize().unwrap();

        let unit = ExecutionUnit::new_root(UnitType::Module, framework);

        unit.declare_resource_access("test:resource", AccessMode::Owner)
            .unwrap();

        unit.set_resource("test:resource", &"test_value", None)
            .unwrap();

        let value: Option<String> = unit.get_resource("test:resource").unwrap();
        assert_eq!(value, Some("test_value".to_string()));

        let owned = unit.owned_resources();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0], "test:resource");
    }

    #[test]
    fn execution_units_are_mirrored_as_ecs_entities() {
        let framework = FrameworkState::initialize().unwrap();
        let state = framework.runtime_state();
        let parent = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            "conversation:ecs-test",
        ));
        let parent_id = parent.id().to_string();
        let child = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &parent).unwrap());
        let child_id = child.id().to_string();

        assert!(parent.entity_id().is_some());
        assert!(child.entity_id().is_some());

        let parent_snapshot = state.units().unit(&parent_id).unwrap();
        assert_eq!(parent_snapshot.identity.unit_type, UnitType::Module);
        assert_eq!(parent_snapshot.hierarchy.depth, 0);
        assert_eq!(
            parent_snapshot.scope.conversation_id.as_deref(),
            Some("ecs-test")
        );

        let child_snapshot = state.units().unit(&child_id).unwrap();
        assert_eq!(child_snapshot.identity.unit_type, UnitType::StateMachine);
        assert_eq!(
            child_snapshot.hierarchy.parent_id.as_deref(),
            Some(parent_id.as_str())
        );
        assert_eq!(child_snapshot.hierarchy.depth, 1);
        assert_eq!(state.units().children_of(&parent_id).len(), 1);
        assert_eq!(state.units().descendants_of(&parent_id).len(), 1);

        drop(child);
        let dropped_child = state.units().unit(&child_id).unwrap();
        assert_eq!(
            dropped_child.lifecycle.status,
            crate::ecs::UnitLifecycleStatus::Dropped
        );
    }

    #[test]
    fn resource_access_is_mirrored_to_ecs() {
        let framework = FrameworkState::initialize().unwrap();
        let state = framework.runtime_state();
        let owner = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));
        let reader = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
        let resource_key = format!("resource:{}", owner.id());

        owner
            .declare_resource_access(&resource_key, AccessMode::Owner)
            .unwrap();
        owner
            .grant_access_to(&resource_key, reader.id(), AccessMode::Read)
            .unwrap();

        let owner_access = state.resources().access_of(owner.id()).unwrap();
        assert_eq!(
            owner_access.owned_resources.get(&resource_key),
            Some(&AccessMode::Owner)
        );
        let reader_access = state.resources().access_of(reader.id()).unwrap();
        assert_eq!(
            reader_access.granted_resources.get(&resource_key),
            Some(&AccessMode::Read)
        );

        owner
            .transfer_ownership(&resource_key, reader.id())
            .unwrap();
        let owner_access = state.resources().access_of(owner.id()).unwrap();
        assert!(!owner_access.owned_resources.contains_key(&resource_key));
        let reader_access = state.resources().access_of(reader.id()).unwrap();
        assert_eq!(
            reader_access.owned_resources.get(&resource_key),
            Some(&AccessMode::Owner)
        );
        assert!(!reader_access.granted_resources.contains_key(&resource_key));
    }

    #[test]
    fn wildcard_resource_grants_do_not_require_a_synthetic_ecs_unit() {
        let framework = FrameworkState::initialize().unwrap();
        let owner = ExecutionUnit::new_root(UnitType::Module, framework);
        let resource_key = format!("resource:wildcard:{}", owner.id());

        owner
            .declare_resource_access(&resource_key, AccessMode::Owner)
            .unwrap();
        owner
            .grant_access_to(&resource_key, "*", AccessMode::ReadWrite)
            .unwrap();

        assert!(owner
            .resource_registry
            .get_grants(&resource_key)
            .contains(&"*".to_string()));
    }

    #[test]
    fn event_lines_are_mirrored_to_runtime_state() {
        let framework = FrameworkState::initialize().unwrap();
        let state = framework.runtime_state();
        let unit = ExecutionUnit::new_root(UnitType::Module, framework);
        let unit_id = unit.id().to_string();

        unit.create_event_line("updates", EventLinePolicy::subtree())
            .unwrap();
        unit.set_default_event_line("updates").unwrap();

        let lines = state.events().lines_of(&unit_id);
        let line = lines
            .iter()
            .find(|line| line.line_name == "updates")
            .expect("event line should be mirrored");

        assert_eq!(line.owner_unit_id, unit_id);
        assert!(line.is_default);
        assert!(line.policy_name.contains("Subtree"));
    }

    #[test]
    fn shared_components_are_mirrored_to_runtime_state() {
        struct TestSharedComponent;

        let framework = FrameworkState::initialize().unwrap();
        let state = framework.runtime_state();
        let unit = ExecutionUnit::new_root(UnitType::Module, framework);
        let unit_id = unit.id().to_string();

        assert_eq!(unit.shared_components.read().capacity(), 0);
        unit.attach_shared_component(Arc::new(TestSharedComponent))
            .unwrap();

        let shared_components = state.shared_components().components_of(&unit_id);
        assert_eq!(shared_components.len(), 1);
        assert_eq!(shared_components[0].unit_id, unit_id);
        assert_eq!(
            shared_components[0].type_name,
            std::any::type_name::<TestSharedComponent>()
        );
    }

    struct CaptureHandler {
        events: Arc<Mutex<Vec<BaseEvent>>>,
    }

    #[async_trait]
    impl EventHandler for CaptureHandler {
        async fn handle(&self, event: &BaseEvent) -> Result<()> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn execution_unit_context_events_use_local_scope() {
        let framework = FrameworkState::initialize().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            "conversation:conv-test",
        ));
        unit.event_bus()
            .subscribe(
                "test.scope.event".to_string(),
                Arc::new(CaptureHandler {
                    events: Arc::clone(&events),
                }),
            )
            .await
            .unwrap();

        let ctx = unit.create_context();
        ctx.world_event_bus
            .publish(BaseEvent::new("test.scope.event", serde_json::json!({})))
            .await
            .unwrap();

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0].scope_id.as_deref(),
            Some("conversation:conv-test")
        );
        assert_eq!(captured[0].conversation_id.as_deref(), Some("conv-test"));
    }

    #[test]
    fn hierarchy_validation_rejects_cycles_and_duplicate_ancestors() {
        let cycle = ExecutionUnit::validate_lineage(
            "unit:a",
            &["unit:root".to_string(), "unit:a".to_string()],
        );
        assert!(cycle.is_err());

        let duplicate = ExecutionUnit::validate_lineage(
            "unit:a",
            &["unit:root".to_string(), "unit:root".to_string()],
        );
        assert!(duplicate.is_err());
    }
}
