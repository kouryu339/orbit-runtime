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
`conversationId`, `agentId`, `turnId`, `permissions`, and `hostContext`.
