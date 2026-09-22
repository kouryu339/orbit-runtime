# 12 Runtime Jev 执行契约

Jev 是基于有限 JSON 状态和预声明原子动作的轻量选择器。Agent 只调用
`RunJev(jevname, task)`；Jev 服务根据当前状态从注册动作中选择下一步，Runtime 仍用现有
动态工具注册表执行具体工具。

## 12.1 生命周期和归属

`JevManager` 负责定义注册表和运行实例。每次运行创建独立的 `ExecutionUnit::Jev`、
`jev_run_id`、快照和单调递增的 `snapshot_revision`。带 AI 归属时，
`conversation_id` 与 `agent_id` 必须同时提供；两者都省略表示宿主运行。
`parent_run_id` 用于表达 Workflow/Jev 嵌套关系。

Runtime 发布以下可审计事件：

- `jev.execution_started`
- `jev.decision`
- `jev.tool_started`
- `jev.tool_completed`
- `jev.snapshot_updated`
- `jev.execution_completed`

即使请求、响应解析或工具执行发生异常，也会发送失败的 `jev.execution_completed`。

## 12.2 定义和状态迭代

```json
{
  "name": "text_matcher",
  "description": "生成并验证匹配规则",
  "model": "jev-latest",
  "initial_state": { "regex": null, "last_execution": null },
  "instructions": "缺少 regex 时生成；验证成功后完成；无法继续时 blocked。",
  "actions": {
    "generate": {
      "description": "生成 regex",
      "tool": "GenerateRegex",
      "arguments": { "requirement": { "source": "task" } }
    },
    "finish": {
      "description": "任务已经完成",
      "terminal_status": "completed"
    },
    "blocked": {
      "description": "权限或输入不足",
      "terminal_status": "blocked"
    }
  },
  "max_steps": 8
}
```

参数绑定仅允许三种来源：初始 `task`、快照 JSON Pointer、定义中的字面量。Jev 不生成任意
工具参数结构或代码；开放文本、正则等值应由专门的原子工具生成，再通过快照更新反馈给
Jev。

工具返回值可以包含：

```json
{
  "result": {
    "value": "ORD-[0-9]{6}",
    "jev_snapshot_update": {
      "set": { "regex": "ORD-[0-9]{6}", "last_execution": { "status": "ok" } },
      "remove": ["previous_error"]
    }
  },
  "to_ai": "regex generated",
  "error_code": 0
}
```

Runtime 取出 `jev_snapshot_update` 后按顶层字段执行 `set/remove`，提交新快照并增加 revision；
该控制字段不会作为普通工具结果继续暴露。

## 12.3 工具边界和权限

工具目录使用三个独立可见性字段：

- `agent_enabled`：普通 Agent 可直接调用。
- `workflow_enabled`：Workflow 可调用。
- `jev_enabled`：Jev 可调用。

宏中的 `jev_only` 会设置 `agent_enabled=false`、`workflow_enabled=false`、
`jev_enabled=true`。RPC SDK 也可直接声明这三个字段。

宿主 `jev.run` 使用宿主直连执行语义。Agent 的 `RunJev` 在每一步选中工具后，仍按当前
conversation 的 `PermissionBroker` 对工具 effect 执行 `Full / Ask / Deny`。权限拒绝会写入
当前 Jev 快照的 `last_execution`，供后续动作选择 blocked 或替代路径；它不会绕过既有 AI
工具策略。

## 12.4 FFI 调用顺序

```text
create
-> jev.configure
-> jev.register (可重复注册不同 name)
-> runtime.start
-> jev.list / jev.run / Agent RunJev
-> 从统一事件拉取通道记录 Jev 事件
```

HTTP 决策请求超时为 30 秒；连接失败、超时、HTTP 429 和 5xx 最多重试三次。API key 只保留
在 Runtime 内存配置中，不写入 Jev 事件或运行快照。
