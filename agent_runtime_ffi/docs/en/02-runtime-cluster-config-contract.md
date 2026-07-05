# 2 Runtime Create Options Contract

This document describes the current `agent-runtime-create-options/v1`
boundary. Runtime is an agent engine; it does not own business knowledge bases,
Redis, MQ, HTTP SSE, cross-pod owner leases, or the full lifecycle of a vector
database. Those belong to the host application or an external control plane.

ABI 1 uses the pull event queue. Runtime-internal Redis/cache/coordination/event-sink configuration is unsupported; ignored unknown fields do not preserve old behavior.

For new hosts, prefer the explicit pre-start registration flow:

```text
create options -> runtime.register_resources -> [runtime.register_llm empty/optional] -> runtime.register_agent_cluster -> start
```

This separates product resources and Agent profiles, model/provider selection,
and concrete conversation-cluster instances. Resources, LLM, and Cluster
registrations replace the old assistant-style entry points.

One-line rule: do not put Agents, tools, models, or knowledge bases into
Runtime create options. They answer how this runtime process starts; the three
pre-start registrations answer what product capabilities exist.

For `SKILL.md` role/feature authoring, tool visibility, runtime built-ins, and user-defined RPC tool references, see [`10-runtime-skill-authoring-guide.md`](./10-runtime-skill-authoring-guide.md). For RPC tool-set definitions and the sidecar return-value contract, see [`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md). For host-side FFI call order, see [`01-runtime-ffi-usage-guide.md`](./01-runtime-ffi-usage-guide.md).

## 2.1 Top-Level Shape

```json
{
  "schema": "agent-runtime-create-options/v1",
  "log_level": "info",
  "language": "en",
  "restore_policy": "strict",
  "data_dir": "./data/runtime"
}
```

| Field | Required | Description |
|---|---:|---|
| `schema` | no | Fixed to `agent-runtime-create-options/v1`; empty input uses defaults. |
| `log_level` | no | Default diagnostics level. |
| `language` | no | Prompt and partial UI language. |
| `restore_policy` | no | Recovery policy slot: `strict`, `compatible`, or `unsafe_force`. |
| `data_dir` | no | Local helper directory for runtime logs and local helper state. |

Production capability config should live only in `resources.json`,
`llm-providers.json`, and `agent-cluster.json`. Internal defaults used to boot
the engine are implementation details, not public configuration contract.

LLM/provider configuration may be empty or delayed. New hosts may omit
`llm-providers.json` at first launch, or register an empty provider document
with no current model. Resources and cluster config still define which Agents
and tools exist; they must not embed provider secrets or fake model placeholders.
Until the host later calls `runtime.configure_providers` or
`runtime.reload_llm` and selects a valid current model, conversations can be
created/restored but LLM turns must fail recoverably with a provider/model
configuration error. Product UI should guide the user to configure a provider
and model instead of shipping example credentials.

Old top-level `retrieval` and `runtime.retrieval` are rejected. Retrieval is an
Agent capability: configure it on a resource Agent profile or a cluster Agent
instance. Resources register one or more knowledge endpoints; the Agent selects
one endpoint by `endpoint_id`.

Agent system-prompt output constraints also belong to the Agent profile or a
concrete cluster Agent instance, not to provider/model config or conversation
archive state. Use `systemPromptConstraints` for these prompt-level rules:

```json
{
  "id": "service.researcher",
  "name": "Researcher",
  "role": "researcher",
  "systemPromptConstraints": {
    "frontendWidgetsEnabled": false
  }
}
```

The legacy `frontendWidgetsEnabled` / `frontend_widgets_enabled` boolean is
still accepted for compatibility, but new configs should use
`systemPromptConstraints`.

## 2.2 Ownership of Removed Runtime Config Fields

| Old field | Current owner |
|---|---|
| `skills_dir` | `runtime.register_resources` `skills.root_dir`. |
| `max_thinking_rounds` | `runtime.register_agent_cluster` `max_thinking_rounds`. |
| `cluster_id` | Agent Cluster registration `id`. |
| `runtime_profile_id` / `runtime_instance_id` | Host/control-plane identity, not Runtime public config. |
| `persistence` | Host behavior. Runtime does not own product conversation persistence. |
| `llm_config_dir` / `llm_config_path` | `runtime.register_llm`. |

No longer supported as runtime-internal configuration:

- `cache_backend`
- `coordination`
- `events`
- `events.sink`
- reusing a Redis URL as internal cache / ledger / owner lease / event sink

The current deserializer may ignore unknown fields, but that is only parse-layer behavior. New configs and examples should remove these fields. Redis Stream, Kafka, RocketMQ, HTTP SSE, state mirrors, and cross-pod owner leases belong to the host or control plane.

## 2.3 Cluster Tool Permissions

An `agent-runtime-agent-cluster-registration/v1` document may define the execution policy at its cluster root:

```json
{
  "permissions": {
    "read_only": "full",
    "controlled_change": "ask",
    "destructive": "deny"
  }
}
```

Runtime derives the effect from existing tool metadata: read-only means `readonly=true, destructive=false`; controlled change means both flags are false; destructive means `readonly=false, destructive=true`. Both flags being true is invalid and execution is rejected.

Each effect accepts `full`, `ask`, or `deny`. All three default to `full` when omitted. `ask` emits a permission request before execution and suspends that tool call. `deny` returns a policy-denied, not-executed tool result without requesting approval. Arguments of tools marked `secret` are redacted from permission events.

## 2.4 Agent Retrieval

Register knowledge endpoints in `agent-runtime-resources-registration/v1`:

```json
{
  "rpc_endpoints": [
    {
      "id": "policy-knowledge",
      "protocol": "json-lines",
      "endpoint": "127.0.0.1:50201"
    },
    {
      "id": "product-knowledge",
      "protocol": "json-lines",
      "endpoint": "127.0.0.1:50202"
    }
  ]
}
```

Configure the default policy on an Agent profile:

```json
{
  "id": "service.researcher",
  "name": "Researcher",
  "role": "researcher",
  "retrieval": {
    "enabled": true,
    "endpoint_id": "policy-knowledge",
    "mode": "before_thinking",
    "trigger": "first_thinking_per_user_turn",
    "tool_name": "RagRetrieve",
    "profiles": ["order_admin_policy"],
    "top_k": 5,
    "score_threshold": 0.3,
    "fail_policy": "soft",
    "inject_as": "dynamic_context"
  }
}
```

`cluster.agents[].retrieval` may override the profile for one concrete Agent
instance. Instance configuration wins; there is no cluster-root retrieval
switch. An enabled configuration requires an existing `json-lines` endpoint.

| Field | Description |
|---|---|
| `enabled` | Enables retrieval for this Agent. |
| `endpoint_id` | Required endpoint id registered by resources. |
| `mode` | Currently only `before_thinking`: retrieve before each think. |
| `trigger` | Currently only `first_thinking_per_user_turn`: automatic retrieval runs once for the same user input. |
| `tool_name` | Retrieval system/tool name, default `RagRetrieve`. It is connected by runtime internal glue and does not use `rpc_tools`. |
| `profiles` | Knowledge profiles/namespaces. Runtime passes them through and does not care whether the backend is Qdrant, pgvector, Milvus, or a local index. |
| `top_k` | Maximum number of chunks to return. |
| `score_threshold` | Minimum score. Backend adapters define exact score semantics, but larger should mean more relevant. |
| `fail_policy` | `soft` skips failed retrieval and continues the turn; `hard` aborts the turn on retrieval failure. |
| `inject_as` | Currently only `dynamic_context`: retrieved content is written into agent cache and passed to the LLM as dynamic context. |

### 2.4.1 Retrieval System Contract

When runtime calls the internal RAG dynamic system named by `tool_name`, input is:

```json
{
  "query": "latest user question",
  "profiles": ["order_admin_policy"],
  "top_k": 5,
  "score_threshold": 0.3
}
```

Output must follow:

```json
{
  "error_code": 0,
  "to_ai": "retrieved chunks, sources, and notes ready for LLM injection"
}
```

An empty `to_ai` means no retrieval result. `error_code != 0` is handled according to `fail_policy`.

## 2.5 RAG Boundary

Runtime is responsible for:

- Deciding whether to retrieve before thinking from the current Agent's retrieval config.
- Routing each call to the current Agent's `endpoint_id`.
- Building the retrieval request.
- Calling the internal RAG glue system.
- Writing `to_ai` into agent cache and passing it as `dynamic_context` to the LLM.
- Allowing explicit second-pass calls to the same RAG system.

Runtime is not responsible for:

- Vector database selection.
- Document cleaning, chunking, embedding, or indexing.
- Collection creation/deletion.
- Index rebuilding.
- Business data lifecycle management.

The vector backend only needs to satisfy this unified contract:

```text
input: query + profiles + top_k + score_threshold + filters
output: chunks/documents + score + metadata + source, formatted into to_ai
```

Therefore resources describe available knowledge endpoints, while each Agent
describes whether retrieval is enabled, when it runs, and which profiles are
passed through. The runtime does not bind the host to a specific vector database.
