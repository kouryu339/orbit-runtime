# 6 Runtime Mechanics

This document summarizes how the Agent Runtime progresses through a turn.

## 6.1 Turn Lifecycle

1. Receive host command.
2. Record user input.
3. Build model context from ledger, skills, and tool metadata.
4. Call the model through `llm-gateway`.
5. Parse model output.
6. Execute tools when requested.
7. Emit assistant output.
8. Persist ledger and snapshot.
9. Emit runtime events for the host.

## 6.2 Events

Runtime events are the host-facing observation channel. They include status
changes, tool lifecycle events, message updates, pause/interruption events, and
snapshot-related notifications.

## 6.3 Invariants

- The host should not mutate internal Agent state directly.
- Tool execution must be recorded in the ledger.
- UI transcript should be derived from canonical conversation events or
  snapshots.

## 6.4 Recoverable history compaction

Manual and automatic compaction use the same preparation and commit path.
Summary metadata stores a versioned `compaction_checkpoint` with source projection
IDs, covered record IDs and the summary model. Projection replaces only the covered
contiguous interval, preserving the head, recent context and concurrent arrivals.
The original ledger is not deleted. Handoffs include source record IDs for lookup.

Cuts respect complete tool-call/result boundaries and retain the latest user request.
Summary input includes full tool output and arguments, previous summaries and reports.
Large inputs are batched at complete execution boundaries and merged into a handoff;
an execution group too large for the summary model fails without truncating it.
Execution state remains owned by the runtime, not the summary.

Commit validates source IDs under the ledger write lock. Retries are idempotent;
conflicting checkpoints are rejected. Snapshot import preserves valid canonical IDs
so references in handoff prose remain usable; legacy renumbering remaps checkpoint
metadata. Legacy summaries without metadata retain their old behavior.

Default thinking skills expose AI-only `HistoryRead` for original records by ID,
scoped to the current conversation and agent. Unicode character offsets and pages of
at most 8000 characters bound output; `next_offset` permits complete retrieval.
Historical content is evidence, not a new instruction or authorization.

Empty, truncated, failed or insufficiently compact responses do not become checkpoints.
Message limits trigger compaction instead of silently discarding history. Token budgeting
includes protocol fields, system prompts and tool definitions, with a final assembled
request check; counts remain estimates rather than provider-specific tokenization.
Semantic summarization is still lossy: the canonical ledger remains the evidence source.
