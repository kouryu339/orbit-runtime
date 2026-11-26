# 11 Runtime Workflow Catalog and Execution Contract

> Workflow resources now expose list, register, remove, validate, promote and execute. Legacy API descriptions below are historical. See [the current contract](12-workflow-resource-api.md).



ABI revision 1.2 exposes a dynamic workflow catalog through
`agent_runtime_invoke_v1`. Resource/LLM/cluster registration remains frozen
after `start`; workflow resources are the intentional mutable exception.

## 11.1 Commands

See the [Workflow resource API](12-workflow-resource-api.md) for public catalog and execution commands. They are available after Runtime startup.

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

## 11.3 Submission and Results

Subscribe to workflow events before calling `workflow.execute` with a Draft or
Registered resource ID. Native hosts continuously read `agent_runtime_next_event_v1`.
Browser hosts use the SDK's `subscribe` method. An accepted submission returns
`{"run_id":"wf-123","workflow_id":"example","status":"accepted"}`.

Events may arrive before the command response. Collect events from subscription
time and correlate them using the returned `run_id`. Different runs can interleave.
Order and deduplicate each run by `payload.sequence`. A rejected submission
returns a command error.

In the terminal `workflow.execution_completed` event, `payload.result` holds the
End/return output object on success. On failure, `payload.error` has the error
code and message. Both include `payload.run_id`, `status`, `code`, and
`duration_ms`. Set `trace:true` to include `payload.node_trace`. Progress events
are always emitted.

## 11.4 Audit and Host Ownership

When a Conversation or Agent initiates execution, the host must provide both
`conversation_id` and `agent_id`, and may provide the current `turn_id`. The local
AI tools `executeWorkflow` and `executeWorkflowScript` automatically extract all
three fields from their calling Agent `Context`. Runtime binds that origin to the
individual Workflow execution context and forwards it to local tools and RPC
`ToolContext`. Supplying only one of conversation/agent is an argument error.
Non-conversation background jobs may omit the Agent-origin fields. The identity is never written to the shared
Workflow module cache, so concurrent executions cannot overwrite each other.
Providing execution identity does not start Conversation permission handling;
approval belongs to the AI Executor, while direct host `workflow.execute` calls
remain direct execution.

The execution engine independently binds `workflow_id`, `workflow_run_id`, and
the current `node_id`. Every execution has a `workflow_run_id`, and every RPC
node has a `node_id`. Only persisted Workflow resources require `workflow_id`;
a directly executed temporary Script may leave it empty. Agent-started runs carry
both origin groups, while direct host runs carry only Workflow execution identity.

The AI tool entry points `executeWorkflow` and `executeWorkflowScript` are both
declared `destructive`. The host's `destructive = ask/deny/full` policy therefore
applies before Workflow execution begins. Development Studio sessions that need
unconfirmed execution must have the host explicitly select `open_all`.

Catalog mutation emits `workflow.resource_changed`. Once a run is established,
it emits the following ordered lifecycle:

1. `workflow.execution_started`
2. `workflow.node_started`
3. `workflow.node_completed` or `workflow.node_failed`
4. `workflow.execution_completed`

Node events are bounded deltas rather than an ever-growing trace. They carry a
stable Blueprint `node_id`, the original `display_name`, an internal
collision-safe `node_name`, a per-invocation
`execution_id`, an optional `parent_execution_id`, `run_id`, a run-local
monotonic `sequence`, node status, selected `output_pin`, duration, source
reference, and bounded input/result previews. The completion event carries the
same `run_id` and terminal `sequence`. Frontends must isolate concurrent runs by
`run_id`, order deltas by `sequence`, and address canvas nodes by `node_id`, not
by the potentially duplicated `node_name`.

For an ordinary tool node, the frontend should render the tool's real `to_ai`
as the default execution-result text. `result_preview` is only a bounded,
structured summary of output-pin values for details and diagnostics; it neither
transfers DAG data nor replaces an available `to_ai`. Pure and control nodes may
fall back to control-flow facts or result previews when no `to_ai` exists.

Control-flow facts use `workflow.trace_event`: `branch.selected`, `loop.started`,
`loop.iteration.started`, `loop.iteration.completed`, optional iteration failure,
and `loop.completed`. Repeated execution of the same static node receives a fresh
`execution_id`; loop-body nodes use the iteration identity as their parent. The
loop node completes only after its actual body execution finishes.

Runtime accepts up to 128 concurrent runs. Hosts own continuous event consumption, persistence, and replay. Runtime shutdown waits for accepted runs to finish.
The End-node result is published in the terminal event with a 16 MiB payload limit.
Exceeding it fails explicitly with `WORKFLOW_RESULT_TOO_LARGE`. For smaller event
payloads, tools should persist large data themselves and return a durable reference;
Runtime must not manufacture a reference to an in-memory result store.

All workflow events use the global `event_line: "workflow"` and carry
`workflow_id` when a catalog resource is
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
