# 7 Runtime FFI Persistence and Recovery Host Contract

This document describes only the stable events and calls exposed by FFI. The
state-machine recovery algorithm, ledger-tail analysis, and unresolved tool-call
handling live in `ai-assistant`; see
[`ai-assistant/docs/en/05_agent_and_persistence.md`](../../../ai-assistant/docs/en/05_agent_and_persistence.md).

## 7.1 Event Surface

FFI does not expose internal conversation events directly. `ai-assistant`
AgentGateway normalizes ledger, focus, task, plan, skills, and dynamic snapshot
changes into stable export events. FFI wraps those events as
`agent-runtime-event/v1` for the host.

Stable events for hosts:

```text
conversation:created
conversation:closed
conversation.ledger_delta
conversation.state_delta
frontend:state_snapshot
```

`frontend:state_snapshot` is the UI render channel. `conversation.ledger_delta`
and `conversation.state_delta` are the recovery persistence channels.
Tool permission state is not a standalone event outlet: pending approvals are
in `frontend:state_snapshot.payload.pending_permissions`, and the user-visible
tool lifecycle fact is a `conversation.ledger_delta` record with
`metadata.subtype = "tool_call_permission_requested"`.
LLM usage/error facts are not standalone stable host events. The authoritative
usage/error facts for all focused and background Agents are
`conversation.ledger_delta` records with `metadata.subtype = "llm_usage"` or
`"llm_error"`. Do not depend on internal event names, telemetry mirrors, or
internal event splitting.

## 7.2 What Hosts Persist

Hosts that need pod-loss, process-crash, or migration recovery should persist
conversation-level deltas continuously:

- `conversation:created`: conversation manifest, cluster, route, and business binding.
- `conversation.ledger_delta`: idempotent ledger append keyed by
  `conversation_id + record_id`. This includes assistant/user/tool records and
  gateway facts such as LLM usage/error for background Agents.
- `conversation.state_delta`: focus, agent task, agent skills, agent plan, and
  dynamic snapshot mirrors.
- `conversation:closed`: lifecycle end for that conversation.

Hosts that need usage accounting, cost attribution, audit, or diagnostics
should derive those facts from `conversation.ledger_delta` records. Do not build
recovery or accounting on a separate `llm_usage` / `llm_error` event stream.

Exporting one snapshot only at close time does not cover accidental crashes.
Persist deltas during healthy runtime; snapshots are useful for hydration,
migration, manual save, or delta reconciliation.

## 7.3 Host-Owned External State

Runtime does not keep external resources alive. The host must rebind or
republish these after recovery:

- business objects, database transactions, file handles, browser pages, external sessions;
- host-owned dynamic snapshots written through `conversation.set_dynamic_snapshot`;
- time-sensitive business facts whose validity only the host can judge.

After recovery, bind current external resources first, then republish current
dynamic snapshots so the model does not read stale host-owned state.

## 7.4 Recovery Calls

After an unexpected failure, the host may rebuild an
`agent-runtime-conversation-snapshot/v1` from persisted ledger/state deltas and
then call:

- `conversation.spawn_from_snapshot`: create a new conversation from the snapshot.
- `conversation.import_snapshot`: import the snapshot into a target conversation.

FFI passes the snapshot to `ai-assistant` recovery. The core runtime decides
whether the restored entry is `thinking`, `executing`, or `suspended`; FFI does
not synthesize history in the event layer.

## 7.5 Shutdown and Event Drain

Runtime shutdown closes conversations one by one:

1. Runtime enumerates existing conversations and closes each one.
2. A conversation may emit final ledger/state deltas before `conversation:closed`.
3. Different conversation states may produce different shutdown event sequences.
4. Once the FFI handle enters shutdown/closing, normal invoke calls are rejected,
   but `next_event` remains the drain path.
5. Do not destroy the handle after the first `conversation:closed`; keep draining
   events and tracking every host-owned conversation.
6. Destroy the handle only after all tracked conversations are closed and the
   event queue is drained.

`conversation:closed` means one conversation ended. It does not mean the Rust
runtime handle has ended.

## 7.6 Idempotency

Recommended idempotency keys:

```text
ledger: conversation_id + record_id
state delta: conversation_id + op + op-specific id/version
conversation lifecycle: conversation_id + lifecycle event type
```

If the host detects event sequence gaps or frontend reconnects, reconcile from
persisted deltas and, when needed, a snapshot. Do not treat
`frontend:state_snapshot` as the only durable source.
