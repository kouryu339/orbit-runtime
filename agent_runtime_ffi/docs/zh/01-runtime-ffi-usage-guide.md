# 1 Agent Runtime ABI 1 对外协议契约

本文是 `sdk/runtime/c/include/agent_runtime.h` 与 `agent_runtime_ffi/src/lib.rs` 的宿主
契约。ABI 1 只导出生命周期和传输函数；功能通过版本化 JSON command 扩展。

## 1.1 类型、返回码与通用规则

```c
typedef uint64_t AgentRuntimeHandle;
```

`0` 是无效 handle。返回码：`0 OK`、`1 INVALID_ARGUMENT`、`2 INVALID_HANDLE`、
`3 BAD_STATE`、`4 TIMEOUT`、`5 UNSUPPORTED`、`100 RUNTIME`、`101 PANIC`。

- 输入字符串必须是非 NULL、NUL 结尾的 UTF-8。
- `char**` 输出在调用开始时置 NULL；非 NULL 成功输出由宿主调用
  `agent_runtime_free_string_v1` 恰好释放一次。
- 直接返回的 `const char*` 是静态或线程局部借用指针，不得释放。
- 不调用宿主 callback。事件只能通过 `next_event_v1` 拉取。
- 不同线程可以调用；同一 handle 的 facade 操作由内部锁串行化。
- shutdown 开始后不得再发 start/invoke；如果宿主需要关闭期 ledger delta 或
  conversation closed 通知，应继续调用 `next_event_v1` drain，直到队列关闭返回
  `BAD_STATE`。

## 1.2 个导出函数

### 1.2.1 `agent_runtime_abi_version_v1`

```c
uint32_t agent_runtime_abi_version_v1(void);
```

始终返回 `1`。非阻塞、线程安全，无错误通道。宿主必须先以 ABI major 判断能否加载。

### 1.2.2 `agent_runtime_version_v1`

返回 Cargo 产品版本的静态 UTF-8 指针，永不为 NULL，DLL 卸载前有效，不得释放。
产品版本不等于 ABI 兼容版本。

### 1.2.3 `agent_runtime_capabilities_v1`

返回静态 `agent-runtime-capabilities/v1` JSON，包含 ABI minor、当前 command 列表、
pull event transport、shutdown timeout/retry 和线程模型。宿主必须按该列表做功能发现。

### 1.2.4 `agent_runtime_create_v1`

```c
int agent_runtime_create_v1(const char* create_options_json,
                            AgentRuntimeHandle* out_handle);
```

接受空字符串默认参数，或 inline `agent-runtime-create-options/v1` JSON，创建未启动
Runtime。不支持配置文件路径。create options 只包含 `log_level`、`language`、
`restore_policy`、`data_dir` 这类进程级启动参数；Agent、工具、LLM provider、
Skill 和 cluster 通过 Resources、LLM、Agent Cluster 三类注册表达。真实 Agent 宿主
必须在 `start` 前注册 resources 和 cluster；LLM/provider 可为空或延迟配置，但未配置
provider 与当前模型前，LLM turn 不能完成。
`out_handle` 必须非 NULL，失败时写 `0`。成功后 handle 归宿主所有，最终必须先
shutdown 再 destroy。

### 1.2.5 `agent_runtime_start_v1`

冻结注册并初始化资源、LLM、RPC/retrieval 和内置服务。handle 必须处于 OPEN；重复的
成功 start 幂等。启动后再执行 registration command 是否被接受由 Runtime 状态校验，
宿主应在 start 前完成注册。

### 1.2.6 `agent_runtime_invoke_v1`

```c
int agent_runtime_invoke_v1(AgentRuntimeHandle handle,
                            const char* request_json,
                            char** out_response_json);
```

请求必须是：

```json
{"schema":"agent-runtime-command/v1","id":"host-1","command_id":"optional","type":"conversation.send_message","payload":{}}
```

`payload` 省略/null 等价空对象；非对象非法。未给 `command_id` 时 Runtime 生成
`ffi_cmd_N`。若已进入 command dispatch，无论业务成功失败都尽力返回
`agent-runtime-result/v1`：

```json
{"schema":"agent-runtime-result/v1","id":"host-1","command_id":"ffi_cmd_1","ok":true,"result":{}}
```

业务失败时 `ok=false` 且包含 `error`，同时 C 返回码非零。JSON 解析、空指针、handle
状态等边界错误可能没有 response，只能读取 last_error。同一 handle 的 invoke 串行，
调用可能持续到该操作完成或准入结果产生。

### 1.2.7 `agent_runtime_next_event_v1`

从 handle 私有队列取一个 `agent-runtime-event/v1` JSON。`timeout_ms=0` 非阻塞；正数
最多等待对应毫秒。无事件返回 `TIMEOUT` 且输出保持 NULL；队列关闭返回 BAD_STATE。
shutdown 期间及之后，宿主仍可继续 poll 来 drain 已入队的关闭期事件，直到
`BAD_STATE` 表示队列已关闭。成功字符串必须 free。多个线程同时消费同一队列会竞争事件，
宿主通常应只设一个 reader。

### 1.2.8 `agent_runtime_shutdown_v1`

首次调用立即将 handle 置 CLOSING，新 runtime 调用返回 BAD_STATE；随后等待在途 FFI
调用归零，在 worker 中关闭 conversations、Studio 和事件生产者。`next_event_v1`
在关闭期间仍可用于 drain 已排队的 shutdown 事件。超时返回 TIMEOUT，关闭继续在后台进行，
宿主可用同一 handle 重试。只有返回 OK 才保证状态为 CLOSED。

### 1.2.9 `agent_runtime_destroy_v1`

仅移除已经 CLOSED 且无在途调用的 handle。未完成 shutdown 返回 BAD_STATE。成功后
该数值 handle 永久无效；destroy 不隐式 shutdown。

### 1.2.10 `agent_runtime_last_error_json_v1`

返回当前宿主线程最近一次失败 ABI 调用的 `agent-runtime-error/v1`：

```json
{"schema":"agent-runtime-error/v1","code":3,"kind":"bad_state","message":"..."}
```

指针可为 NULL，只到同线程下一次 ABI 调用前有效，不得 free。成功 ABI 调用会清空
该线程错误。必须在失败调用的同一线程立即读取。

### 1.2.11 `agent_runtime_free_string_v1`

释放 invoke/next_event 通过 `char**` 转移给宿主的指针。NULL 可安全传入；静态版本、
capabilities、last_error 指针或任意外部指针传入属于未定义行为。

## 1.3 Command 契约

注册类 payload 可用 `{"input":"path-or-json"}` 或 `{"registration":{...}}`。

## 1.4 start 前的三类注册配置

`agent_runtime_create_v1` 只创建一个尚未启动的 Runtime handle。宿主要运行真正的
Agent，应在 `agent_runtime_start_v1` 前注册产品资源、可为空的 LLM/provider 状态和
具体 Agent cluster：

| 配置 | 命令 | 作用 |
|---|---|---|
| Resource registration | `runtime.register_resources` | 注册 Skill 根目录、Agent profiles、工具/RAG endpoints、workflow 根目录和 data/log 根目录。 |
| LLM/provider registration | `runtime.register_llm` | 首次启动可为空。注册模型 provider、凭据/base URL、模型 id、上下文窗口和当前模型；也可用于延迟模型配置的空注册。 |
| Agent cluster registration | `runtime.register_agent_cluster` | 从 profile 创建具体 Agent 实例，并声明初始 focus 种子。后续焦点交接和路由由 Runtime 内置机制负责。 |

这些注册属于宿主管理面，应在 `start` 前完成；普通前端客户端不应直接获得这些管理
命令。`start` 后，这些注册在当前 Runtime 生命周期内视为冻结，宿主应基于已注册的
cluster id 创建 conversation。

不要把旧 `agents`、`agent`、`rpc_tools`、`retrieval` 或 `workflow` 放进
create options。FFI 文档不提供这些旧入口的使用方式；对应能力必须分别由 Resources、
LLM 和 Agent Cluster 三类注册配置表达。

| type | 时机与 payload | result |
|---|---|---|
| `runtime.register_resources` | start 前；`registration` JSON object | `{}` |
| `runtime.register_llm` | start 前；`registration` JSON object | `{}` |
| `runtime.reload_llm` | `input` 或 `registration` JSON/file；接受 `agent-runtime-llm-registration/v1` 或 provider config | `{}` |
| `runtime.register_agent_cluster` | start 前；`registration` JSON object | `{}` |
| `runtime.set_auth_context` | `context`，省略时使用整个 payload | `{}` |
| `runtime.configure_providers` | `registration` JSON object | `{}` |
| `runtime.get_provider_definitions` | 空 payload | provider definitions JSON/string |
| `runtime.set_current_model` | `model_uid:uint32` | `{}` |
| `runtime.set_language` | `language:string` | `{}` |
| `runtime.export_snapshot` | 空 payload | runtime snapshot |
| `conversation.spawn` | `spawn` 对象或直接展开 spawn 字段 | conversation info |
| `conversation.spawn_from_snapshot` | `spawn` + `snapshot` | conversation info + `restored:true` |
| `conversation.send_message` | `conversation_id`, `content` | admission decision |
| `conversation.pause` | `conversation_id` | admission decision |
| `conversation.close` | `conversation_id` | `{}` |
| `conversation.export_snapshot` | `conversation_id`, `options?` object/string | conversation snapshot |
| `conversation.agent_tasks` | `conversation_id` | `agent-runtime-agent-tasks/v1` |
| `conversation.materialize` | `conversation_id`, `options?` | info + `state_loaded:true` |
| `conversation.import_snapshot` | `snapshot`, `options?` | `{}` |
| `conversation.set_dynamic_snapshot` | `conversation_id`, `agent_id`, `field_name`, `text` | `{}` |
| `conversation.resolve_tool_permission` | `conversation_id`, `tool_call_id`, `decision:"allow"|"deny"` | `{resolved:boolean}`；`false` 表示申请已经结束或不存在 |
| `conversation.set_summary_model` | `conversation_id`, `model_name` | admission decision |
| `conversation.compact_history` | `conversation_id`, `agent_ids?:string[]` | admission + report |
| `studio.open_workflow` | `options?` | Workflow Studio open result |
| `studio.open_agent_test` | `options?` | Agent Test Studio open result |

`conversation.spawn_from_snapshot` 消费的是持久化的
`agent-runtime-conversation-snapshot/v1`，用于恢复或复制 conversation。它不是
“从尾部观察快照继续运行”的语义；尾部快照更适合 UI 刷新、导出或诊断。

registration、spawn、snapshot 和 event 的详细 schema 分别见配置契约、Skill/RPC 指南、
前端消息契约与事件格式文档。capabilities 是运行时最终功能发现来源。

## 1.5 正确调用顺序

```text
abi/version/capabilities
-> create
-> invoke register_resources/[register_llm 或延迟空模型]/register_agent_cluster
-> start
-> invoke commands + next_event pull loop
-> shutdown (TIMEOUT 可重试)
-> destroy
```
