# 1 LLM Gateway Architecture

`llm-gateway` is the model-provider abstraction layer for `orbit-runtime`.

It hides provider-specific request formats behind a configuration-driven API so
the Agent Runtime can call text, vision, speech, and OCR models through a common
interface.

## 1.1 Responsibilities

- Load provider and model definitions from JSON configuration.
- Resolve the current model by global model UID.
- Dispatch requests to OpenAI-compatible, Anthropic-compatible, and native
  provider adapters.
- Support LLM, VLM, ASR, and OCR call paths.
- Normalize provider responses into runtime-facing result types.
- Provide diagnostics and retry helpers for provider failures.

## 1.2 Key Terms

- **Provider**: a configured API endpoint and credential set, such as DeepSeek,
  Qwen, Claude, or a custom OpenAI-compatible endpoint.
- **Model UID**: a stable runtime identifier used by host apps to select a model
  without depending on provider-specific model IDs.
- **Model ID**: the provider-facing model name sent to the upstream API.
- **Current model**: the model UID selected by `current_model_uid`.

## 1.3 Configuration

Runtime examples read provider configuration from:

```text
examples/python_ctypes/llm_config.json
```

Built-in model metadata lives in:

```text
ai-gateway/config/builtin_models.json
```

## 1.4 Runtime Flow

```text
Agent Runtime
  -> model UID
  -> llm-gateway config resolver
  -> conversation request context headers
  -> provider adapter
  -> upstream model API
  -> normalized response
```

The Agent Runtime should depend on normalized gateway types instead of
provider-specific HTTP payloads.

## 1.5 Conversation Request Headers

`ai-gateway` can receive per-conversation request headers from the runtime request context. The headers are appended by the OpenAI-compatible and Anthropic-compatible adapters immediately before the outbound HTTP request is sent.

This is intended for integrated deployments where the host application uses a user/session token instead of a static API key, for example:

```json
{
  "Authorization": "Bearer runtime-session-token",
  "X-User-Id": "user-a"
}
```

Ownership rules:

- The runtime or host decides which headers are present for a conversation.
- `ai-gateway` only forwards the headers for the current async request context.
- Headers are not written to provider config, ledger, snapshots, or global state.
- `Content-Type` is controlled by the provider adapter and is not overridden by context headers.

When context headers are present, HTTPS is required for non-loopback endpoints by default. Local loopback HTTP is allowed for development. A host can explicitly opt in to insecure non-loopback HTTP through the FFI conversation option `allow_insecure_llm_request_headers`, accepting the credential leakage risk outside the runtime core's responsibility boundary.
