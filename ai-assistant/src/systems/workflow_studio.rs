//! Workflow Studio editor-only AI tools.

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::event::BaseEvent;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::skills::systems::mgr;

const HOST_DYNAMIC_SNAPSHOTS_KEY: &str = "host_dynamic_snapshots";
const REFERENCE_SKILL_NAMES_KEY: &str = "workflow_studio.reference_skill_names";

fn next_revision() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

async fn publish_studio_draft_update(
    ctx: &Context,
    script_text: String,
    blueprint: corework::workflow::blueprint_json::BlueprintJson,
    origin: Option<Value>,
    replace_draft: bool,
) -> std::result::Result<AIOutput, FrameworkError> {
    let revision = next_revision();
    let workflow_name = blueprint.metadata.name.clone();
    let node_count = blueprint.nodes.len();
    let connection_count = blueprint.connections.len();
    let conversation_id = ctx.conversation_id.clone().filter(|id| !id.is_empty());
    let intent_script = script_text.clone();
    let intent_blueprint = blueprint.clone();
    let intent_origin = origin.clone();
    let payload = serde_json::json!({
        "schema": "workflow-studio-frontend-intent/v1",
        "type": crate::events::types::WORKFLOW_STUDIO_DRAFT_UPDATE,
        "conversation_id": conversation_id,
        "revision": revision,
        "script": script_text,
        "blueprint": blueprint,
        "origin": origin,
        "replace_draft": replace_draft,
        "payload": {
            "revision": revision,
            "script": intent_script,
            "blueprint": intent_blueprint,
            "origin": intent_origin,
            "replace_draft": replace_draft,
        },
    });
    let mut event = BaseEvent::new(
        crate::events::types::WORKFLOW_STUDIO_DRAFT_UPDATE,
        payload.clone(),
    );
    if let Some(conversation_id) = conversation_id {
        event = event.with_conversation_id(conversation_id);
    }
    ctx.event_bus.publish(event).await?;
    let mut snapshots = ctx
        .cache
        .get::<HashMap<String, String>>(HOST_DYNAMIC_SNAPSHOTS_KEY)
        .await?
        .unwrap_or_default();
    snapshots.insert(
        "workflow_studio.current_draft".to_string(),
        script_text.clone(),
    );
    ctx.cache
        .set(HOST_DYNAMIC_SNAPSHOTS_KEY, &snapshots, None)
        .await?;
    Ok(AIOutput::success(
        serde_json::json!({
            "schema": "workflow-studio-draft-update-result/v1",
            "revision": revision,
            "workflow_name": workflow_name,
            "nodes": node_count,
            "connections": connection_count,
            "origin": payload.get("origin").cloned().unwrap_or(Value::Null),
            "replace_draft": replace_draft,
        }),
        format!(
            "Compiled and published a Workflow Studio draft_update event for '{}' ({} nodes, {} connections). Do not claim the Studio frontend has applied it until workflow_studio.current_draft is visible in the frontend snapshot. Do not repeat the script as a plain assistant message.",
            workflow_name, node_count, connection_count
        ),
    ))
}

#[define_operation(
    name = "updateCurrentWorkflowDraft",
    display_name = "Update Current Workflow Draft",
    category = "Workflow Studio",
    description = "Compile and publish an updated Workflow Studio draft_update event. On compile failure, returns a syntax error and does not update the draft. On success, a runtime event carries the complete BlueprintJson for the Studio frontend to apply; this does not save a local workflow file. Do not report that the frontend has applied the draft until workflow_studio.current_draft appears in the frontend snapshot.",
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
        let blueprint = match corework::workflow::chain_compiler_v2::compile_chain_v2(&script_text)
        {
            Ok(v) => v,
            Err(e) => {
                return Ok(AIOutput::error(
                    400,
                    format!("script compile failed at line {}: {}", e.line, e.message),
                ))
            }
        };
        publish_studio_draft_update(ctx, script_text, blueprint, None, false).await
    }

    fn name(&self) -> &str {
        "updateCurrentWorkflowDraft"
    }
}

#[define_operation(
    name = "openWorkflowDraft",
    display_name = "Open Workflow Draft",
    category = "Workflow Studio",
    description = "Publish a Workflow Studio draft_update event for a persisted workflow copy by exact file name, or for a new blank Studio draft when file_name is omitted or empty. The frontend applies the unsaved web draft asynchronously and this does not save a workflow file. Do not report that the frontend has applied the draft until workflow_studio.current_draft appears in the frontend snapshot.",
    system_only,
    params {
        file_name: "String@Exact file_name from workflow_studio.workflows. Optional; empty opens a new blank draft.",
        draft_name: "String@Name for a new blank draft when file_name is empty. Optional."
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
        let target = args.get("file_name").map(str::trim).unwrap_or("");

        let (blueprint, origin) = if target.is_empty() {
            let draft_name = args
                .get("draft_name")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Untitled draft");
            let mut blueprint = corework::workflow::chain_compiler_v2::compile_chain_v2(
                "input\nreturn",
            )
            .map_err(|e| {
                FrameworkError::InvalidData(format!(
                    "compile blank Studio draft failed at line {}: {}",
                    e.line, e.message
                ))
            })?;
            blueprint.metadata.name = draft_name.to_string();
            blueprint.metadata.id = draft_name.to_string();
            (
                blueprint,
                serde_json::json!({
                    "kind": "scratch",
                    "draft_id": format!("scratch_{}", next_revision())
                }),
            )
        } else {
            let path = match find_visible_workflow_path(ctx, target).await? {
                Some(path) => path,
                None => {
                    return Ok(AIOutput::error(
                        404,
                        format!("workflow not found in current Studio list: {target}"),
                    ))
                }
            };
            let blueprint =
                match corework::workflow::blueprint_json::BlueprintJson::from_workflow_file(&path) {
                    Ok(value) => value,
                    Err(e) => return Ok(AIOutput::error(400, e)),
                };
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_string();
            (
                blueprint,
                serde_json::json!({
                    "kind": "workflow_copy",
                    "file_name": file_name,
                    "path": path.to_string_lossy(),
                }),
            )
        };
        let script = match corework::workflow::chain_decompiler::decompile_chain(&blueprint) {
            Ok(value) => value,
            Err(e) => return Ok(AIOutput::error(400, e.to_string())),
        };
        publish_studio_draft_update(ctx, script, blueprint, Some(origin), true).await
    }

    fn name(&self) -> &str {
        "openWorkflowDraft"
    }
}

async fn find_visible_workflow_path(
    ctx: &Context,
    file_name: &str,
) -> std::result::Result<Option<PathBuf>, FrameworkError> {
    let file_path = Path::new(file_name);
    if file_path.file_name().and_then(|value| value.to_str()) != Some(file_name)
        || !corework::workflow::blueprint_json::BlueprintJson::is_workflow_file_path(file_path)
    {
        return Ok(None);
    }

    let snapshots = ctx
        .cache
        .get::<HashMap<String, String>>(HOST_DYNAMIC_SNAPSHOTS_KEY)
        .await?
        .unwrap_or_default();
    let is_visible = snapshots
        .get("workflow_studio.workflows")
        .and_then(|snapshot| serde_json::from_str::<Value>(snapshot).ok())
        .is_some_and(|snapshot| workflow_file_name_is_visible(&snapshot, file_name));
    if !is_visible {
        return Ok(None);
    }

    if let Some(editor_context) = ctx
        .cache
        .get::<String>("workflow_studio.editor_context")
        .await?
    {
        if let Ok(value) = serde_json::from_str::<Value>(&editor_context) {
            if let Some(workflows_dir) = value.get("workflows_dir").and_then(Value::as_str) {
                let candidate = Path::new(workflows_dir).join(file_name);
                if candidate.exists()
                    && corework::workflow::blueprint_json::BlueprintJson::is_workflow_file_path(
                        &candidate,
                    )
                {
                    return Ok(Some(candidate));
                }
            }
        }
    }

    Ok(None)
}

fn workflow_file_name_is_visible(snapshot: &Value, file_name: &str) -> bool {
    snapshot
        .get("workflows")
        .and_then(Value::as_array)
        .is_some_and(|workflows| {
            workflows
                .iter()
                .any(|item| item.get("file_name").and_then(Value::as_str) == Some(file_name))
        })
}

#[define_operation(
    name = "searchSkillRefs",
    display_name = "Search Skill References",
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
    use corework::event::{BaseEvent, EventBus, EventHandler};
    use corework::event_line::EventLinePolicy;
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::world::FrameworkState;
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
    async fn draft_update_identity_comes_from_call_context() {
        let framework = FrameworkState::initialize().unwrap();
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            framework,
            "studio-scope",
        ));
        let event_line = unit
            .create_event_line("studio", EventLinePolicy::private())
            .unwrap();
        unit.set_default_event_line("studio").unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        event_line
            .subscribe(
                crate::events::types::WORKFLOW_STUDIO_DRAFT_UPDATE.to_string(),
                Arc::new(CaptureEvents {
                    events: Arc::clone(&captured),
                }),
            )
            .await
            .unwrap();
        let ctx = unit
            .create_context()
            .with_conversation_id("studio-conversation");
        let input = AIInput::from_args(HashMap::from([(
            "script".to_string(),
            "input text:String\nreturn result=$text".to_string(),
        )]));

        let output = UpdateCurrentWorkflowDraftSystem
            .execute(input, &ctx)
            .await
            .unwrap();
        assert_eq!(output.error_code, 0, "{}", output.to_ai);

        let event = captured
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("draft update event");
        assert_eq!(event.scope_id.as_deref(), Some("studio-scope"));
        assert_eq!(
            event.conversation_id.as_deref(),
            Some("studio-conversation")
        );
        assert_eq!(
            event.payload["schema"],
            "workflow-studio-frontend-intent/v1"
        );
        assert_eq!(
            event.payload["type"],
            crate::events::types::WORKFLOW_STUDIO_DRAFT_UPDATE
        );
        assert_eq!(event.payload["conversation_id"], "studio-conversation");
        assert!(event.payload.get("agent_id").is_none());
        assert!(event.payload.get("session_id").is_none());
        assert!(event.payload["blueprint"]["nodes"].is_array());
        assert!(event.payload["payload"]["blueprint"]["nodes"].is_array());
        assert!(event.payload.get("note").is_none());
        assert!(event.payload["payload"].get("note").is_none());
        let snapshots = ctx
            .cache
            .get::<HashMap<String, String>>(HOST_DYNAMIC_SNAPSHOTS_KEY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            snapshots.get("workflow_studio.current_draft"),
            Some(&"input text:String\nreturn result=$text".to_string())
        );
    }

    #[test]
    fn workflow_snapshot_matches_exact_file_name_only() {
        let snapshot = serde_json::json!({
            "schema": "workflow-studio-workflow-list-snapshot/v1",
            "workflows": [{
                "name": "Customer support",
                "file_name": "customer_support.workflow.json",
                "description": "Support workflow"
            }]
        });

        assert!(workflow_file_name_is_visible(
            &snapshot,
            "customer_support.workflow.json"
        ));
        assert!(!workflow_file_name_is_visible(
            &snapshot,
            "Customer support"
        ));
        assert!(!workflow_file_name_is_visible(
            &snapshot,
            "./customer_support.workflow.json"
        ));
    }

    #[test]
    fn studio_tools_are_registered_as_system_only_operations() {
        for name in [
            "openWorkflowDraft",
            "updateCurrentWorkflowDraft",
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
