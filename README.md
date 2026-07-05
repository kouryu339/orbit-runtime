# orbit-runtime

`orbit-runtime` is an embeddable Rust Agent runtime for host applications.

It packages model access, Agent conversations, skills, tool execution, runtime
events, snapshots, and persistence behind a stable runtime boundary. Hosts can
embed the runtime through the C ABI dynamic library and keep their application
logic independent from the internal Rust implementation.

The primary integration path is:

```text
Host app
  -> agent_runtime.dll / libagent_runtime.so
  -> Agent Runtime
  -> LLM Gateway
  -> optional RPC Tool sidecars
```

## What Is Included

| Path | Purpose |
| --- | --- |
| `corework/` | Orchestration foundation: state machines, events, cache, workflows, and the RPC Tool protocol. |
| `ai-gateway/` | Model gateway for LLM / VLM / ASR / OCR providers and OpenAI-compatible endpoints. |
| `ai-assistant/` | Agent runtime: conversation state machine, skills, tool execution, ledger, and persistence. |
| `agent_runtime_ffi/` | C ABI wrapper that builds the native runtime library. |
| `ai-conversation-ui/` | Lit-based conversation frontend used by host integrations. |
| `examples/guides/` | Host integration, skills, tool, frontend, and runtime guides. |
| `scripts/` | Build, release packaging, and local development helper scripts. |

## Build The Runtime

Build the native runtime library from the repository root:

```powershell
cargo build -p agent-runtime-ffi --release
```

On Windows this produces:

```text
target/release/agent_runtime.dll
```

On Linux this produces:

```text
target/release/libagent_runtime.so
```

The C ABI header is:

```text
agent_runtime_ffi/include/agent_runtime.h
```

## Prepare Release Packages

Release packages are prepared by:

```powershell
.\scripts\prepare-release.ps1
```

The script currently supports Windows and Linux packages:

```text
dist/releases/orbit-runtime-runtime-vX.Y.Z-windows-x86_64.zip
dist/releases/orbit-runtime-runtime-vX.Y.Z-linux-x86_64.zip
```

Each package contains the native library, `agent_runtime.h`, `LICENSE`, a short
package README, `NOTICE`, and a `.sha256` checksum next to the zip file. macOS
artifacts are intentionally not produced by this script because this release
environment does not provide an Apple build machine.

Useful variants:

```powershell
.\scripts\prepare-release.ps1 -Targets windows
.\scripts\prepare-release.ps1 -Targets linux
.\scripts\prepare-release.ps1 -Targets linux -SkipBuild
```

## Release Trust

The source code in the Git tag is the primary artifact for this project. Native
runtime binaries are convenience artifacts published from the corresponding
GitHub Release.

Only trust release assets published under
[`kouryu339/orbit-runtime`](https://github.com/kouryu339/orbit-runtime). Do not
treat third-party repackaged native binaries as official builds.

Current release packages include SHA-256 checksums. The project aims to keep
release packages reproducible from the tagged source and to add CI verification,
SBOM publication, and signed release tags as the release process matures.

## Recommended Reading

Start with the host-facing guides:

1. `examples/guides/en/01-sdk-runtime-connection-flow.md`
2. `examples/guides/en/03-external-tools.md`
3. `examples/guides/en/04-skills.md`
4. `examples/guides/en/05-host-runtime-frontend.md`

Lower-level design documents live in:

```text
corework/docs/
ai-assistant/docs/
agent_runtime_ffi/docs/
```

## Status

Covered integration paths:

- FFI runtime creation, startup, command dispatch, event polling, snapshot export, shutdown, and destroy.
- Agent conversation lifecycle, tool execution, skill loading, ledger state, and persistence contracts.
- LLM gateway integration for provider configuration and OpenAI-compatible endpoints.
- Lit frontend integration through host-provided runtime events.
- Windows and Linux native runtime release package preparation.

Still evolving:

- SDK package distribution around the native runtime artifact.
- Additional host examples.
- macOS release packaging from an Apple build environment.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
