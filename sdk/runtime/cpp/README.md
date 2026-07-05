# corework/runtime-host-cpp

C++ Runtime Host SDK for Agent Runtime ABI 1.

This SDK is a small header-only RAII wrapper over the stable C ABI in
`sdk/runtime/c/include/agent_runtime.h`. Product hosts should use this wrapper
or the C ABI directly; they should not link Rust implementation crates.

## Minimal Usage

```cpp
#include <corework_runtime/runtime.hpp>

using namespace corework::runtime;

int main() {
    RuntimeCreateOptions options;
    options.data_dir = "./data/runtime";

    Runtime runtime = Runtime::load("path/to/agent_runtime.dll", options);
    runtime.register_resources_path("config/resources.json");
    runtime.register_llm_path("config/llm-providers.json");
    runtime.register_agent_cluster_path("config/agent-cluster.json");
    runtime.start();

    ConversationSpawnOptions spawn;
    spawn.cluster_id = "product-instance";
    spawn.tool_host_context_json = R"({"product":{"surface":"cpp-host"}})";
    auto conversation = runtime.spawn_conversation(spawn);
    runtime.send_message(conversation.conversation_id, "Hello");

    while (auto event = runtime.next_public_event(250)) {
        // Relay event.json to the product frontend.
    }
}
```

## API Shape

The C++ API mirrors the Rust Runtime Host SDK design:

```text
Runtime::load(library_path, create_options)
  -> register_resources_path/json(...)
  -> register_llm_path/json(...)
  -> register_agent_cluster_path/json(...)
  -> start()
  -> spawn_conversation(...)
  -> invoke(...) + next_event(...)
  -> shutdown()
```

Key classes:

- `RuntimeCreateOptions`: create options for `agent-runtime-create-options/v1`.
- `Runtime`: owns the dynamic library and Runtime handle.
- `ConversationSpawnOptions`: product conversation creation input.
- `ConversationInfo`: typed conversation id plus raw JSON result.
- `AdmissionResult`: command admission result for message/pause/history work.
- `RuntimeEvent`: one `agent-runtime-event/v1` JSON envelope.
- `ConversationRegistry`: optional current/background conversation tracker.
- `RuntimeError`: exception carrying ABI return code and Runtime error JSON.
- `RuntimeTimeout`, `RuntimeStateError`, and `UnsupportedCommand`: typed
  specializations for timeout, lifecycle, and capability failures.

Typed helpers cover the ABI 1 command set exposed by the native Runtime:

- runtime registration and model/provider administration;
- conversation spawn, restore, materialize, import, export, close, pause, and
  send message;
- dynamic snapshot, tool permission resolution, summary model, history
  compaction, and studio open commands;
- public event filtering through `next_public_event()` and
  `is_public_runtime_event()`;
- raw `invoke(command_type, payload_json)` for forward-compatible commands.

Hosts can require command support during load:

```cpp
auto runtime = Runtime::load(
    "path/to/agent_runtime.dll",
    options,
    {
        "conversation.spawn",
        "conversation.send_message",
        "conversation.export_snapshot",
    });
```

## Shutdown Drain

`shutdown()` is separate from `destroy()` so hosts can persist final events:

```cpp
runtime.shutdown();
try {
    while (auto event = runtime.next_public_event(250)) {
        if (auto delta = ledger_delta_from_event(*event)) {
            // Persist *delta.
        }
        if (auto state = state_delta_from_event(*event)) {
            // Persist *state.
        }
    }
} catch (const RuntimeStateError&) {
    // Queue closed after shutdown.
}
runtime.destroy();
```

## Contract

The wrapper:

- dynamically loads `agent_runtime`;
- checks `agent_runtime_abi_version_v1() == 1`;
- reads `agent_runtime_capabilities_v1()` and validates requested commands;
- frees every `char*` output with `agent_runtime_free_string_v1()`;
- keeps command invocation and event polling as separate calls;
- drains events during shutdown when the host calls `next_event()`;
- calls `shutdown()` and `destroy()` from the destructor if needed, leaking the
  dynamic library instead of unloading it if an orderly close cannot complete.

The public event stream is one bus per runtime handle. It includes
`conversation:created`, `conversation:closed`, `conversation.ledger_delta`,
`conversation.state_delta`, and `frontend:state_snapshot`. LLM usage/error facts
are represented in ledger metadata. Studio/test internals and runtime
diagnostics are internal and are not part of the public event stream.

`conversation.spawn_from_snapshot` should be used for durable recovery or
copying from an `agent-runtime-conversation-snapshot/v1`. It is intentionally
not the same concept as continuing from a tail snapshot exported during normal
runtime operation.

This SDK embeds Runtime. It does not implement Agent tools; use `sdk/rpctools/cpp` for
C++ RPC tool sidecars.

## Native Release Artifact

This header targets Orbit Runtime native release `v0.4.0` / ABI 1. Download the
platform package from:

```text
https://github.com/kouryu339/orbit-runtime/releases/tag/v0.4.0
```

Use `bin/agent_runtime.dll` on Windows x86_64 or `lib/libagent_runtime.so` on
Linux x86_64, then pass the absolute library path to `Runtime::load`.
