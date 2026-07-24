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
| `workflow.execute` | `id`, `mode?`, `inputs?`, `trace?`, `conversation_id?`, `agent_id?` | Execute Registered, or Draft with `mode=test`. |
| `workflow.execute_script` | `script`, `inputs?`, `trace?`, `conversation_id?`, `agent_id?` | Compile and execute temporary text without registration. |

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
    "script": "input url:String\n1: EXEC BrowserOpenPage --url input.url\nreturn page_id=1.page_id url=1.url"
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

Scripts, AI prompts, and Runtime tool/node catalogs exposed to hosts use `num`
as the single public numeric type. The compiler continues accepting historical
numeric type labels and canonicalizes them to `num`, preserving old scripts.
Runtime's concrete internal numeric representation is not part of script or
ABI semantics.

Inside nested `FOR` blocks, `$item` and `$index` always refer to the innermost
loop; entering that loop temporarily shadows the outer bindings. To use an
outer item inside the nested loop, declare a variable first and save the value
with `setvar` in the outer loop. The outer implicit bindings are restored after
the inner loop exits.

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
input url:String
1: EXEC BrowserOpenPage --url input.url
return page_id=1.page_id url=1.url
```

Value syntax determines parameter semantics. Quoted content is a fixed
`String`; an unquoted number is `num`; `true` and `false` are booleans, and a
bool target pin also accepts `1` and `0`; `[...]` is an array; `input.name`,
`$name`, and `N.pin` reference an input, variable, and previous output. Do not
quote references: quoting turns them into fixed strings and removes the data
connection. Arrays may mix literals and dynamic references, for example
`[input.video_path, $backup_path, 1.path]`.

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

Compilation failures include the source line and compiler message, and tell
callers to verify that every referenced tool is present in the current Agent's
active tools with explicit registered description, input pins, and output pins.
Execution trace includes node status, source line, duration, input/result
previews, AI-facing node message, and error details.

## 11.4 Audit and Host Ownership

When a Conversation or Agent initiates execution, the host must provide both
`conversation_id` and `agent_id`. Runtime binds that identity pair to the
individual Workflow execution context and forwards it to local tools and RPC
`ToolContext`. Supplying only one field is an argument error. Non-conversation
background jobs may omit both. The identity is never written to the shared
Workflow module cache, so concurrent executions cannot overwrite each other.
Providing execution identity does not start Conversation permission handling;
approval belongs to the AI Executor, while direct host `workflow.execute` calls
remain direct execution.

The AI tool entry points `executeWorkflow` and `executeWorkflowScript` are both
declared `destructive`. The host's `destructive = ask/deny/full` policy therefore
applies before Workflow execution begins. Development Studio sessions that need
unconfirmed execution must have the host explicitly select `open_all`.

Catalog mutation emits `workflow.resource_changed`; every attempted execution
emits `workflow.execution_completed`. Both use the global
`event_line: "workflow"`, carry `workflow_id` when a catalog resource is
involved, and the event envelope still does not carry `conversation_id`. The
execution identity applies only to tool calls within that run and does not
change the Workflow event line aggregate. They form a Workflow projector
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
context. `listWorkflows`, `readWorkflow`, `createWorkflowDraft`,
`updateWorkflow`, `compileWorkflow`, `testWorkflow`, `registerWorkflow`,
`deleteWorkflow`, `executeWorkflow`, and `executeWorkflowScript` all use that
module. The HTTP canvas endpoints use it too. Studio-only `searchSkillRefs`
searches design references and does not own another resource CRUD path.

The browser canvas is a View. It never owns an independent authoritative draft
and cannot bypass optimistic revision checks by executing an unsaved temporary
blueprint. Valid canvas mutations autosave through the catalog with the
selected `expected_revision`; revision conflicts are surfaced instead of
overwriting external edits. After a global Workflow event, Studio refreshes `workflow.list` and
rereads the affected selected resource. Opening or selecting an existing
resource is conversation/UI context and does not emit a catalog mutation event.
Runtime also projects that global event line into the editor Agent's dynamic
catalog/current-resource snapshots. Studio observes completion of
`createWorkflowDraft` or `readWorkflow` on the editor conversation ledger and
then rereads the selected catalog resource by stable `workflow_id`.

Event publication is best effort after a successful local mutation. A host
that needs durable audit delivery should persist/relay events and reconcile
with `workflow.list`/`workflow.read`; Runtime does not implement an outbox or a
distributed event broker inside the shared library.
