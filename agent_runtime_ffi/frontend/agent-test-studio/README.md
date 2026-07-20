# Agent Test Studio Frontend

This private workspace package builds the embedded developer UI served by the
Agent Runtime FFI test-studio server. Pair lifecycle, relay, evidence, and
conversation ownership remain in Rust; the React frontend renders controller
state and sends Studio commands.

```text
npm install
npm run build
```

The generated `dist/index.html` and `dist/assets/*` files are committed because
`agent_runtime_ffi` embeds them at compile time with `include_bytes!`.

Run `npm run dev` only for frontend development. Normal Runtime hosts open the
embedded Studio through `studio.open_agent_test`.

The embedded conversation uses the shared Lit approval shelf. Studio SSE
delivers `frontend:state_snapshot.pending_permissions`; user decisions return
through `POST /api/tool-permission` and are restricted to the supervisor
conversation. Rebuild this package after changing the shared conversation
component because Runtime embeds the generated assets.
