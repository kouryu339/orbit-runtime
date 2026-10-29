# RPC 工具集动态新增

启动后，宿主通过 `runtime.add_rpc_endpoint` 发现并整体登记一个新的外部 gRPC 服务。
先检查 capabilities 中的 `features.dynamic_rpc_additions == 1`。
此接口属于宿主管理，不自动作为 AI 工具开放。

```json
{
  "schema": "agent-runtime-command/v1",
  "type": "runtime.add_rpc_endpoint",
  "payload": {
    "schema": "agent-runtime-rpc-endpoint-add/v1",
    "endpoint": {
      "id": "project-tools",
      "protocol": "grpc",
      "endpoint": "http://127.0.0.1:55061",
      "timeout_ms": 30000,
      "to_ai_max_chars": 12000
    }
  }
}
```

[请求 Schema](../rpc-endpoint-add.schema.json)。字段沿用资源登记中的 RPC 配置。
第一版只接受外部 gRPC，必须省略 `launch`。不接受内联工具定义、更新标志或未知字段。
`timeout_ms` 为发现的总截止时间，同时成为该端点后续调用的超时；范围 1–60000。
`to_ai_max_chars` 为 1–1000000，默认 600。发现限制为 1–1024 个工具和 4 MiB 元数据。
服务必须实现标准 `AgentToolService.ListTools` 和 `Execute`。

## 新增规则

- 端点 ID 已存在，即使配置完全相同，也返回 `rpc_endpoint_exists`。
- 任一工具与已有动态工具、内置工具或 `WorkflowRef_` 命名空间冲突，整体返回 `rpc_tool_name_conflict`。
- 目录格式错误、内部重名、超时或连接失败时，原有目录保持不变。
- 不提供更新、移除、重新发现或替换；已有契约与审批不变。
- 新增工具不会自动进入 Agent 的活动工具集合。Skill、集群限制和审批照常生效。
- 已有会话的工具说明、模型工具定义、参数校验和 Workflow 编辑读取同一个实例目录。
  新增不会重新编译旧工作流，也不会改变已经发出的模型请求。

发现期间不持有 Runtime 的操作写锁。提交时再次检查冲突和关闭状态。
并发相同 ID 或工具名只有一次提交成功；互不冲突的新增都可提交。
关闭已经开始时拒绝尚未提交的发现结果。

## 返回和事件

成功结果包含 `endpoint_id`、`protocol`、`catalog_revision`、`contract_hash`、
`tool_count`、`tool_names` 和 `persistence: "host_managed"`。
`contract_hash` 是按工具名排序后有效元数据的 SHA-256；不表示网络服务身份。
提交后发出一次 `runtime.rpc_catalog_changed`，载荷包含相同摘要和 `command_id`。
事件、日志和公共查询不包含连接地址、进程参数或环境变量。

继续使用 `runtime.get_rpc_endpoint_definitions`、`runtime.get_tool_definitions`
和 `runtime.get_workflow_node_definitions` 查询有效目录。
前两项返回目录版本；动态端点的 `verification_scope` 为 `add_list_tools`。
`ready` 只表示发现曾成功，不是持续健康检查。
重复请求的冲突名称位于 `error.details.conflicts`；发现失败为 `rpc_failed`，
输入与额外限额问题为 `invalid_argument`；远端目录契约检查也可能返回 `rpc_failed`。
关闭状态为 `bad_state`。

## 保存与 SDK

Runtime 更新当前实例的内存配置，不写宿主配置文件、不监听文件变化。
宿主必须保存成功接收的端点配置；重启时通过资源登记在 start 之前恢复，
使依赖这些工具的 Workflow 源文档能够正常加载。
若成功响应丢失，查询端点和工具目录确认，不用重试来覆盖配置。

Python / Rust：`add_rpc_endpoint(endpoint)`；C++：`add_rpc_endpoint_json(endpoint_json)`；
Go：`AddRPCEndpoint(ctx, endpoint)`。SDK 参数均为 `endpoint` 对象，SDK 包装请求 Schema。
C 直接使用现有 `agent_runtime_invoke_v1`，没有新增导出符号。

RPC 目录按实例隔离。全局 Workflow 资源所有权仍存在原有约束，
本改动不承诺在同一进程同时启动多个完整 Runtime。
