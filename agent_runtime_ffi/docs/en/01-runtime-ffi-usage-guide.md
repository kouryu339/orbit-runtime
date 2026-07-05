# 1 Agent Runtime ABI 1 Host Contract

This is the host contract implemented by `sdk/runtime/c/include/agent_runtime.h`
and `agent_runtime_ffi/src/lib.rs`. ABI 1 exports lifecycle and transport only;
features are versioned JSON commands.

## 1.1 Common Rules

`AgentRuntimeHandle` is `uint64_t`; zero is invalid. Status codes are: `0 OK`,
`1 INVALID_ARGUMENT`, `2 INVALID_HANDLE`, `3 BAD_STATE`, `4 TIMEOUT`,
`5 UNSUPPORTED`, `100 RUNTIME`, and `101 PANIC`.

- Inputs are non-NULL NUL-terminated UTF-8.
- Non-NULL `char**` outputs transfer ownership to the host and must be freed
  exactly once with `agent_runtime_free_string_v1`.
- Direct `const char*` results are borrowed and must not be freed.
- ABI 1 has no callbacks. Events use the pull queue.
- Calls may originate on different threads; facade operations on one handle are
  internally serialized. Do not start/invoke after shutdown begins. Continue
  polling events until the queue closes if the host needs shutdown-time ledger
  deltas or conversation-closed notifications.

## 1.2 Exported Functions

### 1.2.1 `agent_runtime_abi_version_v1()`

Returns ABI major `1`. It is non-blocking and cannot fail.

### 1.2.2 `agent_runtime_version_v1()`

Returns a non-NULL static product-version string valid until library unload.
Product version does not determine ABI compatibility.

### 1.2.3 `agent_runtime_capabilities_v1()`

Returns static `agent-runtime-capabilities/v1` JSON containing ABI minor,
supported commands, pull-event schema, shutdown behavior, and threading model.
Hosts must use it for feature discovery.

### 1.2.4 `agent_runtime_create_v1(create_options_json, out_handle)`

Accepts either an empty string for defaults or inline
`agent-runtime-create-options/v1` JSON and creates an unstarted runtime. Config
file paths are not supported. Create options are limited to process-level
startup parameters such as `log_level`, `language`, `restore_policy`, and
`data_dir`; Agents, tools, LLM providers, Skills, and clusters are registered
through Resources, LLM, and Agent Cluster configuration. Resources and clusters
are required before `start` for a real Agent host. LLM registration may be empty
or deferred, but no LLM turn can complete until a provider and current model are
configured.
`out_handle` is required and is set to zero on failure. A successful handle must
eventually be shut down and then destroyed.

### 1.2.5 `agent_runtime_start_v1(handle)`

Freezes registration and initializes resources, LLMs, RPC/retrieval, and built-in
services. Repeated successful starts are idempotent. Register before starting.

### 1.2.6 `agent_runtime_invoke_v1(handle, request_json, out_response_json)`

Accepts:

```json
{"schema":"agent-runtime-command/v1","id":"host-1","command_id":"optional","type":"conversation.send_message","payload":{}}
```

Payload may be omitted/null or must be an object. Runtime generates `ffi_cmd_N`
when `command_id` is absent. Once dispatch is reached, success and command
failure both attempt to return `agent-runtime-result/v1`; failures have
`ok:false` and an error object while the C status is nonzero. Boundary failures
may have no response. Calls on one handle are serialized. Free every non-NULL
response.

### 1.2.7 `agent_runtime_next_event_v1(handle, timeout_ms, out_event_json)`

Pulls one `agent-runtime-event/v1`. Zero is a non-blocking poll; positive values
wait up to that many milliseconds. No event returns TIMEOUT with a NULL output;
a closed queue returns BAD_STATE. Hosts may keep polling during and after
shutdown until BAD_STATE indicates the queue is closed. Use one reader per
handle unless competing consumers are intentional.

Ledger persistence consumers may listen for:

```json
{
  "schema": "agent-runtime-event/v1",
  "type": "conversation.ledger_delta",
  "conversation_id": "conv-1",
  "conversation_event_seq": 8,
  "payload": {
    "schema": "agent-runtime-ledger-delta/v1",
    "op": "append",
    "record_id": 8,
    "conversation_id": "conv-1",
    "record": {}
  }
}
```

Use `(conversation_id, record_id)` as the idempotency key. Hosts that do not
need runtime-ledger replication can ignore these events.

### 1.2.8 `agent_runtime_shutdown_v1(handle, timeout_ms)`

Transitions the handle to CLOSING immediately, rejects new runtime calls, waits
for in-flight ABI calls, then shuts down conversations, Studios, and event
producers. `next_event_v1` remains available for draining queued shutdown
events. TIMEOUT is retryable and shutdown continues in the background. Only OK
guarantees CLOSED.

### 1.2.9 `agent_runtime_destroy_v1(handle)`

Removes a CLOSED handle with no active calls. It does not implicitly shut down.
After success the numeric handle is invalid.

### 1.2.10 `agent_runtime_last_error_json_v1()`

Returns borrowed thread-local `agent-runtime-error/v1` JSON for the latest
failed ABI call on that host thread, or NULL. Read it immediately on the same
thread. The next ABI call invalidates/clears it. Do not free it.

### 1.2.11 `agent_runtime_free_string_v1(value)`

Frees only ownership-transferred invoke/event strings. NULL is accepted.
Passing metadata, last-error, or foreign pointers is undefined behavior.

## 1.3 Commands

Registration commands accept either `{"input":"path-or-json"}` or
`{"registration":{...}}`.

## 1.4 Pre-Start Registration Configs

`agent_runtime_create_v1` only creates an unstarted Runtime handle. A host that
wants to run Agents registers product resources, optional LLM/provider state,
and the concrete Agent cluster before `agent_runtime_start_v1`:

| Config | Command | Purpose |
|---|---|---|
| Resource registration | `runtime.register_resources` | Registers Skill roots, Agent profiles, tool/RAG endpoints, workflow roots, and data/log roots. |
| LLM/provider registration | `runtime.register_llm` | Optional at first boot. Registers model providers, credentials/base URLs, model ids, context windows, and current model selection. It may be an empty registration for delayed model setup. |
| Agent cluster registration | `runtime.register_agent_cluster` | Creates concrete Agent instances from profiles and declares the initial focus seed. Runtime built-ins own later focus handoff and routing. |

These registrations are part of the host-admin plane. They should happen before
`start`; frontend clients should not be given direct access to them. After
`start`, registration is frozen for the current Runtime lifecycle and hosts
should spawn conversations against registered cluster ids.

Do not put old `agents`, `agent`, `rpc_tools`, `retrieval`, or `workflow`
entries into create options. FFI docs do not provide those old entry points as
supported configuration; the corresponding capabilities must be expressed
through Resources, LLM, and Agent Cluster registrations.

| Type | Payload | Result |
|---|---|---|
| `runtime.register_resources` | `registration` JSON object | `{}` |
| `runtime.register_llm` | `registration` JSON object | `{}` |
| `runtime.reload_llm` | `input` or `registration` JSON/file; accepts `agent-runtime-llm-registration/v1` or provider config | `{}` |
| `runtime.register_agent_cluster` | `registration` JSON object | `{}` |
| `runtime.set_auth_context` | `context`, or whole payload | `{}` |
| `runtime.configure_providers` | `registration` JSON object | `{}` |
| `runtime.get_provider_definitions` | empty | definitions |
| `runtime.set_current_model` | `model_uid:uint32` | `{}` |
| `runtime.set_language` | `language:string` | `{}` |
| `runtime.export_snapshot` | empty | runtime snapshot |
| `conversation.spawn` | `spawn` or expanded spawn fields | conversation info |
| `conversation.spawn_from_snapshot` | `spawn`, `snapshot` | info + restored |
| `conversation.send_message` | `conversation_id`, `content` | admission |
| `conversation.pause` | `conversation_id` | admission |
| `conversation.close` | `conversation_id` | `{}` |
| `conversation.export_snapshot` | `conversation_id`, `options?` | snapshot |
| `conversation.agent_tasks` | `conversation_id` | task-board document |
| `conversation.materialize` | `conversation_id`, `options?` | loaded info |
| `conversation.import_snapshot` | `snapshot`, `options?` | `{}` |
| `conversation.set_dynamic_snapshot` | conversation/agent/field/text | `{}` |
| `conversation.resolve_tool_permission` | `conversation_id`, `tool_call_id`, `decision` (`allow` or `deny`) | `{resolved:boolean}`; false means the request is no longer pending |
| `conversation.set_summary_model` | `conversation_id`, `model_name` | admission |
| `conversation.compact_history` | `conversation_id`, `agent_ids?` | admission + report |
| `studio.open_workflow` | `options?` | Studio open result |
| `studio.open_agent_test` | `options?` | Studio open result |

`conversation.spawn_from_snapshot` consumes a durable
`agent-runtime-conversation-snapshot/v1` for recovery or copying a
conversation. It is not the same semantic operation as continuing from a
tail-of-conversation snapshot exported for observation or UI refresh.

## 1.5 Required Order

```text
abi/version/capabilities -> create
-> register_resources/[register_llm or delayed empty model]/register_agent_cluster
-> start
-> invoke + event pull loop -> shutdown (retry timeout) -> destroy
```
