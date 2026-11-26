//! Background execution and terminal events for the native and browser host APIs.
use super::executor::WorkflowExecutionOutcome;
use super::WorkflowsModule;
use crate::error::{FrameworkError, Result};
use crate::platform::{self, task::JoinHandle, time::Instant};
use crate::workflow::execution::WorkflowNodeStatus;
use futures::FutureExt;
use serde_json::{json, Value};
use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};

const MAX_ACTIVE_RUNS: usize = 128;
const MAX_TERMINAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub struct HostWorkflowRuns {
    tasks: Vec<JoinHandle<()>>,
}

impl HostWorkflowRuns {
    pub fn submit<F>(
        &mut self,
        module: Arc<WorkflowsModule>,
        workflow_id: String,
        trace: bool,
        execute: impl FnOnce(String) -> F,
    ) -> Result<Value>
    where
        F: Future<Output = Result<WorkflowExecutionOutcome>> + Send + 'static,
    {
        self.tasks.retain(|task| !task.is_finished());
        if self.tasks.len() >= MAX_ACTIVE_RUNS {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow run capacity reached ({MAX_ACTIVE_RUNS} active runs)"
            )));
        }
        let run_id = crate::workflow::execution::trace::new_workflow_run_id();
        let task_run_id = run_id.clone();
        let task_workflow_id = workflow_id.clone();
        let execution = execute(task_run_id.clone());
        self.tasks.push(platform::spawn(async move {
            let started = Instant::now();
            let outcome = AssertUnwindSafe(execution).catch_unwind().await;
            let mut payload = json!({
                "schema": "agent-runtime-workflow-trace/v1",
                "event_line": "workflow",
                "trace_type": "workflow.failed",
                "workflow_run_id": task_run_id,
                "run_id": task_run_id,
                "workflow_id": task_workflow_id,
                "sequence": 1,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "status": "failed",
                "code": -1,
                "duration_ms": started.elapsed().as_millis()
            });
            match outcome {
                Ok(Ok(outcome)) => {
                    if let Some(report) = &outcome.report.trace {
                        payload["sequence"] = json!(report.event_count.saturating_add(1));
                        if trace {
                            payload["node_trace"] = json!(report);
                        }
                    }
                    if let Some(error) = outcome.error {
                        let node = outcome.report.trace.as_ref().and_then(|report| {
                            report.nodes.iter().rev().find(|node| node.status == WorkflowNodeStatus::Failed)
                        });
                        payload["error"] = json!({
                            "code": "WORKFLOW_EXECUTION_FAILED",
                            "message": error,
                            "node_id": node.and_then(|node| node.node_id.as_deref()),
                            "execution_id": node.map(|node| &node.execution_id)
                        });
                    } else {
                        payload["trace_type"] = json!("workflow.completed");
                        payload["status"] = json!("succeeded");
                        payload["code"] = json!(0);
                        payload["result"] = outcome.report.outputs_json();
                    }
                }
                Ok(Err(error)) => {
                    payload["code"] = json!(400);
                    payload["error"] = json!({
                        "code": "WORKFLOW_PREPARATION_FAILED", "message": error.to_string()
                    });
                }
                Err(_) => {
                    payload["error"] = json!({
                        "code": "WORKFLOW_EXECUTION_PANICKED", "message": "workflow execution panicked"
                    });
                }
            }
            if serde_json::to_vec(&payload).map_or(true, |bytes| bytes.len() > MAX_TERMINAL_BYTES) {
                let fields = payload.as_object_mut().expect("terminal payload is an object");
                fields.remove("result");
                fields.remove("node_trace");
                payload["trace_type"] = json!("workflow.failed");
                payload["status"] = json!("failed");
                payload["code"] = json!(413);
                payload["error"] = json!({
                    "code": "WORKFLOW_RESULT_TOO_LARGE",
                    "message": "workflow terminal event exceeded the 16 MiB limit"
                });
            }
            module.publish_execution_event(payload).await;
        }));
        Ok(json!({
            "schema": "agent-runtime-workflow-run/v1",
            "workflow_id": workflow_id,
            "workflow_run_id": run_id,
            "run_id": run_id,
            "status": "accepted"
        }))
    }

    pub async fn drain(self) {
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

impl WorkflowsModule {
    pub async fn execute_catalog_outcome_with_run_id(
        &self,
        id: &str,
        inputs: std::collections::HashMap<String, Value>,
        context: &super::WorkflowExecutionContext,
        run_id: String,
    ) -> Result<WorkflowExecutionOutcome> {
        match self.read_workflow_resource(id)?.summary.kind {
            super::WorkflowResourceKind::Registered => {
                self.execute_registered_outcome_with_context_and_run_id(
                    id,
                    inputs,
                    context,
                    Some(run_id),
                )
                .await
            }
            super::WorkflowResourceKind::Draft => {
                self.execute_from_blueprint_outcome_with_context_and_run_id(
                    self.draft_blueprint(id)?,
                    inputs,
                    context,
                    Some(run_id),
                )
                .await
            }
        }
    }
}
