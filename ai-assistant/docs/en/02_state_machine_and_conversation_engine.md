# 2 State Machine and Conversation Engine

The conversation engine models each turn as explicit runtime states.

## 2.1 States

- `thinking`: prepare context, call the model, and decide the next action.
- `executing`: run tool calls and collect tool results.
- `saying`: emit the assistant message and finalize the turn.
- `asking`: request missing user input or clarification.
- `suspended`: pause execution until the host resumes or interrupts.

## 2.2 Why a State Machine

State machines make long-running Agent behavior observable and recoverable. The
host can see which phase is active, handle interruptions, and rebuild UI state
from snapshots and events.

## 2.3 Conversation Source of Truth

The canonical transcript should come from FFI `frontend:state_snapshot` events or
runtime snapshots, not from partial debug events.
