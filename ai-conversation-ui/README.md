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

## Development

```text
npm run check
npm test
npm run build
```

The public Runtime event contract is documented in
[`agent_runtime_ffi/docs/en/05-runtime-event-format.md`](../agent_runtime_ffi/docs/en/05-runtime-event-format.md).
