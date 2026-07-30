# 6 配置内置工具与多 Agent 协作

Runtime 内置了一组 Agent 协作工具。它们不需要外部 sidecar。入口权限由 Skill 的
`tools` 白名单控制；Runtime 建立真实委派任务后，才按委派者/执行者关系动态加入对应的
任务生命周期工具。配置多 Agent 不是打开一个总开关，而是组合：

```text
Agent profile + cluster 实例 + focus + role Skill 中的协作工具
```

## 6.1 Local 工具并非全局可见

`#[define_operation]` 注册的是 Runtime 可调用的 local AI system。注册成功只表示
它可以被 Skill 引用，不表示每个 Agent 自动看见它。最终可见集合仍由当前 Agent
的 system/role/feature Skills 合并得到。

当前面向普通 Agent 的 local 工具分组如下：

| 分组 | 工具 | 用途 |
|---|---|---|
| 等待与推理 | `Wait` | 等待超时或 conversation scope 中的指定事件；适合等待后台任务，不要轮询。 |
| 等待与推理 | `ContinueThinking` | 不执行业务动作，只请求额外一轮思考。 |
| 输出 | `WriteMarkdown` | 在 Runtime 数据目录写持久 Markdown 报告。 |
| 渐进 Skill | `GetSkillsList`, `UpdateSkills` | 发现并全量替换动态 feature Skills。 |
| 计划 | `PlanWrite`, `PlanUpdate`, `PlanFinish` | 建立、调整和完成当前执行计划。 |
| Agent 协作 | `CreateAgent`, `AppointAgent`, `DismissAgent`, `ListAgents`, `ReportToAgent` | 临时 Agent 与持久焦点协作。 |
| 后台任务 | `CreateBackgroundAgentTask`, `WaitAgentTask`, `PauseAgent`, `RequestAgentTaskInput`, `RespondAgentTaskInput`, `UpdateAgentTask`, `ReportAgentTaskProgress`, `ReportAgentTask`, `CompleteAgentTask`, `CancelAgentTask` | task board 驱动的双向主从后台协作。 |
| 知识检索 | `RagRetrieve` | 当前 Agent 配置 retrieval 后提供的本地路由工具。 |

这 24 个名称就是当前普通 Agent 可由 Skill 引用的 local AI operation 全集。定义
来源分别是 `corework/src/common_tools.rs`、`ai-assistant/src/systems`、
`ai-assistant/src/agent/systems.rs`，以及 Runtime 动态注册的 `RagRetrieve`。
ledger、prompt、Skill 装载和 Draft 系统虽然也在本地 registry 中运行，但属于状态机
内部系统，不会作为普通 Agent 的 AI tool contract 暴露。

普通工具只有被当前 Agent 激活的 system/role/feature Skill 在 `tools` 中显式引用时，
Runtime 才会把工具描述暴露给 AI；没有引用的工具即使已经注册，AI 通常也不知道
它存在。后台任务生命周期是有意保留的例外：`CreateBackgroundAgentTask` 成功后，
Runtime 只按该任务的真实委派关系动态加入后续控制，并随执行者生命周期回收；这不构成
按任意 Agent id 操作的权限。

role Skill 应只声明完成该角色核心职责所需的最小工具集；可组合的业务功能按模块
拆进 feature Skills，需要时再激活。多个激活 Skill 引用了同一个工具时，Runtime
会按工具名去重，不会重复注入工具描述，不必为了避免重复而破坏 Skill 的模块边界。

system thinking Skill 当前也只是通过自己的 `tools` 显式引入 `GetSkillsList`、
`UpdateSkills`、三个 Plan 工具和 `ContinueThinking`，并非这些工具绕过了白名单。
Runtime 会把这组工具合入每个普通 Agent 的基础白名单，`UpdateSkills` 只替换动态
feature Skills，不会移除它们。
例如主 Agent 的职责确实需要创建后台任务并等待结果时，可以在对应 role 或 feature
Skill 中声明：

```yaml
tools: ["CreateBackgroundAgentTask"]
```

创建任务后，Runtime 会向委派者加入 `WaitAgentTask`、`RespondAgentTaskInput`、
`UpdateAgentTask`、`CompleteAgentTask`、`CancelAgentTask` 和 `PauseAgent`，并向后台执行者加入
`RequestAgentTaskInput`、`ReportAgentTaskProgress` 和 `ReportAgentTask`。
`WaitAgentTask` 按 `task_id` 等待；任一由当前 Agent 委派的任务请求输入或提交候选结果、目标任务进入终态、新用户消息或超时都会唤醒。其他任务的普通状态变化不会唤醒；需关注的任务通过 `attention_task_id` 单独标识，不把原等待目标误判为完成；
新用户消息会让主 Agent 先恢复处理指令，因此可以立即决定是否暂停任务执行者。

新建委派任务的标识只由 Runtime 生成，并使用 `agent_task_` 专用命名空间；AI 调用方不能自定义
`task_id`，恢复旧快照时原有任务标识仍然有效。任务标识只是协作句柄，不能替代后台执行者在
结构化 `result` 中返回的业务标识。候选报告的完整 JSON `result` 和产物列表会保留在委托方的
LLM 上下文与尾部快照中；后台 Agent 的报告、进度和输入请求统一序列化并标记为不可信数据，
不能作为指令或工具调用执行。
后台 Agent 请求输入后，Runtime 通过不向 AI 暴露的 Corework System 校验
conversation、ExecutionUnit 和 Agent 调用身份，再由任务关系推导委派者；仅当委派者已停止时
才唤醒。用户输入和内部唤醒共用每 Agent 调度门与单一 driver 槽位。
通用 `Wait` 仍用于普通定时和事件等待，不承担任务终态判断；但直属子任务存在待处理求助或候选结果时，
它会以 `external_attention` 提前返回并给出 `attention_task_id`，避免主 Agent 在普通等待中
错过求助，同时明确原等待条件尚未完成。

## 6.2 特殊 Agent 的专属工具

两个内置特殊 Agent 使用独立 system role Skill。Workflow 工具本身是统一能力，
是否开放持久化目录操作由宿主配置的 role/feature Skill 决定。

### 6.2.1 Workflow Editor Agent

`workflow_editor` role 的白名单是：

| 工具 | 作用 |
|---|---|
| `listWorkflows` / `readWorkflow` | 查看 Draft 或 Registered 资源。 |
| `createWorkflowDraft` | 编译完整脚本并创建 Draft。 |
| `updateWorkflow` | 按稳定 id 和当前 revision 更新资源。 |
| `compileWorkflow` / `testWorkflow` | 编译或测试指定资源。 |
| `registerWorkflow` / `deleteWorkflow` | 注册或删除指定资源。 |
| `executeWorkflow` / `executeWorkflowScript` | 执行正式资源或一次性脚本。 |
| `searchSkillRefs` | 搜索业务 Skill 参考资料，辅助设计节点和参数。 |

这些工具只在 Workflow Studio conversation 中配合 editor selection/dynamic snapshots
工作，但资源真相始终属于统一 Draft/Registered 目录；浏览器画布只是 View。
Studio 内置 Agent 通过 `workflow_editor` role 获得完整 Workflow 目录工具，并通过
`systemSkills.thinking = "thinking-pro"` 获得通用脚本语法和一次性多行执行能力。普通
Agent 选择 `thinking-pro` 时只会增加 `executeWorkflowScript`；需要 Draft/Registered
CRUD、注册或正式执行时，宿主必须另行配置 role/feature Skill。旧 Draft 系统与文件路径
工具已经移除。

### 6.2.2 Agent Test Studio

Agent Test 有两个隔离角色：

| 角色 | 工具 |
|---|---|
| `agent_test_supervisor` | `AdversaryCreate`, `AdversaryDestroy`, `AdversaryInspect`, `WriteMarkdown` |
| `agent_test_adversary` | `AdversaryConclude` |

Supervisor 创建和检查对抗测试，最后用 `WriteMarkdown` 产出报告；Adversary 只能在
测试达到结论时调用 `AdversaryConclude`。Supervisor 的角色规则明确禁止
`Wait`、轮询和 Plan 工具，所以不能因为这些工具是 local system 就自动加给它。

专属工具还依赖 Studio 已打开并绑定对应 runtime；在普通 conversation 中即使
错误引用，也会因缺少 Studio runtime/context 而失败。

## 6.3 协作工具

| 工具 | 作用 |
|---|---|
| `CreateBackgroundAgentTask` | 从注册 profile 创建后台 Agent，并发布 conversation task。 |
| `ReportAgentTaskProgress` | 后台 Agent 持久化阶段进度；任务保持运行，执行者继续工作。 |
| `ReportAgentTask` | 后台 Agent 提交候选最终结果；任务进入 `reported` 并停驻，等待委托方决定。 |
| `RequestAgentTaskInput` | 当前执行者按 `task_id` 向委派者索要缺失信息，不结束任务。 |
| `RespondAgentTaskInput` | 委派者按 `task_id + request_id` 回答，并自动注入该任务当前执行者。 |
| `UpdateAgentTask` | 父目标变化后更新受影响的任务、递增 revision，并自动通知当前执行者。 |
| `CompleteAgentTask` | 委托方接受 `reported` 结果，将任务置为 completed/failed 并回收执行者。 |
| `CancelAgentTask` | 委托方显式放弃非终态任务，将其置为 canceled 并回收执行者。 |
| `WaitAgentTask` | 按当前 Agent 创建的 `task_id` 等待；输入请求、候选结果、任务终态、新用户消息或超时均会返回。 |
| `PauseAgent` | 按任务快照中的 `agent_id` 暂停当前 Agent 创建的任务执行者；支持 `wait_for_tool` 和 `detach_tool`。 |
| `AppointAgent` | 将工作和会话焦点交给已有持久 Agent。 |
| `ReportToAgent` | 向指定 Agent 汇报，可选择是否 handoff focus。 |
| `CreateAgent` | 创建临时 OneShot Agent；适合直接的一次性执行。 |
| `DismissAgent` | 销毁可移除的子 Agent。 |
| `ListAgents` | 读取当前 Agent 列表，主要用于诊断或动态选择。 |

不要把所有协作工具发给所有角色。谁能委托、谁必须报告、谁能切换焦点，应由
对应 role Skill 明确授权。

## 6.4 主从后台架构

适合“主 Agent 持续面对用户，后台 Agent 并发查资料或执行子任务”的场景。后台
Agent 不抢 focus，完成后通过 task report 回到委托方 ledger。

主 Agent role：

```yaml
---
name: service_boss
kind: role
tools: ["CreateBackgroundAgentTask"] # Runtime 会加入受任务关系约束的等待、回答、更新、完成、取消和暂停工具
---

# 服务负责人

- 政策研究任务必须调用 `CreateBackgroundAgentTask`，并使用已注册的
  `name="service.policy_researcher"`。
- `ReportAgentTask` 只是候选结果；验收后调用 `CompleteAgentTask`，需要继续修改则调用
  `UpdateAgentTask`，放弃任务才调用 `CancelAgentTask`。
```

后台角色：

```yaml
---
name: policy_researcher
kind: role
tools: ["RagRetrieve"] # Runtime 会为后台任务自动加入请求输入、进度报告和最终报告工具
---
```

resources 注册主 Agent 和可动态创建的后台 Agent profile：

```json
{
  "agents": {
    "profiles": [{
      "id": "service.boss",
      "name": "Service Boss",
      "role": "service_boss"
    }, {
      "id": "service.policy_researcher",
      "name": "Policy Researcher",
      "role": "policy_researcher",
      "retrieval": {
        "enabled": true,
        "endpoint_id": "policy-knowledge",
        "profiles": ["policy"],
        "top_k": 5
      }
    }]
  }
}
```

cluster 只需要预声明前台主 Agent：

```json
{
  "schema": "agent-runtime-agent-cluster-registration/v1",
  "id": "service-desk",
  "description": "Customer service with background research.",
  "focus_agent_id": "service.boss",
  "agents": [{
    "id": "service-boss-1",
    "profile": "service.boss"
  }]
}
```

主 Agent 调用 `CreateBackgroundAgentTask` 时使用后台 profile 的 id/name，并给出
`objective` 和 `acceptance`。Runtime 创建唯一后台实例、登记任务和委托关系。
把允许调用的后台 profile id/name 写进主 Agent role，而不是只在宿主配置中注册，能让
模型明确知道应该委派给谁；注册本身只提供可解析资源，不会自动把 profile 目录注入模型。
后台角色使用 `ReportAgentTaskProgress` 汇报阶段结果并继续工作；使用 `ReportAgentTask`
提交候选最终结果并停驻。委托方调用 `CompleteAgentTask` 接受，或用 `UpdateAgentTask`
恢复同一执行者继续迭代，也可用 `CancelAgentTask` 放弃任务。
缺少信息时，后台角色调用 `RequestAgentTaskInput`，委派者由 `WaitAgentTask` 唤醒后使用
`RespondAgentTaskInput` 回答。用户修改父目标时，主 Agent 应对每个受影响的任务调用
`UpdateAgentTask`；Runtime 只负责鉴权和精确路由，不依靠关键词猜测任务相关性。

## 6.5 协作焦点架构

协作焦点架构首先是一种上下文隔离和优化策略：让不同角色只加载自己的 role、
feature Skills 和任务历史，避免无关工具说明与上下文持续挤占同一个 Agent。

它适合关联性较低、责任边界清楚的阶段性任务，例如售前、独立风控审核、售后处理。
如果多个步骤强依赖同一段对话、隐含判断或连续推理，优先让单一 Agent 保持上下文；
明确而稳定的连续对话通常比未知的多 Agent handoff 更可靠。只有某个角色需要加载的
Skill 上下文明显很大，隔离收益足以覆盖交接成本时，才建议对强关联任务使用焦点接力。

所有固定 Agent 在 cluster 注册时已经存在，focus 表示当前唯一负责与用户交互和推进
任务的 Agent。

```json
{
  "schema": "agent-runtime-agent-cluster-registration/v1",
  "id": "commerce-team",
  "description": "Sales, risk, and after-sales collaboration.",
  "focus_agent_id": "commerce.sales",
  "agents": [{
    "id": "sales-1",
    "profile": "commerce.sales"
  }, {
    "id": "risk-1",
    "profile": "commerce.risk"
  }, {
    "id": "after-sales-1",
    "profile": "commerce.after_sales"
  }]
}
```

`focus_agent_id` 可以写具体实例 id；当某 profile 在 cluster 中只有一个实例时，也
可以写 profile id。一个 profile 有多个实例时必须写具体实例 id，避免焦点歧义。

建议授权：

```yaml
# sales role
tools: ["AppointAgent"]

# risk / after-sales role
tools: ["ReportToAgent"]
```

- `AppointAgent`：把明确责任和必要上下文交给目标，并切换 focus。
- `ReportToAgent --handoff true`：汇报后把 focus 交回目标。
- `ReportToAgent --handoff false`：只报告事实，当前 Agent 继续负责。

固定角色协作不提供“向非焦点 Agent 发消息并让其并行 thinking”的旁路。需要目标
Agent 执行时必须用 `AppointAgent` 转移 focus；需要真正并发时使用后台任务体系。
不要靠提示词假装切换角色。真正的责任转移必须通过协作工具产生路由和 focus 事件，
前端始终按 conversation 的 canonical focus/state 渲染。

## 6.6 如何选择

| 需求 | 架构 |
|---|---|
| 主 Agent 始终面对用户，子任务并发执行 | 主从后台 |
| 低关联、边界清楚的固定专家任务，需要隔离上下文 | 协作焦点 |
| 强关联、依赖连续对话的多个步骤 | 单 Agent；除非 Skill 上下文过大才考虑协作焦点 |
| 单次、无需任务榜单的短任务 | `CreateAgent` OneShot |

两种架构可以组合，但同一项工作只能有一个明确委托方和一个验收出口。
