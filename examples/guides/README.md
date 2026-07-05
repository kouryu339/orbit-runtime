# Corework Integration Guides

This is the product-integration entry point. If you are building an application
on top of Corework, start here before reading SDK API references or FFI
contracts.

The guides are intentionally prescriptive. A working Agent is not created by a
frontend chat window or by one SDK call. Build it in this five-step order:

```text
1. Tools       Prepare callable built-in or sidecar tools.
2. Configs     Prepare resources, LLM providers, and agent cluster configs.
3. Skills      Write role/feature Skills and tool allowlists.
4. Connect     Use Runtime Host SDK and register the three configs.
5. Run         Start Runtime, spawn conversation, relay events, persist if needed.
```

## What Goes Where

| Documentation area | Purpose | Do not use it for |
|---|---|---|
| `examples/guides` | Product integration flow and recommended order. | Low-level ABI ownership details. |
| `sdk` | SDK package entry points and language-specific usage. | Explaining the whole Agent system. |
| `agent_runtime_ffi/docs` | Stable ABI, JSON command, event, provider, and schema contracts. | Step-by-step product onboarding. |
| `ai-assistant/docs` | Internal AI system architecture and mechanisms. | Host SDK usage or public ABI contracts. |

## Required Path

1. [SDK Runtime Connection Flow](en/01-sdk-runtime-connection-flow.md) / [SDK Runtime 连接流程](zh/01-sdk-runtime-connection-flow.md)

   The full `1 -> 2 -> 3 -> 4 -> 5` path from tool descriptors to frontend events.

2. [Runtime Create Options and Registrations](en/02-runtime-configs.md) / [Runtime 创建参数与注册配置](zh/02-runtime-configs.md)

   Step 2 detail: resources, LLM providers, and agent cluster configs.

3. [Connect External Tools](en/03-external-tools.md) / [接入外部 Tools](zh/03-external-tools.md)

   Step 1 detail: how Agent Tool sidecars publish descriptors and return
   `AIOutput.to_ai`.

4. [Write Skills](en/04-skills.md) / [编写 Skill](zh/04-skills.md)

   Step 3 detail: how role and feature Skills grant tool visibility through
   enforced `tools` allowlists.

5. [Connect the Host, Runtime, and Frontend](en/05-host-runtime-frontend.md) /
   [连接宿主、Runtime 与前端](zh/05-host-runtime-frontend.md)

   Step 4 and 5 detail: Runtime Host SDK lifecycle and frontend event relay.

## Advanced Topics

- [Configure Built-in Tools and Multi-Agent Collaboration](en/06-builtin-tools-and-agents.md) /
  [配置内置工具与多 Agent 协作](zh/06-builtin-tools-and-agents.md)
- [Progressive Skills and Per-Agent RAG](en/07-progressive-skills-and-rag.md) /
  [渐进式 Skill 与 Agent RAG](zh/07-progressive-skills-and-rag.md)
- [Connect External RAG](en/08-external-rag.md) / [接入外部 RAG](zh/08-external-rag.md)
- [Context Structure and Snapshot Mechanism](en/09-context-and-snapshots.md) /
  [上下文结构和快照机制](zh/09-context-and-snapshots.md)
- [Persistence and Recovery](en/10-persistence-and-recovery.md) /
  [持久化与恢复机制](zh/10-persistence-and-recovery.md)

## First Implementation Checklist

- Tool sidecars exist and can publish descriptors.
- `resources.json` registers Skill roots, Agent profiles, endpoints, and data roots.
- `llm-providers.json` registers at least one usable model and current model uid.
- `agent-cluster.json` creates at least one focus Agent from a resource profile.
- Role and feature Skills reference only the tools each Agent should see.
- The host registers resources, LLM providers, and cluster before `start`.
- The frontend only calls host conversation APIs and renders Runtime events.

Detailed ABI, schema, provider, and event field references remain under
[`agent_runtime_ffi/docs`](../../agent_runtime_ffi/docs). SDK-specific APIs are
documented under [`sdk`](../../sdk).
