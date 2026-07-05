# 2 Runtime 创建参数契约

本文描述当前 `agent-runtime-create-options/v1` 边界。已经移除的 Redis 内部运行态配置不再出现在配置契约中：runtime 内部使用进程内内存状态、进程内 coordination 和进程内事件序号；跨 pod 路由、owner lease、状态镜像、Redis Stream、Kafka、RocketMQ、HTTP SSE 都属于宿主或外部控制面。

当前 ABI 1 使用 pull event queue。runtime 内部不支持旧版 Redis/cache/coordination/events sink 配置；未知字段即使被反序列化忽略，也不代表旧语义仍然生效。

新的宿主优先使用显式的 start 前注册流程：

```text
create options -> runtime.register_resources -> [runtime.register_llm 空/可选] -> runtime.register_agent_cluster -> start
```

这会把产品资源与 Agent profile、模型/provider 选择、具体 conversation cluster
实例分开。Resources、LLM、Cluster 三个配置已经覆盖旧 assistant 入口承担的工作。

一句话原则：新接入不要再把 Agent、工具、模型和知识库塞进 Runtime create options；
它只回答“这个 runtime 进程怎么启动”，业务能力由 start 前三类注册回答。

相关文档：

- FFI 调用顺序：[`01-runtime-ffi-usage-guide.md`](./01-runtime-ffi-usage-guide.md)
- 事件 envelope 与前端消费：[`05-runtime-event-format.md`](./05-runtime-event-format.md)
- 前端消息契约：[`06-runtime-frontend-message-contract.md`](./06-runtime-frontend-message-contract.md)
- RPC 工具：[`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md)
- Skill 编写：[`10-runtime-skill-authoring-guide.md`](./10-runtime-skill-authoring-guide.md)

## 2.1 顶层结构

```json
{
  "schema": "agent-runtime-create-options/v1",
  "log_level": "info",
  "language": "zh-CN",
  "restore_policy": "strict",
  "data_dir": "./data/runtime"
}
```

| 字段 | 必填 | 说明 |
|---|---:|---|
| `schema` | 否 | 固定为 `agent-runtime-create-options/v1`；传空字符串时使用默认参数。 |
| `log_level` | 否 | 默认日志级别。 |
| `language` | 否 | prompt 与部分 UI 文案语言。 |
| `restore_policy` | 否 | 恢复策略枚举：`strict`、`compatible`、`unsafe_force`。 |
| `data_dir` | 否 | runtime 本地辅助目录，用于日志和本地辅助状态。 |

正式产品配置只应以 `resources.json`、`llm-providers.json` 和
`agent-cluster.json` 为准。运行时内部为了启动引擎而存在的默认值不构成公开配置契约。

LLM/provider 配置允许为空或延迟。新的宿主首次启动时可以不提供 `llm-providers.json`，
也可以注册一个没有 provider、没有当前模型的空文档。Resources 与 cluster 配置仍然负责定义
有哪些 Agent 和工具；它们不得内嵌 provider 密钥，也不得写入虚假的模型占位。在宿主后续调用
`runtime.configure_providers` 或 `runtime.reload_llm` 并选择有效当前模型前，conversation 可以
创建/恢复，但需要 LLM 的 turn 必须以可恢复的 provider/model 配置错误停止。产品 UI 应引导
用户先配置模型与厂商，而不是随发布包写入示例凭据。

Agent 的 system prompt 输出约束也归属于 Agent profile 或具体 cluster Agent 实例，
不属于 provider/model config，也不属于 conversation archive 状态。新配置使用
`systemPromptConstraints` 承载这类 prompt 级规则：

```json
{
  "id": "service.researcher",
  "name": "Researcher",
  "role": "researcher",
  "systemPromptConstraints": {
    "frontendWidgetsEnabled": false
  }
}
```

旧的 `frontendWidgetsEnabled` / `frontend_widgets_enabled` 布尔字段仍兼容读取；
新配置应优先使用 `systemPromptConstraints`。

## 2.2 已移除 Runtime Config 字段的归属

| 旧字段 | 当前归属 |
|---|---|
| `skills_dir` | `runtime.register_resources` 的 `skills.root_dir`。 |
| `max_thinking_rounds` | `runtime.register_agent_cluster` 的 `max_thinking_rounds`。 |
| `cluster_id` | Agent Cluster 注册的 `id`。 |
| `runtime_profile_id` / `runtime_instance_id` | 宿主或控制面的身份信息，不属于 Runtime 公开配置。 |
| `persistence` | 宿主行为；Runtime 不拥有产品会话持久化策略。 |
| `llm_config_dir` / `llm_config_path` | `runtime.register_llm`。 |

不再支持作为 runtime 内部依赖的字段：

- `cache_backend`
- `coordination`
- `events`
- `events.sink`
- Redis URL 复用为内部 cache/ledger/lease/event sink

如果旧配置仍包含这些字段，serde 默认可能会忽略未知字段，但这只是解析层行为，不代表 runtime 会继续执行旧语义。新文档、新示例和生产配置都应删除这些字段；需要 Redis Stream、MQ、SSE、状态镜像或跨 pod lease 时，放到宿主应用或外部控制面配置。

## 2.3 不支持的旧入口

以下旧入口不属于当前契约，新示例和生产配置都不要使用：

- `agent`
- `agents`
- `workflow`
- `retrieval`
- `runtime.retrieval`
- `rpc_tools`

替代关系：

| 旧入口 | 当前配置 |
|---|---|
| `agents` / `agent` | `resources.json` 的 Agent profiles + `agent-cluster.json` 的具体 Agent 实例 |
| `rpc_tools` | `resources.json` 的 `rpc_endpoints[]` |
| `retrieval` / `runtime.retrieval` | resource profile 或 `cluster.agents[].retrieval` |
| `workflow` | resources 注册的 workflow 能力 |

旧版顶层 `retrieval` 和 `runtime.retrieval` 都会被拒绝。资源注册表通过
`rpc_endpoints[]` 注册一个或多个知识库端点；Agent profile 的 `retrieval.endpoint_id`
选择其中一个端点。`cluster.agents[].retrieval` 可以覆盖 profile 默认值，实例配置
优先。启用检索时，端点必须存在且协议必须为 `json-lines`。

`RagRetrieve` 是 runtime local glue，不需要作为业务 `rpc_tools` 重复注册。它是否
能被 AI 主动调用仍由当前 role/feature Skill 的 `tools` 白名单决定；未列入 Skill
时仍可执行该 Agent 配置的默认 before-thinking 召回。

## 2.5 Cluster 工具权限

`agent-runtime-agent-cluster-registration/v1` 可在 cluster 根节点配置工具执行策略：

```json
{
  "permissions": {
    "read_only": "full",
    "controlled_change": "ask",
    "destructive": "deny"
  }
}
```

Runtime 直接使用工具已有的 `readonly` 与 `destructive` 元数据分类，不增加另一套工具权限标记：

| 工具元数据 | 分类 |
|---|---|
| `readonly=true, destructive=false` | `read_only` |
| `readonly=false, destructive=false` | `controlled_change` |
| `readonly=false, destructive=true` | `destructive` |
| 两者都为 `true` | 配置非法，拒绝执行 |

每一类的模式为 `full`、`ask` 或 `deny`。默认全部为 `full`，保持未配置 cluster 的现有行为。`ask` 会在工具实际执行前产生权限事件并暂停该工具调用；`deny` 不产生申请，直接向 Agent 返回“策略拒绝且未执行”的工具结果。标记为 `secret` 的工具不会把参数值暴露到权限事件。

## 2.6 工具端点

FFI 配置页不支持 `rpc_tools` 旧入口。外部业务工具端点必须放在
`agent-runtime-resource-registration/v1` 的 `rpc_endpoints[]` 中，并通过
`runtime.register_resources` 在 start 前注册。工具是否对模型可见，还要由
role/feature skill 的 `tools` 字段引用具体工具名。

正式业务工具推荐 `protocol = "grpc"`。完整契约见
[`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md)。

## 2.7 事件出口

FFI core 不通过配置内置 Redis/MQ/SSE sink。宿主通过 `agent_runtime_next_event_v1` 拉取 canonical runtime event JSON 后自行选择：

- 直接更新本机 UI；
- 转成 HTTP SSE；
- 写入 Redis Stream；
- 写入 Kafka/RocketMQ/NATS 等 MQ；
- 交给状态镜像器、计费、监控或审计系统。

Go 微服务参考实现见：

```text
examples/go_order_admin/docs/01-runtime-event-adapters.md
```

## 2.8 微服务边界

runtime pod 只拥有当前进程内的执行态。微服务部署中应由宿主控制面维护：

```text
user_id
  -> backend_conversation_id
  -> runtime_conversation_id
  -> runtime_pod_id / runtime_instance_id
```

命令面必须路由到 owner runtime pod；事件面可以广播、桥接、回放和镜像。owner lease 是命令路由问题，不是 FFI 事件流问题。
