# 5 05 Agent、Ledger 与持久化

## 5.1 两种协作模型

### 5.1.1 固定 Agent 焦点接力

多个实例在 cluster 创建时注册。`AppointAgent` 将职责与 focus 交给目标；
`ReportToAgent` 汇报并可选择 handoff。固定角色只有 focus Agent 获得执行权，不提供
驱动非焦点 Agent 并行 thinking 的消息旁路。focus 是
ConversationState 中的实例 id；profile id 仅在能唯一解析时可作为配置引用。

该模型的首要价值是上下文隔离：低关联任务可以分离 role、feature Skills 与历史，
减少无关上下文。强关联任务默认应保留在同一 Agent 中，因为连续对话比多 Agent
handoff 更稳定；只有 Skill 上下文明显过大时，才用隔离收益换取交接成本。

### 5.1.2 后台主从任务

前台调用 `CreateBackgroundAgentTask`，Runtime 从 resources 中的 Agent profile 创建
唯一后台实例，写入 task created/assigned 事件和任务契约。后台 Agent 不改变 focus，
结束时必须调用 `ReportAgentTask`。Gateway 根据 `task_id`/reporter 找到委托关系，
更新任务终态并将报告写入委托方 ledger。需要再次工作时创建新实例，不复用一次性
任务上下文。

## 5.2 Ledger

`LedgerRecord` 是 conversation 消息真相源，包含 user、assistant、tool、agent_report、
gateway_message 和 summary 等角色。前端投影隐藏内部 summary/gateway 细节；LLM 投影
按 Agent 可见性过滤，并从最近 compact summary 后构建 history。

Agent 私有 cache 只保存执行态。跨 Agent 可见结果、焦点变化和任务报告必须经事件
路由写入 ledger/ConversationState。

## 5.3 快照

Runtime 支持 conversation snapshot 的 export/import/materialize 和 spawn-from-snapshot。
快照包含 ledger、cluster/Agent 状态及可恢复 cache 字段；宿主动态快照不作为持久业务
真相，恢复后应重新发布。展示态、pending 工具、错误提示等瞬时字段会在恢复时清理。

## 5.4 恢复入口与状态机重建

恢复不是在导入 ledger 时伪造历史，而是先分析 ledger 尾部，重建状态机现场。核心规则：

- 最后一条是 `user`：进入 `thinking`，表示用户说完但 AI 尚未开始处理。
- 最后一条是干净 `assistant` 且没有工具调用：进入 `suspended`，表示 AI 已自然回答完。
- 最后一条 `assistant` 含工具调用但还没有闭合工具结果：进入 `executing`。
- 已有 `tool_call_started` 但没有 `tool_call_finished` / `tool_call_failed`：进入 `executing`。
- 最后一条是闭合 tool result：进入 `thinking`，让 AI 基于工具结果继续下一轮。

恢复到 `thinking` / `suspended` 时只恢复状态机入口，不在恢复过程中主动发起新的模型请求。
恢复到 `executing` 时会把待执行工具、`tool_call_id` 和恢复结果写入 agent cache，
再由 executing 状态消费。

## 5.5 未闭合工具调用

未闭合工具调用按安全性处理：

- 只读工具：不写恢复结果，允许 executing 重新执行一次，相当于重新查询。
- 非只读、破坏性或安全性未知工具：不重跑真实工具，而是用原 `tool_call_id`
  写入一条恢复 tool result。该结果会告诉 AI：运行时中断，未观察到闭合结果，
  不要假设成功，也不要直接重复执行；应先查验外部系统，或向用户/主 agent 汇报不确定性。

多 Agent 场景下，未闭合工具按 `agent_id` 分组生成恢复执行计划。子 agent 的恢复
tool result 会引导它向主 agent 报告“不确定，需要查验或重新规划”，而不是直接询问用户。

## 5.6 本地文件持久化

`persistence.mode=host_managed` 时宿主拥有会话存储与恢复流程；`local_files` 保留旧的
JSONL/index/cache snapshot 调试模式。不要把本地文件、Runtime 内存和宿主数据库同时
描述成唯一真相源。

## 5.7 生命周期

Conversation close 先关闭 command gate，再调用 `Conversation::shutdown()`；Cluster
禁止新 driver、abort 并等待已记录 driver，最后清空 Agent。Runtime 级 shutdown 会
关闭所有 conversation 与 Studio 服务。对外硬保证以 FFI `shutdown_v1` 返回 OK 为准。

面向宿主的稳定事件出口由 AgentGateway 统一产生：`conversation:created`、
`conversation:closed`、`conversation.ledger_delta`、`conversation.state_delta` 和
`frontend:state_snapshot`。FFI、SDK 或应用宿主应依赖这些事件，而不是依赖内部事件名。

下一篇：[06 运行时机制](06_runtime_mechanics.md)
