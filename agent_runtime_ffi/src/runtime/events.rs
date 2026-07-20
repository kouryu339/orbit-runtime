use super::*;

pub(super) async fn install_event_forwarders(
    event_bus: Arc<dyn EventBus>,
    event_sender: Arc<Mutex<Option<std_mpsc::Sender<String>>>>,
    projector: Arc<HostEventProjector>,
    event_log: Arc<StdMutex<VecDeque<Value>>>,
    agent_test_event_tx: mpsc::UnboundedSender<Value>,
) -> Result<(), RuntimeError> {
    for event_type in forwarded_runtime_event_types() {
        let handler: Arc<dyn EventHandler> = Arc::new(RuntimeEventForwarder {
            event_sender: Arc::clone(&event_sender),
            projector: Arc::clone(&projector),
            event_log: Arc::clone(&event_log),
            agent_test_event_tx: agent_test_event_tx.clone(),
        });
        event_bus
            .subscribe(event_type.to_string(), handler)
            .await
            .map_err(|e| RuntimeError::Internal(e.to_string()))?;
    }

    Ok(())
}

fn forwarded_runtime_event_types() -> [&'static str; 7] {
    [
        ai_assistant::events::types::FRONTEND_STATE_SNAPSHOT,
        ai_assistant::events::types::CONVERSATION_LEDGER_DELTA,
        ai_assistant::events::types::CONVERSATION_STATE_DELTA,
        CONVERSATION_CREATED_EVENT,
        CONVERSATION_CLOSED_EVENT,
        WORKFLOW_RESOURCE_CHANGED_EVENT,
        WORKFLOW_EXECUTION_COMPLETED_EVENT,
    ]
}

struct RuntimeEventForwarder {
    event_sender: Arc<Mutex<Option<std_mpsc::Sender<String>>>>,
    projector: Arc<HostEventProjector>,
    event_log: Arc<StdMutex<VecDeque<Value>>>,
    agent_test_event_tx: mpsc::UnboundedSender<Value>,
}

#[async_trait]
impl EventHandler for RuntimeEventForwarder {
    async fn handle(&self, event: &BaseEvent) -> corework::error::Result<()> {
        let envelope = self.projector.project(event).await.unwrap_or_else(|error| {
            json!({
                "schema": "agent-runtime-event/v1",
                "type": "runtime_error",
                "payload": {
                    "message": format!("event projection failed: {error}")
                }
            })
        });
        if let Ok(mut event_log) = self.event_log.lock() {
            event_log.push_back(envelope.clone());
            while event_log.len() > 512 {
                event_log.pop_front();
            }
        }
        let _ = self.agent_test_event_tx.send(envelope.clone());
        if is_internal_studio_event(&envelope) {
            return Ok(());
        }
        let json = serde_json::to_string(&envelope).unwrap_or_else(|_| {
            r#"{"type":"runtime_error","payload":{"message":"event serialization failed"}}"#
                .to_string()
        });
        let sender = match self.event_sender.lock() {
            Ok(sender) => sender.clone(),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "event sender mutex poisoned; dropping projected event"
                );
                None
            }
        };
        if let Some(sender) = sender {
            let _ = sender.send(json);
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "AgentRuntimeFfiEventForwarder"
    }
}

pub(super) fn is_internal_studio_event(envelope: &serde_json::Value) -> bool {
    let is_internal_conversation_id = |conversation_id: &str| {
        conversation_id.starts_with("studio_") || conversation_id.starts_with("agent-test:")
    };
    if envelope
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(is_internal_conversation_id)
    {
        return true;
    }
    let is_internal_cluster_id = |cluster_id: &str| {
        cluster_id == "workflow-studio"
            || cluster_id == "agent-test-supervisor"
            || cluster_id == "agent-test-adversary"
            || cluster_id.starts_with("agent-test")
    };
    envelope
        .pointer("/payload/cluster_id")
        .or_else(|| envelope.pointer("/payload/record/metadata/cluster_id"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(is_internal_cluster_id)
}

pub(super) struct HostEventProjector {
    sequence_backend: Arc<dyn RuntimeSequenceBackend>,
    conversation_id: Option<String>,
    metadata: RuntimeEventMetadata,
}

impl HostEventProjector {
    pub(super) fn runtime(
        sequence_backend: Arc<dyn RuntimeSequenceBackend>,
        metadata: RuntimeEventMetadata,
    ) -> Self {
        Self {
            sequence_backend,
            conversation_id: None,
            metadata,
        }
    }

    #[cfg(test)]
    pub(super) fn for_conversation(
        sequence_backend: Arc<dyn RuntimeSequenceBackend>,
        conversation_id: impl Into<String>,
        metadata: RuntimeEventMetadata,
    ) -> Self {
        Self {
            sequence_backend,
            conversation_id: Some(conversation_id.into()),
            metadata,
        }
    }

    pub(super) async fn project(
        &self,
        event: &BaseEvent,
    ) -> Result<serde_json::Value, RuntimeError> {
        let event_seq = self.sequence_backend.next_global_event_seq().await?;
        // Prefer the projector's bound conversation_id (per-conversation forwarder).
        // For runtime/pod-level forwarders, prefer the BaseEvent metadata
        // stamped by the scoped execution-unit event bus, then fall back to
        // payload-level conversation_id for older event producers.
        let derived_conversation = self
            .conversation_id
            .clone()
            .or_else(|| event.conversation_id.clone())
            .or_else(|| extract_conversation_id_from_payload(&event.payload));
        let conversation_event_seq = match derived_conversation.as_deref() {
            Some(conversation_id) => Some(
                self.sequence_backend
                    .next_conversation_event_seq(conversation_id)
                    .await?,
            ),
            None => None,
        };
        Ok(project_event_with_conversation(
            event,
            event_seq,
            derived_conversation.as_deref(),
            conversation_event_seq,
            &self.metadata,
        ))
    }
}

impl Default for HostEventProjector {
    fn default() -> Self {
        Self::runtime(
            Arc::new(LocalRuntimeSequenceBackend::default()),
            RuntimeEventMetadata::default(),
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct RuntimeEventMetadata {
    pub(super) cluster_id: String,
    pub(super) runtime_profile_id: String,
    pub(super) cluster_fingerprint: Option<String>,
    pub(super) runtime_instance_id: String,
}

impl RuntimeEventMetadata {
    pub(super) fn from_runtime(runtime: &RuntimeSection) -> Self {
        Self {
            cluster_id: runtime.cluster_id.clone(),
            runtime_profile_id: runtime.runtime_profile_id.clone(),
            cluster_fingerprint: runtime.cluster_fingerprint.clone(),
            runtime_instance_id: runtime.runtime_instance_id.clone(),
        }
    }
}

impl Default for RuntimeEventMetadata {
    fn default() -> Self {
        Self::from_runtime(&RuntimeSection::default())
    }
}

#[cfg(test)]
pub(super) fn project_event(event: &BaseEvent, event_seq: u64) -> serde_json::Value {
    project_event_with_conversation(
        event,
        event_seq,
        None,
        None,
        &RuntimeEventMetadata::default(),
    )
}

pub(super) fn project_event_with_conversation(
    event: &BaseEvent,
    event_seq: u64,
    conversation_id: Option<&str>,
    conversation_event_seq: Option<u64>,
    metadata: &RuntimeEventMetadata,
) -> serde_json::Value {
    let ty = host_event_type(event.event_type.as_str());
    let payload = event.payload.clone();

    let mut envelope = json!({
        "schema": "agent-runtime-event/v1",
        "event_seq": event_seq,
        "type": ty,
        "source": event.event_type,
        "event_id": event.event_id,
        "timestamp": event.timestamp,
        "payload": payload,
        "cluster_id": &metadata.cluster_id,
        "runtime_profile_id": &metadata.runtime_profile_id,
        "runtime_instance_id": &metadata.runtime_instance_id,
    });
    if let Some(cluster_fingerprint) = metadata.cluster_fingerprint.as_deref() {
        envelope["cluster_fingerprint"] =
            serde_json::Value::String(cluster_fingerprint.to_string());
    }
    if let Some(conversation_id) = conversation_id {
        envelope["conversation_id"] = serde_json::Value::String(conversation_id.to_string());
    }
    if let Some(conversation_event_seq) = conversation_event_seq {
        envelope["conversation_event_seq"] =
            serde_json::Value::Number(conversation_event_seq.into());
    }
    if let Some(event_line) = event
        .payload
        .get("event_line")
        .and_then(serde_json::Value::as_str)
    {
        envelope["event_line"] = serde_json::Value::String(event_line.to_string());
    }
    envelope
}

pub(super) fn extract_conversation_id_from_payload(payload: &serde_json::Value) -> Option<String> {
    let obj = payload.as_object()?;
    if let Some(v) = obj.get("conversation_id").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Some(v) = obj
        .get("record")
        .and_then(|record| record.get("conversation_id"))
        .and_then(|v| v.as_str())
    {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

pub(super) fn host_event_type(source: &str) -> &str {
    match source {
        ai_assistant::events::types::MESSAGES_CHANGED => "conversation_changed",
        ai_assistant::events::types::CONVERSATION_LEDGER_DELTA => "conversation.ledger_delta",
        ai_assistant::events::types::CONVERSATION_STATE_DELTA => "conversation.state_delta",
        ai_assistant::events::types::AGENT_PAUSE_REQUESTED => "pause_requested",
        ai_assistant::events::types::FOCUS_STATUS_CHANGED => "ui_snapshot_changed",
        ai_assistant::events::types::TURN_START => "turn_started",
        ai_assistant::events::types::THINKING_DONE => "thinking_done",
        ai_assistant::events::types::TOOL_START => "tool_started",
        ai_assistant::events::types::TOOL_END => "tool_finished",
        ai_assistant::events::types::ASKING => "assistant_message",
        ai_assistant::events::types::TURN_DONE => "turn_done",
        ai_assistant::events::types::STREAM_RESET => "stream_reset",
        ai_assistant::events::types::SNAPSHOT_UPDATED => "snapshot_updated",
        ai_assistant::events::types::DRAFT_LOCKED => "draft_locked",
        ai_assistant::events::types::DRAFT_UNLOCKED => "draft_unlocked",
        ai_assistant::events::types::AGENT_ACTIVE => "agent_active",
        ai_assistant::events::types::AGENT_COMPLETED => "agent_completed",
        ai_assistant::events::types::AGENT_SUSPENDED => "agent_suspended",
        ai_assistant::events::types::AGENT_RESUMED => "agent_resumed",
        ai_assistant::events::types::AGENT_CANCELED => "agent_canceled",
        ai_assistant::events::types::INTERRUPTED => "interrupted",
        CONVERSATION_CREATED_EVENT => "conversation:created",
        CONVERSATION_CLOSED_EVENT => "conversation:closed",
        other => other,
    }
}

pub(super) fn sanitize_log_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('.').trim_matches('_');
    if sanitized.is_empty() {
        "unnamed".to_string()
    } else {
        sanitized.chars().take(120).collect()
    }
}

pub(super) fn validate_conversation_log_policy(
    policy: &ConversationLogPolicy,
) -> Result<(), RuntimeError> {
    if policy.max_files_per_cluster == 0 {
        return Err(RuntimeError::InvalidConfig(
            "data.conversation_logs.max_files_per_cluster must be greater than zero".to_string(),
        ));
    }
    if policy.max_file_bytes < 1_024 {
        return Err(RuntimeError::InvalidConfig(
            "data.conversation_logs.max_file_bytes must be at least 1024".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn create_conversation_log_path(
    logs_dir: Option<&Path>,
    cluster_id: &str,
    conversation_id: &str,
    created_at: chrono::DateTime<chrono::Utc>,
    policy: &ConversationLogPolicy,
) -> Option<PathBuf> {
    let logs_dir = logs_dir?;
    let cluster_dir = logs_dir.join(sanitize_log_component(cluster_id));
    if let Err(error) = fs::create_dir_all(&cluster_dir) {
        tracing::warn!(
            cluster_id = %cluster_id,
            conversation_id = %conversation_id,
            "create conversation log directory failed: {error}"
        );
        return None;
    }
    prune_conversation_logs(&cluster_dir, policy);
    let timestamp = created_at.format("%Y%m%dT%H%M%S%.3fZ");
    let path = cluster_dir.join(format!(
        "{}_{}.log",
        sanitize_log_component(conversation_id),
        timestamp
    ));
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(_) => Some(path),
        Err(error) => {
            tracing::warn!(
                cluster_id = %cluster_id,
                conversation_id = %conversation_id,
                path = %path.display(),
                "create conversation log file failed: {error}"
            );
            None
        }
    }
}

pub(super) fn prune_conversation_logs(cluster_dir: &Path, policy: &ConversationLogPolicy) {
    let Ok(entries) = fs::read_dir(cluster_dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let retention = if policy.retention_days == 0 {
        None
    } else {
        Some(std::time::Duration::from_secs(
            policy.retention_days.saturating_mul(24 * 60 * 60),
        ))
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("log") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        if retention.is_some_and(|retention| {
            now.duration_since(modified)
                .map(|age| age > retention)
                .unwrap_or(false)
        }) {
            if let Err(error) = fs::remove_file(&path) {
                tracing::warn!(path = %path.display(), "remove expired conversation log failed: {error}");
            }
            continue;
        }
        files.push((modified, path));
    }

    files.sort_by_key(|(modified, _)| *modified);
    let keep_existing = policy.max_files_per_cluster.saturating_sub(1);
    let remove_count = files.len().saturating_sub(keep_existing);
    for (_, path) in files.into_iter().take(remove_count) {
        if let Err(error) = fs::remove_file(&path) {
            tracing::warn!(path = %path.display(), "remove excess conversation log failed: {error}");
        }
    }
}

pub(super) fn append_conversation_log_path(
    path: Option<&Path>,
    runtime_instance_id: &str,
    cluster_id: &str,
    conversation_id: &str,
    event: &str,
    details: Value,
    max_file_bytes: u64,
) {
    let Some(path) = path else {
        return;
    };
    let line = json!({
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "runtime_instance_id": runtime_instance_id,
        "cluster_id": cluster_id,
        "conversation_id": conversation_id,
        "event": event,
        "details": details,
    })
    .to_string();
    let projected_bytes = line.len().saturating_add(1) as u64;
    if fs::metadata(path)
        .map(|metadata| metadata.len().saturating_add(projected_bytes) > max_file_bytes)
        .unwrap_or(false)
    {
        tracing::warn!(
            conversation_id = %conversation_id,
            path = %path.display(),
            max_file_bytes,
            "conversation log size limit reached; entry dropped"
        );
        return;
    }
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Err(error) = writeln!(file, "{line}") {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    path = %path.display(),
                    "append conversation log failed: {error}"
                );
            }
        }
        Err(error) => tracing::warn!(
            conversation_id = %conversation_id,
            path = %path.display(),
            "open conversation log failed: {error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_global_workflow_events() {
        let forwarded = forwarded_runtime_event_types();
        assert!(forwarded.contains(&WORKFLOW_RESOURCE_CHANGED_EVENT));
        assert!(forwarded.contains(&WORKFLOW_EXECUTION_COMPLETED_EVENT));
    }

    #[test]
    fn llm_facts_are_public_only_through_ledger_deltas() {
        let forwarded = forwarded_runtime_event_types();
        assert!(!forwarded.contains(&ai_assistant::events::types::LLM_USAGE));
        assert!(!forwarded.contains(&ai_assistant::events::types::LLM_ERROR));
    }

    #[test]
    fn internal_studio_events_are_not_public_host_events() {
        assert!(is_internal_studio_event(&json!({
            "type": "frontend:state_snapshot",
            "conversation_id": "studio_1_editor"
        })));
        assert!(is_internal_studio_event(&json!({
            "type": "conversation.ledger_delta",
            "conversation_id": "agent-test:pair-1:target"
        })));
        assert!(is_internal_studio_event(&json!({
            "type": "conversation:created",
            "payload": { "cluster_id": "workflow-studio" }
        })));
        assert!(!is_internal_studio_event(&json!({
            "type": "frontend:state_snapshot",
            "conversation_id": "conv_1"
        })));
    }
}
