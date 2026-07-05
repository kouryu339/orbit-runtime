# 4 编写 Skill

Skill 是 Agent 的角色和工作方法，不是工具实现。接入项目通常需要：

- 一个 `role` Skill：定义身份、职责、边界和默认工具。
- 若干 `feature` Skill：定义可选能力、具体流程和附加工具。

## 4.1 目录结构

```text
skills/
  role/
    order_admin/
      SKILL.md
  feature/
    after_sales/
      SKILL.md
```

资源注册通过 `skills.root_dir` 指向这个目录：

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "id": "commerce-resources",
  "skills": {
    "root_dir": "../skills",
    "builtin_system": true
  }
}
```

## 4.2 SKILL.md

```markdown
---
name: order_admin
description: "处理订单查询和售后服务。"
kind: role
tools: ["OrderList", "OrderGet", "ReturnCreate"]
---

# 订单服务角色

你负责查询当前用户的订单并处理明确的售后请求。

## 工作规则

- 用户不知道订单号时，先调用 `OrderList`。
- 修改业务状态前确认目标和影响。
- 不猜测订单、用户或退款状态。
```

Frontmatter 的核心字段：

| 字段 | 作用 |
|---|---|
| `name` | Skill 的稳定引用名，应与目录名一致。 |
| `description` | 说明何时使用该 Skill。 |
| `kind` | `role` 或 `feature`。 |
| `tools` | 允许该 Skill 暴露给 AI 的已注册工具名。 |

`tools` 只控制可见性，不会创建工具。工具必须由 runtime 内置系统或外部
Tool sidecar 注册。工具名区分大小写。

这里的可见性是 Runtime 强制执行的白名单，不只是给模型看的建议：

- Skill 未引用的工具不会进入该 Agent 的工具描述上下文；
- 即使工具已经注册，Agent 直接构造调用也会被执行层拒绝；
- 一个工具通过 `to_ai` 建议调用后续工具时，后续工具也必须在 active Skill 中；
- 有强顺序关系的一组工具，应由同一个 role/feature Skill 一并引用，并在正文中
  说明调用顺序、前置条件和终止条件。

## 4.3 将 Skill 绑定到 Agent

先在 resources 中注册可复用 Agent profile：

```json
{
  "agents": {
    "profiles": [
      {
        "id": "commerce.order_admin",
        "name": "Order Admin",
        "role": "order_admin",
        "features": ["after_sales"]
      }
    ]
  }
}
```

再由 agent cluster 创建具体实例：

```json
{
  "schema": "agent-runtime-agent-cluster-registration/v1",
  "id": "commerce-service",
  "focus_agent_id": "commerce.order_admin",
  "agents": [
    {
      "id": "order-agent-1",
      "profile": "commerce.order_admin"
    }
  ]
}
```

Profile 表示 Agent 类型；`agents[].id` 表示这次注册的具体实例。同一种
Profile 可以创建多个实例。需要委托、报告或后台 Agent 能力时，把相应工具
写进角色 Skill，由角色显式决定何时使用。

完整格式参考
[`10-runtime-skill-authoring-guide.md`](../../../agent_runtime_ffi/docs/zh/10-runtime-skill-authoring-guide.md)。
