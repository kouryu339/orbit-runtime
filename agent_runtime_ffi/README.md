# Agent Runtime FFI

`agent-runtime-ffi` builds the native `agent_runtime` dynamic library and owns
the stable C ABI boundary. The ABI exposes lifecycle, command invocation, event
polling, error retrieval, and runtime-owned string release. Runtime features are
negotiated through JSON capabilities and versioned command/event schemas.

ABI declarations and ownership rules are defined by
[`sdk/runtime/c/include/agent_runtime.h`](../sdk/runtime/c/include/agent_runtime.h)
and [`docs`](docs/README.md). Host applications should normally use a Runtime
Host SDK under [`sdk/runtime`](../sdk/runtime/README.md) instead of calling raw
FFI functions directly.

```text
cargo test -p agent-runtime-ffi
cargo clippy -p agent-runtime-ffi --all-targets -- -D warnings
```

Never free Runtime-owned strings with the host allocator; release them with
`agent_runtime_free_string_v1`.

## Runtime Implementation Map

`src/runtime.rs` is the Runtime composition root. Keep feature ownership in its
dedicated module:

| Module | Ownership |
|---|---|
| `runtime/conversation_operations.rs` | Spawn, initialization, close, materialize, conversation commands, and lifecycle bookkeeping. |
| `runtime/recovery.rs` | Snapshot import/export, ledger/state replay, and interrupted-tool repair. |
| `runtime/agent_cluster.rs` | Agent cluster registration, profile expansion, focus seed, and default tool policy. |
| `runtime/workflow_operations.rs` | Dynamic workflow catalog CRUD, execution response, trace, and workflow audit events. |
| `runtime/resources.rs` / `runtime/rpc.rs` | Resource registration and RPC tool discovery/projection. |
| `runtime/events.rs` | Stable host event projection and sequencing. |

Do not move these concerns back into `runtime.rs`; it should coordinate module
lifecycle and shared dependencies rather than become another feature owner.
