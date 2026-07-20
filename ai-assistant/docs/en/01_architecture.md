# 1 AI Assistant Architecture

`ai-assistant` is the Agent Runtime layer of `orbit-runtime`.

It connects user messages, model gateway calls, skills, tools, runtime events,
ledger persistence, and host-facing snapshots.

## 1.1 Main Components

- Conversation state machine.
- Skill loader and prompt composer.
- LLM gateway adapter.
- Tool runner and runtime tool registry.
- Ledger and persistence systems.
- Agent routing and cluster primitives.
- Runtime event definitions.

## 1.2 Runtime Flow

```text
Host command
  -> Agent Runtime
  -> thinking state
  -> model gateway
  -> optional tool execution
  -> assistant response
  -> ledger + snapshot + runtime events
```

## 1.3 Boundary

The runtime is designed to be embedded through `agent_runtime_ffi`. Host
applications send JSON commands and receive runtime events rather than calling
internal Agent state directly.
