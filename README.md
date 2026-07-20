# orbit-runtime

`orbit-runtime` is an embeddable Rust Agent runtime for applications that need
LLM conversations, skills, tool execution, runtime events, snapshots, and
persistence behind a stable native boundary.

The runtime is designed to be hosted by desktop apps, services, scripting
bridges, or language SDKs. Host applications load the native library, register
configuration, start conversations, and relay events without linking directly to
the internal Rust crates.

```text
Host application
  -> agent_runtime.dll / libagent_runtime.so
  -> Agent Runtime
  -> LLM Gateway
  -> optional RPC Tool sidecars
```

## Project Highlights

- **Embeddable Agent runtime**: create and control conversations from a host
  process while keeping the runtime implementation isolated behind an ABI.
- **Stable C ABI boundary**: ship `agent_runtime.dll` or `libagent_runtime.so`
  with `agent_runtime.h`, then build higher-level SDKs on top of that native
  contract.
- **Host-oriented event model**: hosts poll a single runtime event stream and
  decide how to render UI state, persist archives, or fan out messages.
- **Skills and tool governance**: role and feature Skills describe what an
  Agent can see and which tools it may call, so product capability is explicit.
- **RPC Tool sidecar protocol**: external tools run out of process and publish
  descriptors through a language-neutral gRPC contract.
- **LLM Gateway integration**: provider configuration and OpenAI-compatible
  endpoints are handled through a dedicated gateway layer.
- **Snapshots and persistence contracts**: conversations can export runtime
  state for recovery, migration, and host-owned storage.
- **Split SDK surface**: Runtime Host SDKs embed the runtime; RPC Tool SDKs
  implement callable business tools.

## Repository Layout

| Path | Purpose |
| --- | --- |
| `corework/` | Orchestration foundation: state machines, events, cache, workflows, and the RPC Tool protocol. |
| `ai-gateway/` | Model gateway for LLM / VLM / ASR / OCR providers and OpenAI-compatible endpoints. |
| `ai-assistant/` | Agent runtime: conversation state machine, skills, tool execution, ledger, and persistence. |
| `agent_runtime_ffi/` | C ABI wrapper that builds the native runtime library. |
| `ai-conversation-ui/` | Lit-based conversation frontend used by host integrations. |
| `sdk/` | Runtime Host SDKs and RPC Tool SDKs by language. |
| `examples/guides/` | Host integration, skills, tool, frontend, and runtime guides. |
| `scripts/` | Build, release packaging, and local development helper scripts. |

## Example Programs

The example material is intentionally split into two layers.

### Integration Guides In This Repository

`examples/guides/` is the recommended starting point for building your own host
application. It walks through the full integration order:

```text
1. Tools       Prepare callable built-in tools or RPC sidecars.
2. Configs     Register resources, LLM providers, and Agent clusters.
3. Skills      Write role and feature Skills with explicit tool allowlists.
4. Connect     Load the native runtime through a Runtime Host SDK.
5. Run         Start conversations, relay events, and persist host state.
```

Start here:

- `examples/guides/en/01-sdk-runtime-connection-flow.md`
- `examples/guides/en/03-external-tools.md`
- `examples/guides/en/04-skills.md`
- `examples/guides/en/05-host-runtime-frontend.md`

### Desktop Reference App

A complete Tauri desktop host is maintained as a separate open-source example:

```text
https://github.com/kouryu339/assistant-tauri
```

Use it when you want to see a real host application wiring together the Runtime
Host SDK, native runtime artifact, Lit conversation UI, RPC Tool sidecars,
resource registration, frontend event relay, and release packaging.

Keeping the desktop app in a separate repository helps `orbit-runtime` stay
focused on the runtime, SDK contracts, and native release artifacts.

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

## SDKs

SDKs live under `sdk/` and are split by integration direction:

| SDK family | Used by | Direction |
| --- | --- | --- |
| Runtime Host SDK | Desktop apps, services, scripting hosts | Host -> Runtime |
| RPC Tool SDK | Tool sidecars and product capability adapters | Runtime -> Tool |

See `sdk/README.md` for language support and the native runtime release manifest.

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

Start with examples before reading implementation details:

1. `examples/guides/en/01-sdk-runtime-connection-flow.md`
2. `examples/guides/en/03-external-tools.md`
3. `examples/guides/en/04-skills.md`
4. `examples/guides/en/05-host-runtime-frontend.md`
5. `examples/guides/en/11-dynamic-workflows.md`
6. `sdk/README.md`

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
- Additional host examples and packaging patterns.
- macOS release packaging from an Apple build environment.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
