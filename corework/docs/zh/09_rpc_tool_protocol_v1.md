# 9 RPC 工具协议 v1

> 状态：协议收敛稿
> 真源头：`corework/proto/corework_agent_tool_v1.proto`
>
> 破坏性升级：`snapshot.get`、`snapshot.put` 与
> `allowed_snapshot_prefixes` 已删除，不提供旧接口兼容。动态 AI 上下文只能由
> 宿主通过 runtime FFI 主动发布纯文本字段。

## 9.1 目标

Corework RPC 工具协议用于把 Python、Node.js、Go、Rust、C++、C# 等语言实现的业务工具接入 Corework/Agent Runtime。

v1 的边界是：

```text
sidecar process
  -> exposes AgentToolService
  -> reports ToolDescriptor through ListTools
  -> receives Execute requests
  -> may call controlled host capabilities
  -> returns AIOutput
```

业务工具不直接链接 Corework，也不直接访问 Rust `Context`。所有跨边界能力必须通过协议显式声明和调用。

## 9.2 Schema

正式 schema 字符串固定为：

```text
corework-agent-tool/v1
```

`ListToolsResponse.schema` 必须等于该值。Corework 可以在 `ListToolsRequest.accepted_schema` 中发送自己接受的 schema 列表。

## 9.3 服务

```proto
service AgentToolService {
  rpc ListTools(ListToolsRequest) returns (ListToolsResponse);
  rpc Execute(stream ToolStreamMessage) returns (stream ToolStreamMessage);
}
```

`ListTools` 用于自动发现工具。`Execute` 是双向流，用于工具执行和执行期间的 HostCall。

## 9.4 工具描述

每个远程工具由 `ToolDescriptor` 描述，并映射为 Corework 的 `RuntimeToolMetadata`。

工具注册规则：

1. `name` 必须非空，并且在当前 runtime 内全局唯一。
2. `parameters.name` 必须非空，同一工具内不得重复。
3. `outputs.name` 必须非空，同一工具内不得重复。
4. `required_capabilities` 只能声明 v1 支持的能力。
5. 已删除的 snapshot capability 声明必须被拒绝。

## 9.5 错误码

v1 使用 `ToolErrorCode` 作为唯一协议错误码枚举。

```proto
enum ToolErrorCode {
  TOOL_ERROR_CODE_UNSPECIFIED = 0;
  TOOL_ERROR_CODE_OK = 1;
  TOOL_ERROR_CODE_INVALID_ARGUMENT = 100;
  TOOL_ERROR_CODE_MISSING_ARGUMENT = 101;
  TOOL_ERROR_CODE_PERMISSION_DENIED = 102;
  TOOL_ERROR_CODE_NOT_FOUND = 103;
  TOOL_ERROR_CODE_CONFLICT = 104;
  TOOL_ERROR_CODE_INTERNAL = 200;
  TOOL_ERROR_CODE_TIMEOUT = 201;
  TOOL_ERROR_CODE_CANCELLED = 202;
  TOOL_ERROR_CODE_UNAVAILABLE = 203;
  TOOL_ERROR_CODE_HOST_CAPABILITY_DENIED = 300;
  TOOL_ERROR_CODE_HOST_CAPABILITY_UNSUPPORTED = 301;
  TOOL_ERROR_CODE_HOST_CALL_FAILED = 302;
  TOOL_ERROR_CODE_PROTOCOL_ERROR = 400;
  TOOL_ERROR_CODE_INVALID_OUTPUT = 401;
  TOOL_ERROR_CODE_SCHEMA_MISMATCH = 402;
}
```

规则：

1. `TOOL_ERROR_CODE_UNSPECIFIED` 只表示未设置，最终工具结果中应被拒绝。
2. `TOOL_ERROR_CODE_OK` 表示成功。
3. 非 OK 表示失败或部分失败，Corework 应把该调用视为工具失败。
4. `AIOutput.error_code` 表示业务工具执行结果。
5. `ToolError.code` 表示协议级失败，例如 schema 不兼容、输出非法、sidecar 内部异常。
6. JSON-lines debug transport 仍可使用 legacy `0 = success`，但 gRPC/proto v1 必须使用 `TOOL_ERROR_CODE_OK`。

## 9.6 AIOutput

每个成功完成协议流的工具调用必须返回 `AIOutput`。

```proto
message AIOutput {
  string result_json = 1;
  string to_ai = 2;
  ToolErrorCode error_code = 3;
}
```

规则：

1. `to_ai` 必须非空。失败结果也必须提供 AI 可读解释。
2. `result_json` 必须是合法 JSON 文本，可以是 `null`、对象、数组、字符串、数字或布尔值。
3. `error_code` 必须显式设置，不能是 `TOOL_ERROR_CODE_UNSPECIFIED`。
4. `error_code == TOOL_ERROR_CODE_OK` 表示执行成功。
5. `error_code != TOOL_ERROR_CODE_OK` 表示业务工具失败，`to_ai` 应说明失败原因和可修正动作。

## 9.7 Execute 流

Corework 调用工具时发送 `ToolStreamMessage.execute_request`。

执行规则：

1. Corework 发送的首个消息必须是 `execute_request`。
2. `call_id` 在一次工具调用内必须稳定。
3. sidecar 可以发送多条 `host_call` 或 `log`。
4. Corework 必须对每个 `host_call.id` 返回对应 `host_result.id`。
5. sidecar 最终必须返回且只能返回一次 `ai_output` 或 `error`。
6. `ai_output` 和 `error` 之后，双方应结束该调用流。
7. endpoint 级 `timeout_ms` 控制整次调用，包括 HostCall 往返。

## 9.8 HostCall

v1 只支持以下 host capability：

```text
workspace.resolve_path
workspace.resolve_working_path
workspace.create_path
workspace.create_working_path
workspace.save_as_edited
```

capability 声明格式：

```text
workspace.resolve_path
workspace.resolve_working_path
workspace.create_path
workspace.create_working_path
workspace.save_as_edited
```

`workspace.resolve_path` / `workspace.resolve_working_path`:
```json
{"path":"D:/docs/source.pptx"}
```
result:
```json
{"working_path":"...","source_path":"D:/docs/source.pptx"}
```

`workspace.create_path` / `workspace.create_working_path`:
```json
{"path":"D:/docs/output.pptx"}
```
result:
```json
{"working_path":"...","source_path":"D:/docs/output.pptx"}
```

`workspace.save_as_edited`:
```json
{"source_path":"D:/docs/source.pptx","suffix":"_edited"}
```
result:
```json
{"saved_path":"D:/docs/source_edited.pptx"}
```

HostCall 规则：

1. `args_json` 必须是合法 JSON 对象。
2. 工具必须在 `required_capabilities` 显式声明调用的 `workspace.*` 操作。
3. 未知或已删除的 op 必须返回 `TOOL_ERROR_CODE_HOST_CAPABILITY_UNSUPPORTED`。

动态内容更新不经过 `HostCall`。页面、文档或业务对象的最新纯文本状态由宿主
调用 `agent_runtime_set_agent_dynamic_snapshot_field_v1` 发布给对应 agent。

`HostResult.code` 规则：

1. `ok == true` 时，`code` 应为 `TOOL_ERROR_CODE_OK`。
2. `ok == false` 时，`code` 必须是非 OK 错误码。
3. `value_json` 成功时是 JSON 文本，失败时可以是错误文本或结构化错误 JSON。

## 9.9 Corework 注册路径

正式 gRPC 路径应为：

```text
config.rpc_tools endpoint
  -> AgentToolService.ListTools
  -> validate ToolDescriptor
  -> RuntimeToolMetadata
  -> RuntimeToolRegistry
  -> runtime_tools::register_runtime_tool
  -> SystemRegistry::register_dynamic
  -> ACTIVE_TOOLS
```

当前 JSON-lines 传输仅作为 debug/demo transport 保留，不是 v1 的正式生态协议。

## 9.10 SDK 和工具生成器

Corework 可以提供类似 Kitex 的开发体验，但不替代 protobuf/gRPC：

```text
corework_agent_tool_v1.proto
  -> corework-toolgen
  -> language SDK scaffold
  -> @tool / #[tool] business function
  -> sidecar exposes AgentToolService
```

SDK 必须隐藏 stream 细节，但不能绕过协议规则。SDK 生成或收集的 metadata 必须能被 `ListTools` 原样上报。
