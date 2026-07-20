# corework-runtime

Rust Runtime Host SDK for Agent Runtime ABI 1.

This crate is the primary Runtime Host SDK surface. It dynamically loads the
native `agent_runtime` library through the stable C ABI in
`sdk/runtime/c/include/agent_runtime.h`; product hosts should not link directly
to `agent_runtime_ffi` implementation crates.

The wrapper provides:

1. Dynamic loading of `agent_runtime`.
2. ABI and capabilities checks.
3. Safe ownership for returned UTF-8 strings via `agent_runtime_free_string_v1`.
4. Typed lifecycle: create, register configs, start, invoke, poll events,
   shutdown, destroy.
5. `RuntimeHostBuilder` / `RuntimeApp` for the normal host bootstrap flow.
6. Typed helpers for common conversation, provider, and Studio commands.
7. Event helpers for ledger/state delta detection.
8. A host-side public Runtime event bus with optional per-conversation filtered
   subscriptions.
9. `ConversationRegistry` for current/background/waiting instance tracking.

Minimum host flow:

```rust
use corework_runtime::{ConversationSpawnOptions, Runtime, RuntimeCreateOptions};
use serde_json::json;

let mut runtime = Runtime::load("path/to/agent_runtime.dll")?;
runtime.create_with_options(&RuntimeCreateOptions {
    data_dir: Some("./data/runtime".into()),
    ..RuntimeCreateOptions::default()
})?;
runtime.register_resources_path("config/resources.json")?;
runtime.register_llm_path("config/llm-providers.json")?;
runtime.register_agent_cluster_path("config/agent-cluster.json")?;
runtime.start()?;

let conversation = runtime.spawn_conversation(ConversationSpawnOptions {
    cluster_id: "product-instance".to_string(),
    tool_host_context: Some(json!({"product": {"surface": "rust-host"}})),
    ..ConversationSpawnOptions::default()
})?;

runtime.send_message(&conversation.conversation_id, "Hello")?;
```

Cluster tool permissions are defaults. Override selected categories for one
conversation with `ToolPermissionPolicyOverride`:

```rust
use corework_runtime::{ToolPermissionMode, ToolPermissionPolicyOverride};

let conversation = runtime.spawn_conversation(ConversationSpawnOptions {
    cluster_id: "product-instance".to_string(),
    permissions: Some(ToolPermissionPolicyOverride {
        destructive: Some(ToolPermissionMode::Ask),
        ..ToolPermissionPolicyOverride::default()
    }),
    ..ConversationSpawnOptions::default()
})?;
```

Omitted categories inherit the cluster. Resolve an `ask` request with
`decide_tool_permission(..., ToolPermissionDecision::Allow/Deny)` after reading
it from `frontend:state_snapshot.payload.pending_permissions`.

After `start`, hosts manage the Draft/Registered lifecycle through
`create_workflow_draft`, `read_workflow`, `register_workflow_draft`,
`update_workflow`, `compile_workflow_draft`, `delete_workflow`, and
`list_workflows`. Use `workflow_script_to_blueprint` and
`workflow_blueprint_to_script` for stateless conversion without changing the
catalog. Use `execute_workflow` only for Registered resources,
`test_workflow_draft` for explicit untrusted tests, and
`execute_workflow_script` for temporary text. `subscribe_workflow(id)` filters
the global Workflow event line. See the shared
[Dynamic Workflow ABI](../README.md#dynamic-workflow-abi).

After startup, `get_tool_definitions` returns the unified registered local/RPC
tool catalog. It is discovery metadata, not the active or approved tool set of
a conversation.

`get_workflow_node_definitions` returns the shared Corework/local/RPC graph-node
catalog, including Pure-node `display_name` templates and complete pins.

`get_agent_cluster_definitions` exposes effective registered and built-in Agent
topology without prompt content. `get_rpc_endpoint_definitions` exposes
sanitized endpoint registration and startup verification without connection
addresses, launch environment, or credentials.

Recommended host flow:

```rust
use corework_runtime::{
    ConversationRegistry, ConversationRegistryAction, ConversationSpawnOptions,
    RuntimeCreateOptions, RuntimeHostBuilder,
};
use serde_json::json;

let app = RuntimeHostBuilder::new("path/to/agent_runtime.dll")
    .create_options(RuntimeCreateOptions {
        data_dir: Some("./data/runtime".into()),
        ..RuntimeCreateOptions::default()
    })
    .resources_path("config/resources.json")
    .llm_path("config/llm-providers.json")
    .agent_cluster_path("config/agent-cluster.json")
    .start()?;

let events = app.subscribe_events();
let conversation = app.spawn_conversation(ConversationSpawnOptions {
    cluster_id: "product-instance".to_string(),
    tool_host_context: Some(json!({"product": {"surface": "rust-host"}})),
    ..ConversationSpawnOptions::default()
})?;

let mut registry = ConversationRegistry::new();
registry.select_current(conversation.conversation_id.clone());

while let Ok(event) = events.recv() {
    if let Some(ConversationRegistryAction::CloseBackground { conversation_id }) =
        registry.observe_runtime_event(&event)
    {
        let _ = app.close_conversation(&conversation_id);
    }
}
```

Hosts should poll Runtime events once and fan them out through the SDK event
bus. Per-conversation SSE is a filtered view over that bus, not a separate
Runtime event source:

```rust
use corework_runtime::{RuntimeEventBus, RuntimeEventPump};

let bus = RuntimeEventBus::new();
let subscription = bus.subscribe_conversation(conversation.conversation_id.clone());
let pump = RuntimeEventPump::new(runtime.event_reader(), bus.clone()).spawn();

// In a host worker, relay complete event envelopes to UI/SSE/storage.
let event = subscription.recv()?;
```

The public bus forwards only stable conversation events:

```text
conversation:created
conversation:closed
conversation.ledger_delta
conversation.state_delta
frontend:state_snapshot
workflow.resource_changed
workflow.execution_completed
```

LLM usage/error facts are read from `conversation.ledger_delta` records with
`metadata.subtype = "llm_usage"` or `"llm_error"`. Diagnostics, workspace/tool
bridge events, and Studio/test internal events are host or Runtime-internal
surfaces rather than public Runtime event bus entries.

During shutdown, keep polling until the Runtime event queue closes if the host
needs final ledger/state/closed events. `RuntimeApp::close` performs
shutdown-drain-destroy in that order.

This SDK embeds Runtime. It does not implement Agent tools; use `sdk/rpctools/rust` for
Rust RPC tool sidecars.

## Native Release Artifact

The SDK is wired to Orbit Runtime native release `v0.4.5` / ABI 1.3. Use
`corework_runtime::release` to discover the correct asset for the current
platform:

```rust
if let Some(artifact) = corework_runtime::release::current_platform_artifact() {
    println!("{}", corework_runtime::release::release_download_url(artifact));
    println!("sha256={}", artifact.sha256);
    println!("library={}", artifact.library);
}
```

Download/extract the asset into your application bundle or cache, then pass the
absolute library path to `Runtime::load` or `RuntimeHostBuilder::new`.
