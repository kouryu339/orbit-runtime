# 5 连接宿主、Runtime 与前端

宿主应用负责加载 native Runtime library，使用 Runtime Host SDK 管理生命周期，
并把 Runtime 事件转发给前端。浏览器或 UI 不应直接加载动态库。

从工具描述到 conversation spawn 的完整接入顺序，先看
[SDK Runtime 连接流程](01-sdk-runtime-connection-flow.md)。本文聚焦这些输入准备好
之后的宿主、Runtime 和前端边界。

```text
Frontend <-> Host API/SSE/WebSocket <-> Runtime Host SDK <-> Native Runtime
                                                    |
                                                    +-> Tool sidecars
```

## 5.1 启动顺序

```text
加载动态库并校验 ABI
  -> 创建 Runtime
  -> 注册 resources
  -> 注册 LLM
  -> 注册 agent cluster
  -> start
  -> spawn conversation
  -> 轮询并转发 events
```

`start` 前的三步注册就是 Agent 搭建的核心：

| 配置 | 命令 | 为什么重要 |
|---|---|---|
| Resources | `runtime.register_resources` | 注册 Skill 根目录、可复用 Agent profile、工具/RAG endpoint、workflow 和 data/log 根目录。 |
| LLM providers | `runtime.register_llm` | 注册 provider、模型 id、凭据/base URL、上下文窗口和当前模型。 |
| Agent cluster | `runtime.register_agent_cluster` | 从 profile 创建具体 Agent 实例，并声明初始 focus 种子。后续焦点交接和路由由 Runtime 内置机制负责。 |

不要把这三份配置当成可选示例。没有它们，前端会话窗口只是外壳；Runtime 无法形成
真实 Agent 上下文，不能调用模型，也不能按 Skill 白名单路由工具。

Python Host SDK 示例：

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
    runtime.invoke("runtime.register_llm", {"input": "config/llm.json"})
    runtime.invoke(
        "runtime.register_agent_cluster",
        {"input": "config/agent-cluster.json"},
    )
    runtime.start()

    conversation = runtime.invoke(
        "conversation.spawn",
        {"cluster_id": "commerce-service"},
    )
```

Go SDK 可在 `runtimehost.Start(options)` 中传入 resource 和 cluster registration，
并通过 `EventSink` 接收事件。具体 API 以
[`sdk/runtime`](../../../sdk/runtime/README.md) 为准。

## 5.2 宿主 API

宿主至少应向自己的前端提供：

- 创建或恢复 conversation；
- 发送用户消息；
- 暂停当前运行；
- 获取首次加载或断线恢复所需的 snapshot；
- Runtime event 流。

前端命令必须经过宿主白名单和身份校验。模型配置和资源注册不能直接暴露给浏览器。
Agent 路由和焦点交接是 Runtime 内置机制；宿主只授权相关内置命令，不自己管理路由。

## 5.3 转发事件

Runtime Host SDK 通过事件轮询或 `EventSink` 得到完整的
`agent-runtime-event/v1` JSON。宿主可以用 SSE、WebSocket、Tauri event 或消息
队列转发，但应保留原始 envelope，不要重新发明一套消息语义。

前端消费规则：

- `frontend:state_snapshot` 是会话 UI 的 canonical 状态来源；
- 从 `payload.ledger_delta.record` 追加或更新聊天内容；
- 用 `payload.conversation_state` 判断发送、暂停等交互状态；
- 用 `call_id` 合并工具占位、开始和结束记录；
- 从 `conversation.ledger_delta` 中读取 `metadata.subtype = "llm_usage"` 或
  `"llm_error"` 的事实记录；这两类事实不再提供独立的公共事件流；
- snapshot 用于首次 hydrate 和断线对账，不用于替代事件流轮询。

前端消息结构见
[`06-runtime-frontend-message-contract.md`](../../../agent_runtime_ffi/docs/zh/06-runtime-frontend-message-contract.md)，
事件 envelope 见
[`05-runtime-event-format.md`](../../../agent_runtime_ffi/docs/zh/05-runtime-event-format.md)。
模型上下文顺序、尾部动态快照和摘要位置见
[`上下文结构和快照机制`](09-context-and-snapshots.md)。
需要支持持久化、pod 丢失恢复和 shutdown drain 的宿主，见
[`持久化与恢复机制`](10-persistence-and-recovery.md)。

## 5.4 Runtime 级 LLM Header

宿主通过 `runtime.set_auth_context` 为当前 Runtime 的 LLM 请求注入自定义 header。
配置会作用于已有和后续 conversation，并继续传递到自动续写和后台 Agent driver。
凭据属于宿主传输配置，不会进入模型上下文、ledger、Skill 或动态快照。

可运行参考：

- [`examples/python_ctypes`](../../python_ctypes)：Python Host SDK 与简单浏览器前端。
- [`examples/go_order_admin`](../../go_order_admin)：Go Host SDK、HTTP/SSE 和业务前端。
