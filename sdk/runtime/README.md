# Corework Runtime Host SDK

The Runtime Host SDK is for applications that embed the native Agent Runtime:
desktop apps, servers, service hosts, and test harnesses. It wraps the stable C
ABI and exposes lifecycle, command invocation, and event polling.

This SDK does not author tools. Use the RPC Tool SDKs under `sdk/rpctools/<language>`
to build sidecar tools.

## What The Host Must Prepare First

Before the host opens a conversation, the product must prepare:

```text
1. Tools       Tool set descriptors.
2. Configs     Runtime create options plus Resources/LLM/Cluster configs.
3. Skills      Role/feature Skills.
4. Connect     Runtime Host SDK connection and three config registrations.
5. Run         Runtime start and conversation spawn.
```

Create options are passed directly to the Runtime Host SDK. They are not a
config file and only cover process-level startup parameters:

```json
{
  "schema": "agent-runtime-create-options/v1",
  "log_level": "info",
  "language": "zh-CN",
  "restore_policy": "strict",
  "data_dir": "./data/runtime"
}
```

The three registration configs are mandatory for a real Agent host:

| Config | Purpose |
|---|---|
| resources | Skill roots, Agent profiles, tool/RAG endpoints, data/log/workflow roots. |
| LLM providers | Provider credentials/base URLs, model ids, context windows, current model. |
| agent cluster | Concrete Agent instances and the initial focus seed. Runtime owns later focus handoff and routing. |

The full product sequence is documented in
[`examples/guides/en/01-sdk-runtime-connection-flow.md`](../../examples/guides/en/01-sdk-runtime-connection-flow.md)
and
[`examples/guides/zh/01-sdk-runtime-connection-flow.md`](../../examples/guides/zh/01-sdk-runtime-connection-flow.md).

## Host Lifecycle

```text
load library
  -> verify ABI and capabilities
  -> create Runtime handle
  -> register resources
  -> register LLM providers
  -> register agent cluster
  -> start
  -> spawn conversation
  -> invoke commands + poll events
  -> shutdown while the event poller drains final events
  -> destroy
  -> unload library
```

`shutdown` timeout is retryable. `destroy` is only legal after shutdown
completes. Continue polling during shutdown if the host needs final
conversation-close, ledger, or state events.

## Layers

1. `c/include/agent_runtime.h`: frozen ABI declarations and ownership contract.
2. Language bindings: dynamic loading, UTF-8 conversion, error decoding, and
   native string ownership.
3. Language client: typed lifecycle, command envelope, event polling, and
   capability checks.
4. Product adapter: application-specific host APIs and frontend event relay.

Product adapters do not belong in the core SDK.

## Language Entries

| Language | Path | Status | Native boundary |
|---|---|---|---|
| C | [`c/include/agent_runtime.h`](c/include/agent_runtime.h) | stable ABI | direct C ABI |
| Python | [`python/README.md`](python/README.md) | wrapper | ctypes over C ABI |
| Go | [`go/README.md`](go/README.md) | wrapper | cgo over C ABI |
| Rust | [`rust/README.md`](rust/README.md) | crate wrapper | dynamic C ABI |
| C++ | [`cpp/README.md`](cpp/README.md) | header-only wrapper | dynamic C ABI |
| C# | [`csharp/README.md`](csharp/README.md) | design entry | P/Invoke over C ABI |
| Java | [`java/README.md`](java/README.md) | design entry | JNA/Panama/JNI over C ABI |

## Host API Boundary

Expose product-level conversation APIs to the frontend:

- create or restore conversation;
- send user message;
- pause or close work;
- get recovery snapshot;
- subscribe to Runtime events.

Keep resource registration, model administration, and cluster setup behind host
authorization. Focus handoff and agent routing are Runtime built-ins; the host
only decides which built-in commands are exposed to a caller.

## Persistence And Recovery Events

The SDK does not own durable conversation storage. Hosts that need recovery
should persist public Runtime conversation events while the conversation is
healthy:

- `frontend:state_snapshot` for UI hydration and render state;
- `conversation.ledger_delta`, keyed by `(conversation_id, record_id)`;
- `conversation.state_delta` for focus, agent tasks, plan, skills, and dynamic
  snapshot mirrors;
- `conversation:created` and `conversation:closed` for lifecycle routing.

LLM usage/error facts are carried by `conversation.ledger_delta` records with
`metadata.subtype = "llm_usage"` or `"llm_error"`. Do not build recovery,
accounting, or SDK integrations around a separate stable `llm_usage` /
`llm_error` event stream.

The Rust and Python SDKs expose a single public Runtime event bus with optional
filtered subscriptions. The Go SDK delivers the same public stream through
`EventSink`, and the C++ SDK exposes `next_public_event()`. SSE, MQ, Redis
Stream, replay cursors, heartbeats, and authorization belong to the host adapter
after it receives complete Runtime event envelopes.

Recovery is performed by the native Runtime through
`conversation.spawn_from_snapshot` or `conversation.import_snapshot` using a
durable `agent-runtime-conversation-snapshot/v1`. This is intentionally
separate from any tail snapshot exported during normal operation. The SDK only
transports commands and events.

## Distribution

Native Runtime artifacts are published from this repository's GitHub Release.
SDKs should use `release_manifest.json` as the compatibility source of truth:

```text
repository: kouryu339/orbit-runtime
release:    v0.4.0
ABI:        1
```

Current native packages:

| Platform | Asset | Library path inside package |
|---|---|---|
| Windows x86_64 | `orbit-runtime-runtime-v0.4.0-windows-x86_64.zip` | `bin/agent_runtime.dll` |
| Linux x86_64 | `orbit-runtime-runtime-v0.4.0-linux-x86_64.zip` | `lib/libagent_runtime.so` |

macOS is intentionally not published in this release because the release build
machine does not provide an Apple environment.

The Python SDK can download and verify the matching package:

```powershell
corework-runtime-fetch-native
```

All hosts can use the repository scripts:

```powershell
.\sdk\runtime\scripts\fetch-native-runtime.ps1
```

```bash
./sdk/runtime/scripts/fetch-native-runtime.sh
```

Rust, Go, and C++ hosts can also use the release helper constants in their
language SDKs or read `sdk/runtime/release_manifest.json`. After download, place
the native library in the loader path or pass its absolute path to the Runtime
Host SDK.
