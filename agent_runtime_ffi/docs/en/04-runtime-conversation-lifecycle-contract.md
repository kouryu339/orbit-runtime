# 4 Runtime and Conversation Lifecycle Contract

This document describes the current implementation, not a future plan. See
`01-runtime-ffi-usage-guide.md` for the C ABI contract.

## 4.1 Runtime

```text
create_v1 -> OPEN/unstarted
invoke registration commands
start_v1 -> OPEN/started
invoke commands + next_event pull
shutdown_v1 -> CLOSING -> CLOSED
destroy_v1 -> handle removed
```

When shutdown starts, new start/invoke calls are rejected. The runtime waits for
in-flight ABI calls, closes all conversations, Studio services, and event
producers. TIMEOUT is retryable. Destroy does not implicitly shut down the
runtime.

## 4.2 Conversation

`conversation.spawn` creates shared ConversationState, Gateway, Cluster, and
Agent instances from a registered cluster/profile. A successful result includes
conversation/scope/tenant/user/created_at fields.

`send_message`, `pause`, summary model changes, and compact commands go through
command admission. An accepted result only means the command was admitted; later
facts must be observed through the pull event stream. `conversation.close`
closes the command gate, stops Agent drivers, and removes the conversation from
the manager.

## 4.3 Snapshots

- `conversation.export_snapshot` exports `agent-runtime-conversation-snapshot/v1`.
- `conversation.spawn_from_snapshot` creates and restores a conversation from a
  durable `agent-runtime-conversation-snapshot/v1`. It is distinct from
  continuing from a tail snapshot exported for UI refresh or observation.
- `conversation.import_snapshot` imports a snapshot into a Runtime-owned conversation.
- `conversation.materialize` materializes registered state as a runnable conversation.
- `runtime.export_snapshot` exports a Runtime-level observation snapshot.

Host dynamic text is written through `conversation.set_dynamic_snapshot` using
conversation/agent/field. It has no separate C function. Dynamic text is not a
durable business truth; the host must republish it after recovery.

## 4.4 Multi-Agent

Fixed Agents are registered in the cluster and hand off focus. Background
Agents are created dynamically by task tools from profiles and do not steal
focus. The host may read task state through `conversation.agent_tasks`; the
model observes reports and ledger facts rather than a task-list polling tool.

## 4.5 Events

All canonical events enter the handle-private queue. The host reads
`agent-runtime-event/v1` through `agent_runtime_next_event_v1`. FFI does not call
callbacks. HTTP SSE, MQ, and Redis Stream are host-side adapters after the host
receives JSON events. The stable event surface is defined only by
[`07-runtime-persistence-and-recovery-contract.md`](07-runtime-persistence-and-recovery-contract.md).
