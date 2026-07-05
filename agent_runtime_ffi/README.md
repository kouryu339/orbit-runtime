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
