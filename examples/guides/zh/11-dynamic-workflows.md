# 11 创建草稿、注册与执行 Workflow

产品需要在 Runtime 启动后增加、更新、删除或执行 workflow 时，使用 Runtime Host
SDK 的动态 workflow API。Workflow 目录允许动态修改；普通 Resources、LLM、Cluster
注册仍然保持冻结。

```python
draft = runtime.create_workflow_draft({
    "schema": "agent-runtime-workflow-resource/v1",
    "id": "open-page",
    "name": "Open page",
    "script": "input url\n1: BrowserOpenPage --url $url\nreturn page_id=1.page_id url=1.url",
})

runtime.compile_workflow_draft(draft["id"])
registered = runtime.register_workflow_draft(
    draft["id"], expected_revision=draft["revision"]
)

execution = runtime.execute_workflow(
    workflow_id=registered["id"],
    inputs={"url": "https://example.com"},
    trace=True,
)
page_id = execution["result"]["outputs"]["page_id"]
```

Draft 不可信，不能走生产执行入口；测试草稿使用
`test_workflow_draft(id, inputs)`。资源响应明确提供 `kind`、`revision`、`trusted` 和
`production_executable`，不要从显示名称推断状态。需要防止旧编辑覆盖新版本时，在
更新、注册和删除时传 `expected_revision`。

临时脚本文本可以不注册，直接编译执行：

```python
execution = runtime.execute_workflow_script(
    script="input url\n1: BrowserOpenPage --url $url\nreturn page_id=1.page_id",
    inputs={"url": "https://example.com"},
)
```

脚本必须使用工具 descriptor 发布的输出名，不要引用 `step.Result`；RPC AIOutput
envelope 在节点暴露给 workflow 前已经展开。宿主需要的每个值都要放进最终 `return`，
然后从 `result.outputs` 读取。

读取 `result` 前必须先检查 `code`。编译或执行失败时没有 `result`，错误原因写在
`trace`。宿主需要目录与执行审计时，持久化 `workflow.resource_changed` 和
`workflow.execution_completed`。它们位于全局 `event_line=workflow`，不属于任何
Conversation 聚合根。跨 Pod 锁与鉴权仍由宿主负责。

正式 schema 见
[Runtime Workflow 契约](../../../agent_runtime_ffi/docs/zh/11-runtime-workflow-execution-contract.md)。
