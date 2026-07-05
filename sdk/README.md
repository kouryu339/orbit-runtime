# Orbit Runtime SDKs

This directory is the public SDK entry point. Users usually face two different
integration jobs:

1. Build tools that Agents can call.
2. Embed and control the Runtime from a host application.

Those jobs use different SDK families and different trust boundaries.

## Choose The SDK

| You need to... | Use | Direction | Primary docs |
|---|---|---|---|
| Expose product capabilities as Agent tools | RPC Tool SDK | Runtime -> tool sidecar | Language folders under `sdk/rpctools/<language>` and `examples/guides/*/03-external-tools.md` |
| Embed the Agent Runtime in a desktop/server host | Runtime Host SDK | Host -> native Runtime | [`runtime/README.md`](runtime/README.md) and `examples/guides/*/01-sdk-runtime-connection-flow.md` |

Do not mix the two concepts:

- RPC Tool SDKs implement callable business tools. They do not create
  conversations or register Agent clusters.
- Runtime Host SDKs load the native Runtime, register configs, start the
  Runtime, spawn conversations, send messages, and poll events. They do not
  implement business tools.

## Recommended Integration Order

```text
1. Tools       Prepare callable sidecars or built-in tools.
2. Configs     Prepare resources, LLM providers, and agent cluster configs.
3. Skills      Write role/feature Skills and tool allowlists.
4. Connect     Use Runtime Host SDK and register the three configs.
5. Run         Start Runtime and spawn conversation.
```

The authoritative product flow is in
[`examples/guides/en/01-sdk-runtime-connection-flow.md`](../examples/guides/en/01-sdk-runtime-connection-flow.md)
and
[`examples/guides/zh/01-sdk-runtime-connection-flow.md`](../examples/guides/zh/01-sdk-runtime-connection-flow.md).

## Layout

```text
sdk/
  rpctools/
    python/     RPC Tool SDK for Python
    node/       RPC Tool SDK for Node.js
    go/         RPC Tool SDK for Go
    rust/       RPC Tool SDK for Rust
    cpp/        RPC Tool SDK for C++
    csharp/     RPC Tool SDK for C#
    java/       RPC Tool SDK for Java
  runtime/
    c/          Stable C ABI headers
    python/     Python Runtime Host SDK
    go/         Go Runtime Host SDK
    rust/       Rust Runtime Host SDK
    cpp/        C++ Runtime Host SDK
    csharp/     C# Runtime Host SDK design entry
    java/       Java Runtime Host SDK design entry
```

Future Runtime Host SDKs should be added under `sdk/runtime/<language>`.
Language wrappers must use the C ABI as their native boundary rather than
linking Rust implementation crates.

Current status:

| Family | Languages |
|---|---|
| RPC Tool SDK | Python, Node.js, Go, Rust, C++, C#, Java scaffold |
| Runtime Host SDK | C ABI, Rust, Python, Go, C++, C#/Java design entries |

## Stable Protocols

RPC Tool SDK:

```text
corework/proto/corework_agent_tool_v1.proto
```

Runtime Host SDK:

```text
agent-runtime-command/v1
agent-runtime-result/v1
agent-runtime-event/v1
agent-runtime-error/v1
agent-runtime-capabilities/v1
```

The Runtime Host SDK checks `agent_runtime_abi_version_v1()` and capabilities.
It must not infer compatibility from the product version.

## Native Runtime Release

Runtime Host SDKs are aligned with the native packages from:

```text
https://github.com/kouryu339/orbit-runtime/releases/tag/v0.4.0
```

Use `runtime/release_manifest.json` to resolve the platform archive, checksum,
and library path. Python can download the matching package with:

```powershell
corework-runtime-fetch-native
```
