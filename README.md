# orbit-runtime

**Build Agent products from composable roles, Skills, tools, retrieval, and
runtime lifecycles—behind one embeddable Rust boundary.**

`orbit-runtime` is not a fixed supervisor/worker framework. It is a native
runtime for assembling the Agent topology a product actually needs: one focused
Agent, a persistent specialist team, delegated background workers, or a mixture
of all three.

Desktop apps, services, and scripting hosts embed the Runtime through
`agent_runtime.dll` / `libagent_runtime.so` and a stable C ABI. Browser products
use the JS/TS SDK to run the shared Rust core as WASM in a dedicated Web Worker.
Lit and Svelte provide conversation interfaces for these hosts.

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

## Quick Documentation

Five standalone visual guides explain the Agent loop, session pause and recovery,
workflow engine, interaction and SSE, and external RPC tools. Start with
[Quick Documentation](examples/quick-docs/README.md). The guides are in Simplified
Chinese and can be opened locally without a build or network connection.

## Why orbit-runtime

| Capability | Product-level result |
| --- | --- |
| **Composable Agent clusters** | Choose delegation, focus handoff, progressive capabilities, and retrieval per role instead of inheriting one topology. |
| **Skills as capability boundaries** | Role, feature, and system Skills jointly define instructions and the enforced tool allowlist. |
| **Reliable background work** | Task-scoped authorization, wait/wakeup, input requests, progress, revision, two-phase acceptance, pause, cancellation, and recovery. |
| **Native and browser runtime** | Embed DLL/SO through a stable ABI, or use the JS/TS Worker SDK with the shared WASM core. |
| **Observable and recoverable state** | Consume ordered events and export snapshots without reconstructing truth from chat text. |
| **Structured tool calls** | Use native function calling or the complete JSON call format introduced in 0.5.5, with typed arguments and enforced tool access. |
| **External tools and event sources** | Connect language-neutral RPC tools, dynamically add endpoints, and subscribe to plugin events. |
| **JSON workflow resources** | Author structured workflow logic while Runtime compiles and validates the execution graph. |
| **Lit and Svelte interfaces** | Integrate model and reasoning controls, approvals, pause/resume, and host-managed attachments. |

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
kind: capability
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

## Tool Calling

Each Agent uses one tool transport selected by `toolProtocol`: `native_fc`,
`exec_legacy`, or `disabled`. Omitting the field defaults to `native_fc`;
text-mode tool calling is selected through `exec_legacy`. Native FC is
implemented across OpenAI Chat Completions, OpenAI Responses, and Anthropic
Messages adapters.
Tool names, JSON arguments, provider call IDs, Responses output items, and
formal `tool` results remain structured in the Ledger and survive snapshot
restoration.

The 0.5.5 preview adds complete JSON calls to text-mode conversations. A call
contains a `tool` name and an `args` object. Nested objects, arrays, numbers,
booleans, strings, and JSON null values retain their types through execution
and RPC delivery. Legacy `EXEC` syntax remains a compatibility input.

Providers that implement the strict function-schema subset can additionally
set `strictToolSchema: true`; compatible endpoints keep it disabled. See the
[Runtime cluster configuration contract](agent_runtime_ffi/docs/en/02-runtime-cluster-config-contract.md)
for the full configuration and recovery rules.

## 0.5.5 Feature Overview

The 0.5.5 feature set builds on the native, browser, and frontend capabilities
delivered through 0.5.4. The latest published stable release is currently 0.5.4.
Complete JSON tool calls are available in the
[0.5.5 preview branch](https://github.com/kouryu339/orbit-runtime/tree/jiangrong/json-tool-arguments).

- **Complete JSON tool calls and typed arguments**. Text-mode Agents submit a
  complete `{"tool":"ToolName","args":{}}` object. Native FC and text calls
  share parameter validation, approval, and execution. RPC plugins receive
  the typed parameter object through `args_json`.
- **Structured workflow resources**. Workflow logic is authored as
  `orbit.workflow.resource/v1` JSON. Runtime compiles it into an execution
  graph, including conditions, loops, Switch branches, variables, Pure
  expressions, and reusable workflow references. Draft and formal resources
  share registration, validation, promotion, and execution contracts.
- **Dynamic RPC tools and event sources**. Hosts can add tool endpoints while
  Runtime is running. Plugins also publish events through discovery,
  subscription, polling, and explicit acknowledgement. Hosts bind accepted
  events to their own tasks and workflows.
- **Browser WASM with a JS/TS SDK**. A dedicated Web Worker runs the shared
  Rust Agent and workflow core, with asynchronous host tools, streaming model
  calls, approvals, and IndexedDB persistence. Native integrations use the
  corresponding Windows DLL or Linux SO.
- **Lit and Svelte conversation interfaces**. Both frontends expose model and
  reasoning controls, tool approvals, and pause/resume. Svelte also supports
  host extensions to the attachment menu, file cards, and configured drag/drop
  formats, and is distributed as a standalone JavaScript artifact.
- **Dynamic clusters and transactional Skills**. Hosts register clusters and
  submit Role or Capability Skill content through validated transactions.
  Runtime enforces live-conversation ownership and version checks when updating
  definitions.
- **Conversation lifecycle and recovery**. Hosts pause and resume work,
  archive full conversation history, and restore it using current Skills and
  permissions. Snapshots retain messages, tool calls, results, and call IDs.
- **Scoped retrieval and host file decoding**. Grep/GrepJson combine cluster
  limits with conversation-specific read grants. Hosts provide text or JSON
  decoding for additional file formats through the native decoding interface.

Download published artifacts from
[GitHub Releases](https://github.com/kouryu339/orbit-runtime/releases/latest).

## Repository Layout

| Path | Purpose |
| --- | --- |
| `corework/` | Orchestration foundation: state machines, events, cache, workflows, and the RPC Tool protocol. |
| `ai-gateway/` | Model gateway for LLM / VLM / ASR / OCR providers and OpenAI-compatible endpoints. |
| `ai-assistant/` | Agent runtime: conversation state machine, skills, tool execution, ledger, and persistence. |
| `agent_runtime_ffi/` | C ABI wrapper that builds the native runtime library. |
| `agent_runtime_wasm/` | Browser Worker entry point for the shared Rust runtime. |
| `ui/lit/` | Lit implementation of the shared conversation frontend contract. |
| `ui/svelte/` | Svelte implementation of the same contract, distributed as frontend build output. |
| `sdk/` | Runtime Host SDKs, the browser JS/TS SDK, and RPC Tool SDKs. |
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

### Additional Project Example: Reidom

[Reidom](https://github.com/kouryu339/reidom) is an AI-native MIDI arranging
application combining a Tauri desktop UI, a JUCE project/audio engine, an Agent
Tool service, and Orbit Runtime configuration and Skills. It provides another
product example of connecting a host, domain tools, and the runtime. Follow that
repository's setup and version requirements when building it.

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
| Browser JS/TS SDK | Browser apps using a dedicated Web Worker | Host -> Worker -> WASM Runtime |
| RPC Tool SDK | Tool sidecars and product capability adapters | Runtime -> Tool |

See `sdk/README.md` for language support and the native runtime release manifest.

## Prepare Release Packages

Release packages are prepared by:

```powershell
.\scripts\prepare-release.ps1
```

The script prepares native libraries, browser WASM, frontend JavaScript, and
matching SDK sources:

```text
dist/releases/orbit-runtime-runtime-vX.Y.Z-windows-x86_64.zip
dist/releases/orbit-runtime-runtime-vX.Y.Z-linux-x86_64.zip
dist/releases/orbit-runtime-browser-vX.Y.Z.zip
dist/releases/orbit-runtime-browser-vX.Y.Z.tgz
dist/releases/orbit-runtime-frontend-svelte-vX.Y.Z.js
dist/releases/orbit-runtime-frontend-svelte-vX.Y.Z.zip
dist/releases/orbit-runtime-sdk-vX.Y.Z.zip
```

Native packages contain the library, `agent_runtime.h`, public schemas,
integration guides, and license notices. Browser packages include the Worker,
WASM module, JS/TS SDK, and schemas. Svelte is available as a standalone JS
file and an archive. Artifacts have adjacent `.sha256` checksums.

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

Stable version updates and host migration reports: [Release history](agent_runtime_ffi/docs/releases/README.md).

## Status

Covered integration paths:

- FFI runtime creation, startup, command dispatch, event polling, snapshot export, shutdown, and destroy.
- Agent conversation lifecycle, tool execution, skill loading, ledger state, and persistence contracts.
- LLM gateway integration for provider configuration and OpenAI-compatible endpoints.
- Lit and Svelte frontend integration through host-provided runtime events.
- Browser Worker integration through the JS/TS SDK and WASM runtime.
- Windows DLL, Linux SO, browser WASM, and standalone Svelte packaging.

Still evolving:

- Additional host examples and packaging patterns.
- macOS release packaging from an Apple build environment.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
