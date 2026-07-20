# 1 Corework Architecture

`corework` is the orchestration substrate used by the Agent Runtime.

It provides reusable runtime primitives for stateful systems, workflow
execution, scoped data, events, tool registration, and RPC tool integration.

## 1.1 Layers

| Layer | Purpose |
| --- | --- |
| Infrastructure | `World`, `Instance`, `Context`, cache, event bus, telemetry. |
| System layer | `SystemOperation` units registered through macros or registries. |
| Workflow layer | Blueprint-style nodes, pins, graph validation, and execution. |
| Coordination layer | Saga-style long-running flow and compensation primitives. |

## 1.2 Design Goals

- Keep business operations small and composable.
- Make data flow explicit through typed inputs and outputs.
- Isolate runtime state through scoped cache and instance boundaries.
- Support both direct Rust usage and host-language integration through FFI and
  RPC tools.

## 1.3 Key Modules

- `cache`, `scoped_cache`, `workspace`, `world`
- `event`, `event_line`
- `system`, `module`, `ai_system`
- `workflow`
- `rpc_tool`, `rpc_proto`
- `rag`
