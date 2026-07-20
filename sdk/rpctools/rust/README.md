# corework-agent-tool Rust SDK

Local development dependency:

```toml
corework-agent-tool = { path = "../../../sdk/rpctools/rust" }
```

Authoring API:

```rust
use corework_agent_tool::{register_tool, serve, AIOutput, ToolContext};
```

`serve()` exposes `corework.agent_tool.v1.AgentToolService` with:

- `ListTools`: returns descriptors registered by `register_tool`.
- `Execute`: dispatches by `tool_name`, runs async handlers, returns `AIOutput`,
  and supports workspace helpers through v1 `HostCall`. Dynamic context fields
  are published by the host through the runtime FFI.

Workspace helpers:

- `ctx.workspace_resolve_path(path).await?`
- `ctx.workspace_resolve_working_path(path).await?`
- `ctx.workspace_create_path(path).await?`
- `ctx.workspace_create_working_path(path).await?`
- `ctx.workspace_save_as_edited(source_path, suffix).await?`

Declare the matching `required_capability`, for example
`workspace.resolve_path` or `workspace.save_as_edited`, in the tool descriptor.
Corework runtime executes the actual `corework::workspace::*` logic; sidecars
should not duplicate those rules.

`ToolContext` also exposes runtime execute metadata: `call_id`, `tool_call_id`,
`idempotency_key`, `session_id`, `provider_id`, `cluster_id`,
`runtime_instance_id`, `conversation_id`, `agent_id`, `turn_id`, `permissions`,
and `host_context`.

The crate generates Rust protobuf/gRPC bindings from
`corework/proto/corework_agent_tool_v1.proto` at build time.
