# Corework LLM Gateway

`llm-gateway` provides configuration-driven access to text, vision, speech,
and OCR providers for the Agent Runtime. Provider selection and credentials are
resolved from runtime configuration; applications should not hard-code secrets
or expose provider administration directly to an untrusted frontend.

The main public modules cover provider configuration, OpenAI- and
Anthropic-compatible transports, retry classification, multimodal calls, and
request-scoped headers. Start with [`docs/en/01_architecture.md`](docs/en/01_architecture.md)
or [`docs/01_架构设计.md`](docs/01_架构设计.md).

Supported API paradigms are OpenAI Chat Completions, OpenAI Responses, and
Anthropic Messages. Native function calls are normalized to stable tool names,
JSON arguments, and provider call IDs. Responses output items are retained for
the next turn instead of being flattened into chat text.

Compatible providers use non-strict tool schemas by default. Set
`strictToolSchema` (or `strict_tool_schema`) on a provider only when the
endpoint supports strict function schemas. This changes schema generation; it
does not enable protocol fallback.

```text
cargo test -p llm-gateway
cargo clippy -p llm-gateway --all-targets -- -D warnings
```
