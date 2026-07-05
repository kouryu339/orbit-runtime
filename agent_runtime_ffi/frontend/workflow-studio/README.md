# Workflow Studio Frontend

This private workspace package builds the Workflow Studio UI embedded by
`agent_runtime_ffi`. Rust owns workflow loading, compilation, testing, and
persistence; the React application edits and visualizes the current draft.

```text
npm install
npm run build
```

The generated `dist/index.html` and `dist/assets/*` files are committed because
the Runtime embeds them at compile time with `include_bytes!`.

Run `npm run dev` only while developing the Studio frontend. Runtime hosts open
the embedded application through `studio.open_workflow`.
