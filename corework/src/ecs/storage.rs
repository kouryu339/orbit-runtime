use crate::ecs::entity::UnitEntityId;
use dashmap::DashMap;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Versioned<T> {
    pub value: T,
    pub version: u64,
    pub changed_at: SystemTime,
}

impl<T> Versioned<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            version: 0,
            changed_at: SystemTime::now(),
        }
    }

    pub fn update(&mut self, value: T) {
        self.value = value;
        self.version = self.version.saturating_add(1);
        self.changed_at = SystemTime::now();
    }
}

#[derive(Debug)]
pub struct ComponentStore<T> {
    values: DashMap<UnitEntityId, Versioned<T>>,
}

impl<T> ComponentStore<T> {
    pub fn new() -> Self {
        Self {
            values: DashMap::new(),
        }
    }

    pub fn insert(&self, entity: UnitEntityId, value: T) {
        self.values.insert(entity, Versioned::new(value));
    }

    pub fn update(&self, entity: UnitEntityId, value: T) -> bool {
        if let Some(mut existing) = self.values.get_mut(&entity) {
            existing.update(value);
            true
        } else {
            false
        }
    }

    pub fn modify<R>(&self, entity: UnitEntityId, modify: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut existing = self.values.get_mut(&entity)?;
        let result = modify(&mut existing.value);
        existing.version = existing.version.saturating_add(1);
        existing.changed_at = SystemTime::now();
        Some(result)
    }

    pub fn remove(&self, entity: UnitEntityId) -> Option<Versioned<T>> {
        self.values.remove(&entity).map(|(_, value)| value)
    }

    pub fn contains(&self, entity: UnitEntityId) -> bool {
        self.values.contains_key(&entity)
    }
}

impl<T: Clone> ComponentStore<T> {
    pub fn get(&self, entity: UnitEntityId) -> Option<Versioned<T>> {
        self.values.get(&entity).map(|value| value.clone())
    }

    pub fn values(&self) -> Vec<(UnitEntityId, Versioned<T>)> {
        self.values
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }
}

impl<T> Default for ComponentStore<T> {
    fn default() -> Self {
        Self::new()
    }
}
