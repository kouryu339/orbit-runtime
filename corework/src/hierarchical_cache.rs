use crate::cache::Cache;
use crate::cache::{CacheExt, CacheValue};
use crate::error::Result;
use crate::scoped_cache::ScopedCache;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

const SUBTREE_NAMESPACE: &str = "__subtree__";

/// Explicit cache views bound to an execution unit.
pub struct HierarchicalCache {
    backend: Arc<dyn Cache>,
    local: Arc<ScopedCache>,
    subtree: Arc<ScopedCache>,
    owner_id: String,
    lineage: Arc<[String]>,
}

impl HierarchicalCache {
    pub fn new(
        backend: Arc<dyn Cache>,
        local: Arc<ScopedCache>,
        owner_id: impl Into<String>,
        ancestor_ids: &[String],
    ) -> Self {
        let owner_id = owner_id.into();
        let mut lineage = Vec::with_capacity(ancestor_ids.len() + 1);
        lineage.push(owner_id.clone());
        lineage.extend(ancestor_ids.iter().rev().cloned());

        Self {
            local,
            subtree: Arc::new(ScopedCache::new(
                backend.clone(),
                format!("{SUBTREE_NAMESPACE}:{owner_id}"),
            )),
            backend,
            owner_id,
            lineage: lineage.into(),
        }
    }

    pub fn local(&self) -> Arc<dyn Cache> {
        self.local.clone()
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn lineage(&self) -> &[String] {
        &self.lineage
    }

    fn subtree_key(owner_id: &str, key: &str) -> String {
        format!("{SUBTREE_NAMESPACE}:{owner_id}:{key}")
    }

    pub async fn get_subtree_raw(&self, key: &str) -> Result<Option<Value>> {
        let keys: Vec<String> = self
            .lineage
            .iter()
            .map(|owner_id| Self::subtree_key(owner_id, key))
            .collect();
        let values = self.backend.mget_raw(&keys).await?;
        Ok(values.into_iter().flatten().next())
    }

    pub(crate) async fn get_subtree_from_raw(
        &self,
        owner_id: &str,
        key: &str,
    ) -> Result<Option<Value>> {
        self.backend
            .get_raw(&Self::subtree_key(owner_id, key))
            .await
    }

    pub(crate) async fn set_subtree_for_raw(
        &self,
        owner_id: &str,
        key: &str,
        value: Value,
        ttl: Option<Duration>,
    ) -> Result<()> {
        ScopedCache::new(
            self.backend.clone(),
            format!("{SUBTREE_NAMESPACE}:{owner_id}"),
        )
        .disable_auto_cleanup()
        .set_raw(key, value, ttl)
        .await
    }

    pub async fn set_subtree_raw(
        &self,
        key: &str,
        value: Value,
        ttl: Option<Duration>,
    ) -> Result<()> {
        self.subtree.set_raw(key, value, ttl).await
    }

    pub async fn delete_subtree_raw(&self, key: &str) -> Result<()> {
        self.subtree.delete(key).await
    }

    pub async fn get_subtree<V: CacheValue>(&self, key: &str) -> Result<Option<V>> {
        self.get_subtree_raw(key)
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub async fn set_subtree<V: CacheValue>(
        &self,
        key: &str,
        value: &V,
        ttl: Option<Duration>,
    ) -> Result<()> {
        self.set_subtree_raw(key, serde_json::to_value(value)?, ttl)
            .await
    }

    pub async fn get_global_raw(&self, key: &str) -> Result<Option<Value>> {
        self.backend.get_raw(key).await
    }

    pub async fn set_global_raw(
        &self,
        key: &str,
        value: Value,
        ttl: Option<Duration>,
    ) -> Result<()> {
        self.backend.set_raw(key, value, ttl).await
    }

    pub async fn delete_global_raw(&self, key: &str) -> Result<()> {
        self.backend.delete(key).await
    }

    pub async fn get_global<V: CacheValue>(&self, key: &str) -> Result<Option<V>> {
        self.backend.get(key).await
    }

    pub async fn set_global<V: CacheValue>(
        &self,
        key: &str,
        value: &V,
        ttl: Option<Duration>,
    ) -> Result<()> {
        self.backend.set(key, value, ttl).await
    }
}
