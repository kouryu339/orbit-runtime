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

## Image input

The attachment button appears when the transport provides `imageInput.importImage`.
The component accepts up to eight PNG, JPEG, WebP, or GIF files (20 MiB each),
shows local previews, allows removal before sending, and also accepts images
pasted into the message composer. Pasting text with an image keeps normal text
paste behavior. It calls the host's
importer for each file and sends the returned `imageId` in ordered message
`parts`. A message can contain images without text. User messages display
solid-color `（图片1）` references from the Runtime ledger after sending or recovery;
local preview URLs are not persisted.

The browser cannot supply a native path to `conversation.import_image`. The
host's importer must save the selected `File` to a host-accessible temporary
path, invoke Runtime `conversation.import_image` with the conversation ID and
that path, and return `{ imageId }`. The host can then remove its temporary
copy; Runtime owns the imported media. Configure this through `imageImport` in
the Tauri or HTTP transport config. Hosts with a custom `sendArgs` or
`createSendBody` must forward `request.parts` to
`conversation.send_message`; text-only messages continue using `content`.
If a host does not implement image import, Lit hides the attachment button.
The host must authorize the conversation before staging or importing the file,
and must clean up its temporary copy on both success and failure. The Runtime
revalidates media and model capability. For the full host command sequence, see
the [host/frontend guide](../../examples/guides/zh/05-host-runtime-frontend.md).

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
