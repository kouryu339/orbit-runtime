//!

use crate::cache::Cache;
use crate::error::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

///
pub struct ScopedCache {
    inner: Arc<dyn Cache>,
    scope_id: String,
    auto_cleanup: bool,
    tracked_keys: Arc<parking_lot::RwLock<Vec<String>>>,
}

impl ScopedCache {
    pub fn new(cache: Arc<dyn Cache>, scope_id: impl Into<String>) -> Self {
        let scope_id = scope_id.into();
        let tracked_keys = shared_tracked_keys(&scope_id);
        Self {
            inner: cache,
            scope_id,
            auto_cleanup: true,
            tracked_keys,
        }
    }

    pub fn disable_auto_cleanup(mut self) -> Self {
        self.auto_cleanup = false;
        self
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    fn make_scoped_key(&self, key: &str) -> String {
        format!("{}:{}", self.scope_id, key)
    }

    fn track_key(&self, key: &str) {
        let mut keys = self.tracked_keys.write();
        if !keys.contains(&key.to_string()) {
            keys.push(key.to_string());
        }
    }

    pub async fn cleanup(&self) -> Result<()> {
        let keys = self.tracked_keys.read().clone();
        for key in keys.iter() {
            let scoped_key = self.make_scoped_key(key);
            let _ = self.inner.delete(&scoped_key).await;
        }
        self.tracked_keys.write().clear();
        Ok(())
    }

    pub fn stats(&self) -> ScopedCacheStats {
        ScopedCacheStats {
            scope_id: self.scope_id.clone(),
            tracked_keys_count: self.tracked_keys.read().len(),
        }
    }

    ///
    pub async fn dump(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let keys = self.tracked_keys.read().clone();
        let mut result = std::collections::HashMap::new();
        for key in keys.iter() {
            let scoped_key = self.make_scoped_key(key);
            if let Ok(Some(value)) = self.inner.get_raw(&scoped_key).await {
                result.insert(key.clone(), value);
            }
        }
        result
    }

    ///
    pub async fn restore(
        &self,
        snapshot: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        let items: Vec<(String, serde_json::Value)> = snapshot
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.mset_raw(&items, None).await
    }
}

fn shared_tracked_keys(scope_id: &str) -> Arc<parking_lot::RwLock<Vec<String>>> {
    static TRACKED_KEYS: std::sync::OnceLock<
        dashmap::DashMap<String, Weak<parking_lot::RwLock<Vec<String>>>>,
    > = std::sync::OnceLock::new();

    let registry = TRACKED_KEYS.get_or_init(dashmap::DashMap::new);

    if let Some(existing) = registry.get(scope_id) {
        if let Some(keys) = existing.upgrade() {
            return keys;
        }
    }

    let keys = Arc::new(parking_lot::RwLock::new(Vec::new()));
    registry.insert(scope_id.to_string(), Arc::downgrade(&keys));
    keys
}

#[derive(Debug, Clone)]
pub struct ScopedCacheStats {
    pub scope_id: String,
    pub tracked_keys_count: usize,
}

#[async_trait]
impl Cache for ScopedCache {
    async fn get_raw(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let scoped_key = self.make_scoped_key(key);
        self.inner.get_raw(&scoped_key).await
    }

    async fn set_raw(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let scoped_key = self.make_scoped_key(key);
        self.track_key(key);
        self.inner.set_raw(&scoped_key, value, ttl).await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let scoped_key = self.make_scoped_key(key);
        self.inner.delete(&scoped_key).await
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let scoped_key = self.make_scoped_key(key);
        self.inner.exists(&scoped_key).await
    }

    async fn mget_raw(&self, keys: &[String]) -> Result<Vec<Option<serde_json::Value>>> {
        let scoped_keys: Vec<String> = keys.iter().map(|k| self.make_scoped_key(k)).collect();
        self.inner.mget_raw(&scoped_keys).await
    }

    async fn mset_raw(
        &self,
        items: &[(String, serde_json::Value)],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let scoped_items: Vec<(String, serde_json::Value)> = items
            .iter()
            .map(|(k, v)| {
                self.track_key(k);
                (self.make_scoped_key(k), v.clone())
            })
            .collect();
        self.inner.mset_raw(&scoped_items, ttl).await
    }

    async fn dump_raw(&self) -> Result<HashMap<String, serde_json::Value>> {
        Ok(self.dump().await)
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let scoped_key = self.make_scoped_key(key);
        self.track_key(key);
        self.inner.incr(&scoped_key, delta).await
    }

    async fn expire(&self, key: &str, ttl: Duration) -> Result<()> {
        let scoped_key = self.make_scoped_key(key);
        self.inner.expire(&scoped_key, ttl).await
    }

    async fn flush(&self) -> Result<()> {
        self.cleanup().await
    }
}

impl Clone for ScopedCache {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            scope_id: self.scope_id.clone(),
            auto_cleanup: self.auto_cleanup,
            tracked_keys: self.tracked_keys.clone(),
        }
    }
}

impl Drop for ScopedCache {
    fn drop(&mut self) {
        if self.auto_cleanup && Arc::strong_count(&self.tracked_keys) == 1 {
            let inner = self.inner.clone();
            let tracked_keys = self.tracked_keys.read().clone();
            let scope_id = self.scope_id.clone();

            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    for key in tracked_keys.iter() {
                        let scoped_key = format!("{}:{}", scope_id, key);
                        let _ = inner.delete(&scoped_key).await;
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;

    #[tokio::test]
    async fn dump_raw_exports_only_tracked_unscoped_keys() {
        let inner = Arc::new(InMemoryCache::new());
        let scoped = ScopedCache::new(inner.clone(), "dump_scope").disable_auto_cleanup();
        let other = ScopedCache::new(inner, "other_scope").disable_auto_cleanup();

        scoped
            .set_raw("answer", serde_json::json!(42), None)
            .await
            .unwrap();
        other
            .set_raw("answer", serde_json::json!("hidden"), None)
            .await
            .unwrap();

        let snapshot = scoped.dump_raw().await.unwrap();

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.get("answer"), Some(&serde_json::json!(42)));
        assert!(!snapshot.contains_key("dump_scope:answer"));
    }
    /*
    use super::*;
    use crate::cache::{CacheExt, InMemoryCache};

    #[tokio::test]
    async fn test_scoped_cache_isolation() {
        let cache = Arc::new(InMemoryCache::new());

        let scope1 = ScopedCache::new(cache.clone(), "scope1");
        let scope2 = ScopedCache::new(cache.clone(), "scope2");

        scope1.set("key1", &"value1_scope1", None).await.unwrap();
        scope2.set("key1", &"value1_scope2", None).await.unwrap();

        let val1: Option<String> = scope1.get("key1").await.unwrap();
        let val2: Option<String> = scope2.get("key1").await.unwrap();

        assert_eq!(val1, Some("value1_scope1".to_string()));
        assert_eq!(val2, Some("value1_scope2".to_string()));
    }

    #[tokio::test]
    async fn test_auto_cleanup() {
        let cache = Arc::new(InMemoryCache::new());

        {
            let scoped = ScopedCache::new(cache.clone(), "temp_scope");
            scoped.set("temp_key", &"temp_value", None).await.unwrap();

            let val: Option<String> = scoped.get("temp_key").await.unwrap();
            assert_eq!(val, Some("temp_value".to_string()));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        let result: Option<String> = cache.get("temp_scope:temp_key").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_manual_cleanup() {
        let cache = Arc::new(InMemoryCache::new());
        let scoped = ScopedCache::new(cache.clone(), "manual_scope")
            .disable_auto_cleanup();

        scoped.set("key1", &"value1", None).await.unwrap();
        scoped.set("key2", &"value2", None).await.unwrap();

        scoped.cleanup().await.unwrap();

        let val1: Option<String> = scoped.get("key1").await.unwrap();
        let val2: Option<String> = scoped.get("key2").await.unwrap();

        assert_eq!(val1, None);
        assert_eq!(val2, None);
    }
    */
}
