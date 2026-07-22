# 10 Runtime Skill 编写指南

本文描述 runtime 下 `SKILL.md` 的编写契约，重点说明 role/feature 的区别、`tools` 字段的可见性规则，以及 runtime 内置系统、RAG、用户自定义 RPC 工具如何被 skill 引用。RPC 工具集自身的定义、`ToolDescriptor`、`AIOutput.to_ai` 和 sidecar 接入规范，见 [`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md)。

## 10.1 Skill 类型

runtime 当前把 skill 分为两类核心用途：

| 类型 | `kind` | 用途 |
|---|---|---|
| role skill | `role` | 定义 agent 的身份、职责、边界、语气、业务权限和默认工具集合。 |
| feature skill | `feature` | 定义某一类可动态启用的能力、流程、操作规范或业务工作流。 |

agent 有且仅有一个身份标识，这个身份标识读取自 role skill。也就是说，一个 agent 应该只绑定一个 role skill；feature skill 是挂在这个身份之上的可选能力扩展。

### 10.1.1 Role Skill

role skill 回答“这个 agent 是谁、负责什么、默认能做什么”。

```yaml
---
name: order_admin
description: "Buyer-facing order and after-sales assistant."
kind: role
tools: ["UserGet", "OrderList", "OrderGet", "ProductSearch", "RagRetrieve"]
---
```

适合放在 role skill 里的内容：

- agent 的身份和职责边界。
- 默认业务工具，例如订单、用户、商品、前端跳转工具。
- 用户数据隔离、权限、安全规则。
- 对所有 feature 都应该生效的业务约束。

不建议一个 agent 同时加载多个 role skill。多个 role 会让身份、职责和工具权限变得冲突。

### 10.1.2 Feature Skill

feature skill 回答“在当前身份下，额外启用哪种能力或流程”。

```yaml
---
name: workflow
description: "Shopping and after-sales workflow."
kind: feature
tools: ["OrderReturnEligibilityCheck", "ReturnCreate", "ReturnPlan", "ReturnReview"]
---
```

适合放在 feature skill 里的内容：

- 某个业务流程，例如退货、改地址、商品导购。
- 某个工具组合的调用顺序和判断条件。
- 某种临时能力，例如 web fetch、office 文档处理、workflow 执行。

feature skill 可以由 `UpdateSkills` 动态切换。切换 feature 后，runtime 会根据当前 active skills 重新计算可见工具。

### 10.1.3 Skill 目录布局与升级迁移

`skills.root_dir` 支持两种布局，但不能混用。

推荐使用分层布局：

```text
skills/
  role/
    product_instance/
      SKILL.md
  feature/
    workflow/
      SKILL.md
```

兼容的旧扁平布局如下：

```text
skills/
  product_instance/
    SKILL.md
```

目录检测规则是：只要 `skills.root_dir` 下存在 `system/`、`role/`、`main/` 或 `feature/` 中任意一个目录，Runtime 就进入分层模式。进入分层模式后，role 只从 `role/<name>/SKILL.md` 读取（`main/` 仅作为旧 role 目录兼容），feature 只从 `feature/<name>/SKILL.md` 读取，不再扫描根目录中的 Skill。

因此不要使用下面这种混合布局：

```text
skills/
  product_instance/SKILL.md
  feature/workflow/SKILL.md
```

从旧扁平布局升级到分层布局时，应把 role **移动**到 `skills/role/<name>/SKILL.md`，不要在根目录和 `role/` 下复制两份。`kind: role` 的 Skill 也不能迁移到 `feature/`。Runtime 启动时会验证所有已配置 Agent（包括 cluster Agent）引用的 role，因此错误目录会在启动阶段直接报告。

## 10.2 Tools 可见性规则

`SKILL.md` 的 `tools` 字段不是说明文字，而是 AI 工具可见性的关键入口。

一个工具想被 LLM 看到，必须同时满足两件事：

1. 工具已经注册到 runtime。
2. 工具名写进当前可用 skill 的 `tools` 字段。

只把用户自定义 RPC 工具端点写进 `resources.json` 的 `rpc_endpoints[]` 还不够。`rpc_endpoints[]` 只负责把工具端点注册进 runtime；如果没有在 role 或 feature skill 的 `tools` 中引用具体工具名，AI 依旧看不到这个工具定义，也不会主动调用它。

```yaml
---
name: order_admin
kind: role
tools: ["UserGet", "OrderList", "OrderGet", "FrontendNavigate"]
---
```

`tools` 中的名称必须是系统注册名，大小写敏感。它可以引用三类工具：

| 来源 | 示例 | 说明 |
|---|---|---|
| runtime 内置 AI 系统 | `GetSkillsList`, `UpdateSkills`, `PlanWrite` | 由 `ai-assistant` / `corework` 注册。 |
| runtime 内部胶水系统 | `RagRetrieve` | 例如 RAG 二次召回工具，不需要写入 `rpc_tools`。 |
| 用户自定义 RPC 工具 | `OrderList`, `UserGet`, `FrontendNavigate` | 由 `resources.json` 的 `rpc_endpoints[]` 注册端点，然后还必须被 skill 的 `tools` 引用。 |

## 10.3 推荐写法

role skill 通常声明 agent 的常驻业务工具：

```yaml
---
name: order_admin
kind: role
tools: [
  "UserGet",
  "UserRiskAssess",
  "OrderList",
  "OrderGet",
  "ProductSearch",
  "FrontendNavigate"
]
---
```

feature skill 通常声明某个流程额外需要的工具：

```yaml
---
name: return_workflow
kind: feature
tools: [
  "OrderReturnEligibilityCheck",
  "ReturnCreate",
  "ReturnPlan",
  "ReturnReview"
]
---
```

系统层 thinking skill 通常声明通用调度和计划工具：

```yaml
---
name: thinking
system_layer: true
tools: ["GetSkillsList", "UpdateSkills", "PlanWrite", "PlanUpdate", "PlanFinish"]
tool_filter: "all"
---
```

系统层 thinking skill 不是 role，也不应定义 Agent 的身份、职责、领域或语气。它只规定通用的思考、行动、计划、工具调度与验证方式；Agent 的身份和业务边界仍由 role skill 与宿主上下文决定。

## 10.4 Runtime 内置系统

以下系统是当前建议给 skill 引用的 runtime 内置 AI 系统。

| 系统名 | 类别 | 建议用途 | 备注 |
|---|---|---|---|
| `GetSkillsList` | skill 管理 | 查看当前可用 skills 和已激活 skills。 | 只读。 |
| `UpdateSkills` | skill 管理 | 切换当前需要导入的 feature skills。 | 会重算 active tools。 |
| `PlanWrite` | 计划 | 创建当前执行计划。 | 适合多步骤任务。 |
| `PlanUpdate` | 计划 | 更新当前执行计划。 | 需要已有 active plan。 |
| `PlanFinish` | 计划 | 结束当前执行计划。 | 完成后清理 active plan 状态。 |
| `CreateAgent` | agent 协作 | 创建一次性子 agent 执行独立任务。 | 需要指定 role skill。 |
| `AppointAgent` | agent 协作 | 把焦点交给已有持久 agent。 | 面向多 agent 调度。 |
| `ListAgents` | agent 协作 | 查看可用 agent。 | 只读。 |
| `ReportToAgent` | agent 协作 | 子 agent 向目标 agent 汇报结果。 | 默认 agent 不能向自己汇报。 |
| `DismissAgent` | agent 协作 | 销毁子 agent。 | 破坏性操作，谨慎暴露。 |
| `RagRetrieve` | RAG | 显式二次召回。 | 名称来自当前 Agent 的 `retrieval.tool_name`，默认 `RagRetrieve`。 |

### 10.4.1 Skill 管理参数

| 系统名 | 参数 | 说明 |
|---|---|---|
| `GetSkillsList` | 无 | 返回可用 skills 与当前 active skills。 |
| `UpdateSkills` | `skills` | 替换当前导入的 feature skills。多个 skill 名称按当前工具协议传入。 |

`UpdateSkills` 是替换语义，不是增量追加。调用后 runtime 会根据新 skill 集合重新计算 active tools。

### 10.4.2 计划工具参数

| 系统名 | 参数 | 说明 |
|---|---|---|
| `PlanWrite` | `title`, `content`, `summary?` | 创建一个新的当前计划。`title` 和 `content` 必填。 |
| `PlanUpdate` | `content`, `title?`, `summary?` | 更新当前计划。`content` 必填。 |
| `PlanFinish` | `note?` | 标记当前计划完成。 |

计划工具适合系统级 thinking skill 使用。业务 role skill 如果不负责通用任务推进，通常不需要直接引用这些工具。

复杂要求或用户明确给出的多步骤要求适合先用 `PlanWrite` 建立计划。active plan 存在时，除非用户明确改变或取消目标，否则 Agent 应把推进该计划作为高优先级任务；实质步骤完成、计划变化或出现阻塞时使用 `PlanUpdate`，全部目标和必要验证完成后才使用 `PlanFinish`。临时旁支和单次工具结果不应隐式替换当前计划。

### 10.4.3 Agent 协作参数

| 系统名 | 参数 | 说明 |
|---|---|---|
| `CreateAgent` | `name`, `class?`, `skills`, `workflow?`, `intent`, `interval?` | 创建临时 agent。当前 `class` 主要支持 `oneshot`；`skills` 必须包含且只能包含一个 role skill。 |
| `AppointAgent` | `name`, `message?` | 指派或切换到已有持久 agent。 |
| `ListAgents` | 无 | 列出当前可见 agent。 |
| `ReportToAgent` | `target`, `report_type`, `reason`, `artifacts?`, `handoff?` | 子 agent 汇报状态。`report_type` 支持 `completed`、`need_help`、`canceled`。 |
| `DismissAgent` | `name` | 销毁指定子 agent。 |

Agent 协作工具建议只放在确实负责调度、委派、导航的 role skill 里。普通业务 skill 暴露这些工具会扩大模型的动作空间。

## 10.5 RAG 引用规则

RAG 有两条路径：

- 默认召回：当前 Agent 的 `retrieval.enabled=true` 时，runtime 在 think 前根据用户消息召回，并把结果写入该 Agent 可读上下文。
- 二次召回：模型在思考后认为需要更精确的 query 时，调用 `RagRetrieve`，结果作为工具结果进入后续上下文。

二次召回工具不作为业务 RPC 工具注册。它是 runtime 内部胶水能力，按当前 Agent 的 `retrieval.endpoint_id` 路由到 resources 注册的专用知识库端点。默认工具名来自 Agent 的 `retrieval.tool_name`，通常是 `RagRetrieve`。

| 系统名 | 参数 | 说明 |
|---|---|---|
| `RagRetrieve` | `query`, `profiles?`, `top_k?`, `score_threshold?` | 按 query 从 runtime 配置的召回后端取回上下文，并返回可写入模型上下文的结果。 |

如果 skill 希望 AI 可以主动二次召回，必须把 `RagRetrieve` 写进 `tools`：

```yaml
---
name: product_qa
kind: feature
tools: ["ProductSearch", "RagRetrieve"]
---
```

如果只需要 think 前默认召回，可以不在 skill 中暴露 `RagRetrieve`。

## 10.6 用户自定义 RPC 工具

用户自定义 RPC 工具需要两步接入。

第一步，在 `resources.json` 的 `rpc_endpoints[]` 注册工具端点：

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "rpc_endpoints": [
    {
      "id": "order-tools-sidecar",
      "endpoint": "127.0.0.1:7001",
      "protocol": "grpc",
      "timeout_ms": 30000
    }
  ]
}
```

第二步，在需要使用该工具的 role 或 feature skill 中引用：

```yaml
---
name: order_admin
kind: role
tools: ["OrderList"]
---
```

只有第一步没有第二步，AI 不会看到 `OrderList`。只有第二步没有第一步，AI 可能看到名称，但执行时会因为系统未注册而失败。

RPC 工具端点可以通过 `ListTools` 暴露多个具体工具。skill 的 `tools` 中应引用具体工具名，而不是工具集端点名。工具实现必须返回非空 `AIOutput.to_ai`，成功和失败都要给出可写入 AI 上下文的摘要。RPC 工具不能更新动态上下文；如果工具改变了后续推理需要的状态，宿主必须通过 `agent_runtime_invoke_v1` 发送 `conversation.set_dynamic_snapshot`，重新发布当前纯文本字段。旧 `snapshot.get` / `snapshot.put` 方法全部不兼容。详细写法见 [`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md)。

## 10.7 高级思考与 Workflow 工具

默认 `thinking` 保持轻量。需要让某类 Agent 使用高级思考规则并执行一次性多行脚本时，
可在 resource Agent profile 或具体 cluster Agent 上完整替换 thinking system Skill：

```json
{
  "systemSkills": { "thinking": "thinking-pro" }
}
```

具体 cluster Agent 的映射覆盖 profile 默认值。`thinking-pro` 不是叠加 feature，而是
`thinking` 状态的替代实现，因此不会同时注入两套思考规范。它只额外声明
`executeWorkflowScript`，用于编译和执行一次性完整脚本，不会写入 Workflow 目录。

持久化 Workflow 目录能力不属于思考状态。宿主必须通过 role 或 feature Skill 的 `tools`
白名单独立开放 `listWorkflows`、`readWorkflow`、`createWorkflowDraft`、`updateWorkflow`、
`compileWorkflow`、`testWorkflow`、`registerWorkflow`、`deleteWorkflow` 和
`executeWorkflow`。内置 `workflow_editor` role 是完整目录能力的标准示例。普通 Agent 和
Workflow Studio 仍共享同一个 `WorkflowsModule`，但权限来源不同。旧 `Wf*`、文件路径型
执行工具和 Studio 专用 Draft CRUD 已移除，不应再写入 Skill。

## 10.8 编写原则

skill 只需要关心 AI 可调用工具，不需要枚举 runtime 内部实现系统。代码里存在的普通 system、内部 glue、prompt 构建、ledger 查询等实现细节，不等于能被 LLM 通过 `tools` 调用。

原则上，进入 `tools` 的名称应该满足三个条件：

1. 它已经按 AI 工具契约注册到 runtime。
2. 它的参数和返回值适合暴露给 LLM。
3. 它的语义稳定、边界清晰，失败时能给出可恢复的错误。

进入 `tools` 的系统会扩大模型可选动作空间，应优先暴露稳定、语义清晰、失败边界明确的工具。
