# 5 Agent and Persistence

Persistence lets the runtime recover conversation state and expose consistent UI
snapshots to host applications.

## 5.1 Persisted Data

- User messages.
- Assistant messages.
- Tool calls and tool results.
- Runtime snapshots.
- Conversation index metadata.
- Logs.

## 5.2 Ledger

The ledger is the append-only record of turn-level activity. It is used to build
conversation snapshots and to restore context after host restarts or event gaps.

## 5.3 Snapshot

A snapshot is a host-friendly view of current runtime state, including
conversation messages, Agent definition references, conversation-scoped Agent
instances, the task board, and status information. Registered Agent
definitions, permissions, tools, Skills, and model policy remain registry-owned
canonical configuration; a conversation snapshot does not duplicate or
override them.

## 5.4 Recovery Entry

Recovery rebuilds the state-machine site from the ledger tail instead of
prepending synthetic history:

- a user tail enters `thinking`;
- a clean assistant tail with no tool calls enters `suspended`;
- an assistant tool call or unresolved `tool_call_started` enters `executing`;
- a closed tool result enters `thinking`.

Restoring `thinking` or `suspended` only restores the entry state; it does not
send a model request during import. Restoring `executing` writes pending tools,
tool call ids, and recovery results into agent cache so the executing state can
consume them.

## 5.5 Unresolved Tools

Unresolved tool calls are handled by effect:

- read-only tools may run again as a fresh query;
- non-read-only, destructive, or unknown-safety tools are not replayed. Runtime
  writes a recovery tool result with the original `tool_call_id`, telling the AI
  to verify external state, report uncertainty, or let a child agent report back
  to the main agent.

For multi-agent conversations, unresolved tools are grouped by `agent_id`; each
agent keeps its own `tool_call_id`.

## 5.6 Host Guidance

Hosts should use snapshots for recovery and `frontend:state_snapshot` events for
canonical transcript updates.

`conversation.state_delta` operations `agent.upsert` and `agent.retired`
describe the appearance, state change, and retirement of runtime instances.
They carry only the instance id, registered definition reference, display name,
and runtime state. They do not copy registered permissions or tool policy.
