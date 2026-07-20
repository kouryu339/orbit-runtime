use std::collections::{BTreeMap, HashMap};

use corework::error::FrameworkError;
use tokio::sync::{Mutex, RwLock};

use crate::ledger::{LedgerRecord, LedgerRole};

#[derive(Debug, Clone)]
pub struct LedgerReadOptions {
    pub agent_id: Option<String>,
    pub after_latest_summary: bool,
    pub limit: usize,
}

impl Default for LedgerReadOptions {
    fn default() -> Self {
        Self {
            agent_id: None,
            after_latest_summary: false,
            limit: usize::MAX,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConversationRequestHeaders {
    pub headers: BTreeMap<String, String>,
    pub allow_insecure: bool,
}

pub struct ConversationState {
    conversation_id: String,
    request_headers: RwLock<ConversationRequestHeaders>,
    dynamic_snapshots: RwLock<HashMap<String, HashMap<String, String>>>,
    task_board: RwLock<HashMap<String, AgentTaskEntry>>,
    ledger: RwLock<Vec<LedgerRecord>>,
    focus_agent_id: RwLock<String>,
    focus_transition_lock: Mutex<()>,
    append_lock: Mutex<()>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Pending,
    Assigned,
    Running,
    Reported,
    Completed,
    Failed,
    Canceled,
}

impl AgentTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTaskReport {
    pub report_type: String,
    pub summary: String,
    #[serde(default)]
    pub result: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<String>,
    pub reported_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTaskEntry {
    pub task_id: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    pub delegator_agent_id: String,
    pub delegator_agent_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_agent_name: Option<String>,
    pub status: AgentTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<AgentTaskReport>,
    pub created_at: String,
    pub updated_at: String,
}

impl ConversationState {
    pub fn new(
        conversation_id: impl Into<String>,
        request_headers: ConversationRequestHeaders,
        focus_agent_id: impl Into<String>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            request_headers: RwLock::new(request_headers),
            dynamic_snapshots: RwLock::new(HashMap::new()),
            task_board: RwLock::new(HashMap::new()),
            ledger: RwLock::new(Vec::new()),
            focus_agent_id: RwLock::new(focus_agent_id.into()),
            focus_transition_lock: Mutex::new(()),
            append_lock: Mutex::new(()),
        }
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub async fn request_headers(&self) -> ConversationRequestHeaders {
        self.request_headers.read().await.clone()
    }

    pub async fn set_request_headers(
        &self,
        headers: BTreeMap<String, String>,
        allow_insecure: bool,
    ) {
        *self.request_headers.write().await = ConversationRequestHeaders {
            headers,
            allow_insecure,
        };
    }

    pub async fn set_dynamic_snapshot_field(
        &self,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> Option<String> {
        self.dynamic_snapshots
            .write()
            .await
            .entry(agent_id.to_string())
            .or_default()
            .insert(field_name.to_string(), text.to_string())
    }

    pub async fn dynamic_snapshots(&self, agent_id: &str) -> HashMap<String, String> {
        self.dynamic_snapshots
            .read()
            .await
            .get(agent_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn create_agent_task(
        &self,
        task_id: impl Into<String>,
        title: impl Into<String>,
        objective: impl Into<String>,
        acceptance: Vec<String>,
        delegator_agent_id: impl Into<String>,
        delegator_agent_name: impl Into<String>,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let now = chrono::Local::now().to_rfc3339();
        let entry = AgentTaskEntry {
            task_id: task_id.into(),
            title: title.into(),
            objective: objective.into(),
            acceptance,
            delegator_agent_id: delegator_agent_id.into(),
            delegator_agent_name: delegator_agent_name.into(),
            assignee_agent_id: None,
            assignee_agent_name: None,
            status: AgentTaskStatus::Pending,
            report: None,
            created_at: now.clone(),
            updated_at: now,
        };
        let mut board = self.task_board.write().await;
        if board.contains_key(&entry.task_id) {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' already exists",
                entry.task_id
            )));
        }
        board.insert(entry.task_id.clone(), entry.clone());
        Ok(entry)
    }

    pub async fn assign_agent_task(
        &self,
        task_id: &str,
        assignee_agent_id: impl Into<String>,
        assignee_agent_name: impl Into<String>,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        if entry.status.is_terminal() {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' is already terminal",
                task_id
            )));
        }
        entry.assignee_agent_id = Some(assignee_agent_id.into());
        entry.assignee_agent_name = Some(assignee_agent_name.into());
        entry.status = AgentTaskStatus::Running;
        entry.updated_at = chrono::Local::now().to_rfc3339();
        Ok(entry.clone())
    }

    pub async fn report_agent_task(
        &self,
        task_id: &str,
        reporter_agent_id: &str,
        report: AgentTaskReport,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        if entry.assignee_agent_id.as_deref() != Some(reporter_agent_id) {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' cannot report task '{}'",
                reporter_agent_id, task_id
            )));
        }
        if entry.status.is_terminal() {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' is already terminal",
                task_id
            )));
        }
        let terminal_status = match report.report_type.as_str() {
            "failed" => AgentTaskStatus::Failed,
            "canceled" => AgentTaskStatus::Canceled,
            _ => AgentTaskStatus::Completed,
        };
        entry.report = Some(report);
        entry.status = terminal_status;
        entry.updated_at = chrono::Local::now().to_rfc3339();
        Ok(entry.clone())
    }

    pub async fn agent_task(&self, task_id: &str) -> Option<AgentTaskEntry> {
        self.task_board.read().await.get(task_id).cloned()
    }

    pub async fn upsert_agent_task(&self, entry: AgentTaskEntry) -> AgentTaskEntry {
        self.task_board
            .write()
            .await
            .insert(entry.task_id.clone(), entry.clone());
        entry
    }

    pub async fn agent_tasks(&self) -> Vec<AgentTaskEntry> {
        let mut tasks: Vec<_> = self.task_board.read().await.values().cloned().collect();
        tasks.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.task_id.cmp(&b.task_id))
        });
        tasks
    }

    pub async fn append(&self, mut record: LedgerRecord) -> Result<LedgerRecord, FrameworkError> {
        let _guard = self.append_lock.lock().await;
        let mut ledger = self.ledger.write().await;
        record.conversation_id = self.conversation_id.clone();
        record.record_id = ledger
            .last()
            .map(|record| record.record_id + 1)
            .unwrap_or(1);
        ledger.push(record.clone());
        Ok(record)
    }

    pub async fn list_recent(&self, opts: LedgerReadOptions) -> Vec<LedgerRecord> {
        let mut records = self.ledger.read().await.clone();
        if let Some(agent_id) = opts.agent_id {
            records.retain(|record| record.agent_id == agent_id);
        }
        if opts.after_latest_summary {
            if let Some(index) = records
                .iter()
                .rposition(|record| record.role == LedgerRole::Summary)
            {
                records = records[index..].to_vec();
            }
        }
        if opts.limit != usize::MAX && records.len() > opts.limit {
            records = records[records.len() - opts.limit..].to_vec();
        }
        records
    }

    pub async fn replace(&self, mut records: Vec<LedgerRecord>) {
        let _guard = self.append_lock.lock().await;
        records.sort_by_key(|record| record.record_id);
        for (index, record) in records.iter_mut().enumerate() {
            record.conversation_id = self.conversation_id.clone();
            record.record_id = index as u64 + 1;
        }
        *self.ledger.write().await = records;
    }

    pub async fn focus(&self) -> String {
        self.focus_agent_id.read().await.clone()
    }

    pub(crate) async fn set_focus(&self, agent_id: impl Into<String>) {
        *self.focus_agent_id.write().await = agent_id.into();
    }

    pub(crate) async fn lock_focus_transition(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.focus_transition_lock.lock().await
    }
}
