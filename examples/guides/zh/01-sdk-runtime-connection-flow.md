# 1 SDK Runtime 连接流程

这是产品接入的必走流程。不要从前端聊天框开始，也不要只从 Runtime Host SDK
开始。真正的 Agent 只有在工具、配置、Skills、Runtime 注册和 conversation spawn
都完成后才存在。

```text
1. Tools       准备可调用的内置工具或 sidecar 工具。
2. Configs     准备 resources、LLM providers 和 agent cluster 配置。
3. Skills      编写 role/feature Skills 和工具白名单。
4. Connect     使用 Runtime Host SDK，并注册三份配置。
5. Run         start Runtime，spawn conversation，转发 events，按需持久化。
```

## 1.1 Tools

先决定 Agent 可以调用哪些能力。

外部工具需要：

- 使用 Agent Tool SDK 实现每个 tool sidecar；
- 通过 sidecar 协议发布 descriptor；
- 使用稳定、大小写固定的工具名；
- 描述参数、输出、副作用、幂等性和 capability；
- 返回有用的 `AIOutput.to_ai`，供模型继续推理；
- 通过 gRPC 或 json-lines endpoint 可连接。

此时 Runtime 还不知道这些 endpoint。你只是在准备可调用工具集。

## 1.2 Configs

准备三份配置：

| 文件 | 注册命令 | 定义内容 |
|---|---|---|
| `resources.json` | `runtime.register_resources` | Skill 根目录、Agent profiles、工具/RAG endpoints、data/log/workflow 根目录。 |
| `llm-providers.json` | `runtime.register_llm` | Providers、凭据/base URL、模型 id、上下文窗口、当前模型。 |
| `agent-cluster.json` | `runtime.register_agent_cluster` | 具体 Agent 实例和初始 focus 种子。后续焦点交接和路由由 Runtime 内置机制负责。 |

配置文件描述 Runtime 世界。它们要等宿主连接 Runtime 并调用注册命令后才会加载。

## 1.3 Skills

在 `resources.json` 引用的 `skills.root_dir` 下编写 role 和 feature Skills。

Skills 决定哪些工具对 Agent 可见：

```yaml
tools: ["WordOpenSession", "WordApplySessionPatch"]
```

工具名必须匹配 Runtime 内置 operation 或 tool sidecar descriptor。已经注册但没有
被 active role/feature Skill 引用的工具，不会进入模型上下文，也会在执行时被拒绝。

## 1.4 Connect

宿主管理 native Runtime。浏览器或前端不能直接加载动态库，也不应直接注册资源。

```text
加载 native library
  -> 校验 ABI 和 capabilities
  -> create Runtime handle
```

在 `start` 前注册三份配置：

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

如果跳过这一步，Runtime 无法形成真实 Agent 上下文，无法选择模型，也无法路由工具。

## 1.5 Run

```python
    runtime.start()
    conversation = runtime.invoke(
        "conversation.spawn",
        {"cluster_id": "product-instance"},
    )
```

`start` 会冻结当前 Runtime 生命周期内的注册信息。`spawn` 基于已注册的 cluster id
创建 conversation。

### 1.5.1 产品级 Host API

前端只应访问产品级 conversation 操作：

- 创建或恢复 conversation；
- 发送用户消息；
- 暂停或关闭 conversation；
- 获取恢复 snapshot；
- 订阅 Runtime events。

resources 注册、LLM/provider 管理和 cluster setup 属于宿主管理面。焦点交接和
Agent 路由是 Runtime 内置机制；宿主只控制哪些内置命令可以暴露给调用方。

### 1.5.2 转发事件并渲染

宿主通过 SSE、WebSocket、Tauri events 或其他产品传输转发完整的
`agent-runtime-event/v1` envelope。

前端渲染规则：

- `frontend:state_snapshot` 是 canonical UI state；
- `ledger_delta.record` 追加或更新对话内容；
- `conversation_state` 控制交互状态；
- `call_id` 合并工具占位、开始和结束记录。

## 1.6 最小检查表

- Tool sidecars 已启动并能发布 descriptors。
- `resources.json` 注册 endpoints 和 Agent profiles。
- `llm-providers.json` 有可用的当前模型。
- `agent-cluster.json` 至少创建一个 focus Agent。
- Skills 只引用该 Agent 真正应该看见的工具。
- 宿主在 `start` 前注册三份配置。
- 前端只调用宿主提供的 conversation API。
