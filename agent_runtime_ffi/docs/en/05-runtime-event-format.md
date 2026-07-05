# 5 Runtime Event and Host Transport Contract

ABI 1 natively uses the `agent_runtime_next_event_v1` pull queue and has no
callbacks. SSE, MQ, Redis Stream, and similar transports are host adapters after
the host receives event JSON.

```text
conversation bus -> Runtime export -> handle queue -> next_event_v1
-> host adapter -> UI / SSE / MQ / Stream
```

Events use `agent-runtime-event/v1` and carry cluster/runtime identifiers,
conversation id, process-local global and conversation sequences, event id,
UTC timestamp, source, type, and payload. Sequence numbers are ordering aids
inside one Runtime instance, not durable cross-process cursors.

This document only describes the event envelope and host transport shape. The
stable event surface is maintained in
[`07-runtime-persistence-and-recovery-contract.md`](07-runtime-persistence-and-recovery-contract.md).
Do not treat this document as a second event-type registry.

Tool approval does not use a standalone stable event outlet. The pending
approval list is carried by `frontend:state_snapshot.payload.pending_permissions`,
and the matching tool bubble state is carried by ledger records with
`metadata.subtype = "tool_call_permission_requested"`. Hosts respond with the
`conversation.resolve_tool_permission` command using `conversation_id`,
`tool_call_id`, and `decision`; no second permission-request id exists.

LLM usage/error facts are carried by `conversation.ledger_delta` records with
`metadata.subtype = "llm_usage"` or `"llm_error"`. Runtime does not export a
separate public `llm_usage` / `llm_error` event stream. This matters for background Agents because
`frontend:state_snapshot` is a UI projection and may only include the current
focus.

The queue is consumptive. TIMEOUT means no event arrived. Replay, fan-out,
global cursors, authentication, and SSE heartbeats belong to the host. A host
SSE relay places the complete Runtime envelope in `data:` and generates its own
`id:`. Do not tail logs as a state protocol, and do not treat command admission
as the final assistant result.
