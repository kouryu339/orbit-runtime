# Workflow Studio Frontend

This private workspace package builds the Workflow Studio UI embedded by
`agent_runtime_ffi`. The Runtime `WorkflowsModule` owns the Draft/Registered
catalog, compilation, execution, revisions, and persistence. React visualizes
the selected resource and autosaves each valid graph edit through that module
with the selected `expected_revision`; it is not a separate draft source of
truth.

Workflow resources use two synchronized representations. Script is the compact
AI-readable semantic form. Blueprint JSON is the executable graph and carries
canvas position, size, and layout metadata. A script update recompiles the
graph and migrates layout by node id or source step; a canvas update decompiles
the saved graph back to script. Registered resources persist Blueprint JSON.

```text
npm install
npm run build
```

The generated `dist/index.html` and `dist/assets/*` files are committed because
the Runtime embeds them at compile time with `include_bytes!`.

Run `npm run dev` only while developing the Studio frontend. Runtime hosts open
the embedded application through `studio.open_workflow`.

The embedded conversation uses the shared Lit approval shelf. Studio SSE
delivers `frontend:state_snapshot.pending_permissions`; user decisions return
through `POST /api/tool-permission` and are resolved against the Studio-owned
conversation only. Rebuild this package after changing the shared conversation
component because Runtime embeds the generated assets.

Workflow mutations and executions arrive over the same Studio SSE connection
as public Runtime events with `event_line: "workflow"`. The UI refreshes the
catalog and rereads the affected selected resource by stable `workflow_id`.
There is no conversation-scoped draft-update event.

The Runtime also subscribes a Workflow snapshot projector to the global event
line for the Studio-owned editor Agent. It updates the selected resource and
catalog tail snapshots after each mutation. `POST /api/studio-state` remains a
pull/selection synchronization path, not the catalog change transport.
