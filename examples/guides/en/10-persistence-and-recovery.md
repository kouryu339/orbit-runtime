# 10 Persistence and Recovery

This guide is for application hosts. The core recovery mechanism lives in
`ai-assistant`; FFI and SDK layers expose stable events and recovery commands.
The host decides whether to persist them, where to store them, and when to send
them back to Runtime.

## 10.1 Events to Persist

Hosts that need refresh, migration, pod-loss, or process-crash recovery should
persist these events continuously:

- `conversation:created`: conversation manifest, cluster, routing, and business binding.
- `conversation.ledger_delta`: ledger records keyed by `conversation_id + record_id`.
- `conversation.state_delta`: focus, agent tasks, skills, plans, and dynamic snapshot mirrors.
- `conversation:closed`: the lifecycle end for one conversation.

Use `frontend:state_snapshot` for UI hydration and gap reconciliation, not as
the only durable source.

## 10.2 Recovery Flow

```text
read host persistence
  -> rebuild agent-runtime-conversation-snapshot/v1
  -> call conversation.spawn_from_snapshot or conversation.import_snapshot
  -> Runtime restores thinking/executing/suspended from the ledger tail
  -> host rebinds external resources
  -> host republishes current dynamic snapshots
```

Runtime does not simply rewind to the latest user message. A user tail resumes
in `thinking`; a clean assistant tail resumes in `suspended`; unresolved tools
resume in `executing`; a closed tool result resumes in `thinking`.

## 10.3 Unresolved Tools

During recovery:

- read-only tools may be executed again as a fresh query;
- non-read-only or unknown-safety tools are not replayed. Runtime writes a
  recovery tool result with the original `tool_call_id`, so the AI can verify
  external state, report uncertainty, or let a child agent report back to the
  main agent.

For multi-agent conversations, unresolved tools are grouped by `agent_id` and
each agent keeps its own `tool_call_id`.

## 10.4 Shutdown Persistence

During manual pod shutdown or runtime shutdown, Runtime closes conversations one
by one. Each conversation may emit final ledger/state deltas before
`conversation:closed`.

The host should keep draining the Runtime event queue until all tracked
conversations have emitted `conversation:closed`, final deltas are persisted,
and runtime shutdown has completed. A single `conversation:closed` event is not
a signal that the whole Runtime handle has closed.

## 10.5 External Host State

Runtime does not keep external business resources alive. After recovery, the
host must rebind databases, files, browser pages, external sessions, and then
republish current `conversation.set_dynamic_snapshot` values. Old dynamic
snapshots are last observations, not durable facts.

See also:

- [`ai-assistant/docs/en/05_agent_and_persistence.md`](../../../ai-assistant/docs/en/05_agent_and_persistence.md)
- [`agent_runtime_ffi/docs/en/07-runtime-persistence-and-recovery-contract.md`](../../../agent_runtime_ffi/docs/en/07-runtime-persistence-and-recovery-contract.md)
- [`sdk/runtime`](../../../sdk/runtime/README.md)
