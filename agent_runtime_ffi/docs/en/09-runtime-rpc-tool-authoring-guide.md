# 9 Runtime RPC Tool-Set Authoring Guide

> **Breaking update:** RPC tools no longer read or write dynamic context
> snapshots. `snapshot.get`, `snapshot.put`, `allowed_snapshot_prefixes`, and
> matching SDK helpers have been removed. The host publishes changing page or
> document state through the `conversation.set_dynamic_snapshot` command sent
> with `agent_runtime_invoke_v1`.
>
> **Old interfaces are unsupported:** old endpoint config is rejected, old
> `required_capabilities` declarations are invalid, and old HostCall operations
> fail. Sidecars must be rebuilt against an SDK without snapshot helpers.

This document describes the definition, registration, and return-value contract for user-defined RPC tool sets. It complements [`10-runtime-skill-authoring-guide.md`](./10-runtime-skill-authoring-guide.md): the skill document explains "which tools are visible to the AI"; this document explains "how an RPC tool endpoint should expose tools."

The stable low-level protocol comes from:

```text
corework/proto/corework_agent_tool_v1.proto
```

Prefer using the language SDKs under `sdk/` to implement sidecars instead of hand-writing gRPC details. Rust, Python, Node.js, and C# currently include runnable sidecars. Go and C++ keep authoring APIs, but their gRPC wiring still needs to be completed.

## 9.1 Integration Path

For an RPC tool to be called by an agent correctly, three layers must be in place:

1. The tool sidecar exposes tool descriptors through `AgentToolService.ListTools`.
2. `resources.json` registers the sidecar endpoint under `rpc_endpoints[]`, and
   the host submits it before start with `runtime.register_resources`.
3. The role or feature skill that needs the tool references the tool name in `tools`.

`rpc_endpoints[]` only connects the endpoint to runtime. `SKILL.md.tools` decides whether the current agent can see those tools. For endpoints with `protocol = "grpc"`, tool metadata must come from `ListTools`; do not duplicate inline tool definitions in runtime config.

## 9.2 Boundary: RagRetrieve Is Not A Business RPC Tool

`RagRetrieve` is a runtime built-in local glue capability used for before-thinking retrieval injection. Resources register knowledge endpoints, while an agent profile or `cluster.agents[]` configures retrieval policy. It is not a user-defined business RPC tool; list it in role/feature skill `tools` only when the model needs explicit second-pass retrieval.

Do not turn business RPC tools into hand-written json-lines just because `RagRetrieve` uses a json-lines retrieval link. Keep the boundary clear:

| Type | Owner | Config/exposure |
|---|---|---|
| `RagRetrieve` | Runtime built-in local glue / retrieval link | Driven by the current agent's `retrieval` config and routed through a dedicated resource endpoint. |
| User-defined business RPC tool | External sidecar | Must expose `AgentToolService.ListTools` / `Execute` through the official SDK. Tool declarations come from SDK registration, and skills reference concrete tool names. |

Production business tool endpoints should use `protocol = "grpc"`. Do not use hand-written json-lines sidecars to host production business RPC tools.

## 9.3 Tool-Set Endpoint

One sidecar endpoint may expose multiple tools. Organize tool sets by business domain, for example:

- `pptx-buns`: PPTX read, create, edit, and save operations.
- `excel-buns`: Excel read, write, edit, worksheet management, and batch operations.

Tool names inside a tool set should be stable, semantically clear, and case-stable. Do not expose temporary function names, internal helpers, or capabilities that are not intended for direct LLM use.

## 9.4 ToolDescriptor Contract

Every tool must describe itself through `ToolDescriptor`:

| Field | Requirement |
|---|---|
| `name` | Stable registration name. It is case-sensitive and is also the name referenced by skill `tools`. |
| `description` | Purpose description shown to the LLM. It should explain what the tool does and when to use it. |
| `parameters` | Parameter names, types, required flags, defaults, and descriptions. Names must match what the handler reads. |
| `outputs` | Optional, but recommended for declaring key fields in structured results. |
| `readonly` | Set to `true` for read-only query tools. |
| `destructive` | Set to `true` for deletion, overwrite, or irreversible mutation. |
| `idempotent` | Set to `true` when repeated execution has the same result or is safe to retry. |
| `open_world` | Set to `true` when the tool accesses the open external world or uncontrolled resources. |
| `secret` | Set to `true` for secrets or sensitive credentials. Do not expose these tools to ordinary skills by default. |
| `category` / `display_name` | Used for UI and debugging classification. Prefer the tool-set name and a readable display name. |
| `required_capabilities` | Explicitly declare required host capabilities, such as `workspace.resolve_path`. Dynamic context is not an RPC capability. |

### 9.4.1 Parameter Rules

Prefer simple, serializable parameters that can be expressed from CLI or JSON:

- Review each parameter first: does the AI really need to control it? If yes, adjust the requirement to reduce hallucination risk and improve correctness.
- Use ordinary parameters for strings, numbers, and booleans.
- Return recoverable errors for missing required parameters; do not panic.
- For single-choice string enums, list all possible enum values and their meanings in `parameters`. For multi-choice configuration, avoid asking the AI to return `Vec<string>` with several enum values; prefer multiple boolean parameters when possible.
- For path-like parameters, do not operate on raw paths directly. Prefer resolving or creating managed workspace paths through `workspace.*` HostCall.

## 9.5 AIOutput Return Contract

Tool execution must ultimately return `AIOutput`:

```text
result_json + to_ai + error_code
```

`to_ai` is required and must be non-empty. Runtime writes it back to the AI-visible tool-result channel, and SDKs also reject empty `to_ai`.

| Field | Meaning |
|---|---|
| `result_json` | Structured JSON consumed by programs or UI. On success, include machine-readable results when possible. On failure, it may be `null` or a structured error. |
| `to_ai` | Human-readable summary for the LLM to continue reasoning. It must be provided for both success and failure. |
| `error_code` | `TOOL_ERROR_CODE_OK` means success. Failures should use an appropriate non-OK error code. |

`to_ai` should not be only "ok" or "error". It should tell the model:

- Whether execution succeeded.
- What the key result is.
- If it failed, why it failed.
- How the model can recover next, such as filling parameters, changing paths, reading structure first, or narrowing scope.

## 9.6 Success And Failure Both Need `to_ai`

As in `pptx-buns` and `excel-buns`, wrap success and failure consistently:

```rust
fn ok(result: Value, to_ai: impl Into<String>) -> AIOutput {
    AIOutput { result, to_ai: to_ai.into(), error_code: ToolErrorCode::Ok }
}

fn err(to_ai: impl Into<String>) -> AIOutput {
    AIOutput { result: Value::Null, to_ai: to_ai.into(), error_code: ToolErrorCode::Internal }
}
```

When business execution fails, still return `AIOutput` and write the error into `to_ai`:

```rust
if !r.success {
    return Ok(err(format!("[failed] {}", r.description)));
}

Ok(ok(
    serde_json::json!({ "path": saved_path }),
    format!("[success] {}", r.description),
))
```

This lets the next thinking round see the failure fact and reason instead of receiving only a runtime exception. Use gRPC `ToolError` or the SDK exception path only when no business result can be formed, such as protocol errors, unknown tools, sidecar crashes, or handler panics.

## 9.7 HostCall Capabilities

When an RPC tool needs to access host-managed resources, it should use protocol-level `HostCall` and declare the capability in `required_capabilities`.

Current official v1 capabilities include:

| Capability | Purpose |
|---|---|
| `workspace.resolve_path` | Resolve a user-provided path into a host-managed work path. |
| `workspace.resolve_working_path` | Resolve a workspace path. |
| `workspace.create_path` | Create a host-managed path for a new file. |
| `workspace.create_working_path` | Create a workspace path. |
| `workspace.save_as_edited` | Save an edited work file as a user-visible file. |

Document-processing tools should use `workspace.*` to avoid direct sidecar rewrites of original user files. `snapshot.get:*` and `snapshot.put:*` are not supported v1 capabilities and must not appear in descriptors.

## 9.8 Dynamic State After Tool Execution

An RPC sidecar cannot directly update AI-visible dynamic context. Dynamic context for an agent is owned by the host application and keyed by `(conversation_id, agent_id, field_name)`.

When execution changes information needed by later reasoning, use this flow:

1. The tool returns structured identifiers and a concise change summary through `AIOutput.result_json` and `AIOutput.to_ai`.
2. The host observes that business change, reads or renders the latest state, and publishes each current plain-text field with `conversation.set_dynamic_snapshot`.
3. On the next thinking entry, runtime injects all current fields for that agent. Field names are update keys only; they do not hide field content from the model.

For example, after a document-editing tool returns the edited working path, the host may render a current document preview and publish it:

```json
{"schema":"agent-runtime-command/v1","type":"conversation.set_dynamic_snapshot","payload":{"conversation_id":"conv-1","agent_id":"editor-1","field_name":"document_preview","text":"..."}}
```

A recommended tool `to_ai` shape is:

```text
[success] Updated the document at the managed working path. The host should refresh the current document preview before follow-up reasoning relies on document content.
```

Dynamic fields are not persisted in agent cache snapshots. After conversation recovery, the host must publish the current fields again before relying on them.

## 9.9 Execution Context

When runtime calls an RPC tool, it includes the current call context in `ExecuteRequest`. Each language SDK should expose these fields through the handler's `ToolContext`:

| Protocol field | Meaning |
|---|---|
| `call_id` | This RPC call id. |
| `tool_call_id` | LLM function/tool call id, used to pair with the tool call in conversation history. |
| `idempotency_key` | Idempotency key, useful for deduplicating write tools. |
| `session_id` | Current runtime/session id. |
| `provider_id` | Tool provider/endpoint id. |
| `cluster_id` | Current runtime cluster. |
| `runtime_instance_id` | Current runtime instance. |
| `conversation_id` | Current conversation id. Business tools should prefer it for session isolation, routing lookup, or audit writes. |
| `agent_id` | Agent id that initiated the tool call. Useful for distinguishing sources in multi-agent scenarios. |
| `turn_id` | Current turn id. |
| `permissions` | Permissions passed by runtime. |
| `host_context_json` | Extended context passed through by the host. SDKs may parse it into language-native objects. |

Expose field names according to language convention: Python/Rust/C++ use `conversation_id`, Go/C# use `ConversationID` / `ConversationId`, and Node.js uses `conversationId`. Tools should not guess conversation or agent from global state; prefer reading `ToolContext`.

## 9.10 Registering With Runtime

Register the sidecar endpoint in `resources.json` under `rpc_endpoints[]`, then
call `runtime.register_resources` before start. Current endpoint fields are:

| Field | Meaning |
|---|---|
| `id` | Unique endpoint id. It must not be empty and is usually the tool-set name plus `-sidecar`. |
| `endpoint` | Sidecar listen address, for example `127.0.0.1:50104`. |
| `protocol` | Production tool ecosystems should use `grpc`. |
| `timeout_ms` | Per-call timeout, including HostCall round trips. |
| `launch` | Optional. Fill this when runtime should start the sidecar process. `kind` supports `external` / `process`. |

Business RPC tools should not inline tool declarations in runtime config. The production pattern is: resources only register a gRPC endpoint; the sidecar returns the tool list from `ListTools`; role/feature skills then select visible capabilities by tool name. This business-tool declaration rule does not apply to `RagRetrieve` retrieval config.

`allowed_snapshot_prefixes` has been removed from endpoint configuration. Configurations that still include it are invalid.

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "rpc_endpoints": [
    {
      "id": "pptx-buns-sidecar",
      "endpoint": "127.0.0.1:50104",
      "protocol": "grpc",
      "timeout_ms": 60000,
      "launch": {
        "kind": "process",
        "program": "cargo",
        "args": [
          "run",
          "--quiet",
          "--manifest-path",
          "agent-tools/pptx-buns/Cargo.toml",
          "--bin",
          "pptx-buns-sidecar"
        ],
        "working_dir": "../..",
        "env": {
          "PPTX_BUNS_ADDR": "127.0.0.1:50104",
          "RUST_LOG": "info"
        },
        "startup_timeout_ms": 120000,
        "shutdown_timeout_ms": 3000
      }
    }
  ]
}
```

If one endpoint exposes multiple tools through `ListTools`, skills reference concrete tool names, not endpoint `id`:

```yaml
---
name: office_operator
kind: feature
tools: [
  "DescribePresentation",
  "CreateBlankPresentation",
  "PptxFindAndReplace",
  "DescribeWorkbook",
  "ReadTable",
  "SetCell"
]
---
```

## 9.11 Recommended Practices

- Let each tool do one clear thing. Put composition flows in skill guidance instead of one universal tool.
- Make read tools `readonly=true` and `idempotent=true` by default.
- Mark delete, overwrite, and bulk-clear tools as `destructive=true`, and describe the affected scope clearly.
- Return failure `AIOutput` with `to_ai` for parameter errors, missing files, JSON parse failures, and business validation failures.
- `result_json` is for programs, and `to_ai` is for the next model reasoning step. Do not use either as a replacement for the other.
- Do not put large results entirely into `to_ai`; provide summaries, counts, key fields, and next-step suggestions. Put structured tool output in `result_json`; publish AI-visible changing text from the host through FFI when needed.
- For information that changes frequently because of tool calls and is needed for later reasoning, make the host refresh the relevant dynamic text fields after the tool changes the source state.
- Write tool descriptions and parameter descriptions for the LLM, not only as internal implementation names.
- Do not put host credentials, internal paths, or private data into `to_ai` unless the agent is explicitly allowed to see them.
- Sidecars should be discoverable through `ListTools` at startup. When an endpoint is unavailable, runtime should produce clear registration or call failures.
- Do not mistake runtime built-in glue such as `RagRetrieve` for a business RPC tool, and do not move business tools back to hand-written json-lines. Business tools should always use the official SDK to declare, register, and return `AIOutput`.

## 9.12 Minimal Rust SDK Shape

```rust
use corework_agent_tool::{AIOutput, ToolContext, ToolDescriptor, ToolErrorCode};
use serde_json::Value;

fn ok(result: Value, to_ai: impl Into<String>) -> AIOutput {
    AIOutput { result, to_ai: to_ai.into(), error_code: ToolErrorCode::Ok }
}

pub fn register_all_tools() {
    let descriptor = ToolDescriptor::builder("OrderList")
        .description("Query the current user's order list.")
        .parameter("user_id", "String", true, None, "User ID.")
        .output("orders", "Array", "Order list.")
        .readonly(true)
        .idempotent(true)
        .category("order")
        .display_name("OrderList")
        .build();

    corework_agent_tool::register_tool(
        descriptor,
        |_ctx: ToolContext, args: Value| async move {
            let user_id = args.get("user_id").and_then(Value::as_str).unwrap_or("");
            if user_id.is_empty() {
                return Ok(AIOutput {
                    result: Value::Null,
                    to_ai: "[failed] Missing user_id. Confirm the user identity first.".to_string(),
                    error_code: ToolErrorCode::MissingArgument,
                });
            }

            Ok(ok(
                serde_json::json!({ "orders": [] }),
                "[success] Found 0 orders."
            ))
        },
    );
}
```

## 9.13 Relationship To SSE Events

An RPC tool's `AIOutput.to_ai` enters the AI tool-result channel. What the frontend sees through the runtime event stream are ledger events such as tool started, succeeded, and failed. Their display contract is described in [`05-runtime-event-format.md`](./05-runtime-event-format.md).

The frontend should not rely directly on sidecar internal logs to decide whether a tool succeeded. It should use runtime-produced `tool_call_finished` / `tool_call_failed` events and the corresponding `record.content` and `metadata`.
