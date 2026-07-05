use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::role_contract::AdversaryPersona;
use super::tools::AdversaryConclusionReport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdversaryPairStatus {
    Starting,
    Running,
    Concluding,
    Concluded,
    Destroyed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryPair {
    pub pair_id: String,
    pub adversary_conversation_id: String,
    pub target_conversation_id: String,
    pub persona: AdversaryPersona,
    pub status: AdversaryPairStatus,
}

#[derive(Default)]
pub struct AgentTestController {
    pairs: Arc<RwLock<HashMap<String, AdversaryPair>>>,
    terminal_pair_ids: Arc<RwLock<HashSet<String>>>,
    reports: Arc<RwLock<HashMap<String, AdversaryConclusionReport>>>,
}

impl AgentTestController {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Vec<AdversaryPair> {
        self.pairs.read().await.values().cloned().collect()
    }

    pub async fn pair(&self, pair_id: &str) -> Option<AdversaryPair> {
        self.pairs.read().await.get(pair_id).cloned()
    }

    pub async fn pair_for_adversary_conversation(
        &self,
        conversation_id: &str,
    ) -> Option<AdversaryPair> {
        self.pairs
            .read()
            .await
            .values()
            .find(|pair| pair.adversary_conversation_id == conversation_id)
            .cloned()
    }

    pub async fn report(&self, pair_id: &str) -> Option<AdversaryConclusionReport> {
        self.reports.read().await.get(pair_id).cloned()
    }

    pub async fn reserve_pair(&self, pair: AdversaryPair) -> Result<(), String> {
        let pair_id = pair.pair_id.trim();
        if pair_id.is_empty() {
            return Err("pair_id must not be empty".to_string());
        }
        if self.terminal_pair_ids.read().await.contains(pair_id) {
            return Err(format!(
                "pair '{}' is terminal and cannot be rebuilt",
                pair_id
            ));
        }
        let mut pairs = self.pairs.write().await;
        if pairs.contains_key(pair_id) {
            return Err(format!("pair '{}' already exists", pair_id));
        }
        pairs.insert(pair_id.to_string(), pair);
        Ok(())
    }

    pub async fn activate_pair(&self, pair_id: &str) -> Result<(), String> {
        let mut pairs = self.pairs.write().await;
        let pair = pairs
            .get_mut(pair_id)
            .ok_or_else(|| format!("pair '{}' was not reserved", pair_id))?;
        if pair.status != AdversaryPairStatus::Starting {
            return Err(format!(
                "pair '{}' cannot activate from {:?}",
                pair_id, pair.status
            ));
        }
        pair.status = AdversaryPairStatus::Running;
        Ok(())
    }

    pub async fn release_pair_reservation(&self, pair_id: &str) {
        self.pairs.write().await.remove(pair_id);
    }

    pub async fn mark_terminal(
        &self,
        pair_id: &str,
        status: AdversaryPairStatus,
    ) -> Result<(), String> {
        if !matches!(
            status,
            AdversaryPairStatus::Concluded | AdversaryPairStatus::Destroyed
        ) {
            return Err("terminal pair status must be concluded or destroyed".to_string());
        }
        let mut pairs = self.pairs.write().await;
        let pair = pairs
            .get_mut(pair_id)
            .ok_or_else(|| format!("pair '{}' was not found", pair_id))?;
        pair.status = status;
        drop(pairs);
        self.terminal_pair_ids
            .write()
            .await
            .insert(pair_id.to_string());
        Ok(())
    }

    pub async fn begin_conclusion(
        &self,
        adversary_conversation_id: &str,
    ) -> Result<AdversaryPair, String> {
        let mut pairs = self.pairs.write().await;
        let pair = pairs
            .values_mut()
            .find(|pair| pair.adversary_conversation_id == adversary_conversation_id)
            .ok_or_else(|| {
                format!(
                    "adversary conversation '{}' is not attached to a pair",
                    adversary_conversation_id
                )
            })?;
        if pair.status != AdversaryPairStatus::Running {
            return Err(format!(
                "pair '{}' cannot conclude from {:?}",
                pair.pair_id, pair.status
            ));
        }
        pair.status = AdversaryPairStatus::Concluding;
        let pair = pair.clone();
        drop(pairs);
        self.terminal_pair_ids
            .write()
            .await
            .insert(pair.pair_id.clone());
        Ok(pair)
    }

    pub async fn finish_conclusion(&self, report: AdversaryConclusionReport) -> Result<(), String> {
        let pair_id = report.pair_id.clone();
        let mut pairs = self.pairs.write().await;
        let pair = pairs
            .get_mut(&pair_id)
            .ok_or_else(|| format!("pair '{}' was not found", pair_id))?;
        if pair.status != AdversaryPairStatus::Concluding {
            return Err(format!(
                "pair '{}' cannot finish conclusion from {:?}",
                pair_id, pair.status
            ));
        }
        pair.status = AdversaryPairStatus::Concluded;
        drop(pairs);
        self.reports.write().await.insert(pair_id, report);
        Ok(())
    }

    pub async fn mark_failed(&self, pair_id: &str) -> Result<(), String> {
        let mut pairs = self.pairs.write().await;
        let pair = pairs
            .get_mut(pair_id)
            .ok_or_else(|| format!("pair '{}' was not found", pair_id))?;
        if matches!(
            pair.status,
            AdversaryPairStatus::Concluded | AdversaryPairStatus::Destroyed
        ) {
            return Err(format!(
                "pair '{}' cannot fail from {:?}",
                pair_id, pair.status
            ));
        }
        pair.status = AdversaryPairStatus::Failed;
        drop(pairs);
        self.terminal_pair_ids
            .write()
            .await
            .insert(pair_id.to_string());
        Ok(())
    }
}
