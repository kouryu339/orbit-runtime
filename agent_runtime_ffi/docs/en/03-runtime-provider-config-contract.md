# 3 Provider Configuration Contract

Provider configuration covers vendors, credentials/base URLs, model definitions,
and current-model selection. It is separate from Agents, Skills, and tools.

It is one of the host registration planes for a real Agent host: resources
define what the product exposes, provider config defines which model Runtime may
call, and agent cluster config defines the concrete Agents that can run.

Before start, hosts may register providers by invoking `runtime.register_llm`
with an `agent-runtime-llm-registration/v1` document. This registration may be
omitted or empty for delayed model setup. `agent_runtime_create_v1` does not
load provider files or runtime config paths.

Hosts should pass the JSON object directly as `payload.registration`. Config
files and persistence locations are host implementation details and should not
cross the Runtime boundary. `payload.input` still accepts a JSON object, JSON
string, or file path for compatibility with older hosts.

Runtime commands are `runtime.register_llm`, `runtime.reload_llm`,
`runtime.configure_providers` (prefer a `registration` JSON object),
`runtime.get_provider_definitions`, `runtime.set_current_model` (`model_uid` is
uint32 and changes only the default for new conversations),
`conversation.set_model` (changes one existing conversation), and
`runtime.set_auth_context`. `runtime.reload_llm` accepts the same
`payload.input` / `payload.registration` boundary as registration commands; the
input may be an `agent-runtime-llm-registration/v1` document or a provider
config document/file. All commands go through `agent_runtime_invoke_v1`; there
is no standalone provider C function.

## Empty Model / Delayed Model Configuration

Provider configuration may be absent at first launch, or present as an empty
`agent-runtime-llm-registration/v1` / provider config with `providers: []` and
`current_model_uid: null`. This is a supported state, not a migration fallback.

In the empty-model state:

- Runtime may start, register clusters, create or restore conversations, and
  expose provider definitions as an empty list.
- Any turn that needs an LLM must stop with a recoverable provider/model
  configuration error until the host configures providers and selects a current
  model.
- Hosts should show a product-level "configure provider and model first" state
  instead of shipping example providers or overwriting user secrets.

Hosts may configure models later by calling `runtime.configure_providers` or
`runtime.reload_llm`, then `runtime.set_current_model` when the loaded config
does not already contain a valid `current_model_uid`. A non-null
`current_model_uid` is valid only when it references one of the loaded enabled
models. This value is only the fallback for new conversations and does not
overwrite an existing conversation's model. Hosts must use
`conversation.set_model` for an existing conversation; cluster and Agent configs
must not embed provider secrets or fake model placeholders to compensate for
delayed setup.

Do not overwrite a valid loaded configuration with example placeholders.
Hosts own production secrets. Failures use the result envelope and thread-local
last-error contract.

## Model reasoning by API paradigm

The supported paradigms remain `openai_chat_completions`, `openai_responses`, and `anthropic_messages`. Encode reasoning by the selected paradigm only; no vendor-specific endpoint, model-name, or parameter mapping is added.

Each `enabled_models[]` entry accepts an optional `reasoning` object (legacy bundles use `models[]`). Public provider definitions, persistence and explicit host configuration reload preserve it. Omitting the object, or using `{}`, leaves requests unchanged.

| Configuration | Paradigm | Request field |
|---|---|---|
| effort | Chat Completions | reasoning_effort |
| effort | Responses | reasoning.effort |
| summary | Responses | reasoning.summary |
| effort | Anthropic | output_config.effort |
| thinking | Anthropic | thinking.type |
| budget_tokens | Anthropic | thinking.budget_tokens |

Effort values: `none/minimal/low/medium/high/xhigh/max`. Anthropic rejects `none/minimal`; use `thinking: "disabled"` to request disabled thinking. Responses summary accepts `auto/concise/detailed`. Anthropic thinking accepts `disabled/enabled/adaptive` independently of effort; omit it to leave the decision to the upstream service. Enabled thinking requires `budget_tokens >= 1024`; other modes reject a budget.

Example enabled model:

```json
{
  "uid": 1001,
  "model_id": "host-model",
  "max_context_tokens": 128000,
  "reasoning": {"effort": "high", "summary": "auto"}
}
```

Use this example with Responses. For Chat, omit summary. Anthropic may use `{"effort":"high","thinking":"adaptive"}` or `{"thinking":"enabled","budget_tokens":2048}`. This config is independent of system prompts, Runtime thinking states and thinking-round limits.

- Registration validates the configuration against the selected paradigm. Wrong fields, unknown enums and conflicting mode/budget combinations fail explicitly.
- Both streaming and non-streaming requests, tools and retries use the model UID's reasoning settings through explicit arguments, never a reverse model-name lookup.
- Chat with explicit effort encodes the requested output limit as `max_completion_tokens`; Responses uses `max_output_tokens`, Anthropic uses `max_tokens`.
- Explicit non-none Chat/Responses effort or enabled/adaptive Anthropic thinking omits temperature and top_p. Unconfigured or disabled settings preserve the existing sampling behavior.
- Anthropic fixed thinking budget must be less than request max_tokens (existing default 4096). Runtime never increases it automatically. Enabled/adaptive thinking conflicts with forced tool/any choice and fails before sending rather than silently weakening tool requirements.
- Anthropic signed thinking/redacted blocks are retained in provider_items and replayed with history and ledger data; they are not executable content or displayed answer text. Responses excludes Anthropic thinking blocks when changing paradigms.
- Supported values still depend on the target compatible endpoint/model. Upstream rejection remains an error; Runtime does not silently omit reasoning on retries.

Lit's model editor exposes paradigm-specific settings and preserves reasoning on other models when saving providers. Native ABI and command schemas are unchanged; SDKs continue to use their existing JSON registration/configuration boundary.

References: [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning), [Claude effort](https://platform.claude.com/docs/en/build-with-claude/effort), [Claude extended thinking](https://platform.claude.com/docs/en/build-with-claude/extended-thinking).
