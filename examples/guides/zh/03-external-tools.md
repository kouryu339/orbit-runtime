# 3 接入外部 Tools

外部 Tool 是独立进程中的业务能力。Runtime 连接 Tool sidecar，发现工具描述，
执行调用；Skill 决定哪些工具对某个 Agent 可见。

```text
Agent -> Runtime -> Agent Tool RPC -> Tool sidecar -> Business service
```

理解外部 Tool 时，要区分三个上下文阶段：

1. 调用前：Runtime 把当前 Skill 和允许使用的工具契约放进 Agent 上下文。
2. 执行时：Runtime 把本次调用的 `ToolContext` 交给 sidecar。
3. 执行后：sidecar 返回 `AIOutput`，其中 `to_ai` 进入 Agent 的工具结果上下文。

## 3.1 Agent 调用前能看见什么

对于当前 active Skill 白名单中的每个工具，Runtime 会向 Agent 暴露：

- 工具 `name` 和 `description`；
- 调用语法；
- 参数名称、类型、是否必填、默认值和描述；
- 输出字段名称、类型和描述；
- `readonly`、`destructive`、`idempotent`、`open_world` 等行为属性。

Agent 不会看到所有已注册工具。Skill frontmatter 中的 `tools` 是白名单：只有
被当前角色或能力 Skill 引用的工具，才会进入这个 Agent 的工具上下文，也只有
这些工具能通过执行层校验。endpoint 注册只是让 Runtime 知道工具存在，不代表
任何 Agent 都能调用它。

## 3.2 选择 SDK

Agent Tool SDK 位于：

```text
sdk/rpctools/python
sdk/rpctools/node
sdk/rpctools/go
sdk/rpctools/rust
sdk/rpctools/csharp
sdk/rpctools/cpp
```

优先使用该语言 SDK 的 `register tool + serve` API，不要在业务项目里自行实现
Runtime 内部协议。各 SDK 当前支持程度以其 README 为准。

## 3.3 定义工具

Python 示例：

```python
from corework_agent_tool import AIOutput, ToolErrorCode, register_tool, serve

@register_tool(
    name="OrderGet",
    description="读取当前用户的一笔订单。",
    parameters=[
        {
            "name": "order_id",
            "param_type": "String",
            "required": True,
            "description": "订单编号。",
        }
    ],
    readonly=True,
    destructive=False,
    idempotent=True,
    open_world=False,
    secret=False,
)
def order_get(ctx, order_id: str) -> AIOutput:
    order = load_order(ctx, order_id)
    return AIOutput(
        result=order,
        to_ai=f"""已找到订单 {order['id']}。

| 字段 | 值 |
|---|---|
| 状态 | {order['status']} |
| 金额 | {order['amount']} |
| 可申请售后 | {order['returnable']} |

如果用户确认申请售后，下一步调用 `ReturnCreate`，传入本订单号和明确的售后原因。""",
        error_code=ToolErrorCode.OK,
    )

serve(host="127.0.0.1", port=50051)
```

工具描述应准确声明参数、副作用、幂等性和所需 capability。用户、会话、Agent
等隔离信息从 `ToolContext` 读取，不要让 AI 或用户伪造这些参数。

## 3.4 ToolContext 中有什么

Runtime 执行工具时，会通过 SDK 向 handler 提供本次调用的上下文：

| 字段 | 用途 |
|---|---|
| `call_id` / `tool_call_id` | 关联本次工具调用、事件和日志。 |
| `idempotency_key` | 对写操作做幂等保护。 |
| `session_id` / `conversation_id` | 定位当前会话作用域。 |
| `agent_id` / `turn_id` | 定位发起调用的 Agent 和轮次。 |
| `provider_id` | 标识当前 Tool endpoint/provider。 |
| `cluster_id` / `runtime_instance_id` | 标识运行集群与 Runtime 实例；部分适配器当前可能为空。 |
| `permissions` | Runtime 为该工具授予的 capability。 |
| `host_context` | 宿主发布的结构化业务上下文。 |

这些字段属于可信执行上下文，适合做租户隔离、当前用户解析、审计和幂等控制。
不要把 `user_id`、`conversation_id`、权限或租户标识重新设计成 AI 可填写的工具
参数。只有真正需要用户表达的业务输入，才应出现在 parameters 中。

如果工具需要访问宿主管理的文件路径，应使用 SDK 的 `workspace.*` HostCall，并在
descriptor 中声明相应 capability；不要绕过宿主自行猜测路径映射。

## 3.5 AIOutput：`result` 与 `to_ai`

每个 handler 必须返回 `AIOutput`：

```text
result      结构化执行结果，供 Runtime、宿主、日志或程序化处理
to_ai       非空文本，作为本次函数调用结果写回 Agent 上下文
error_code  成功或失败状态
```

**Agent 后续推理直接读取的是 `to_ai`，不要假设 `result` JSON 会自动完整进入模型
上下文。** 因此我们推荐把 `to_ai` 写全：它应包含 Agent 完成当前任务或决定下一步
所需的事实，而不只是“查询成功”。

好的 `to_ai` 通常包含：

- 执行了什么，以及成功、失败或部分成功；
- 查到的关键记录、精确 ID、状态、数量和时间；
- Agent 回答用户必须知道的字段；
- 数据较多时使用 Markdown 表格或紧凑列表；
- 是否截断、分页，以及还有多少数据；
- 失败原因、是否可重试、缺少什么输入；
- 存在强顺序流程时，明确下一步可调用哪个工具以及需要哪些参数。

例如查询订单后可以返回：

```markdown
共找到 2 笔待发货订单：

| order_id | 商品 | 金额 | 状态 |
|---|---|---:|---|
| O-1024 | 山茶油礼盒 | 298.00 | paid |
| O-1025 | 家庭装山茶油 | 168.00 | processing |

结果未截断。若用户要查看某笔订单详情，下一步调用 `OrderGet`，并传入对应
`order_id`。
```

不要把整库原始 JSON、内部堆栈、密钥或无关字段塞进 `to_ai`。对大结果先概括，
再列出完成任务所需的数据。如果业务数据中含有用户生成文本，应把它明确当作
数据引用，避免把其中的指令当成对 Agent 的新要求。

### 3.5.1 强顺序工具调用

`to_ai` 可以指导 Agent 继续调用后续工具，例如：

```text
资格检查已通过。下一步可以调用 `ReturnCreate` 创建售后单，参数使用
order_id=O-1024，并补充用户确认的 reason。
```

但这种文字只是在告诉 Agent 下一步怎么做，并不会绕过权限。必须确保
`ReturnCreate` 也出现在当前 active Skill 的 `tools` 中，否则 Agent 看不到它，
执行层也会拒绝调用。对于固定工作流，前置工具和所有可能的后续工具应由同一个
role/feature Skill 一并引用，并在 Skill 正文中写清调用条件和终止条件。

错误返回同样要写可行动的 `to_ai`：说明失败阶段、原因、哪些输入仍然有效，以及
应该重试、改参数、询问用户还是停止。不要只返回“失败”或把内部异常原样暴露。

## 3.6 注册 endpoint

在 resource registration 中注册连接信息：

```json
{
  "rpc_endpoints": [
    {
      "id": "order-tools",
      "protocol": "grpc",
      "endpoint": "127.0.0.1:50051",
      "timeout_ms": 30000
    }
  ]
}
```

`grpc` sidecar 通过 `AgentToolService.ListTools` 返回工具描述，不要在 resources
里重复声明工具元数据。`json-lines` 可用于当前仍采用该适配器的 SDK 或服务。

如果由 Runtime 管理本地 sidecar 生命周期，可以为 endpoint 添加 `launch`；
生产环境也可以由容器、systemd 或编排平台独立管理进程。

## 3.7 暴露给 Agent

注册 endpoint 后，还要在角色或能力 Skill 中引用工具：

```yaml
tools: ["OrderList", "OrderGet", "ReturnCreate"]
```

工具可调用需要同时满足：

1. endpoint 已注册且可连接；
2. 工具描述已被 Runtime 注册；
3. 当前 Agent 的 active Skill 引用了该工具。

这是一条强制白名单，不是使用建议。即使工具已经通过 RPC 注册，如果没有被当前
Skill 引用，Agent 不会获得其工具描述，直接构造调用也会被 Runtime 拒绝。

SDK 入口见 [`sdk/README.md`](../../../sdk/README.md)，完整协议和 capability 规则见
[`09-runtime-rpc-tool-authoring-guide.md`](../../../agent_runtime_ffi/docs/zh/09-runtime-rpc-tool-authoring-guide.md)。
