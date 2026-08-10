# orbit-runtime

**Build Agent products from composable roles, Skills, tools, retrieval, and
runtime lifecycles—behind one embeddable Rust boundary.**

`orbit-runtime` is not a fixed supervisor/worker framework. It is a native
runtime for assembling the Agent topology a product actually needs: one focused
Agent, a persistent specialist team, delegated background workers, or a mixture
of all three.

Desktop apps, services, scripting bridges, and language SDKs can host the same
runtime through `agent_runtime.dll` / `libagent_runtime.so` and a stable C ABI.

```text
Product host
  ├─ registers models, Agent profiles, Skills, and tool endpoints
  ├─ starts conversations and consumes one event stream
  └─ owns UI, approvals, archives, and recovery
          │
          ▼
    orbit-runtime
      ├─ Agent state machines and auditable ledgers
      ├─ Skill-governed tool execution
      ├─ delegated tasks and focus handoff
      └─ LLM gateway + optional RPC Tool/RAG sidecars
```

## Why orbit-runtime

| Capability | Product-level result |
| --- | --- |
| **Composable Agent clusters** | Choose delegation, focus handoff, progressive capabilities, and retrieval per role instead of inheriting one topology. |
| **Skills as capability boundaries** | Role, feature, and system Skills jointly define instructions and the enforced tool allowlist. |
| **Reliable background work** | Task-scoped authorization, wait/wakeup, input requests, progress, revision, two-phase acceptance, pause, cancellation, and recovery. |
| **Embeddable native runtime** | Keep Rust internals behind a stable ABI while hosts use C++, Python, Go, Rust, or another bridge. |
| **Observable and recoverable state** | Consume ordered events and export snapshots without reconstructing truth from chat text. |
| **Out-of-process business tools** | Implement tool sidecars through a language-neutral gRPC contract and keep product integrations isolated. |

## Assemble The Cluster You Need

The cluster is assembled from small, independent contracts:

| Layer | Responsibility |
| --- | --- |
| Agent profile | Reusable model, role, initial features, and retrieval binding. |
| Role Skill | Identity, delegation policy, lifecycle rules, and minimum entry-point tools. |
| Feature Skill | Optional domain capability loaded only when the current task needs it. |
| System Skill | Cross-cutting behavior such as planning and progressive Skill discovery/loading. |
| Runtime relation | The actual delegator/assignee or focus relationship that authorizes follow-up actions. |

### Example: a lead that can hire a policy researcher

Register `research.policy` as an Agent profile, grant the lead only the task
publication entry point, and write the allowed worker profile into the lead's
Role Skill:

```markdown
---
name: research_lead
description: "Plans research and accepts delegated results."
kind: role
tools: ["CreateBackgroundAgentTask"]
---

# Research lead

- Delegate policy research with `CreateBackgroundAgentTask` using
  `name="research.policy"`.
- Treat `ReportAgentTask` as a candidate result. Accept it with
  `CompleteAgentTask`, request another revision with `UpdateAgentTask`, or
  abandon it with `CancelAgentTask`.
```

That small declaration produces a controlled lifecycle:

```text
lead publishes task for registered profile "research.policy"
  -> Runtime creates a unique background Agent and task relation
  -> worker may request input, report progress, and submit a candidate result
  -> lead may wait, respond, revise, pause, accept, or cancel
  -> Runtime retires the worker only after an explicit terminal decision
```

The lead does not receive arbitrary Agent control. Runtime derives every
follow-up permission from the concrete task relation and rejects self-targeting
or unrelated Agent IDs.

### Add capabilities progressively

```markdown
---
name: policy_recall
description: "Run a narrower policy lookup when automatic retrieval is insufficient."
kind: feature
tools: ["RagRetrieve"]
---

Use `RagRetrieve` for a narrower second lookup when the automatically retrieved
context is insufficient.
```

With the built-in `thinking` system Skill, an Agent can use `GetSkillsList` and
`UpdateSkills` to discover and activate `policy_recall` only when needed.
Instructions and tool schemas do not have to occupy every model request.

Retrieval is independently bound per Agent. Runtime can perform automatic
pre-thinking retrieval, while an active feature such as `policy_recall` exposes
`RagRetrieve` for a narrower second pass. The call remains pinned to that
Agent's configured endpoint.

This is the core assembly model:

```text
Role Skill          = who the Agent is and what lifecycle it may initiate
Feature Skills      = capabilities it can load now
System Skills       = how it plans, reasons, and discovers capabilities
Agent profile       = model + role + retrieval defaults
Runtime relations   = who may control or report to whom
```

Change those inputs and the same runtime can become a research team, support
desk, review pipeline, workflow author, or a single-purpose Agent—without
forking the orchestration core.

Full working configuration and lifecycle details:

- [Write Skills](examples/guides/en/04-skills.md)
- [Configure built-in tools and multi-Agent collaboration](examples/guides/en/06-builtin-tools-and-agents.md)
- [Progressive Skills and per-Agent RAG](examples/guides/en/07-progressive-skills-and-rag.md)
- [Connect an external RAG service](examples/guides/en/08-external-rag.md)

## 0.4.8 Beta Focus

The `0.4.8-beta.1` release candidate adds bounded diagnostics for latency
investigation without placing full logs, prompts, or tool payloads on the
Runtime event path:

- **Model latency diagnostics**: every provider attempt records start, success,
  failure, retry scheduling, backoff, exhaustion, response headers, streaming
  first-event latency, and completion duration.
- **Tool-protocol retry evidence**: responses rejected before ledger write record
  the validation reason, thinking attempt, and complete pre-normalization model
  content in the local Runtime diagnostic log so hidden syntax retries can be
  reproduced. Treat this diagnostic file as sensitive data.
- **RPC latency diagnostics**: JSON-lines and gRPC tools record request start,
  first response, HostCall activity, completion, failure, and timeout using
  stable call, conversation, Agent, and turn identifiers.
- **Non-blocking log delivery**: diagnostics use a bounded background writer;
  saturation drops diagnostic entries with an auditable drop count instead of
  delaying model or tool execution.

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
- `examples/guides/en/06-builtin-tools-and-agents.md`
- `examples/guides/en/07-progressive-skills-and-rag.md`
- `examples/guides/en/08-external-rag.md`

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
5. `examples/guides/en/06-builtin-tools-and-agents.md`
6. `examples/guides/en/07-progressive-skills-and-rag.md`
7. `examples/guides/en/08-external-rag.md`
8. `examples/guides/en/11-dynamic-workflows.md`
9. `sdk/README.md`

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
