use super::*;

const MAX_ACTIVE_WORKFLOW_RUNS: usize = 128;
const CREATED_RUN_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowRunPhase {
    Created,
    Running,
}

#[derive(Debug, Clone, Copy)]
struct WorkflowRunControl {
    phase: WorkflowRunPhase,
    created_at: std::time::Instant,
}

/// Minimal control-plane registry for two-phase workflow execution.
/// Trace events and terminal results are delivered through the public event
/// channel; the host owns persistence and replay.
#[derive(Debug, Default)]
pub(super) struct WorkflowRunRegistry {
    active: HashMap<String, WorkflowRunControl>,
}

impl WorkflowRunRegistry {
    pub fn create(&mut self) -> Result<String, String> {
        self.expire_abandoned();
        if self.active.len() >= MAX_ACTIVE_WORKFLOW_RUNS {
            return Err(format!(
                "workflow run capacity reached ({MAX_ACTIVE_WORKFLOW_RUNS} active runs)"
            ));
        }
        let workflow_run_id = format!("wf-{}", uuid::Uuid::new_v4());
        self.active.insert(
            workflow_run_id.clone(),
            WorkflowRunControl {
                phase: WorkflowRunPhase::Created,
                created_at: std::time::Instant::now(),
            },
        );
        Ok(workflow_run_id)
    }

    pub fn begin(&mut self, workflow_run_id: &str) -> Result<(), String> {
        self.expire_abandoned();
        let control = self
            .active
            .get_mut(workflow_run_id)
            .ok_or_else(|| format!("workflow run '{workflow_run_id}' was not started"))?;
        if control.phase != WorkflowRunPhase::Created {
            return Err(format!(
                "workflow run '{workflow_run_id}' has already been run"
            ));
        }
        control.phase = WorkflowRunPhase::Running;
        Ok(())
    }

    pub fn finish_from_envelope(&mut self, envelope: &Value) {
        let Some(payload) = envelope.get("payload") else {
            return;
        };
        let Some(workflow_run_id) = payload
            .get("workflow_run_id")
            .or_else(|| payload.get("run_id"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let trace_type = payload.get("trace_type").and_then(Value::as_str);
        let terminal = matches!(trace_type, Some("workflow.completed" | "workflow.failed"))
            || envelope.get("type").and_then(Value::as_str)
                == Some(WORKFLOW_EXECUTION_COMPLETED_EVENT);
        if terminal {
            self.active.remove(workflow_run_id);
        }
    }

    fn expire_abandoned(&mut self) {
        self.active.retain(|_, control| {
            control.phase == WorkflowRunPhase::Running
                || control.created_at.elapsed() < CREATED_RUN_TTL
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_identity_is_one_shot_and_released_by_terminal_event() {
        let mut registry = WorkflowRunRegistry::default();
        let run_id = registry.create().unwrap();
        registry.begin(&run_id).unwrap();
        assert!(registry.begin(&run_id).is_err());
        registry.finish_from_envelope(&json!({
            "type": WORKFLOW_EXECUTION_COMPLETED_EVENT,
            "payload": {
                "workflow_run_id": run_id,
                "trace_type": "workflow.completed"
            }
        }));
        assert!(registry.begin(&run_id).is_err());
    }
}
