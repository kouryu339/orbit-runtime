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
uint32), and `runtime.set_auth_context`. `runtime.reload_llm` accepts the same
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
models. Existing conversations use the newly selected current model for later
LLM calls; cluster and Agent configs must not embed provider secrets or fake
model placeholders to compensate for delayed setup.

Do not overwrite a valid loaded configuration with example placeholders.
Hosts own production secrets. Failures use the result envelope and thread-local
last-error contract.
