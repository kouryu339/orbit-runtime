//! Admission for reference changes and explicit Skill content replacement.
//! Readers never hold this gate while driving an Agent or calling an LLM.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use corework::cache::{Cache, CacheExt};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::context::keys;

pub static SKILL_UPDATE_GATE: RwLock<()> = RwLock::const_new(());
static CATALOG_ADMISSIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Prevent whole-catalog replacement while a host is creating its first Agent.
/// Acquire under read admission, after checking the configured resource root.
pub struct CatalogAdmission {
    _private: (),
}
impl CatalogAdmission {
    pub fn begin() -> Self {
        CATALOG_ADMISSIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self { _private: () }
    }
    pub fn active() -> bool {
        CATALOG_ADMISSIONS.load(std::sync::atomic::Ordering::SeqCst) != 0
    }
}
impl Drop for CatalogAdmission {
    fn drop(&mut self) {
        CATALOG_ADMISSIONS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
static CACHES: OnceLock<Mutex<Vec<Weak<dyn Cache>>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SkillDependency {
    pub conversation_id: String,
    pub agent_id: String,
    pub skill_names: Vec<String>,
}

/// Called inside read admission, before an instance can execute.
pub fn track(cache: &Arc<dyn Cache>) {
    let mut caches = CACHES.get_or_init(Default::default).lock().unwrap();
    caches.retain(|entry| entry.strong_count() > 0);
    let weak = Arc::downgrade(cache);
    if !caches.iter().any(|entry| entry.ptr_eq(&weak)) {
        caches.push(weak);
    }
}

/// Called only after the instance driver has stopped.
pub fn untrack(cache: &Arc<dyn Cache>) {
    if let Some(caches) = CACHES.get() {
        let weak = Arc::downgrade(cache);
        caches
            .lock()
            .unwrap()
            .retain(|entry| entry.strong_count() > 0 && !entry.ptr_eq(&weak));
    }
}

pub async fn references(cache: &Arc<dyn Cache>) -> crate::Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for key in [keys::MAIN_SKILLS, keys::IMPORTED_SKILLS] {
        names.extend(cache.get::<Vec<String>>(key).await?.unwrap_or_default());
    }
    let system = cache
        .get::<BTreeMap<String, String>>(keys::SYSTEM_SKILLS)
        .await?
        .unwrap_or_default();
    names.extend(system.values().cloned());
    names.insert(
        system
            .get("thinking")
            .cloned()
            .unwrap_or_else(|| "thinking".into()),
    );
    Ok(names)
}

/// Caller holds write admission for mutations, read admission for inspection.
/// The registry is process-wide, matching the existing shared SkillManager.
pub async fn dependencies(names: Option<&BTreeSet<String>>) -> crate::Result<Vec<SkillDependency>> {
    let caches = {
        let mut registry = CACHES.get_or_init(Default::default).lock().unwrap();
        registry.retain(|entry| entry.strong_count() > 0);
        registry
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    };
    let mut result = BTreeSet::new();
    for cache in caches {
        let mut used = references(&cache).await?;
        if let Some(names) = names {
            used.retain(|name| names.contains(name));
        }
        if used.is_empty() {
            continue;
        }
        let conversation_id = crate::agent::conversation_id_from_cache(&*cache)
            .await
            .unwrap_or_else(|| crate::ledger::DEFAULT_CONVERSATION_ID.into());
        let agent_id = crate::agent::source_id_from_cache(&*cache).await;
        result.insert(SkillDependency {
            conversation_id,
            agent_id,
            skill_names: used.into_iter().collect(),
        });
    }
    Ok(result.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::cache::InMemoryCache;

    #[tokio::test]
    async fn whole_catalog_replacement_cannot_overtake_first_agent_admission() {
        let admission = {
            let _gate = SKILL_UPDATE_GATE.read().await;
            CatalogAdmission::begin()
        };
        assert!(
            super::super::systems::replace_skill_manager(super::super::SkillManager::new("unused"))
                .await
                .unwrap_err()
                .to_string()
                .contains("Agent creation")
        );
        drop(admission);
    }

    #[tokio::test]
    async fn update_admission_waits_for_in_progress_reference_registration() {
        let admission = SKILL_UPDATE_GATE.read().await;
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        crate::agent::set_conversation_id_in_cache(&*cache, "usage-race-conversation")
            .await
            .unwrap();
        cache
            .set(
                crate::state_machine::agent_keys::AGENT_ID,
                &"background-worker".to_string(),
                None,
            )
            .await
            .unwrap();
        let names = BTreeSet::from(["usage-race-role".to_string()]);
        let (attempt, attempted) = tokio::sync::oneshot::channel();
        let mut writer = corework::platform::spawn(async move {
            let _ = attempt.send(());
            let _update = SKILL_UPDATE_GATE.write().await;
            dependencies(Some(&names)).await.unwrap()
        });
        attempted.await.unwrap();
        assert!(corework::platform::time::timeout(
            std::time::Duration::from_millis(20),
            &mut writer
        )
        .await
        .is_err());
        cache
            .set(
                keys::MAIN_SKILLS,
                &vec!["usage-race-role".to_string()],
                None,
            )
            .await
            .unwrap();
        track(&cache);
        drop(admission);
        let result = writer.await.unwrap();
        assert_eq!(
            result,
            vec![SkillDependency {
                conversation_id: "usage-race-conversation".into(),
                agent_id: "background-worker".into(),
                skill_names: vec!["usage-race-role".into()],
            }]
        );
        untrack(&cache);
    }

    #[tokio::test]
    async fn weak_tracking_and_explicit_shutdown_release_all_skill_categories() {
        let _admission = SKILL_UPDATE_GATE.read().await;
        let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
        cache
            .set(keys::MAIN_SKILLS, &vec!["usage-role".to_string()], None)
            .await
            .unwrap();
        cache
            .set(
                keys::IMPORTED_SKILLS,
                &vec!["usage-feature".to_string()],
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                keys::SYSTEM_SKILLS,
                &BTreeMap::from([("thinking".to_string(), "thinking-pro".to_string())]),
                None,
            )
            .await
            .unwrap();
        track(&cache);
        track(&cache);
        let names = BTreeSet::from([
            "usage-role".into(),
            "usage-feature".into(),
            "thinking-pro".into(),
        ]);
        let result = dependencies(Some(&names)).await.unwrap();
        let matching = result
            .iter()
            .filter(|entry| entry.skill_names.contains(&"usage-role".into()))
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(
            matching[0].skill_names,
            ["thinking-pro", "usage-feature", "usage-role"]
        );
        untrack(&cache);
        assert!(!dependencies(Some(&names))
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.skill_names.contains(&"usage-role".into())));
        track(&cache);
        drop(cache);
        assert!(!dependencies(Some(&names))
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.skill_names.contains(&"usage-role".into())));
    }
}
