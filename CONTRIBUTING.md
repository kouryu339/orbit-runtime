# Contributing

Thank you for contributing to `orbit-runtime`.

## Before A Pull Request

Keep changes scoped, preserve public ABI and JSON schema compatibility, and add
tests for behavior changes. Do not commit credentials, generated build output,
local runtime data, or editor-specific files.

Run the checks relevant to your change:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For the conversation UI:

```text
cd ai-conversation-ui
npm run check
npm test
npm run build
```

Documentation and source files must be UTF-8. Update the English and Chinese
contract documents together when changing a public command, event, or schema.

## Compatibility

- Keep the C ABI stable within ABI major version 1.
- Add Runtime features through versioned JSON commands and capabilities.
- Treat `agent_runtime_capabilities_v1()` as the machine-readable public surface.
- Do not expose Studio/test conversations or internal event types to host event streams.
