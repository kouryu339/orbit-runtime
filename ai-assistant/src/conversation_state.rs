use std::collections::{BTreeMap, HashMap};

use corework::error::FrameworkError;
use tokio::sync::{watch, Mutex, RwLock};

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
    task_revision: watch::Sender<u64>,
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

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Assigned => "assigned",
            Self::Running => "running",
            Self::Reported => "reported",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
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
pub struct AgentTaskProgress {
    pub progress_id: String,
    pub stage_id: String,
    pub summary: String,
    #[serde(default)]
    pub result: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_stage: Option<String>,
    pub reported_at: String,
    pub task_revision: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskInputRequestStatus {
    Pending,
    Answered,
}

impl AgentTaskInputRequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Answered => "answered",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTaskInputRequest {
    pub request_id: String,
    pub requester_agent_id: String,
    pub question: String,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default)]
    pub blocking: bool,
    pub status: AgentTaskInputRequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTaskUpdate {
    pub update_id: String,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Vec<String>>,
    pub updated_by_agent_id: String,
    pub created_at: String,
    pub task_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTaskEntry {
    pub task_id: String,
    #[serde(default = "default_task_revision")]
    pub revision: u64,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<AgentTaskProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_requests: Vec<AgentTaskInputRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub updates: Vec<AgentTaskUpdate>,
    pub created_at: String,
    pub updated_at: String,
}

fn default_task_revision() -> u64 {
    1
}

impl ConversationState {
    pub fn new(
        conversation_id: impl Into<String>,
        request_headers: ConversationRequestHeaders,
        focus_agent_id: impl Into<String>,
    ) -> Self {
        let (task_revision, _) = watch::channel(0);
        Self {
            conversation_id: conversation_id.into(),
            request_headers: RwLock::new(request_headers),
            dynamic_snapshots: RwLock::new(HashMap::new()),
            task_board: RwLock::new(HashMap::new()),
            task_revision,
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
            revision: 1,
            title: title.into(),
            objective: objective.into(),
            acceptance,
            delegator_agent_id: delegator_agent_id.into(),
            delegator_agent_name: delegator_agent_name.into(),
            assignee_agent_id: None,
            assignee_agent_name: None,
            status: AgentTaskStatus::Pending,
            report: None,
            progress: Vec::new(),
            input_requests: Vec::new(),
            updates: Vec::new(),
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
        drop(board);
        self.bump_task_revision();
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
        entry.revision = entry.revision.saturating_add(1);
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok(entry)
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
        if entry.status != AgentTaskStatus::Running {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' must be running before a result can be reported",
                task_id
            )));
        }
        entry.report = Some(report);
        entry.status = AgentTaskStatus::Reported;
        entry.revision = entry.revision.saturating_add(1);
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok(entry)
    }

    pub async fn report_agent_task_progress(
        &self,
        task_id: &str,
        reporter_agent_id: &str,
        progress_id: impl Into<String>,
        stage_id: impl Into<String>,
        summary: impl Into<String>,
        result: serde_json::Value,
        artifacts: Vec<String>,
        next_stage: Option<String>,
    ) -> Result<(AgentTaskEntry, AgentTaskProgress), FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        if entry.assignee_agent_id.as_deref() != Some(reporter_agent_id) {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' cannot report progress for task '{}'",
                reporter_agent_id, task_id
            )));
        }
        if entry.status != AgentTaskStatus::Running {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' must be running before progress can be reported",
                task_id
            )));
        }
        let progress_id = progress_id.into();
        if entry
            .progress
            .iter()
            .any(|progress| progress.progress_id == progress_id)
        {
            return Err(FrameworkError::InvalidOperation(format!(
                "task progress '{}' already exists for task '{}'",
                progress_id, task_id
            )));
        }
        entry.revision = entry.revision.saturating_add(1);
        let progress = AgentTaskProgress {
            progress_id,
            stage_id: stage_id.into(),
            summary: summary.into(),
            result,
            artifacts,
            next_stage,
            reported_at: chrono::Local::now().to_rfc3339(),
            task_revision: entry.revision,
        };
        entry.progress.push(progress.clone());
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok((entry, progress))
    }

    pub async fn complete_agent_task(
        &self,
        task_id: &str,
        delegator_agent_id: &str,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        if entry.delegator_agent_id != delegator_agent_id {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' is not the delegator of task '{}'",
                delegator_agent_id, task_id
            )));
        }
        if entry.status != AgentTaskStatus::Reported {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' must be reported before completion",
                task_id
            )));
        }
        entry.status = if entry
            .report
            .as_ref()
            .is_some_and(|report| report.report_type == "failed")
        {
            AgentTaskStatus::Failed
        } else {
            AgentTaskStatus::Completed
        };
        entry.revision = entry.revision.saturating_add(1);
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok(entry)
    }

    pub async fn cancel_agent_task(
        &self,
        task_id: &str,
        delegator_agent_id: &str,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        if entry.delegator_agent_id != delegator_agent_id {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' is not the delegator of task '{}'",
                delegator_agent_id, task_id
            )));
        }
        if entry.status.is_terminal() {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' is already terminal",
                task_id
            )));
        }
        entry.status = AgentTaskStatus::Canceled;
        entry.revision = entry.revision.saturating_add(1);
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok(entry)
    }

    pub async fn request_agent_task_input(
        &self,
        task_id: &str,
        requester_agent_id: &str,
        request_id: impl Into<String>,
        question: impl Into<String>,
        required_fields: Vec<String>,
        blocking: bool,
    ) -> Result<(AgentTaskEntry, AgentTaskInputRequest), FrameworkError> {
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
        if entry.assignee_agent_id.as_deref() != Some(requester_agent_id) {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' is not the current assignee of task '{}'",
                requester_agent_id, task_id
            )));
        }
        let request_id = request_id.into();
        if entry
            .input_requests
            .iter()
            .any(|request| request.request_id == request_id)
        {
            return Err(FrameworkError::InvalidOperation(format!(
                "task input request '{}' already exists",
                request_id
            )));
        }
        let request = AgentTaskInputRequest {
            request_id,
            requester_agent_id: requester_agent_id.to_string(),
            question: question.into(),
            required_fields,
            blocking,
            status: AgentTaskInputRequestStatus::Pending,
            answer: None,
            requested_at: chrono::Local::now().to_rfc3339(),
            answered_at: None,
            delivery: None,
        };
        entry.input_requests.push(request.clone());
        entry.revision = entry.revision.saturating_add(1);
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok((entry, request))
    }

    pub async fn set_agent_task_input_delivery(
        &self,
        task_id: &str,
        request_id: &str,
        delivery: impl Into<String>,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        let request = entry
            .input_requests
            .iter_mut()
            .find(|request| request.request_id == request_id)
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "task input request '{}' not found for task '{}'",
                    request_id, task_id
                ))
            })?;
        request.delivery = Some(delivery.into());
        entry.updated_at = chrono::Local::now().to_rfc3339();
        Ok(entry.clone())
    }

    pub async fn respond_agent_task_input(
        &self,
        task_id: &str,
        request_id: &str,
        responder_agent_id: &str,
        answer: impl Into<String>,
    ) -> Result<(AgentTaskEntry, AgentTaskInputRequest), FrameworkError> {
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
        if entry.delegator_agent_id != responder_agent_id {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' is not the delegator of task '{}'",
                responder_agent_id, task_id
            )));
        }
        let request = entry
            .input_requests
            .iter_mut()
            .find(|request| request.request_id == request_id)
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "task input request '{}' not found for task '{}'",
                    request_id, task_id
                ))
            })?;
        if entry.assignee_agent_id.as_deref() != Some(request.requester_agent_id.as_str()) {
            return Err(FrameworkError::InvalidOperation(format!(
                "task '{}' assignee changed after input request '{}'",
                task_id, request_id
            )));
        }
        if request.status != AgentTaskInputRequestStatus::Pending {
            return Err(FrameworkError::InvalidOperation(format!(
                "task input request '{}' is already answered",
                request_id
            )));
        }
        request.answer = Some(answer.into());
        request.status = AgentTaskInputRequestStatus::Answered;
        request.answered_at = Some(chrono::Local::now().to_rfc3339());
        let request = request.clone();
        entry.revision = entry.revision.saturating_add(1);
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok((entry, request))
    }

    pub async fn update_agent_task(
        &self,
        task_id: &str,
        updater_agent_id: &str,
        update_id: impl Into<String>,
        instruction: impl Into<String>,
        objective: Option<String>,
        acceptance: Option<Vec<String>>,
    ) -> Result<(AgentTaskEntry, AgentTaskUpdate), FrameworkError> {
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
        if entry.delegator_agent_id != updater_agent_id {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent '{}' is not the delegator of task '{}'",
                updater_agent_id, task_id
            )));
        }
        if entry.assignee_agent_id.is_none() {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task '{}' has no assignee",
                task_id
            )));
        }
        let update_id = update_id.into();
        if entry
            .updates
            .iter()
            .any(|update| update.update_id == update_id)
        {
            return Err(FrameworkError::InvalidOperation(format!(
                "agent task update '{}' already exists",
                update_id
            )));
        }
        if entry.status == AgentTaskStatus::Reported {
            entry.status = AgentTaskStatus::Running;
        }
        if let Some(new_objective) = objective.as_ref() {
            entry.objective = new_objective.clone();
        }
        if let Some(new_acceptance) = acceptance.as_ref() {
            entry.acceptance = new_acceptance.clone();
        }
        entry.revision = entry.revision.saturating_add(1);
        let update = AgentTaskUpdate {
            update_id,
            instruction: instruction.into(),
            objective,
            acceptance,
            updated_by_agent_id: updater_agent_id.to_string(),
            created_at: chrono::Local::now().to_rfc3339(),
            task_revision: entry.revision,
            delivery: None,
        };
        entry.updates.push(update.clone());
        entry.updated_at = chrono::Local::now().to_rfc3339();
        let entry = entry.clone();
        drop(board);
        self.bump_task_revision();
        Ok((entry, update))
    }

    pub async fn set_agent_task_update_delivery(
        &self,
        task_id: &str,
        update_id: &str,
        delivery: impl Into<String>,
    ) -> Result<AgentTaskEntry, FrameworkError> {
        let mut board = self.task_board.write().await;
        let entry = board.get_mut(task_id).ok_or_else(|| {
            FrameworkError::InvalidOperation(format!("agent task '{}' not found", task_id))
        })?;
        let update = entry
            .updates
            .iter_mut()
            .find(|update| update.update_id == update_id)
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "agent task update '{}' not found for task '{}'",
                    update_id, task_id
                ))
            })?;
        update.delivery = Some(delivery.into());
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
        self.bump_task_revision();
        entry
    }

    pub fn subscribe_agent_tasks(&self) -> watch::Receiver<u64> {
        self.task_revision.subscribe()
    }

    fn bump_task_revision(&self) {
        self.task_revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
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
