# 11 Runtime Workflow 目录与执行契约

ABI revision 1.2 通过 `agent_runtime_invoke_v1` 提供动态 workflow 目录。
Resources、LLM 和 cluster 注册在 `start` 后仍然冻结；workflow resource 是明确允许
动态增删改的独立目录。

## 11.1 命令

| 命令 | Payload | Result |
|---|---|---|
| `workflow.create` | `resource` | 创建不可信的 Draft。 |
| `workflow.read` | `id` | 读取 Draft 或 Registered。 |
| `workflow.register` | `id`, `expected_revision?`, `name?` | 将有效 Draft 提升为 Registered。 |
| `workflow.update` | `resource`, `expected_revision?` | 更新已有资源。 |
| `workflow.compile` | `id` | 返回 Draft 校验结果与 blueprint。 |
| `workflow.delete` | `id`, `expected_revision?` | 删除任一种资源。 |
| `workflow.list` | `kind?` | 返回全部资源，或按 `draft`/`registered` 过滤。 |
| `workflow.convert.script_to_blueprint` | `script` | 编译为 Blueprint JSON，不修改目录。 |
| `workflow.convert.blueprint_to_script` | `blueprint` | 校验并反编译，不修改目录。 |
| `workflow.execute` | `id`, `mode?`, `inputs?`, `trace?`, `conversation_id?`, `agent_id?` | 执行 Registered；Draft 仅允许 `mode=test`。 |
| `workflow.execute_script` | `script`, `inputs?`, `trace?`, `conversation_id?`, `agent_id?` | 不注册，直接编译并执行临时脚本文本。 |

目录命令要求 Runtime 已经 `start`。只有 Draft 可以创建；无效脚本可以作为草稿保存，
但编译成功前不能提升为 Registered。提升时保持稳定 id。Draft 与 Registered 的 name
全局唯一。Resource 使用 `agent-runtime-workflow-resource/v1`：

```json
{
  "resource": {
    "schema": "agent-runtime-workflow-resource/v1",
    "id": "open-page",
    "name": "Open page",
    "description": "Open a URL and return the browser page identity",
    "script": "input url:String\n1: EXEC BrowserOpenPage --url input.url\nreturn page_id=1.page_id url=1.url"
  }
}
```

资源视图明确返回 `kind`、`revision`、`trusted` 与 `production_executable`，调用方不能
从显示名称推断状态。`expected_revision` 是可选的目录内防覆盖机制，不承担跨进程协调。
创建和更新必须在 `script` 与 `blueprint` 中恰好传入一种源表示；有效资源读取时会返回
两种表示。Script 是供 AI 与代码审阅使用的紧凑语义形式；Blueprint JSON 是可执行图，
同时保存位置、尺寸和布局。Script 重新编译时按节点 id 或源 step 迁移已有布局。两个
转换命令是无状态函数，不创建资源、不增加 revision，也不发出 Workflow 事件。

Script、AI 提示词以及 Runtime 向宿主公开的工具/节点能力表统一使用 `num` 表示数字。
编译器继续接受历史数字类型名称并在编译阶段规范化为 `num`，因此旧脚本保持兼容；
Runtime 内部采用的具体数字表示不属于脚本或 ABI 语义。

嵌套 `FOR` 中的 `$item`/`$index` 始终指向当前最内层循环，内层会暂时遮蔽外层绑定。
若内层需要外层项，脚本必须提前声明变量，并在外层循环中通过 `setvar` 保存后再读取；
离开内层循环后，外层隐式绑定恢复。

Draft 存在于 Runtime state；Registered 持久化到配置的 workflow root。Draft 的跨重启
恢复由宿主根据事件或资源视图自行持久化与重建。
Registered 执行不传 mode；Draft 只有显式 `mode=test` 才可测试。语言 SDK 将二者分别
暴露为 `execute_workflow` 和 `test_workflow_draft`。

## 11.2 工具输出投影

Workflow 节点引脚以工具注册时声明的输入/输出 schema 为准。RPC 工具声明
`page_id` 和 `url` 时，workflow 能看到的输出就只有 `page_id` 与 `url`：

```text
input url:String
1: EXEC BrowserOpenPage --url input.url
return page_id=1.page_id url=1.url
```

参数值由写法决定：引号内容是固定 `String`；裸数字是 `num`；`true`、`false`
以及 bool 目标引脚上的 `1`、`0` 是布尔值；`[...]` 是数组；`input.name`、
`$name`、`N.pin` 分别引用输入、变量和前序输出。引用不得加引号，否则只会得到
固定字符串并失去数据连线。数组允许混合固定项和动态引用，例如
`[input.video_path, $backup_path, 1.path]`。

RPC transport 可以返回包含 `error_code`、`to_ai` 和 `result` 的 AI output envelope。
该 envelope 是传输/执行结果，不是 workflow 引脚 schema。Runtime 会展开
`AIOutput.result`，校验每个已注册输出字段，然后把这些字段作为节点输出。因此外部
RPC 节点不会暴露虚构的 `Result` 引脚，脚本也不应写成 `1.Result.page_id`。

如果工具没有注册任何输出字段，该节点就没有数据输出引脚。此时引用任意字段都应在
编译阶段报告“节点不存在该输出”，不能用 transport 层的 `Result` 误导调用者。

## 11.3 程序执行结果

两个执行命令成功时都返回：

```json
{
  "code": 0,
  "trace": "Workflow execution trace:\n- line 2 step 1 succeeded: ...",
  "result": {
    "outputs": {
      "page_id": "page-1",
      "url": "https://example.com"
    },
    "duration_ms": 12
  }
}
```

`result.outputs` 是 workflow End/`return` 节点的输出 map。内部节点输出不会自动进入
程序结果；宿主需要的每个值都必须在脚本最终 `return` 中接到 End。请求
`trace=true` 时，`result.node_trace` 还会包含结构化逐节点 trace。

失败时始终返回 `code` 与文本 `trace`，并省略 `result`：

| Code | 含义 |
|---|---|
| `400` | 请求非法、脚本编译失败或 workflow 建立失败。 |
| `404` | 找不到已注册 workflow selector。 |
| `-1` | 已经开始执行，但节点或 workflow 执行失败。 |

编译失败信息必须包含源码行号和编译器信息，并提示调用者确认所引用工具已经出现在当前
Agent 的 active tools 中，且注册元数据具有明确的描述、输入引脚和输出引脚。执行 trace
包含逐节点状态、源码行、耗时、输入/结果预览、节点给 AI 的消息和错误详情。

## 11.4 审计与宿主职责

由 Conversation 或 Agent 发起执行时，宿主必须同时传入 `conversation_id` 与 `agent_id`。
Runtime 将这对身份绑定到本次独立 Workflow 执行上下文，并透传给本地工具与 RPC
`ToolContext`；只传其中一个会返回参数错误。非会话型后台任务可以同时省略二者。该上下文
不得写入 Workflow 模块共享缓存，以免并发执行串用调用者身份。传入执行身份不会启动
Conversation 审批；审批只属于 AI Executor，宿主直接调用 `workflow.execute` 仍是直接执行。

AI 工具入口 `executeWorkflow` 和 `executeWorkflowScript` 都声明为 `destructive`。因此宿主
配置的 `destructive = ask/deny/full` 会在进入工作流执行前生效；开发期 Studio 若需要
无确认执行，必须由宿主显式选择 `open_all`。

目录变更产生 `workflow.resource_changed`；每次执行尝试产生
`workflow.execution_completed`。二者位于全局 `event_line: "workflow"`，涉及目录资源时
携带 `workflow_id`，事件信封仍不携带 `conversation_id`，也不进入 Conversation
ledger/state。执行身份只用于本次工具调用上下文，不改变 Workflow 全局事件线的聚合根。
宿主如需把 workflow 注入 Agent 尾部快照，应订阅该事件线、读取资源，再显式更新对应
Conversation 的动态快照。它们是公共审计事件，不是分布式协调协议。
Runtime 只串行化同一个 ABI handle 上的调用；跨进程鉴权、持久化、幂等、锁和多 Pod
协调仍由宿主负责。

## 11.5 Workflow Studio 与内部 AI

Workflow Studio、内部 Workflow Editor Agent 和 ABI 调用方共享同一个
`WorkflowsModule` 实例。Editor conversation 只保存选择上下文
（`workflow_id`、`revision`），Prompt 中的动态快照也是从目录资源投影出来的只读视图。
`listWorkflows`、`readWorkflow`、`createWorkflowDraft`、`updateWorkflow`、
`compileWorkflow`、`testWorkflow`、`registerWorkflow`、`deleteWorkflow`、
`executeWorkflow` 与 `executeWorkflowScript`，以及画布 HTTP 接口，都使用同一个模块。
Studio 额外提供的 `searchSkillRefs` 只负责搜索设计参考，不拥有另一套资源 CRUD。

浏览器画布只是 View，不持有另一份权威草稿，也不能绕过 revision 检查直接执行未保存的
临时 blueprint。画布中的有效变更会带当前 `expected_revision` 自动保存到目录；若外部
编辑已推进 revision，则明确返回冲突，不覆盖外部修改。收到全局 Workflow 事件后，Studio 重新读取目录，并按稳定
`workflow_id` 刷新当前资源。仅打开或选择已有资源属于 UI/conversation 上下文，不产生
目录变更事件。
Runtime 还会监听这条全局事件线，将目录和当前资源投影到 Editor Agent 的动态尾部快照。
Studio 会在 editor conversation ledger 中观察 `createWorkflowDraft` 或 `readWorkflow`
完成记录，再按稳定 `workflow_id` 重新读取当前选择的目录资源。

事件在本地状态变更成功后尽力发布。需要持久审计交付的宿主应自行持久化或转发事件，并用
`workflow.list`/`workflow.read` 做状态对账；共享库内部不实现事务 outbox 或分布式事件代理。
