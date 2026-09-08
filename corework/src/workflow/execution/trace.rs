use crate::ai_system::AIOutput;
use crate::workflow::core::DataValue;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::HashMap;
use std::time::{Instant, SystemTime};
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSourceRef {
    pub line: Option<u32>,
    #[serde(default)]
    pub col: Option<u32>,
    pub step: Option<String>,
    pub kind: Option<String>,
    pub tool: Option<String>,
    pub text: Option<String>,
}

impl WorkflowSourceRef {
    pub fn from_json(value: &JsonValue) -> Option<Self> {
        let obj = value.as_object()?;
        let source = Self {
            line: obj
                .get("line")
                .and_then(JsonValue::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
            col: obj
                .get("col")
                .or_else(|| obj.get("column"))
                .and_then(JsonValue::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
            step: obj
                .get("step")
                .and_then(JsonValue::as_str)
                .map(str::to_string),
            kind: obj
                .get("kind")
                .and_then(JsonValue::as_str)
                .map(str::to_string),
            tool: obj
                .get("tool")
                .and_then(JsonValue::as_str)
                .map(str::to_string),
            text: obj
                .get("text")
                .and_then(JsonValue::as_str)
                .map(str::to_string),
        };

        if source.line.is_none()
            && source.col.is_none()
            && source.step.is_none()
            && source.kind.is_none()
            && source.tool.is_none()
            && source.text.is_none()
        {
            None
        } else {
            Some(source)
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkflowNodeStatus {
    Started,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeTrace {
    /// Stable Blueprint node id. `node_name` remains display/runtime oriented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Unique identity for this concrete invocation. A node inside a loop gets
    /// a fresh execution id on every iteration.
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    pub node_name: String,
    #[serde(default)]
    pub display_name: String,
    pub node_type: String,
    pub source: Option<WorkflowSourceRef>,
    pub status: WorkflowNodeStatus,
    pub output_pin: Option<String>,
    pub duration_ms: Option<u64>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub to_ai: Option<String>,
    pub error_code: Option<i64>,
    #[serde(default)]
    pub input_preview: Option<JsonValue>,
    pub result_preview: Option<JsonValue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionTrace {
    pub workflow_id: String,
    pub workflow_name: String,
    pub run_id: String,
    pub nodes: Vec<WorkflowNodeTrace>,
    /// Number of ordered node lifecycle deltas emitted for this run.
    #[serde(default)]
    pub event_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTraceEvent {
    pub run_id: String,
    pub sequence: u64,
    pub workflow_name: String,
    pub event_type: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<WorkflowNodeTrace>,
    #[serde(default, skip_serializing_if = "JsonValue::is_null")]
    pub detail: JsonValue,
}

#[derive(Debug)]
pub struct WorkflowTraceRecorder {
    trace: WorkflowExecutionTrace,
    source_map: HashMap<String, WorkflowSourceRef>,
    node_ids: HashMap<String, String>,
    started_at: HashMap<String, Instant>,
    node_indices: HashMap<String, usize>,
    active_executions: Vec<String>,
    event_sender: Option<Sender<WorkflowTraceEvent>>,
}

impl WorkflowTraceRecorder {
    pub fn new(
        workflow_name: impl Into<String>,
        source_map: HashMap<String, WorkflowSourceRef>,
    ) -> Self {
        let workflow_name = workflow_name.into();
        Self::new_with_events(
            workflow_name.clone(),
            workflow_name,
            source_map,
            HashMap::new(),
            None,
        )
    }

    pub fn new_with_events(
        workflow_id: impl Into<String>,
        workflow_name: impl Into<String>,
        source_map: HashMap<String, WorkflowSourceRef>,
        node_ids: HashMap<String, String>,
        event_sender: Option<Sender<WorkflowTraceEvent>>,
    ) -> Self {
        Self::new_with_run_id(
            workflow_id,
            workflow_name,
            new_workflow_run_id(),
            source_map,
            node_ids,
            event_sender,
        )
    }

    pub fn new_with_run_id(
        workflow_id: impl Into<String>,
        workflow_name: impl Into<String>,
        run_id: impl Into<String>,
        source_map: HashMap<String, WorkflowSourceRef>,
        node_ids: HashMap<String, String>,
        event_sender: Option<Sender<WorkflowTraceEvent>>,
    ) -> Self {
        let initial_event_count = u64::from(event_sender.is_some());
        Self {
            trace: WorkflowExecutionTrace {
                workflow_id: workflow_id.into(),
                workflow_name: workflow_name.into(),
                run_id: run_id.into(),
                nodes: Vec::new(),
                // Event-enabled executions reserve sequence 1 for
                // workflow.started, emitted by the workflow service.
                event_count: initial_event_count,
                event_error: None,
            },
            source_map,
            node_ids,
            started_at: HashMap::new(),
            node_indices: HashMap::new(),
            active_executions: Vec::new(),
            event_sender,
        }
    }

    pub fn run_id(&self) -> &str {
        &self.trace.run_id
    }

    fn emit_event(
        &mut self,
        event_type: impl Into<String>,
        node: Option<WorkflowNodeTrace>,
        detail: JsonValue,
    ) {
        let sequence = self.trace.event_count.saturating_add(1);
        if let Some(sender) = self.event_sender.as_ref().cloned() {
            let event = WorkflowTraceEvent {
                run_id: self.trace.run_id.clone(),
                sequence,
                workflow_name: self.trace.workflow_name.clone(),
                event_type: event_type.into(),
                timestamp: now_timestamp(),
                node,
                detail,
            };
            match sender.try_send(event) {
                Ok(()) => self.trace.event_count = sequence,
                Err(error) => {
                    self.trace.event_error = Some(format!(
                        "workflow trace delivery failed after sequence {}: {}",
                        self.trace.event_count, error
                    ));
                    self.event_sender = None;
                }
            }
        } else if self.trace.event_error.is_none() {
            self.trace.event_count = sequence;
        }
    }

    pub fn begin_node(
        &mut self,
        node_name: impl Into<String>,
        node_type: impl Into<String>,
        parent_execution_id: Option<String>,
    ) -> String {
        let node_name = node_name.into();
        let index = self.trace.nodes.len();
        let execution_id = new_workflow_execution_id("node");
        self.started_at.insert(execution_id.clone(), Instant::now());
        self.node_indices.insert(execution_id.clone(), index);
        self.active_executions.push(execution_id.clone());
        let display_name = runtime_display_name(&node_name).to_string();
        self.trace.nodes.push(WorkflowNodeTrace {
            node_id: self.node_ids.get(&node_name).cloned(),
            execution_id: execution_id.clone(),
            parent_execution_id,
            source: self.source_map.get(&node_name).cloned(),
            node_name,
            display_name,
            node_type: node_type.into(),
            status: WorkflowNodeStatus::Started,
            output_pin: None,
            duration_ms: None,
            started_at: now_timestamp(),
            finished_at: None,
            to_ai: None,
            error_code: None,
            input_preview: None,
            result_preview: None,
            error: None,
        });
        if let Some(node) = self.trace.nodes.last().cloned() {
            self.emit_event("node.started", Some(node), JsonValue::Null);
        }
        execution_id
    }

    pub fn finish_node(&mut self, execution_id: &str, output_pin: Option<String>) {
        if let Some(index) = self.node_indices.get(execution_id).copied() {
            if matches!(
                self.trace.nodes[index].status,
                WorkflowNodeStatus::Failed | WorkflowNodeStatus::Succeeded
            ) && self.trace.nodes[index].finished_at.is_some()
            {
                self.close_active_execution(execution_id);
                return;
            }
            let duration_ms = self
                .started_at
                .remove(execution_id)
                .map(|started| started.elapsed().as_millis() as u64);
            let node = &mut self.trace.nodes[index];
            let already_failed = matches!(node.status, WorkflowNodeStatus::Failed);
            if !already_failed {
                node.status = WorkflowNodeStatus::Succeeded;
            }
            node.output_pin = output_pin;
            node.duration_ms = duration_ms;
            node.finished_at = Some(now_timestamp());
            if !already_failed {
                let node = node.clone();
                self.emit_event("node.completed", Some(node), JsonValue::Null);
            }
            self.close_active_execution(execution_id);
        }
    }

    pub fn fail_node(&mut self, execution_id: &str, error: impl Into<String>) {
        if let Some(index) = self.node_indices.get(execution_id).copied() {
            if matches!(
                self.trace.nodes[index].status,
                WorkflowNodeStatus::Failed | WorkflowNodeStatus::Succeeded
            ) && self.trace.nodes[index].finished_at.is_some()
            {
                self.close_active_execution(execution_id);
                return;
            }
            let duration_ms = self
                .started_at
                .remove(execution_id)
                .map(|started| started.elapsed().as_millis() as u64);
            let node = &mut self.trace.nodes[index];
            node.status = WorkflowNodeStatus::Failed;
            node.error = Some(error.into());
            node.duration_ms = duration_ms;
            node.finished_at = Some(now_timestamp());
            let node = node.clone();
            self.emit_event("node.failed", Some(node), JsonValue::Null);
            self.close_active_execution(execution_id);
        }
    }

    pub fn record_ai_output(
        &mut self,
        to_ai: Option<String>,
        error_code: Option<i64>,
        result_preview: Option<JsonValue>,
    ) {
        if let Some(index) = self.current_node_index() {
            let node = &mut self.trace.nodes[index];
            if to_ai.is_some() {
                node.to_ai = to_ai.map(|value| bounded_text(&value, 4096));
            }
            if error_code.is_some() {
                node.error_code = error_code;
            }
            if result_preview.is_some() {
                node.result_preview = result_preview;
            }
        }
    }

    pub fn record_node_values(
        &mut self,
        execution_id: &str,
        input_preview: Option<JsonValue>,
        result_preview: Option<JsonValue>,
    ) {
        if let Some(index) = self.node_indices.get(execution_id).copied() {
            let node = &mut self.trace.nodes[index];
            if input_preview.is_some() {
                node.input_preview = input_preview;
            }
            if result_preview.is_some() && node.result_preview.is_none() {
                node.result_preview = result_preview;
            }
        }
    }

    pub fn finish(mut self) -> WorkflowExecutionTrace {
        for (execution_id, started) in self.started_at.drain() {
            let Some(index) = self.node_indices.get(&execution_id).copied() else {
                continue;
            };
            if let Some(node) = self.trace.nodes.get_mut(index) {
                node.duration_ms = Some(started.elapsed().as_millis() as u64);
                node.finished_at = Some(now_timestamp());
            }
        }
        self.trace
    }

    pub fn emit_control_event(&mut self, event_type: &str, detail: JsonValue) {
        self.emit_event(event_type, None, detail);
    }

    pub fn current_execution_id(&self) -> Option<&str> {
        self.active_executions.last().map(String::as_str)
    }

    fn current_node_index(&self) -> Option<usize> {
        self.current_execution_id()
            .and_then(|id| self.node_indices.get(id).copied())
    }

    fn close_active_execution(&mut self, execution_id: &str) {
        if self
            .active_executions
            .last()
            .is_some_and(|id| id == execution_id)
        {
            self.active_executions.pop();
        } else {
            self.active_executions.retain(|id| id != execution_id);
        }
    }
}

fn runtime_display_name(runtime_name: &str) -> &str {
    runtime_name
        .rsplit_once(" [runtime:")
        .filter(|(_, suffix)| suffix.ends_with(']'))
        .map(|(display_name, _)| display_name)
        .unwrap_or(runtime_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowToAiMode {
    BriefOnSuccess,
    Detailed,
    DetailedOnError,
}

#[derive(Debug, Clone)]
pub struct WorkflowExecutionReport {
    pub outputs: HashMap<String, DataValue>,
    pub trace: Option<WorkflowExecutionTrace>,
}

impl WorkflowExecutionReport {
    pub fn into_ai_output(self, mode: WorkflowToAiMode) -> AIOutput {
        AIOutput::success(self.outputs_json(), self.to_ai(mode, None))
    }

    pub fn outputs_json(&self) -> JsonValue {
        data_outputs_to_json(&self.outputs)
    }

    pub fn to_ai(&self, mode: WorkflowToAiMode, error: Option<&str>) -> String {
        let failed = error.is_some()
            || self
                .trace
                .as_ref()
                .map(|trace| {
                    trace
                        .nodes
                        .iter()
                        .any(|node| matches!(node.status, WorkflowNodeStatus::Failed))
                })
                .unwrap_or(false);

        match mode {
            WorkflowToAiMode::BriefOnSuccess if !failed => {
                "Workflow executed successfully.".to_string()
            }
            WorkflowToAiMode::DetailedOnError if !failed => {
                "Workflow executed successfully.".to_string()
            }
            WorkflowToAiMode::BriefOnSuccess => error
                .map(|e| format!("Workflow execution failed: {e}"))
                .unwrap_or_else(|| "Workflow execution failed.".to_string()),
            WorkflowToAiMode::Detailed | WorkflowToAiMode::DetailedOnError => {
                format_trace_summary(self.trace.as_ref(), error)
            }
        }
    }
}

pub fn data_outputs_to_json(outputs: &HashMap<String, DataValue>) -> JsonValue {
    let mut obj = JsonMap::new();
    for (key, value) in outputs {
        obj.insert(key.clone(), value.value.clone());
    }
    JsonValue::Object(obj)
}

pub fn bounded_json_preview(value: JsonValue) -> JsonValue {
    const MAX_PREVIEW_BYTES: usize = 4096;
    let value = sanitize_trace_value(value, 0);

    let Ok(serialized) = serde_json::to_string(&value) else {
        return JsonValue::String("<unserializable value>".to_string());
    };
    if serialized.len() <= MAX_PREVIEW_BYTES {
        return value;
    }

    let mut end = MAX_PREVIEW_BYTES;
    while !serialized.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::json!({
        "truncated": true,
        "bytes": serialized.len(),
        "preview": &serialized[..end]
    })
}

pub fn format_trace_summary(trace: Option<&WorkflowExecutionTrace>, error: Option<&str>) -> String {
    let mut lines = Vec::new();
    if let Some(error) = error {
        lines.push(format!("Workflow execution failed: {error}"));
    } else {
        lines.push("Workflow execution trace:".to_string());
    }

    let Some(trace) = trace else {
        return lines.join("\n");
    };

    for node in &trace.nodes {
        let source = node
            .source
            .as_ref()
            .map(format_source_ref)
            .unwrap_or_else(|| node.node_name.clone());
        let status = match node.status {
            WorkflowNodeStatus::Started => "started",
            WorkflowNodeStatus::Succeeded => "succeeded",
            WorkflowNodeStatus::Failed => "failed",
            WorkflowNodeStatus::Skipped => "skipped",
        };
        let mut line = format!("- {source} {status}");
        if let Some(pin) = &node.output_pin {
            line.push_str(&format!(" via {pin}"));
        }
        if let Some(duration_ms) = node.duration_ms {
            line.push_str(&format!(" duration_ms={duration_ms}"));
        }
        if let Some(code) = node.error_code {
            line.push_str(&format!(" error_code={code}"));
        }
        if let Some(to_ai) = &node.to_ai {
            line.push_str(&format!(": {to_ai}"));
        }
        if let Some(inputs) = &node.input_preview {
            line.push_str(&format!(" inputs={inputs}"));
        }
        if let Some(result) = &node.result_preview {
            line.push_str(&format!(" result={result}"));
        }
        if let Some(error) = &node.error {
            line.push_str(&format!(" ({error})"));
        }
        lines.push(line);
    }

    lines.join("\n")
}

fn format_source_ref(source: &WorkflowSourceRef) -> String {
    if let Some(step) = &source.step {
        if let Some(line) = source.line {
            return format!("line {line} step {step}");
        }
        return format!("step {step}");
    }
    if let Some(line) = source.line {
        return format!("line {line}");
    }
    source
        .text
        .clone()
        .unwrap_or_else(|| "<unknown source>".to_string())
}

pub(crate) fn new_workflow_run_id() -> String {
    format!("wf-{}", uuid::Uuid::new_v4())
}

fn sanitize_trace_value(value: JsonValue, depth: usize) -> JsonValue {
    const MAX_DEPTH: usize = 6;
    const MAX_ARRAY_ITEMS: usize = 20;
    const MAX_OBJECT_KEYS: usize = 64;
    if depth >= MAX_DEPTH {
        return serde_json::json!({"truncated": true, "reason": "max_depth"});
    }
    match value {
        JsonValue::Object(object) => {
            let total = object.len();
            let mut sanitized = JsonMap::new();
            for (key, value) in object.into_iter().take(MAX_OBJECT_KEYS) {
                let normalized = key.to_ascii_lowercase().replace(['-', ' '], "_");
                let sensitive = [
                    "password",
                    "passwd",
                    "token",
                    "cookie",
                    "authorization",
                    "api_key",
                    "secret",
                ]
                .iter()
                .any(|needle| normalized.contains(needle));
                sanitized.insert(
                    key,
                    if sensitive {
                        JsonValue::String("<redacted>".to_string())
                    } else {
                        sanitize_trace_value(value, depth + 1)
                    },
                );
            }
            if total > MAX_OBJECT_KEYS {
                sanitized.insert(
                    "_trace_truncated".to_string(),
                    serde_json::json!({"total_keys": total, "shown_keys": MAX_OBJECT_KEYS}),
                );
            }
            JsonValue::Object(sanitized)
        }
        JsonValue::Array(values) => {
            let total = values.len();
            let mut preview = values
                .into_iter()
                .take(MAX_ARRAY_ITEMS)
                .map(|value| sanitize_trace_value(value, depth + 1))
                .collect::<Vec<_>>();
            if total > MAX_ARRAY_ITEMS {
                preview.push(serde_json::json!({
                    "_trace_truncated": true,
                    "total_items": total,
                    "shown_items": MAX_ARRAY_ITEMS
                }));
            }
            JsonValue::Array(preview)
        }
        other => other,
    }
}

pub(crate) fn new_workflow_execution_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

fn now_timestamp() -> String {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now()).to_rfc3339()
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}… <truncated; original_bytes={}>",
        &value[..end],
        value.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_redacts_secrets_and_bounds_arrays() {
        let preview = bounded_json_preview(serde_json::json!({
            "api_key": "secret-value",
            "nested": {"authorization": "Bearer secret"},
            "items": (0..30).collect::<Vec<_>>()
        }));
        assert_eq!(preview["api_key"], "<redacted>");
        assert_eq!(preview["nested"]["authorization"], "<redacted>");
        assert_eq!(preview["items"].as_array().unwrap().len(), 21);
        assert_eq!(preview["items"][20]["_trace_truncated"], true);
    }

    #[test]
    fn repeated_node_invocations_have_distinct_execution_ids() {
        let mut recorder = WorkflowTraceRecorder::new_with_run_id(
            "workflow-1",
            "Workflow",
            "run-1",
            HashMap::new(),
            HashMap::new(),
            None,
        );
        let first = recorder.begin_node("node", "Impure", None);
        recorder.finish_node(&first, Some("Then".to_string()));
        let second = recorder.begin_node("node", "Impure", Some("iteration-1".to_string()));
        recorder.finish_node(&second, Some("Then".to_string()));
        let trace = recorder.finish();
        assert_ne!(first, second);
        assert_eq!(
            trace.nodes[1].parent_execution_id.as_deref(),
            Some("iteration-1")
        );
        assert_eq!(trace.nodes[0].status, WorkflowNodeStatus::Succeeded);
        assert_eq!(trace.nodes[1].status, WorkflowNodeStatus::Succeeded);
    }

    #[test]
    fn bounded_event_channel_reports_overflow_without_sequence_gap() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let mut recorder = WorkflowTraceRecorder::new_with_run_id(
            "workflow-1",
            "Workflow",
            "run-1",
            HashMap::new(),
            HashMap::new(),
            Some(sender),
        );
        let execution_id = recorder.begin_node("遍历数组 [runtime:1.1]", "Impure", None);
        recorder.finish_node(&execution_id, Some("Then".to_string()));
        let trace = recorder.finish();

        assert_eq!(trace.event_count, 2);
        assert!(trace
            .event_error
            .as_deref()
            .is_some_and(|error| { error.contains("delivery failed after sequence 2") }));
        assert_eq!(trace.nodes[0].display_name, "遍历数组");
    }
}
