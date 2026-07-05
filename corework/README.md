# Corework

`corework` is the orchestration foundation of `ai-framework`.

It provides the runtime primitives used by the Agent layer: state machines,
scoped cache, event bus, workflow execution, system registration, node metadata,
and the RPC Tool protocol.

## Core Concepts

- `SystemOperation`: a typed business operation that can be registered and
  invoked by name.
- `Context`: execution context containing cache, event bus, telemetry, and
  runtime services.
- `World` and `Instance`: global resources and per-run scoped data.
- `ScopedCache`: namespaced cache isolation for workflows and runtime sessions.
- `Blueprint Workflow`: node-based orchestration with exec pins and data pins.
- `NodeRegistry`: compile-time and runtime node metadata registry.
- `RPC Tool`: gRPC protocol for out-of-process tools.
- `Saga`: long-running workflow coordination and compensation primitives.

## Workflow Layer

The workflow implementation includes:

- control-flow nodes such as start/end, branch, loop, and break;
- data nodes for math, logic, strings, arrays, objects, variables, and type
  conversion;
- workflow recording, snapshots, flowchart generation, and execution helpers;
- JSON blueprint loading and dynamic node invocation.

## RPC Tool Protocol

The stable protocol source is:

```text
proto/corework_agent_tool_v1.proto
```

Host-side runtimes discover tools through `AgentToolService.ListTools` and invoke
them through `AgentToolService.Execute`. Tool results use the `AIOutput` envelope.

## Documentation

Chinese documents are in:

```text
docs/zh/
```

English companion documents are in:

```text
docs/en/
```

Workflow-specific notes:

```text
src/workflow/README.md
src/workflow/en/README.md
```
