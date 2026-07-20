# corework-agent-tool Java SDK

This directory is the Java Agent Tool SDK entry point.

Status: scaffold. It defines the public Java-side concepts that mirror the
Agent Tool protocol. The production server binding should expose
`corework.agent_tool.v1.AgentToolService` from
`corework/proto/corework_agent_tool_v1.proto`.

Use this SDK when a Java service wants to expose product capabilities as Agent
tools. Do not use it to create conversations or embed the Runtime; Runtime host
wrappers live under `sdk/runtime/<language>`.

## Public Concepts

```java
ToolDescriptor descriptor = new ToolDescriptor(
    "OrderLookup",
    "Read order summary by id",
    "read",
    List.of("workspace.resolve_path")
);

ToolHandler handler = (context, arguments) ->
    AIOutput.ok("Order status: paid", Map.of("status", "paid"));
```

`ToolContext` carries runtime execute metadata:

- `callId`
- `toolCallId`
- `idempotencyKey`
- `sessionId`
- `providerId`
- `clusterId`
- `runtimeInstanceId`
- `conversationId`
- `agentId`
- `turnId`
- `permissions`
- `hostContextJson`

Dynamic AI context is host-owned and must be published through the Runtime FFI.
The removed `snapshot.*` helpers are not compatible with this version.

## Next Implementation Step

1. Generate Java gRPC/protobuf bindings from
   `corework/proto/corework_agent_tool_v1.proto`.
2. Implement `AgentToolServer` over those bindings.
3. Map `ListTools` from registered `ToolDescriptor` values.
4. Dispatch `Execute` by `tool_name` and return non-empty `AIOutput.to_ai`.
