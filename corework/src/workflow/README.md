# Workflow Architecture

This module contains the node-based workflow engine used by `corework`.

The design is inspired by Blueprint-style execution: exec pins carry control
flow, data pins carry typed values, and nodes are grouped by execution semantics.

## Directory Layout

```text
workflow/
  core/        Core types: pins, data values, connections, node outputs
  nodes/       Node traits and built-in node families
  execution/   Execution context, stack frames, flow state, node state
  builder/     Blueprint builder helpers
  registry/    Node metadata and factory registry
  workflows/   Draft workflow model, recorder, executor, snapshots, commands
```

Legacy files such as `blueprint.rs`, `control_nodes.rs`, `data_nodes.rs`, and
`dynamic_node.rs` remain for compatibility while the newer module layout evolves.

## Node Semantics

| Node type | Trait | Exec pins | Side effects | Examples |
| --- | --- | --- | --- | --- |
| Pure | `PureNode` | No | No | Add, Multiply |
| Impure | `ImpureNode` | Yes | Yes | Branch, ForLoop |
| Event | `EventNode` | Output | Yes | Custom event |
| Latent | `LatentNode` | Yes | Yes | Delay, async load |

## Execution Context

`ExecutionContext` builds on existing runtime services:

- stack frames for local variables and return addresses;
- `ScopedCache` for scoped persistent data;
- `EventBus` for event-driven workflows;
- node state for loops, DoOnce/DoN, delay, and similar stateful nodes;
- execution-flow tracking and maximum-step guards.

## Design Principles

- Clear separation between pure computation and side-effecting nodes.
- Explicit control flow through exec pins.
- Typed data flow through data pins.
- Wildcard pins for generic data types with type inference.
- Backward compatibility for existing workflow APIs.
