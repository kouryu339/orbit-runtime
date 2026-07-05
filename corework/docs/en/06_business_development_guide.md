# 6 Business Development Guide

This guide describes how to add business capabilities on top of `corework`.

## 6.1 Recommended Flow

1. Define domain data types.
2. Implement small `SystemOperation` units.
3. Register systems through the local registration pattern.
4. Compose operations through workflows when orchestration is needed.
5. Emit events for observable runtime progress.
6. Use scoped cache for intermediate or per-session state.

## 6.2 System Design

Prefer narrow systems with clear input and output types:

```text
ValidateInput -> LoadState -> ExecuteAction -> PersistResult -> EmitEvent
```

This keeps each operation testable and makes workflow composition easier.

## 6.3 Error Handling

Use typed errors for expected failures and reserve generic errors for unexpected
runtime problems. Tool-facing errors should be converted into protocol-level
error codes and structured messages.
