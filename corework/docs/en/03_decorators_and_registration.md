# 3 Decorators and Registration

`corework` uses Rust macros and registries to make systems and nodes discoverable
at runtime.

## 3.1 System Registration

`#[buns_system]` marks a type as a `SystemOperation` candidate. Registered
systems can later be collected and exposed to the Agent Runtime or workflow
engine.

## 3.2 Node Registration

Workflow nodes use metadata registration so editors and runtime loaders can
discover:

- node type;
- display name and category;
- input and output pins;
- permissions;
- factory function.

## 3.3 Why Registration Matters

Registration lets tool and workflow capabilities be linked into the binary
without central manual wiring. For sidecar tools, the same idea is represented by
RPC metadata returned from `AgentToolService.ListTools`.
