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
pasted into the message composer. Each image inserts an `@图片1`-style reference
at the cursor; pasted text and image references stay in their original order.
It calls the host's
importer for each file and sends the returned `imageId` in ordered message
`parts`. A message can contain images without text. User messages display
inline thumbnails when `capabilities.resolveImage({imageId, conversationId, signal})`
returns a browser URL. Clicking calls `openImage`; unresolved or failed images
retain their `@图片1` text reference.
References use a stable color derived from the referenced object's ID; the
same object keeps the same color across messages. Plain `@name` mentions use
the same palette, keyed by name. The reference color helper is exported for
other host reference types. Local preview URLs are not persisted.

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

Tool cards never render `tool_command`, raw record `content`/`text`, serialized
results, or raw errors, including when expanded. Only an explicit short prose
summary from `metadata.extra.user_summary` is eligible for display; otherwise
the component supplies a localized status explanation. JSON/code-like or
multiline summaries fall back to the status explanation. Hosts must supply
reviewed user-facing prose, not copy raw tool output into this field.
Raw execution data can still exist in the transport/ledger for execution and
audit; hiding it in the UI is not a substitute for host-side access control.

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

## Composer controls

The input uses a two-row layout: text above; images and permissions on the left,
model and reasoning controls plus one primary action on the right. No voice
input is provided. The model menu also links to provider management.

`composerController` optionally implements `ConversationComposerController`
from the `host` export. `getSettings({conversationId, modelUid})` supplies
`effort`, `supportedEfforts`, `permissionMode`, `permissionModes`, and
`canResume`. `setEffort` and `setPermissionMode` return a command result;
the UI reads settings back after success. `effort: null` restores the model
default. The host must apply these settings to this conversation and validate
supported values, authorization and current execution state atomically.
These callbacks must not re-register a shared model or approve pending tools
on the client's behalf. Unimplemented settings remain disabled.

| State | Primary action |
| --- | --- |
| Offline / initializing | Disabled |
| Ready, empty input | Disabled send |
| Ready, text or images | Send |
| Thinking / executing / waiting approval | Pause if supported; approvals stay explicit |
| Sending / pausing / resuming / compacting | Busy indicator, disabled |
| Confirmed paused, empty input | Continue if supported |
| Confirmed paused, new text or images | Send the new message |

Pause admission does not immediately unlock input: the UI waits for a Runtime
waiting snapshot. Continue calls optional `transport.resume(request)`, never
inserts a synthetic user message. The Tauri adapter exposes it only with
`commands.resume`; the HTTP adapter only with `resumePath`. These are
host-supplied routes, not new Runtime DLL commands. If the host cannot resume,
continue remains disabled. Recovered sessions use `getSettings().canResume`.
The shared state machine fences late responses after conversation/model changes
and blocks duplicate commands. Runtime snapshots remain execution truth.

Provider editing uses separate Configure and Import JSON modes, a scrolling body
and fixed actions. Selecting an import file stages its JSON for review; it does
not register providers until submission. Both implementations share preset
selection, duplicate model/context validation and modal focus handling. Busy
requests disable close, mode changes and repeated submission; rejected requests
retain the candidate and show an inline error. Blank API keys are permitted for
local/keyless providers and preserve existing keys when the host supports that
contract. The host remains responsible for authorization and atomic registration.
