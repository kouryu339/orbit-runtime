# 9 Runtime RPC 工具集定义指南

> **破坏性升级：** RPC 工具不再读写动态上下文快照。`snapshot.get`、
> `snapshot.put`、`allowed_snapshot_prefixes` 及对应 SDK helper 均已删除。
> 页面或文档等动态状态必须由宿主通过 `agent_runtime_invoke_v1` 的
> `conversation.set_dynamic_snapshot` command 面向对应
> `(conversation_id, agent_id)` 发布纯文本字段。
>
> **旧接口不支持：** 旧 endpoint 配置会被拒绝，旧
> `required_capabilities` 声明无效，旧 HostCall 操作会失败；sidecar
> 必须使用不含 snapshot helper 的 SDK 重新构建。

本文描述用户自定义 RPC 工具集的定义、注册和返回值契约。它补充 [`10-runtime-skill-authoring-guide.md`](./10-runtime-skill-authoring-guide.md)：skill 文档说明“哪些工具对 AI 可见”，本文说明“一个 RPC 工具端点应该怎样暴露工具”。

底层稳定协议来自：

```text
corework/proto/corework_agent_tool_v1.proto
```

推荐优先使用 `sdk/` 下的语言 SDK 编写 sidecar，而不是手写 gRPC 细节。当前 Rust、Python、Node.js、C# 已包含可运行的 sidecar；Go、C++ 保留了作者 API，gRPC 接线仍需补齐。

## 9.1 接入路径

一个 RPC 工具要被 agent 正常调用，需要三层同时完成：

1. 工具 sidecar 通过 `AgentToolService.ListTools` 暴露工具描述。
2. `resources.json` 的 `rpc_endpoints[]` 注册该 sidecar 端点，并由宿主通过
   `runtime.register_resources` 在 start 前提交。
3. 需要使用该工具的 role 或 feature skill 在 `tools` 中引用工具名。

`rpc_endpoints[]` 只负责把端点接进 runtime；`SKILL.md.tools` 才决定当前 agent 是否能看到这些工具。对 `protocol = "grpc"` 的端点，工具元数据必须来自 `ListTools`，不要在 runtime config 里重复内联工具定义。

## 9.2 边界：RagRetrieve 不是业务 RPC 工具

`RagRetrieve` 属于 runtime 内置的 local glue 能力，用于 before-thinking 检索注入。知识库连接由 resources 注册，检索策略由 Agent profile 或 `cluster.agents[]` 配置。它不是用户自定义业务 RPC 工具；只有需要模型主动二次召回时，才把它写进对应 role/feature skill 的 `tools`。

不要因为 `RagRetrieve` 使用了一个 json-lines 检索链接，就把业务 RPC 工具也做成手写 json-lines。两者边界必须分清：

| 类型 | 归属 | 配置/暴露方式 |
|---|---|---|
| `RagRetrieve` | runtime 内置 local glue / 检索链接 | 由当前 Agent 的 `retrieval` 配置驱动，通过 resources 中的专用检索端点连接知识库或向量服务。 |
| 用户自定义业务 RPC 工具 | 外部 sidecar | 必须通过官方 SDK 暴露 `AgentToolService.ListTools` / `Execute`，工具声明来自 SDK 注册，skill 中引用具体工具名。 |

正式业务工具端点应使用 `protocol = "grpc"`。不要用手写 json-lines sidecar 承载正式业务 RPC 工具。

## 9.3 工具集端点

一个 sidecar 端点可以暴露多个工具。建议按业务域组织工具集，例如：

- `pptx-buns`：PPTX 读取、创建、编辑、保存。
- `excel-buns`：Excel 读取、写入、编辑、工作表管理、批量操作。

工具集内的名称应稳定、语义清晰、大小写固定。不要把临时函数名、内部 helper 或不准备给 LLM 直接调用的能力暴露为工具。

## 9.4 ToolDescriptor 规范

每个工具必须通过 `ToolDescriptor` 描述自己：

| 字段 | 要求 |
|---|---|
| `name` | 稳定注册名，大小写敏感，也是 skill `tools` 中引用的名称。 |
| `description` | 给 LLM 看的用途描述，应说明工具能做什么、何时使用。 |
| `parameters` | 参数名、类型、是否必填、默认值和说明。参数名要与 handler 实际读取一致。 |
| `outputs` | 可选，但建议为结构化结果声明关键字段。 |
| `readonly` | 只读查询工具设为 `true`。 |
| `destructive` | 删除、覆盖、不可逆修改设为 `true`。 |
| `idempotent` | 重复执行结果相同或可安全重试时设为 `true`。 |
| `open_world` | 会访问开放外部世界或不受控资源时设为 `true`。 |
| `secret` | 涉及秘密值或敏感凭据时设为 `true`，默认不要暴露给普通 skill。 |
| `category` / `display_name` | 用于 UI 和调试归类，建议填工具集名和可读名称。 |
| `required_capabilities` | 需要 host 能力时显式声明，例如 `workspace.resolve_path`。动态上下文不是 RPC capability。 |

### 9.4.1 参数约定

参数优先保持简单、可序列化、可从 CLI 或 JSON 表达：

- 请先进行参数检讨，这个参数确定需要ai控制吗？如果确定需要，如何调整参数要求让ai幻觉率下降，正确率上升
- 字符串、数字、布尔值直接用普通参数。
- 必填参数缺失时返回可恢复错误，不要 panic。
- 单选类的字符串enum需要在parameters列举所有可能的枚举及其作用，多选配置不建议要求使用VEC<string>要求ai返回多个枚举中的字符串，尽量使用多个bool要求的参数的形式。
- 路径类参数不要直接操作原始路径，优先通过 `workspace.*` HostCall 解析或创建工作路径。

## 9.5 AIOutput 返回契约

工具执行最终必须返回 `AIOutput`：

```text
result_json + to_ai + error_code
```

其中 `to_ai` 是必填字段，必须非空。Runtime 会把它写回 AI 可见的工具结果通道；SDK 也会拒绝空 `to_ai`。

| 字段 | 说明 |
|---|---|
| `result_json` | 给程序或 UI 消费的结构化 JSON。成功时尽量放机器可读结果；失败时可以是 `null` 或结构化错误。 |
| `to_ai` | 给 LLM 继续推理的可读摘要。无论成功还是失败都必须提供。 |
| `error_code` | `TOOL_ERROR_CODE_OK` 表示成功；失败要使用合适的非 OK 错误码。 |

`to_ai` 不应只是“ok”或“error”。它应该告诉模型：

- 执行是否成功；
- 关键结果是什么；
- 如果失败，原因是什么；
- 模型下一步可以如何恢复，例如补参数、换路径、先读取结构、缩小范围。

## 9.6 成功和失败都要 to_ai

推荐像 `pptx-buns`、`excel-buns` 一样，把成功和失败统一封装：

```rust
fn ok(result: Value, to_ai: impl Into<String>) -> AIOutput {
    AIOutput { result, to_ai: to_ai.into(), error_code: ToolErrorCode::Ok }
}

fn err(to_ai: impl Into<String>) -> AIOutput {
    AIOutput { result: Value::Null, to_ai: to_ai.into(), error_code: ToolErrorCode::Internal }
}
```

业务执行失败时也返回 `AIOutput`，并把错误写进 `to_ai`：

```rust
if !r.success {
    return Ok(err(format!("[失败] {}", r.description)));
}

Ok(ok(
    serde_json::json!({ "path": saved_path }),
    format!("[成功] {}", r.description),
))
```

这样下一轮 thinking 能看到工具失败的事实和原因，而不是只收到一个运行时异常。只有协议错误、未知工具、sidecar 崩溃、handler panic 等无法形成业务结果的情况，才应走 gRPC `ToolError` 或 SDK 的异常路径。

## 9.7 HostCall 能力

RPC 工具需要访问宿主管理资源时，应通过协议内的 `HostCall`，并在 `required_capabilities` 中声明。

当前 v1 官方能力包括：

| 能力 | 用途 |
|---|---|
| `workspace.resolve_path` | 把用户传入路径解析为 host 管理的工作路径。 |
| `workspace.resolve_working_path` | 解析工作区路径。 |
| `workspace.create_path` | 为新文件创建 host 管理路径。 |
| `workspace.create_working_path` | 创建工作区路径。 |
| `workspace.save_as_edited` | 将编辑后的工作文件另存为用户可见文件。 |

文档处理类工具建议使用 `workspace.*`，避免 sidecar 直接改写用户原文件。`snapshot.get:*` 和 `snapshot.put:*` 已不再是受支持的 v1 capability，不得写入工具描述。

## 9.8 工具执行后的动态状态

RPC sidecar 不能直接更新 AI 可见的动态上下文。某个 agent 的动态上下文由宿主按 `(conversation_id, agent_id, field_name)` 管理。

当工具执行改变了后续推理依赖的信息时，采用以下流程：

1. 工具通过 `AIOutput.result_json` 和 `AIOutput.to_ai` 返回结构化标识与简洁变更摘要。
2. 宿主观测该业务变更，读取或渲染最新状态，并通过 `conversation.set_dynamic_snapshot` command 发布每一个当前纯文本字段。
3. 下一次进入 thinking 时，runtime 会为该 agent 注入全部当前字段。字段名只用于更新定位，不用于对模型隐藏字段内容。

例如，文档编辑工具返回工作路径后，宿主可以生成当前文档预览并发布：

```json
{"schema":"agent-runtime-command/v1","type":"conversation.set_dynamic_snapshot","payload":{"conversation_id":"conv-1","agent_id":"editor-1","field_name":"document_preview","text":"..."}}
```

推荐的工具 `to_ai` 写法类似：

```text
[成功] 已在受管理工作路径中更新文档。后续推理依赖文档内容前，宿主应刷新当前文档预览字段。
```

动态字段不会随 agent cache snapshot 持久化。会话恢复后，宿主必须重新发布当前字段，才能继续依赖这些信息。

## 9.9 执行上下文

Runtime 调用 RPC 工具时，会在 `ExecuteRequest` 中带上当前调用上下文。各语言 SDK 应把这些字段暴露到工具 handler 的 `ToolContext`：

| 协议字段 | 含义 |
|---|---|
| `call_id` | 本次 RPC 调用 ID。 |
| `tool_call_id` | LLM function/tool call ID，用于和对话历史中的工具调用配对。 |
| `idempotency_key` | 幂等键，适合写入类工具做去重。 |
| `session_id` | 当前 runtime/session 标识。 |
| `provider_id` | 工具 provider/endpoint 标识。 |
| `cluster_id` | 当前 runtime cluster。 |
| `runtime_instance_id` | 当前 runtime 实例。 |
| `conversation_id` | 当前 conversation ID。业务工具需要按会话隔离、查路由或写审计时优先使用它。 |
| `agent_id` | 当前发起工具调用的 agent ID。多 agent 场景中用于区分来源。 |
| `turn_id` | 当前 turn ID。 |
| `permissions` | runtime 传入的权限列表。 |
| `host_context_json` | 宿主透传的扩展上下文 JSON。SDK 可以解析成语言原生对象。 |

字段命名按语言习惯暴露，例如 Python/Rust/C++ 使用 `conversation_id`，Go/C# 使用 `ConversationID` / `ConversationId`，Node.js 使用 `conversationId`。工具不应自己从全局状态猜 conversation 或 agent，优先读取 `ToolContext`。

## 9.10 注册到 runtime

在 `resources.json` 的 `rpc_endpoints[]` 中注册 sidecar 端点，并在 start 前调用
`runtime.register_resources`。当前端点字段如下：

| 字段 | 说明 |
|---|---|
| `id` | 端点唯一 ID，不能为空，通常用工具集名加 `-sidecar`。 |
| `endpoint` | sidecar 监听地址，例如 `127.0.0.1:50104`。 |
| `protocol` | 当前正式工具生态建议使用 `grpc`。 |
| `timeout_ms` | 单次工具调用超时，包含 HostCall 往返。 |
| `launch` | 可选。由 runtime 启动 sidecar 进程时填写。`kind` 支持 `external` / `process`。 |

业务 RPC 工具不要把工具声明内联在 runtime config 里。正式写法是：resources 只注册 gRPC endpoint，工具列表由 sidecar 的 `ListTools` 返回；role/feature skill 再按工具名选择可见能力。`RagRetrieve` 的检索配置不适用这条业务工具声明规则。

`allowed_snapshot_prefixes` 已从 endpoint 配置删除。仍包含该字段的配置无效。

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "rpc_endpoints": [
    {
      "id": "pptx-buns-sidecar",
      "endpoint": "127.0.0.1:50104",
      "protocol": "grpc",
      "timeout_ms": 60000,
      "launch": {
        "kind": "process",
        "program": "cargo",
        "args": [
          "run",
          "--quiet",
          "--manifest-path",
          "agent-tools/pptx-buns/Cargo.toml",
          "--bin",
          "pptx-buns-sidecar"
        ],
        "working_dir": "../..",
        "env": {
          "PPTX_BUNS_ADDR": "127.0.0.1:50104",
          "RUST_LOG": "info"
        },
        "startup_timeout_ms": 120000,
        "shutdown_timeout_ms": 3000
      }
    }
  ]
}
```

如果一个端点通过 `ListTools` 暴露多个工具，skill 中引用的是具体工具名，而不是 endpoint `id`：

```yaml
---
name: office_operator
kind: feature
tools: [
  "DescribePresentation",
  "CreateBlankPresentation",
  "PptxFindAndReplace",
  "DescribeWorkbook",
  "ReadTable",
  "SetCell"
]
---
```

## 9.11 推荐实践

- 每个工具只做一件清晰的事，组合流程写在 skill 指导里，不写进单个万能工具。
- 读取类工具默认 `readonly=true`、`idempotent=true`。
- 删除、覆盖、批量清空等工具必须 `destructive=true`，并在描述里写清楚影响范围。
- 参数错误、文件不存在、JSON 解析失败、业务校验失败，都返回带 `to_ai` 的失败 `AIOutput`。
- 成功结果的 `result_json` 面向程序，`to_ai` 面向下一轮模型推理，两者不要互相替代。
- 大结果不要全部塞进 `to_ai`；给摘要、数量、关键字段和下一步建议。结构化工具结果放入 `result_json`；需要向 AI 暴露的变化文本由宿主通过 FFI 发布。
- 对会因工具调用频繁变化、且后续推理依赖最新状态的信息，应在工具改变源状态后由宿主刷新对应动态文本字段。
- 工具描述和参数描述要面向 LLM 写，不要只写内部实现名。
- 不要把 host 凭据、内部路径、隐私数据放进 `to_ai`，除非该 agent 明确有权限看到。
- sidecar 启动时应能被 `ListTools` 探测；端点不可用时 runtime 应能给出明确注册或调用失败。
- 不要把 runtime 内置 glue（例如 `RagRetrieve`）误判为业务 RPC 工具；也不要把业务工具退回到手写 json-lines。业务工具始终使用官方 SDK 声明、注册和返回 `AIOutput`。

## 9.12 最小 Rust SDK 形状

```rust
use corework_agent_tool::{AIOutput, ToolContext, ToolDescriptor, ToolErrorCode};
use serde_json::Value;

fn ok(result: Value, to_ai: impl Into<String>) -> AIOutput {
    AIOutput { result, to_ai: to_ai.into(), error_code: ToolErrorCode::Ok }
}

pub fn register_all_tools() {
    let descriptor = ToolDescriptor::builder("OrderList")
        .description("查询当前用户订单列表。")
        .parameter("user_id", "String", true, None, "用户 ID。")
        .output("orders", "Array", "订单列表。")
        .readonly(true)
        .idempotent(true)
        .category("order")
        .display_name("OrderList")
        .build();

    corework_agent_tool::register_tool(
        descriptor,
        |_ctx: ToolContext, args: Value| async move {
            let user_id = args.get("user_id").and_then(Value::as_str).unwrap_or("");
            if user_id.is_empty() {
                return Ok(AIOutput {
                    result: Value::Null,
                    to_ai: "[失败] 缺少 user_id，请先确认用户身份。".to_string(),
                    error_code: ToolErrorCode::MissingArgument,
                });
            }

            Ok(ok(
                serde_json::json!({ "orders": [] }),
                "[成功] 查询到 0 个订单。"
            ))
        },
    );
}
```

## 9.13 和 SSE 事件的关系

RPC 工具的 `AIOutput.to_ai` 会进入 AI 工具结果通道。前端通过 runtime event stream 看到的是工具开始、成功、失败等 ledger 事件；这些事件的展示契约见 [`05-runtime-event-format.md`](./05-runtime-event-format.md)。

前端不要直接依赖 sidecar 的内部日志判断工具是否成功，应以 runtime 产生的 `tool_call_finished` / `tool_call_failed` 事件和对应 `record.content`、`metadata` 为准。

客户端 agent 项目、测试平台或外层 conversation gateway 也不要根据 `agents[].status` 反推当前是否能继续发送用户消息、是否能暂停或是否能压缩。`saying`、`suspended`、`thinking`、`executing` 是 runtime 内部状态视图，只适合展示、诊断和日志关联；交互能力必须消费 runtime SSE `frontend:state_snapshot` 中的能力位：

```text
payload.conversation_state
```

如果工具执行改变了后续推理依赖的业务对象状态，例如售后工单、订单售后状态、照片/证据存档、文档内容或页面结构，工具应在 `to_ai` 中清楚报告变化，宿主应随后通过 FFI 发布最新动态文本字段。客户端项目不要把业务工具返回、sidecar 日志或 agent 状态当作 runtime 一轮对话完成的依据；一轮必须先进入非 `waiting`，再由最新 `frontend:state_snapshot` 回到 `conversation_state=waiting`，且工具调用全部收敛。
