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
