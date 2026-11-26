# 11 Runtime Workflow 目录与执行契约

> 当前 Workflow 对外接口保留列表、注册、移除、验证、转正、执行。本文其余旧接口和示例为历史资料。参见 [当前六接口契约](12-workflow-resource-api.md)。



ABI revision 1.2 通过 `agent_runtime_invoke_v1` 提供动态 workflow 目录。
Resources、LLM 和 cluster 注册在 `start` 后仍然冻结；workflow resource 是明确允许
动态增删改的独立目录。

## 11.1 命令

公开命令见 [工作流资源 API](12-workflow-resource-api.md)。Runtime 启动后可使用。

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

## 11.3 执行提交与结果

宿主先监听工作流事件，再用 `workflow.execute` 提交 Draft 或 Registered 资源。
DLL 宿主持续调用 `agent_runtime_next_event_v1`，浏览器宿主使用 SDK 的 `subscribe`。
提交成功返回 `{"run_id":"wf-123","workflow_id":"example","status":"accepted"}`。

事件可能先于提交响应到达。宿主应从订阅时开始保留事件，再用响应里的
`run_id` 归集。不同运行可交错，单次运行按 `payload.sequence` 排序和去重。
提交失败时直接返回命令错误。

终态事件为 `workflow.execution_completed`。成功时 `payload.result` 是 End/return
的输出对象，失败时 `payload.error` 包含错误码和消息。两种终态均包含
`payload.run_id`、`status`、`code`、`duration_ms`。传入 `trace:true`
可在终态附带 `payload.node_trace`。进度事件始终可用。

## 11.4 审计与宿主职责

由 Conversation 或 Agent 发起执行时，宿主必须同时传入 `conversation_id` 与 `agent_id`，
并可传入当前 `turn_id`。AI 本地工具 `executeWorkflow` 和 `executeWorkflowScript` 会自动从
调用它们的 Agent `Context` 提取这三个字段。Runtime 将该来源身份绑定到本次独立 Workflow
执行上下文，并透传给其中的本地工具与 RPC `ToolContext`；只传 conversation/agent 中一个
会返回参数错误。非会话型后台任务可以同时省略 Agent 来源字段。该上下文
不得写入 Workflow 模块共享缓存，以免并发执行串用调用者身份。传入执行身份不会启动
Conversation 审批；审批只属于 AI Executor，宿主直接调用 `workflow.execute` 仍是直接执行。

执行引擎独立绑定 `workflow_id`、`workflow_run_id` 与当前 `node_id`。其中每次执行都有
`workflow_run_id`，RPC 节点都有 `node_id`；只有持久化 Workflow 资源要求 `workflow_id`，
直接执行的临时 Script 允许该字段为空。Agent 启动 Workflow 时，Agent 来源字段与 Workflow
执行字段同时存在；宿主直接执行 Workflow 时只有 Workflow 执行字段。

AI 工具入口 `executeWorkflow` 和 `executeWorkflowScript` 都声明为 `destructive`。因此宿主
配置的 `destructive = ask/deny/full` 会在进入工作流执行前生效；开发期 Studio 若需要
无确认执行，必须由宿主显式选择 `open_all`。

目录变更产生 `workflow.resource_changed`。一次已经建立的执行按顺序产生：

1. `workflow.execution_started`
2. `workflow.node_started`
3. `workflow.node_completed` 或 `workflow.node_failed`
4. `workflow.execution_completed`

节点事件是增量而不是不断增长的完整 Trace，携带稳定 Blueprint `node_id`、原始
`display_name`、内部防重名 `node_name`、每次实际调用唯一的 `execution_id`、可选 `parent_execution_id`、`run_id`、运行内单调递增的 `sequence`、节点状态、实际 `output_pin`、耗时、
源码引用和有界输入/结果预览。`workflow.execution_completed` 携带相同 `run_id` 及终态
`sequence`。前端应以 `run_id` 隔离并发运行、以 `sequence` 排序，并以 `node_id` 关联画布；
不要使用可重复的 `node_name` 作为画布主键。

对普通工具节点，前端应把真实 `to_ai` 作为默认的“执行结果”正文；`result_preview` 只是
输出引脚值的有界结构化摘要，供详情和诊断使用，不参与 DAG 传值，也不能在有 `to_ai` 时
取代它。纯节点和控制节点没有 `to_ai` 时，前端再使用分支、循环事实或结果摘要组织展示。

统一 Trace payload 使用 `agent-runtime-workflow-trace/v1`。节点事件的 `trace_type` 为
`node.started`、`node.completed` 或 `node.failed`；IF 产生 `branch.selected`；循环产生
`loop.started`、`loop.iteration.started`、`loop.iteration.completed`、可选失败事件及
`loop.completed`。循环内同一 `node_id` 每次执行都有新的 `execution_id`，循环体节点以
迭代身份作为 `parent_execution_id`。循环节点只有在实际循环体结束后才完成。

Runtime 最多同时接受 128 个运行。宿主负责持续收取、持久化和补读事件。关闭 Runtime 会等待已接受的运行完成。
End 节点最终结果通过终态事件发布，单终态事件的 payload 上限 16 MiB；超过上限会以
`WORKFLOW_RESULT_TOO_LARGE` 明确失败。宿主需要更小的事件载荷时，应由工具先把大数据保存到
自己的存储并让工作流返回引用，而不能生成指向 Runtime 内存的伪 `result_ref`。

输入元数据包含 `name`、`display_name`、`type`、`required`、`default`、`description`、
`sensitive`、`element_type` 和 `format`。旧 Blueprint 缺少扩展字段时仍可读取：默认显示名
取参数名，是否必填由是否存在默认值推导。`format=file` 是宿主 UI/传输提示，不自动扩大
Runtime 基础数据类型，也不自动授予读取宿主文件系统的能力。

这些事件都位于全局 `event_line: "workflow"`，涉及目录资源时
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
