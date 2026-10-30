# RPC 事件源 v1

状态：开发分支 `jiangrong/rpc-event-sources`，尚未发布。检测 Native capabilities
的 `features.rpc_event_sources == 1`，不要凭 0.5.3 版本号推断支持。
标准协议为 `corework/proto/corework_event_source_v1.proto`；请求 Schema 见
`../event-source.schema.json`。

`EventSourceService` 独立提供 `ListEventTypes`、`Subscribe`、`Acknowledge`。
插件可只提供事件，或在同一地址同时提供 `AgentToolService`。
原工具发现与执行协议保持兼容。
Runtime 接入层管理连接、确认转发及内存限额，不新增 AI 工具、调度任务或文件 IO。
持久化、去重、业务过滤和绑定项目／员工由宿主负责。

## 宿主接口

继续使用 C ABI `agent_runtime_invoke_v1` 和 `agent-runtime-command/v1` 外层；
以下为 type 和 payload。接口仅在 start 后开放，没有新增 C 导出。

| type | payload | 含义 |
| --- | --- | --- |
| `runtime.add_event_source` | `{id, endpoint, timeout_ms?}` | 发现并追加事件端点，无需提供工具。 |
| `runtime.get_event_source_definitions` | `{}` | 查询有效目录，不暴露连接地址。 |
| `runtime.subscribe_events` | `{endpoint_id, subscription_id, consumer_id, event_types?, capacity?, after_cursor?, stream_id?}` | 创建独立订阅，返回 connecting。 |
| `runtime.poll_source_events` | `{subscription_id, limit?}` | 非阻塞读取有界批次与状态。 |
| `runtime.acknowledge_events` | `{subscription_id, cursor}` | 累计确认至已读取游标。 |
| `runtime.unsubscribe_events` | `{subscription_id}` | 取消并释放缓冲，不自动确认。 |

同一插件的工具与事件分别用 add_rpc_endpoint 和 add_event_source 注册，
两个目录分开，不需要添加假工具。事件端点 ID 不允许更新或覆盖。
Runtime 不保存宿主端点配置，重启后由宿主重新登记和订阅。
Python SDK 提供去掉 runtime. 的同名方法；Rust 新订阅方法名为 subscribe_source_events，
避免与已有 RuntimeHost.subscribe_events() 会话事件订阅冲突。Go 使用对应 PascalCase 名称；
C++ 使用 `*_json` 方法和 `get_event_source_definitions()`。

## 事件身份与恢复

目录声明 source_id、stream_id、event_types（名称、描述、schema_version、数据 Schema）、
replayable、durability（live／memory／durable）与 retention_ms。
一个 source 是一个有序流；微信应包含稳定账号身份，多分区分别注册为不同来源。
日志清空或重建时必须更换 stream_id。

事件示例：

```json
{
  "schema":"corework-source-event/v1",
  "endpoint_id":"wechat-events",
  "source_id":"wechat:account-a",
  "stream_id":"journal-generation-1",
  "event_id":"stable-event-id",
  "event_type":"wechat.message.received",
  "schema_version":"1",
  "occurred_at":"2026-10-30T01:00:00Z",
  "cursor":"opaque-exclusive-resume-token",
  "data":{"message_id":"message-123","text":"hello"}
}
```

Runtime 校验来源、版本、类型、JSON、RFC3339 时间及长度。
数据 Schema 描述业务字段，首版不在 Runtime 内执行任意业务 JSON Schema；
插件和宿主负责业务验证。事件不能绕过宿主授权或工具审批。
重放的 event_id、cursor 和内容必须保持不变；Runtime event_seq 不能替代来源 cursor。
cursor 是不透明值，不按字符串大小比较。

恢复订阅同时提交 after_cursor 和保存时的 stream_id，严格从该位置之后开始。
缺少或不匹配的 stream_id 返回 source_changed，不得跨日志恢复。
空 after_cursor：可重放来源从最早仍保留的事件开始，实时来源从订阅建立时开始。
来源以 gRPC OUT_OF_RANGE 表示 cursor_expired、FAILED_PRECONDITION 表示 source_changed。
Runtime 遇到这些错误后不会跳到最新位置或清空宿主 checkpoint。

可重放来源临时断线后从最后已确认位置重连，退避 100ms 至 5s。
未确认的重放事件在当前缓冲区内合并，宿主仍必须去重。
实时来源断线后停止为 failed/live_source_disconnected，由宿主决定是否建立新订阅并提示缺口。
没有持久日志的微信 Hook 应声明 replayable:false、durability:live。

## 接收确认与任务去重

poll 返回 events、status、error_kind、acknowledged_cursor、pending_count、capacity。
状态有 connecting／connected／reconnecting／failed／stopped；它不表示业务任务成功。
**未确认事件会在后续 poll 中再次返回**，因此读取响应丢失后可重试。
poll 本身不确认接收、不推进恢复位置。宿主不能假定每个事件只收到一次。

ACK 表示当前订阅过滤条件下，至该游标的全部已投递事件均已可靠接收。
拒绝未读取或伪造的游标。重复确认当前 checkpoint 安全；
远程 ACK 失败／超时保留 checkpoint 和待确认事件。插件也必须使重复 ACK 幂等。

宿主建议流程：

1. inbox 按 `(source_id, stream_id, event_id)` 唯一去重。
2. 同一存储事务保存接收 checkpoint；如立即触发任务，一起写待触发记录。
3. 提交后 ACK，不等待员工或工作流业务执行结束。
4. 调度器以事件身份、绑定 ID／版本生成稳定任务键。

如果稍后过滤与绑定，必须可靠扫描持久化 inbox，不能 ACK 后只依赖内存回调。
外部发送消息等副作用还需独立幂等键；不宣称 exactly-once。
取消后内存缓冲释放，未确认事件能否恢复取决于来源的日志保留能力。

## 限额与关闭

- 每 Runtime 最多 32 端点、16 订阅。同 source／consumer 仅一个本地存活订阅；
  跨 Runtime 的消费者独占由来源服务实现，不提供分布式租约。
- capacity 为 1～128，默认 32，包含已 poll 但未确认的事件；limit 同范围和默认值。
- 数据 JSON 最大 64 KiB，单个 gRPC 帧／目录最大 256 KiB。缓冲满时停止读网络，
  只有 ACK 才释放容量。单订阅最多约 8 MiB 业务原始 JSON，另有解析和网络缓冲开销。
- timeout_ms 为 1～60000，默认 5000，适用于发现、订阅握手和 ACK。
  订阅没有工具级总超时；读取空闲上限 30s，来源建议每 5s 发 heartbeat。
- 网络发现和 ACK 不持有 Runtime 全局锁。取消和 shutdown 停止后台流。
- 专用 poll 不占用原有 next_event 无界队列，避免挤占会话与审批事件。
- 注册和订阅没有新增文件 IO。端点配置与 checkpoint 由宿主保存。

## Node 插件 SDK

```js
import {MemoryEventSource, serveEventSource} from '@corework/agent-tool';
const source = new MemoryEventSource({
  sourceId:'wechat:account-a',
  replayable:false,
  eventTypes:[{
    name:'wechat.message.received', description:'当前登录账号收到消息。',
    schema_version:'1', data_schema:{type:'object'},
  }],
});
await serveEventSource('127.0.0.1:55062', source);
// Hook 回调中：source.publish('wechat.message.received', message);
```

工具和事件共存使用 `serve(address,{eventSource:source})`。
持久化插件自行实现 describe()、subscribe(request,{signal})、acknowledge(request)，
subscribe 返回可取消 AsyncIterable，再通过 eventSourceService 或 serve 挂载。
Node 包携带独立 proto；Rust 插件 SDK 的 events_proto 提供生成的客户端和服务端 trait。
其他语言可直接基于公开 proto 生成代码。

MemoryEventSource 是参考日志，默认最多 256 条／5 分钟，上限可配置到 4096 条。
即使开启 replayable，也只声明 memory，重启更换 stream_id，淘汰后报游标过期。
生产持久化来源必须先提交日志再投递，声明保留期限并保持重放身份不变。

## WASM 与验证

本轮实现 Native 接入，WASM capabilities 明确列出 rpc_event_sources 尚不支持。
浏览器后续通过宿主异步桥接相同数据／确认语义，不直接连接 gRPC。
没有修改已发布 v0.5.3 的附件。

验证：`cargo test -p agent-runtime-ffi --test event_sources`、
`npm test --prefix sdk/rpctools/node`、
`python agent_runtime_ffi/tests/event_sources.py DLL_PATH`。
均使用本地模拟来源，不启动微信、不发送真实消息、不调用真实模型。
