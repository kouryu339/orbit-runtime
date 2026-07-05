use crate::ai_system::AIOutput;
use crate::workflow::core::DataValue;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSourceRef {
    pub line: Option<u32>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkflowNodeStatus {
    Started,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeTrace {
    pub node_name: String,
    pub node_type: String,
    pub source: Option<WorkflowSourceRef>,
    pub status: WorkflowNodeStatus,
    pub output_pin: Option<String>,
    pub duration_ms: Option<u64>,
    pub to_ai: Option<String>,
    pub error_code: Option<i64>,
    pub result_preview: Option<JsonValue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionTrace {
    pub workflow_name: String,
    pub run_id: String,
    pub nodes: Vec<WorkflowNodeTrace>,
}

#[derive(Debug)]
pub struct WorkflowTraceRecorder {
    trace: WorkflowExecutionTrace,
    source_map: HashMap<String, WorkflowSourceRef>,
    started_at: HashMap<usize, Instant>,
}

impl WorkflowTraceRecorder {
    pub fn new(
        workflow_name: impl Into<String>,
        source_map: HashMap<String, WorkflowSourceRef>,
    ) -> Self {
        Self {
            trace: WorkflowExecutionTrace {
                workflow_name: workflow_name.into(),
                run_id: make_run_id(),
                nodes: Vec::new(),
            },
            source_map,
            started_at: HashMap::new(),
        }
    }

    pub fn begin_node(&mut self, node_name: impl Into<String>, node_type: impl Into<String>) {
        let node_name = node_name.into();
        let index = self.trace.nodes.len();
        self.started_at.insert(index, Instant::now());
        self.trace.nodes.push(WorkflowNodeTrace {
            source: self.source_map.get(&node_name).cloned(),
            node_name,
            node_type: node_type.into(),
            status: WorkflowNodeStatus::Started,
            output_pin: None,
            duration_ms: None,
            to_ai: None,
            error_code: None,
            result_preview: None,
            error: None,
        });
    }

    pub fn finish_node(&mut self, node_name: &str, output_pin: Option<String>) {
        if let Some(index) = self.last_node_index(node_name) {
            let duration_ms = self
                .started_at
                .remove(&index)
                .map(|started| started.elapsed().as_millis() as u64);
            let node = &mut self.trace.nodes[index];
            if !matches!(node.status, WorkflowNodeStatus::Failed) {
                node.status = WorkflowNodeStatus::Succeeded;
            }
            node.output_pin = output_pin;
            node.duration_ms = duration_ms;
        }
    }

    pub fn fail_node(&mut self, node_name: &str, error: impl Into<String>) {
        if let Some(index) = self.last_node_index(node_name) {
            let duration_ms = self
                .started_at
                .remove(&index)
                .map(|started| started.elapsed().as_millis() as u64);
            let node = &mut self.trace.nodes[index];
            node.status = WorkflowNodeStatus::Failed;
            node.error = Some(error.into());
            node.duration_ms = duration_ms;
        }
    }

    pub fn record_ai_output(
        &mut self,
        to_ai: Option<String>,
        error_code: Option<i64>,
        result_preview: Option<JsonValue>,
    ) {
        if let Some(node) = self.trace.nodes.last_mut() {
            if to_ai.is_some() {
                node.to_ai = to_ai;
            }
            if error_code.is_some() {
                node.error_code = error_code;
            }
            if result_preview.is_some() {
                node.result_preview = result_preview;
            }
        }
    }

    pub fn finish(mut self) -> WorkflowExecutionTrace {
        for (index, started) in self.started_at.drain() {
            if let Some(node) = self.trace.nodes.get_mut(index) {
                node.duration_ms = Some(started.elapsed().as_millis() as u64);
            }
        }
        self.trace
    }

    fn last_node_index(&self, node_name: &str) -> Option<usize> {
        self.trace
            .nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, node)| node.node_name == node_name)
            .map(|(index, _)| index)
    }
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
        if let Some(code) = node.error_code {
            line.push_str(&format!(" error_code={code}"));
        }
        if let Some(to_ai) = &node.to_ai {
            line.push_str(&format!(": {to_ai}"));
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

fn make_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("wf-{nanos}")
}
