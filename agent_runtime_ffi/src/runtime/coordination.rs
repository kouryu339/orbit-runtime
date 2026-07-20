use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::watch;
use tokio::time::sleep;

use super::RuntimeError;

pub(crate) const LOCAL_LEASE_TTL_MS: u64 = 30_000;
pub(crate) const LOCAL_LEASE_RENEW_INTERVAL_MS: u64 = 10_000;

#[allow(dead_code)]
#[async_trait]
pub(crate) trait RuntimeStateStore: Send + Sync {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>, RuntimeError>;
    async fn put_json(
        &self,
        key: &str,
        value: serde_json::Value,
        ttl_ms: Option<u64>,
    ) -> Result<(), RuntimeError>;
    async fn delete(&self, key: &str) -> Result<(), RuntimeError>;
}

#[derive(Default)]
pub(crate) struct LocalRuntimeStateStore {
    values: StdMutex<HashMap<String, serde_json::Value>>,
}

#[async_trait]
impl RuntimeStateStore for LocalRuntimeStateStore {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>, RuntimeError> {
        self.values
            .lock()
            .map_err(|_| RuntimeError::Internal("local state store mutex poisoned".to_string()))
            .map(|values| values.get(key).cloned())
    }

    async fn put_json(
        &self,
        key: &str,
        value: serde_json::Value,
        _ttl_ms: Option<u64>,
    ) -> Result<(), RuntimeError> {
        self.values
            .lock()
            .map_err(|_| RuntimeError::Internal("local state store mutex poisoned".to_string()))?
            .insert(key.to_string(), value);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), RuntimeError> {
        self.values
            .lock()
            .map_err(|_| RuntimeError::Internal("local state store mutex poisoned".to_string()))?
            .remove(key);
        Ok(())
    }
}

#[allow(dead_code)]
#[async_trait]
pub(crate) trait RuntimeCoordinationBackend: Send + Sync {
    async fn acquire_lease(
        &self,
        key: &str,
        owner: &str,
        ttl_ms: u64,
    ) -> Result<bool, RuntimeError>;
    async fn renew_lease(&self, key: &str, owner: &str, ttl_ms: u64) -> Result<bool, RuntimeError>;
    async fn release_lease(&self, key: &str, owner: &str) -> Result<(), RuntimeError>;
}

#[async_trait]
pub(crate) trait RuntimeSequenceBackend: Send + Sync {
    async fn next_global_event_seq(&self) -> Result<u64, RuntimeError>;
    async fn next_conversation_event_seq(&self, conversation_id: &str)
        -> Result<u64, RuntimeError>;
}

#[derive(Default)]
pub(crate) struct LocalRuntimeSequenceBackend {
    global_event_seq: StdMutex<u64>,
    conversation_event_seq: StdMutex<HashMap<String, u64>>,
}

#[async_trait]
impl RuntimeSequenceBackend for LocalRuntimeSequenceBackend {
    async fn next_global_event_seq(&self) -> Result<u64, RuntimeError> {
        let mut seq = self
            .global_event_seq
            .lock()
            .map_err(|_| RuntimeError::Internal("local event seq mutex poisoned".to_string()))?;
        *seq += 1;
        Ok(*seq)
    }

    async fn next_conversation_event_seq(
        &self,
        conversation_id: &str,
    ) -> Result<u64, RuntimeError> {
        let mut seqs = self.conversation_event_seq.lock().map_err(|_| {
            RuntimeError::Internal("local conversation event seq mutex poisoned".to_string())
        })?;
        let seq = seqs.entry(conversation_id.to_string()).or_insert(0);
        *seq += 1;
        Ok(*seq)
    }
}

#[derive(Default)]
pub(crate) struct LocalRuntimeCoordinationBackend {
    leases: StdMutex<HashMap<String, (String, Instant)>>,
}

#[async_trait]
impl RuntimeCoordinationBackend for LocalRuntimeCoordinationBackend {
    async fn acquire_lease(
        &self,
        key: &str,
        owner: &str,
        ttl_ms: u64,
    ) -> Result<bool, RuntimeError> {
        let now = Instant::now();
        let expires_at = now + Duration::from_millis(ttl_ms.max(1));
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| RuntimeError::Internal("local lease mutex poisoned".to_string()))?;
        if let Some((current_owner, current_expires_at)) = leases.get(key) {
            if current_owner != owner && *current_expires_at > now {
                return Ok(false);
            }
        }
        leases.insert(key.to_string(), (owner.to_string(), expires_at));
        Ok(true)
    }

    async fn renew_lease(&self, key: &str, owner: &str, ttl_ms: u64) -> Result<bool, RuntimeError> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| RuntimeError::Internal("local lease mutex poisoned".to_string()))?;
        match leases.get_mut(key) {
            Some((current_owner, expires_at)) if current_owner == owner => {
                *expires_at = Instant::now() + Duration::from_millis(ttl_ms.max(1));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn release_lease(&self, key: &str, owner: &str) -> Result<(), RuntimeError> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| RuntimeError::Internal("local lease mutex poisoned".to_string()))?;
        if leases
            .get(key)
            .map(|(current_owner, _)| current_owner == owner)
            .unwrap_or(false)
        {
            leases.remove(key);
        }
        Ok(())
    }
}

pub(crate) fn lease_renew_interval(lock_ttl_ms: u64, renew_interval_ms: u64) -> Duration {
    let interval_ms = if renew_interval_ms > 0 {
        renew_interval_ms
    } else {
        (lock_ttl_ms / 3).max(1)
    };
    Duration::from_millis(interval_ms.min(lock_ttl_ms.max(1)).max(1))
}

pub(crate) async fn run_lease_renewer(
    backend: Arc<dyn RuntimeCoordinationBackend>,
    key: String,
    owner: String,
    ttl_ms: u64,
    interval: Duration,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
            _ = sleep(interval) => {
                match backend.renew_lease(&key, &owner, ttl_ms).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!("conversation turn lease renewal lost ownership");
                        return;
                    }
                    Err(error) => {
                        tracing::warn!("conversation turn lease renewal failed: {}", error);
                        return;
                    }
                }
            }
        }
    }
}
