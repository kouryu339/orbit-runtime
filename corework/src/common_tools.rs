//! Common AI-callable tools that are useful across products.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify};

use crate::ai_system::{AIInput, AIOutput};
use crate::cache::CacheExt;
use crate::define_operation;
use crate::error::{FrameworkError, Result};
use crate::event::{BaseEvent, EventHandler};
use crate::orchestration::Context;
use crate::system::SystemOperation;

const DEFAULT_WAIT_MS: u64 = 30_000;
const MAX_WAIT_MS: u64 = 300_000;
const RUNTIME_DATA_DIR_CACHE_KEY: &str = "runtime:data_dir";

struct WaitEventHandler {
    name: String,
    scope_id: Option<String>,
    conversation_id: Option<String>,
    event: Mutex<Option<BaseEvent>>,
    notify: Notify,
}

impl WaitEventHandler {
    fn new(ctx: &Context) -> Self {
        Self {
            name: format!("WaitEventHandler:{}", uuid::Uuid::new_v4()),
            scope_id: ctx.scope_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            event: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    fn matches(&self, event: &BaseEvent) -> bool {
        if let Some(scope_id) = self.scope_id.as_deref() {
            if event.scope_id.as_deref() != Some(scope_id) {
                return false;
            }
        }
        if let Some(conversation_id) = self.conversation_id.as_deref() {
            if event.conversation_id.as_deref() != Some(conversation_id) {
                return false;
            }
        }
        true
    }
}

#[async_trait]
impl EventHandler for WaitEventHandler {
    async fn handle(&self, event: &BaseEvent) -> Result<()> {
        if self.matches(event) {
            *self.event.lock().await = Some(event.clone());
            self.notify.notify_one();
        }
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[define_operation(
    name = "Wait",
    display_name = "Wait",
    category = "Utility",
    description = "Yield execution until a timeout expires or an optional event arrives in the current scope. Use this instead of repeatedly polling while another task is still working.",
    params {
        timeout_ms: "Number@Maximum wait in milliseconds. Optional; defaults to 30000 and is capped at 300000.",
        event_type: "String@Optional exact event type. When set, return early after a matching event arrives in the current scope and conversation.",
        reason: "String@Optional short explanation of what is being awaited."
    },
    outputs {
        wake_reason: "String@Either timeout or event.",
        elapsed_ms: "Number@Actual elapsed milliseconds.",
        event: "Any@Matched event when wake_reason is event; otherwise null."
    },
    destructive = false,
    readonly = true,
    idempotent = false,
    open_world = false
)]
pub struct Wait;

#[async_trait]
impl SystemOperation for Wait {
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
            Err(output) => return Ok(output),
        };
        let timeout_ms = match args.get("timeout_ms") {
            Some(raw) => match raw.parse::<u64>() {
                Ok(0) => return Ok(AIOutput::error(400, "timeout_ms must be greater than zero")),
                Ok(value) => value.min(MAX_WAIT_MS),
                Err(_) => return Ok(AIOutput::error(400, "timeout_ms must be an integer")),
            },
            None => DEFAULT_WAIT_MS,
        };
        let event_type = args
            .get("event_type")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let reason = args.get("reason").unwrap_or("").trim().to_string();
        let started = Instant::now();

        let event = if let Some(event_type) = event_type.as_deref() {
            wait_for_event(ctx, event_type, timeout_ms).await?
        } else {
            tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
            None
        };

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let wake_reason = if event.is_some() { "event" } else { "timeout" };
        let event_json = event
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok())
            .unwrap_or(serde_json::Value::Null);
        let summary = if reason.is_empty() {
            format!("Wait finished by {wake_reason} after {elapsed_ms} ms.")
        } else {
            format!("Wait for '{reason}' finished by {wake_reason} after {elapsed_ms} ms.")
        };

        Ok(AIOutput::success(
            serde_json::json!({
                "wake_reason": wake_reason,
                "elapsed_ms": elapsed_ms,
                "event": event_json,
            }),
            summary,
        ))
    }

    fn name(&self) -> &str {
        "Wait"
    }
}

#[define_operation(
    name = "ContinueThinking",
    display_name = "Continue Thinking",
    category = "Utility",
    description = "Request one additional thinking round without performing any business action. Use only when a complex problem genuinely needs another reasoning pass before choosing a tool or answering. Do not call repeatedly to delay a decision.",
    params {},
    outputs {
        continue_thinking: "Boolean@Always true, indicating that the runtime should continue to the next thinking round."
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ContinueThinking;

#[async_trait]
impl SystemOperation for ContinueThinking {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: AIInput,
        _ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        Ok(AIOutput::success(
            serde_json::json!({"continue_thinking": true}),
            "Continue with another reasoning pass. Reassess the problem and then choose a concrete action or answer.",
        ))
    }

    fn name(&self) -> &str {
        "ContinueThinking"
    }
}

#[define_operation(
    name = "WriteMarkdown",
    display_name = "Write Markdown",
    category = "Utility",
    description = "Write a UTF-8 Markdown document under the runtime data/md directory. Use this for durable reports or summaries that should be shown to the user.",
    params {
        file_name: "String@Relative .md file name under data/md. Safe subdirectories are allowed, for example agent-test/final-report.md.",
        content: "String@Complete UTF-8 Markdown content to write.",
        overwrite: "Boolean@Whether to replace an existing file. Optional; defaults to true."
    },
    outputs {
        file_name: "String@Normalized relative file name under data/md.",
        path: "String@Absolute path of the written Markdown file.",
        bytes_written: "Number@Number of UTF-8 bytes written."
    },
    destructive = true,
    readonly = false,
    idempotent = true,
    open_world = false
)]
pub struct WriteMarkdown;

#[async_trait]
impl SystemOperation for WriteMarkdown {
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
            Err(output) => return Ok(output),
        };
        let file_name = match args.safe_require("file_name") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let content = match args.safe_require("content") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let overwrite = match args.get("overwrite") {
            Some(value) => match value.parse::<bool>() {
                Ok(value) => value,
                Err(_) => return Ok(AIOutput::error(400, "overwrite must be a boolean")),
            },
            None => true,
        };
        let relative_path = match validate_markdown_relative_path(&file_name) {
            Ok(path) => path,
            Err(message) => return Ok(AIOutput::error(400, message)),
        };
        let data_dir = match ctx.cache.get::<String>(RUNTIME_DATA_DIR_CACHE_KEY).await? {
            Some(path) if !path.trim().is_empty() => PathBuf::from(path),
            _ => {
                return Ok(AIOutput::error(
                    500,
                    "runtime data directory is not configured",
                ))
            }
        };
        let markdown_dir = data_dir.join("md");
        let destination = markdown_dir.join(&relative_path);

        let exists = match tokio::fs::try_exists(&destination).await {
            Ok(exists) => exists,
            Err(error) => {
                return Ok(AIOutput::error(
                    500,
                    format!("failed to inspect Markdown destination: {error}"),
                ))
            }
        };
        if !overwrite && exists {
            return Ok(AIOutput::error(
                409,
                format!("Markdown file already exists: {}", relative_path.display()),
            ));
        }
        if let Some(parent) = destination.parent() {
            if let Err(error) = tokio::fs::create_dir_all(parent).await {
                return Ok(AIOutput::error(
                    500,
                    format!("failed to create Markdown directory: {error}"),
                ));
            }
        }
        if let Err(error) = tokio::fs::write(&destination, content.as_bytes()).await {
            return Ok(AIOutput::error(
                500,
                format!("failed to write Markdown file: {error}"),
            ));
        }

        let normalized_file_name = relative_path.to_string_lossy().replace('\\', "/");
        let absolute_path = destination.display().to_string();
        Ok(AIOutput::success(
            serde_json::json!({
                "file_name": normalized_file_name,
                "path": absolute_path,
                "bytes_written": content.len(),
            }),
            format!("Markdown written to {absolute_path}"),
        ))
    }

    fn name(&self) -> &str {
        "WriteMarkdown"
    }
}

fn validate_markdown_relative_path(file_name: &str) -> std::result::Result<PathBuf, String> {
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return Err("file_name must not be empty".to_string());
    }

    let path = Path::new(file_name);
    if path.is_absolute()
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("md"))
    {
        return Err("file_name must be a relative path ending in .md".to_string());
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            _ => return Err(
                "file_name must not contain root, parent, current-directory, or prefix components"
                    .to_string(),
            ),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("file_name must not be empty".to_string());
    }
    Ok(normalized)
}

async fn wait_for_event(
    ctx: &Context,
    event_type: &str,
    timeout_ms: u64,
) -> Result<Option<BaseEvent>> {
    let handler = Arc::new(WaitEventHandler::new(ctx));
    let handler_name = handler.name().to_string();
    ctx.world_event_bus
        .subscribe(event_type.to_string(), handler.clone())
        .await?;

    let notified = handler.notify.notified();
    let woke = tokio::time::timeout(Duration::from_millis(timeout_ms), notified)
        .await
        .is_ok();
    let event = if woke {
        handler.event.lock().await.clone()
    } else {
        None
    };

    ctx.world_event_bus
        .unsubscribe(event_type, &handler_name)
        .await?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;
    use crate::event::{BaseEvent, InMemoryEventBus};
    use crate::monitoring::NoopTelemetry;

    fn test_context() -> Context {
        Context::new(
            Arc::new(InMemoryCache::new()),
            Arc::new(InMemoryEventBus::new()),
            Arc::new(NoopTelemetry),
        )
        .with_scope_id("scope-a")
        .with_conversation_id("conversation-a")
    }

    #[test]
    fn wait_is_registered_as_a_public_ai_tool() {
        let metadata = crate::system::SystemRegistry::list_ai_systems()
            .into_iter()
            .find(|metadata| metadata.name == "Wait")
            .expect("Wait should be registered as an AI-callable system");

        assert_eq!(metadata.tool_kind, "local");
        assert!(metadata.description.contains("Yield execution"));
        assert!(metadata
            .parameters
            .iter()
            .any(|param| param.name == "timeout_ms"));
        assert!(metadata
            .parameters
            .iter()
            .any(|param| param.name == "event_type"));
    }

    #[test]
    fn continue_thinking_is_registered_as_a_public_ai_tool() {
        let metadata = crate::system::SystemRegistry::list_ai_systems()
            .into_iter()
            .find(|metadata| metadata.name == "ContinueThinking")
            .expect("ContinueThinking should be registered as an AI-callable system");

        assert_eq!(metadata.tool_kind, "local");
        assert!(metadata.parameters.is_empty());
        assert!(metadata.readonly);
        assert!(!metadata.destructive);
    }

    #[tokio::test]
    async fn continue_thinking_returns_success_without_side_effects() {
        let output = ContinueThinking
            .execute(
                AIInput {
                    input: String::new(),
                },
                &test_context(),
            )
            .await
            .unwrap();

        assert!(output.is_ok());
        assert_eq!(output.result["continue_thinking"], true);
    }

    #[test]
    fn write_markdown_is_registered_as_a_public_ai_tool() {
        let metadata = crate::system::SystemRegistry::list_ai_systems()
            .into_iter()
            .find(|metadata| metadata.name == "WriteMarkdown")
            .expect("WriteMarkdown should be registered as an AI-callable system");

        assert_eq!(metadata.tool_kind, "local");
        assert!(metadata.description.contains("data/md"));
        assert!(metadata
            .parameters
            .iter()
            .any(|param| param.name == "file_name"));
    }

    #[tokio::test]
    async fn write_markdown_writes_below_runtime_data_md() {
        let ctx = test_context();
        let root =
            std::env::temp_dir().join(format!("corework-write-markdown-{}", uuid::Uuid::new_v4()));
        ctx.cache
            .set(
                RUNTIME_DATA_DIR_CACHE_KEY,
                &root.display().to_string(),
                None,
            )
            .await
            .unwrap();

        let output = WriteMarkdown
            .execute(
                AIInput {
                    input: "--file_name agent-test/final-report.md --content \"# Final report\""
                        .to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();

        assert!(output.is_ok());
        assert_eq!(output.result["file_name"], "agent-test/final-report.md");
        assert_eq!(
            tokio::fs::read_to_string(root.join("md/agent-test/final-report.md"))
                .await
                .unwrap(),
            "# Final report"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn write_markdown_rejects_paths_outside_data_md() {
        let ctx = test_context();
        let root =
            std::env::temp_dir().join(format!("corework-write-markdown-{}", uuid::Uuid::new_v4()));
        ctx.cache
            .set(
                RUNTIME_DATA_DIR_CACHE_KEY,
                &root.display().to_string(),
                None,
            )
            .await
            .unwrap();

        for file_name in ["../outside.md", "report.txt", "C:\\outside.md"] {
            let output = WriteMarkdown
                .execute(
                    AIInput {
                        input: format!("--file_name \"{file_name}\" --content report"),
                    },
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!output.is_ok(), "{file_name} should be rejected");
        }
        assert!(!root.join("outside.md").exists());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn wait_returns_after_timeout() {
        let output = Wait
            .execute(
                AIInput {
                    input: "--timeout_ms 5".to_string(),
                },
                &test_context(),
            )
            .await
            .unwrap();

        assert!(output.is_ok());
        assert_eq!(output.result["wake_reason"], "timeout");
    }

    #[tokio::test]
    async fn wait_returns_matching_event() {
        let ctx = test_context();
        let publisher = ctx.world_event_bus.clone();
        let task_ctx = ctx.clone();
        let wait_task = tokio::spawn(async move {
            Wait.execute(
                AIInput {
                    input: "--timeout_ms 1000 --event_type agent-test.adversary.completed"
                        .to_string(),
                },
                &task_ctx,
            )
            .await
            .unwrap()
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        publisher
            .publish(
                BaseEvent::new(
                    "agent-test.adversary.completed",
                    serde_json::json!({"pair_id": "adv-1"}),
                )
                .with_scope("scope-a")
                .with_conversation_id("conversation-a"),
            )
            .await
            .unwrap();

        let output = wait_task.await.unwrap();
        assert!(output.is_ok());
        assert_eq!(output.result["wake_reason"], "event");
        assert_eq!(output.result["event"]["payload"]["pair_id"], "adv-1");
    }

    #[tokio::test]
    async fn wait_ignores_other_scope_events() {
        let ctx = test_context();
        let publisher = ctx.world_event_bus.clone();
        let task_ctx = ctx.clone();
        let wait_task = tokio::spawn(async move {
            Wait.execute(
                AIInput {
                    input: "--timeout_ms 30 --event_type done".to_string(),
                },
                &task_ctx,
            )
            .await
            .unwrap()
        });

        tokio::time::sleep(Duration::from_millis(5)).await;
        publisher
            .publish(BaseEvent::new("done", serde_json::json!({})).with_scope("scope-b"))
            .await
            .unwrap();

        let output = wait_task.await.unwrap();
        assert_eq!(output.result["wake_reason"], "timeout");
    }
}
