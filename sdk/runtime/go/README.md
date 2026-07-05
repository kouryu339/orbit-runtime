# corework/runtime-host-go

Go host SDK for Agent Runtime ABI 1.

The package uses cgo and links against `agent_runtime`. Install the platform
runtime library in the linker search path, or provide `CGO_LDFLAGS`.

The SDK is aligned with Orbit Runtime native release `v0.4.0` / ABI 1. Use
`CurrentRuntimeArtifact()` and `RuntimeReleaseURL()` when your installer needs
to fetch the matching package:

```go
artifact, err := runtimehost.CurrentRuntimeArtifact()
if err != nil {
    log.Fatal(err)
}
fmt.Println(runtimehost.RuntimeReleaseURL(artifact))
fmt.Println(artifact.SHA256)
fmt.Println(artifact.Library)
```

Download and extract the package before building/running the cgo host. The
native library must be visible to the linker and runtime loader.

```go
runtime, err := runtimehost.Start(runtimehost.Options{
    CreateOptions: json.RawMessage(`{
        "schema": "agent-runtime-create-options/v1",
        "log_level": "info",
        "language": "zh-CN",
        "restore_policy": "strict",
        "data_dir": "./data/runtime"
    }`),
    ResourceRegistration: resourcesJSON,
    LlmRegistration:      llmProvidersJSON,
    ClusterRegistration:  agentClusterJSON,
    EventSink: func(event json.RawMessage) {
        // EventSink receives the public Runtime event stream only.
        delta, ok, err := runtimehost.ParseLedgerDelta(event)
        if err != nil {
            return
        }
        if ok {
            // Persist locally, upsert into a database with
            // (delta.ConversationID, delta.RecordID), or ignore if ledger
            // replication is not needed by this host.
            _ = delta
            return
        }

        state, ok, err := runtimehost.ParseStateDelta(event)
        if err != nil || !ok {
            return
        }
        // Use the same state delta for frontend updates and persistence.
        // dynamic_snapshot.set is host-owned and should be refreshed after
        // restore; agent_task.upsert and focus.set are runtime internal state.
        _ = state
    },
})
if err != nil {
    log.Fatal(err)
}
defer runtime.Close()
```

Ledger replication is event-driven through `conversation.ledger_delta`; internal
runtime state is event-driven through `conversation.state_delta`. The SDK does
not choose storage for the host: write JSONL, upsert into a database, update a
frontend store, or drop the event.

The event pump is one public bus for the runtime handle, not one stream per
conversation. `EventSink` and `Events()` include only stable host-facing events:
`conversation:created`, `conversation:closed`, `conversation.ledger_delta`,
`conversation.state_delta`, and `frontend:state_snapshot`. LLM usage/error facts
are represented in ledger metadata. Studio/test internals and runtime
diagnostics are not public runtime events.

`Runtime.Close` keeps the event pump alive while shutdown drains the final
ledger/state deltas and conversation-closed notifications.

Track `conversation:created` and `conversation:closed` when the host owns
conversation routing or durable lifecycle state. A `conversation:closed` event
is scoped to one conversation; it is not a signal that the native runtime handle
has closed. Hosts with multiple active conversations should keep draining events
until all tracked conversations are closed and runtime shutdown has completed.

Recovery after host or pod failure should rebuild a durable
`agent-runtime-conversation-snapshot/v1` from persisted ledger/state deltas and
then call `SpawnConversationFromSnapshot` or `ImportConversationSnapshot`. This
is distinct from a tail snapshot exported during normal runtime operation. The
native runtime rebuilds the state-machine entry and unresolved tool-call
handling; the Go SDK only transports the command and events.

Use `NewConversationRegistry()` when a host wants a shared policy for
current/background/data-only conversations:

```go
registry := runtimehost.NewConversationRegistry()
var runtime *runtimehost.Runtime
var err error
runtime, err = runtimehost.Start(runtimehost.Options{
    EventSink: func(event json.RawMessage) {
        for _, action := range registry.ObserveEvent(event) {
            if action.Kind == "close_background" {
                _ = runtime.CloseConversation(context.Background(), action.ConversationID)
            }
        }
    },
})
```

The public C header is shared with the other Runtime Host SDKs at
`sdk/runtime/c/include/agent_runtime.h`.
