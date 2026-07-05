# corework-agent-tool Python SDK

Local development install:

```powershell
uv run --with-editable sdk/rpctools/python python examples/tools/python/tools.py
```

Authoring API:

```python
from corework_agent_tool import AIOutput, ToolErrorCode, register_tool, serve
```

`serve()` exposes `corework.agent_tool.v1.AgentToolService` with:

- `ListTools`: returns descriptors collected by `@register_tool`.
- `Execute`: dispatches by `tool_name`, returns `AIOutput`, and supports
  workspace helpers through v1 `HostCall`. Dynamic context fields must be
  published by the host through the runtime FFI.

Workspace helpers:

- `ctx.workspace_resolve_path(path)`
- `ctx.workspace_resolve_working_path(path)`
- `ctx.workspace_create_path(path)`
- `ctx.workspace_create_working_path(path)`
- `ctx.workspace_save_as_edited(source_path, suffix)`

Declare the matching `required_capabilities`, for example
`workspace.resolve_path` or `workspace.save_as_edited`, in `@register_tool`
metadata. Corework runtime executes the actual `corework::workspace::*` logic;
sidecars should not duplicate those rules.

`ToolContext` also exposes runtime execute metadata: `call_id`, `tool_call_id`,
`idempotency_key`, `session_id`, `provider_id`, `cluster_id`,
`runtime_instance_id`, `conversation_id`, `agent_id`, `turn_id`, `permissions`,
and `host_context`.

The SDK compiles `corework/proto/corework_agent_tool_v1.proto` at startup for
local development. If the proto lives outside this repository layout, set
`COREWORK_AGENT_TOOL_PROTO` to its absolute path.
