# 1 SDK Runtime Connection Flow

This is the required product onboarding flow. Do not start from a frontend chat
window, and do not start from the Runtime Host SDK alone. A real Agent exists
only after tools, configs, Skills, Runtime registration, and conversation spawn
are all in place.

```text
1. Tools       Prepare callable built-in or sidecar tools.
2. Configs     Prepare resources, LLM providers, and agent cluster configs.
3. Skills      Write role/feature Skills and tool allowlists.
4. Connect     Use Runtime Host SDK and register the three configs.
5. Run         Start Runtime, spawn conversation, relay events, persist if needed.
```

## 1.1 Tools

Decide which capabilities the Agent may call.

For external tools:

- implement each tool sidecar with an Agent Tool SDK;
- publish descriptors through the sidecar protocol;
- use stable, case-sensitive tool names;
- describe parameters, outputs, side effects, idempotency, and capabilities;
- return useful `AIOutput.to_ai` text for model reasoning;
- make the endpoint reachable through gRPC or json-lines.

At this point the Runtime does not know about the endpoint yet. You are only
preparing the callable tool set.

## 1.2 Configs

Prepare three config documents:

| File | Registered by | Defines |
|---|---|---|
| `resources.json` | `runtime.register_resources` | Skill root, Agent profiles, tool/RAG endpoints, data/log/workflow roots. |
| `llm-providers.json` | `runtime.register_llm` | Providers, credentials/base URLs, model ids, context windows, current model. |
| `agent-cluster.json` | `runtime.register_agent_cluster` | Concrete Agent instances and the initial focus seed. Runtime owns later focus handoff and routing. |

Config files describe the runtime world. They are not loaded until the host
connects and invokes the registration commands.

## 1.3 Skills

Write role and feature Skills under the `skills.root_dir` referenced by
`resources.json`.

Skills decide which tools are visible to the Agent:

```yaml
tools: ["WordOpenSession", "WordApplySessionPatch"]
```

The tool names must match built-in operations or tool sidecar descriptors. A
registered tool that is not referenced by an active role/feature Skill is absent
from model context and rejected at execution.

## 1.4 Connect

The host owns the native Runtime. A browser or frontend must not load the
dynamic library and must not register resources directly.

```text
load native library
  -> verify ABI and capabilities
  -> create Runtime handle
```

Register all three configs before `start`:

```python
from corework_runtime import Runtime

create_options = {
    "schema": "agent-runtime-create-options/v1",
    "log_level": "info",
    "language": "zh-CN",
    "restore_policy": "strict",
    "data_dir": "./data/runtime",
}

with Runtime("agent_runtime.dll", create_options) as runtime:
    runtime.invoke("runtime.register_resources", {"input": "config/resources.json"})
    runtime.invoke("runtime.register_llm", {"input": "config/llm-providers.json"})
    runtime.invoke(
        "runtime.register_agent_cluster",
        {"input": "config/agent-cluster.json"},
    )
```

If this step is skipped, the Runtime cannot form a real Agent context, choose a
model, or route tools.

## 1.5 Run

```python
    runtime.start()
    conversation = runtime.invoke(
        "conversation.spawn",
        {"cluster_id": "product-instance"},
    )
```

`start` freezes registration for the current Runtime lifecycle. `spawn` creates
a conversation against a registered cluster id.

### 1.5.1 Product-Level Host APIs

Expose only product-level conversation operations to the frontend:

- create or restore conversation;
- send user message;
- pause or close conversation;
- get recovery snapshot;
- subscribe to Runtime events.

Keep resource registration, LLM/provider administration, and cluster setup
behind host authorization. Focus handoff and agent routing are Runtime
built-ins; the host only controls which built-in commands are exposed.

### 1.5.2 Relay Events And Render

Relay complete `agent-runtime-event/v1` envelopes through SSE, WebSocket, Tauri
events, or another product transport.

Frontend rendering rules:

- `frontend:state_snapshot` is canonical UI state;
- `ledger_delta.record` appends or updates conversation content;
- `conversation_state` controls interaction state;
- `call_id` correlates tool placeholder, started, and finished records.

## 1.6 Minimum Checklist

- Tool sidecars run and publish descriptors.
- `resources.json` registers endpoints and Agent profiles.
- `llm-providers.json` contains a usable current model.
- `agent-cluster.json` creates at least one focus Agent.
- Skills reference only tools the Agent should actually see.
- Host registers all three configs before `start`.
- Frontend talks only to host conversation APIs.
