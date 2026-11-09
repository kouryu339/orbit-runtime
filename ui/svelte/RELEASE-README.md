# Orbit Conversation — Svelte frontend

Use `orbit-conversation-svelte.js` as a browser ES module. It includes the Svelte
runtime, component styles, Markdown, math, diagram rendering, and fonts. No Svelte
compiler, Lit element, npm install, or CDN is required by the consuming page.
Serve the file over HTTP(S) or your desktop app's asset protocol; do not open it
as a standalone HTML page. The host still supplies a ConversationTransport.

```html
<div id="conversation" style="height: 100dvh"></div>
<script type="module">
  import { mountConversation } from './orbit-conversation-svelte.js';
  // Obtain transport from your host integration; this is not a demo transport.
  const conversation = mountConversation(document.querySelector('#conversation'), {
    transport: window.conversationTransport,
    locale: 'zh-CN',
    colorScheme: 'light',
  });
  // conversation.api.send(text), pause(), resume(), and other public component methods.
  // await conversation.destroy() when removing the view.
</script>
```

Exports include `Conversation`, `RichContent`, `ProviderEditor`, `PlanCard`,
`mountConversation`, and shared host/protocol/transport runtime utilities.

Pass `composerActions` to `mountConversation` to add host actions to the ＋
menu. Each item takes a unique `id`, a `label`, and a
`run({ conversationId })` callback. The optional `disabled` flag disables
the item.

Store the file as a host resource and send the user's question and an opaque
`resource_id` reference through the ordinary text message path. A host-provided
tool can read that resource on demand. The tool should check that the resource
is accessible to the current conversation before returning content.

Pass `fileInput` as a mount prop to enable dropping files into the composer
and picking files from the ＋ menu. The same `accept` list applies to both.
Use dot-prefixed extensions, exact MIME types, or MIME groups such as
`text/*`. An absent or empty list leaves file input disabled.

```js
mountConversation(target, {
  transport,
  fileInput: {
    accept: ['.pdf', '.docx', '.xlsx', '.csv', '.txt'],
    importFile: ({ file, conversationId, signal }) =>
      resources.storeFile({ file, conversationId, signal }), // { resourceId }
  },
});
```

Files are staged until Send, with a limit of eight per message. The host validates
the file content, size, and resource access in its importer and reader.
Svelte adds only resource IDs to the model-facing text and supplies
`display_content` and `extra.file_attachments` in `transport.send().metadata`.
Forward the model-facing `content` or `parts` through the ordinary send path.

In the frontend snapshot, decorate the corresponding user record with
`metadata.display_content` for the user-facing text and
`metadata.extra.file_attachments` for the files to show. Keep this display
metadata with the host's history so it can be applied again on reload. Keep the
original model-facing text in the snapshot record's `content` so the UI can
reconcile its pending message with the confirmed record.

```json
{
  "record_id": "message-42",
  "role": "user",
  "content": "Compare these totals.\nResource: {\"resource_id\":\"res_42\"}",
  "metadata": {
    "display_content": "Compare these totals.",
    "extra": {
      "file_attachments": [
        { "resource_id": "res_42", "name": "quarterly.xlsx", "size_bytes": 8192 }
      ]
    }
  }
}
```

Use an empty `display_content` for a file-only bubble. File names and sizes
are rendered as text. The host can omit `size_bytes` when it is unavailable.
PDF, Word, Excel, CSV, PowerPoint, TXT, and Markdown have built-in type labels.
Other extensions use the generic file label.

Source projects and TypeScript contracts are in `ui/svelte` and `ui/lit` at this
release's Git tag. A Svelte project can use the source package directly instead.

Styles are installed once in the document head when the module is imported.
The bundle targets modern browsers (ES2022); hosts with a Content Security Policy
must permit its injected stylesheet and embedded font data URLs.

`manifest.json` records the matching Runtime release, entry point, source commit,
and file SHA-256 values. Keep `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.txt`
when redistributing the frontend. The separately attached versioned `.js` is
byte-for-byte identical to this archive's stable entry file.
