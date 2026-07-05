# corework-runtime-host Java SDK

Java Runtime Host SDK design entry for Agent Runtime ABI 1.

Status: design entry. The implementation should use the stable C ABI in
`sdk/runtime/c/include/agent_runtime.h` through JNA, Panama FFM, or JNI. The
native boundary must remain the C ABI; Java hosts should not link Rust
implementation crates.

The wrapper should expose:

```java
var options = RuntimeCreateOptions.builder()
    .schema("agent-runtime-create-options/v1")
    .logLevel("info")
    .language("zh-CN")
    .restorePolicy("strict")
    .dataDir("./data/runtime")
    .build();
try (RuntimeHost runtime = RuntimeHost.open(options)) {
    runtime.registerResources("config/resources.json");
    runtime.registerLlm("config/llm-providers.json");
    runtime.registerAgentCluster("config/agent-cluster.json");
    runtime.start();
    var conversation = runtime.spawnConversation("product-instance");
}
```

Implementation requirements:

- check `agent_runtime_abi_version_v1() == 1`;
- read `agent_runtime_capabilities_v1()` before enabling optional features;
- free runtime-owned response strings with `agent_runtime_free_string_v1()`;
- expose blocking and executor-backed event polling;
- map Runtime command results and ABI errors separately.

This SDK embeds Runtime. It does not implement Agent tools; use `sdk/rpctools/java` for
Java RPC tool sidecars.
