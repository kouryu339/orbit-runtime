# 5 Runtime 事件与宿主传输契约

ABI 1 的原生事件传输是 `agent_runtime_next_event_v1` pull queue，不存在 callback。
SSE、MQ、Redis Stream 等都是宿主拿到事件 JSON 后的外部适配。

```text
conversation event buses + global workflow projector -> Runtime export -> handle queue
-> next_event_v1 -> host adapter -> UI / SSE / MQ / Stream
```

## 5.1 Envelope

```json
{
  "schema":"agent-runtime-event/v1",
  "cluster_id":"service",
  "runtime_profile_id":"service-v1",
  "runtime_instance_id":"runtime-1",
  "cluster_fingerprint":null,
  "conversation_id":"conv-1",
  "conversation_event_seq":39,
  "event_seq":588,
  "event_id":"uuid",
  "timestamp":"2026-06-19T08:00:00Z",
  "source":"frontend:state_snapshot",
  "type":"frontend:state_snapshot",
  "payload":{}
}
```

`conversation_id` 与 `conversation_event_seq` 只属于 Conversation 事件。全局 Workflow
事件使用 `event_line: "workflow"`，并在 payload 中携带 `workflow_id`；它不会被绑定到
Conversation。二者可以共用同一个 pull queue，但语义事件线与聚合根保持分离。

两个 sequence 只在当前 Runtime 实例内辅助排序，不是跨进程持久 cursor。event queue
是消费式队列；TIMEOUT 不是错误事件。宿主需要重放、全局 cursor、SSE heartbeat、
鉴权或 fan-out 时，在取出 JSON 后自行实现。

## 5.2 事件出口

本文只描述 event envelope 和宿主 transport 形态，不再维护独立事件清单。稳定事件出口以
[`07-runtime-persistence-and-recovery-contract.md`](07-runtime-persistence-and-recovery-contract.md)
为准，不要把本文当成第二份事件类型注册表。

### 5.2.1 工具权限状态

工具权限不再使用独立稳定事件出口。待确认列表由
`frontend:state_snapshot.payload.pending_permissions` 承载；对应工具气泡状态由
`metadata.subtype = "tool_call_permission_requested"` 的 ledger 记录承载。
宿主渲染申请后，使用 FFI command `conversation.resolve_tool_permission` 回送
`conversation_id + tool_call_id + decision`。不要创建第二个 permission request id，
也不要把某个事件到达本身当作工具已经执行。请求方向由 snapshot/event 驱动，
决定方向是宿主 command；不要等待或虚构公共 permission-response 事件。
字段规范使用 snake_case；为兼容旧 Lit 客户端，该命令也接受
`conversationId + toolCallId`，但新集成不应继续生成 camelCase ABI payload。

LLM 用量/错误事实由 `conversation.ledger_delta` 记录承载，其中
`metadata.subtype = "llm_usage"` 或 `"llm_error"`。Runtime 不导出独立的公共
`llm_usage` / `llm_error` 事件流。原因是 `frontend:state_snapshot` 是 UI 投影，可能只包含当前焦点 Agent，不能覆盖后台 Agent。

宿主不得用日志 tail 替代事件队列，也不得假设一次 `invoke` 后立即已有最终 assistant
结果。`send_message` 返回 admission，进度和最终状态从事件流获得。

## 5.3 SSE 适配

```text
id: <host-owned-cursor>
data: <complete-agent-runtime-event-json>

```

`id`、`Last-Event-ID` 和 heartbeat 均由宿主负责。`data` 必须保留完整 Runtime envelope。
