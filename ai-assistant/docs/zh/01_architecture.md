# 1 01 架构设计

`ai-assistant` 是 conversation-scoped 的多 Agent 执行引擎。一个 Conversation 拥有
一个共享 `ConversationState`、一个 `AgentGateway` 和一个 `AgentCluster`；每个 Agent
拥有独立状态机与 scoped cache，但共享 ledger、focus、动态快照和后台任务榜单。

## 1.1 所有权

```text
ConversationManager
  -> ConversationRuntime
       -> Conversation
            -> ConversationState (shared component)
            -> AgentGateway
            -> AgentCluster
                 -> AgentRuntime[]
                      -> StateMachine + scoped cache
```

`ConversationManager` 负责创建、查找、命令串行化、关闭和快照操作。
`Conversation` 负责组装共享执行单元、默认 Agent、Gateway 路由和 Cluster。

## 1.2 ConversationState

`ConversationState` 是 conversation 内的 canonical mutable state：

- append-only 语义的 `LedgerRecord` 列表；
- 当前 `focus_agent_id`；
- 按 Agent 隔离的宿主动态文本字段；
- conversation-scoped `AgentTaskEntry` 榜单；
- ledger revision 和前端投影所需状态。

Agent cache 不是跨 Agent 真相源。需要被委托方、前端或恢复流程看到的事实必须写入
ConversationState/ledger，不能只留在某个 Agent 的 cache。

## 1.3 Gateway

Gateway 是消息、焦点和协作事件的协调层：

```text
host command / Agent event
  -> admission + command gate
  -> Gateway route
  -> ConversationState / AgentCluster
  -> ledger append
  -> frontend:state_snapshot and domain events
```

它处理用户消息、pause、appoint/report、后台任务 created/assigned/reported/completed、
history compact 和前端快照发布。前端只消费共享投影，不读取任意 Agent 私有 cache。

## 1.4 AgentCluster

Cluster 保存 Agent 实例并以 `ConversationState.focus()` 作为 canonical focus。固定
协作 Agent 在 conversation 创建时注册；后台 Agent 按任务动态创建且不抢 focus。
Cluster 还持有 driver `JoinHandle`，shutdown 时禁止新 driver、abort 并等待现有 driver，
然后清空 Agent 集合。

## 1.5 配置边界

- Agent profile 定义稳定身份：`id/name/role/features/retrieval`。
- cluster registration 创建具体实例，可覆盖 profile 配置并指定 focus。
- `immutable_cache` 只在实例创建时读取并注入一次；创建完成后不存在更新接口。
- role/feature Skill 的 `tools` 是模型工具白名单。
- resources 注册 Skill、workflow、数据目录、Agent profile 和外部 RPC/知识端点。

下一篇：[02 状态机与对话引擎](02_state_machine_and_conversation_engine.md)
