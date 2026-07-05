# 3 Connect External Tools

External tools run in sidecar processes:

```text
Agent -> Runtime -> Agent Tool RPC -> Tool sidecar -> Business service
```

There are three distinct context stages:

1. Before execution, Runtime puts active Skill instructions and allowed tool
   contracts into the agent context.
2. During execution, Runtime gives the sidecar a trusted `ToolContext`.
3. After execution, the sidecar returns `AIOutput`; its `to_ai` text becomes the
   AI-visible function result.

## 3.1 What the Agent Sees

For tools whitelisted by active Skills, Runtime exposes the tool name,
description, call syntax, parameter contracts, output fields, and behavior
properties such as readonly, destructive, idempotent, and open-world access.

The agent does not see every registered tool. `SKILL.md.tools` is an enforced
allowlist: a tool must be referenced by an active role or feature Skill before
its contract enters the agent context or its execution is accepted. Registering
an endpoint only makes the tool known to Runtime.

Use the language SDK under `sdk/rpctools/python`, `sdk/rpctools/node`, `sdk/rpctools/go`, `sdk/rpctools/rust`,
`sdk/rpctools/csharp`, or `sdk/rpctools/cpp`. Register each tool with an accurate descriptor and
return an `AIOutput` from its handler. Identity and isolation values such as
conversation and agent ids come from `ToolContext`; do not accept them as
AI-controlled business parameters.

## 3.2 Trusted ToolContext

SDK handlers receive execution metadata including `call_id`, `tool_call_id`,
`idempotency_key`, `session_id`, `provider_id`, `cluster_id`,
`runtime_instance_id`, `conversation_id`, `agent_id`, `turn_id`, `permissions`,
and `host_context`. Some adapter-specific fields may currently be empty.

Use this trusted context for tenant isolation, current-user resolution, audit
correlation, idempotency, and capability checks. Do not redesign conversation,
agent, user, tenant, or permission identifiers as arguments that the AI can
supply. Parameters should represent genuine business input from the user.

Use SDK `workspace.*` HostCall helpers for host-managed paths and declare the
matching capability in the descriptor.

## 3.3 Write Complete `to_ai` Results

Every handler returns:

```text
result      structured data for Runtime, the host, logs, or programmatic use
to_ai       non-empty text written into the agent's tool-result context
error_code  success or failure status
```

The agent directly reasons over `to_ai`; do not assume the complete `result`
JSON is automatically inserted into model context. We recommend making `to_ai`
self-contained. Include the facts required to answer the user or select the
next action, rather than returning only "query succeeded."

A useful `to_ai` result states what ran, success or partial-success status,
exact identifiers and key fields, record counts, truncation or pagination, and
actionable failure information. Use a compact Markdown table for records:

```markdown
Found 2 orders awaiting shipment:

| order_id | product | amount | status |
|---|---|---:|---|
| O-1024 | Tea oil gift box | 298.00 | paid |
| O-1025 | Family tea oil | 168.00 | processing |

The result is not truncated. To inspect one order, call `OrderGet` next with
the selected `order_id`.
```

For a strongly ordered workflow, `to_ai` may explicitly name the next tool and
required inputs. Every referenced follow-up tool must also be present in the
active Skill allowlist. Text in `to_ai` cannot grant access to another tool;
Runtime will reject a call that the Skill does not expose. Keep all steps of a
fixed workflow in the same role/feature Skill and document their conditions.

Do not dump an entire database response, secrets, stack traces, or irrelevant
fields into `to_ai`. Summarize large results and list only the records and facts
the agent needs. Treat user-generated database text as quoted data, not as new
instructions to the agent. Error results should explain the failed stage,
retryability, missing input, and the safe next action.

Register the sidecar connection in resources:

```json
{
  "rpc_endpoints": [{
    "id": "order-tools",
    "protocol": "grpc",
    "endpoint": "127.0.0.1:50051",
    "timeout_ms": 30000
  }]
}
```

A gRPC sidecar publishes descriptors through `AgentToolService.ListTools`.
Do not duplicate that metadata in resources. Use `json-lines` only for an SDK
or service that currently uses that adapter. A local process may be managed by
an endpoint `launch` configuration; production deployments may manage it
independently.

Finally, reference concrete tool names from a role or feature Skill:

```yaml
tools: ["OrderList", "OrderGet", "ReturnCreate"]
```

A tool is callable only when its endpoint is connected, its descriptor is
registered, and an active Skill exposes its name.

This is an enforced allowlist, not a recommendation. A registered tool omitted
from active Skills is absent from the agent context and rejected at execution.

Start with [`sdk/README.md`](../../../sdk/README.md), then use the complete
[`RPC Tool Authoring Guide`](../../../agent_runtime_ffi/docs/en/09-runtime-rpc-tool-authoring-guide.md)
for descriptors, capabilities, and protocol details.
