# Agent Runtime FFI Contracts

This directory is the contract reference for hosts and SDK implementers. It
documents stable ABI functions, JSON command/result/event schemas, provider
configuration, frontend message payloads, persistence contracts, and authoring
contracts for Skills and RPC tools.

It is not the product onboarding guide. Start product integration from
[`examples/guides`](../../examples/guides), then return here for exact field
contracts.

## Documentation Boundaries

| Area | Purpose |
|---|---|
| `agent_runtime_ffi/docs` | Normative ABI, JSON schema, event, provider, persistence, Skill, and RPC tool contracts. |
| `examples/guides` | Strong product integration flow and recommended setup order. |
| `sdk` | Runtime Host SDK and Agent Tool SDK package usage. |
| `ai-assistant/docs` | Internal AI system architecture and mechanisms. |

## Contracts And Guides

| Type | English | 中文 |
|---|---|---|
| ABI host contract | [en](en/01-runtime-ffi-usage-guide.md) | [zh](zh/01-runtime-ffi-usage-guide.md) |
| Runtime config contract | [en](en/02-runtime-cluster-config-contract.md) | [zh](zh/02-runtime-cluster-config-contract.md) |
| Conversation lifecycle contract | [en](en/04-runtime-conversation-lifecycle-contract.md) | [zh](zh/04-runtime-conversation-lifecycle-contract.md) |
| Event format | [en](en/05-runtime-event-format.md) | [zh](zh/05-runtime-event-format.md) |
| Frontend message contract | [en](en/06-runtime-frontend-message-contract.md) | [zh](zh/06-runtime-frontend-message-contract.md) |
| Persistence and recovery host contract | [en](en/07-runtime-persistence-and-recovery-contract.md) | [zh](zh/07-runtime-persistence-and-recovery-contract.md) |
| Provider config contract | [en](en/03-runtime-provider-config-contract.md) | [zh](zh/03-runtime-provider-config-contract.md) |
| Diagnostics contract | [en](en/08-runtime-diagnostics-contract.md) | [zh](zh/08-runtime-diagnostics-contract.md) |
| RPC tool authoring guide | [en](en/09-runtime-rpc-tool-authoring-guide.md) | [zh](zh/09-runtime-rpc-tool-authoring-guide.md) |
| Skill authoring guide | [en](en/10-runtime-skill-authoring-guide.md) | [zh](zh/10-runtime-skill-authoring-guide.md) |
| Workflow catalog and execution contract | [en](en/11-runtime-workflow-execution-contract.md) | [zh](zh/11-runtime-workflow-execution-contract.md) |

The ABI source of truth is `sdk/runtime/c/include/agent_runtime.h` and
`agent_runtime_ffi/src/lib.rs`. Runtime features must be discovered through
`agent_runtime_capabilities_v1()`.
