# agent-tool-go

Local development use from an example module:

```go
replace github.com/corework/agent-tool-go => ../../../sdk/rpctools/go
```

Authoring API:

```go
agenttool.RegisterTool(...)
agenttool.Serve(...)
```

This module currently provides the Python-style ToolProvider authoring API:

- `agenttool.RegisterTool(...)`
- `agenttool.Serve(...)`
- `agenttool.Context`
- `agenttool.AIOutput`

The current transport is the local JSON-lines protocol used by the runtime's
`rpc_tools` integration. Runtime discovers descriptors registered with
`agenttool.RegisterTool(...)` through the SDK `list_tools` handshake, then
routes AI calls back to `agenttool.Serve(...)`.

`agenttool.Context` is populated from the runtime execute request. Tool
handlers can read call/session fields such as `CallID`, `ToolCallID`,
`SessionID`, `ProviderID`, `ClusterID`, `RuntimeInstanceID`, `ConversationID`,
`AgentID`, `TurnID`, `Permissions`, and `HostContext`.

Dynamic AI context is host-owned and updated through the runtime FFI.
The removed RPC `snapshot.*` helpers are not compatible with this version.
