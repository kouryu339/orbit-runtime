# Agent Runtime Conversation Element

`@agent-runtime/conversation-element` is a Lit web component for rendering and
controlling one Agent Runtime conversation. It is host-transport agnostic: the
application supplies a `ConversationTransport`, and the component consumes
normalized snapshots and lifecycle events.

```ts
import {
  AgentRuntimeConversationElement,
  type ConversationTransport,
} from '@agent-runtime/conversation-element';

const conversation = document.querySelector<AgentRuntimeConversationElement>(
  'agent-runtime-conversation',
)!;
conversation.transport = transport satisfies ConversationTransport;
await conversation.connect();
```

The package exports the custom element, transport and persistence contracts,
the conversation reducer, and rich-content presentation helpers. Native Runtime
loading and authorization belong to the host, never to the browser component.

## Tool Approval

`frontend:state_snapshot.payload.pending_permissions` is the canonical pending
approval list. The component renders every pending request in an approval shelf
immediately above the composer. Matching tool bubbles remain in
`waiting_permission` state but do not duplicate the decision buttons.

The configured transport must implement `resolveToolPermission(request)`. HTTP
SSE hosts normally receive events through SSE and expose the reverse command at
`POST /api/tool-permission`; Tauri hosts map it to
`ai_resolve_tool_permission`. The host owns authorization and forwards the
decision to Runtime command `conversation.resolve_tool_permission`.
The default Tauri permission payload uses Runtime's canonical
`conversation_id` and `tool_call_id` field names inside `args`.

## Development

```text
npm run check
npm test
npm run build
```

The public Runtime event contract is documented in
[`agent_runtime_ffi/docs/en/05-runtime-event-format.md`](../../agent_runtime_ffi/docs/en/05-runtime-event-format.md).

## Execution plans

The conversation renders a collapsible plan card with step states and completion
progress. The focused Agent's snapshot supplies `active_agent_id` and `plan`;
`plan: null` clears it. `conversation.state_delta` with `op: agent_plan.set`
updates the plan for its `agent_id`, without replacing another Agent's card.
The reducer ignores older revisions of the same plan. Snapshot hydration restores
the card; the host remains responsible for storing Runtime state deltas.
Legacy plans with Markdown `content` remain readable.
