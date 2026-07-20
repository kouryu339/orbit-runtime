# 5 Connect the Host, Runtime, and Frontend

The host application loads the native Runtime library through a Runtime Host
SDK and relays commands and events to its frontend. A browser or UI must not
load the native library directly.

For the complete setup sequence from tool descriptors to conversation spawn,
start with [SDK Runtime Connection Flow](01-sdk-runtime-connection-flow.md). This
page focuses on the host/runtime/frontend boundary after those inputs are
prepared.

```text
Frontend <-> Host API/SSE/WebSocket <-> Runtime Host SDK <-> Native Runtime
                                                    |
                                                    +-> Tool sidecars
```

Use this startup order:

```text
load library and verify ABI
  -> create Runtime
  -> register resources
  -> register LLM configuration
  -> register agent clusters
  -> start
  -> spawn conversations
  -> poll and relay events
```

The three registration steps before `start` are the core Agent setup:

| Config | Command | Why it matters |
|---|---|---|
| Resources | `runtime.register_resources` | Registers Skill roots, reusable Agent profiles, tool/RAG endpoints, workflows, and data roots. |
| LLM providers | `runtime.register_llm` | Registers providers, model ids, credentials/base URLs, context windows, and current model selection. |
| Agent cluster | `runtime.register_agent_cluster` | Creates concrete Agent instances from profiles and declares the initial focus seed. Runtime owns later focus handoff and routing. |

Do not treat these as optional examples. A frontend conversation window without
these registrations is only a shell; Runtime cannot form a real Agent context,
call a model, or route tools without them.

The host should expose only product-level operations to the frontend: create or
restore a conversation, send a message, pause work, obtain a recovery snapshot,
and subscribe to events. Keep resource registration, model administration, and
cluster setup behind host authorization. Agent routing and focus handoff are
Runtime built-ins; the host authorizes access to those commands instead of
managing the route itself.

Relay the complete `agent-runtime-event/v1` envelope through SSE, WebSocket,
Tauri events, or another host transport. On the frontend:

- treat `frontend:state_snapshot` as canonical conversation UI state;
- apply `payload.ledger_delta.record` to chat content;
- derive interaction availability from `payload.conversation_state`;
- correlate tool records by `call_id`;
- render `frontend:state_snapshot.payload.pending_permissions` in one approval
  shelf immediately above the composer; keep the tool bubble as status only;
- send allow/deny back through the host to
  `conversation.resolve_tool_permission`; this is a reverse command, not an
  event or an SSE write;
- read LLM usage and error facts from `conversation.ledger_delta` records whose
  `metadata.subtype` is `llm_usage` or `llm_error`; there is no separate public
  event stream for these facts;
- use snapshots for hydration and gap recovery, not as the normal update loop.

The host API used by the frontend should therefore include a narrowly scoped
permission endpoint such as:

```text
POST /api/tool-permission
{ conversation_id, tool_call_id, decision: "allow" | "deny" }
```

The host must authorize the conversation id before forwarding the command.
Stale or duplicate decisions return `resolved:false` and must not be treated as
a newly executed tool.

Runtime Host APIs are documented in [`sdk/runtime`](../../../sdk/runtime/README.md).
See the canonical
[`Frontend Message Contract`](../../../agent_runtime_ffi/docs/en/06-runtime-frontend-message-contract.md)
and [`Runtime Event Format`](../../../agent_runtime_ffi/docs/en/05-runtime-event-format.md)
for frontend payload details.

See [Context Structure and Snapshot Mechanism](09-context-and-snapshots.md) for
model message order, the tail dynamic snapshot, and summary placement.
Hosts that need durable conversations, pod-loss recovery, or shutdown event
draining should also read [Persistence and Recovery](10-persistence-and-recovery.md).

## 5.1 Runtime-wide LLM Headers

The host can use `runtime.set_auth_context` to inject custom headers into LLM
requests for the current Runtime. They apply to existing and future
conversations and propagate through automatic continuation and background Agent
drivers. Credentials are host transport configuration; they do not enter model
context, ledger, Skills, or dynamic snapshots.

Runnable references:

- [`examples/python_ctypes`](../../python_ctypes): Python Host SDK and a small browser frontend.
- [`examples/go_order_admin`](../../go_order_admin): Go Host SDK, HTTP/SSE, and a business frontend.
