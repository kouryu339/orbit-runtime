# 2 Runtime 创建参数与注册配置

搭建 Agent 首先是直接传入 Runtime create options，然后完成三类 Runtime 注册。
前端聊天框、宿主 IPC 和工具渲染都建立在这些输入之上；它们本身不能定义一个真正的 Agent。

```text
create options -> resources -> LLM providers -> agent cluster -> start -> spawn conversation
```

create options 是传给 FFI create 调用的直接参数，不是配置文件；它只承载进程级
启动参数，不定义 Agent、Skill、工具、provider 或 cluster。

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

`agent-runtime-resource-registration/v1` 回答：这个产品向 Runtime 暴露了哪些资源？

它注册：

- `skills.root_dir`：role 和 feature Skills。
- `agents.profiles`：可复用 Agent profile。
- `rpc_endpoints`：RPC/RAG/tool endpoint。
- workflow、data、log 根目录等产品资源。

示例：

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

resources 只声明可用资源。它不会启动会话，也不会让所有工具对所有 Agent 可见。
工具可见性仍由当前 active role/feature Skill 的 `tools` 白名单决定。

## 2.2 LLM Provider Registration

`agent-runtime-llm-registration/v1` 回答：Runtime 可以调用哪个模型？

它注册 provider、凭据或 base URL、模型 ID、上下文窗口和 `current_model_uid`。

宿主应在 `start` 前通过 `runtime.register_llm` 注册它。示例文件不要写入真实密钥。

## 2.3 Agent Cluster Registration

`agent-runtime-agent-cluster-registration/v1` 回答：本次会话集群里有哪些具体 Agent，
以及 spawn conversation 时 Runtime 应使用哪个初始 focus 种子。

它从 resources 中已经注册的 profile 创建具体实例：

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

profile 是可复用类型；cluster 的 `agents[]` 是具体实例。`focus_agent_id` 只是
初始 focus 种子。后续焦点交接和路由是 Runtime 内置机制，由协作命令和
conversation state 驱动。当同一个 profile 有多个实例时，必须使用具体实例 id，
避免 focus 歧义。

## 2.4 Host 启动顺序

```python
runtime.invoke("runtime.register_resources", {"input": "config/resources.json"})
runtime.invoke("runtime.register_llm", {"input": "config/llm-providers.json"})
runtime.invoke("runtime.register_agent_cluster", {"input": "config/agent-cluster.json"})
runtime.start()
conversation = runtime.invoke("conversation.spawn", {"cluster_id": "product-instance"})
```

`start` 后应把这些注册视为当前 Runtime 生命周期内已经冻结。前端只调用产品级
conversation API；resources、模型和 cluster 注册属于宿主管理面。
