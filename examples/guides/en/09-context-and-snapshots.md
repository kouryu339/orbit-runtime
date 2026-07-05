# 9 Context Structure and Snapshot Mechanism

This guide covers only the message order sent to the model, the tail snapshot,
and the position of conversation summaries. Here, snapshot means only the
dynamic context written by `conversation.set_dynamic_snapshot`. It does not
mean a conversation archive or a Runtime state export.

## 9.1 Message Order

Each LLM call is assembled in this order:

```text
1. system message
   - system Skills
   - role Skill
   - active feature Skills
   - visible tool and workflow descriptions

2. conversation history
   - user, assistant, and tool messages
   - a summary, when one exists
   - the current user message

3. final dynamic-context user message
   - the current Agent's host dynamic snapshot
   - recorder chain, current plan, retrieval, and env/runtime context
```

The fixed relationship is:

```text
system -> history (including summary) -> final dynamic context (including snapshot)
```

Do not inject undefined fixed context through the cluster. Stable
responsibilities, behavior rules, and capability boundaries belong in the role
Skill. Task-specific tool rules belong in feature Skills activated through
`UpdateSkills`.

## 9.2 Tail Snapshot

The host publishes current business context for one Agent with
`conversation.set_dynamic_snapshot`:

```json
{
  "schema": "agent-runtime-command/v1",
  "type": "conversation.set_dynamic_snapshot",
  "payload": {
    "conversation_id": "conv-current",
    "agent_id": "service-agent-1",
    "field_name": "current_order",
    "text": "Order SO-1008 is awaiting payment."
  }
}
```

Runtime stores the latest text by `(conversation_id, agent_id, field_name)`.
Writing the same field again replaces its previous value. An Agent may have
multiple fields, but they do not become multiple model snapshots. Runtime sorts
them by field name, combines their text, and injects them into one dynamic
context user message fixed at the absolute tail of every thinking round. Field
names are update and ordering keys for the host and are not shown to the model.

The tail message identifies its content as current supplemental facts for the
preceding request, not as a new user request. Snapshot text is never placed in
the system message or inserted into the middle of history.

Use the snapshot for changing host-owned state such as the current document,
order, selection, or page object. Put stable role rules in the role Skill,
task-specific capability rules in feature Skills, and conversational facts that
must become history in the ledger.

## 9.3 Summary Position

A summary is neither a snapshot nor a system instruction. After compaction,
Runtime writes a `Summary` ledger record. The frontend does not render that
record as a chat message, but the next LLM call renders it as a user context
message describing the compressed earlier conversation.

The summary remains in conversation history, followed by retained subsequent
history and the current user message. The final dynamic context comes last:

```text
system
-> summary and subsequent history
-> current user message
-> final dynamic context containing the latest snapshot
```

The summary records what happened earlier. The tail snapshot states what the
external world looks like now. They must not be merged or used interchangeably.

## 9.4 Use Cases

Prefer snapshots for external state that changes frequently and materially
affects task accuracy. The typical trigger is a tool operation that may change
frontend or external state unpredictably, or a predictable change whose actual
result must still be observed before deciding whether the task is complete.

Typical cases include:

- browser structure changed by clicks, navigation, dialogs, asynchronous loading, or partial updates;
- frontend view, selection, form, loading, error, and available-action state;
- editor document, selection, cursor, diagnostics, and post-tool file structure;
- workflows that must inspect updated business state to verify a tool result or choose the next action.

After a click, a page may navigate, open a dialog, remain loading, or reject the
operation. A successful tool call alone does not prove the task result. The
host should refresh the browser or frontend snapshot so the model judges the
real post-operation state. This creates the loop:

```text
read snapshot -> invoke tool -> recapture and replace snapshot -> continue or verify
```

Low-frequency stable information that does not affect later tool selection or
completion checks usually does not belong in a snapshot.

## 9.5 Update Strategies

The host decides when to recapture and inject snapshots. Runtime only provides
Agent- and field-scoped replacement and reads the latest value during thinking.
It neither observes host state nor imposes one refresh policy.

Two common strategies are available:

### 9.5.1 Observe State Changes

The host can observe frontend, browser, editor, or business state and call
`conversation.set_dynamic_snapshot` whenever relevant state changes. This fits
state that may change through user actions, asynchronous loading, pushed
events, or other processes in addition to Agent tools.

### 9.5.2 Update Within Tools

The host can also embed snapshot refresh logic in an external Tool. After the
Tool performs a click, edit, navigation, or business mutation, it reads the
real resulting state, replaces the relevant snapshot, and only then returns
the Tool result to Runtime. The next thinking round receives both the result
and the refreshed tail snapshot.

```text
receive Tool call
-> perform external operation
-> await the necessary resulting state change
-> recapture real state
-> replace snapshot
-> return Tool result
-> next thinking round reads the latest snapshot
```

Both strategies may be combined. Observation catches user and asynchronous
changes, while Tool-integrated refresh provides an ordered post-operation
observation for Agent-initiated actions. Keep each `field_name` semantically
stable and prevent an older concurrent observation from overwriting a newer
one.

Do not use snapshots for role rules, task-specific tool rules, conversation
history, durable host storage, or credentials. Those belong in the role Skill,
feature Skills, ledger, host persistence layer, and transport configuration
respectively.

## 9.6 Design Principles

The boundary is: the host owns current state, while the ledger owns
conversation history. The model should not infer what the browser, frontend,
or editor must look like from a tool result. The host observes the real state
again after the operation, and Runtime keeps that observation close to the next
inference.

Snapshots use replacement rather than append semantics. Each `field_name` is a
stable slot whose new value replaces its old value. This prevents changing
state from inflating the ledger. Multiple slots are still combined into one
tail message, so state synchronization is not misrepresented as several user
requests.

Fixed tail placement provides freshness, isolation from system and historical
context, and a deterministic read position even after history compaction.
Snapshot text should state facts rather than issue a new task. User intent still
comes from the current user message; the snapshot only describes the current
environment in which that request should be answered.
