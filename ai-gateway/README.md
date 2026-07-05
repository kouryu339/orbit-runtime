# Corework LLM Gateway

`llm-gateway` provides configuration-driven access to text, vision, speech,
and OCR providers for the Agent Runtime. Provider selection and credentials are
resolved from runtime configuration; applications should not hard-code secrets
or expose provider administration directly to an untrusted frontend.

The main public modules cover provider configuration, OpenAI- and
Anthropic-compatible transports, retry classification, multimodal calls, and
request-scoped headers. Start with [`docs/en/01_architecture.md`](docs/en/01_architecture.md)
or [`docs/01_架构设计.md`](docs/01_架构设计.md).

Supported API paradigms are OpenAI Chat Completions and Anthropic Messages.
`openai_responses` is reserved in configuration but currently rejected with a
configuration error; it must not be advertised as an available transport.

```text
cargo test -p llm-gateway
cargo clippy -p llm-gateway --all-targets -- -D warnings
```
