# 2 Runtime Create Options and Registrations

Building an Agent starts with direct Runtime create options plus three Runtime
registrations. UI code, host IPC, and tool rendering sit on top of these inputs;
they do not define the Agent by themselves.

```text
create options -> resources -> LLM providers -> agent cluster -> start -> spawn conversation
```

Create options are passed directly to the FFI create call. They are not a config
file and they do not define Agents, Skills, tools, providers, or clusters.

```json
{
  "schema": "agent-runtime-create-options/v1",
  "log_level": "info",
  "language": "zh-CN",
  "restore_policy": "strict",
  "data_dir": "./data/runtime"
}
```

## 2.1 Resource Registration

`agent-runtime-resource-registration/v1` answers: what can this product expose
to Runtime?

It registers:

- `skills.root_dir`: role and feature Skills.
- reusable Agent profiles under `agents.profiles`.
- RPC/RAG/tool endpoints under `rpc_endpoints`.
- workflow, data, and log roots when the product uses them.

Example:

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "id": "product-resources",
  "skills": { "root_dir": "../skills", "builtin_system": true },
  "agents": {
    "profiles": [{
      "id": "product.main",
      "name": "Product Agent",
      "role": "product_agent",
      "features": ["word", "excel"]
    }]
  },
  "rpc_endpoints": [{
    "id": "word-tools",
    "protocol": "grpc",
    "endpoint": "127.0.0.1:50103",
    "timeout_ms": 60000
  }]
}
```

Resources register availability. They do not start a conversation and do not
make every tool visible to every Agent. Tool visibility still comes from active
role/feature Skill `tools` allowlists.

## 2.2 LLM Provider Registration

`agent-runtime-llm-registration/v1` answers: which model can Runtime call?

It registers providers, credentials or base URLs, model IDs, context windows,
and `current_model_uid`.

Hosts register it before `start` with `runtime.register_llm`. Keep example files
free of real keys.

## 2.3 Agent Cluster Registration

`agent-runtime-agent-cluster-registration/v1` answers: which concrete Agents
exist in this conversation cluster, and what initial focus seed Runtime should
use when spawning a conversation.

It creates concrete instances from resource-registered profiles:

```json
{
  "schema": "agent-runtime-agent-cluster-registration/v1",
  "id": "product-instance",
  "focus_agent_id": "product.main",
  "agents": [{
    "id": "product-main-1",
    "profile": "product.main"
  }]
}
```

Profiles are reusable types; cluster `agents[]` are concrete instances.
`focus_agent_id` is only the initial focus seed. Later focus handoff and routing
are Runtime built-ins, driven by collaboration commands and conversation state.
Use a concrete instance id when a profile has multiple instances.

## 2.4 Host Startup Order

```python
runtime.invoke("runtime.register_resources", {"input": "config/resources.json"})
runtime.invoke("runtime.register_llm", {"input": "config/llm-providers.json"})
runtime.invoke("runtime.register_agent_cluster", {"input": "config/agent-cluster.json"})
runtime.start()
conversation = runtime.invoke("conversation.spawn", {"cluster_id": "product-instance"})
```

After `start`, treat these registrations as frozen for that Runtime lifecycle.
The frontend should call product-level conversation APIs only; resource, model,
and cluster registration are host-side administration.
