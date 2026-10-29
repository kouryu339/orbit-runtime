# Dynamic RPC additions

After start, a trusted host can call `runtime.add_rpc_endpoint` to discover and
atomically append an external gRPC endpoint. Check
`features.dynamic_rpc_additions == 1` in capabilities first. This management
command is not automatically exposed to agents.

```json
{
  "schema": "agent-runtime-command/v1",
  "type": "runtime.add_rpc_endpoint",
  "payload": {
    "schema": "agent-runtime-rpc-endpoint-add/v1",
    "endpoint": {
      "id": "project-tools",
      "protocol": "grpc",
      "endpoint": "http://127.0.0.1:55061",
      "timeout_ms": 30000,
      "to_ai_max_chars": 12000
    }
  }
}
```

See the [request schema](../rpc-endpoint-add.schema.json). Omit `launch`:
dynamic process management, JSON-lines and retrieval bindings remain startup-only.
Tools must come from standard `AgentToolService.ListTools`; inline metadata,
replacement flags and unknown fields are rejected. Discovery has a total deadline
of `timeout_ms` (1–60000 ms), a limit of 1024 tools and 4 MiB of metadata, and must
return at least one tool. The same timeout applies to subsequent calls.
`to_ai_max_chars` defaults to 600 and accepts 1–1000000.

An existing endpoint ID is always `rpc_endpoint_exists`, even for identical
configuration. A conflict with any existing dynamic tool, built-in tool or the
reserved `WorkflowRef_` prefix rejects the whole batch with
`rpc_tool_name_conflict`. Conflicts appear in `error.details.conflicts`.
Invalid configuration/limits use `invalid_argument`; discovery or remote catalog
contract failures can use `rpc_failed`. No replacement, removal or refresh command is provided.

Discovery holds no facade operation lock. Commit rechecks the latest catalog and
lifecycle: only one competing registration of the same ID/name can succeed, and
shutdown rejects an outstanding discovery result with `bad_state`. Failed batches
publish nothing. Metadata and executors are published together per Runtime scope.

Registration grants no agent access. Skills, active tool lists and approvals still
apply. Existing contexts read the shared catalog for descriptions, native tool
definitions, validation and Workflow authoring. Existing graphs and already issued
model requests are not rewritten.

The result includes `endpoint_id`, `protocol`, `catalog_revision`, `contract_hash`,
`tool_count`, `tool_names` and `persistence: "host_managed"`. The contract hash is
SHA-256 of effective metadata sorted by tool name; it is not server authentication.
One `runtime.rpc_catalog_changed` event follows commit, with the same summary and
`command_id`. Connection addresses and process configuration are not included.

Use the existing endpoint, tool and Workflow-node definition queries. Endpoint and
tool queries include `catalog_revision`; added endpoints report
`verification_scope: "add_list_tools"`. `ready` records successful discovery,
not continuous health monitoring.

The host persists accepted endpoint configuration and registers it before start on
the next launch, allowing dependent Workflow sources to restore. Runtime does not
write or watch host configuration files. Query the catalog after a lost response;
re-registration never updates configuration.

SDK methods accept the endpoint object: Python/Rust `add_rpc_endpoint`,
C++ `add_rpc_endpoint_json`, Go `AddRPCEndpoint`. C uses the existing invoke ABI.
RPC catalogs are instance-scoped; existing global Workflow resource ownership
still prevents claiming support for simultaneous full Runtime instances.
