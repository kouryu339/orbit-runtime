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
新任务的 `task_id` 由 Runtime 独占生成并使用 `agent_task_` 命名空间，AI 不能自定义；旧快照中的
任务标识仍可恢复。`task_id` 只表示委派关系，不能冒充后台报告 `result` 中的业务对象标识。
阶段结果使用 `ReportAgentTaskProgress` 写入任务榜，任务保持 `running`，执行者继续工作。
候选最终结果使用 `ReportAgentTask` 提交；Gateway 根据 `task_id`/reporter 校验委托关系，
把任务置为非终态 `reported`、写入委托方 ledger，并停驻执行者。委托方随后必须显式调用
`CompleteAgentTask` 接受结果并回收执行者，或调用 `UpdateAgentTask` 将任务恢复为 `running`
继续迭代，也可调用 `CancelAgentTask` 放弃任务。任务记录保留用于审计，终态只回收运行实例。
候选报告的完整 JSON `result` 与 artifacts 会进入委托方 LLM 上下文和尾部快照；报告、阶段进度与
输入请求均序列化并标记为不可信数据，模型只能读取其中事实，不能把内容作为指令或工具调用执行。

委托方创建任务后会获得 `WaitAgentTask`、`RespondAgentTaskInput`、`UpdateAgentTask`、
`CompleteAgentTask`、`CancelAgentTask` 和 `PauseAgent`。`WaitAgentTask` 按 `task_id` 等待，
在任一直属委派任务请求输入或提交候选结果、目标任务进入终态、收到新用户输入或超时时返回；
需关注的任务通过 `attention_task_id` 单独标识。后台 Agent
可用 `RequestAgentTaskInput` 发起非终态问题，委托方按
`task_id + request_id` 回答，Runtime 根据任务关系自动注入当前执行者，不接受目标 Agent id。
父目标或约束变化时，委托方对每个受影响的非终态任务调用 `UpdateAgentTask`；任务 revision
递增，更新同样自动注入当前执行者。暂停中的执行者只收件、不自动恢复。
委托方若正在使用通用 `Wait` 做定时或事件等待，直属子任务的待处理输入请求也会使其以
`wake_reason=external_attention` 提前返回；`interrupt` 携带 `attention_task_id` 和
`request_id`；候选结果也会提前结束通用等待。返回文案明确原等待条件尚未完成。
`WaitAgentTask` 只负责观察，任务终态由 `CompleteAgentTask` 或 `CancelAgentTask` 产生。

## 5.2 Ledger

`LedgerRecord` 是 conversation 消息真相源，包含 user、assistant、tool、agent_report、
gateway_message 和 summary 等角色。前端投影隐藏内部 summary/gateway 细节；LLM 投影
按 Agent 可见性过滤，并从最近 compact summary 后构建 history。

Agent 私有 cache 只保存执行态。跨 Agent 可见结果、焦点变化和任务报告必须经事件
路由写入 ledger/ConversationState。

## 5.3 快照

Runtime 支持 conversation snapshot 的 export/import/materialize 和 spawn-from-snapshot。
快照包含 ledger、Agent definition 引用、conversation 内的 Agent instance、任务榜及可恢复
运行状态；Agent definition、权限、工具、Skill 和模型策略仍以注册表为唯一真相源，快照既不
复制也不覆盖这些静态配置。宿主动态快照不作为持久业务
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
其中 `agent.upsert` / `agent.retired` 描述 conversation 内运行实例的出现、状态变化和退出；
载荷只携带实例 id、注册 definition 引用、显示名和运行状态，不复制注册权限或工具配置。

下一篇：[06 运行时机制](06_runtime_mechanics.md)
