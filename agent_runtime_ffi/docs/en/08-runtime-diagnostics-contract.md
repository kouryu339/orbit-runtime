# 8 Runtime Diagnostics Contract

Logs are for human diagnostics. Events are for machine-consumed state. Hosts,
frontends, and state mirrors should consume `agent-runtime-event/v1`; they must
not parse runtime log files for state synchronization.

## 8.1 Default Logs

Default runtime logs should be low-noise, useful for diagnosis, and avoid
leaking conversation context.

| Category | Default level | Include | Exclude |
| --- | --- | --- | --- |
| Startup/config summary | `info` | runtime instance id, profile id, data_dir, skills_dir, provider config load status | API keys, full provider JSON, full prompt |
| Lifecycle | `info` | runtime start/stop, conversation create/close/import/materialize ids and result | user message text, full snapshot |
| External dependency errors | `warn/error` | LLM HTTP status, error code, model id, retry count, short summary | request body, full response body, auth headers |
| Event export health | `warn/error` | envelope build failure, handle queue disconnect | every normal event payload |
| Tool execution errors | `warn/error` | tool name, call id, error code, duration, failure summary | full arguments, full result |
| Recovery/routing errors | `warn/error` | not_owner, runtime unavailable, materialize/rebind failure, route lease busy | full ledger history |

## 8.2 Opt-In Diagnostics

These outputs must be behind explicit diagnostic switches:

| Content | Reason | Switch |
| --- | --- | --- |
| LLM messages, prompt, history, compacted context | private and large | `RUNTIME_CONTEXT_PROBE=1` |
| Dynamic snapshot text | may contain page state or user input | `RUNTIME_CONTEXT_PROBE=1` |
| LLM request/response details | may contain prompt, tool schema, model output | `RUNTIME_LLM_TRACE=1` |
| Full `frontend:state_snapshot` payloads | events, not logs | consume through `next_event_v1` |
| LLM usage/error facts | durable accounting and diagnostics | consume `conversation.ledger_delta` records with `metadata.subtype = "llm_usage"` or `"llm_error"` |
| Normal MQ/SSE publish success | high noise | record failures or sampled metrics only |

## 8.3 Environment Variables

Recommended defaults:

```bash
RUST_LOG=info,agent_runtime_ffi=info,ai_assistant=info,llm_gateway=warn,corework=warn
RUNTIME_LLM_TRACE=0
RUNTIME_CONTEXT_PROBE=0
AI_GATEWAY_DIAGNOSTICS=off
```

LLM request diagnostics:

```bash
RUNTIME_LLM_TRACE=1
RUNTIME_LLM_TRACE_FILE=./data/logs/agent-runtime-llm-trace.jsonl
```

Context probe diagnostics:

```bash
RUNTIME_CONTEXT_PROBE=1
RUNTIME_CONTEXT_PROBE_FILE=./data/logs/runtime-context-probe.log
```

## 8.4 Outputs

| Output | Responsibility |
| --- | --- |
| `{data_dir}/logs/agent-runtime.log` | startup, config summary, key errors |
| `RUNTIME_LLM_TRACE_FILE` | opt-in LLM trace JSONL |
| `RUNTIME_CONTEXT_PROBE_FILE` | opt-in context probe |
| FFI pull queue | `agent-runtime-event/v1` from `agent_runtime_next_event_v1` |
| Host stdout/stderr | host-service logs collected by the host process manager |

## 8.5 Log/Event Boundary

Stable external protocol:

```text
agent-runtime-event/v1
```

Diagnostic files:

```text
agent-runtime.log
agent-runtime-llm-trace.jsonl
runtime-context-probe.log
```

The host may adapt pull-queue events to SSE, Redis Stream, Kafka, RocketMQ, or
another transport. It should not tail runtime logs to implement state sync.
