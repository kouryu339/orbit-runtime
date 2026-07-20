# 4 Runtime 与 Conversation 生命周期契约

本文描述当前实现，不是未来计划。C ABI 详见 `01-runtime-ffi-usage-guide.md`。

## 4.1 Runtime

```text
create_v1 -> OPEN/unstarted
invoke registration commands
start_v1 -> OPEN/started
invoke commands + next_event pull
shutdown_v1 -> CLOSING -> CLOSED
destroy_v1 -> handle removed
```

shutdown 一开始就拒绝新 start/invoke，等待在途 ABI 调用，再关闭所有
conversation、Studio 和事件生产者。`next_event_v1` 是关闭期事件的 drain 通道，
宿主可继续拉取直到队列关闭返回 BAD_STATE。TIMEOUT 可重试，destroy 不隐式关闭。

## 4.2 Conversation

`conversation.spawn` 使用注册 cluster/profile 创建共享 ConversationState、Gateway、
Cluster 和 Agent 实例。成功结果给出 conversation/scope/tenant/user/created_at。

`send_message`、`pause`、summary model 和 compact 经过 command admission；结果中的
accepted 只说明命令获准，后续事实由 pull event 流观察。`conversation.close` 关闭命令
门、停止 Agent drivers 并从 manager 移除会话。

## 4.3 快照

- `conversation.export_snapshot` 导出 `agent-runtime-conversation-snapshot/v1`。
- `conversation.spawn_from_snapshot` 从持久化的
  `agent-runtime-conversation-snapshot/v1` 创建并恢复一个 conversation。它不同于
  从用于 UI 刷新或观察的尾部快照继续运行。
- `conversation.import_snapshot` 将快照登记到 Runtime。
- `conversation.materialize` 把已登记状态实例化为可运行 conversation。
- `runtime.export_snapshot` 导出 Runtime 级观察快照。

宿主动态文本通过 `conversation.set_dynamic_snapshot` 按 conversation/agent/field 写入，
不通过独立 C 函数。动态文本不是长期业务真相，恢复后由宿主重新发布。

## 4.4 多 Agent

固定 Agent 在 cluster 中预注册并通过 focus 接力；后台 Agent 由任务工具按 profile
动态创建且不抢 focus。任务状态可由宿主调用 `conversation.agent_tasks` 读取，模型本身
依靠报告事件与 ledger，不提供任务榜单轮询工具。

## 4.5 事件

所有 canonical event 进入 handle 私有队列，宿主通过
`agent_runtime_next_event_v1` 拉取 `agent-runtime-event/v1`。FFI 不调用 callback，
HTTP SSE/MQ/Redis Stream 都是宿主拿到 JSON 后的外部适配。稳定事件出口只以
[`07-runtime-persistence-and-recovery-contract.md`](07-runtime-persistence-and-recovery-contract.md)
为准。
