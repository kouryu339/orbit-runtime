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

Both renderers also share the display-item projection and progressive-reveal
helpers from `lit/src/content`. Svelte's scoped styles are generated from the
Lit component styles with `node ui/sync-svelte-styles.mjs`. Run the script with
`--check` to detect drift; Svelte does not import or wrap Lit custom elements.

Feature parity is checked against `ui/feature-parity.json`. A feature should not
be marked supported until both implementations have a concrete code path and a
test or build-time verification route.
