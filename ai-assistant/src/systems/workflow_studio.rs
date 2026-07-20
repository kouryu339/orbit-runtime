//! Workflow Studio editor-only AI tools.

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use corework::workflow::workflows::{
    WorkflowEditorSession, WorkflowResourceKind, WorkflowValidation, WorkflowsModule,
};
use serde_json::Value;
use std::sync::Arc;

use crate::skills::systems::mgr;

const REFERENCE_SKILL_NAMES_KEY: &str = "workflow_studio.reference_skill_names";

fn editor_services(
    ctx: &Context,
) -> std::result::Result<(Arc<WorkflowsModule>, Arc<WorkflowEditorSession>), FrameworkError> {
    Ok((
        ctx.resolve_shared_component::<WorkflowsModule>()?,
        ctx.resolve_shared_component::<WorkflowEditorSession>()?,
    ))
}

#[define_operation(
    name = "updateCurrentWorkflowDraft",
    display_name = "使用脚本{script}更新当前工作流草稿",
    category = "Workflow Studio",
    description = "Compile and update the currently selected Workflow resource through the unified Draft/Registered catalog. Uses the selected revision for lost-update protection and publishes workflow.resource_changed on the global Workflow event line.",
    system_only,
    params {
        script: "String@Complete workflow script text for the desired current draft. 必填."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct UpdateCurrentWorkflowDraftSystem;

#[async_trait]
impl SystemOperation for UpdateCurrentWorkflowDraftSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let script = match args.safe_require("script") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let script_text = script.replace(r"\n", "\n").replace(r#"\""#, "\"");
        if script_text.trim().is_empty() {
            return Ok(AIOutput::error(400, "script must not be empty"));
        }
        let (workflows, session) = editor_services(ctx)?;
        let selection = match session.selection() {
            Some(selection) => selection,
            None => return Ok(AIOutput::error(400, "no Workflow resource is selected")),
        };
        let current = match workflows.read_workflow_resource(&selection.workflow_id) {
            Ok(current) => current,
            Err(error) => return Ok(AIOutput::error(404, error.to_string())),
        };
        let mut blueprint =
            match corework::workflow::chain_compiler_v2::compile_chain_v2_with_runtime_tools(
                &script_text,
                session.runtime_tools(),
            ) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(AIOutput::error(
                        400,
                        format!("script compile failed at line {}: {}", e.line, e.message),
                    ))
                }
            };
        blueprint.metadata.id = current.summary.id.clone();
        blueprint.metadata.name = current.summary.name.clone();
        blueprint.metadata.description = current.summary.description.clone();
        if let Some(previous) = current.blueprint.as_ref() {
            corework::workflow::workflows::preserve_workflow_blueprint_layout(
                previous,
                &mut blueprint,
            );
        }
        let updated = match current.summary.kind {
            WorkflowResourceKind::Draft => {
                workflows
                    .update_draft_resource(
                        &current.summary.id,
                        Some(selection.revision),
                        &current.summary.name,
                        &current.summary.description,
                        Some(script_text),
                        Some(blueprint),
                        WorkflowValidation::valid(),
                    )
                    .await
            }
            WorkflowResourceKind::Registered => {
                workflows
                    .update_registered_resource(&blueprint, Some(selection.revision))
                    .await
            }
        };
        let updated = match updated {
            Ok(updated) => updated,
            Err(error) => return Ok(AIOutput::error(409, error.to_string())),
        };
        session.select(updated.summary.id.clone(), updated.summary.revision);
        Ok(AIOutput::success(
            serde_json::to_value(&updated)
                .map_err(|error| FrameworkError::SerializationError(error))?,
            format!(
                "Updated Workflow '{}' at revision {} through the unified catalog.",
                updated.summary.name, updated.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "updateCurrentWorkflowDraft"
    }
}

#[define_operation(
    name = "openWorkflowDraft",
    display_name = "打开工作流{workflow_id}并返回草稿{draft_name}",
    category = "Workflow Studio",
    description = "Select an existing Workflow resource by stable id, or create and select a new blank Draft when workflow_id is omitted. The selected resource remains owned by the unified Workflow catalog.",
    system_only,
    params {
        workflow_id: "String@Stable id from workflow.list. Optional; empty creates a new blank Draft.",
        draft_name: "String@Globally unique name for a new blank Draft. Optional."
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct OpenWorkflowDraftSystem;

#[async_trait]
impl SystemOperation for OpenWorkflowDraftSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let target = args.get("workflow_id").map(str::trim).unwrap_or("");
        let (workflows, session) = editor_services(ctx)?;
        let workflow = if target.is_empty() {
            let draft_name = args
                .get("draft_name")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Untitled draft {}", uuid::Uuid::new_v4().simple()));
            let mut blueprint = corework::workflow::chain_compiler_v2::compile_chain_v2(
                "input\nreturn",
            )
            .map_err(|e| {
                FrameworkError::InvalidData(format!(
                    "compile blank Studio draft failed at line {}: {}",
                    e.line, e.message
                ))
            })?;
            blueprint.metadata.name = draft_name.clone();
            match workflows
                .create_draft_resource(
                    None,
                    &draft_name,
                    "",
                    Some("input\nreturn".to_string()),
                    Some(blueprint),
                    WorkflowValidation::valid(),
                )
                .await
            {
                Ok(workflow) => workflow,
                Err(error) => return Ok(AIOutput::error(409, error.to_string())),
            }
        } else {
            match workflows.read_workflow_resource(target) {
                Ok(workflow) => workflow,
                Err(error) => return Ok(AIOutput::error(404, error.to_string())),
            }
        };
        session.select(workflow.summary.id.clone(), workflow.summary.revision);
        Ok(AIOutput::success(
            serde_json::to_value(&workflow)
                .map_err(|error| FrameworkError::SerializationError(error))?,
            format!(
                "Selected Workflow '{}' ({:?}, revision {}).",
                workflow.summary.name, workflow.summary.kind, workflow.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "openWorkflowDraft"
    }
}

#[define_operation(
    name = "registerCurrentWorkflowDraft",
    display_name = "将当前工作流草稿注册为{name}",
    category = "Workflow Studio",
    description = "Promote the currently selected valid Draft to a trusted Registered Workflow using its selected revision. Publishes workflow.resource_changed on the global Workflow event line.",
    system_only,
    params {
        name: "String@Optional globally unique Registered display name."
    },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct RegisterCurrentWorkflowDraftSystem;

#[async_trait]
impl SystemOperation for RegisterCurrentWorkflowDraftSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let (workflows, session) = editor_services(ctx)?;
        let selection = match session.selection() {
            Some(selection) => selection,
            None => return Ok(AIOutput::error(400, "no Workflow Draft is selected")),
        };
        let registered = match workflows
            .register_draft_resource(
                &selection.workflow_id,
                Some(selection.revision),
                args.get("name")
                    .map(str::trim)
                    .filter(|name| !name.is_empty()),
            )
            .await
        {
            Ok(registered) => registered,
            Err(error) => return Ok(AIOutput::error(409, error.to_string())),
        };
        session.select(registered.summary.id.clone(), registered.summary.revision);
        Ok(AIOutput::success(
            serde_json::to_value(&registered).map_err(FrameworkError::SerializationError)?,
            format!(
                "Registered Workflow '{}' at revision {}.",
                registered.summary.name, registered.summary.revision
            ),
        ))
    }

    fn name(&self) -> &str {
        "registerCurrentWorkflowDraft"
    }
}

#[define_operation(
    name = "searchSkillRefs",
    display_name = "搜索技能引用{query}，最多返回{max_results}条上下文{context_paragraphs}",
    category = "Workflow Studio",
    description = "Search runtime reference skill documents for a tool name, workflow name, or business keyword.",
    system_only,
    params {
        query: "String@Keyword, tool name, workflow name, or policy phrase to search. 必填.",
        context_paragraphs: "Number@Number of paragraphs before/after each hit, default 2, max 5.",
        max_results: "Number@Maximum matches to return, default 8, max 32."
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct SearchSkillRefsSystem;

#[async_trait]
impl SystemOperation for SearchSkillRefsSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let query = match args.safe_require("query") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let context_paragraphs = args.get_i64_or("context_paragraphs", 2).clamp(0, 5) as usize;
        let max_results = args.get_i64_or("max_results", 8).clamp(1, 32) as usize;
        let skill_names = reference_skill_names(ctx).await?;
        if skill_names.is_empty() {
            return Ok(AIOutput::error(
                404,
                "No reference skill names are available in workflow_studio.reference_skill_names.",
            ));
        }

        let matches =
            load_and_search_reference_skills(&skill_names, &query, context_paragraphs, max_results)
                .await?;
        let to_ai = if matches.is_empty() {
            format!("No parent skill references matched '{}'.", query)
        } else {
            let mut lines = vec![format!(
                "Found {} runtime skill reference(s) for '{}':",
                matches.len(),
                query
            )];
            for hit in &matches {
                let skill = hit.get("skill").and_then(Value::as_str).unwrap_or("");
                let heading = hit.get("heading").and_then(Value::as_str).unwrap_or("");
                let paragraph = hit.get("paragraph").and_then(Value::as_str).unwrap_or("");
                if heading.is_empty() {
                    lines.push(format!("- {}: {}", skill, paragraph));
                } else {
                    lines.push(format!("- {} / {}: {}", skill, heading, paragraph));
                }
            }
            lines.join("\n")
        };
        Ok(AIOutput::success(
            serde_json::json!({
                "schema": "workflow-studio-skill-ref-search-result/v1",
                "query": query,
                "matches": matches,
            }),
            to_ai,
        ))
    }

    fn name(&self) -> &str {
        "searchSkillRefs"
    }
}

async fn reference_skill_names(ctx: &Context) -> std::result::Result<Vec<String>, FrameworkError> {
    Ok(ctx
        .cache
        .get::<Vec<String>>(REFERENCE_SKILL_NAMES_KEY)
        .await?
        .unwrap_or_default())
}

async fn load_and_search_reference_skills(
    skill_names: &[String],
    query: &str,
    context_paragraphs: usize,
    max_results: usize,
) -> std::result::Result<Vec<Value>, FrameworkError> {
    let mut manager = mgr().write().await;
    let refs: Vec<&str> = skill_names.iter().map(String::as_str).collect();
    let _ = manager.load_many(&refs).await;
    let query_lc = query.to_lowercase();
    let mut results = Vec::new();

    for skill_name in skill_names {
        let Some(skill) = manager.get(skill_name) else {
            continue;
        };
        let metadata_text = serde_json::to_string(&skill.metadata).unwrap_or_default();
        let metadata_matched = metadata_text.to_lowercase().contains(&query_lc);
        if !metadata_matched {
            continue;
        }
        let paragraphs = split_paragraphs(&skill.instructions);
        let mut body_matched = false;
        for (idx, paragraph) in paragraphs.iter().enumerate() {
            if !paragraph.to_lowercase().contains(&query_lc) {
                continue;
            }
            body_matched = true;
            results.push(serde_json::json!({
                "skill": skill.metadata.name,
                "path": skill.base_path.as_ref().map(|p| p.join("SKILL.md").to_string_lossy().to_string()).unwrap_or_default(),
                "metadata_matched": true,
                "body_matched": true,
                "heading": nearest_heading(&paragraphs, idx),
                "paragraph": paragraph,
                "before": context_text(&paragraphs, idx.saturating_sub(context_paragraphs), idx),
                "after": context_text(&paragraphs, idx + 1, (idx + 1 + context_paragraphs).min(paragraphs.len())),
                "note": Value::Null,
            }));
            if results.len() >= max_results {
                return Ok(results);
            }
        }
        if !body_matched {
            results.push(serde_json::json!({
                "skill": skill.metadata.name,
                "path": skill.base_path.as_ref().map(|p| p.join("SKILL.md").to_string_lossy().to_string()).unwrap_or_default(),
                "metadata_matched": true,
                "body_matched": false,
                "heading": Value::Null,
                "paragraph": "",
                "before": "",
                "after": "",
                "note": "skill metadata references the query, but body has no explicit paragraph match",
            }));
            if results.len() >= max_results {
                return Ok(results);
            }
        }
    }
    Ok(results)
}

fn split_paragraphs(body: &str) -> Vec<String> {
    body.split("\n\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn nearest_heading(paragraphs: &[String], idx: usize) -> Option<String> {
    paragraphs[..idx.min(paragraphs.len())]
        .iter()
        .rev()
        .find(|paragraph| paragraph.trim_start().starts_with('#'))
        .cloned()
}

fn context_text(paragraphs: &[String], start: usize, end: usize) -> String {
    paragraphs.get(start..end).unwrap_or(&[]).join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::ai_system::AISystemFactory;
    use corework::event::{BaseEvent, EventBus, EventHandler, InMemoryEventBus};
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::world::FrameworkState;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct CaptureEvents {
        events: Arc<Mutex<Vec<BaseEvent>>>,
    }

    #[async_trait]
    impl EventHandler for CaptureEvents {
        async fn handle(&self, event: &BaseEvent) -> corework::error::Result<()> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn editor_uses_catalog_and_publishes_global_workflow_events() {
        let framework = FrameworkState::initialize().unwrap();
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            "studio-scope",
        ));
        let event_bus = Arc::new(InMemoryEventBus::new());
        let captured = Arc::new(Mutex::new(Vec::new()));
        event_bus
            .subscribe(
                corework::workflow::workflows::WORKFLOW_RESOURCE_CHANGED_EVENT.to_string(),
                Arc::new(CaptureEvents {
                    events: Arc::clone(&captured),
                }),
            )
            .await
            .unwrap();
        event_bus
            .subscribe(
                corework::workflow::workflows::WORKFLOW_EXECUTION_COMPLETED_EVENT.to_string(),
                Arc::new(CaptureEvents {
                    events: Arc::clone(&captured),
                }),
            )
            .await
            .unwrap();
        let workflows_dir = std::env::temp_dir().join(format!(
            "ai-framework-workflow-editor-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let workflows = Arc::new(
            WorkflowsModule::new_with_event_bus(workflows_dir, event_bus.clone()).unwrap(),
        );
        let session = Arc::new(WorkflowEditorSession::new(Vec::new()));
        unit.attach_shared_component(Arc::clone(&workflows))
            .unwrap();
        unit.attach_shared_component(Arc::clone(&session)).unwrap();
        let ctx = unit.create_context();

        let opened = OpenWorkflowDraftSystem
            .execute(
                AIInput::from_args(HashMap::from([(
                    "draft_name".to_string(),
                    "Event test draft".to_string(),
                )])),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(opened.error_code, 0, "{}", opened.to_ai);
        let selected = session.selection().expect("selected draft");
        assert_eq!(selected.revision, 1);

        let input = AIInput::from_args(HashMap::from([(
            "script".to_string(),
            "input text:String\nreturn result=$text".to_string(),
        )]));

        let output = UpdateCurrentWorkflowDraftSystem
            .execute(input, &ctx)
            .await
            .unwrap();
        assert_eq!(output.error_code, 0, "{}", output.to_ai);
        let updated = workflows
            .read_workflow_resource(&selected.workflow_id)
            .unwrap();
        assert_eq!(updated.summary.revision, 2);
        assert_eq!(
            updated.script.as_deref(),
            Some("input text:String\nreturn result=$text")
        );

        let registered = RegisterCurrentWorkflowDraftSystem
            .execute(AIInput::from_args(HashMap::new()), &ctx)
            .await
            .unwrap();
        assert_eq!(registered.error_code, 0, "{}", registered.to_ai);
        let resource = workflows
            .read_workflow_resource(&selected.workflow_id)
            .unwrap();
        assert_eq!(resource.summary.kind, WorkflowResourceKind::Registered);
        assert_eq!(resource.summary.revision, 3);

        let tested = corework::workflow::workflows::script_tools::TestWorkflow
            .execute(
                AIInput::from_args(HashMap::from([
                    ("input.text".to_string(), "hello".to_string()),
                    ("trace".to_string(), "true".to_string()),
                ])),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(tested.error_code, 0, "{}", tested.to_ai);

        let events = captured.lock().unwrap().clone();
        assert_eq!(
            events.len(),
            4,
            "create, update, register, and test execution must emit"
        );
        assert_eq!(events[0].payload["operation"], "created");
        assert_eq!(events[1].payload["operation"], "updated");
        assert_eq!(events[2].payload["operation"], "registered");
        assert!(events[..3]
            .iter()
            .all(|event| event.event_type == "workflow.resource_changed"));
        assert_eq!(events[3].event_type, "workflow.execution_completed");
        assert_eq!(events[3].payload["status"], "succeeded");
        for event in events {
            assert_eq!(event.payload["event_line"], "workflow");
            assert_eq!(event.payload["workflow_id"], selected.workflow_id);
            assert!(event.conversation_id.is_none());
        }
    }

    #[test]
    fn studio_tools_are_registered_as_system_only_operations() {
        for name in [
            "openWorkflowDraft",
            "updateCurrentWorkflowDraft",
            "registerCurrentWorkflowDraft",
            "searchSkillRefs",
        ] {
            let metadata = &inventory::iter::<AISystemFactory>()
                .find(|factory| factory.metadata.name == name)
                .unwrap_or_else(|| panic!("missing Studio operation metadata for {name}"))
                .metadata;
            assert_eq!(metadata.tool_kind, "local");
        }

        let update = &inventory::iter::<AISystemFactory>()
            .find(|factory| factory.metadata.name == "updateCurrentWorkflowDraft")
            .unwrap()
            .metadata;
        assert!(update
            .parameters
            .iter()
            .any(|parameter| parameter.name == "script" && parameter.required));
    }
}
