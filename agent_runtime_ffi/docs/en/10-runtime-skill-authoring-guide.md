# 10 Runtime Skill Authoring Guide

This document defines the `SKILL.md` authoring contract for runtime skills. It explains role/feature skills, tool visibility, and how skills reference runtime built-ins, RAG, and user-defined RPC tools. For RPC tool-set definitions, `ToolDescriptor`, `AIOutput.to_ai`, and sidecar integration rules, see [`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md).

## 10.1 Skill Types

Runtime uses two core skill types:

| Type | `kind` | Purpose |
|---|---|---|
| role skill | `role` | Defines the agent identity, responsibilities, boundaries, tone, business permissions, and default tools. |
| feature skill | `feature` | Defines an optional capability, workflow, operation rule, or business procedure. |

An agent has exactly one identity, and that identity is read from its role skill. In practice, an agent should bind exactly one role skill. Feature skills are optional capability extensions on top of that role.

### 10.1.1 Role Skill

A role skill answers: who is this agent, what is it responsible for, and what can it do by default?

```yaml
---
name: order_admin
description: "Buyer-facing order and after-sales assistant."
kind: role
tools: ["UserGet", "OrderList", "OrderGet", "ProductSearch", "RagRetrieve"]
---
```

Put these in a role skill:

- Agent identity and responsibility boundaries.
- Default business tools, such as user, order, product, or frontend navigation tools.
- Data isolation, permission, and safety rules.
- Business constraints that should apply to all features.

Do not load multiple role skills for one agent. Multiple roles make identity, responsibilities, and tool permissions ambiguous.

### 10.1.2 Feature Skill

A feature skill answers: what extra capability or workflow is enabled under the current role?

```yaml
---
name: workflow
description: "Shopping and after-sales workflow."
kind: feature
tools: ["OrderReturnEligibilityCheck", "ReturnCreate", "ReturnPlan", "ReturnReview"]
---
```

Put these in a feature skill:

- A business workflow, such as returns, address changes, or product guidance.
- Tool ordering and decision rules for a specific task.
- Temporary capabilities such as web fetch, office document operations, or workflow execution.

Feature skills can be switched dynamically through `UpdateSkills`. After a feature switch, runtime recomputes active tools from the current active skills.

## 10.2 Tool Visibility

The `tools` field in `SKILL.md` is not just documentation. It controls which tools the AI can see.

For a tool to be visible to the LLM, both conditions must be true:

1. The tool is registered in runtime.
2. The tool name is listed in the active role or feature skill `tools` field.

Registering a user-defined RPC endpoint in `resources.json` under `rpc_endpoints[]` is not enough. `rpc_endpoints[]` registers the endpoint with runtime; `SKILL.md.tools` exposes concrete tool names to the current agent. If a tool is not referenced by an active skill, the AI will not see its definition and will not choose it.

```yaml
---
name: order_admin
kind: role
tools: ["UserGet", "OrderList", "OrderGet", "FrontendNavigate"]
---
```

Tool names are system registration names and are case-sensitive. A skill can reference three kinds of tools:

| Source | Example | Description |
|---|---|---|
| runtime built-in AI system | `GetSkillsList`, `UpdateSkills`, `PlanWrite` | Registered by `ai-assistant` / `corework`. |
| runtime internal glue system | `RagRetrieve` | For example explicit second-pass RAG retrieval; it does not belong in `rpc_tools`. |
| user-defined RPC tool | `OrderList`, `UserGet`, `FrontendNavigate` | Endpoint registered by `resources.json` `rpc_endpoints[]`, then concrete tools referenced by skill `tools`. |

## 10.3 Recommended Shape

Role skills usually declare persistent business tools:

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

Feature skills usually declare extra tools for a workflow:

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

The system thinking skill usually declares generic planning and skill-management tools:

```yaml
---
name: thinking
system_layer: true
tools: ["GetSkillsList", "UpdateSkills", "PlanWrite", "PlanUpdate", "PlanFinish"]
tool_filter: "all"
---
```

## 10.4 Runtime Built-Ins

These runtime built-in AI systems are currently recommended for skill use.

| System | Category | Suggested use | Notes |
|---|---|---|---|
| `GetSkillsList` | skill management | Inspect available and active skills. | Read-only. |
| `UpdateSkills` | skill management | Switch active feature skills. | Recomputes active tools. |
| `PlanWrite` | planning | Create the current execution plan. | Useful for multi-step tasks. |
| `PlanUpdate` | planning | Update the current execution plan. | Requires an active plan. |
| `PlanFinish` | planning | Finish the current execution plan. | Clears active plan state. |
| `CreateAgent` | agent collaboration | Create a one-shot child agent for an independent task. | Requires a role skill. |
| `AppointAgent` | agent collaboration | Hand off focus to an existing persistent agent. | For multi-agent routing. |
| `ListAgents` | agent collaboration | List visible agents. | Read-only. |
| `ReportToAgent` | agent collaboration | Let a child agent report to a target agent. | The default agent cannot report to itself. |
| `DismissAgent` | agent collaboration | Destroy a child agent. | Destructive; expose carefully. |
| `RagRetrieve` | RAG | Explicit second-pass retrieval. | Name comes from the current agent's `retrieval.tool_name`, default `RagRetrieve`. |

### 10.4.1 Skill Management Parameters

| System | Params | Description |
|---|---|---|
| `GetSkillsList` | none | Returns available skills and active skills. |
| `UpdateSkills` | `skills` | Replaces the active feature skills. |

`UpdateSkills` has replacement semantics, not append semantics.

### 10.4.2 Planning Parameters

| System | Params | Description |
|---|---|---|
| `PlanWrite` | `title`, `content`, `summary?` | Creates a new current plan. `title` and `content` are required. |
| `PlanUpdate` | `content`, `title?`, `summary?` | Updates the current plan. `content` is required. |
| `PlanFinish` | `note?` | Marks the current plan finished. |

Planning tools are mainly for the system thinking skill. Business role skills usually do not need to reference them directly.

### 10.4.3 Agent Collaboration Parameters

| System | Params | Description |
|---|---|---|
| `CreateAgent` | `name`, `class?`, `skills`, `workflow?`, `intent`, `interval?` | Creates a temporary agent. `class` mainly supports `oneshot`; `skills` must contain exactly one role skill. |
| `AppointAgent` | `name`, `message?` | Appoints or switches to an existing persistent agent. |
| `ListAgents` | none | Lists visible agents. |
| `ReportToAgent` | `target`, `report_type`, `reason`, `artifacts?`, `handoff?` | Child agent report. `report_type` supports `completed`, `need_help`, and `canceled`. |
| `DismissAgent` | `name` | Destroys a child agent. |

Expose agent collaboration tools only in skills that truly perform routing or delegation.

## 10.5 RAG Rules

RAG has two paths:

- Default retrieval: when the current agent has `retrieval.enabled=true`, runtime retrieves before thinking based on the latest user message and injects the result into that agent's context.
- Second-pass retrieval: after thinking, the model may call `RagRetrieve` with a more precise query; the result becomes a tool result in later context.

The second-pass retrieval tool is runtime internal glue rather than a business RPC tool. It routes through the current agent's `retrieval.endpoint_id` to a dedicated knowledge endpoint registered in resources. Its name comes from the agent's `retrieval.tool_name`, usually `RagRetrieve`.

| System | Params | Description |
|---|---|---|
| `RagRetrieve` | `query`, `profiles?`, `top_k?`, `score_threshold?` | Retrieves context from the configured retrieval backend and returns AI-readable content. |

If a skill wants the AI to trigger second-pass retrieval, list `RagRetrieve` in `tools`:

```yaml
---
name: product_qa
kind: feature
tools: ["ProductSearch", "RagRetrieve"]
---
```

If the skill only needs default before-thinking retrieval, it does not need to expose `RagRetrieve`.

## 10.6 User-Defined RPC Tools

User-defined RPC tools need two steps.

First, register the endpoint in `resources.json` under `rpc_endpoints[]`:

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

Second, reference it in the role or feature skill that should use it:

```yaml
---
name: order_admin
kind: role
tools: ["OrderList"]
---
```

With only step one, the AI cannot see `OrderList`. With only step two, the AI may see the name, but execution will fail because the system is not registered.

An RPC tool endpoint may expose multiple concrete tools through `ListTools`. The skill `tools` list should reference concrete tool names, not the tool-set endpoint name. Tool implementations must return non-empty `AIOutput.to_ai`; both success and failure should provide a summary that can be written into AI context. RPC tools cannot update dynamic context: if a tool changes state that later reasoning needs, the host must send `conversation.set_dynamic_snapshot` through `agent_runtime_invoke_v1`. Old `snapshot.get` / `snapshot.put` methods are not compatible. For details, see [`09-runtime-rpc-tool-authoring-guide.md`](./09-runtime-rpc-tool-authoring-guide.md).

## 10.7 Conditional Systems

These systems are not part of the default stable contract. Reference them only when the module is compiled, registered, and explicitly allowed by the product.

| System | Category | Description |
|---|---|---|
| `WfListWorkflows` | workflow | Lists runnable workflows. |
| `WfRunWorkflow` | workflow | Runs a workflow. |
| `WfRunScript` | workflow | Runs workflow script. Confirm safety boundaries before exposing it. |
| `WfReviseWorkflow` | workflow | Modifies a workflow. Expose only if the product allows the LLM to edit flows. |
| `BuildWorkflowFromChainSystem` | workflow | Builds a workflow from a chain. More of an internal builder; not recommended by default. |

If the agent only needs to call configured business RPC tools, workflow systems are not required.

## 10.8 Authoring Principles

Skills only need to care about AI-callable tools. Ordinary systems, internal glue, prompt builders, ledger queries, and other runtime implementation details do not automatically become LLM-callable `tools`.

A name should enter `tools` only when:

1. It is registered with runtime under the AI tool contract.
2. Its parameters and results are suitable for LLM use.
3. Its semantics are stable and errors are recoverable.

Every tool added to `tools` expands the model's action space. Prefer stable tools with clear semantics and failure boundaries.
