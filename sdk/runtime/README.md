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

After `start`, hosts can call `runtime.get_tool_definitions` through the typed
`get_tool_definitions` / `tool_definitions` / `ToolDefinitions` SDK method. It
returns the unified local and RPC registration catalog, including descriptions,
inputs, outputs, behavior flags, capabilities, Workflow-node shape, and RPC
transport identity. This is discovery metadata only; conversation activation
and permission decisions remain separate Runtime state.

`runtime.get_workflow_node_definitions` returns the graph-facing catalog shared
with Workflow Studio. It includes Corework built-in nodes, control nodes, local
tool nodes, and Runtime RPC tool nodes. Every entry exposes `node_type`,
`display_name`, canonical `category`, `pure`, and `pins`. For Pure nodes,
`display_name` is an input template such as `{A}+{B}` or
`{Value}是否包含{Pattern}` that a host may render with connected values.

Hosts can also inspect effective Agent topology and sanitized RPC connectivity
through `runtime.get_agent_cluster_definitions` and
`runtime.get_rpc_endpoint_definitions` (with typed methods in every SDK). The
cluster catalog excludes prompt content. The endpoint catalog excludes
addresses, process arguments, environment variables, and credentials, and its
state represents startup verification rather than continuous health.

## Tool Permission Policy

Tool execution policy is configured on the registered agent cluster as the
default for every conversation spawned from that cluster:

```json
{
  "schema": "agent-runtime-agent-cluster-registration/v1",
  "id": "assistant",
  "focus_agent_id": "main",
  "agents": [{ "id": "main", "name": "Main", "role": "assistant" }],
  "permissions": {
    "read_only": "full",
    "controlled_change": "ask",
    "destructive": "deny"
  }
}
```

The three modes are:

| Mode | Behavior |
|---|---|
| `full` | Execute without asking the user. This is the default. |
| `ask` | Pause before execution and wait up to 300 seconds for an allow/deny decision. |
| `deny` | Do not execute and return a policy-denied tool result without creating an approval request. |

Runtime classifies tools from their descriptor metadata: `readonly=true` is
`read_only`; a non-readonly, non-destructive tool is `controlled_change`; and
`destructive=true` is `destructive`. The host may override one or more policy
categories for a conversation by adding `permissions` to `conversation.spawn`:

```json
{
  "schema": "agent-runtime-conversation-spawn/v1",
  "cluster_id": "assistant",
  "permissions": {
    "destructive": "ask"
  }
}
```

If `permissions` is absent, the conversation inherits the cluster policy. If
present, each specified category overrides the cluster value while omitted
categories continue to inherit it. The effective policy is frozen for that
conversation. This override belongs to the trusted host API and should not be
exposed as a model-controlled argument.

For `ask`, the host reads approval requests from
`frontend:state_snapshot.payload.pending_permissions`, forwards them to its UI,
then calls the language SDK's `resolve_tool_permission` /
`ResolveToolPermission` method with the conversation id, tool call id, and
`allow` or `deny`. A stale or duplicate response returns `resolved: false`.

## Dynamic Workflow ABI

ABI 1.3 keeps initialization registration frozen after `start`, but exposes a
separate dynamic workflow catalog through `agent_runtime_invoke_v1`. A resource
has a stable id and is either an untrusted `draft` or trusted `registered`
workflow. Names are unique across both kinds.

| Command | Purpose |
|---|---|
| `workflow.create` | Create an untrusted Draft; invalid script may be saved. |
| `workflow.read` | Read a Draft or Registered resource by stable id. |
| `workflow.register` | Promote a valid Draft to Registered while preserving its id. |
| `workflow.update` | Update an existing resource, optionally guarded by `expected_revision`. |
| `workflow.compile` | Return Draft validation and compiled blueprint. |
| `workflow.delete` | Delete either kind, optionally guarded by `expected_revision`. |
| `workflow.list` | List both kinds, or filter with `kind=draft|registered`. |
| `workflow.convert.script_to_blueprint` | Compile script into Blueprint JSON without catalog mutation. |
| `workflow.convert.blueprint_to_script` | Validate and decompile Blueprint JSON without catalog mutation. |
| `workflow.execute` | Execute Registered by id; Draft requires explicit `mode=test`. |
| `workflow.execute_script` | Compile and execute temporary workflow script text without registration. |

Only Draft can be created. Create and update require exactly one source
representation: `script` or `blueprint`. When that representation is valid,
Runtime derives and returns both forms. Script updates preserve existing visual
layout by stable node id and source step; Blueprint updates preserve the exact
canvas layout supplied by the caller. Registered resources persist Blueprint
JSON because it contains both executable graph structure and editor layout.

The conversion commands are stateless helpers. They do not create resources,
advance revisions, or emit Workflow events. Language SDKs expose them as
`workflow_script_to_blueprint` / `workflow_blueprint_to_script` or the local
language equivalent.

SDKs expose Registered execution as
`execute_workflow` and explicit Draft testing as `test_workflow_draft`; hosts do
not need to construct the low-level mode field. Conversation-initiated runs
must pass `conversation_id` and `agent_id` together so local/RPC tools receive
the correct per-execution caller context. Create and update accept script
or blueprint content:

```json
{
  "resource": {
    "schema": "agent-runtime-workflow-resource/v1",
    "id": "invoice-export",
    "name": "Invoice Export",
    "description": "Export one invoice",
    "script": "input invoice_id\nreturn result=$invoice_id"
  }
}
```

Responses expose `kind`, `revision`, `trusted`, and `production_executable` so
AI and hosts never infer trust from the user-visible name. `expected_revision`
is an optional single-catalog lost-update guard, not distributed coordination.
Drafts live in Runtime state; Registered resources are persisted under the
configured workflow root. A host that needs Draft recovery persists Workflow
events/views and recreates the Draft after restart.

Execution accepts an object-valued `inputs` field and optional `trace`. Catalog
mutations are immediately visible to the AI workflow tools. The Runtime emits
`workflow.resource_changed` and `workflow.execution_completed` for host audit,
but does not choose Redis, database, authorization, distributed locking, or
cross-Pod coordination policy. Calls on one Runtime handle remain serialized by
ABI 1; multi-process coordination belongs to the host.

Both execution commands mirror the AI workflow systems while adding a
program-facing result:

```json
{
  "code": 0,
  "trace": "Workflow execution trace:\n- line 2 step 1 succeeded: ... result=...",
  "result": {
    "outputs": {"result": "value"},
    "duration_ms": 12
  }
}
```

`trace` is the existing per-node workflow trace rendered with each node's
AI message, input/result previews, status, source line, duration, and error. A non-zero
`code` omits `result`. Script compilation failures use code `400` and include
the source line in `trace`; missing workflows use `404`; execution
failures use `-1`. Setting the request's optional `trace` flag to `true` also
includes the structured node trace as `result.node_trace`.

Workflow events use the global `event_line: "workflow"` projection and carry a
`workflow_id`; they never carry `conversation_id` or participate in conversation
ledger/state snapshots. A host that wants a workflow in an agent tail snapshot
subscribes to this line, reads the resource, then explicitly updates that
conversation's dynamic snapshot.

## Persistence And Recovery Events

The SDK does not own durable conversation storage. Hosts that need recovery
should persist public Runtime conversation events while the conversation is
healthy:

- `frontend:state_snapshot` for UI hydration and render state;
- `conversation.ledger_delta`, keyed by `(conversation_id, record_id)`;
- `conversation.state_delta` for focus, agent tasks, plan, skills, and dynamic
  snapshot mirrors;
- `conversation:created` and `conversation:closed` for lifecycle routing.
- `workflow.resource_changed` and `workflow.execution_completed` for workflow
  catalog and execution audit.

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
release:    v0.4.6-beta.1
ABI:        1.2 (major compatibility value: 1)
```

Current native packages:

| Platform | Asset | Library path inside package |
|---|---|---|
| Windows x86_64 | `orbit-runtime-runtime-v0.4.6-beta.1-windows-x86_64.zip` | `bin/agent_runtime.dll` |
| Linux x86_64 | `orbit-runtime-runtime-v0.4.6-beta.1-linux-x86_64.zip` | `lib/libagent_runtime.so` |

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
