# Conversation UI implementations

`lit/` and `svelte/` are peer implementations of the same Orbit Runtime
conversation contract. Both consume `frontend:state_snapshot`, use the same
ledger/tool semantics, and keep model selection conversation-scoped.

- `lit/` publishes the framework-neutral host, protocol, content, and transport
  contracts together with the Lit custom element.
- `svelte/` consumes those framework-neutral subpath exports and implements its
  own Svelte rendering layer. It does not embed the Lit custom element.

Both directories produce ordinary frontend build output. Desktop or other
native hosts remain separate consumers of these UI packages.

Feature parity is checked against `ui/feature-parity.json`. A feature should not
be marked supported until both implementations have a concrete code path and a
test or build-time verification route.
