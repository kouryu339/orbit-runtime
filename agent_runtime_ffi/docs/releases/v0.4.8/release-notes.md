# Orbit Runtime v0.4.8

## Changes since v0.4.7

- Native function calling now supports streaming tool calls and preserves typed JSON arrays for direct RPC tool inputs.
- Workflow execution exposes ordered run traces, including nested references and RPC nodes. Script and blueprint revisions are checked together before execution.
- Workflow authoring gains structured plans, local search, registered Workflow references, and clearer multi-error compiler diagnostics.
- Jev execution units can be registered by the host and select from explicitly allowed atomic tools while retaining conversation ownership, snapshots, permission checks, and host-visible events.
- Agent tool definitions keep input requirements available in the resident catalog; `ToolDescribe` provides full input and output contracts on demand.
- History compaction uses recoverable checkpoints and begins when the complete projected request reaches 90% of the active model context window. It summarizes only the current Agent's messages and does not set a Runtime output cap for summaries.
- Runtime honors configured context windows for models absent from its embedded catalog and reports the effective window to hosts.
- The Pure `Split` node returns an empty string array for `null`, so a downstream `FOR` loop performs zero iterations. Empty strings retain their existing single-empty-item behavior.

## Host integration

- ABI remains 1.3. Replace the native library and update the matching SDK release metadata and SHA-256 checksums together.
- The `main` branch's conversation recovery rollback is retained: restoring a conversation does not automatically restart an Agent driver, and Lit does not queue a first message during connection or pause.
- Restart the Runtime after replacing the native library. Existing model context is not rewritten with updated embedded guidance.

## Distribution

Windows x86_64 and Linux x86_64 native archives are published. No macOS archive is available from this release machine.
