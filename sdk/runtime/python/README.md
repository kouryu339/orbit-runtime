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

`RuntimeHostBuilder` is the recommended host entry point. It performs the
standard bootstrap sequence, starts one background event pump, and exposes one
public event bus. Use `Runtime` directly only when a host needs raw ABI control.

`RuntimeHostBuilder.from_release()` downloads the matching native Runtime
package from `kouryu339/orbit-runtime` release `v0.4.0`, verifies SHA256, and
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
`conversation.state_delta`, and `frontend:state_snapshot`. LLM usage/error
facts should be read from ledger metadata. Studio/test internals and runtime
diagnostics are internal; diagnostics are delivered through the optional
builder callback, not the public event stream.
