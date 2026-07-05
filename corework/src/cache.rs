//! Cache traits and built-in cache helpers.
use crate::error::{FrameworkError, Result};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

pub trait CacheValue: Serialize + DeserializeOwned + Clone + Send + Sync + Debug {}
impl<T: Serialize + DeserializeOwned + Clone + Send + Sync + Debug> CacheValue for T {}

fn expires_at_from_ttl(ttl: Duration) -> Result<chrono::DateTime<Utc>> {
    let ttl = chrono::Duration::from_std(ttl)
        .map_err(|_| FrameworkError::InvalidOperation("cache ttl is out of range".to_string()))?;
    Ok(Utc::now() + ttl)
}

#[async_trait]
pub trait Cache: Send + Sync {
    async fn get_raw(&self, key: &str) -> Result<Option<serde_json::Value>>;

    async fn set_raw(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<()>;

    async fn delete(&self, key: &str) -> Result<()>;

    async fn exists(&self, key: &str) -> Result<bool>;
    /// Get multiple raw values in one call.
    async fn mget_raw(&self, keys: &[String]) -> Result<Vec<Option<serde_json::Value>>>;

    async fn mset_raw(
        &self,
        items: &[(String, serde_json::Value)],
        ttl: Option<Duration>,
    ) -> Result<()>;
    /// Dump all raw values when the backend supports it.
    async fn dump_raw(&self) -> Result<HashMap<String, serde_json::Value>> {
        Err(FrameworkError::InvalidOperation(
            "cache dump is not supported by this cache implementation".to_string(),
        ))
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64>;

    async fn expire(&self, key: &str, ttl: Duration) -> Result<()>;

    async fn flush(&self) -> Result<()>;
}

#[async_trait]
pub trait CacheExt: Cache {
    async fn get<V: CacheValue>(&self, key: &str) -> Result<Option<V>> {
        if let Some(value) = self.get_raw(key).await? {
            Ok(Some(serde_json::from_value(value)?))
        } else {
            Ok(None)
        }
    }

    async fn set<V: CacheValue>(&self, key: &str, value: &V, ttl: Option<Duration>) -> Result<()> {
        let json_value = serde_json::to_value(value)?;
        self.set_raw(key, json_value, ttl).await
    }

    async fn mget<V: CacheValue>(&self, keys: &[String]) -> Result<Vec<Option<V>>> {
        let values = self.mget_raw(keys).await?;
        values
            .into_iter()
            .map(|opt_val| {
                opt_val
                    .map(|v| serde_json::from_value(v))
                    .transpose()
                    .map_err(crate::error::FrameworkError::SerializationError)
            })
            .collect()
    }

    async fn mset<V: CacheValue>(
        &self,
        items: &[(String, V)],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let json_items: Result<Vec<_>> = items
            .iter()
            .map(|(k, v)| {
                serde_json::to_value(v)
                    .map(|json| (k.clone(), json))
                    .map_err(crate::error::FrameworkError::SerializationError)
            })
            .collect();
        self.mset_raw(&json_items?, ttl).await
    }
    /// Read a nested field from an object stored in the cache.
    ///
    /// # Example
    /// ```ignore
    /// let sub_id: String = cache.get_field("sub_question", "sub_id").await?.unwrap();
    /// let question_type: String = cache.get_field("sub_question", "question_type").await?.unwrap();
    /// ```
    async fn get_field<V: CacheValue>(&self, key: &str, field_path: &str) -> Result<Option<V>> {
        use crate::workflow::core::DataValue;

        if let Some(json_value) = self.get_raw(key).await? {
            let data_value = DataValue::new("Any", json_value);

            if let Some(field_value) = data_value.get_field_path(field_path) {
                return Ok(Some(serde_json::from_value(field_value.value.clone())?));
            }
        }

        Ok(None)
    }
    /// Set a nested field inside an object stored in the cache.
    async fn set_field<V: CacheValue>(&self, key: &str, field_path: &str, value: &V) -> Result<()> {
        use crate::workflow::core::DataValue;

        let mut data_value = if let Some(json_value) = self.get_raw(key).await? {
            DataValue::new("Object", json_value)
        } else {
            DataValue::from_json_object(serde_json::json!({}))?
        };

        let field_data_value = DataValue::new("Any", serde_json::to_value(value)?);
        data_value.set_field_path(field_path, field_data_value)?;

        self.set_raw(key, data_value.value.clone(), None).await
    }
}

impl<T: Cache + ?Sized> CacheExt for T {}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub default_ttl: Option<Duration>,
    pub key_prefix: String,
    pub enable_compression: bool,
    pub max_value_size: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            default_ttl: Some(Duration::from_secs(3600)),
            key_prefix: String::new(),
            enable_compression: false,
            max_value_size: 1024 * 1024, // 1MB
        }
    }
}

#[derive(Debug, Clone)]
pub enum CacheBackendConfig {
    Memory(CacheConfig),
}

impl Default for CacheBackendConfig {
    fn default() -> Self {
        Self::Memory(CacheConfig::default())
    }
}

pub fn create_cache_backend(config: CacheBackendConfig) -> Result<Arc<dyn Cache>> {
    match config {
        CacheBackendConfig::Memory(config) => Ok(Arc::new(InMemoryCache::with_config(config))),
    }
}

use chrono::{DateTime, Utc};
use dashmap::DashMap;

#[derive(Clone)]
struct CacheEntry {
    value: serde_json::Value,
    expires_at: Option<DateTime<Utc>>,
}

pub struct InMemoryCache {
    store: Arc<DashMap<String, CacheEntry>>,
    config: CacheConfig,
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            store: Arc::new(DashMap::new()),
            config: CacheConfig::default(),
        }
    }

    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            store: Arc::new(DashMap::new()),
            config,
        }
    }

    fn is_expired(&self, entry: &CacheEntry) -> bool {
        if let Some(expires_at) = entry.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }

    fn make_key(&self, key: &str) -> String {
        if self.config.key_prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}:{}", self.config.key_prefix, key)
        }
    }
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Cache for InMemoryCache {
    async fn get_raw(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let full_key = self.make_key(key);

        if let Some(entry) = self.store.get(&full_key) {
            if self.is_expired(&entry) {
                drop(entry);
                self.store.remove(&full_key);
                return Ok(None);
            }
            Ok(Some(entry.value.clone()))
        } else {
            Ok(None)
        }
    }

    async fn set_raw(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let full_key = self.make_key(key);
        let ttl = ttl.or(self.config.default_ttl);

        let expires_at = match ttl {
            Some(ttl) => Some(expires_at_from_ttl(ttl)?),
            None => None,
        };

        self.store
            .insert(full_key, CacheEntry { value, expires_at });

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let full_key = self.make_key(key);
        self.store.remove(&full_key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let full_key = self.make_key(key);

        if let Some(entry) = self.store.get(&full_key) {
            if self.is_expired(&entry) {
                drop(entry);
                self.store.remove(&full_key);
                return Ok(false);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn mget_raw(&self, keys: &[String]) -> Result<Vec<Option<serde_json::Value>>> {
        let mut results = Vec::with_capacity(keys.len());

        for key in keys {
            results.push(self.get_raw(key).await?);
        }

        Ok(results)
    }

    async fn mset_raw(
        &self,
        items: &[(String, serde_json::Value)],
        ttl: Option<Duration>,
    ) -> Result<()> {
        for (key, value) in items {
            self.set_raw(key, value.clone(), ttl).await?;
        }
        Ok(())
    }

    async fn dump_raw(&self) -> Result<HashMap<String, serde_json::Value>> {
        let prefix = if self.config.key_prefix.is_empty() {
            None
        } else {
            Some(format!("{}:", self.config.key_prefix))
        };
        let mut snapshot = HashMap::new();

        for entry in self.store.iter() {
            if self.is_expired(entry.value()) {
                continue;
            }

            let key = entry.key();
            let logical_key = match prefix.as_deref() {
                Some(prefix) => key
                    .strip_prefix(prefix)
                    .map(str::to_string)
                    .unwrap_or_else(|| key.clone()),
                None => key.clone(),
            };
            snapshot.insert(logical_key, entry.value().value.clone());
        }

        Ok(snapshot)
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let full_key = self.make_key(key);

        let mut entry = self.store.entry(full_key.clone()).or_insert(CacheEntry {
            value: serde_json::Value::Number(0.into()),
            expires_at: match self.config.default_ttl {
                Some(ttl) => Some(expires_at_from_ttl(ttl)?),
                None => None,
            },
        });

        let current = entry.value.as_i64().unwrap_or(0);
        let new_value = current + delta;
        entry.value = serde_json::Value::Number(new_value.into());

        Ok(new_value)
    }

    async fn expire(&self, key: &str, ttl: Duration) -> Result<()> {
        let full_key = self.make_key(key);

        if let Some(mut entry) = self.store.get_mut(&full_key) {
            entry.expires_at = Some(expires_at_from_ttl(ttl)?);
        }

        Ok(())
    }

    async fn flush(&self) -> Result<()> {
        self.store.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_dump_raw_returns_unexpired_logical_keys() {
        let cache = InMemoryCache::with_config(CacheConfig {
            key_prefix: "scope".to_string(),
            default_ttl: None,
            enable_compression: false,
            max_value_size: 1024,
        });

        cache
            .set_raw("visible", serde_json::json!({"ok": true}), None)
            .await
            .unwrap();
        cache
            .set_raw(
                "expired",
                serde_json::json!("old"),
                Some(Duration::from_millis(1)),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;

        let snapshot = cache.dump_raw().await.unwrap();

        assert_eq!(
            snapshot.get("visible"),
            Some(&serde_json::json!({"ok": true}))
        );
        assert!(!snapshot.contains_key("scope:visible"));
        assert!(!snapshot.contains_key("expired"));
    }
    /*
    use super::*;

    #[tokio::test]
    async fn test_basic_operations() {
        let cache = InMemoryCache::new();

        // Set and get
        cache.set("key1", &"value1", None).await.unwrap();
        let result: Option<String> = cache.get("key1").await.unwrap();
        assert_eq!(result, Some("value1".to_string()));

        // Exists
        assert!(cache.exists("key1").await.unwrap());
        assert!(!cache.exists("nonexistent").await.unwrap());

    #[tokio::test]
    async fn test_expiration() {
        let cache = InMemoryCache::new();

        // Set with 100ms TTL
        cache.set("temp", &"value", Some(Duration::from_millis(100))).await.unwrap();
        assert!(cache.exists("temp").await.unwrap());

        tokio::time::sleep(Duration::from_millis(150)).await;

        let result: Option<String> = cache.get("temp").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_incr() {
        let cache = InMemoryCache::new();

        let count = cache.incr("counter", 1).await.unwrap();
        assert_eq!(count, 1);

        let count = cache.incr("counter", 5).await.unwrap();
        assert_eq!(count, 6);

        let count = cache.incr("counter", -2).await.unwrap();
        assert_eq!(count, 4);
    }
    */
}
