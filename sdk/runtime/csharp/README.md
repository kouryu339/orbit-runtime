# Corework.RuntimeHost C# SDK

C# Runtime Host SDK design entry for Agent Runtime ABI 1.

Status: design entry. The implementation should use P/Invoke over
`sdk/runtime/c/include/agent_runtime.h` and ship platform-specific native
runtime binaries beside the .NET host application.

The wrapper should expose:

```csharp
var options = new RuntimeCreateOptions {
    Schema = "agent-runtime-create-options/v1",
    LogLevel = "info",
    Language = "zh-CN",
    RestorePolicy = "strict",
    DataDir = "./data/runtime",
};
await using var runtime = await RuntimeHost.OpenAsync(options);
await runtime.RegisterResourcesAsync("config/resources.json");
await runtime.RegisterLlmAsync("config/llm-providers.json");
await runtime.RegisterAgentClusterAsync("config/agent-cluster.json");
await runtime.StartAsync();
var conversation = await runtime.SpawnConversationAsync("product-instance");
```

Implementation requirements:

- validate ABI and capabilities before creating conversations;
- marshal UTF-8 strings explicitly;
- release runtime-owned strings with `agent_runtime_free_string_v1`;
- surface `agent-runtime-result/v1` failures as typed exceptions;
- expose an async event stream for `agent-runtime-event/v1`.

This SDK embeds Runtime. It does not implement Agent tools; use `sdk/rpctools/csharp` for
C# RPC tool sidecars.
