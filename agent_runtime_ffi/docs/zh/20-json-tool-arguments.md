# JSON 工具参数与 RPC 调用

同一个工具支持原生 Function Calling、文本 JSON 和旧 CLI。RPC 工具无需重复注册。会话的 tool_protocol 决定模型使用原生 FC 或正文 EXEC。

正文调用：

```text
EXEC SomeTool {
  "payload": {"sheet":"申请表","items":[1,true,null]},
  "count": 20,
  "label": "001"
}
```

旧 CLI 继续支持 EXEC SomeTool --count 20 --label 001。CLI 保留历史类型转换；JSON 路径严格检查类型，不把字符串 "20" 转成数字。

原生 FC 直接使用模型接口中的工具名、arguments 和 provider call ID；arguments 解码后保留 JSON 类型，不经过 CLI。RPC 服务优先读取 args_json。JSON 调用的兼容字段 args_cli 为空；CLI 调用仍带原命令参数。

参数类型：Json、Any、Value 允许任意 JSON 值，包括 null；Object/Map 只接受对象；Array 或 Array<T> 接受数组，Array<T> 递归检查元素类型；String、Number、Boolean 保持其类型。Json/Any 可以直接承载普通字符串，无需二次 JSON 编码。原生 FC 的严格闭合 Schema 模式仍拒绝未声明内部字段的开放 Json/Object；需要此类参数时使用非严格工具 Schema。

JSON 必须是顶层对象，最大 1 MiB。拒绝重复键（包括嵌套对象）、尾随内容、非法 JSON、未知参数、缺失必需字段和类型不符。字段缺失可采用工具默认值；显式 null 在 Json/Any 中保留。原生 FC 的可空可选标量占位 null 延续原有省略行为。JSON 字符串不展开变量、步骤引用、Pure 表达式、EXEC 或 heredoc。

活动工具与执行权限检查沿用统一链路，JSON 参数不会授予额外权限；会话、Agent、调用身份由 Runtime 提供，不能通过业务参数覆盖。

内置工具沿用 AIInput.parse_args() 可继续读取兼容字符串视图；需要保留类型时使用 AIInput.json_arguments()。运行期 ParsedToolCall.arguments 为规范参数，params 为旧格式兼容投影；旧快照缺少 arguments 仍可读取。当前快照保存结构化参数，原生 FC 恢复从原始 arguments 重建，并保留 provider call ID。

正文 JSON 调用在前端仍显示工具状态组件，参数不会作为普通聊天正文显示；审批使用同一份规范参数。工作流专用的 inputs 键值对投影维持兼容，普通 RPC 的 inputs 参数可使用实际声明类型。
