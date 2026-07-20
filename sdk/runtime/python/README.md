# corework-runtime

Python host SDK for Agent Runtime ABI 1.

```python
from corework_runtime import RuntimeHostBuilder

create_options = {
    "schema": "agent-runtime-create-options/v1",
    "log_level": "info",
    "language": "zh-CN",
    "restore_policy": "strict",
    "data_dir": "./data/runtime",
}

with (
    RuntimeHostBuilder.from_release()
    .create_options(create_options)
    .resources_path("resources.json")
    .llm_path("llm.json")
    .agent_cluster_path("cluster.json")
    .start()
) as app:
    conversation = app.spawn_conversation(cluster_id="assistant")
    app.send_message(conversation["conversation_id"], "Hello")

    with app.subscribe() as events:
        for event in events:
            # Relay event to the product frontend or persistence layer.
            ...
```

Cluster tool permissions are defaults. A trusted host can override selected
categories for one conversation:

```python
conversation = app.spawn_conversation(
    cluster_id="assistant",
    permissions={"destructive": "ask"},
)
```

Omitted categories inherit the cluster. Read approval requests from
`frontend:state_snapshot.payload.pending_permissions`, then call
`app.resolve_tool_permission(conversation_id, tool_call_id, "allow")` (or pass
`"deny"`).

`RuntimeHostBuilder` is the recommended host entry point. It performs the
standard bootstrap sequence, starts one background event pump, and exposes one
public event bus. Use `Runtime` directly only when a host needs raw ABI control.

The started runtime supports Draft/Registered workflow methods on `Runtime` and
`RuntimeApp`: `create_workflow_draft`, `read_workflow`,
`register_workflow_draft`, `update_workflow`, `compile_workflow_draft`,
`delete_workflow`, `list_workflows`, `execute_workflow`,
`test_workflow_draft`, `execute_workflow_script`,
`workflow_script_to_blueprint`, and `workflow_blueprint_to_script`. Conversion
is stateless and does not emit catalog events. `subscribe_workflow(id)`
filters the global Workflow event line. See the shared
[Dynamic Workflow ABI](../README.md#dynamic-workflow-abi).

After startup, `tool_definitions()` returns the unified registered local/RPC
tool catalog. It does not imply that every tool is active or approved in a
particular conversation.

`workflow_node_definitions()` returns the shared Corework/local/RPC graph-node
catalog, including Pure-node `display_name` templates and complete pins.

`agent_cluster_definitions()` returns effective registered and built-in Agent
topology without prompt content. `rpc_endpoint_definitions()` returns sanitized
endpoint registration and startup verification without addresses, launch
environment, or credentials.

`RuntimeHostBuilder.from_release()` downloads the matching native Runtime
package from `kouryu339/orbit-runtime` release `v0.4.5`, verifies SHA256, and
caches it under the user cache directory. To only fetch the native package:

```powershell
corework-runtime-fetch-native
```

The explicit path form is still supported when a host manages the native
artifact itself:

```python
RuntimeHostBuilder("path/to/agent_runtime.dll")
```

Ledger and internal runtime-state replication are event-driven. Hosts that need
migration or persistence can listen for `conversation.ledger_delta` and
`conversation.state_delta`; hosts that do not need them can ignore the events.
Also track `conversation:created` and `conversation:closed` if the host owns
conversation lifecycle or route persistence.

```python
import json
from corework_runtime import (
    RuntimeHostBuilder,
    ledger_delta_from_event,
    state_delta_from_event,
)

with RuntimeHostBuilder.from_release().create_options(create_options).start() as app:
    for event in app.subscribe():
        delta = ledger_delta_from_event(event)
        if delta is not None:
            # Option A: append to local JSONL.
            with open("ledger-deltas.jsonl", "a", encoding="utf-8") as file:
                file.write(json.dumps(delta, ensure_ascii=False) + "\n")

            # Option B: upsert into a database using
            # (delta["conversation_id"], delta["record_id"]).
            continue

        state = state_delta_from_event(event)
        if state is not None:
            # Use the same event for frontend updates and persistence.
            # dynamic_snapshot.set is host-owned and should be refreshed after
            # restore; agent_task.upsert and focus.set are runtime internal
            # state that can be replayed.
            with open("state-deltas.jsonl", "a", encoding="utf-8") as file:
                file.write(json.dumps(state, ensure_ascii=False) + "\n")
```

During shutdown, keep polling events until the runtime event queue closes if you
need the last ledger/state deltas and conversation-closed notifications. Use the
two-step lifecycle when you need this drain window:

`RuntimeApp.close()` first requests runtime shutdown, gives the event pump a
short drain window, then destroys the native handle. Hosts that need custom
drain policies can still use `Runtime.shutdown()`, `Runtime.next_event()`, and
`Runtime.destroy()` directly.

`conversation:closed` is scoped to one conversation. It is not a signal that the
native runtime handle is closed. If several conversations are active, keep
tracking each one until all expected close events have been persisted and the
event queue has been drained.

To recover after a host or pod failure, rebuild a durable
`agent-runtime-conversation-snapshot/v1` from persisted ledger/state deltas and
call `app.spawn_conversation_from_snapshot(...)` or
`app.import_conversation_snapshot(...)`. This is intentionally separate from
tail snapshots exported during normal runtime operation.

The public event bus only publishes stable host-facing events:
`conversation:created`, `conversation:closed`, `conversation.ledger_delta`,
`conversation.state_delta`, `frontend:state_snapshot`,
`workflow.resource_changed`, and `workflow.execution_completed`. LLM usage/error
facts should be read from ledger metadata. Studio/test internals and runtime
diagnostics are internal; diagnostics are delivered through the optional
builder callback, not the public event stream.
