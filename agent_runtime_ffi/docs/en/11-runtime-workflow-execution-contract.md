# 11 Runtime Workflow Catalog and Execution Contract

ABI revision 1.2 exposes a dynamic workflow catalog through
`agent_runtime_invoke_v1`. Resource/LLM/cluster registration remains frozen
after `start`; workflow resources are the intentional mutable exception.

## 11.1 Commands

| Command | Payload | Result |
|---|---|---|
| `workflow.create` | `resource` | Create an untrusted Draft. |
| `workflow.read` | `id` | Read a Draft or Registered resource. |
| `workflow.register` | `id`, `expected_revision?`, `name?` | Promote a valid Draft to Registered. |
| `workflow.update` | `resource`, `expected_revision?` | Update an existing resource. |
| `workflow.compile` | `id` | Return Draft validation and blueprint. |
| `workflow.delete` | `id`, `expected_revision?` | Delete either kind. |
| `workflow.list` | `kind?` | `{ "workflows": [...] }`; kind is `draft` or `registered`. |
| `workflow.convert.script_to_blueprint` | `script` | Compile to Blueprint JSON without changing the catalog. |
| `workflow.convert.blueprint_to_script` | `blueprint` | Validate and decompile without changing the catalog. |
| `workflow.execute` | `id`, `mode?`, `inputs?`, `trace?` | Execute Registered, or Draft with `mode=test`. |
| `workflow.execute_script` | `script`, `inputs?`, `trace?` | Compile and execute temporary text without registration. |

Catalog commands require a started Runtime. Only Draft can be created. A Draft
may preserve invalid script for editing, but it cannot be promoted until
compilation succeeds. Promotion preserves the stable id. Names are unique
across Draft and Registered. A resource uses
`agent-runtime-workflow-resource/v1`:

```json
{
  "resource": {
    "schema": "agent-runtime-workflow-resource/v1",
    "id": "open-page",
    "name": "Open page",
    "description": "Open a URL and return the browser page identity",
    "script": "input url\n1: BrowserOpenPage --url $url\nreturn page_id=1.page_id url=1.url"
  }
}
```

Every view exposes `kind`, `revision`, `trusted`, and `production_executable`.
`expected_revision` is optional optimistic concurrency for lost-update
protection inside one catalog; cross-process coordination remains host-owned.
Create and update accept exactly one source representation, `script` or
`blueprint`. A valid read exposes both. Script is the compact semantic form for
AI and source review; Blueprint JSON is the executable graph and owns visual
layout. Recompiling a script migrates existing position, size, display, and
layout metadata by node id or source step. The two conversion commands are
stateless: they do not create resources, advance revisions, or emit events.

Drafts live in Runtime state; Registered workflows are persisted in the
configured workflow root. Durable Draft recovery is host-owned.
`inputs` must be an object. Registered execution omits mode. Draft execution is
only allowed with `mode=test`; language SDKs expose that as
`test_workflow_draft`.

## 11.2 Tool Output Projection

Workflow node pins come from the registered tool input/output schema. For an
RPC tool that declares `page_id` and `url`, the workflow-visible outputs are
exactly `page_id` and `url`:

```text
1: BrowserOpenPage --url $url
return page_id=1.page_id url=1.url
```

The RPC transport may return an AI output envelope containing `error_code`,
`to_ai`, and `result`. That envelope is a transport/execution result, not a
workflow pin schema. Runtime unwraps `AIOutput.result`, validates every
registered output field, and publishes those fields as node outputs. External
RPC nodes therefore do not expose a synthetic `Result` pin and scripts must not
write `1.Result.page_id`.

If a tool has no registered output fields, its node has no data output pins.
Referencing any field then fails compilation as a missing node output, rather
than suggesting a transport-level `Result` pin.

## 11.3 Program Result

On success both execution commands return:

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

`result.outputs` is the output map of the workflow End/`return` node. Internal
node outputs are not automatically copied into the program result; scripts
must connect every value required by the host in the final `return` statement.
When request `trace=true`, `result.node_trace` additionally contains the
structured per-node trace.

Failure responses always contain `code` and text `trace`, and omit `result`:

| Code | Meaning |
|---|---|
| `400` | Invalid request, script compilation failure, or workflow setup failure. |
| `404` | Registered workflow selector was not found. |
| `-1` | Execution started but a node or workflow failed. |

Compilation failures include the source line and compiler message in `trace`.
Execution trace includes node status, source line, duration, input/result
previews, AI-facing node message, and error details.

## 11.4 Audit and Host Ownership

Catalog mutation emits `workflow.resource_changed`; every attempted execution
emits `workflow.execution_completed`. Both use the global
`event_line: "workflow"`, carry `workflow_id` when a catalog resource is
involved, and never carry `conversation_id`. They form a Workflow projector
event line parallel to, not inside, Conversation state. A host may subscribe,
read the changed resource, and explicitly project it into a conversation's
dynamic snapshot when needed. These are public audit events, not a distributed
coordination protocol. Runtime serializes calls on one ABI handle.
Cross-process authorization, persistence, idempotency, locking, and multi-Pod
coordination remain host responsibilities.

## 11.5 Workflow Studio and Internal AI

Workflow Studio, its internal Workflow Editor Agent, and ABI callers share one
`WorkflowsModule` instance. The editor conversation stores only a selection
(`workflow_id`, `revision`) and receives catalog-derived snapshots for prompt
context. `openWorkflowDraft`, `readWorkflow`, `updateCurrentWorkflowDraft`,
`registerCurrentWorkflowDraft`, `compileWorkflowScript`, and `testWorkflow`
all read or mutate that module. The HTTP canvas endpoints use the same module.

The browser canvas is a View. It never owns an independent authoritative draft
and cannot bypass optimistic revision checks by executing an unsaved temporary
blueprint. Valid canvas mutations autosave through the catalog with the
selected `expected_revision`; revision conflicts are surfaced instead of
overwriting external edits. After a global Workflow event, Studio refreshes `workflow.list` and
rereads the affected selected resource. Opening or selecting an existing
resource is conversation/UI context and does not emit a catalog mutation event.
Runtime also projects that global event line into the editor Agent's dynamic
catalog/current-resource snapshots. Studio observes completion of `openWorkflowDraft` on the editor conversation
ledger and then rereads the selected catalog resource.

Event publication is best effort after a successful local mutation. A host
that needs durable audit delivery should persist/relay events and reconcile
with `workflow.list`/`workflow.read`; Runtime does not implement an outbox or a
distributed event broker inside the shared library.
