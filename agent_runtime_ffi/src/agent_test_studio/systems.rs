//! Compile-time registered Agent Test Studio tools.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use tokio::sync::RwLock;

use crate::runtime::AgentTestRuntimeHost;

use super::role_contract::AdversaryPersona;
use super::tool_runtime::AgentTestToolRuntime;
use super::tools::{
    AdversaryConcludeArgs, AdversaryCreateArgs, AdversaryDestroyArgs, AdversaryFinding,
    AdversaryInspectArgs, InspectMode,
};

type BoundRuntime = AgentTestToolRuntime<AgentTestRuntimeHost>;

fn runtime_slot() -> &'static RwLock<Option<Arc<BoundRuntime>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<BoundRuntime>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

pub(crate) async fn bind_runtime(runtime: Arc<BoundRuntime>) {
    *runtime_slot().write().await = Some(runtime);
}

pub(crate) async fn clear_runtime() {
    *runtime_slot().write().await = None;
}

async fn current_runtime() -> Result<Arc<BoundRuntime>, AIOutput> {
    runtime_slot().read().await.clone().ok_or_else(|| {
        AIOutput::error(
            503,
            "Agent Test Studio is not active. Open the Studio before calling this tool.",
        )
    })
}

fn required(args: &corework::ai_system::SimpleArgs, name: &str) -> Result<String, AIOutput> {
    args.safe_require(name)
}

fn optional(args: &corework::ai_system::SimpleArgs, name: &str) -> Option<String> {
    args.get(name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn list(args: &corework::ai_system::SimpleArgs, name: &str) -> Vec<String> {
    optional(args, name)
        .map(|value| {
            value
                .split(['\n', ',', ';', '；'])
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn turn_list(args: &corework::ai_system::SimpleArgs) -> Result<Vec<u32>, AIOutput> {
    list(args, "evidence_turns")
        .into_iter()
        .map(|value| {
            value.parse::<u32>().map_err(|_| {
                AIOutput::error(
                    400,
                    "evidence_turns must contain positive integers separated by commas or semicolons",
                )
            })
        })
        .collect()
}

fn success(result: serde_json::Value, message: String) -> AIOutput {
    AIOutput::success(result, message)
}

fn runtime_error(error: crate::runtime::RuntimeError) -> AIOutput {
    AIOutput::error(error.code(), error.to_string())
}

#[define_operation(
    name = "AdversaryCreate",
    display_name = "Create Adversary Test",
    category = "Agent Test Studio",
    description = "Create one isolated adversary and target conversation pair, then start the adversarial test.",
    system_only,
    params {
        identity: "String@Adversary persona identity. 必填.",
        personality: "String@Adversary persona personality. 必填.",
        background: "String@Adversary persona background. 必填.",
        goal: "String@Adversary test goal. 必填.",
        strategy: "String@Conversation strategy used by the adversary. 必填.",
        hidden_facts: "String@Optional hidden facts separated by commas, semicolons, or newlines.",
        boundaries: "String@Optional persona boundaries separated by commas, semicolons, or newlines.",
        initial_message: "String@Optional first message sent to the target; defaults to goal."
    },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct AdversaryCreate;

#[async_trait]
impl SystemOperation for AdversaryCreate {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, _ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let request = AdversaryCreateArgs {
            persona: AdversaryPersona {
                identity: match required(&args, "identity") {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                },
                personality: match required(&args, "personality") {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                },
                background: match required(&args, "background") {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                },
                goal: match required(&args, "goal") {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                },
                strategy: match required(&args, "strategy") {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                },
                hidden_facts: list(&args, "hidden_facts"),
                boundaries: list(&args, "boundaries"),
            },
            initial_message: optional(&args, "initial_message"),
        };
        let runtime = match current_runtime().await {
            Ok(runtime) => runtime,
            Err(output) => return Ok(output),
        };
        Ok(match runtime.create(request).await {
            Ok(result) => {
                let pair_id = result["pair_id"].as_str().unwrap_or("unknown").to_string();
                success(result, format!("Created adversary pair {pair_id}."))
            }
            Err(error) => runtime_error(error),
        })
    }

    fn name(&self) -> &str {
        "AdversaryCreate"
    }
}

#[define_operation(
    name = "AdversaryDestroy",
    display_name = "Destroy Adversary Test",
    category = "Agent Test Studio",
    description = "Irreversibly stop and remove one adversary test pair.",
    system_only,
    params {
        pair_id: "String@Pair identifier returned by AdversaryCreate. 必填.",
        reason: "String@Reason for destroying the pair. 必填."
    },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct AdversaryDestroy;

#[async_trait]
impl SystemOperation for AdversaryDestroy {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, _ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let pair_id = match required(&args, "pair_id") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let reason = match required(&args, "reason") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let runtime = match current_runtime().await {
            Ok(runtime) => runtime,
            Err(output) => return Ok(output),
        };
        Ok(
            match runtime
                .destroy(AdversaryDestroyArgs { pair_id, reason })
                .await
            {
                Ok(result) => {
                    let pair_id = result["pair_id"].as_str().unwrap_or("unknown").to_string();
                    success(result, format!("Destroyed adversary pair {pair_id}."))
                }
                Err(error) => runtime_error(error),
            },
        )
    }

    fn name(&self) -> &str {
        "AdversaryDestroy"
    }
}

#[define_operation(
    name = "AdversaryInspect",
    display_name = "Inspect Adversary Test",
    category = "Agent Test Studio",
    description = "Read the report or transcript evidence for one adversary test pair.",
    system_only,
    params {
        pair_id: "String@Pair identifier returned by AdversaryCreate. 必填.",
        mode: "String@Inspection mode: report or transcript. 必填."
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct AdversaryInspect;

#[async_trait]
impl SystemOperation for AdversaryInspect {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, _ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let pair_id = match required(&args, "pair_id") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let mode = match required(&args, "mode") {
            Ok(value) if value.eq_ignore_ascii_case("report") => InspectMode::Report,
            Ok(value) if value.eq_ignore_ascii_case("transcript") => InspectMode::Transcript,
            Ok(_) => return Ok(AIOutput::error(400, "mode must be report or transcript")),
            Err(output) => return Ok(output),
        };
        let runtime = match current_runtime().await {
            Ok(runtime) => runtime,
            Err(output) => return Ok(output),
        };
        Ok(
            match runtime
                .inspect(AdversaryInspectArgs { pair_id, mode })
                .await
            {
                Ok(result) => success(result, "Adversary pair inspection completed.".to_string()),
                Err(error) => runtime_error(error),
            },
        )
    }

    fn name(&self) -> &str {
        "AdversaryInspect"
    }
}

#[define_operation(
    name = "AdversaryConclude",
    display_name = "Conclude Adversary Test",
    category = "Agent Test Studio",
    description = "Submit the adversary conclusion and permanently close both paired conversations.",
    system_only,
    params {
        summary: "String@Conclusion summary. 必填.",
        finding_title: "String@Optional finding title; provide all finding fields together.",
        finding_observation: "String@Optional observed behavior; provide all finding fields together.",
        finding_expected_behavior: "String@Optional expected behavior; provide all finding fields together.",
        evidence_turns: "String@Optional evidence turn numbers separated by commas or semicolons."
    },
    destructive = true,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct AdversaryConclude;

#[async_trait]
impl SystemOperation for AdversaryConclude {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let summary = match required(&args, "summary") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let findings = match (
            optional(&args, "finding_title"),
            optional(&args, "finding_observation"),
            optional(&args, "finding_expected_behavior"),
        ) {
            (None, None, None) => Vec::new(),
            (Some(title), Some(observation), Some(expected_behavior)) => {
                let evidence_turns = match turn_list(&args) {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                };
                vec![AdversaryFinding {
                    title,
                    observation,
                    expected_behavior,
                    evidence_turns,
                }]
            }
            _ => {
                return Ok(AIOutput::error(
                    400,
                    "finding_title, finding_observation, and finding_expected_behavior must be provided together",
                ))
            }
        };
        let conversation_id = match ctx.conversation_id.as_deref() {
            Some(value) if !value.is_empty() => value,
            _ => {
                return Ok(AIOutput::error(
                    400,
                    "AdversaryConclude requires a conversation context",
                ))
            }
        };
        let runtime = match current_runtime().await {
            Ok(runtime) => runtime,
            Err(output) => return Ok(output),
        };
        Ok(
            match runtime
                .conclude(conversation_id, AdversaryConcludeArgs { summary, findings })
                .await
            {
                Ok(report) => {
                    let pair_id = report.pair_id.clone();
                    match serde_json::to_value(report) {
                        Ok(result) => {
                            success(result, format!("Concluded adversary pair {pair_id}."))
                        }
                        Err(error) => AIOutput::error(500, error.to_string()),
                    }
                }
                Err(error) => runtime_error(error),
            },
        )
    }

    fn name(&self) -> &str {
        "AdversaryConclude"
    }
}

#[cfg(test)]
mod tests {
    use corework::ai_system::AISystemFactory;

    fn metadata(name: &str) -> &'static corework::ai_system::AISystemMetadata {
        &inventory::iter::<AISystemFactory>()
            .find(|factory| factory.metadata.name == name)
            .unwrap_or_else(|| panic!("missing AI operation metadata for {name}"))
            .metadata
    }

    #[test]
    fn agent_test_tools_are_compile_time_registered_operations() {
        for name in [
            "AdversaryCreate",
            "AdversaryDestroy",
            "AdversaryInspect",
            "AdversaryConclude",
        ] {
            let tool = metadata(name);
            assert_eq!(tool.tool_kind, "local");
        }
    }

    #[test]
    fn adversary_create_exposes_flattened_required_parameters() {
        let tool = metadata("AdversaryCreate");
        let required: Vec<&str> = tool
            .parameters
            .iter()
            .filter(|parameter| parameter.required)
            .map(|parameter| parameter.name)
            .collect();
        assert_eq!(
            required,
            ["identity", "personality", "background", "goal", "strategy"]
        );
        assert!(tool
            .parameters
            .iter()
            .all(|parameter| parameter.name != "json"));
    }
}
