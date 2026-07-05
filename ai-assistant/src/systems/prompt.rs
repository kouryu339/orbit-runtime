use async_trait::async_trait;
use corework::ai_system::AISystemFactory;
use corework::buns_system;
use corework::cache::CacheExt;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::rpc_tool::RuntimeToolMetadata;
use corework::system::SystemOperation;
use serde::{Deserialize, Serialize};

use crate::context::{AssistantContext, Message};
use crate::skills::systems::mgr;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
struct PromptSection {
    body: String,
}

impl PromptSection {
    fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }

    fn from_optional(body: &str) -> Option<Self> {
        if body.is_empty() {
            None
        } else {
            Some(Self::new(body))
        }
    }
}

fn compose_prompt_sections(sections: Vec<PromptSection>) -> String {
    sections
        .into_iter()
        .map(|section| section.body)
        .filter(|body| !body.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ============================================================================
// Environment context
// ============================================================================

/// Build a host environment context section for the system prompt.
pub(crate) fn build_env_context_section() -> String {
    use chrono::{Datelike, Local};

    let now = Local::now();
    let is_en = crate::prompt_assets::language() == "en";
    let date_str = now.format("%Y-%m-%d %H:%M").to_string();
    let weekday = match (is_en, now.weekday()) {
        (true, chrono::Weekday::Mon) => "Monday",
        (true, chrono::Weekday::Tue) => "Tuesday",
        (true, chrono::Weekday::Wed) => "Wednesday",
        (true, chrono::Weekday::Thu) => "Thursday",
        (true, chrono::Weekday::Fri) => "Friday",
        (true, chrono::Weekday::Sat) => "Saturday",
        (true, chrono::Weekday::Sun) => "Sunday",
        (false, chrono::Weekday::Mon) => "星期一",
        (false, chrono::Weekday::Tue) => "星期二",
        (false, chrono::Weekday::Wed) => "星期三",
        (false, chrono::Weekday::Thu) => "星期四",
        (false, chrono::Weekday::Fri) => "星期五",
        (false, chrono::Weekday::Sat) => "星期六",
        (false, chrono::Weekday::Sun) => "星期日",
    };

    let username = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string());
    let userprofile =
        std::env::var("USERPROFILE").unwrap_or_else(|_| format!("C:/Users/{}", username));

    let desktop = format!("{}/Desktop", userprofile);
    let documents = std::env::var("USERPROFILE")
        .map(|p| format!("{}/Documents", p))
        .unwrap_or_else(|_| format!("{}/Documents", userprofile));
    let downloads = format!("{}/Downloads", userprofile);

    let onedrive = std::env::var("OneDrive")
        .ok()
        .or_else(|| std::env::var("OneDriveConsumer").ok());

    let onedrive_line = onedrive
        .map(|od| format!("- **OneDrive**: `{}`", od))
        .unwrap_or_default();

    crate::prompt_assets::template("env_context.md")
        .replace("{{CURRENT_TIME}}", &date_str)
        .replace("{{WEEKDAY}}", weekday)
        .replace("{{USERNAME}}", &username)
        .replace("{{USERPROFILE}}", &userprofile)
        .replace("{{DESKTOP}}", &desktop)
        .replace("{{DOCUMENTS}}", &documents)
        .replace("{{DOWNLOADS}}", &downloads)
        .replace("{{ONEDRIVE_LINE}}", &onedrive_line)
        .trim()
        .to_string()
}

/// System names used by prompt systems.
pub mod system_names {
    pub const BUILD_SYSTEM_PROMPT: &str = "BuildSystemPrompt";
    pub const COMPOSE_MESSAGES: &str = "ComposeMessages";
}

// ============================================================================
// ============================================================================

/// Default maximum history messages.
const DEFAULT_MAX_HISTORY_MESSAGES: usize = 200;

// ============================================================================

/// Read state instructions from system skills, with a hard-coded fallback.
pub(crate) fn state_instruction(state: &str) -> String {
    if let Some(mgr_arc) = crate::skills::systems::SKILL_MANAGER.get() {
        if let Ok(guard) = mgr_arc.try_read() {
            if let Some(text) = guard.get_state_instruction(state) {
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    fallback_state_instruction(state)
}

fn fallback_state_instruction(state: &str) -> String {
    use crate::state_machine::states;
    let filename = match state {
        states::THINKING => "state_thinking.md",
        states::SAYING => "state_saying.md",
        states::EXECUTING => "state_executing.md",
        _ => "state_default.md",
    };
    crate::prompt_assets::template(filename).trim().to_string()
}

// ============================================================================
// Tool description formatting
// ============================================================================

/// Format page structure snapshots as a prompt section.
#[cfg(test)]
pub(crate) fn format_page_structures_section(
    structures: &HashMap<String, String>,
    _page_overview: &[serde_json::Value],
) -> String {
    if structures.is_empty() {
        return String::new();
    }

    let mut output = String::from("## Current Page Structure\n\n");

    output.push_str("The following page snapshots are injected for this agent. Only operate on a page_id shown in a snapshot; open a new owned page when no suitable page is available.\n");
    output.push_str(
        "Diff markers follow the usual meaning: `+` added, `-` removed, `~` changed.\n\n",
    );

    for (page_id, snapshot) in structures {
        output.push_str(&format!("### Page ID: {}\n", page_id));
        output.push_str(snapshot);
        output.push('\n');
        output.push('\n');
    }

    output
}

/// `snapshots` is host-published `HashMap<field_name, text>`. This function
/// injects text only; field names are update keys and remain hidden from the model.
/// Returns an empty string when no host dynamic text is present.
pub(crate) fn format_host_dynamic_snapshots_section(snapshots: &HashMap<String, String>) -> String {
    if snapshots.is_empty() {
        return String::new();
    }
    let sorted: std::collections::BTreeMap<&str, &str> = snapshots
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let mut output = String::new();
    for (_key, text) in &sorted {
        output.push_str(text);
        output.push_str("\n\n");
    }
    output
}

pub(crate) fn format_immutable_cache_entries_section(entries: &BTreeMap<String, String>) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    for text in entries.values() {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(text);
    }
    output
}

/// Format tool metadata as prompt text for EXEC line-protocol usage.
pub(crate) fn format_tools_section(tool_names: &[String]) -> String {
    if tool_names.is_empty() {
        return String::new();
    }

    let mut output = crate::prompt_assets::template("available_tools_header.md")
        .trim()
        .to_string();
    output.push_str("\n\n");
    output.push_str(crate::prompt_assets::template("tools_preamble.md").trim());
    output.push_str("\n\n");
    output.push_str(crate::prompt_assets::template("tool_parameter_intro.md").trim());
    output.push_str("\n\n");

    let all_factories: Vec<&AISystemFactory> =
        inventory::iter::<AISystemFactory>.into_iter().collect();

    for name in tool_names {
        if let Some(factory) = all_factories
            .iter()
            .find(|f| f.metadata.name == name.as_str())
        {
            let meta = &factory.metadata;

            let mut tags = Vec::new();
            if meta.readonly {
                tags.push(
                    crate::prompt_assets::template("tool_tag_readonly.md")
                        .trim()
                        .to_string(),
                );
            }
            if meta.destructive {
                tags.push(
                    crate::prompt_assets::template("tool_tag_destructive.md")
                        .trim()
                        .to_string(),
                );
            }
            if !meta.idempotent {
                tags.push(
                    crate::prompt_assets::template("tool_tag_non_idempotent.md")
                        .trim()
                        .to_string(),
                );
            }
            let tags_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join("/"))
            };

            output.push_str(&format!("### {}{}\n", meta.name, tags_str));

            if !meta.outputs.is_empty() {
                output.push_str(crate::prompt_assets::template("tool_outputs_label.md").trim());
                let pins: Vec<String> = meta
                    .outputs
                    .iter()
                    .map(|o| format!("`{}`", o.name))
                    .collect();
                output.push_str(&format!("{}\n", pins.join(", ")));
            }

            output.push_str(&format!("{}\n", meta.description));

            if !meta.parameters.is_empty() {
                output.push_str(crate::prompt_assets::template("tool_parameters_label.md").trim());
                output.push('\n');
                let required_label = crate::prompt_assets::template("tool_required_label.md")
                    .trim()
                    .to_string();
                let optional_label = crate::prompt_assets::template("tool_optional_label.md")
                    .trim()
                    .to_string();
                let default_label = crate::prompt_assets::template("tool_default_label.md")
                    .trim()
                    .to_string();
                for p in meta.parameters {
                    let req = if p.required {
                        required_label.as_str()
                    } else {
                        optional_label.as_str()
                    };
                    let default_str = p
                        .default_value
                        .map(|d| format!(", {}={}", default_label, d))
                        .unwrap_or_default();
                    output.push_str(&format!(
                        "  - `{}` ({}{}): {}\n",
                        p.name, req, default_str, p.description
                    ));
                }
            } else {
                output.push_str(crate::prompt_assets::template("tool_no_parameters.md").trim());
                output.push('\n');
            }

            if !meta.outputs.is_empty() {
                output
                    .push_str(crate::prompt_assets::template("tool_output_fields_label.md").trim());
                output.push('\n');
                for o in meta.outputs {
                    output.push_str(&format!(
                        "  - `{}` ({}): {}\n",
                        o.name,
                        o.field_type,
                        if o.description.is_empty() {
                            crate::prompt_assets::template("tool_output_value_fallback.md")
                                .trim()
                                .to_string()
                        } else {
                            o.description.to_string()
                        }
                    ));
                }
            }

            output.push('\n');
        } else if let Some(meta) = crate::runtime_tools::get_runtime_tool(name) {
            push_runtime_tool_section(&mut output, &meta);
        } else {
            output.push_str(&format!("### {}\n(no description)\n\n", name));
        }
    }

    output
}

fn push_runtime_tool_section(output: &mut String, meta: &RuntimeToolMetadata) {
    let mut tags = Vec::new();
    if meta.readonly {
        tags.push("readonly");
    }
    if meta.destructive {
        tags.push("destructive");
    }
    if !meta.idempotent {
        tags.push("non-idempotent");
    }
    if meta.open_world {
        tags.push("RPC");
    }
    let tags_str = if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.join("/"))
    };

    output.push_str(&format!("### {}{}\n", meta.name, tags_str));
    output.push_str("Call syntax: `EXEC ");
    output.push_str(&meta.name);
    for p in &meta.parameters {
        if p.required {
            output.push_str(&format!(" --{} <{}>", p.name, runtime_param_type_label(p)));
        }
    }
    if meta.parameters.iter().any(|p| !p.required) {
        output.push_str(" [optional parameters use the same `--name value` form]");
    }
    output.push_str("`\n");

    if !meta.outputs.is_empty() {
        output.push_str("**Output pins**: ");
        let pins: Vec<String> = meta
            .outputs
            .iter()
            .map(|o| format!("`{}`", o.name))
            .collect();
        output.push_str(&format!("{}\n", pins.join(", ")));
    }
    output.push_str(&format!("{}\n", meta.description));

    if !meta.parameters.is_empty() {
        output.push_str("Parameters:\n");
        for p in &meta.parameters {
            let req = if p.required {
                "**required**"
            } else {
                "optional"
            };
            let default_str = p
                .default_value
                .as_ref()
                .map(|d| format!(", default={}", d))
                .unwrap_or_default();
            output.push_str(&format!(
                "  - `--{}` ({}, type={}{}): {}\n",
                p.name,
                req,
                runtime_param_type_label(p),
                default_str,
                p.description
            ));
        }
    } else {
        output.push_str("(no parameters)\n");
    }

    if !meta.outputs.is_empty() {
        output.push_str("Outputs:\n");
        for o in &meta.outputs {
            output.push_str(&format!(
                "  - `{}` ({}): {}\n",
                o.name,
                o.field_type,
                if o.description.is_empty() {
                    "Output value".to_string()
                } else {
                    o.description.clone()
                }
            ));
        }
    }
    output.push('\n');
}

fn runtime_param_type_label(parameter: &corework::rpc_tool::RuntimeAIParameter) -> &str {
    let ty = parameter.param_type.trim();
    if ty.is_empty() {
        "String"
    } else {
        ty
    }
}

// ============================================================================
// 1. BuildSystemPrompt
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSystemPromptInput {
    pub persona: Option<String>,

    /// Current state machine state name, such as "thinking" or "executing".
    #[serde(default = "default_state")]
    pub current_state: String,
}

fn default_state() -> String {
    crate::state_machine::states::THINKING.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSystemPromptOutput {
    pub system_message: Message,
}

#[buns_system(
    "BuildSystemPrompt",
    description = "Build a complete system prompt from persona, skills, tools, workflows, and state instructions.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct BuildSystemPromptSystem;

#[async_trait]
impl SystemOperation for BuildSystemPromptSystem {
    type Input = BuildSystemPromptInput;
    type Output = BuildSystemPromptOutput;
    type Error = FrameworkError;

    fn name(&self) -> &str {
        system_names::BUILD_SYSTEM_PROMPT
    }

    async fn execute(
        &self,
        input: BuildSystemPromptInput,
        ctx: &Context,
    ) -> Result<BuildSystemPromptOutput, FrameworkError> {
        let mut sections: Vec<PromptSection> = Vec::new();

        let Some(persona) = input.persona.filter(|p| !p.trim().is_empty()) else {
            return Err(FrameworkError::InvalidOperation(
                "BuildSystemPrompt requires a role skill persona".to_string(),
            ));
        };
        sections.push(PromptSection::new(persona));

        let immutable_cache_entries =
            AssistantContext::get_immutable_cache_entries(&ctx.cache).await?;
        let immutable_cache_section =
            format_immutable_cache_entries_section(&immutable_cache_entries);
        if !immutable_cache_section.is_empty() {
            sections.push(PromptSection::new(immutable_cache_section));
        }

        sections.push(PromptSection::new(build_env_context_section()));

        {
            let m = mgr().read().await;

            let catalog = m.catalog_prompt();
            if !catalog.is_empty() {
                sections.push(PromptSection::new(catalog));
            }

            let active_names = AssistantContext::all_active_skills(&ctx.cache).await?;
            if !active_names.is_empty() {
                let name_refs: Vec<&str> = active_names.iter().map(|s| s.as_str()).collect();
                let active_prompt = m.active_skills_prompt(&name_refs);
                if !active_prompt.is_empty() {
                    sections.push(PromptSection::new(active_prompt));
                }
            }
        }

        let all_tools = AssistantContext::all_active_tools(&ctx.cache).await?;
        let tools_section = format_tools_section(&all_tools);
        if !tools_section.is_empty() {
            sections.push(PromptSection::new(tools_section));
        }

        let workflows_section = format_workflows_section_from_ctx(ctx);
        if !workflows_section.is_empty() {
            sections.push(PromptSection::new(workflows_section));
        }

        let plan_execution_rules = crate::prompt_assets::template("plan_execution_rules.md")
            .trim()
            .to_string();
        if !plan_execution_rules.is_empty() {
            sections.push(PromptSection::new(plan_execution_rules));
        }

        sections.push(PromptSection::new(build_response_protocol_instruction()));

        let instruction = state_instruction(&input.current_state);
        let instruction_header = crate::prompt_assets::template("current_instruction_header.md")
            .trim()
            .to_string();
        sections.push(PromptSection::new(format!(
            "{}\n\n{}",
            instruction_header, instruction
        )));

        let full_text = compose_prompt_sections(sections);
        let system_message = Message::system(full_text);

        Ok(BuildSystemPromptOutput { system_message })
    }
}

// ============================================================================
// 2. ComposeMessages
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeMessagesInput {
    pub system_message: Message,

    /// Maximum history messages; defaults when omitted.
    #[serde(default)]
    pub max_history_messages: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeMessagesOutput {
    pub messages: Vec<Message>,
    /// Total message count, including system.
    pub message_count: usize,
    /// Number of truncated history messages.
    pub truncated_count: usize,
}

#[buns_system(
    "ComposeMessages",
    description = "Compose the system prompt and conversation history into a message sequence.",
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ComposeMessagesSystem;

#[async_trait]
impl SystemOperation for ComposeMessagesSystem {
    type Input = ComposeMessagesInput;
    type Output = ComposeMessagesOutput;
    type Error = FrameworkError;

    fn name(&self) -> &str {
        system_names::COMPOSE_MESSAGES
    }

    async fn execute(
        &self,
        input: ComposeMessagesInput,
        ctx: &Context,
    ) -> Result<ComposeMessagesOutput, FrameworkError> {
        let max_msgs = input
            .max_history_messages
            .unwrap_or(DEFAULT_MAX_HISTORY_MESSAGES);

        let history = AssistantContext::get_conversation(&ctx.cache).await?;

        let (selected, truncated_count) = select_history(&history, max_msgs);

        let agent_id = ctx
            .cache
            .get::<String>(crate::state_machine::agent_keys::AGENT_ID)
            .await?
            .unwrap_or_else(|| crate::agent::keys::BOSS_AGENT_ID.to_string());
        let dynamic_snapshots = ctx
            .resolve_shared_component::<crate::conversation_state::ConversationState>()?
            .dynamic_snapshots(&agent_id)
            .await;
        let structures_section = format_host_dynamic_snapshots_section(&dynamic_snapshots);
        let mut messages = Vec::with_capacity(2 + selected.len());
        messages.push(input.system_message);
        messages.extend(selected.into_iter().cloned());
        if !structures_section.is_empty() {
            messages.push(Message::user(structures_section));
        }

        let message_count = messages.len();

        Ok(ComposeMessagesOutput {
            messages,
            message_count,
            truncated_count,
        })
    }
}

/// Select the most recent complete message rounds from history.
pub(crate) fn select_history(history: &[Message], max_messages: usize) -> (&[Message], usize) {
    if history.is_empty() || max_messages == 0 {
        return (&history[history.len()..], history.len());
    }

    let n = history.len();
    let mut cut_index = n;
    let mut count: usize = 0;

    let mut i = n;
    while i > 0 {
        let start = find_round_start(history, i);
        let round_len = i - start;

        if count + round_len > max_messages {
            break;
        }

        count += round_len;
        cut_index = start;
        i = start;
    }

    let selected = &history[cut_index..];
    let truncated = cut_index;
    (selected, truncated)
}

fn find_round_start(history: &[Message], end: usize) -> usize {
    if end == 0 {
        return 0;
    }
    let last = &history[end - 1];

    match last.role.as_str() {
        "user"
            if end >= 3
                && history[end - 2].role == "tool"
                && history[end - 3].role == "assistant"
                && history[end - 3].tool_calls.is_some() =>
        {
            end - 3
        }
        "user" => end - 1,
        "tool"
            if end >= 2
                && history[end - 2].role == "assistant"
                && history[end - 2].tool_calls.is_some() =>
        {
            end - 2
        }
        _ => end - 1,
    }
}

// ============================================================================
/// Build system prompt text.
pub fn build_system_prompt_text(
    persona: &str,
    _skills_catalog: &str,
    active_skills_prompt: &str,
    tools_section: &str,
    workflows_section: &str,
    _page_structures_section: &str,
    current_state: &str,
    frontend_widgets_enabled: bool,
) -> String {
    let mut sections: Vec<PromptSection> = Vec::new();

    sections.push(PromptSection::new(persona));

    if let Some(section) = PromptSection::from_optional(active_skills_prompt) {
        sections.push(section);
    }
    if let Some(section) = PromptSection::from_optional(tools_section) {
        sections.push(section);
    }
    if let Some(section) = PromptSection::from_optional(workflows_section) {
        sections.push(section);
    }
    let plan_execution_rules = crate::prompt_assets::template("plan_execution_rules.md")
        .trim()
        .to_string();
    if !plan_execution_rules.is_empty() {
        sections.push(PromptSection::new(plan_execution_rules));
    }
    // Host-published dynamic context is intentionally omitted from system prompt.
    // Callers append it at the request tail to preserve reusable prompt prefixes.

    sections.push(PromptSection::new(build_response_protocol_instruction()));
    if frontend_widgets_enabled {
        let widget_protocol = crate::prompt_assets::template("frontend_widget_protocol.md")
            .trim()
            .to_string();
        if !widget_protocol.is_empty() {
            sections.push(PromptSection::new(widget_protocol));
        }
    }

    let instruction_header = crate::prompt_assets::template("current_instruction_header.md")
        .trim()
        .to_string();
    let state_section = PromptSection::new(format!(
        "{}\n\n{}",
        instruction_header,
        state_instruction(current_state)
    ));
    sections.push(state_section);

    compose_prompt_sections(sections)
}

/// Build response protocol instructions.
pub fn build_response_protocol_instruction() -> String {
    crate::prompt_assets::template("function_calling.md")
        .trim()
        .to_string()
}

// ============================================================================

/// Read workflow registry from context and format it as prompt text.
pub(crate) fn format_workflows_section_from_ctx(ctx: &Context) -> String {
    let world = match ctx.get_world_cache() {
        Ok(w) => w,
        Err(_) => return String::new(),
    };

    let registry: Vec<serde_json::Value> = world
        .get_resource("wf:registry")
        .unwrap_or(None)
        .unwrap_or_default();

    if registry.is_empty() {
        return String::new();
    }

    format_workflows_section(&registry)
}

/// Format workflow registry entries as prompt text.
pub fn format_workflows_section(registry: &[serde_json::Value]) -> String {
    if registry.is_empty() {
        return String::new();
    }

    let mut output = String::from("## Registered Workflows\n\n");
    output.push_str(
        "Available workflows can be executed with `WfRunWorkflow --name \"name\" --inputs \"{...}\"`.\n\n",
    );

    for entry in registry {
        let meta = match entry.get("metadata") {
            Some(m) => m,
            None => continue,
        };

        let name = meta
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed");
        let desc = meta
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let source = entry
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("Local");

        let source_tag = match source {
            "Official" => " [official]",
            _ => "",
        };

        output.push_str(&format!("### {}{}\n", name, source_tag));
        if !desc.is_empty() {
            output.push_str(&format!("{}\n", desc));
        }

        let inputs = meta
            .get("inputs")
            .and_then(|v| v.as_array())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if !inputs.is_empty() {
            output.push_str("Inputs:\n");
            for pin in inputs {
                let pname = pin.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let ptype = pin
                    .get("data_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Any");
                let pdesc = pin
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let has_default = pin.get("default_value").is_some();
                let req = if has_default {
                    "optional"
                } else {
                    "**required**"
                };
                output.push_str(&format!(
                    "  - `{}` ({}, {}): {}\n",
                    pname, ptype, req, pdesc
                ));
            }
        }

        let outputs = meta
            .get("outputs")
            .and_then(|v| v.as_array())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if !outputs.is_empty() {
            output.push_str("Outputs:\n");
            for pin in outputs {
                let pname = pin.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let ptype = pin
                    .get("data_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Any");
                let pdesc = pin
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                output.push_str(&format!("  - `{}` ({}): {}\n", pname, ptype, pdesc));
            }
        }

        output.push('\n');
    }

    output
}

// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_history_empty() {
        let history: Vec<Message> = vec![];
        let (selected, truncated) = select_history(&history, 10);
        assert!(selected.is_empty());
        assert_eq!(truncated, 0);
    }

    #[test]
    fn test_select_history_all_fit() {
        let history = vec![
            Message::user("hello"),
            Message::assistant("hi there"),
            Message::user("show code"),
        ];
        let (selected, truncated) = select_history(&history, 10);
        assert_eq!(selected.len(), 3);
        assert_eq!(truncated, 0);
    }

    #[test]
    fn test_select_history_truncation() {
        let history = vec![
            Message::user("first"),
            Message::assistant("second"),
            Message::user("third"),
            Message::assistant("fourth"),
            Message::user("latest"),
        ];
        let (selected, truncated) = select_history(&history, 2);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].content, "fourth");
        assert_eq!(selected[1].content, "latest");
        assert_eq!(truncated, 3);
    }

    #[test]
    fn test_select_history_tool_pair() {
        let history = vec![
            Message::user("check weather"),
            Message::assistant("checking"),
            Message::tool("sunny"),
            Message::user("thanks"),
        ];

        let (selected, truncated) = select_history(&history, 50);
        assert_eq!(selected.len(), 4);
        assert_eq!(truncated, 0);

        let (selected, _) = select_history(&history, 1);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].content, "thanks");
    }

    #[test]
    fn test_build_system_prompt_text() {
        let result = build_system_prompt_text(
            "You are a test assistant.",
            "",
            "## Active Skills\n- test",
            "## Available Tools\n- Calculator",
            "",
            "",
            "thinking",
            true,
        );
        assert!(result.contains("You are a test assistant."));
        assert!(result.contains("## Active Skills"));
        assert!(result.contains("## Available Tools"));
    }

    #[test]
    fn test_build_system_prompt_text_excludes_host_dynamic_context() {
        let result = build_system_prompt_text(
            "You are a test assistant.",
            "",
            "",
            "",
            "",
            "volatile page snapshot",
            "thinking",
            true,
        );

        assert!(!result.contains("volatile page snapshot"));
    }

    #[test]
    fn test_plain_prompt_excludes_frontend_widget_protocol() {
        let result = build_system_prompt_text(
            "You are a test assistant.",
            "",
            "",
            "",
            "",
            "",
            "thinking",
            false,
        );

        assert!(!result.contains("[input:text"));
    }

    #[tokio::test]
    async fn test_compose_messages_appends_host_dynamic_context_at_tail() {
        use std::sync::Arc;

        let framework = corework::world::FrameworkState::initialize().unwrap();
        let unit = Arc::new(corework::execution_unit::ExecutionUnit::new_root(
            corework::execution_unit::UnitType::Module,
            framework,
        ));
        let state = Arc::new(crate::conversation_state::ConversationState::new(
            "prompt-test",
            Default::default(),
            crate::agent::keys::BOSS_AGENT_ID,
        ));
        unit.attach_shared_component(Arc::clone(&state)).unwrap();
        let ctx = unit.create_context();
        AssistantContext::set_conversation(&ctx.cache, &[Message::user("current request")])
            .await
            .unwrap();
        state
            .set_dynamic_snapshot_field(
                crate::agent::keys::BOSS_AGENT_ID,
                "page",
                "volatile page snapshot",
            )
            .await;

        let result = ComposeMessagesSystem
            .execute(
                ComposeMessagesInput {
                    system_message: Message::system("stable system prompt"),
                    max_history_messages: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.messages[0].content, "stable system prompt");
        assert_eq!(result.messages[1].content, "current request");
        assert!(result.messages[2]
            .content
            .contains("volatile page snapshot"));
        assert_eq!(result.messages.len(), 3);
    }

    #[test]
    fn test_format_tools_section_empty() {
        let section = format_tools_section(&[]);
        assert!(section.is_empty());
    }

    #[test]
    fn test_host_dynamic_snapshots_inject_all_text_without_field_names() {
        let snapshots = HashMap::from([
            ("private_location".to_string(), "Location text.".to_string()),
            ("private_ticket".to_string(), "Ticket text.".to_string()),
        ]);

        let section = format_host_dynamic_snapshots_section(&snapshots);

        assert!(section.contains("Location text."));
        assert!(section.contains("Ticket text."));
        assert!(!section.contains("private_location"));
        assert!(!section.contains("private_ticket"));
    }

    #[test]
    fn test_immutable_cache_entries_inject_values_in_key_order() {
        let entries = BTreeMap::from([
            ("zeta".to_string(), "Z entry".to_string()),
            ("alpha".to_string(), "A entry".to_string()),
        ]);

        let section = format_immutable_cache_entries_section(&entries);

        assert_eq!(section, "A entry\n\nZ entry");
        assert!(!section.contains("alpha"));
        assert!(!section.contains("zeta"));
    }

    #[test]
    fn test_runtime_tool_section_includes_exec_cli_contract() {
        let meta = RuntimeToolMetadata {
            name: "UserGet".to_string(),
            display_name: "Get User".to_string(),
            description: "Read user profile.".to_string(),
            tool_kind: "rpc".to_string(),
            parameters: vec![corework::rpc_tool::RuntimeAIParameter {
                name: "user_id".to_string(),
                param_type: "Number".to_string(),
                required: true,
                default_value: None,
                description: "User id.".to_string(),
            }],
            outputs: vec![corework::rpc_tool::RuntimeAIOutputField {
                name: "user".to_string(),
                field_type: "Object".to_string(),
                description: "User profile.".to_string(),
            }],
            destructive: false,
            readonly: true,
            idempotent: true,
            open_world: false,
            secret: false,
            required_capabilities: Vec::new(),
            endpoint_id: "user-tools".to_string(),
            service: "user-service".to_string(),
            method: "execute".to_string(),
        };
        let mut section = String::new();

        push_runtime_tool_section(&mut section, &meta);

        assert!(section.contains("Call syntax: `EXEC UserGet --user_id <Number>`"));
        assert!(section.contains("`--user_id` (**required**, type=Number)"));
        assert!(!section.contains("user_id ="));
    }

    #[test]
    fn test_format_page_structures_with_diff() {
        use std::collections::HashMap;

        let mut current = HashMap::new();
        let empty_overview: Vec<serde_json::Value> = vec![];

        current.insert(
            "page1".to_string(),
            "+ - button \"Login\" [sel='...']\n  - link \"Register\" [sel='...']".to_string(),
        );

        let result = format_page_structures_section(&current, &empty_overview);
        assert!(result.contains("+ - button \"Login\""));
        assert!(result.contains("  - link \"Register\""));

        let overview = vec![
            serde_json::json!({
                "page_id": "page1",
                "url": "https://example.com",
                "title": "Example",
                "is_current": true,
            }),
            serde_json::json!({
                "page_id": "page2",
                "url": "https://google.com",
                "title": "Google",
                "is_current": false,
            }),
        ];
        let result3 = format_page_structures_section(&current, &overview);
        assert!(result3.contains("page1"));
        assert!(!result3.contains("### Open Pages"));
        assert!(!result3.contains("page2"));
        assert!(!result3.contains("https://google.com"));
    }
}
