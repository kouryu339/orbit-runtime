//! JSON resource lifecycle. Source files are durable; compiled graphs are accepted in memory.
use super::{
    catalog::{DraftWorkflowEntry, DRAFT_REGISTRY},
    executor::{BlueprintEntry, WorkflowSource, REGISTRY},
    revision, WorkflowResourceKind, WorkflowsModule,
};
use crate::{
    error::{FrameworkError, Result},
    rpc_tool::RuntimeToolMetadata,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub const RESOURCE_SCHEMA: &str = "orbit.workflow.resource/v1";
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceDocument {
    schema: String,
    id: String,
    metadata: ResourceMetadata,
    state: WorkflowResourceKind,
    revision: u64,
    logic: crate::workflow::logic::LogicDocument,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceMetadata {
    name: String,
    description: String,
    extensions: HashMap<String, Value>,
}

fn parse(source: &str) -> Result<ResourceDocument> {
    let value = crate::workflow::logic::source::normalize_source(source).map_err(diagnostic)?;
    parse_document(value)
}

fn parse_document(value: Value) -> Result<ResourceDocument> {
    let resource: ResourceDocument = serde_json::from_value(value)?;
    if resource.schema != RESOURCE_SCHEMA {
        return Err(invalid("Expected orbit.workflow.resource/v1"));
    }
    if WorkflowsModule::validate_resource_id(&resource.id)? != resource.id {
        return Err(invalid("Resource id cannot contain surrounding whitespace"));
    }
    if resource.metadata.name.trim().is_empty()
        || resource.revision == 0
        || resource.revision > 9_007_199_254_740_991
    {
        return Err(invalid(
            "Resource name must be nonempty and revision must be a positive safe integer",
        ));
    }
    if resource
        .metadata
        .extensions
        .values()
        .any(|value| !value.is_object())
    {
        return Err(invalid(
            "metadata.extensions values must be namespaced objects",
        ));
    }
    Ok(resource)
}

/// Header discovery defers tool/reference compilation until Runtime startup.
pub fn resource_source_kind(source: &str) -> Result<Option<WorkflowResourceKind>> {
    let value = match crate::workflow::logic::source::normalize_source(source) {
        Ok(value) => value,
        Err(error) if source.contains("orbit.workflow.resource") => return Err(diagnostic(error)),
        Err(_) => return Ok(None),
    };
    match value.get("schema").and_then(Value::as_str) {
        Some(RESOURCE_SCHEMA) => Ok(Some(parse_document(value)?.state)),
        Some(schema) if schema.starts_with("orbit.workflow.resource/") => {
            Err(invalid(format!("Unsupported resource schema: {schema}")))
        }
        _ => Ok(None),
    }
}
fn stamp(source: &str, state: WorkflowResourceKind, revision: u64) -> Result<String> {
    crate::workflow::logic::source::stamp_resource_source(
        source,
        if state == WorkflowResourceKind::Draft {
            "draft"
        } else {
            "registered"
        },
        revision,
    )
    .map_err(diagnostic)
}
fn diagnostic(error: crate::workflow::logic::Diagnostic) -> FrameworkError {
    invalid(serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()))
}
fn invalid(message: impl Into<String>) -> FrameworkError {
    FrameworkError::InvalidOperation(message.into())
}

impl WorkflowsModule {
    pub async fn register_logic_resource(
        &self,
        source: &str,
        tools: &[RuntimeToolMetadata],
        commit: bool,
    ) -> Result<Value> {
        self.accept_source(source, tools, commit, None).await
    }

    async fn accept_source(
        &self,
        source: &str,
        tools: &[RuntimeToolMetadata],
        commit: bool,
        restore_path: Option<&Path>,
    ) -> Result<Value> {
        let resource = parse(source)?;
        let restoring = restore_path.is_some();
        let id = resource.id.clone();
        let result = {
            let _guard = self.reference_lock.lock();
            let summary = self
                .list_workflow_catalog(None)?
                .into_iter()
                .find(|entry| entry.id == id);
            let current = summary
                .as_ref()
                .map(|_| self.read_workflow_resource(&id))
                .transpose()?;
            if let Some(current) = &current {
                if restoring {
                    return Err(invalid(format!("Duplicate Workflow source identity: {id}")));
                }
                if current.summary.revision != resource.revision {
                    return Err(invalid(format!(
                        "Workflow revision conflict: expected {}, current {}",
                        resource.revision, current.summary.revision
                    )));
                }
                if current.summary.kind != resource.state {
                    return Err(invalid(
                        "Registration cannot change existing state; use workflow.promote",
                    ));
                }
            } else if !restoring && resource.revision != 1 {
                return Err(invalid("A new resource must have revision 1"));
            }
            let path = restore_path
                .map(Path::to_path_buf)
                .or_else(|| {
                    current
                        .as_ref()
                        .and_then(|current| current.resource_path.as_ref())
                        .map(PathBuf::from)
                })
                .or_else(|| {
                    self.get_registry()
                        .ok()?
                        .into_iter()
                        .find(|entry| entry.metadata.id == id)
                        .map(|entry| PathBuf::from(entry.file_path))
                })
                .unwrap_or_else(|| self.workflows_dir.join(format!("{id}.workflow.json")));
            self.ensure_owned_resource_path(&path)?;
            if commit && !restoring && current.is_none() && path.exists() {
                return Err(invalid(format!(
                    "Resource file already exists: {}",
                    path.display()
                )));
            }
            let name = resource.metadata.name.trim();
            self.ensure_name_available(name, Some(&id))?;
            let previous =
                current.as_ref().and_then(|current| {
                    current.blueprint.clone().zip(current.script.clone()).map(
                        |(blueprint, script)| revision::PreparedRevision {
                            resource_source: current.resource_source.clone(),
                            resource_path: current.resource_path.clone(),
                            logic_source: current.logic_source.clone(),
                            logic: current.logic.clone(),
                            blueprint,
                            script,
                            source: current.source.clone().unwrap_or_else(|| "legacy".into()),
                            validator_version: revision::VALIDATOR_VERSION.into(),
                        },
                    )
                });
            let mut effective = tools.to_vec();
            effective.extend(self.workflow_reference_tools_unlocked()?);
            let mut prepared = revision::prepare(
                revision::WorkflowChange::Logic(resource.logic),
                previous.as_ref(),
                &effective,
            )?;
            prepared.blueprint.metadata.id = id.clone();
            prepared.blueprint.metadata.name = name.into();
            prepared.blueprint.metadata.description = resource.metadata.description;
            prepared.blueprint.metadata.extensions = resource.metadata.extensions;
            if !restoring {
                self.validate_prepared_source(&prepared)?;
            }
            let revision = if commit && current.is_some() {
                resource
                    .revision
                    .checked_add(1)
                    .filter(|value| *value <= 9_007_199_254_740_991)
                    .ok_or_else(|| invalid("Revision overflow"))?
            } else {
                resource.revision
            };
            prepared.resource_source = Some(stamp(source, resource.state, revision)?);
            prepared.resource_path = Some(path.to_string_lossy().into_owned());
            if commit {
                match resource.state {
                    WorkflowResourceKind::Draft => {
                        let mut drafts = self.get_draft_registry()?;
                        drafts.retain(|entry| entry.id != id);
                        drafts.push(DraftWorkflowEntry {
                            id: id.clone(),
                            name: name.into(),
                            description: prepared.blueprint.metadata.description.clone(),
                            revision,
                            resource_source: prepared.resource_source.clone(),
                            resource_path: prepared.resource_path.clone(),
                            logic_source: None,
                            logic: prepared.logic.clone(),
                            source: Some("logic".into()),
                            validator_version: Some(prepared.validator_version.clone()),
                            script: Some(prepared.script.clone()),
                            blueprint: Some(prepared.blueprint.clone()),
                            validation: super::WorkflowValidation::valid(),
                        });
                        self.commit_source_registry(
                            &path,
                            prepared.resource_source.as_ref().unwrap(),
                            DRAFT_REGISTRY,
                            &drafts,
                            !restoring,
                        )?;
                    }
                    WorkflowResourceKind::Registered => {
                        self.persist_logic_registered_unlocked(
                            &prepared,
                            revision,
                            current.is_some(),
                            !restoring,
                        )?;
                    }
                }
            }
            json!({"id":id,"state":resource.state,"revision":revision,"valid":true,"committed":commit,
                "metadata":{"name":name,"description":prepared.blueprint.metadata.description,"extensions":prepared.blueprint.metadata.extensions},
                "inputs":prepared.blueprint.metadata.inputs,"returns":prepared.blueprint.metadata.outputs})
        };
        if commit && !restoring {
            self.publish_event(super::WORKFLOW_RESOURCE_CHANGED_EVENT,json!({
                "schema":"agent-runtime-workflow-change/v1","workflow_id":id,"operation":"registered","kind":result["state"],"revision":result["revision"]
            })).await;
        }
        Ok(result)
    }

    fn validate_prepared_source(&self, prepared: &revision::PreparedRevision) -> Result<()> {
        self.validate_references_unlocked(&prepared.blueprint)?;
        let frozen = self.freeze_references_unlocked(&prepared.blueprint)?;
        crate::workflow::BlueprintLoader::with_workflows(std::sync::Arc::new(frozen))
            .load_from_blueprint_json(prepared.blueprint.clone(), &self.create_run_context())?;
        Ok(())
    }

    pub(crate) fn persist_logic_registered_unlocked(
        &self,
        prepared: &revision::PreparedRevision,
        revision: u64,
        update: bool,
        write: bool,
    ) -> Result<super::executor::RegisteredWorkflow> {
        let mut registry = self.get_registry()?;
        let id = &prepared.blueprint.metadata.id;
        let existing = registry.iter().find(|entry| entry.metadata.id == *id);
        if existing.is_some() != update {
            return Err(invalid("Workflow identity changed during registration"));
        }
        let path = existing
            .map(|entry| PathBuf::from(&entry.file_path))
            .or_else(|| prepared.resource_path.as_ref().map(PathBuf::from))
            .unwrap_or_else(|| self.workflows_dir.join(format!("{id}.workflow.json")));
        self.ensure_owned_resource_path(&path)?;
        self.ensure_name_available(&prepared.blueprint.metadata.name, Some(id))?;
        let mut accepted = prepared.clone();
        accepted.resource_source = Some(stamp(
            prepared
                .resource_source
                .as_deref()
                .ok_or_else(|| invalid("Missing resource source"))?,
            WorkflowResourceKind::Registered,
            revision,
        )?);
        accepted.resource_path = Some(path.to_string_lossy().into_owned());
        let entry = BlueprintEntry {
            accepted: Some(accepted.clone()),
            metadata: prepared.blueprint.metadata.clone(),
            file_path: path.to_string_lossy().into_owned(),
            key: Self::make_key(&prepared.blueprint.metadata),
            source: WorkflowSource::Local,
            revision,
        };
        registry.retain(|candidate| candidate.metadata.id != *id);
        registry.push(entry.clone());
        self.commit_source_registry(
            &path,
            accepted.resource_source.as_ref().unwrap(),
            REGISTRY,
            &registry,
            write,
        )?;
        Ok(Self::registered_workflow(entry))
    }

    fn commit_source_registry<T: Serialize>(
        &self,
        path: &Path,
        source: &str,
        key: &str,
        value: &T,
        write: bool,
    ) -> Result<()> {
        let previous = if write {
            match std::fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(FrameworkError::SystemError(error.to_string())),
            }
        } else {
            None
        };
        if write {
            revision::atomic_write(path, source.as_bytes())
                .map_err(|error| FrameworkError::SystemError(error.to_string()))?;
        }
        if let Err(error) = self.unit.set_resource(key, value, None) {
            if write {
                match previous {
                    Some(bytes) => revision::atomic_write(path, &bytes),
                    None => std::fs::remove_file(path),
                }
                .map_err(|rollback| {
                    invalid(format!(
                        "Registry commit failed: {error}; rollback failed: {rollback}"
                    ))
                })?;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Restore sources in dependency order after RPC node contracts are installed.
    /// Startup performs no writes or execution; failures leave authored files intact.
    pub async fn restore_local_sources(&self, tools: &[RuntimeToolMetadata]) -> Result<usize> {
        let mut pending = Vec::new();
        let mut ids = HashSet::new();
        for entry in
            std::fs::read_dir(&self.workflows_dir).map_err(|error| invalid(error.to_string()))?
        {
            let path = entry.map_err(|error| invalid(error.to_string()))?.path();
            if !path.is_file()
                || !matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("json" | "workflow")
                )
            {
                continue;
            }
            let source =
                std::fs::read_to_string(&path).map_err(|error| invalid(error.to_string()))?;
            if resource_source_kind(&source)?.is_none() {
                tracing::warn!(path=%path.display(), "Legacy workflow source skipped; only orbit.workflow.resource/v1 is automatically loaded");
                continue;
            }
            let doc = parse(&source)?;
            if !ids.insert(doc.id.clone()) {
                return Err(invalid(format!(
                    "Duplicate Workflow source identity: {}",
                    doc.id
                )));
            }
            let mut deps = HashSet::new();
            dependencies(&doc.logic.steps, &mut deps);
            pending.push((doc.id, path, source, deps));
        }
        pending.sort_by(|a, b| a.0.cmp(&b.0));
        let count = pending.len();
        while !pending.is_empty() {
            let next = pending
                .iter()
                .position(|item| {
                    item.3
                        .iter()
                        .all(|dep| !pending.iter().any(|other| &other.0 == dep))
                })
                .ok_or_else(|| {
                    invalid(format!(
                        "Cyclic Workflow source references: {:?}",
                        pending.iter().map(|item| &item.0).collect::<Vec<_>>()
                    ))
                })?;
            let (_, path, source, _) = pending.remove(next);
            self.accept_source(&source, tools, true, Some(&path))
                .await?;
        }
        {
            let _guard = self.reference_lock.lock();
            self.validate_reference_catalog_unlocked()?;
            for accepted in self
                .get_registry()?
                .into_iter()
                .filter_map(|entry| entry.accepted)
            {
                self.validate_prepared_source(&accepted)?;
            }
            for draft in self.get_draft_registry()? {
                if let Some(blueprint) = draft.blueprint {
                    let frozen = self.freeze_references_unlocked(&blueprint)?;
                    crate::workflow::BlueprintLoader::with_workflows(std::sync::Arc::new(frozen))
                        .load_from_blueprint_json(blueprint, &self.create_run_context())?;
                }
            }
        }
        Ok(count)
    }
}

fn dependencies(steps: &[crate::workflow::logic::LogicStep], deps: &mut HashSet<String>) {
    use crate::workflow::logic::Operation;
    for step in steps {
        match &step.operation {
            Operation::Call { tool, .. } => {
                if let Some(id) = tool.strip_prefix(super::reference::PREFIX) {
                    deps.insert(id.into());
                }
            }
            Operation::If {
                then, otherwise, ..
            } => {
                dependencies(then, deps);
                dependencies(otherwise, deps);
            }
            Operation::ForEach { body, .. } => dependencies(body, deps),
            Operation::Switch { cases, default, .. } => {
                for case in cases {
                    dependencies(&case.body, deps);
                }
                dependencies(default, deps);
            }
            Operation::Set { .. } => {}
        }
    }
}
