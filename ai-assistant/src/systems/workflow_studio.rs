//! Workflow Studio-only reference lookup.

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::cache::CacheExt;
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use serde_json::Value;

use crate::skills::systems::mgr;

const REFERENCE_SKILL_NAMES_KEY: &str = "workflow_studio.reference_skill_names";

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

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(error) => return Ok(error),
        };
        let query = match args.safe_require("query") {
            Ok(query) => query,
            Err(error) => return Ok(error),
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
            format!("No parent skill references matched '{query}'.")
        } else {
            let mut lines = vec![format!(
                "Found {} runtime skill reference(s) for '{query}':",
                matches.len()
            )];
            for hit in &matches {
                let skill = hit.get("skill").and_then(Value::as_str).unwrap_or("");
                let heading = hit.get("heading").and_then(Value::as_str).unwrap_or("");
                let paragraph = hit.get("paragraph").and_then(Value::as_str).unwrap_or("");
                if heading.is_empty() {
                    lines.push(format!("- {skill}: {paragraph}"));
                } else {
                    lines.push(format!("- {skill} / {heading}: {paragraph}"));
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

async fn reference_skill_names(ctx: &Context) -> Result<Vec<String>, FrameworkError> {
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
) -> Result<Vec<Value>, FrameworkError> {
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
                "path": skill.base_path.as_ref().map(|path| path.join("SKILL.md").to_string_lossy().to_string()).unwrap_or_default(),
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
                "path": skill.base_path.as_ref().map(|path| path.join("SKILL.md").to_string_lossy().to_string()).unwrap_or_default(),
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
        .filter(|paragraph| !paragraph.is_empty())
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
