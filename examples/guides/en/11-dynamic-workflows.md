# 11 Draft, Register, and Execute Dynamic Workflows

Use the Runtime Host SDK when a product must add, update, delete, or execute
workflows after Runtime startup. Workflow catalog mutation is dynamic; the
normal Resources/LLM/Cluster registrations remain frozen.

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

Drafts are untrusted and cannot use the production execution helper. Use
`test_workflow_draft(id, inputs)` for an explicit test run. Resource responses
publish `kind`, `revision`, `trusted`, and `production_executable`; do not infer
state from the display name. Pass `expected_revision` on update, promotion, and
delete when the host wants lost-update protection.

Temporary text can be compiled and executed without registration:

```python
execution = runtime.execute_workflow_script(
    script="input url\n1: BrowserOpenPage --url $url\nreturn page_id=1.page_id",
    inputs={"url": "https://example.com"},
)
```

Use the output names published by the tool descriptor. Do not reference
`step.Result`: the RPC AIOutput envelope is unwrapped before the node is exposed
to the workflow. Put every value required by the host in the final `return`,
then read it from `result.outputs`.

Always inspect `code` before reading `result`. Compilation and execution
failures omit `result` and explain the failure in `trace`. Persist
`workflow.resource_changed` and `workflow.execution_completed` when the host
needs catalog/execution audit. They use the global `event_line=workflow`, not a
conversation aggregate. Cross-Pod locking and authorization remain host
responsibilities.

The normative schemas are in the
[Runtime Workflow Contract](../../../agent_runtime_ffi/docs/en/11-runtime-workflow-execution-contract.md).
