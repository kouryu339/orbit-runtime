# Corework.AgentTool C# SDK

Local development reference:

```powershell
dotnet add reference ../../../sdk/rpctools/csharp/Corework.AgentTool.csproj
```

This project implements `AgentToolService.ListTools` and `Execute` with
HostCall helpers for `workspace.*`. Dynamic AI context is host-owned and must
be published through the runtime FFI; the removed `snapshot.*` helpers are not
compatible. The SDK generates server bindings from
`corework/proto/corework_agent_tool_v1.proto` at build time.

`ToolContext` also exposes runtime execute metadata: `CallId`, `ToolCallId`,
`IdempotencyKey`, `SessionId`, `ProviderId`, `ClusterId`, `RuntimeInstanceId`,
`ConversationId`, `AgentId`, `TurnId`, `Permissions`, and `HostContextJson`.
