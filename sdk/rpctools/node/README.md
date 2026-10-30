# @corework/agent-tool Node.js SDK

Local development install:

```powershell
cd sdk/rpctools/node
npm install
npm link
```

Authoring API:

```js
import { AIOutput, ToolErrorCode, registerTool, serve } from "@corework/agent-tool";
```

This package implements `AgentToolService.ListTools` and `Execute` with
HostCall helpers for `workspace.*`. Dynamic AI context is host-owned and must
be published through the runtime FFI; the removed `snapshot.*` helpers are not
compatible. `serve()` accepts either
`"127.0.0.1:50052"` or `{ host: "127.0.0.1", port: 50052 }`.

`ToolContext` also exposes runtime execute metadata: `callId`, `toolCallId`,
`idempotencyKey`, `sessionId`, `providerId`, `clusterId`, `runtimeInstanceId`,
`conversationId`, `agentId`, `turnId`, `workflowId`, `workflowRunId`, `nodeId`,
`jevName`, `jevRunId`, `jevSnapshotRevision`, `permissions`, and `hostContext`.
Descriptors may set `agent_enabled`, `workflow_enabled`, and `jev_enabled`;
all three default to true.

## Event sources

Event-only plugins may use `serveEventSource(address, source)`. Tools and events
may share `serve(address, {eventSource: source})`. `MemoryEventSource` is a bounded
reference journal and declares live or process-local memory replay, never durable
storage. Durable adapters implement `describe`, cancellable async `subscribe`,
and idempotent `acknowledge`. See [the event source contract](../../../agent_runtime_ffi/docs/zh/18-rpc-event-sources.md).
