use corework::error::{FrameworkError, Result};
use corework::event::BaseEvent;
use corework::rpc_tool::RuntimeToolMetadata;
use corework::workflow::blueprint_json::BlueprintJson;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::RwLock;
use uuid::Uuid;

use super::executor::{BlueprintEntry, WorkflowsModule};

pub fn preserve_workflow_blueprint_layout(previous: &BlueprintJson, next: &mut BlueprintJson) {
    for node in &mut next.nodes {
        let source_step = workflow_source_step(node);
        let previous_node = previous.nodes.iter().find(|candidate| {
            candidate.id == node.id
                || source_step
                    .as_deref()
                    .is_some_and(|step| workflow_source_step(candidate).as_deref() == Some(step))
        });
        let Some(previous_node) = previous_node else {
            continue;
        };
        node.position = previous_node.position.clone();
        node.size = previous_node.size.clone();
        node.display_name = previous_node
            .display_name
            .clone()
            .or(node.display_name.clone());
        node.comment = previous_node.comment.clone();
        if let Some(layout) = previous_node.properties.get("layout") {
            node.properties.insert("layout".to_string(), layout.clone());
        }
    }
}

fn workflow_source_step(
    node: &corework::workflow::blueprint_json::BlueprintNodeJson,
) -> Option<String> {
    node.properties
        .get("source_script")?
        .get("step")?
        .as_str()
        .map(str::to_string)
}

pub(crate) const DRAFT_REGISTRY: &str = "wf:draft_registry";
pub const WORKFLOW_RESOURCE_CHANGED_EVENT: &str = "workflow.resource_changed";
pub const WORKFLOW_EXECUTION_COMPLETED_EVENT: &str = "workflow.execution_completed";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowResourceKind {
    Draft,
    Registered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowValidation {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl WorkflowValidation {
    pub fn valid() -> Self {
        Self {
            valid: true,
            error: None,
        }
    }

    pub fn invalid(error: impl Into<String>) -> Self {
        Self {
            valid: false,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DraftWorkflowEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blueprint: Option<BlueprintJson>,
    pub(crate) validation: WorkflowValidation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowResourceSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: WorkflowResourceKind,
    pub revision: u64,
    pub trusted: bool,
    pub production_executable: bool,
    pub validation: WorkflowValidation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResourceView {
    #[serde(flatten)]
    pub summary: WorkflowResourceSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<BlueprintJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowEditorSelection {
    pub workflow_id: String,
    pub revision: u64,
}

#[derive(Debug)]
pub struct WorkflowEditorSession {
    selection: RwLock<Option<WorkflowEditorSelection>>,
    runtime_tools: Vec<RuntimeToolMetadata>,
}

impl WorkflowEditorSession {
    pub fn new(runtime_tools: Vec<RuntimeToolMetadata>) -> Self {
        Self {
            selection: RwLock::new(None),
            runtime_tools,
        }
    }

    pub fn selection(&self) -> Option<WorkflowEditorSelection> {
        self.selection.read().ok().and_then(|value| value.clone())
    }

    pub fn select(&self, workflow_id: impl Into<String>, revision: u64) {
        if let Ok(mut selection) = self.selection.write() {
            *selection = Some(WorkflowEditorSelection {
                workflow_id: workflow_id.into(),
                revision,
            });
        }
    }

    pub fn clear(&self) {
        if let Ok(mut selection) = self.selection.write() {
            *selection = None;
        }
    }

    pub fn runtime_tools(&self) -> &[RuntimeToolMetadata] {
        &self.runtime_tools
    }
}

impl WorkflowsModule {
    pub async fn create_draft_resource(
        &self,
        requested_id: Option<&str>,
        name: &str,
        description: &str,
        script: Option<String>,
        blueprint: Option<BlueprintJson>,
        validation: WorkflowValidation,
    ) -> Result<WorkflowResourceView> {
        let id = match requested_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => Self::validate_resource_id(id)?,
            None => format!("wf_{}", Uuid::new_v4().simple()),
        };
        let name = self.validate_resource_name(name)?;
        self.ensure_id_available(&id)?;
        self.ensure_name_available(&name, None)?;

        let mut drafts = self.get_draft_registry()?;
        let entry = DraftWorkflowEntry {
            id,
            name,
            description: description.trim().to_string(),
            revision: 1,
            script,
            blueprint,
            validation,
        };
        drafts.push(entry.clone());
        self.unit.set_resource(DRAFT_REGISTRY, &drafts, None)?;
        let view = Self::draft_view(entry);
        self.publish_resource_change("created", None, &view.summary)
            .await;
        Ok(view)
    }

    pub async fn update_draft_resource(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        name: &str,
        description: &str,
        script: Option<String>,
        blueprint: Option<BlueprintJson>,
        validation: WorkflowValidation,
    ) -> Result<WorkflowResourceView> {
        let id = Self::validate_resource_id(id)?;
        let name = self.validate_resource_name(name)?;
        let mut drafts = self.get_draft_registry()?;
        let position = drafts
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| missing_resource(&id, WorkflowResourceKind::Draft))?;
        let previous_revision = drafts[position].revision;
        ensure_revision(&id, previous_revision, expected_revision)?;
        self.ensure_name_available(&name, Some(&id))?;

        let entry = DraftWorkflowEntry {
            id,
            name,
            description: description.trim().to_string(),
            revision: previous_revision.saturating_add(1),
            script,
            blueprint,
            validation,
        };
        drafts[position] = entry.clone();
        self.unit.set_resource(DRAFT_REGISTRY, &drafts, None)?;
        let view = Self::draft_view(entry);
        self.publish_resource_change("updated", Some(previous_revision), &view.summary)
            .await;
        Ok(view)
    }

    pub async fn update_registered_resource(
        &self,
        blueprint: &BlueprintJson,
        expected_revision: Option<u64>,
    ) -> Result<WorkflowResourceView> {
        let id = Self::validate_resource_id(&blueprint.metadata.id)?;
        let existing = self
            .get_registry()?
            .into_iter()
            .find(|entry| entry.metadata.id == id)
            .ok_or_else(|| missing_resource(&id, WorkflowResourceKind::Registered))?;
        ensure_revision(&id, existing.revision, expected_revision)?;
        let registered =
            self.persist_resource(blueprint, true, Some(existing.revision.saturating_add(1)))?;
        let view = self.read_workflow_resource(&registered.id)?;
        self.publish_resource_change("updated", Some(existing.revision), &view.summary)
            .await;
        Ok(view)
    }

    pub async fn register_draft_resource(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        registered_name: Option<&str>,
    ) -> Result<WorkflowResourceView> {
        let id = Self::validate_resource_id(id)?;
        let mut drafts = self.get_draft_registry()?;
        let position = drafts
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| missing_resource(&id, WorkflowResourceKind::Draft))?;
        let draft = drafts[position].clone();
        ensure_revision(&id, draft.revision, expected_revision)?;
        if !draft.validation.valid {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow draft '{}' is not valid and cannot be registered: {}",
                id,
                draft
                    .validation
                    .error
                    .as_deref()
                    .unwrap_or("compile failed")
            )));
        }
        let mut blueprint = draft.blueprint.clone().ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "workflow draft '{}' has no compiled blueprint",
                id
            ))
        })?;
        let name = self.validate_resource_name(registered_name.unwrap_or(&draft.name))?;
        self.ensure_name_available(&name, Some(&id))?;
        blueprint.metadata.id = id.clone();
        blueprint.metadata.name = name;
        blueprint.metadata.description = draft.description.clone();

        drafts.remove(position);
        self.unit.set_resource(DRAFT_REGISTRY, &drafts, None)?;
        match self.persist_resource(&blueprint, false, Some(draft.revision.saturating_add(1))) {
            Ok(_) => {
                let view = self.read_workflow_resource(&id)?;
                self.publish_event(
                    WORKFLOW_RESOURCE_CHANGED_EVENT,
                    json!({
                        "schema": "agent-runtime-workflow-change/v1",
                        "workflow_id": view.summary.id,
                        "operation": "registered",
                        "previous_kind": "draft",
                        "kind": "registered",
                        "previous_revision": draft.revision,
                        "revision": view.summary.revision,
                        "trusted": true,
                        "production_executable": true
                    }),
                )
                .await;
                Ok(view)
            }
            Err(error) => {
                let mut rollback = self.get_draft_registry()?;
                rollback.push(draft);
                let _ = self.unit.set_resource(DRAFT_REGISTRY, &rollback, None);
                Err(error)
            }
        }
    }

    pub async fn delete_catalog_resource(
        &self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<WorkflowResourceSummary> {
        let id = Self::validate_resource_id(id)?;
        let mut drafts = self.get_draft_registry()?;
        if let Some(position) = drafts.iter().position(|entry| entry.id == id) {
            let entry = drafts[position].clone();
            ensure_revision(&id, entry.revision, expected_revision)?;
            drafts.remove(position);
            self.unit.set_resource(DRAFT_REGISTRY, &drafts, None)?;
            let summary = Self::draft_summary(&entry);
            self.publish_deleted(&summary).await;
            return Ok(summary);
        }
        let registry = self.get_registry()?;
        let entry = registry
            .iter()
            .find(|entry| entry.metadata.id == id)
            .cloned()
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "workflow resource '{}' does not exist",
                    id
                ))
            })?;
        ensure_revision(&id, entry.revision, expected_revision)?;
        let deleted = self.delete_resource(&id)?;
        let summary = WorkflowResourceSummary {
            id: deleted.id,
            name: deleted.name,
            description: deleted.description,
            kind: WorkflowResourceKind::Registered,
            revision: deleted.revision,
            trusted: true,
            production_executable: true,
            validation: WorkflowValidation::valid(),
        };
        self.publish_deleted(&summary).await;
        Ok(summary)
    }

    pub fn read_workflow_resource(&self, id: &str) -> Result<WorkflowResourceView> {
        let id = Self::validate_resource_id(id)?;
        if let Some(entry) = self
            .get_draft_registry()?
            .into_iter()
            .find(|entry| entry.id == id)
        {
            return Ok(Self::draft_view(entry));
        }
        let entry = self
            .get_registry()?
            .into_iter()
            .find(|entry| entry.metadata.id == id)
            .ok_or_else(|| {
                FrameworkError::InvalidOperation(format!(
                    "workflow resource '{}' does not exist",
                    id
                ))
            })?;
        Self::registered_view(entry)
    }

    pub fn list_workflow_catalog(
        &self,
        kind: Option<WorkflowResourceKind>,
    ) -> Result<Vec<WorkflowResourceSummary>> {
        let mut resources = Vec::new();
        if kind != Some(WorkflowResourceKind::Registered) {
            resources.extend(self.get_draft_registry()?.iter().map(Self::draft_summary));
        }
        if kind != Some(WorkflowResourceKind::Draft) {
            resources.extend(self.get_registry()?.into_iter().map(|entry| {
                WorkflowResourceSummary {
                    id: entry.metadata.id,
                    name: entry.metadata.name,
                    description: entry.metadata.description,
                    kind: WorkflowResourceKind::Registered,
                    revision: entry.revision,
                    trusted: true,
                    production_executable: true,
                    validation: WorkflowValidation::valid(),
                }
            }));
        }
        resources.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(resources)
    }

    pub fn draft_blueprint(&self, id: &str) -> Result<BlueprintJson> {
        let id = Self::validate_resource_id(id)?;
        let entry = self
            .get_draft_registry()?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| missing_resource(&id, WorkflowResourceKind::Draft))?;
        if !entry.validation.valid {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow draft '{}' is not valid: {}",
                id,
                entry
                    .validation
                    .error
                    .as_deref()
                    .unwrap_or("compile failed")
            )));
        }
        entry.blueprint.ok_or_else(|| {
            FrameworkError::InvalidOperation(format!(
                "workflow draft '{}' has no compiled blueprint",
                id
            ))
        })
    }

    pub(crate) fn ensure_name_available(&self, name: &str, except_id: Option<&str>) -> Result<()> {
        let normalized = normalize_name(name);
        let registered_conflict = self.get_registry()?.into_iter().find(|entry| {
            except_id != Some(entry.metadata.id.as_str())
                && normalize_name(&entry.metadata.name) == normalized
        });
        let draft_conflict = self.get_draft_registry()?.into_iter().find(|entry| {
            except_id != Some(entry.id.as_str()) && normalize_name(&entry.name) == normalized
        });
        if let Some((id, kind)) = registered_conflict
            .map(|entry| (entry.metadata.id, WorkflowResourceKind::Registered))
            .or_else(|| draft_conflict.map(|entry| (entry.id, WorkflowResourceKind::Draft)))
        {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow name '{}' already exists on '{}' ({:?})",
                name, id, kind
            )));
        }
        Ok(())
    }

    pub(crate) fn get_draft_registry(&self) -> Result<Vec<DraftWorkflowEntry>> {
        Ok(self.unit.get_resource(DRAFT_REGISTRY)?.unwrap_or_default())
    }

    pub async fn publish_execution_event(&self, payload: Value) {
        self.publish_event(WORKFLOW_EXECUTION_COMPLETED_EVENT, payload)
            .await;
    }

    async fn publish_resource_change(
        &self,
        operation: &str,
        previous_revision: Option<u64>,
        workflow: &WorkflowResourceSummary,
    ) {
        self.publish_event(
            WORKFLOW_RESOURCE_CHANGED_EVENT,
            json!({
                "schema": "agent-runtime-workflow-change/v1",
                "workflow_id": workflow.id,
                "operation": operation,
                "kind": workflow.kind,
                "previous_revision": previous_revision,
                "revision": workflow.revision,
                "trusted": workflow.trusted,
                "production_executable": workflow.production_executable
            }),
        )
        .await;
    }

    async fn publish_deleted(&self, workflow: &WorkflowResourceSummary) {
        self.publish_event(
            WORKFLOW_RESOURCE_CHANGED_EVENT,
            json!({
                "schema": "agent-runtime-workflow-change/v1",
                "workflow_id": workflow.id,
                "operation": "deleted",
                "kind": workflow.kind,
                "previous_revision": workflow.revision,
                "revision": workflow.revision.saturating_add(1),
                "trusted": workflow.trusted,
                "production_executable": workflow.production_executable
            }),
        )
        .await;
    }

    async fn publish_event(&self, event_type: &str, mut payload: Value) {
        if let Some(payload) = payload.as_object_mut() {
            payload.insert(
                "event_line".to_string(),
                Value::String("workflow".to_string()),
            );
        }
        if let Err(error) = self
            .event_bus
            .publish(BaseEvent::new(event_type, payload))
            .await
        {
            tracing::warn!(event_type, %error, "publish workflow domain event failed");
        }
    }

    fn ensure_id_available(&self, id: &str) -> Result<()> {
        let registered = self
            .get_registry()?
            .iter()
            .any(|entry| entry.metadata.id == id);
        let draft = self
            .get_draft_registry()?
            .iter()
            .any(|entry| entry.id == id);
        if registered || draft {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow id '{}' already exists",
                id
            )));
        }
        Ok(())
    }

    fn validate_resource_name(&self, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(FrameworkError::InvalidOperation(
                "workflow resource name must not be empty".to_string(),
            ));
        }
        Ok(name.to_string())
    }

    fn draft_summary(entry: &DraftWorkflowEntry) -> WorkflowResourceSummary {
        WorkflowResourceSummary {
            id: entry.id.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            kind: WorkflowResourceKind::Draft,
            revision: entry.revision,
            trusted: false,
            production_executable: false,
            validation: entry.validation.clone(),
        }
    }

    fn draft_view(entry: DraftWorkflowEntry) -> WorkflowResourceView {
        WorkflowResourceView {
            summary: Self::draft_summary(&entry),
            script: entry.script,
            blueprint: entry.blueprint,
        }
    }

    fn registered_view(entry: BlueprintEntry) -> Result<WorkflowResourceView> {
        let blueprint = BlueprintJson::from_workflow_file(&entry.file_path)
            .map_err(FrameworkError::SystemError)?;
        let script = corework::workflow::chain_decompiler::decompile_chain(&blueprint).ok();
        Ok(WorkflowResourceView {
            summary: WorkflowResourceSummary {
                id: entry.metadata.id,
                name: entry.metadata.name,
                description: entry.metadata.description,
                kind: WorkflowResourceKind::Registered,
                revision: entry.revision,
                trusted: true,
                production_executable: true,
                validation: WorkflowValidation::valid(),
            },
            script,
            blueprint: Some(blueprint),
        })
    }
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn ensure_revision(id: &str, actual: u64, expected: Option<u64>) -> Result<()> {
    if let Some(expected) = expected {
        if expected != actual {
            return Err(FrameworkError::InvalidOperation(format!(
                "workflow '{}' revision conflict: expected {}, current {}",
                id, expected, actual
            )));
        }
    }
    Ok(())
}

fn missing_resource(id: &str, kind: WorkflowResourceKind) -> FrameworkError {
    FrameworkError::InvalidOperation(format!(
        "workflow {:?} resource '{}' does not exist",
        kind, id
    ))
}

#[cfg(test)]
mod tests {
    use super::preserve_workflow_blueprint_layout;
    use crate::workflow::chain_compiler_v2::compile_chain_v2;
    use serde_json::json;

    #[test]
    fn preserves_layout_by_source_step_when_compilation_changes_node_ids() {
        let mut previous =
            compile_chain_v2("input\nreturn").expect("previous workflow should compile");
        let previous_node = previous.nodes.first_mut().expect("compiled node");
        previous_node.id = "old-node-id".to_string();
        previous_node
            .properties
            .insert("source_script".to_string(), json!({"step": "first"}));
        previous_node.position.x = 321.0;
        previous_node.position.y = 654.0;
        previous_node.display_name = Some("Pinned layout".to_string());
        previous_node
            .properties
            .insert("layout".to_string(), json!({"collapsed": true}));

        let mut next = compile_chain_v2("input\nreturn").expect("updated workflow should compile");
        let next_node = next.nodes.first_mut().expect("compiled node");
        next_node.id = "new-node-id".to_string();
        next_node
            .properties
            .insert("source_script".to_string(), json!({"step": "first"}));
        preserve_workflow_blueprint_layout(&previous, &mut next);

        let next_node = next
            .nodes
            .iter()
            .find(|node| {
                node.properties
                    .get("source_script")
                    .and_then(|value| value.get("step"))
                    .and_then(|value| value.as_str())
                    == Some("first")
            })
            .expect("updated step node");
        assert_eq!(next_node.position.x, 321.0);
        assert_eq!(next_node.position.y, 654.0);
        assert_eq!(next_node.display_name.as_deref(), Some("Pinned layout"));
        assert_eq!(
            next_node.properties.get("layout"),
            Some(&json!({"collapsed": true}))
        );
    }
}
