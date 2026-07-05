# 10 持久化与恢复机制

本指南面向应用宿主。核心恢复机制在 `ai-assistant` 中实现；FFI 和 SDK 只把稳定事件
和恢复命令暴露给宿主。宿主负责决定是否保存、保存到哪里，以及如何把保存的数据
重新交给 Runtime。

## 10.1 需要监听的事件

如果产品需要支持刷新、迁移、pod 丢失或进程崩溃后的恢复，宿主应持续监听：

- `conversation:created`：保存 conversation manifest、cluster、路由和业务绑定。
- `conversation.ledger_delta`：按 `conversation_id + record_id` 幂等保存 ledger。
- `conversation.state_delta`：保存 focus、agent task、skills、plan 和 dynamic snapshot 镜像。
- `conversation:closed`：标记 conversation 生命周期结束。

`frontend:state_snapshot` 用于 UI hydrate 和断线对账，不应作为唯一持久化来源。

## 10.2 恢复流程

```text
读取宿主持久化记录
  -> 重建 agent-runtime-conversation-snapshot/v1
  -> 调用 conversation.spawn_from_snapshot 或 conversation.import_snapshot
  -> Runtime 根据 ledger 尾部恢复 thinking/executing/suspended
  -> 宿主重新绑定外部资源
  -> 宿主重新发布当前 dynamic snapshots
```

恢复时 Runtime 不会简单回退到最近 user 重新跑。它会根据 ledger 尾部重建状态机入口：
最后 user 进入 `thinking`，干净 assistant 进入 `suspended`，未闭合工具进入 `executing`，
闭合工具结果后回到 `thinking`。

## 10.3 未闭合工具

如果恢复时发现工具调用未闭合：

- 只读工具可以重跑，相当于重新查询。
- 非只读或未知安全性的工具不会被真实重放；Runtime 会用原 `tool_call_id` 写入恢复
  tool result，让 AI 查验外部状态、向用户说明不确定性，或让子 agent 向主 agent 汇报。

多 Agent 情况下，未闭合工具按 `agent_id` 分组恢复，每个 Agent 保留自己的
`tool_call_id`。

## 10.4 Shutdown 期间的持久化

手动关闭 pod 或调用 runtime shutdown 时，Runtime 会逐个关闭 conversation。每个
conversation 可能先产生最后的 ledger/state delta，再产生 `conversation:closed`。

宿主应继续 drain Runtime event queue，直到：

1. 所有宿主跟踪的 conversations 都已收到 `conversation:closed`。
2. 最后的 ledger/state delta 已持久化。
3. runtime shutdown 已完成，事件队列已 drain。

不要把某一个 `conversation:closed` 当成整个 Runtime handle 已关闭。它只说明该
conversation 生命周期结束。

## 10.5 宿主外部状态

Runtime 不保活外部业务资源。恢复后宿主必须重新绑定数据库对象、文件、浏览器页、
外部 session 等，并重新发布当前的 `conversation.set_dynamic_snapshot`。旧 dynamic
snapshot 只能作为最后观测值，不应被当成长期有效事实。

底层契约见：

- [`ai-assistant/docs/zh/05_agent_and_persistence.md`](../../../ai-assistant/docs/zh/05_agent_and_persistence.md)
- [`agent_runtime_ffi/docs/zh/07-runtime-persistence-and-recovery-contract.md`](../../../agent_runtime_ffi/docs/zh/07-runtime-persistence-and-recovery-contract.md)
- [`sdk/runtime`](../../../sdk/runtime/README.md)
