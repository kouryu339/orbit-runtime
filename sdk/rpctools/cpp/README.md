# corework-agent-tool C++ SDK

Local development include path:

```powershell
cl /I sdk/rpctools/cpp/include ...
```

This SDK is currently a scaffold. The public API matches the examples; gRPC
service implementation will be added next.

`ToolContext` includes the same runtime execute metadata fields as the protocol:
`call_id`, `tool_call_id`, `idempotency_key`, `session_id`, `provider_id`,
`cluster_id`, `runtime_instance_id`, `conversation_id`, `agent_id`, `turn_id`,
`workflow_id`, `workflow_run_id`, `node_id`, `jev_name`, `jev_run_id`,
`jev_snapshot_revision`, `permissions`, and `host_context`. Tool descriptors
also expose `agent_enabled`, `workflow_enabled`, and `jev_enabled`.

Dynamic AI context is host-owned and updated through the runtime FFI.
The removed RPC `snapshot.*` helpers are not compatible with this version.
