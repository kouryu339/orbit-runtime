# 3 Decorators and Registration

`corework` uses Rust macros and registries to make systems and nodes discoverable
at runtime.

## 3.1 System Registration

`#[buns_system]` marks a type as a `SystemOperation` candidate. Registered
systems can later be collected and exposed to the Agent Runtime or workflow
engine.

### Local Tool Display Names

For every local AI tool, `name` remains the stable machine identifier while
`display_name` is a user-facing input/output relationship template. Reference
declared parameter and output fields with single braces, for example:

```text
Wait {timeout_ms} ms and return {wake_reason}
Open {workflow_id} and return draft {draft_name}
Write {content} to {file_name} and return {path}
```

Every declared input and output must appear as `{field}` in the local tool's
template, and a template must not reference undeclared fields. Tools with no
declared fields may use a plain action phrase. This convention applies to local
tools only; RPC tool display names remain owned by their endpoint descriptors.

## 3.2 Node Registration

Workflow nodes use metadata registration so editors and runtime loaders can
discover:

- node type;
- display name and category;
- input and output pins;
- permissions;
- factory function.

For graph nodes, `display_name` is also the readable value template. Pure nodes
must reference every data input with single braces, for example `{A}+{B}` or
`{Value} contains {Pattern}`. Control nodes use an action template such as
`Branch on {Condition}`. Referenced fields must match declared pins.

## 3.3 Why Registration Matters

Registration lets tool and workflow capabilities be linked into the binary
without central manual wiring. For sidecar tools, the same idea is represented by
RPC metadata returned from `AgentToolService.ListTools`.
