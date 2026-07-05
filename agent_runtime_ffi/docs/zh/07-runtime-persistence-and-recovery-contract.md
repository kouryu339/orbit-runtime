# 7 Runtime FFI 持久化与恢复宿主契约

本文只描述 FFI 层对宿主暴露的稳定事件和调用约定。状态机恢复、ledger 尾部分析、
未闭合工具处理等核心机制属于 `ai-assistant`，详见
[`ai-assistant/docs/zh/05_agent_and_persistence.md`](../../../ai-assistant/docs/zh/05_agent_and_persistence.md)。

## 7.1 事件出口

FFI 不直接暴露 conversation 内部事件。`ai-assistant` 的 AgentGateway 会把内部
ledger、focus、task、plan、skills、动态快照等变化归一化为稳定 export 事件，FFI
再包装成 `agent-runtime-event/v1` 交给宿主。

宿主需要依赖的稳定事件：

```text
conversation:created
conversation:closed
conversation.ledger_delta
conversation.state_delta
frontend:state_snapshot
```

`frontend:state_snapshot` 是 UI 可渲染状态通道；`conversation.ledger_delta` 和
`conversation.state_delta` 是恢复持久化通道；工具权限状态不再是独立事件出口：
待确认列表来自 `frontend:state_snapshot.payload.pending_permissions`，用户可见的工具生命周期事实来自
`metadata.subtype = "tool_call_permission_requested"` 的 `conversation.ledger_delta` 记录。LLM 用量/错误事实不是独立的稳定宿主事件；
所有焦点和后台 Agent 的权威用量/错误事实都必须来自 `conversation.ledger_delta` 中
`metadata.subtype = "llm_usage"` 或 `"llm_error"` 的记录。不要依赖内部事件名、telemetry 镜像或内部拆分方式。

## 7.2 宿主需要保存什么

需要支持 pod 丢失、进程崩溃或迁移恢复的宿主，应以 conversation 为单位持续保存增量：

- `conversation:created`：记录 conversation manifest、cluster 信息、宿主路由和业务绑定。
- `conversation.ledger_delta`：按 `conversation_id + record_id` 幂等追加 `LedgerRecord`；
  包含 user/assistant/tool 记录，也包含后台 Agent 的 LLM usage/error 等 gateway fact。
- `conversation.state_delta`：按 delta 语义更新 focus、agent task、agent skills、agent plan、
  dynamic snapshot 等镜像状态。
- `conversation:closed`：标记该 conversation 生命周期已结束，并释放宿主侧业务绑定。

需要用量统计、成本归因、审计或诊断的宿主，应从 `conversation.ledger_delta` 记录里派生这些事实；
不要把独立的 `llm_usage` / `llm_error` 事件流作为恢复或计费契约。

只在关闭时导出一次快照不能覆盖意外崩溃。正常运行时应持续落增量；快照可作为
hydrate、迁移、人工保存或增量对账的产物。

## 7.3 宿主需要重新绑定什么

Runtime 不替宿主保活外部资源。以下内容恢复后必须由宿主重新绑定或重新注入：

- 业务对象、数据库事务、文件句柄、窗口、浏览器页、外部 session。
- 由 `conversation.set_dynamic_snapshot` 注入的 host-owned 动态快照。
- 任何时效性强、只能由宿主判断是否仍有效的业务事实。

恢复后，宿主应先绑定当前外部资源，再重新发布当前 dynamic snapshot，避免模型读取旧的
host-owned 状态。

## 7.4 恢复调用

宿主发生意外错误后，可以用已保存的 ledger/state delta 重建
`agent-runtime-conversation-snapshot/v1`，再调用：

- `conversation.spawn_from_snapshot`：创建新的 conversation 并导入 snapshot。
- `conversation.import_snapshot`：把 snapshot 导入指定 conversation。

导入时 FFI 会把 snapshot 交给 `ai-assistant` 恢复。恢复结果由核心运行时根据 ledger 尾部
决定是进入 `thinking`、`executing` 还是 `suspended`；FFI 不在事件层伪造历史。

## 7.5 Shutdown 与事件 drain

显式调用 runtime shutdown 或手动关闭 pod 时，关闭是 conversation 级别逐个发生的：

1. Runtime shutdown 会枚举仍存在的 conversations，并逐个调用 conversation close。
2. 每个 conversation 关闭时，可能先产生最后的 ledger/state 增量，再产生
   `conversation:closed`。
3. 不同 conversation 因当前运行状态不同，关闭期间产生的事件数量和顺序可能不同。
4. FFI handle 进入 shutdown/closing 后，普通 invoke 会被拒绝，但 `next_event` 仍是
   drain 通道。
5. 宿主不要因为收到某一个 `conversation:closed` 就销毁 handle；应继续 drain event queue，
   并持续跟踪所有自己管理的 conversations。
6. 当所有 tracked conversations 都已收到 `conversation:closed`，且事件队列已经 drain 完，
   再进入最终 destroy / pod 退出流程。

`conversation:closed` 只表达某个 conversation 生命周期结束，不表达整个 Rust runtime
handle 已经结束。整个 handle 的关闭完成由 shutdown/destroy 调用结果和事件 drain 共同决定。

## 7.6 幂等与对账

推荐宿主把以下键作为幂等边界：

```text
ledger: conversation_id + record_id
state delta: conversation_id + op + op-specific id/version
conversation lifecycle: conversation_id + lifecycle event type
```

如果宿主检测到事件序列跳跃或前端断线，应优先用最近持久化增量和必要的 snapshot 对账；
不要把 `frontend:state_snapshot` 当成唯一持久化来源。
