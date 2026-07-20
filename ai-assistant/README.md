# AI Assistant

`ai-assistant` is the Agent Runtime layer built on top of `corework` and
`llm-gateway`.

It coordinates user messages, model responses, skill prompts, tool execution,
runtime events, and conversation persistence.

## Capabilities

- Conversation state machine for thinking, executing, saying, asking, and
  suspended states.
- Runtime event emission for host applications and frontends.
- Skill loading for role-level identity and feature-level capabilities.
- Tool execution through built-in systems and RPC tool sidecars.
- Ledger and snapshot persistence for conversation recovery.
- Agent routing primitives for multi-Agent workflows.
- Locale-aware runtime prompt templates loaded from `prompts/{zh,en}`.

## Minimal Usage

```rust
use ai_assistant::AIAssistant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assistant = AIAssistant::with_defaults();
    let response = assistant.process("hello").await?;
    println!("AI: {}", response);
    Ok(())
}
```

## Dependencies

- `corework`: orchestration, event, cache, workflow, and tool protocol support.
- `llm-gateway`: model provider dispatch.
- `tokio`: async runtime.
- `serde`: configuration and event serialization.

## More Documentation

Detailed design notes are under:

```text
docs/
docs/en/
prompts/
```
